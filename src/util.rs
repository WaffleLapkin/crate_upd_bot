use std::{
    future::IntoFuture,
    path::{Path, PathBuf},
    time::Duration,
};

/// Path to crate file in crates.io-index.
///
/// Implementation is stolen from
/// <https://github.com/rust-lang/crates.io/blob/06bfd00ca4c2fce1e9c674d0d792a5ca56d32350/src/git.rs#L179-L187>
pub fn crate_path(name: &str) -> PathBuf {
    let name = name.to_lowercase();
    match name.len() {
        1 => Path::new("1").join(&name),
        2 => Path::new("2").join(&name),
        3 => Path::new("3").join(&name[..1]).join(&name),
        _ => Path::new(&name[0..2]).join(&name[2..4]).join(&name),
    }
}

/// Try executing async function `f`. On error delay for `delay`. If after `n`
/// tries `f` still fails, return last error.
pub async fn tryn<F, Fut, T, E>(n: usize, delay: Duration, mut f: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: IntoFuture<Output = Result<T, E>>,
{
    for _ in 1..n {
        if let ret @ Ok(_) = f().await {
            return ret;
        }

        tokio::time::sleep(delay).await;
    }

    f().await
}
