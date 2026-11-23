//! Bounded Markdown CLI: offline preparation or one explicitly authorized run.

use crate::CliCredentialResolver;
use std::{
    error::Error,
    fs,
    path::{Component, Path, PathBuf},
};
use steerbox::domain::HumanSteeringMode;
use steerbox::event_log::EventLog;
use steerbox::model_call_store::ModelCallStore;
use steerbox::provider_runner::{
    BoundedMarkdownProviderRun, BoundedMarkdownRunConfig, ForegroundProviderRun,
    MarkdownRuntimePathCheck, ProviderRunError, ProviderRunOutcome, ProviderRunRequest,
};
use steerbox::runtime_config::RuntimeConfig;
use steerbox::state_store::StateStore;
use steerbox::stream_reporter::{ProgressEvent, ProgressSink, StreamReporter};

const INVALID: &str = "invalid preflight-markdown arguments";
const CHECK_FAILED: &str = "preflight-markdown scope or runtime check failed";
const EXECUTION_FAILED: &str = "provider-markdown execution failed; audit or workspace may have changed; no automatic rollback";
const PROJECTION_FAILED: &str = "provider-markdown state projection failed; audit or workspace may have changed; no automatic rollback";

struct Arguments {
    config_path: String,
    request: ProviderRunRequest,
    workspace: PathBuf,
    paths: Vec<String>,
    credentials: CliCredentialResolver,
}

fn parse(
    mut args: impl Iterator<Item = String>,
    executing: bool,
) -> Result<Arguments, &'static str> {
    let invalid = if executing {
        "invalid provider-markdown arguments"
    } else {
        INVALID
    };
    let config_path = args.next().ok_or(invalid)?;
    let request = ProviderRunRequest {
        project_id: args.next().ok_or(invalid)?,
        goal_id: args.next().ok_or(invalid)?,
        loop_run_id: args.next().ok_or(invalid)?,
        task_run_id: args.next().ok_or(invalid)?,
        model_id: args.next().ok_or(invalid)?,
        objective: args.next().ok_or(invalid)?,
        observation: args.next().ok_or(invalid)?,
    };
    let mut workspace = None;
    let mut paths = Vec::new();
    let mut authorized = false;
    let mut credentials = None;
    while let Some(flag) = args.next() {
        let value = args.next().ok_or(invalid)?;
        match flag.as_str() {
            "--workspace" if workspace.is_none() && Path::new(&value).is_absolute() => {
                workspace = Some(PathBuf::from(value));
            }
            "--path" if paths.len() < 256 => {
                let relative = Path::new(&value);
                let normalized: PathBuf = relative.components().collect();
                if value.is_empty()
                    || value.starts_with("--")
                    || value.contains('\\')
                    || value.chars().any(char::is_control)
                    || !(value.ends_with(".md") || value.ends_with(".markdown"))
                    || !relative
                        .components()
                        .all(|part| matches!(part, Component::Normal(_)))
                    || normalized.as_os_str() != relative.as_os_str()
                    || paths.contains(&value)
                {
                    return Err(invalid);
                }
                paths.push(value);
            }
            "--authorize-markdown" if executing && !authorized && value == "once" => {
                authorized = true;
            }
            "--credential-source" if executing && credentials.is_none() && value == "env" => {
                credentials = Some(CliCredentialResolver::Environment);
            }
            _ => return Err(invalid),
        }
    }
    if paths.is_empty() || (executing && !authorized) {
        return Err(invalid);
    }
    Ok(Arguments {
        config_path,
        request,
        workspace: workspace.ok_or(invalid)?,
        paths,
        credentials: credentials.unwrap_or(CliCredentialResolver::Unconfigured),
    })
}

pub fn preflight(args: impl Iterator<Item = String>) -> Result<String, Box<dyn Error>> {
    let args = parse(args, false)?;
    // Config errors can contain input text. Do not forward them to the terminal.
    let config = RuntimeConfig::load(args.config_path)
        .map_err(|_| "invalid preflight-markdown configuration")?;
    let prepared = ForegroundProviderRun::prepare_markdown(
        &config,
        args.request,
        BoundedMarkdownRunConfig {
            workspace: args.workspace.clone(),
            allowed_paths: args.paths,
            checkpoint_path: config.checkpoints_path(),
            trace_path: config.traces_path(),
            side_effect_path: config.side_effects_path(),
            approved: false,
            human_steering_mode: HumanSteeringMode::HumanSteerableLoop,
        },
    )
    .map_err(|_| CHECK_FAILED)?;
    let events_path = config.events_path();
    prepared
        .validate_runtime_paths(
            &events_path,
            &config.model_calls_path(),
            Some(&config.state_path()),
        )
        .map_err(|_| CHECK_FAILED)?;
    // Keep the runner's no-symlink and inode checks above: canonicalization alone
    // would conceal symlinks. This extra separation is CLI policy, not a permit.
    validate_separation(&args.workspace, &events_path).map_err(|_| CHECK_FAILED)?;
    Ok("preflight=passed scope=markdown execution=disabled credentials=not_checked transport=not_checked history=not_checked".into())
}

