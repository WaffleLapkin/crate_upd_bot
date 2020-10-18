use fntools::value::ValueExt;
use std::{collections::HashSet, error::Error, fs::File, io::Read, time::Duration};

#[derive(Debug, serde::Deserialize)]
pub struct Config {
    /// Channel to post **ALL** updates
    #[serde(default)]
    pub channel: Option<i64>,
    /// Delay between index fetches
    #[serde(default = "defaults::pull_delay")]
    pub pull_delay: Duration,
    /// Logging level
    #[serde(default = "defaults::loglevel")]
    pub loglevel: log::LevelFilter,
    /// Url of crates.io index (git repo)
    #[serde(default = "defaults::index_url")]
    pub index_url: String,
    /// The path to the local crates.io index git repository
    #[serde(default = "defaults::index_path")]
    pub index_path: String,
    /// Delay after which bot will retry telegram-request
    #[serde(default)]
    pub retry_delay: RetryDelay,
    /// Delay between broadcast send messages
    #[serde(default)]
    pub broadcast_delay_millis: BroadcastDelay,
    /// Delay between notifying about updates
    #[serde(default)]
    pub update_delay_millis: UpdateDelay,
    /// Token of the telegram bot
    pub bot_token: String,
    /// Database configuration
    pub db: DbConfig,
    /// Ban configuration
    #[serde(default)]
    pub ban: BanConfig,
}

impl Config {
    pub fn read() -> Result<Self, Box<dyn Error>> {
        let mut str = String::new();
        File::open("./config.toml")?.read_to_string(&mut str)?;
        Ok(toml::from_str(&str)?)
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct DbConfig {
    pub host: String,
    pub user: String,
    pub dbname: String,
}

impl DbConfig {
    pub fn cfg(&self) -> tokio_postgres::Config {
        tokio_postgres::Config::new().also(|cfg| {
            cfg.host(&self.host).user(&self.user).dbname(&self.dbname);
        })
    }
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct BanConfig {
    /// Names of banned crates (they won't show up in the channel)
    #[serde(default)]
    pub crates: HashSet<String>,
}

#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(transparent)]
pub struct BroadcastDelay {
    pub millis: u64,
}

impl Default for BroadcastDelay {
    fn default() -> Self {
        Self { millis: 250 } // quoter of a sec
    }
}

impl From<BroadcastDelay> for Duration {
    fn from(bd: BroadcastDelay) -> Self {
        Duration::from_millis(bd.millis)
    }
}

#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(transparent)]
pub struct UpdateDelay {
    pub millis: u64,
}

impl Default for UpdateDelay {
    fn default() -> Self {
        Self { millis: 1300 } // 1.3s
    }
}

impl From<UpdateDelay> for Duration {
    fn from(ud: UpdateDelay) -> Self {
        Duration::from_millis(ud.millis)
    }
}

#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(transparent)]
pub struct RetryDelay(pub Duration);

impl Default for RetryDelay {
    fn default() -> Self {
        Self(Duration::from_secs(10)) // 10 secs
    }
}

mod defaults {
    use std::time::Duration;

    pub(super) const fn pull_delay() -> Duration {
        Duration::from_secs(60 * 5) // 5 min
    }

    pub(super) const fn loglevel() -> log::LevelFilter {
        log::LevelFilter::Info
    }

    pub(super) fn index_url() -> String {
        String::from("https://github.com/rust-lang/crates.io-index.git")
    }

    pub(super) fn index_path() -> String {
        String::from("./index")
    }
}
