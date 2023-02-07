// TODO: somehow better handle rate-limits (https://core.telegram.org/bots/faq#broadcasting-to-users)
//       maybe concat many messages into one (in channel) + queues to properly
//       handle limits

// When index collapses, use `git reset --hard origin/master`
#![allow(clippy::type_complexity)]
use std::{convert::Infallible, iter, sync::Arc, time::Duration};

use arraylib::Slice;
use either::Either::{Left, Right};
use fntools::{self, value::ValueExt};
use futures::future::{self, pending};
use git2::{Commit, Delta, Diff, DiffOptions, Repository, Sort};
use log::info;
use std::str;
use teloxide::{adaptors::DefaultParseMode, prelude::*, types::ParseMode};
use tokio::sync::{
    mpsc::{self, Sender},
    oneshot,
};
use tokio_postgres::NoTls;

use crate::{db::Database, krate::Crate, util::tryn};

mod bot;
mod cfg;
mod db;
mod krate;
mod util;

const VERSION: &str = env!("CARGO_PKG_VERSION");

type Bot = DefaultParseMode<teloxide::Bot>;

#[tokio::main]
async fn main() {
    assert_eq!(
        unsafe {
            let opt = libgit2_sys::GIT_OPT_SET_MWINDOW_FILE_LIMIT as _;
            libgit2_sys::git_libgit2_opts(opt, 128)
        },
        0,
    );

    let config = Arc::new(cfg::Config::read().expect("couldn't read config"));

    simple_logger::SimpleLogger::new()
        .with_level(config.loglevel)
        .init()
        .expect("Failed to initialize logger");

    info!("starting");

    let db = {
        let (d, conn) = Database::connect(&config.db.cfg(), NoTls)
            .await
            .expect("couldn't connect to the database");

        // docs says to do so
        tokio::spawn(async move {
            if let Err(e) = conn.await {
                eprintln!("Database connection error: {e}");
            }
        });

        info!("connected to db");
        d
    };

    let index_url = &config.index_url; // Closures still borrow full struct :|
    let index_path = &config.index_path;
    let repo = Repository::open(index_path).unwrap_or_else(move |_| {
        info!("start cloning");
        Repository::clone(index_url, index_path)
            .unwrap()
            .also(|_| info!("cloning finished"))
    });

    let (abortable, abort_handle) = future::abortable(pending::<()>());

    let (tx, mut rx) = mpsc::channel(2);
    let git2_th = {
        let pull_delay = config.pull_delay;
        std::thread::spawn(move || {
            'outer: loop {
                log::info!("start pulling updates");

                if let Err(err) = pull(&repo, tx.clone()) {
                    log::error!("couldn't pull new crate version from the index: {}", err)
                }

                log::info!("pulling updates finished");

                // delay for `config.pull_delay` (default 5 min)
                {
                    let mut pd = pull_delay;
                    const STEP: Duration = Duration::from_secs(5);

                    while pd > Duration::ZERO {
                        if abortable.is_aborted() {
                            break 'outer;
                        }

                        pd = pd.saturating_sub(STEP);
                        std::thread::sleep(STEP);
                    }
                }
            }
        })
    };

    let bot = teloxide::Bot::new(&config.bot_token).parse_mode(ParseMode::Html);

    let notify_loop = async {
        while let Some((res, _unblock)) = rx.recv().await {
            match res {
                Ok((krate, action)) => notify(krate, action, &bot, &db, &config).await,
                Err(e) => {
                    log::error!("diff_one error: {e:?}");
                    if let Some(chat_id) = config.error_report_channel_id {
                        bot.send_message(chat_id, format!("diff_one error: {e:?}"))
                            .await
                            .ok();
                    }
                }
            }

            // implicitly unblock git2 thread by dropping `_unblock`
        }

        // `recv()` returned `None` => `tx` was dropped => `git2_th` was stopped
        // => `abort_handle.abort()` was probably called
    };

    let tg_loop = async {
        bot::run(bot.clone(), db.clone(), Arc::clone(&config)).await;

        // When bot stopped executing (e.g. because of ^C) stop pull loop
        abort_handle.abort();
    };

    tokio::join!(notify_loop, tg_loop);

    git2_th.join().unwrap();
}

