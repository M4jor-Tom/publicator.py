use clap::{Args, Parser, Subcommand, ValueEnum};
use publicator::{
    application::Application,
    config::{Config, Paths},
    error::{Error, Result},
    metadata::{
        Endpoints, GenerationRequest, MetadataClient, ModelOption, Source, VISION_MODELS,
        openrouter_model,
    },
    model::{GENERATE_MARKER, Phase, phase},
    prompts::{PromptArchive, audit},
    publish_journal::{self, PublishJournal, ReconcileOutcome},
    publishing::{PublishReport, PublishRequest, Publisher, first_pending_uuid},
    store::Store,
};
use std::{
    io::{self, IsTerminal},
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

#[derive(Parser)]
#[command(name = "publicator", version)]
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
    EchoFirst {
        #[arg(long)]
        data_dir: Option<PathBuf>,
        #[arg(long)]
        json: Option<PathBuf>,
    },
    GenerateMeta(GenerateMetaArgs),
    VisionModels,
    PromptAudit {
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    DaPublish(DaPublishArgs),
    /// Delete an undelivered publication entry; its artwork becomes a Pick candidate again.
    DeletePublication {
        #[arg(long)]
        uuid: String,
        #[arg(long)]
        data_dir: Option<PathBuf>,
        #[arg(long)]
        json: Option<PathBuf>,
    },
    /// Move an undelivered publication back one state: DESCRIBED to SCHEDULED, SCHEDULED to PICKED.
    RollBack {
        #[arg(long)]
        uuid: String,
        #[arg(long)]
        data_dir: Option<PathBuf>,
        #[arg(long)]
        json: Option<PathBuf>,
    },
    CheckSteps,
}
#[derive(Args)]
struct GenerateMetaArgs {
    #[arg(long, value_enum)]
    source: SourceArg,
    #[arg(long)]
    model: Option<String>,
    #[arg(long, default_value_t = 300)]
    timeout: u64,
    #[arg(long, short = 'y')]
    yes: bool,
    #[arg(long, short)]
    verbose: bool,
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[arg(long)]
    json: Option<PathBuf>,
}
#[derive(Args)]
struct DaPublishArgs {
    #[arg(long, conflicts_with_all = ["all", "reconcile_attempt"])]
    uuid: Option<String>,
    #[arg(long, conflicts_with = "reconcile_attempt")]
    all: bool,
    #[arg(long, conflicts_with_all = ["all", "uuid", "check_steps"]) ]
    reconcile_attempt: Option<String>,
    #[arg(long, value_enum, requires = "reconcile_attempt")]
    outcome: Option<OutcomeArg>,
    #[arg(long, requires = "reconcile_attempt")]
    url: Option<String>,
    #[arg(long, conflicts_with = "reconcile_attempt")]
    headless: bool,
    #[arg(long, conflicts_with = "reconcile_attempt")]
    check_steps: bool,
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[arg(long)]
    json: Option<PathBuf>,
    #[arg(long, short)]
    verbose: bool,
}
#[derive(Clone, Copy, ValueEnum)]
enum SourceArg {
    Openrouter,
    Local,
}
#[derive(Clone, Copy, ValueEnum)]
enum OutcomeArg {
    Scheduled,
    NotSubmitted,
}
impl From<SourceArg> for Source {
    fn from(v: SourceArg) -> Self {
        match v {
            SourceArg::Openrouter => Self::OpenRouter,
            SourceArg::Local => Self::Local,
        }
    }
}

fn paths(data_dir: Option<PathBuf>, json: Option<PathBuf>) -> Result<(Paths, Config, Arc<Store>)> {
    let paths = Paths::resolve(data_dir.as_deref(), json.as_deref())?;
    let config = Config::load(&paths.data_dir)?;
    let store = Arc::new(Store::new(paths.clone(), config.clone())?);
    Ok((paths, config, store))
}
fn metadata_client(data_dir: Option<&Path>) -> Result<MetadataClient> {
    let mut endpoints = Endpoints::default();
    if let Some(ollama) = std::env::var_os("PUBLICATOR_OLLAMA_URL") {
        endpoints.ollama = ollama.to_string_lossy().into_owned();
    }
    let client = MetadataClient::new(reqwest::Client::new(), endpoints, PathBuf::from("llmfit"));
    Ok(
        match data_dir
            .map(publicator::config::openrouter_key)
            .transpose()?
            .flatten()
        {
            Some(key) => client.with_openrouter_key(key),
            None => client,
        },
    )
}
fn choose_model(source: Source, requested: Option<&str>) -> Result<ModelOption> {
    match source {
        Source::OpenRouter => {
            openrouter_model(requested.unwrap_or("openrouter/google/gemma-4-26b-a4b-it:free"))
                .map_err(Error::Validation)
        }
        Source::Local => VISION_MODELS
            .iter()
            .find(|m| Some(m.ollama_name.as_str()) == requested)
            .cloned()
            .ok_or_else(|| {
                Error::Validation("local generation requires a catalogue --model".into())
            }),
    }
}
async fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Validate { data_dir, json } => validate(data_dir, json),
        Command::EchoFirst { data_dir, json } => echo_first(data_dir, json),
        Command::GenerateMeta(args) => generate_meta(args).await,
        Command::VisionModels => vision_models().await,
        Command::PromptAudit { data_dir } => prompt_audit(data_dir),
        Command::DaPublish(args) => da_publish(args).await,
        Command::DeletePublication {
            uuid,
            data_dir,
            json,
        } => delete_publication(&uuid, data_dir, json),
        Command::RollBack {
            uuid,
            data_dir,
            json,
        } => roll_back(&uuid, data_dir, json),
        Command::CheckSteps => until_interrupted(check_steps()).await,
    }
}
fn validate(data_dir: Option<PathBuf>, json: Option<PathBuf>) -> Result<()> {
    let (_, _, store) = paths(data_dir, json)?;
    store.read()?;
    println!("Valid");
    Ok(())
}
fn echo_first(data_dir: Option<PathBuf>, json: Option<PathBuf>) -> Result<()> {
    let (paths, config, store) = paths(data_dir, json)?;
    let zone = publicator::scheduling::timezone(&config)?;
    let rows = store.read()?;
    if let Some(row) = rows.iter().find(|r| {
        phase(r)
            .ok()
            .is_some_and(|p| matches!(p, Phase::Described | Phase::Appeared))
            && r.get("apparitions")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|xs| {
                    xs.iter().any(|a| {
                        a.get("platformName").and_then(serde_json::Value::as_str)
                            == Some("deviantart")
                            && a.get("state").and_then(serde_json::Value::as_str)
                                == Some("unpublished")
                    })
                })
    }) {
        let schedule = publicator::model::effective_schedule(row)
            .ok_or_else(|| Error::Validation("schedule missing".into()))?;
        let date = chrono::DateTime::from_timestamp(schedule, 0)
            .ok_or_else(|| Error::Validation("invalid schedule".into()))?
            .with_timezone(&zone);
        // The original, hash-verified now: the manual upload picks it later, so a staged copy
        // would be gone or leaked by then.
        let image = publicator::artwork::resolve(&paths.data_dir, row)?.path;
        println!("path: {}", image.display());
        println!(
            "title: {}",
            row.get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
        );
        println!("schedule: {}", date.format("%Y-%m-%d %H:%M %Z"));
    } else {
        return Err(Error::NotFound("no unpublished publication".into()));
    }
    Ok(())
}
fn delete_publication(uuid: &str, data_dir: Option<PathBuf>, json: Option<PathBuf>) -> Result<()> {
    let (_, _, store) = paths(data_dir, json)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    Application::new(store, metadata_client(None)?).delete_publication(uuid, now)?;
    println!("deleted {uuid}");
    Ok(())
}
fn roll_back(uuid: &str, data_dir: Option<PathBuf>, json: Option<PathBuf>) -> Result<()> {
    let (_, _, store) = paths(data_dir, json)?;
    let state = match Application::new(store, metadata_client(None)?).roll_back(uuid)? {
        Phase::Scheduled => "SCHEDULED",
        _ => "PICKED",
    };
    println!("rolled back {uuid} to {state}");
    Ok(())
}
async fn generate_meta(args: GenerateMetaArgs) -> Result<()> {
    let GenerateMetaArgs {
        source,
        model,
        timeout,
        yes,
        verbose,
        data_dir,
        json,
    } = args;
    let (paths, _, store) = paths(data_dir, json)?;
    let source: Source = source.into();
    let model = choose_model(source, model.as_deref())?;
    if timeout == 0 {
        return Err(Error::Validation("timeout must be positive".into()));
    }
    let rows = store.read()?;
    let eligible = rows.iter().any(|r| {
        phase(r).ok() == Some(Phase::Scheduled)
            && ["title", "description"]
                .iter()
                .any(|k| r.get(*k).and_then(serde_json::Value::as_str) == Some(GENERATE_MARKER))
    });
    let client = metadata_client(Some(&paths.data_dir))?;
    let install_missing = if source == Source::Local && eligible {
        local_install_consent(&client, &model, Duration::from_secs(timeout), yes).await?
    } else {
        yes
    };
    let request = GenerationRequest {
        source,
        model,
        timeout: Duration::from_secs(timeout),
        install_missing,
    };
    let app = Application::new(store, client);
    let report = app.generate_marked(&request).await?;
    print_generation_report(report, verbose)
}
fn print_generation_report(
    report: publicator::application::BatchReport,
    verbose: bool,
) -> Result<()> {
    for result in &report.results {
        if let Some(error) = &result.error {
            eprintln!("{}: {error}", result.uuid);
        } else if verbose {
            println!("{}: saved", result.uuid);
        }
    }
    println!("generated: {}, failed: {}", report.succeeded, report.failed);
    if report.failed > 0 {
        return Err(Error::Provider(
            "one or more metadata records failed".into(),
        ));
    }
    Ok(())
}
async fn local_install_consent(
    client: &MetadataClient,
    model: &ModelOption,
    timeout: Duration,
    yes: bool,
) -> Result<bool> {
    let deadline = tokio::time::Instant::now() + timeout;
    let status = client.local_status(model, deadline).await?;
    if !status.fit.fit {
        return Err(Error::Provider(format!(
            "Local model does not fit: {}",
            status.fit.reason
        )));
    }
    if !status.available {
        return Err(Error::Provider("Ollama is unavailable".into()));
    }
    if status.installed {
        return Ok(false);
    }
    if yes {
        return Ok(true);
    }
    prompt_install_consent(model)
}
fn prompt_install_consent(model: &ModelOption) -> Result<bool> {
    if !io::stdin().is_terminal() {
        return Err(Error::Provider(
            "local model installation requires interactive consent or --yes".into(),
        ));
    }
    eprint!("Install {}? [Y/n] ", model.ollama_name);
    let mut answer = String::new();
    let read = io::stdin().read_line(&mut answer)?;
    if parse_install_consent(&answer, read) {
        Ok(true)
    } else {
        Err(Error::Provider("local model installation declined".into()))
    }
}
async fn vision_models() -> Result<()> {
    let client = metadata_client(None)?;
    for model in VISION_MODELS.iter() {
        let fit = client.local_fit(model).await?;
        println!(
            "{}: {}{}",
            model.ollama_name,
            fit.level,
            if fit.reason.is_empty() {
                String::new()
            } else {
                format!(" — {}", fit.reason)
            }
        );
    }
    Ok(())
}
fn prompt_audit(data_dir: Option<PathBuf>) -> Result<()> {
    let (paths, config, _) = paths(data_dir, None)?;
    let archive = config
        .prompts
        .as_ref()
        .map(|p| PromptArchive::from_config(&paths.data_dir, p))
        .transpose()?
        .flatten();
    let counts = audit(&paths.data_dir, &config, archive.as_ref())?;
    println!(
        "parsed: {} of {} files\nexact: {} images / {} versions\nnearest: {} images / {} versions\nunknown: {}",
        counts.parsed,
        counts.files,
        counts.exact,
        counts.exact_versions,
        counts.nearest,
        counts.nearest_versions,
        counts.unknown
    );
    Ok(())
}
async fn da_publish(args: DaPublishArgs) -> Result<()> {
    let DaPublishArgs {
        uuid,
        all,
        reconcile_attempt,
        outcome,
        url,
        headless,
        check_steps: check_only,
        data_dir,
        json,
        verbose,
    } = args;
    if check_only {
        return until_interrupted(check_steps()).await;
    }
    let (paths, _, store) = paths(data_dir, json)?;
    let publisher = Publisher::from_env();
    if let Some(attempt) = reconcile_attempt {
        return reconcile(&paths, &store, &attempt, outcome, url);
    }
    let ids = publication_ids(&store, uuid, all)?;
    let app = Application::new(store.clone(), metadata_client(Some(&paths.data_dir))?)
        .with_publisher(publisher);
    let result = until_interrupted(app.publish(
        PublishRequest {
            uuids: ids,
            headless,
        },
        Arc::new(AtomicBool::new(false)),
    ))
    .await;
    print_unresolved(&paths.data_dir, &store, &result);
    print_publish_report(result?, verbose)
}
/// Operator guidance for attempts whose remote outcome is unknown; mirrors the skill's recovery prose.
fn print_unresolved(data_dir: &Path, store: &Store, result: &Result<PublishReport>) {
    if !matches!(
        result,
        Ok(PublishReport { unknown: 1.., .. }) | Err(Error::ReconciliationRequired(_))
    ) {
        return;
    }
    let blocked = publish_journal::publisher_lock(data_dir)
        .and_then(|_lock| PublishJournal::new(data_dir)?.blocked_attempts_locked(store))
        .unwrap_or_else(|error| {
            eprintln!("could not list unresolved attempts: {error}");
            Vec::new()
        });
    if blocked.is_empty() {
        return;
    }
    let dir = data_dir.display();
    for attempt in &blocked {
        let (id, uuid, reason) = (&attempt.attempt_id, &attempt.uuid, &attempt.reason);
        eprintln!("UNRESOLVED {id} ({uuid}): {reason}");
        eprintln!(
            "  on DeviantArt: da-publish --data-dir {dir} --reconcile-attempt {id} --outcome scheduled [--url URL]"
        );
        eprintln!(
            "  absent:        da-publish --data-dir {dir} --reconcile-attempt {id} --outcome not-submitted"
        );
    }
    eprintln!(
        "DeviantArt may already hold these submissions: do not resubmit or redo them by hand.\n\
         Inspect DeviantArt first, then record what you found with one command above.\n\
         Retry or upload by hand only after --outcome not-submitted. Never delete .publicator/ receipts."
    );
}
/// SIGINT/SIGTERM drop `work`, which kills its owned worker/checker process group and releases
/// the publisher lock; an attempt already sent keeps its receipt and stays blocked.
async fn until_interrupted<T>(work: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! {
        biased;
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate.recv() => {}
        result = work => return result,
    }
    Err(Error::Protocol(
        "publishing interrupted; the worker was stopped and an in-flight attempt stays unresolved until reconciled".into(),
    ))
}
fn reconcile(
    paths: &Paths,
    store: &Store,
    attempt: &str,
    outcome: Option<OutcomeArg>,
    url: Option<String>,
) -> Result<()> {
    let outcome = match outcome
        .ok_or_else(|| Error::Validation("--outcome is required for reconciliation".into()))?
    {
        OutcomeArg::Scheduled => ReconcileOutcome::Scheduled { actual_url: url },
        OutcomeArg::NotSubmitted => {
            if url.is_some() {
                return Err(Error::Validation(
                    "--url is only valid with scheduled outcome".into(),
                ));
            }
            ReconcileOutcome::NotSubmitted
        }
    };
    PublishJournal::new(&paths.data_dir)?.reconcile(store, attempt, outcome)?;
    println!("reconciled {attempt}");
    Ok(())
}
fn publication_ids(store: &Store, uuid: Option<String>, all: bool) -> Result<Option<Vec<String>>> {
    Ok(if let Some(id) = uuid {
        Some(vec![id])
    } else if all {
        None
    } else {
        let rows = store.read()?;
        Some(first_pending_uuid(&rows)?.into_iter().collect())
    })
}
fn print_publish_report(report: PublishReport, verbose: bool) -> Result<()> {
    println!(
        "published: {}, failed: {}, unknown: {}, unattempted: {}",
        report.published, report.failed, report.unknown, report.unattempted
    );
    if let Some(error) = report.error {
        eprintln!("{error}");
    }
    if report.failed + report.unknown > 0 {
        return Err(Error::Protocol(
            "publication batch did not complete successfully".into(),
        ));
    }
    if verbose {
        println!("publication run complete");
    }
    Ok(())
}
async fn check_steps() -> Result<()> {
    Publisher::from_env().check_steps().await?;
    println!("check-steps OK");
    Ok(())
}
fn parse_install_consent(answer: &str, bytes_read: usize) -> bool {
    bytes_read > 0
        && matches!(
            answer.trim().to_ascii_lowercase().as_str(),
            "" | "y" | "yes"
        )
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod consent_tests {
    use super::{choose_model, parse_install_consent};
    use publicator::metadata::Source;

    #[test]
    fn default_openrouter_model_remains_configured() {
        assert_eq!(
            choose_model(Source::OpenRouter, None)
                .unwrap()
                .openrouter_name,
            "openrouter/google/gemma-4-26b-a4b-it:free"
        );
    }

    #[test]
    fn only_enter_yes_or_y_authorize_model_installation() {
        for answer in ["\n", "y", "Y", "yes", "YES"] {
            assert!(parse_install_consent(answer, answer.len()), "{answer:?}");
        }
        for answer in ["n", "N", "no", "NO", "maybe", "anything"] {
            assert!(!parse_install_consent(answer, answer.len()), "{answer:?}");
        }
        assert!(!parse_install_consent("", 0));
    }
}
