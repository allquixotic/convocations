# Configuration Schema Reference

## Overview

The Convocations Log Processor uses a TOML-based configuration system stored at `~/.config/convocations/config.toml`. This document describes the current implementation of the configuration schema (version 1).

## Migration from Legacy JSON

The system automatically migrates from the legacy `settings.json` format:
- On first launch, if `config.toml` doesn't exist but `settings.json` does, the JSON configuration is converted to TOML
- The new TOML file is immediately persisted
- Legacy JSON files are preserved for backward compatibility

## `config.toml` Schema

**Location**: `~/.config/convocations/config.toml`

**Structure**:
- `schema_version`: Integer version for future migration handling (currently 2)
- `[runtime]`: Runtime preferences shared between CLI and GUI
- `[ui]`: GUI-only preferences (theme, technical log visibility, etc.)
- `[[presets]]`: Array of preset definitions (both built-in and user-defined)

**Duration Override**: The `duration_override` object has two fields:
- `enabled`: Boolean toggle for the override
- `hours`: Floating-point number representing duration (minimum 1.0)

**Presets**: Built-in presets ("Saturday 10pm-midnight", "Tuesday 7pm", "Tuesday 8pm", "Friday 6pm") preserve their original semantics. User-defined presets use the same structure with `builtin = false`.

```toml
schema_version = 2

[runtime]
chat_log_path = "~/Documents/Elder Scrolls Online/live/Logs/ChatLog.log"
active_preset = "Saturday 10pm-midnight"
weeks_ago = 0
dry_run = false
use_ai_corrections = true
keep_original_output = false
show_diff = true
cleanup_enabled = true
format_dialogue_enabled = true
outfile_override = ""
output_target = "file"
output_directory_override = ""
openrouter_model = "google/gemini-2.5-flash-lite"
free_models_only = false

[runtime.openrouter_api_key]
backend = "keyring"
account = "convocations-openrouter_api_key"

[runtime.duration_override]
enabled = false
hours = 1.0

[ui]
theme = "dark"
show_technical_log = false
follow_technical_log = true

[[presets]]
name = "Saturday 10pm-midnight"
weekday = "saturday"
timezone = "America/New_York"
start_time = "22:00"
duration_minutes = 145
file_prefix = "conv"
default_weeks_ago = 0
builtin = true

[[presets]]
name = "Tuesday 7pm"
weekday = "tuesday"
timezone = "America/New_York"
start_time = "19:00"
duration_minutes = 60
file_prefix = "rsm7"
default_weeks_ago = 0
builtin = true

[[presets]]
name = "Custom preset name"
weekday = "friday"
timezone = "America/New_York"
start_time = "18:30"
duration_minutes = 120
file_prefix = "tp6"
default_weeks_ago = 1
builtin = false
```

## Field Descriptions

### `[runtime]` Section

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `chat_log_path` | string | `~/Documents/Elder Scrolls Online/live/Logs/ChatLog.log` | Path to the ESO ChatLog.log file |
| `active_preset` | string | `Saturday 10pm-midnight` | Name of the currently active preset |
| `weeks_ago` | u32 | 0 | Number of weeks to look back (0 = current week) |
| `dry_run` | bool | false | When true, shows what would be processed without creating output |
| `use_ai_corrections` | bool | true | Enable Gemini AI corrections for spelling/grammar |
| `keep_original_output` | bool | false | Retain `_unedited` file when LLM is enabled |
| `show_diff` | bool | true | Display diff between pre-LLM and post-LLM output |
| `cleanup_enabled` | bool | true | Remove OOC content and normalize punctuation |
| `format_dialogue_enabled` | bool | true | Format dialogue with proper attribution |
| `outfile_override` | Option<string> | None | Override automatic output filename |
| `output_target` | string | `"file"` | Either `"file"` or `"directory"`; chooses which output widget the GUI exposes |
| `output_directory_override` | Option<string> | None | Remembered directory path used when `output_target = "directory"` |
| `duration_override.enabled` | bool | false | Enable custom duration override |
| `duration_override.hours` | f32 | 1.0 | Custom duration in hours (minimum 1.0) |
| `openrouter_model` | Option<string> | `google/gemini-2.5-flash-lite` | Default OpenRouter model used for AI corrections |
| `openrouter_api_key` | secret reference | n/a | Secure reference describing where the OpenRouter key is stored (`{ backend = \"keyring\", account = \"...\" }` or `{ backend = \"local-encrypted\", nonce = \"...\", ciphertext = \"...\" }`). Managed automatically—do not edit manually. |
| `free_models_only` | bool | false | When true, filters the full OpenRouter model list to show only free entries |

