mod cli_args;

use clap::Parser;
use cli_args::{Cli, Command, PresetCommand, SecretCommand};
use rconv_core::{
    apply_runtime_overrides, config::PresetDefinition, curator, load_config, run_cli,
    runtime_preferences_to_convocations, save_config, save_presets_and_ui_only,
};
use rpassword::prompt_password;

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
        Some(Command::Secret(cmd)) => {
            if !cli.process.is_empty() {
                return Err(
                    "Processing flags cannot be combined with secret management commands.".into(),
                );
            }
            handle_secret_command(cmd)
        }
        None => run_process(cli.process).await,
    }
}

async fn run_process(args: cli_args::ProcessArgs) -> Result<(), String> {
    if args.list_curated {
        let entries = curator::catalog_summaries().map_err(|err| err.to_string())?;
        if entries.is_empty() {
            println!("No curated models available.");
        } else {
            println!(
                "Tier    AAII  Slug                          Name [provider]  PriceIn  PriceOut  Source"
            );
            for entry in entries {
                let tier = match entry.tier {
                    curator::CuratedTier::Free => "free",
                    curator::CuratedTier::Cheap => "cheap",
                };
                let price_in = entry
                    .price_in_per_million
                    .map(|v| format!("{v:.2}"))
                    .unwrap_or_else(|| "-".to_string());
                let price_out = entry
                    .price_out_per_million
                    .map(|v| format!("{v:.2}"))
                    .unwrap_or_else(|| "-".to_string());
                let source = match entry.price_source {
                    curator::PriceSource::Aa => "aa",
                    curator::PriceSource::Openrouter => "openrouter",
                };
                println!(
                    "{tier:<5}  {aaii:>5.1}  {slug:<28}  {name} [{provider}]  {price_in:>7}  {price_out:>7}  {source}",
                    tier = tier,
                    aaii = entry.aaii,
                    slug = entry.slug,
                    name = entry.display_name,
                    provider = entry.provider,
                    price_in = price_in,
                    price_out = price_out,
                    source = source
                );
            }
        }
        return Ok(());
    }

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
            if config.presets.iter().any(|preset| preset.name == args.name) {
                return Err(format!("Preset '{}' already exists.", args.name));
            }
            let preset = PresetDefinition {
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
            config.presets.sort_by(|a, b| a.name.cmp(&b.name));
            // Save only presets and UI preferences; runtime preferences are session-only
            save_presets_and_ui_only(&config.presets, &config.ui).map_err(|err| err.to_string())?;
            println!("Created preset '{}'", args.name);
            Ok(())
        }
        PresetCommand::Update(args) => {
            let preset = config
                .presets
                .iter_mut()
                .find(|preset| preset.name == args.name)
                .ok_or_else(|| format!("Preset '{}' not found.", args.name))?;

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

            // Save only presets and UI preferences; runtime preferences are session-only
            save_presets_and_ui_only(&config.presets, &config.ui).map_err(|err| err.to_string())?;
            println!("Updated preset '{}'", args.name);
            Ok(())
        }
        PresetCommand::Delete(args) => {
            let position = config
                .presets
                .iter()
                .position(|preset| preset.name == args.name)
                .ok_or_else(|| format!("Preset '{}' not found.", args.name))?;
            if config.presets[position].builtin {
                return Err(format!(
                    "Preset '{}' is built-in and cannot be deleted.",
                    args.name
                ));
            }

            config.presets.remove(position);

            // Save only presets and UI preferences; runtime preferences are session-only
            // Note: If the deleted preset was active, sanitize_config will reset it on next load
            save_presets_and_ui_only(&config.presets, &config.ui).map_err(|err| err.to_string())?;
            println!("Deleted preset '{}'", args.name);
            Ok(())
        }
    }
}

fn handle_secret_command(command: SecretCommand) -> Result<(), String> {
    let load = load_config();
    for warning in load.warnings {
        eprintln!("Warning: {warning}");
    }
    let mut config = load.config;

    match command {
        SecretCommand::SetOpenRouterKey { key } => {
            let value = match key {
                Some(v) => v,
                None => prompt_password("Enter OpenRouter API key: ")
                    .map_err(|err| format!("Failed to read API key: {err}"))?,
            };
            config
                .runtime
                .set_openrouter_api_key(&value)
                .map_err(|err| format!("{}", err))?;
            save_config(&config).map_err(|err| err.to_string())?;
            println!("OpenRouter API key saved securely.");
            Ok(())
        }
        SecretCommand::ClearOpenRouterKey => {
            config
                .runtime
                .clear_openrouter_api_key()
                .map_err(|err| format!("{}", err))?;
            save_config(&config).map_err(|err| err.to_string())?;
            println!("Cleared saved OpenRouter API key.");
            Ok(())
        }
    }
}
