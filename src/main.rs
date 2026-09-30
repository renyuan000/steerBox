use serde_json::Value;
use std::env;
use std::error::Error;
use std::process;
use steerbox::domain::{EventEnvelope, GoalContract, LoopBudget, LoopStatus};
use steerbox::event_log::EventLog;
use steerbox::loop_controller::ReadOnlyLoopController;
use steerbox::model_adapter::{LocalFixtureModelAdapter, ModelDecision};
use steerbox::runtime_config::RuntimeConfig;

fn main() {
    if let Err(error) = run(env::args().skip(1)) {
        eprintln!("steerbox: {error}");
        eprintln!();
        print_usage();
        process::exit(1);
    }
}

fn run(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    match (args.next().as_deref(), args.next().as_deref()) {
        (Some("runtime"), Some("init")) => {
            let project_ref = args.next().ok_or("missing project_ref")?;
            let config = RuntimeConfig::default_for_project(project_ref)?;
            if config.config_path().exists() {
                return Err(format!(
                    "configuration already exists: {}",
                    config.config_path().display()
                )
                .into());
            }
            config.write_json(config.config_path())?;
            let log = EventLog::open(config.events_path())?;
            println!(
                "initialized project={} data_root={}",
                config.project_ref,
                config.data_root.display()
            );
            println!("config={}", config.config_path().display());
            println!("event_log={}", log.path().display());
        }
        (Some("config"), Some("validate")) => {
            let path = args.next().ok_or("missing config path")?;
            let config = RuntimeConfig::load(path)?;
            println!(
                "valid project={} data_root={} model_profiles={}",
                config.project_ref,
                config.data_root.display(),
                config.model_profiles.len()
            );
        }
        (Some("event"), Some("list")) => {
            let path = args.next().ok_or("missing config path")?;
            let config = RuntimeConfig::load(path)?;
            let log = EventLog::open(config.events_path())?;
            let events = log.read_all()?;
            println!("events={}", events.len());
            for event in events {
                println!(
                    "sequence={} type={} aggregate={}:{}",
                    event.sequence_no.unwrap_or_default(),
                    event.event_type,
                    event.aggregate_type,
                    event.aggregate_id
                );
            }
        }
        (Some("event"), Some("append")) => {
            let config_path = args.next().ok_or("missing config path")?;
            let event_type = args.next().ok_or("missing event_type")?;
            let aggregate_type = args.next().ok_or("missing aggregate_type")?;
            let aggregate_id = args.next().ok_or("missing aggregate_id")?;
            let event_id = args.next().ok_or("missing event_id")?;
            let correlation_id = args.next().ok_or("missing correlation_id")?;
            let payload_text = args.next().unwrap_or_else(|| "{}".into());
            let payload = serde_json::from_str::<Value>(&payload_text)
                .map_err(|error| format!("invalid payload JSON: {error}"))?;
            let config = RuntimeConfig::load(config_path)?;
            let mut log = EventLog::open(config.events_path())?;
            let event = EventEnvelope::new(
                aggregate_type,
                event_id,
                event_type,
                aggregate_id,
                correlation_id,
                payload,
            )?;
            let sequence = log.append(event)?;
            println!("appended sequence={sequence}");
        }
        (Some("loop"), Some("readonly")) => {
            let config_path = args.next().ok_or("missing config path")?;
            let project_id = args.next().ok_or("missing project_id")?;
            let goal_id = args.next().ok_or("missing goal_id")?;
            let loop_run_id = args.next().ok_or("missing loop_run_id")?;
            let task_run_id = args.next().ok_or("missing task_run_id")?;
            let step_id = args.next().ok_or("missing step_id")?;
            let objective = args.next().ok_or("missing objective")?;
            let observation = args.next().ok_or("missing observation")?;
            let config = RuntimeConfig::load(config_path)?;
            let log = EventLog::open(config.events_path())?;
            let goal = GoalContract::new(
                goal_id,
                project_id,
                objective,
                vec![".".into()],
                Vec::new(),
                vec!["manual_review".into()],
            );
            let budget = LoopBudget::new(1, 0, 0, 1, 0, 1, 1, 1);
            let mut controller =
                ReadOnlyLoopController::start(log, goal, loop_run_id, task_run_id, budget)?;
            let step = controller.run_observation(step_id, observation)?;
            let run = controller.complete()?;
            println!(
                "loop={} status={:?} step={} step_status={:?}",
                run.loop_run_id, run.status, step.step_id, step.status
            );
        }
        (Some("loop"), Some("fixture")) => {
            let config_path = args.next().ok_or("missing config path")?;
            let project_id = args.next().ok_or("missing project_id")?;
            let goal_id = args.next().ok_or("missing goal_id")?;
            let loop_run_id = args.next().ok_or("missing loop_run_id")?;
            let task_run_id = args.next().ok_or("missing task_run_id")?;
            let model_id = args.next().ok_or("missing model_id")?;
            let decision = parse_model_decision(args.next().as_deref().unwrap_or("complete_loop"))?;
            let observation = args.next().ok_or("missing observation")?;
            let summary = args
                .next()
                .unwrap_or_else(|| "local fixture response".into());
            let config = RuntimeConfig::load(config_path)?;
            let log = EventLog::open(config.events_path())?;
            let goal = GoalContract::new(
                goal_id,
                project_id,
                "fixture model observation",
                vec![".".into()],
                Vec::new(),
                vec!["manual_review".into()],
            );
            let budget = LoopBudget::new(1, 0, 1, 1, 0, 1, 1, 1);
            let mut controller =
                ReadOnlyLoopController::start(log, goal, loop_run_id, task_run_id, budget)?;
            let adapter = LocalFixtureModelAdapter::new(decision, summary);
            let actual = controller.run_model_step(&adapter, model_id, "step-1", observation)?;
            println!(
                "decision={actual:?} loop_status={:?}",
                controller_status(&controller)
            );
        }
        _ => {
            print_usage();
        }
    }
    Ok(())
}

fn print_usage() {
    eprintln!("Usage:");
    eprintln!("  steerbox runtime init <project_ref>");
    eprintln!("  steerbox config validate <config.json>");
    eprintln!("  steerbox event list <config.json>");
    eprintln!("  steerbox event append <config.json> <event_type> <aggregate_type> <aggregate_id> <event_id> <correlation_id> [payload_json]");
    eprintln!("  steerbox loop readonly <config.json> <project_id> <goal_id> <loop_run_id> <task_run_id> <step_id> <objective> <observation>");
    eprintln!("  steerbox loop fixture <config.json> <project_id> <goal_id> <loop_run_id> <task_run_id> <model_id> [continue_observation|complete_loop|request_human] <observation> [summary]");
}

fn parse_model_decision(value: &str) -> Result<ModelDecision, Box<dyn Error>> {
    match value {
        "continue_observation" => Ok(ModelDecision::ContinueObservation),
        "complete_loop" => Ok(ModelDecision::CompleteLoop),
        "request_human" => Ok(ModelDecision::RequestHuman),
        _ => Err(format!("unsupported fixture decision: {value}").into()),
    }
}

fn controller_status(controller: &ReadOnlyLoopController) -> LoopStatus {
    controller.status()
}
