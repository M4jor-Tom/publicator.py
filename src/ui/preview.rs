use std::path::PathBuf;

pub(super) async fn prepare(
    source: PathBuf,
    sha512: String,
    cache: PathBuf,
) -> Result<PathBuf, String> {
    prepare_lazy(source, Some(sha512), cache).await
}

pub(super) async fn prepare_lazy(
    source: PathBuf,
    sha512: Option<String>,
    cache: PathBuf,
) -> Result<PathBuf, String> {
    tokio::task::spawn_blocking(move || {
        let sha = sha512
            .filter(|sha| sha.len() == 128 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .map(Ok)
            .unwrap_or_else(|| crate::images::file_sha512(&source))?;
        let name = crate::images::thumb_name(&source, &sha)?;
        crate::images::ensure_thumb(&source, &cache, &name)
    })
    .await
    .unwrap_or_else(|error| Err(crate::error::Error::Config(error.to_string())))
    .map_err(|error| error.to_string())
}