/// Fast-Forward (FF) to a given commit.
///
/// Implementation is taken from <https://stackoverflow.com/a/58778350>.
fn fast_forward(repo: &Repository, commit: &git2::Commit) -> Result<(), git2::Error> {
    let fetch_commit = repo.find_annotated_commit(commit.id())?;
    let analysis = repo.merge_analysis(&[&fetch_commit])?;

    if analysis.0.is_up_to_date() {
        Ok(())
    } else if analysis.0.is_fast_forward() {
        let mut reference = repo.find_reference("refs/heads/master")?;
        reference.set_target(fetch_commit.id(), "Fast-Forward")?;
        repo.set_head(reference.name().unwrap())?;
        repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
    } else {
        Err(git2::Error::from_str("Fast-forward only!"))
    }
}

fn pull(
    repo: &Repository,
    ch: Sender<(
        Result<(Crate, ActionKind), git2::Error>,
        oneshot::Sender<Infallible>,
    )>,
) -> Result<(), git2::Error> {
    // fetch changes from remote index
    repo.find_remote("origin")?.fetch(&["master"], None, None)?;

    // Collect all commits in the range `HEAD~1..FETCH_HEAD` (i.e. one before
    // currently checked out to the last fetched)
    let mut walk = repo.revwalk()?;
    walk.push_range("HEAD~1..FETCH_HEAD")?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE)?;
    let commits: Result<Vec<_>, _> = walk.map(|oid| repo.find_commit(oid?)).collect();

    let mut opts = DiffOptions::default();
    let opts = opts.context_lines(0).minimal(true);

    for [prev, next] in Slice::array_windows::<[_; 2]>(&commits?[..]) {
        let message = next
            .message()
            .unwrap_or("<invalid utf-8>")
            .trim_end_matches('\n');

        // Commits from humans tend to be formatted differently, compared to
        // machine-generated ones. This basically makes them unanalyzable.
        if next.author().name() != Some("bors") {
            log::warn!(
                "Skip commit#{} from non-bors user @{}: {message}",
                next.id(),
                next.author().name().unwrap_or("<invalid utf-8>"),
            );

            continue;
        }

        if message == "Crate version removal request" {
            log::warn!("Skip crate version removal request commit#{}", next.id());
            continue;
        }

        if message.starts_with("Merge remote-tracking branch") {
            log::warn!("Skip merge commit#{}", next.id());
            continue;
        }

        let diff = repo.diff_tree_to_tree(Some(&prev.tree()?), Some(&next.tree()?), Some(opts))?;
        let res = diff_one(diff, (prev, next));

        // Send crates.io update to notifier
        let (tx, mut rx) = oneshot::channel();
        ch.blocking_send((res, tx)).ok().unwrap();

        // Wait until the crate is processed before moving on
        while let Err(oneshot::error::TryRecvError::Empty) = rx.try_recv() {
            // Yield/sleep to not spend all resources
            std::thread::sleep(Duration::from_secs(1));
        }

        // 'Move' to the next commit
        fast_forward(repo, next)?;
    }

    Ok(())
}

enum ActionKind {
    NewVersion,
    Yanked,
    Unyanked,
}

