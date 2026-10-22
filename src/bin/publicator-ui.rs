use clap::Parser;
use publicator::{
    application::Application,
    config::{Config, Paths},
    metadata::{Endpoints, MetadataClient},
    publishing::Publisher,
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

fn gui_metadata_endpoints() -> Endpoints {
    Endpoints::default()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let paths = Paths::resolve(args.data_dir.as_deref(), args.json.as_deref())?;
    let config = Config::load(&paths.data_dir)?;
    let store = Arc::new(Store::new(paths, config)?);
    let metadata = MetadataClient::new(
        reqwest::Client::new(),
        gui_metadata_endpoints(),
        PathBuf::from("llmfit"),
    );
    let worker = std::env::var_os("PUBLICATOR_WORKER")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("python/publicator_da/worker.py"));
    let skill = std::env::var_os("PUBLICATOR_SKILL")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".claude/skills/publish-deviantart/SKILL.md"));
    let application =
        Arc::new(Application::new(store, metadata).with_publisher(Publisher::new(worker, skill)));
    Ok(publicator::ui::run(
        application,
        UiOptions {
            openrouter_model: args.openrouter_model,
            ai_timeout: Duration::from_secs(args.ai_timeout),
        },
    )?)
}

#[cfg(test)]
mod tests {
    use super::gui_metadata_endpoints;

    #[test]
    fn gui_ignores_inherited_cli_ollama_endpoint_override() {
        let previous = std::env::var_os("PUBLICATOR_OLLAMA_URL");
        unsafe {
            std::env::set_var("PUBLICATOR_OLLAMA_URL", "http://127.0.0.1:1");
        }
        let ollama = gui_metadata_endpoints().ollama;
        unsafe {
            if let Some(previous) = previous {
                std::env::set_var("PUBLICATOR_OLLAMA_URL", previous);
            } else {
                std::env::remove_var("PUBLICATOR_OLLAMA_URL");
            }
        }
        assert_eq!(ollama, "http://localhost:11434");
    }
}
