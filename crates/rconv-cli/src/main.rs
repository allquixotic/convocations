mod cli_args;

use clap::Parser;
use cli_args::{Cli, Command, PresetCommand};
use rconv_core::config::SATURDAY_PRESET_ID;
use rconv_core::{
    apply_runtime_overrides, config::PresetDefinition, load_config, run_cli,
    runtime_preferences_to_convocations, save_config,
};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(err) = dispatch(cli).await {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

async fn dispatch(cli: Cli) -> Result<(), String> {
    match cli.command {
        Some(Command::Preset(cmd)) => {
            if !cli.process.is_empty() {
                return Err(
                    "Processing flags cannot be combined with preset management commands.".into(),
                );
            }
            handle_preset_command(cmd)
        }
        None => run_process(cli.process).await,
    }
}

async fn run_process(args: cli_args::ProcessArgs) -> Result<(), String> {
    let load = load_config();
    let mut warnings = load.warnings;
    let presets = load.config.presets.clone();
    let (mut runtime_config, mut runtime_warnings) =
        runtime_preferences_to_convocations(&load.config.runtime, &presets);
    warnings.append(&mut runtime_warnings);

    let (overrides, mut override_warnings) = args.to_runtime_overrides()?;
    warnings.append(&mut override_warnings);

    let preset_catalog = runtime_config.presets.clone();
    apply_runtime_overrides(
        &mut runtime_config,
        &overrides,
        &preset_catalog,
        &mut warnings,
    );

    for warning in warnings {
        eprintln!("Warning: {warning}");
    }

    run_cli(runtime_config).await
}

fn handle_preset_command(command: PresetCommand) -> Result<(), String> {
    let load = load_config();
    for warning in load.warnings {
        eprintln!("Warning: {warning}");
    }
    let mut config = load.config;

    match command {
        PresetCommand::Create(args) => {
            if config.presets.iter().any(|preset| preset.id == args.id) {
                return Err(format!("Preset '{}' already exists.", args.id));
            }
            let preset = PresetDefinition {
                id: args.id.clone(),
                name: args.name.clone(),
                weekday: args.weekday.to_ascii_lowercase(),
                timezone: args.timezone.clone(),
                start_time: args.start_time.clone(),
                duration_minutes: args.duration_minutes,
                file_prefix: args.file_prefix.clone(),
                default_weeks_ago: args.default_weeks_ago,
                builtin: false,
            };
            config.presets.push(preset);
            config.presets.sort_by(|a, b| a.id.cmp(&b.id));
            save_config(&config).map_err(|err| err.to_string())?;
            println!("Created preset '{}' ({})", args.name, args.id);
            Ok(())
        }
        PresetCommand::Update(args) => {
            let preset = config
                .presets
                .iter_mut()
                .find(|preset| preset.id == args.id)
                .ok_or_else(|| format!("Preset '{}' not found.", args.id))?;

            if let Some(name) = args.name {
                preset.name = name;
            }
            if let Some(weekday) = args.weekday {
                preset.weekday = weekday.to_ascii_lowercase();
            }
            if let Some(timezone) = args.timezone {
                preset.timezone = timezone;
            }
            if let Some(start_time) = args.start_time {
                preset.start_time = start_time;
            }
            if let Some(duration_minutes) = args.duration_minutes {
                preset.duration_minutes = duration_minutes;
            }
            if let Some(file_prefix) = args.file_prefix {
                preset.file_prefix = file_prefix;
            }
            if let Some(weeks_ago) = args.default_weeks_ago {
                preset.default_weeks_ago = weeks_ago;
            }

            save_config(&config).map_err(|err| err.to_string())?;
            println!("Updated preset '{}'", args.id);
            Ok(())
        }
        PresetCommand::Delete(args) => {
            let position = config
                .presets
                .iter()
                .position(|preset| preset.id == args.id)
                .ok_or_else(|| format!("Preset '{}' not found.", args.id))?;
            if config.presets[position].builtin {
                return Err(format!(
                    "Preset '{}' is built-in and cannot be deleted.",
                    args.id
                ));
            }

            let was_active = config.runtime.active_preset == args.id;
            config.presets.remove(position);
            if was_active {
                config.runtime.active_preset = SATURDAY_PRESET_ID.to_string();
            }

            save_config(&config).map_err(|err| err.to_string())?;
            println!("Deleted preset '{}'", args.id);
            Ok(())
        }
    }
}