/// Get a `crates.io` update from a diff of 2 consecutive commits from a
/// `crates.io-index` repository.
fn diff_one(diff: Diff, commits: (&Commit, &Commit)) -> Result<(Crate, ActionKind), git2::Error> {
    let mut prev = None;
    let mut next = None;

    let mut error = Ok(());

    diff.foreach(
        &mut |_, _| true,
        None,
        None,
        Some(&mut |delta, _hunk, line| {
            match delta.status() {
                // New version of a crate or (un)yanked old version
                Delta::Modified | Delta::Added => {
                    if !(delta.nfiles() == 2 || delta.nfiles() == 1) {
                        error = Err(format!("Unexpected delta.nfiles: {delta:?}"));
                        return false;
                    }

                    match line.origin() {
                        '-' => {
                            if prev.is_some() {
                                error =
                                    Err("Expected number of deletions <= 1 per commit".to_owned());
                                return false;
                            }

                            let krate = match str::from_utf8(line.content()) {
                                Ok(r) => r,
                                Err(e) => {
                                    error = Err(format!("Non UTF-8 diff: {e:?}"));
                                    return false;
                                }
                            };
                            let krate = match serde_json::from_str::<Crate>(krate) {
                                Ok(r) => r,
                                Err(e) => {
                                    error = Err(format!("Couldn't deserialize crate: {e:?}"));
                                    return false;
                                }
                            };

                            prev = Some(krate);
                        }
                        '+' => {
                            if next.is_some() {
                                error =
                                    Err("Expected number of additions = 1 per commit".to_owned());
                                return false;
                            }

                            let krate = match str::from_utf8(line.content()) {
                                Ok(r) => r,
                                Err(e) => {
                                    error = Err(format!("Non UTF-8 diff: {e:?}"));
                                    return false;
                                }
                            };
                            let krate = match serde_json::from_str::<Crate>(krate) {
                                Ok(r) => r,
                                Err(e) => {
                                    error = Err(format!("Couldn't deserialize crate: {e:?}"));
                                    return false;
                                }
                            };

                            next = Some(krate);
                        }
                        _ => { /* don't care */ }
                    }
                }
                delta => {
                    log::warn!("Unexpected delta: {:?}", delta);
                }
            }

            true
        }),
    )?;

    let err =
        |e| git2::Error::from_str(&format!("{e} ({} -> {})", commits.0.id(), commits.1.id(),));

    if let Err(e) = error {
        return Err(err(&*e));
    }

    let next = next.ok_or_else(|| err("Expected number of additions = 1 per commit"))?;
    match (prev.as_ref().map(|c| c.yanked), next.yanked) {
        /* was yanked?, is yanked? */
        (None, false) => {
            // There were no deleted line & crate is not yanked.
            // New version.
            Ok((next, ActionKind::NewVersion))
        }
        (Some(false), true) => {
            // The crate was not yanked and now is yanked.
            // Crate was yanked.
            Ok((next, ActionKind::Yanked))
        }
        (Some(true), false) => {
            // The crate was yanked and now is not yanked.
            // Crate was unyanked.
            Ok((next, ActionKind::Unyanked))
        }
        _unexpected => {
            // Something unexpected happened
            Err(err(&format!(
                "Unexpected diff_one input: {prev:?} -> {next:?} ",
            )))
        }
    }
}

async fn notify(krate: Crate, action: ActionKind, bot: &Bot, db: &Database, cfg: &cfg::Config) {
    let message = format!(
        "Crate was {action}: <code>{krate}#{version}</code> {links}",
        krate = krate.id.name,
        version = krate.id.vers,
        links = krate.html_links(),
        action = match action {
            ActionKind::NewVersion => "updated",
            ActionKind::Yanked => "yanked",
            ActionKind::Unyanked => "unyanked",
        }
    );

    let users = db
        .list_subscribers(&krate.id.name)
        .await
        .map(Left)
        .map_err(|err| log::error!("db error while getting subscribers: {}", err))
        .unwrap_or_else(|()| Right(iter::empty()));

    if let Some(chat_id) = cfg.channel {
        if !cfg.ban.crates.contains(krate.id.name.as_str()) {
            notify_inner(bot, chat_id, &message, cfg, &krate, true).await;
        }
    }

    for chat_id in users {
        notify_inner(bot, chat_id, &message, cfg, &krate, false).await;
        tokio::time::sleep(cfg.broadcast_delay_millis.into()).await;
    }
}

async fn notify_inner(
    bot: &Bot,
    chat_id: ChatId,
    msg: &str,
    cfg: &cfg::Config,
    krate: &Crate,
    quiet: bool,
) {
    tryn(5, cfg.retry_delay.0, || {
        bot.send_message(chat_id, msg)
            .disable_web_page_preview(true)
            .disable_notification(quiet)
    })
    .await
    .map(drop)
    .unwrap_or_else(|err| {
        log::error!(
            "error while trying to send notification about {:?} to {}: {}",
            krate,
            chat_id,
            err
        )
    });
}
