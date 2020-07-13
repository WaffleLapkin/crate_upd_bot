// TODO: somehow better handle rate-limits (https://core.telegram.org/bots/faq#broadcasting-to-users)
//       maybe concat many messages into one (in channel) + queues to properly handle limits

// TODO iterate through all commits instead of diffing them all

use crate::{bot::setup, db::Database, krate::Crate, util::tryn};
use carapax::{methods::SendMessage, types::ParseMode, Api};
use fntools::{self, value::ValueExt};
use git2::{Delta, Diff, DiffOptions, ObjectType, Repository};
use log::info;
use std::{collections::HashMap, str};
use tokio_postgres::NoTls;

mod bot;
mod db;
mod krate;
mod util;
mod cfg;

#[tokio::main]
async fn main() {
    let config = cfg::Config::read().expect("couldn't read config");

    simple_logger::init_with_level(config.loglevel).unwrap();
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
    let repo = Repository::open("./index").unwrap_or_else(move |_| {
        info!("start cloning");
        Repository::clone(&index_url, "./index")
            .unwrap()
            .also(|_| info!("cloning finished"))
    });

    let bot = Api::new(carapax::Config::new(&config.bot_token)).expect("Can't crate Api");

    let lp = setup(bot.clone(), db.clone(), config.retry_delay);
    tokio::spawn(lp.run());

    loop {
        log::info!("start pulling updates");
        pull(&repo, &bot, &db, &config).await.expect("pull failed");
        log::info!("pulling updates finished");

        tokio::time::delay_for(config.pull_delay).await; // delay for 5 min
    }
}

async fn pull(repo: &Repository, bot: &Api, db: &Database, cfg: &cfg::Config) -> Result<(), git2::Error> {
    // TODO: use spawn_blocking here

    // fetch changes from remote index
    repo.find_remote("origin")
        .expect("couldn't find 'origin' remote")
        .fetch(&["master"], None, None)
        .expect("couldn't fetch new version of the index");

    // last commit
    let our_commit = repo
        .head()?
        .resolve()?
        .peel(ObjectType::Commit)?
        .into_commit()
        .map_err(|_| git2::Error::from_str("commit error"))?;

    // last fetched commit
    let their_commit = repo.find_reference("FETCH_HEAD")?.peel_to_commit()?;

    let diff: Diff = repo.diff_tree_to_tree(
        Some(&our_commit.tree()?),
        Some(&their_commit.tree()?),
        Some(DiffOptions::default().context_lines(0).minimal(true)),
    )?;

    let mut deletions = HashMap::new();
    let mut additions = Vec::new();

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
                            let krate = str::from_utf8(line.content()).expect("non-utf8 diff");
                            let krate = serde_json::from_str::<Crate>(krate)
                                .expect("cound't deserialize crate");

                            deletions.insert(krate.id.clone(), krate);
                        }
                        '+' => {
                            let krate = str::from_utf8(line.content()).expect("non-utf8 diff");
                            let krate = serde_json::from_str::<Crate>(krate)
                                .expect("cound't deserialize crate");

                            additions.push(krate);
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

    // Note: we are not using FuturesUnordered here, to prevent "too many requests" error from telegram
    for add in additions {
        let prev = deletions.remove(&add.id);
        notify(add, prev, bot, db, cfg).await;
        tokio::time::delay_for(cfg.update_delay_millis.into()).await;
    }

    // from https://stackoverflow.com/a/58778350
    fn fast_forward(repo: &Repository) -> Result<(), git2::Error> {
        let fetch_head = repo.find_reference("FETCH_HEAD")?;
        let fetch_commit = repo.reference_to_annotated_commit(&fetch_head)?;
        let analysis = repo.merge_analysis(&[&fetch_commit])?;
        if analysis.0.is_up_to_date() {
            Ok(())
        } else if analysis.0.is_fast_forward() {
            let mut reference = repo.find_reference("refs/heads/master")?;
            reference.set_target(fetch_commit.id(), "Fast-Forward")?;
            repo.set_head("refs/heads/master")?;
            repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
        } else {
            Err(git2::Error::from_str("Fast-forward only!"))
        }
    }

    fast_forward(repo)?;

    Ok(())
}

async fn notify(krate: Crate, prev: Option<Crate>, bot: &Api, db: &Database, cfg: &cfg::Config) {
    let message = match (prev.as_ref().map(|c| c.yanked), krate.yanked) {
        /* was yanked, is yanked */
        (None, false) => {
            // There were no deleted line & crate is not yanked.
            // New version.
            Some(format!(
                "Crate was updated: <code>{krate}#{version}</code> \
                    <a href='{docs}'>[docs.rs]</a> \
                    <a href='{crates}'>[crates.io]</a> \
                    <a href='{lib}'>[lib.rs]</a>",
                krate = krate.id.name,
                version = krate.id.vers,
                docs = krate.docsrs(),
                crates = krate.cratesio(),
                lib = krate.librs(),
            ))
        }
        (Some(false), true) => {
            // The crate was not yanked and now is yanked.
            // Crate yanked.
            Some(format!(
                "Crate was yanked: <code>{krate}#{version}</code> \
                    <a href='{docs}'>[docs.rs]</a> \
                    <a href='{crates}'>[crates.io]</a> \
                    <a href='{lib}'>[lib.rs]</a>",
                krate = krate.id.name,
                version = krate.id.vers,
                docs = krate.docsrs(),
                crates = krate.cratesio(),
                lib = krate.librs(),
            ))
        }
        (Some(true), false) => {
            // The crate was yanked and now is not yanked.
            // Crate unyanked.

            Some(format!(
                "Crate was unyanked: <code>{krate}#{version}</code> \
                    <a href='{docs}'>[docs.rs]</a> \
                    <a href='{crates}'>[crates.io]</a> \
                    <a href='{lib}'>[lib.rs]</a>",
                krate = krate.id.name,
                version = krate.id.vers,
                docs = krate.docsrs(),
                crates = krate.cratesio(),
                lib = krate.librs(),
            ))
        }
        _unexpected => {
            // Something unexpected happened
            log::warn!("Unexpected notify input: {:?}, {:?}", krate, prev);
            None
        }
    };

    if let Some(message) = message {
        let users = db
            .list_subscribers(&krate.id.name)
            .await
            .map_err(|err| log::error!("db error while getting subscribers: {}", err))
            .unwrap_or_default();

        let chat_ids = users.into_iter().chain(cfg.channel.into_iter());

        for chat_id in chat_ids {
            tryn(5, cfg.retry_delay.0, || {
                bot.execute(
                    SendMessage::new(chat_id, &message)
                        .parse_mode(ParseMode::Html)
                        .disable_web_page_preview(true),
                )
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
            tokio::time::delay_for(cfg.broadcast_delay_millis.into()).await;
        }
    }
}
