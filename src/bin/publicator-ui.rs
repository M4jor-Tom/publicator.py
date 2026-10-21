use clap::Parser;
use publicator::{
    application::Application,
    config::{Config, Paths},
    metadata::{Endpoints, MetadataClient},
    store::Store,
    ui::UiOptions,
};
use std::{path::PathBuf, sync::Arc, time::Duration};

#[derive(Parser)]
#[command(name = "publicator-ui")]
struct Args {
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[arg(long)]
    json: Option<PathBuf>,
    #[arg(long)]
    openrouter_model: Option<String>,
    #[arg(long, default_value_t = 300)]
    ai_timeout: u64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let paths = Paths::resolve(args.data_dir.as_deref(), args.json.as_deref())?;
    let config = Config::load(&paths.data_dir)?;
    let store = Arc::new(Store::new(paths, config)?);
    let mut endpoints = Endpoints::default();
    if let Some(ollama) = std::env::var_os("PUBLICATOR_OLLAMA_URL") {
        endpoints.ollama = ollama.to_string_lossy().into_owned();
    }
    let metadata = MetadataClient::new(reqwest::Client::new(), endpoints, PathBuf::from("llmfit"));
    let application = Arc::new(Application::new(store, metadata, unix_now));
    Ok(publicator::ui::run(
        application,
        UiOptions {
            openrouter_model: args.openrouter_model,
            ai_timeout: Duration::from_secs(args.ai_timeout),
        },
    )?)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
