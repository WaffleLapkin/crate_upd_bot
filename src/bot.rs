use std::{ops::Not, path::PathBuf, sync::Arc};

use crate::{cfg::Config, db::Database, krate::Crate, util::crate_path, Bot, VERSION};
use fntools::value::ValueExt;
use teloxide::{
    prelude::{Requester, *},
    types::{Me, Message},
    utils::command::{BotCommand, ParseError},
    RequestError,
};

type OptString = Option<String>;

#[derive(BotCommand, PartialEq, Debug)]
#[command(rename = "lowercase", parse_with = "split")]
enum Command {
    Start,
    #[command(parse_with = "opt")]
    Subscribe(OptString),
    #[command(parse_with = "opt")]
    Unsubscribe(OptString),
    List,
}

#[derive(Debug, derive_more::Display, derive_more::From, derive_more::Error)]
enum HErr {
    Tg(RequestError),
    Bd(tokio_postgres::Error),
    GetUser,
    NotAdmin,
}

pub async fn run(bot: Bot, db: Database, cfg: Arc<Config>) {
    let Me { user, .. } = bot.get_me().await.expect("Couldn't get myself :(");
    let name = user.username.expect("Bots *must* have usernames");

    let f = |UpdateWithCx {
                 update: msg,
                 requester: bot,
             }: UpdateWithCx<Bot, Message>,
             cmd: Command,
             (db, cfg): (Database, Arc<Config>)| async move {
        let chat_id = msg.chat.id;

        check_privileges(&bot, &msg).await?;

        match cmd {
            Command::Start => {
                let greeting = format!(
                    "Hi! I will notify you about updates of crates. \
                     Use /subscribe to subscribe for updates of crates you want to be notified about.\n\
                     \n\
                     In case you want to see <b>all</b> updates go to @crates_updates\n\
                     \n\
                     Author: @wafflelapkin\n\
                     His channel [ru]: @ihatereality\n\
                     My source: <a href='https://github.com/WaffleLapkin/crate_upd_bot'>[github]</a>\n\
                     Version: <code>{version}</code>", 
                    version = VERSION
                );
                bot.send_message(chat_id, greeting).await?;
            }
            Command::Subscribe(Some(krate)) => match subscribe(chat_id, &krate, &db, &cfg).await? {
                Some(ver) => {
                    bot.send_message(
                        chat_id,
                        format!(
                            "You've successfully subscribed for updates on <code>{}</code>{} \
                             crate. Use /unsubscribe to unsubscribe.",
                            krate, ver
                        ),
                    )
                    .disable_web_page_preview(true)
                    .await?;
                }
                None => {
                    bot.send_message(
                        chat_id,
                        format!("Error: there is no such crate <code>{}</code>.", krate),
                    )
                    .await?;
                }
            },

            Command::Subscribe(None) => {
                bot.send_message(
                    chat_id,
                    "You need to specify the crate you want to subscribe. Like this: \
                     <pre>/subscribe serde</pre>",
                )
                .await?;
            }

            Command::Unsubscribe(Some(krate)) => {
                db.unsubscribe(chat_id, &krate).await?;
                bot.send_message(
                    chat_id,
                    format!(
                        "You've successfully unsubscribed for updates on <code>{}</code> crate. \
                         Use /subscribe to subscribe back.",
                        krate
                    ),
                )
                .await?;
            }
            Command::Unsubscribe(None) => {
                bot.send_message(
                    chat_id,
                    "You need to specify the crate you want to unsubscribe. Like this: \
                     <code>/unsubscribe serde</code>",
                )
                .await?;
            }
            Command::List => {
                let subscriptions = list(chat_id, &db, &cfg).await?;

                if subscriptions.is_empty() {
                    bot.send_message(
                        chat_id,
                        String::from(
                            "Currently you aren't subscribed to anything. Use /subscribe to \
                             subscribe to some crate.",
                        ),
                    )
                    .await?;
                } else {
                    bot.send_message(
                        chat_id,
                        format!(
                            "You are currently subscribed to:\n— <code>{}",
                            subscriptions.join("\n— <code>")
                        ),
                    )
                    .disable_web_page_preview(true)
                    .await?;
                }
            }
        }

        Ok::<_, HErr>(())
    };

    teloxide::commands_repl(bot, name, with((db, cfg), f)).await
}

async fn list(chat_id: i64, db: &Database, cfg: &Config) -> Result<Vec<String>, HErr> {
    let mut subscriptions: Vec<_> = db.list_subscriptions(chat_id).await?.collect();
    for sub in &mut subscriptions {
        match Crate::read_last(sub, cfg).await {
            Ok(krate) => {
                sub.push('#');
                sub.push_str(&krate.id.vers);
                sub.push_str("</code> ");
                sub.push_str(&krate.html_links());
            }
            Err(_) => {
                sub.push_str(" </code>");
                /* silently ignore error & just don't add links */
            }
        }
    }

    Ok(subscriptions)
}

async fn check_privileges(bot: &Bot, msg: &Message) -> Result<(), HErr> {
    if !msg.chat.is_private() {
        let admins = bot.get_chat_administrators(msg.chat_id()).await?;

        let user_id = msg.from().ok_or(HErr::GetUser)?.id;
        if admins
            .iter()
            .map(|admin| admin.user.id)
            .any(|id| id == user_id)
            .not()
        {
            return Err(HErr::NotAdmin);
        }
    };

    Ok(())
}

async fn subscribe(
    chat_id: i64,
    krate: &str,
    db: &Database,
    cfg: &Config,
) -> Result<Option<String>, HErr> {
    if PathBuf::from(cfg.index_path.as_str())
        .also(|p| p.push(crate_path(krate)))
        .exists()
    {
        db.subscribe(chat_id, krate).await?;

        let ver = match Crate::read_last(krate, cfg).await {
            Ok(krate) => format!(
                " (current version <code>{}</code> {})",
                krate.id.vers,
                krate.html_links()
            ),
            Err(_) => String::new(),
        };

        Ok(Some(ver))
    } else {
        Ok(None)
    }
}

// why aren't we in an FP lang? :(
fn with<A, B, C, U>(ctx: C, f: impl Fn(A, B, C) -> U) -> impl Fn(A, B) -> U
where
    C: Clone,
{
    move |a, b| f(a, b, ctx.clone())
}

fn opt(input: String) -> Result<(Option<String>,), ParseError> {
    match dbg!(input.split_whitespace().count()) {
        0 => Ok((None,)),
        1 => dbg!(Ok((Some(input),))),
        n => Err(ParseError::TooManyArguments {
            expected: 1,
            found: n,
            message: String::from("Wrong number of arguments"),
        }),
    }
}
