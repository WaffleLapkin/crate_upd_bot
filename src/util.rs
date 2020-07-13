use std::{
    future::Future,
    path::{Path, PathBuf}
};
use tokio::time::{delay_for, Duration};

/// Path to crate file in crates.io-index. Implementation is stolen from
/// https://github.com/rust-lang/crates.io/blob/06bfd00ca4c2fce1e9c674d0d792a5ca56d32350/src/git.rs#L179-L187
pub fn crate_path(name: &str) -> PathBuf {
    let name = name.to_lowercase();
    match name.len() {
        1 => Path::new("1").join(&name),
        2 => Path::new("2").join(&name),
        3 => Path::new("3").join(&name[..1]).join(&name),
        _ => Path::new(&name[0..2]).join(&name[2..4]).join(&name),
    }
}

macro_rules! tryok {
    ($e:expr) => {
        match $e {
            Ok(ok) => return Ok(ok),
            Err(err) => err,
        }
    };
}

pub async fn tryn<F, Fut, T, E>(n: usize, del: Duration, mut f: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut err = tryok!(f().await);
    for _ in 0..n {
        delay_for(del).await;
        err = tryok!(f().await);
    }
    Err(err)
}
