use crate::{cfg::Config, util::crate_path};
use std::path::Path;
use tokio::{
    fs::File,
    io,
    io::{AsyncBufReadExt, BufReader},
};

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Crate {
    // TODO: stole from crates.io repo?
    #[serde(flatten)]
    pub id: CrateId,
    pub yanked: bool,
    // ignore all unrelated stuff :D
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, serde::Serialize, serde::Deserialize)]
pub struct CrateId {
    pub name: String,
    pub vers: String,
}

impl Crate {
    // TODO: struct: Display

    pub fn cratesio(&self) -> String {
        format!("https://crates.io/crates/{krate}", krate = self.id.name)
    }

    pub fn librs(&self) -> String {
        format!("https://lib.rs/crates/{krate}", krate = self.id.name)
    }

    pub fn docsrs(&self) -> String {
        // Note:
        // The full url is actually "https://docs.rs/{krate}/{version}/{krate}"
        // but for some crates it doesn't hold e.g.: https://docs.rs/lsk/0.2.0/ls_key/
        // Names differ                                              ^^^       ^^^^^^
        //
        // Anyway, "https://docs.rs/{krate}/{version}" redirects to the right place
        format!(
            "https://docs.rs/{krate}/{version}",
            krate = self.id.name,
            version = self.id.vers,
        )
    }

    pub fn html_links(&self) -> String {
        format!(
            "<a href='{docs}'>[docs.rs]</a> <a href='{crates}'>[crates.io]</a> <a \
             href='{lib}'>[lib.rs]</a>",
            docs = self.docsrs(),
            crates = self.cratesio(),
            lib = self.librs(),
        )
    }

    pub async fn read_last(name: &str, cfg: &Config) -> io::Result<Self> {
        let file = File::open(Path::new(cfg.index_path.as_str()).join(crate_path(name))).await?;
        let mut lines = BufReader::new(file).lines();
        let mut last = None;
        while let next @ Some(_) = lines.next_line().await? {
            last = next
        }
        serde_json::from_str(&last.unwrap())
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))
    }
}
