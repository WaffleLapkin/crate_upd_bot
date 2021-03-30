use std::{future::Future, path::PathBuf, pin::Pin, sync::Arc, time::Duration};

use carapax::{
    longpoll::LongPoll,
    methods::GetChatAdministrators,
    methods::SendMessage,
    types::MessageKind,
    types::{Command, ParseMode},
    Api, Dispatcher, ExecuteError, Handler,
};
use fntools::value::ValueExt;

use crate::{
    cfg::Config,
    db::Database,
    krate::Crate,
    util::{crate_path, tryn},
    VERSION,
};

pub fn setup(
    bot: Api,
    db: Database,
    cfg: Arc<Config>,
) -> LongPoll<Dispatcher<(Api, Database, Arc<Config>)>> {
    let mut dp = Dispatcher::new((bot.clone(), db, cfg));
    dp.add_handler(Handlers);
    LongPoll::new(bot, dp) // TODO: allowed_update
}

struct Handlers;

#[derive(Debug, derive_more::Display, derive_more::From, derive_more::Error)]
enum HErr {
    Tg(ExecuteError),
    Bd(tokio_postgres::Error),
    GetUser,
    NotAdmin,
}

impl Handler<(Api, Database, Arc<Config>)> for Handlers {
    type Input = Command;
    type Output = Result<(), HErr>;

    fn handle<'s: 'async_trait, 'a: 'async_trait, 'async_trait>(
        &'s mut self,
        context: &'a (Api, Database, Arc<Config>),
        input: Self::Input,
    ) -> Pin<Box<dyn Future<Output = Self::Output> + Send + 'async_trait>> {
        async fn handle_(
            _: &mut Handlers,
            (bot, db, cfg): &(Api, Database, Arc<Config>),
            command: Command,
        ) -> Result<(), HErr> {
            let retry_delay = &cfg.retry_delay;
            let msg = command.get_message();
            let chat_id = match &msg.kind {
                MessageKind::Private { from, .. } => from.id,
                _ => {
                    let admins = tryn(5, Duration::from_millis(10000 /* 10 secs */), || {
                        bot.execute(GetChatAdministrators::new(msg.get_chat_id()))
                    })
                    .await?;

                    let user_id = msg.get_user().ok_or(HErr::GetUser)?.id;
                    if admins
                        .iter()
                        .map(|admin| admin.get_user().id)
                        .any(|id| id == user_id)
                    {
                        msg.get_chat_id()
                    } else {
                        return Err(HErr::NotAdmin);
                    }
                }
            };
            match command.get_name() {
                "/start" => {
                    tryn(5, Duration::from_millis(10000 /* 10 secs */), || {
                        bot.execute(
                            SendMessage::new(chat_id, format!("Hi! I will notify you about updates of crates. Use /subscribe to subscribe for updates of crates you want to be notified about.\n\nIn case you want to see <b>all</b> updates go to @crates_updates\n\nAuthor: @wafflelapkin\nHis channel [ru]: @ihatereality\nMy source: <a href='https://github.com/WaffleLapkin/crate_upd_bot'>[github]</a>\nVersion: <code>{version}</code>", version = VERSION))
                                .parse_mode(ParseMode::Html),
                        )
                    })
                    .await?;
                }
                "/subscribe" => match command.get_args() {
                    [krate, ..] => {
                        if PathBuf::from(cfg.index_path.as_str())
                            .also(|p| p.push(crate_path(krate)))
                            .exists()
                        {
                            db.subscribe(chat_id, krate).await?;
                            let v = match Crate::read_last(krate, cfg).await {
                                Ok(krate) => format!(
                                    " (current version <code>{}</code> {})",
                                    krate.id.vers,
                                    krate.html_links()
                                ),
                                Err(_) => String::new(),
                            };
                            tryn(5, retry_delay.0, || bot.execute(
                                    SendMessage::new(
                                        chat_id,
                                        format!("You've successfully subscribed for updates on <code>{}</code>{} crate. Use /unsubscribe to unsubscribe.", krate, v))
                                        .parse_mode(ParseMode::Html)
                                        .disable_web_page_preview(true)
                                )).await?;
                        } else {
                            tryn(5, retry_delay.0, || {
                                bot.execute(
                                    SendMessage::new(
                                        chat_id,
                                        format!(
                                            "Error: there is no such crate <code>{}</code>.",
                                            krate
                                        ),
                                    )
                                    .parse_mode(ParseMode::Html),
                                )
                            })
                            .await?;
                        }
                    }
                    [] => {
                        tryn(5, retry_delay.0, || bot.execute(
                                SendMessage::new(chat_id, "You need to specify the crate you want to subscribe. Like this: <pre>/subscribe serde</pre>")
                                    .parse_mode(ParseMode::Html)
                            )).await?;
                    }
                },
                "/unsubscribe" => match command.get_args() {
                    [krate, ..] => {
                        db.unsubscribe(chat_id, krate).await?;
                        tryn(5, retry_delay.0, || bot.execute(
                                SendMessage::new(
                                    chat_id,
                                    format!("You've successfully unsubscribed for updates on <code>{}</code> crate. Use /subscribe to subscribe back.", krate))
                                    .parse_mode(ParseMode::Html)
                            )).await?;
                    }
                    [] => {
                        tryn(5, retry_delay.0, || bot.execute(
                                SendMessage::new(chat_id, "You need to specify the crate you want to unsubscribe. Like this: <code>/unsubscribe serde</code>")
                                    .parse_mode(ParseMode::Html)
                            )).await?;
                    }
                },
                "/list" => {
                    let mut subscriptions = db.list_subscriptions(chat_id).await?;
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

                    if subscriptions.is_empty() {
                        tryn(5, retry_delay.0, || bot.execute(
                            SendMessage::new(
                                chat_id,
                                String::from("Currently you aren't subscribed to anything. Use /subscribe to subscribe to some crate."))
                                .parse_mode(ParseMode::Html)
                        )).await?;
                    } else {
                        tryn(5, retry_delay.0, || {
                            bot.execute(
                                SendMessage::new(
                                    chat_id,
                                    format!(
                                        "You are currently subscribed to:\n— <code>{}",
                                        subscriptions.join("\n— <code>")
                                    ),
                                )
                                .parse_mode(ParseMode::Html)
                                .disable_web_page_preview(true),
                            )
                        })
                        .await?;
                    }
                }
                _ => {}
            }

            Ok(())
        }

        Box::pin(handle_(self, context, input))
    }
}
