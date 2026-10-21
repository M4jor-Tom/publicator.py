use std::path::PathBuf;

pub(super) async fn prepare(
    source: PathBuf,
    sha512: String,
    cache: PathBuf,
) -> Result<PathBuf, String> {
    tokio::task::spawn_blocking(move || {
        let name = crate::images::thumb_name(&source, &sha512)?;
        crate::images::ensure_thumb(&source, &cache, &name)
    })
    .await
    .unwrap_or_else(|error| Err(crate::error::Error::Config(error.to_string())))
    .map_err(|error| error.to_string())
}
