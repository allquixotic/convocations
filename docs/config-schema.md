# Unified Configuration Plan

## Current `settings.json` Usage

- `src-tauri/src/main.rs` persists `ConvocationsConfig` verbatim as JSON at `~/.config/convocations/settings.json`, exposing it through `/api/settings`, `/api/validate`, and `/api/process`. The helpers `load_settings`, `save_settings`, and `validate_config` enforce the same invariants as the CLI.
- `crates/rconv-core/src/runtime.rs` defines `ConvocationsConfig`, converts it to and from CLI `Args`, and drives the processing pipeline (mode selection, date math, stage toggles, LLM hooks, output naming). Defaults in `ConvocationsConfig::default()` mirror the CLI defaults so GUI and CLI stay identical.

| Field | Default | Consumers |
| --- | --- | --- |
| `last: u32` | 0 | GUI/CLI weeks-ago selector → `calculate_dates_for_event` |
| `dry_run: bool` | false | Short-circuits execution after logging computed paths |
| `infile: String` | `~/Documents/.../ChatLog.log` | Source log path; validated in `validate_config`, used for IO |
| `start` / `end: Option<String>` | `None` | Custom ISO date bounds; validated to prevent clashes with events |
| `rsm7`, `rsm8`, `tp6: bool` | false | Mutually exclusive event presets; choose preset + timezone math |
| `one_hour`, `two_hours: bool` | false | Duration overrides; validated for exclusivity |
| `process_file: Option<String>` | `None` | Switches to pre-filtered mode and bypasses date filtering |
| `format_dialogue: bool` | true | Enables formatting stage for filtered mode |
| `cleanup: bool` | true | Keeps cleanup stage active (GUI exposes toggle) |
| `use_llm: bool` | true | Turns Gemini corrections on/off; warnings if API key missing |
| `keep_orig: bool` | false | Retains `_unedited` output when LLM is on |
| `no_diff: bool` | false | Skips diff generation (GUI checkbox) |
| `outfile: Option<String>` | `None` | Overrides generated path; validated for directory existence |

## Proposed `config.toml` Schema

- Location: `~/.config/convocations/config.toml`.
- File carries a `schema_version` so future migrations can be handled without guessing.
- Runtime fields (shared with CLI) live under `[runtime]`. GUI-only state uses `[ui]`. Presets are described once under `[[presets]]` and referenced by identifier.
- Duration override follows the upcoming "checkbox + hours picker" UX: `enabled` toggles the override, `hours` stores the numeric value.
- Built-in presets adopt the new names ("Saturday 10pm-midnight", "Tuesday 7pm") while preserving their semantics (file prefix, timezone, default duration). User-defined presets share the same structure with `builtin = false`.

```toml
schema_version = 1

[runtime]
chat_log_path = "~/Documents/Elder Scrolls Online/live/Logs/ChatLog.log"
active_preset = "saturday-10pm-midnight"
weeks_ago = 0                       # persisted GUI/CLI selection
dry_run = false
use_ai_corrections = true           # replaces `use_llm`
keep_original_output = false        # inverse of `keep_orig`
show_diff = true                    # mirror of `!no_diff`
cleanup_enabled = true
format_dialogue_enabled = true      # internal flag kept for CLI parity
outfile_override = ""               # empty string = derive automatically

[runtime.duration_override]
enabled = false
hours = 1.0

[ui]
theme = "dark"                      # allowed: "light", "dark", "system"
show_technical_log = false
follow_technical_log = true         # auto-scroll behaviour

[[presets]]
id = "saturday-10pm-midnight"
name = "Saturday 10pm-midnight"
weekday = "saturday"
timezone = "America/New_York"
start_time = "22:00"
duration_minutes = 145              # 2h 25m window, crosses midnight
file_prefix = "conv"
default_weeks_ago = 0
builtin = true

[[presets]]
id = "tuesday-7pm"
name = "Tuesday 7pm"
weekday = "tuesday"
timezone = "America/New_York"
start_time = "19:00"
duration_minutes = 60
file_prefix = "rsm7"
default_weeks_ago = 0
builtin = true

[[presets]]
id = "user-..."
name = "Custom preset name"
weekday = "friday"
timezone = "America/New_York"
start_time = "18:30"
duration_minutes = 120
file_prefix = "tp6"
default_weeks_ago = 1
builtin = false
```

### Section Notes

- `runtime.chat_log_path` remains the single source for CLI + GUI default log path.
- `use_ai_corrections`, `keep_original_output`, and `show_diff` map directly onto the existing processing switches (`use_llm`, `keep_orig`, `no_diff`) and give us room to rename the GUI labels.
- `weeks_ago` persists the most recently selected offset so both interfaces can reopen with the same context.
- `duration_minutes` avoids floating-point hours and makes cross-midnight durations explicit; runtime can translate to hours when needed.
- GUI-only toggles (`show_technical_log`, `follow_technical_log`, future theme toggles) stay isolated from CLI state.
- Preset identifiers allow CRUD operations: built-ins stay immutable, user presets can be edited/deleted without affecting the defaults.

## Legacy Handling Strategy

1. On startup, prefer `config.toml`. If it is missing, attempt to read `settings.json` using the existing JSON loader.
2. Successful JSON loads are converted into the new TOML structure (populate `[runtime]`, seed `[[presets]]` with the built-ins, map the stored CLI flags). Persist the TOML immediately and leave the JSON file untouched for a few releases.
3. If JSON parsing fails, log a warning and fall back to default TOML content (per standing request to ignore malformed `settings.json` rather than erroring).
4. Add a small `schema_version` gate so future migrations can evolve safely once additional GUI state lands.
