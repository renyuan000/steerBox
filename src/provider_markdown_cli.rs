//! Offline CLI preparation only; this module grants no mutation authority.

use std::{
    error::Error,
    fs,
    path::{Component, Path, PathBuf},
};
use steerbox::domain::HumanSteeringMode;
use steerbox::provider_runner::{
    BoundedMarkdownRunConfig, ForegroundProviderRun, ProviderRunRequest,
};
use steerbox::runtime_config::RuntimeConfig;

const INVALID: &str = "invalid preflight-markdown arguments";
const CHECK_FAILED: &str = "preflight-markdown scope or runtime check failed";

struct Arguments {
    config_path: String,
    request: ProviderRunRequest,
    workspace: PathBuf,
    paths: Vec<String>,
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Arguments, &'static str> {
    let config_path = args.next().ok_or(INVALID)?;
    let request = ProviderRunRequest {
        project_id: args.next().ok_or(INVALID)?,
        goal_id: args.next().ok_or(INVALID)?,
        loop_run_id: args.next().ok_or(INVALID)?,
        task_run_id: args.next().ok_or(INVALID)?,
        model_id: args.next().ok_or(INVALID)?,
        objective: args.next().ok_or(INVALID)?,
        observation: args.next().ok_or(INVALID)?,
    };
    let mut workspace = None;
    let mut paths = Vec::new();
    while let Some(flag) = args.next() {
        let value = args.next().ok_or(INVALID)?;
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
                    return Err(INVALID);
                }
                paths.push(value);
            }
            _ => return Err(INVALID),
        }
    }
    if paths.is_empty() {
        return Err(INVALID);
    }
    Ok(Arguments {
        config_path,
        request,
        workspace: workspace.ok_or(INVALID)?,
        paths,
    })
}

pub fn preflight(args: impl Iterator<Item = String>) -> Result<String, Box<dyn Error>> {
    let args = parse(args)?;
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
    let workspace = fs::canonicalize(args.workspace).map_err(|_| CHECK_FAILED)?;
    let runtime =
        fs::canonicalize(events_path.parent().ok_or(CHECK_FAILED)?).map_err(|_| CHECK_FAILED)?;
    if workspace.starts_with(&runtime) || runtime.starts_with(&workspace) {
        return Err(CHECK_FAILED.into());
    }
    Ok("preflight=passed scope=markdown execution=disabled credentials=not_checked transport=not_checked history=not_checked".into())
}
