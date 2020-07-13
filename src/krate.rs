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
}