fn validate_separation(workspace: &Path, events_path: &Path) -> Result<(), Box<dyn Error>> {
    let workspace = fs::canonicalize(workspace)?;
    let runtime = fs::canonicalize(events_path.parent().ok_or(CHECK_FAILED)?)?;
    if workspace.starts_with(&runtime) || runtime.starts_with(&workspace) {
        return Err(CHECK_FAILED.into());
    }
    Ok(())
}

fn revalidate(
    paths: &MarkdownRuntimePathCheck,
    workspace: &Path,
    events: &Path,
) -> Result<(), Box<dyn Error>> {
    paths.revalidate()?;
    validate_separation(workspace, events)
}

/// Keep lifecycle feedback without echoing user IDs, model text or tool payloads.
struct MetadataProgress<S>(S);

impl<S: ProgressSink> ProgressSink for MetadataProgress<S> {
    fn report(&mut self, mut event: ProgressEvent) -> std::io::Result<()> {
        event.subject_id = "bounded-markdown".into();
        event.summary = None;
        event.progress = None;
        self.0.report(event)
    }
}

pub fn run(args: impl Iterator<Item = String>) -> Result<String, Box<dyn Error>> {
    run_with(args, |prepared, credentials, events, calls| {
        prepared.run_with_resolver(
            credentials,
            events,
            calls,
            MetadataProgress(StreamReporter::new(std::io::stdout())),
        )
    })
}

fn run_with(
    args: impl Iterator<Item = String>,
    execute: impl FnOnce(
        BoundedMarkdownProviderRun,
        CliCredentialResolver,
        EventLog,
        ModelCallStore,
    ) -> Result<ProviderRunOutcome, ProviderRunError>,
) -> Result<String, Box<dyn Error>> {
    const SCOPE_FAILED: &str = "provider-markdown scope or runtime check failed";
    let args = parse(args, true)?;
    let config = RuntimeConfig::load(args.config_path)
        .map_err(|_| "invalid provider-markdown configuration")?;
    // Only the complete CLI parse grants this invocation's scope authorization.
    // This is not an ApprovalStore grant or a permission to retry/resume.
    let prepared = ForegroundProviderRun::prepare_markdown(
        &config,
        args.request,
        BoundedMarkdownRunConfig {
            workspace: args.workspace.clone(),
            allowed_paths: args.paths,
            checkpoint_path: config.checkpoints_path(),
            trace_path: config.traces_path(),
            side_effect_path: config.side_effects_path(),
            approved: true,
            human_steering_mode: HumanSteeringMode::HumanSteerableLoop,
        },
    )
    .map_err(|_| SCOPE_FAILED)?;
    let events_path = config.events_path();
    let paths = prepared
        .validate_runtime_paths(
            &events_path,
            &config.model_calls_path(),
            Some(&config.state_path()),
        )
        .map_err(|_| SCOPE_FAILED)?;
    validate_separation(&args.workspace, &events_path).map_err(|_| SCOPE_FAILED)?;
    let events = EventLog::open(&events_path).map_err(|_| EXECUTION_FAILED)?;
    let calls = ModelCallStore::open(config.model_calls_path()).map_err(|_| EXECUTION_FAILED)?;
    revalidate(&paths, &args.workspace, &events_path).map_err(|_| EXECUTION_FAILED)?;
    let result = execute(prepared, args.credentials, events, calls);
    // Project actual events even on execution failure. Never retry the mutation.
    let projection = (|| -> Result<(), Box<dyn Error>> {
        revalidate(&paths, &args.workspace, &events_path)?;
        let events = EventLog::open(&events_path)?.read_all()?;
        let state = StateStore::rebuild(&events)?;
        revalidate(&paths, &args.workspace, &events_path)?;
        state.persist(config.state_path())?;
        Ok(())
    })();
    let outcome = match (result, projection) {
        (Err(_), _) => return Err(EXECUTION_FAILED.into()),
        (Ok(_), Err(_)) => return Err(PROJECTION_FAILED.into()),
        (Ok(outcome), Ok(())) => outcome,
    };
    Ok(format!(
        "decision={:?} loop_status={:?} scope=markdown",
        outcome.decision, outcome.loop_status
    ))
}

#[cfg(all(test, unix))]
#[path = "provider_markdown_cli_tests.rs"]
mod tests;
