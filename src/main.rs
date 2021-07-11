// TODO: somehow better handle rate-limits (https://core.telegram.org/bots/faq#broadcasting-to-users)
//       maybe concat many messages into one (in channel) + queues to properly
// handle limits
use std::sync::Arc;

use arraylib::Slice;
use fntools::{self, value::ValueExt};
use git2::{Delta, Diff, DiffOptions, Repository, Sort};
use log::info;
use std::str;
use teloxide::{
    adaptors::{AutoSend, DefaultParseMode},
    prelude::*,
    types::ParseMode,
};
use tokio_postgres::NoTls;

use crate::{db::Database, krate::Crate, util::tryn};
// bot::setup,

mod bot;
mod cfg;
mod db;
mod krate;
mod util;

const VERSION: &str = env!("CARGO_PKG_VERSION");

type Bot = AutoSend<DefaultParseMode<teloxide::Bot>>;

#[tokio::main]
async fn main() {
    unsafe {
        dbg!(libgit2_sys::git_libgit2_opts(
            libgit2_sys::GIT_OPT_SET_MWINDOW_FILE_LIMIT as _,
            128
        ))
    };

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
                eprintln!("Database connection error: {}", e);
            }
        });

        info!("connected to db");
        d
    };

    let index_url = &config.index_url; // Closures still borrow full struct :|
    let index_path = &config.index_path;
    let repo = Repository::open(index_path).unwrap_or_else(move |_| {
        info!("start cloning");
        Repository::clone(&index_url, index_path)
            .unwrap()
            .also(|_| info!("cloning finished"))
    });

    let bot = teloxide::Bot::new(&config.bot_token)
        .parse_mode(ParseMode::Html)
        .auto_send();

    let pull_loop = async {
        loop {
            log::info!("start pulling updates");
            pull(&repo, &bot, &db, &config).await.expect("pull failed");
            log::info!("pulling updates finished");

            tokio::time::sleep(config.pull_delay).await; // delay for 5 min
        }
    };

    tokio::join!(
        pull_loop,
        bot::run(bot.clone(), db.clone(), Arc::clone(&config))
    );
}

// from https://stackoverflow.com/a/58778350
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

async fn pull(
    repo: &Repository,
    bot: &Bot,
    db: &Database,
    cfg: &cfg::Config,
) -> Result<(), git2::Error> {
    // fetch changes from remote index
    repo.find_remote("origin")
        .expect("couldn't find 'origin' remote")
        .fetch(&["master"], None, None)
        .expect("couldn't fetch new version of the index");

    let mut walk = repo.revwalk()?;
    walk.push_range("HEAD~1..FETCH_HEAD")?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE)?;
    let commits: Result<Vec<_>, _> = walk.map(|oid| repo.find_commit(oid?)).collect();
    let mut opts = DiffOptions::default();
    let opts = opts.context_lines(0).minimal(true);
    for [prev, next] in Slice::array_windows::<[_; 2]>(&commits?[..]) {
        if next.author().name() != Some("bors") {
            log::warn!(
                "Skip commit#{} from non-bors user@{}: {}",
                next.id(),
                next.author().name().unwrap_or("<invalid utf-8>"),
                next.message()
                    .unwrap_or("<invalid utf-8>")
                    .trim_end_matches('\n'),
            );

            continue;
        }

        let diff: Diff =
            repo.diff_tree_to_tree(Some(&prev.tree()?), Some(&next.tree()?), Some(opts))?;
        let (krate, action) = diff_one(diff)?;
        notify(krate, action, bot, db, cfg).await;
        fast_forward(repo, next)?;
        // Try to prevent "too many requests" error from telegram
        tokio::time::sleep(cfg.update_delay_millis.into()).await;
    }

    Ok(())
}

enum ActionKind {
    NewVersion,
    Yanked,
    Unyanked,
}

fn diff_one(diff: Diff) -> Result<(Crate, ActionKind), git2::Error> {
    let mut prev = None;
    let mut next = None;

    diff.foreach(
        &mut |_, _| true,
        None,
        None,
        Some(&mut |delta, _hunk, line| {
            match delta.status() {
                // New version of a crate or (un)yanked old version
                Delta::Modified | Delta::Added => {
                    assert!(delta.nfiles() == 2 || delta.nfiles() == 1);
                    match line.origin() {
                        '-' => {
                            assert!(
                                prev.is_none(),
                                "Expected number of deletions <= 1 per commit"
                            );
                            let krate = str::from_utf8(line.content()).expect("non-utf8 diff");
                            let krate = serde_json::from_str::<Crate>(krate)
                                .expect("cound't deserialize crate");

                            prev = Some(krate);
                        }
                        '+' => {
                            assert!(
                                next.is_none(),
                                "Expected number of additions = 1 per commit"
                            );
                            let krate = str::from_utf8(line.content()).expect("non-utf8 diff");
                            let krate = serde_json::from_str::<Crate>(krate)
                                .expect("cound't deserialize crate");

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

    let next = next.expect("Expected number of additions = 1 per commit");
    match (prev.as_ref().map(|c| c.yanked), next.yanked) {
        /* was yanked, is yanked */
        (None, false) => {
            // There were no deleted line & crate is not yanked.
            // New version.
            Ok((next, ActionKind::NewVersion))
        }
        (Some(false), true) => {
            // The crate was not yanked and now is yanked.
            // Crate yanked.
            Ok((next, ActionKind::Yanked))
        }
        (Some(true), false) => {
            // The crate was yanked and now is not yanked.
            // Crate unyanked.
            Ok((next, ActionKind::Unyanked))
        }
        _unexpected => {
            // Something unexpected happened
            log::warn!("Unexpected diff_one input: {:?}, {:?}", next, prev);
            Err(git2::Error::from_str("Unexpected diff"))
        }
    }
}

async fn notify(krate: Crate, action: ActionKind, bot: &Bot, db: &Database, cfg: &cfg::Config) {
    let message = match action {
        ActionKind::NewVersion => format!(
            "Crate was updated: <code>{krate}#{version}</code> {links}",
            krate = krate.id.name,
            version = krate.id.vers,
            links = krate.html_links(),
        ),
        ActionKind::Yanked => format!(
            "Crate was yanked: <code>{krate}#{version}</code> {links}",
            krate = krate.id.name,
            version = krate.id.vers,
            links = krate.html_links(),
        ),
        ActionKind::Unyanked => format!(
            "Crate was unyanked: <code>{krate}#{version}</code> {links}",
            krate = krate.id.name,
            version = krate.id.vers,
            links = krate.html_links(),
        ),
    };

    let users = db
        .list_subscribers(&krate.id.name)
        .await
        .map_err(|err| log::error!("db error while getting subscribers: {}", err))
        .unwrap_or_default();

    if let Some(ch) = cfg.channel {
        if !cfg.ban.crates.contains(krate.id.name.as_str()) {
            notify_inner(bot, ch, &message, cfg, &krate, true).await;
        }
    }

    for chat_id in users {
        notify_inner(bot, chat_id, &message, cfg, &krate, false).await;
    }
}

async fn notify_inner(
    bot: &Bot,
    chat_id: i64,
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
    tokio::time::sleep(cfg.broadcast_delay_millis.into()).await;
}