`openrouter_api_key` always resolves to a `SecretValue`. Plaintext entries are migrated during load; if the keyring backend is unavailable, the encrypted fallback uses the master key at `~/.config/convocations/secret.key` (0600 permissions).

### `[ui]` Section

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `theme` | string | `dark` | UI theme: `light`, `dark`, or `system` |
| `show_technical_log` | bool | false | Display technical processing log in GUI |
| `follow_technical_log` | bool | true | Auto-scroll technical log as new entries appear |

### `[[presets]]` Section

Each preset defines a recurring RP event with specific timing and formatting:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Display name shown in GUI |
| `weekday` | string | Yes | Day of week: `saturday`, `tuesday`, `friday`, etc. |
| `timezone` | string | Yes | IANA timezone (e.g., `America/New_York`) |
| `start_time` | string | Yes | Event start time in HH:MM format (24-hour) |
| `duration_minutes` | u32 | Yes | Event duration in minutes |
| `file_prefix` | string | Yes | Prefix for output files (e.g., `conv`, `rsm7`) |
| `default_weeks_ago` | u32 | No | Default value for `weeks_ago` when preset is selected |
| `builtin` | bool | No | If true, preset cannot be edited or deleted |

## Implementation Notes

- `chat_log_path`: Single source of truth for log file location across CLI and GUI
- `use_ai_corrections`, `keep_original_output`, `show_diff`: Map to internal flags (`use_llm`, `keep_orig`, `!no_diff`)
- `weeks_ago`: Persisted so both CLI and GUI remember the last selection
- `duration_minutes`: Uses integer minutes to handle cross-midnight sessions precisely (e.g., 145 minutes = 2h 25m)
- `active_preset`: References a preset by name; must exist in the `presets` array
- `openrouter_api_key`: Stored as a secure reference, not plaintext; Convocations writes the actual secret to the OS keyring when possible or encrypts it locally with the per-device master key mentioned above
- `output_target` / `output_directory_override`: Maintain the GUI toggle between "Output File" and "Output Directory" and keep the companion value for each mode
- Built-in presets are automatically restored if missing during config load
- Preset names must be unique; duplicates are removed with a warning

## Configuration Loading Process

The `load_config()` function in `crates/rconv-core/src/config.rs` follows this priority order:

1. **Primary**: Attempt to load `config.toml`
   - If valid, return the configuration with any sanitization warnings
   - If parsing fails, log warning and continue to next step

2. **Fallback**: Attempt to migrate from `settings.json`
   - If valid JSON is found, convert to TOML structure
   - Immediately persist the new `config.toml`
   - Legacy JSON file is preserved
   - If parsing fails, log warning and continue to next step

3. **Default**: Return default configuration
   - Built-in presets are included
   - All settings use documented defaults

## Configuration Validation

The `sanitize_config()` function enforces these invariants:

- **Schema version**: Must match current version (1), otherwise reset to defaults
- **Preset uniqueness**: Duplicate preset names are removed with warnings
- **Built-in presets**: Missing built-ins are automatically restored
- **Active preset**: Must reference an existing preset name
- **Duration validation**: Hours must be finite and ≥ 1.0
- **Preset validation**: duration_minutes must be non-zero, file_prefix must be non-empty
- **Runtime validation**: Applies `validate_config()` from runtime.rs to catch contradictory settings

Warnings are collected and returned with the sanitized configuration for display to the user.
