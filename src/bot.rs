use std::{fmt::Debug, path::Path, sync::Arc};

use futures::{future, Future, FutureExt};
use log::{error, warn};
use teloxide::{
    prelude::{Requester, *},
    types::{Me, Message},
    utils::command::{BotCommand, ParseError},
    RequestError,
};
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::{cfg::Config, db::Database, krate::Crate, util::crate_path, Bot, VERSION};

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

fn opt(input: String) -> Result<(Option<String>,), ParseError> {
    match input.split_whitespace().count() {
        0 => Ok((None,)),
        1 => Ok((Some(input),)),
        n => Err(ParseError::TooManyArguments {
            expected: 1,
            found: n,
            message: String::from("Wrong number of arguments"),
        }),
    }
}

#[derive(Debug, derive_more::Display, derive_more::From, derive_more::Error)]
enum HErr {
    Tg(RequestError),
    Bd(tokio_postgres::Error),
    GetUser,
    NotAdmin,
}

async fn commands_handler(
    (update_with_cx, cmd): (UpdateWithCx<Bot, Message>, Command),
    (db, cfg): (Database, Arc<Config>),
) -> Result<(), HErr> {
    let UpdateWithCx {
            update: msg,
            requester: bot,
    } = update_with_cx;
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
            let subscriptions = list_subscriptions(chat_id, &db, &cfg).await?;

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
}

async fn unblock_handler(
    update_with_cx: UpdateWithCx<Bot, ChatMemberUpdated>,
    (db, _cfg): (Database, Arc<Config>),
) -> Result<(), HErr> {
    let UpdateWithCx {
                       update,
                       requester: bot,
    } = update_with_cx;

        let ChatMemberUpdated {
            from,
            old_chat_member,
            new_chat_member,
            ..
        } = &update;

        if old_chat_member.is_present() && !new_chat_member.is_present() {
            // FIXME: ideally the bot should just mark the user as temporary unavailable
            // (that is: untill unblock/restart), but I'm too lazy to implement it rn.
            for sub in db.list_subscriptions(from.id).await? {
                db.unsubscribe(from.id, &sub).await?;
            }
        } else if !old_chat_member.is_present() && new_chat_member.is_present() {
            bot.send_message(
                from.id,
                "You have previously blocked this bot. This removed all your subsctiptions.",
            )
            .await?;
        } else {
        warn!("Got weird MyChatMember update: {:?}", update);
        }

        Ok::<_, HErr>(())
}

pub async fn run(bot: Bot, db: Database, cfg: Arc<Config>) {
    let Me { user, .. } = bot.get_me().await.expect("Couldn't get myself :(");
    let name = user.username.expect("Bots *must* have usernames");

    let ctx_cloned = (db.clone(), cfg.clone());
    let ctx = (db, cfg);

    let mut dp = Dispatcher::new(bot)
        .messages_handler(move |rx| async move {
            UnboundedReceiverStream::new(rx)
                .commands(name)
                .for_each_concurrent(None, err(with(ctx_cloned, commands_handler)))
                .await
        })
        .my_chat_members_handler(move |rx| async move {
            UnboundedReceiverStream::new(rx)
                .for_each_concurrent(None, err(with(ctx, unblock_handler)))
                .await
        })
        .setup_ctrlc_handler();

    dp.dispatch().await;
}

async fn list_subscriptions(
    chat_id: i64,
    db: &Database,
    cfg: &Config,
) -> Result<Vec<String>, HErr> {
    let mut subscriptions: Vec<_> = db.list_subscriptions(chat_id).await?.collect();
    for sub in &mut subscriptions {
        let path = Path::new(cfg.index_path.as_str()).join(crate_path(sub));
        match Crate::read_last(&path).await {
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
        let user_id = msg.from().ok_or(HErr::GetUser)?.id;

        let admins = bot.get_chat_administrators(msg.chat_id()).await?;
        if !admins.into_iter().any(|admin| admin.user.id == user_id) {
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
    let path = Path::new(cfg.index_path.as_str()).join(crate_path(krate));
    if path.exists() {
        db.subscribe(chat_id, krate).await?;

        let ver = match Crate::read_last(&path).await {
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
fn with<A, B, U>(ctx: B, f: impl Fn(A, B) -> U) -> impl Fn(A) -> U
where
    B: Clone,
{
    move |a| f(a, ctx.clone())
}

/// Process errors (log)
fn err<T, E, F>(f: impl Fn(T) -> F) -> impl Fn(T) -> future::Map<F, fn(Result<(), E>) -> ()>
where
    F: Future<Output = Result<(), E>>,
    E: Debug,
{
    move |x| {
        f(x).map(|r| {
            if let Err(err) = r {
                error!("Error in handler: {:?}", err);
            }
        })
    }
}
