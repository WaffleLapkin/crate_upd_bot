## crate_upd_bot

<!--
[![CI status](https://github.com/WaffleLapkin/crate_upd_bot/workflows/Continuous%20integration/badge.svg)](https://github.com/WaffleLapkin/crate_upd_bot/actions)
-->
[![Telegram (bot)](https://img.shields.io/badge/bot-@crates_upd_bot-9cf?logo=telegram)](https://t.me/crates_upd_bot)
[![LICENSE](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)

Telegram bot that notifies about crate updates (new versions, yanked versions and unyanked versions).

The bot is hosted by [me] under [@crates_upd_bot][bot-nick] nickname in telegram. Feel free to use it ;)

If you want to see **all** updates, subscribe to [@crates_updates][all] (warning: there are a **lot** of updates every day) 

[me]: https://t.me/wafflelapkin
[bot-nick]: https://t.me/crates_upd_bot 
[all]: https://t.me/crates_updates

## Bot interface

The bot supports 3 straightforward commands:
- `/subscribe <crate>` — subscribe for `<crate>` updates (bot will notify you in PM)
- `/unsubscribe <crate>` — unsubscribe for `<crate>` updates
- `/list` — list your current subscriptions

## How it works

Every `pull_delay` (default to 5 min) the bot fetches changes from [`crates.io-index`][index-repo] repo, walk through 
all commits, parses diffs & notifies users.

[index-repo]: https://github.com/rust-lang/crates.io-index.git

## State of the project

It's not my main project, so I don't spend much time on it. The code is pretty weird & 
needs a lot of improvements, though it _seems to work_.

All contributions are appreciated.

## Deployment

1. Create a `postgresql` database. It will store user subscriptions.
1. Execute [`db.sql`](./db.sql) in the database.
1. build the bot
   ```console
   cargo build --release
   ```
1. Edit [`config.toml`](./config.toml). You must set `bot_token` and `db.{host,user,dbname}` though you may set other setting too.
1. Run the binary created in (3). (`target/release/crate_upd_bot`)

(probably it would be better to create a docker image & setup auto deploy, maybe some day....)  


