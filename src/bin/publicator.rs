use clap::{Parser, Subcommand};
use publicator::{
    config::{Config, Paths, validate_publications},
    error::{Error, Result},
    model::Publications,
};
use std::{fs, path::PathBuf};

#[derive(Parser)]
#[command(name = "publicator")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Validate {
        #[arg(long)]
        data_dir: Option<PathBuf>,
        #[arg(long)]
        json: Option<PathBuf>,
    },
}
fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Validate { data_dir, json } => {
            let paths = Paths::resolve(data_dir.as_deref(), json.as_deref())?;
            let config = Config::load(&paths.data_dir)?;
            let bytes = fs::read(&paths.json).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Error::NotFound(paths.json.display().to_string())
                } else {
                    e.into()
                }
            })?;
            let rows: Publications = serde_json::from_slice(&bytes)?;
            validate_publications(&rows, &config)?;
            println!("Valid");
        }
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
