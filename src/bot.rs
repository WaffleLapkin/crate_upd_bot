use std::{fmt::Debug, ops::Not, path::PathBuf, sync::Arc};

use fntools::value::ValueExt;
use teloxide::{
    dispatching::UpdateFilterExt,
    dptree::deps,
    prelude::{Requester, *},
    utils::command::{BotCommands, ParseError},
    RequestError,
};

use crate::{cfg::Config, db::Database, krate::Crate, util::crate_path, Bot, VERSION};

type OptString = Option<String>;

#[derive(BotCommands, Clone, PartialEq, Eq, Debug)]
#[command(rename_rule = "lowercase", parse_with = "split")]
enum Command {
    Start,
    #[command(parse_with = opt)]
    Subscribe(OptString),
    #[command(parse_with = opt)]
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
    let commands = |bot: Bot, msg: Message, cmd: Command, db: Database, cfg: Arc<Config>| async move {
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
                     Its channel [ru]: @ihatereality\n\
                     My source: <a href='https://github.com/WaffleLapkin/crate_upd_bot'>[github]</a>\n\
                     Version: <code>{VERSION}</code>"
                );
                bot.send_message(chat_id, greeting).await?;
            }
            Command::Subscribe(Some(krate)) => match subscribe(chat_id, &krate, &db, &cfg).await? {
                Some(ver) => {
                    bot.send_message(
                        chat_id,
                        format!(
                            "You've successfully subscribed for updates on <code>{krate}</code>{ver} \
                             crate. Use /unsubscribe to unsubscribe."
                        ),
                    )
                    .disable_web_page_preview(true)
                    .await?;
                }
                None => {
                    bot.send_message(
                        chat_id,
                        format!("Error: there is no such crate <code>{krate}</code>."),
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
                        "You've successfully unsubscribed for updates on <code>{krate}</code> crate. \
                         Use /subscribe to subscribe back."
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

    let unblock = |bot: Bot, update: ChatMemberUpdated, db: Database| async move {
        let ChatMemberUpdated {
            chat,
            old_chat_member,
            new_chat_member,
            ..
        } = &update;
        if old_chat_member.is_present() && !new_chat_member.is_present() {
            // FIXME: ideally the bot should just mark the user as temporary unavailable
            // (that is: until unblock/restart), but I'm too lazy to implement it rn.
            for sub in db.list_subscriptions(chat.id).await? {
                db.unsubscribe(chat.id, &sub).await?;
            }
        } else if !old_chat_member.is_present() && new_chat_member.is_present() {
            // Do not trigger when the bot is added to a group
            //
            // FIXME: when we'll store bot bannedness in DB, this should check that the bot
            // was previously blocked instead
            if chat.is_private() {
                bot.send_message(
                    chat.id,
                    "You have previously blocked this bot. This removed all your subscriptions.",
                )
                .await?;
            }
        } else {
            log::warn!("Got weird MyChatMember update: {:?}", update);
        }

        Ok::<_, HErr>(())
    };

    let handler = dptree::entry()
        .branch(
            Update::filter_message()
                .filter_command::<Command>()
                .endpoint(commands),
        )
        .branch(Update::filter_my_chat_member().endpoint(unblock));

    Dispatcher::builder(bot, handler)
        .dependencies(deps![db, cfg])
        .default_handler(|_| async {})
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}

async fn list(chat_id: ChatId, db: &Database, cfg: &Config) -> Result<Vec<String>, HErr> {
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
        let admins = bot.get_chat_administrators(msg.chat.id).await?;

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
    chat_id: ChatId,
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

fn opt(input: String) -> Result<(Option<String>,), ParseError> {
    match input.split_whitespace().count() {
        0 => Ok((None,)),
        1 => Ok((Some(input.trim().to_owned()),)),
        n => Err(ParseError::TooManyArguments {
            expected: 1,
            found: n,
            message: String::from("Wrong number of arguments"),
        }),
    }
}
