use crate::{
    db::Database,
    util::{crate_path, tryn}
};
use carapax::{
    types::{Command, ParseMode},
    methods::SendMessage,
    Api,
    Dispatcher,
    ExecuteError,
    Handler,
    longpoll::LongPoll
};
use std::{
    time::Duration,
    future::Future,
    path::PathBuf,
    pin::Pin
};
use fntools::value::ValueExt;
use crate::cfg::RetryDelay;

pub fn setup(bot: Api, db: Database, retry_delay: RetryDelay) -> LongPoll<Dispatcher<(Api, Database, RetryDelay)>> {
    let mut dp = Dispatcher::new((bot.clone(), db, retry_delay));
    dp.add_handler(Handlers);
    LongPoll::new(bot, dp) // TODO: allowed_update
}

struct Handlers;

#[derive(Debug, derive_more::Display, derive_more::From, derive_more::Error)]
enum HErr {
    Tg(ExecuteError),
    Bd(tokio_postgres::Error),
    GetUser,
}

impl Handler<(Api, Database, RetryDelay)> for Handlers {
    type Input = Command;
    type Output = Result<(), HErr>;

    fn handle<'s: 'async_trait, 'a: 'async_trait, 'async_trait>(
        &'s mut self,
        context: &'a (Api, Database, RetryDelay),
        input: Self::Input,
    ) -> Pin<Box<dyn Future<Output = Self::Output> + Send + 'async_trait>> {
        async fn handle_(
            _: &mut Handlers,
            (bot, db, retry_delay): &(Api, Database, RetryDelay),
            command: Command,
        ) -> Result<(), HErr> {
            let chat_id = command.get_message().get_user().ok_or(HErr::GetUser)?.id;
            match command.get_name() {
                "/start" => {
                    tryn(5, Duration::from_millis(10000 /* 10 secs */), || {
                        bot.execute(
                            SendMessage::new(chat_id, "Greeting message") // TODO
                                .parse_mode(ParseMode::Html),
                        )
                    })
                    .await?;
                }
                "/subscribe" => {
                    match command.get_args() {
                        [krate, ..] => {
                            if PathBuf::from("./index")
                                .also(|p| p.push(crate_path(krate)))
                                .exists()
                            {
                                db.subscribe(chat_id, krate).await?;
                                tryn(5, retry_delay.0, || bot.execute(
                                    SendMessage::new(
                                        chat_id,
                                        format!("You've successfully subscribed for updates on <code>{}</code> crate. Use /unsubscribe to unsubscribe.", krate))
                                        .parse_mode(ParseMode::Html)
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
                    }
                }
                "/unsubscribe" => {
                    match command.get_args() {
                        [krate, ..] => {
                            db.unsubscribe(chat_id, krate).await?;
                            tryn(5, retry_delay.0, || bot.execute(
                                SendMessage::new(
                                    chat_id,
                                    format!("You've successfully unsubscribed for updates on <pre>{}</pre> crate. Use /subscribe to subscribe back.", krate))
                                    .parse_mode(ParseMode::Html)
                            )).await?;
                        }
                        [] => {
                            tryn(5, retry_delay.0, || bot.execute(
                                SendMessage::new(chat_id, "You need to specify the crate you want to unsubscribe. Like this: <code>/unsubscribe serde</code>")
                                    .parse_mode(ParseMode::Html)
                            )).await?;
                        }
                    }
                }
                "/list" => {
                    let subscriptions = db.list_subscriptions(chat_id).await?;

                    if subscriptions.is_empty() {
                        tryn(5, retry_delay.0, || bot.execute(
                            SendMessage::new(
                                chat_id,
                                format!("Currently you aren't subscribed to anything. Use /subscribe to subscribe to some crate."))
                                .parse_mode(ParseMode::Html)
                        )).await?;
                    } else {
                        tryn(5, retry_delay.0, || {
                            bot.execute(
                                SendMessage::new(
                                    chat_id,
                                    format!(
                                        "You are currently subscribed to:\n— <code>{}</code>",
                                        subscriptions.join("</code>\n— <code>")
                                    ),
                                )
                                .parse_mode(ParseMode::Html),
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
