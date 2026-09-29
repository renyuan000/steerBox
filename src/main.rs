use std::env;
use std::error::Error;
use std::process;
use steerbox::event_log::EventLog;
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
}
