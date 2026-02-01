# Convocations Feature List

## Overview

Convocations is a specialized tool that transforms Elder Scrolls Online (ESO) chat logs from role-playing sessions into clean, readable narrative text. It extracts dialogue and emotes from game sessions, removes out-of-character chatter, and optionally applies AI-powered grammar and spelling corrections.

---

## 1. Core Processing Features

### 1.1 Chat Log Extraction
- Automatically locates and reads ESO chat log files from the default game directory
- Filters chat to include only say (dialogue) and emote (action/roleplay) channels
- Removes guild chat, party chat, system messages, and all other non-roleplay content
- Reconstructs multi-line messages that span multiple log entries
- Preserves exact character names and fantasy terminology

### 1.2 Date/Time Filtering
- Filters chat logs to specific date and time ranges based on event schedules
- Supports four built-in event presets with pre-configured schedules:
  - **Saturday 10pm-midnight**: Saturday events (2 hours 25 minutes)
  - **Tuesday 7pm (RSM7)**: Tuesday evening events (1 hour)
  - **Tuesday 8pm (RSM8)**: Tuesday evening events (1 hour)
  - **Friday 6pm (TP6)**: Friday evening events (1 hour)
- Automatically handles timezone conversions (Eastern Time base)
- Correctly handles Daylight Saving Time transitions
- Supports "weeks ago" navigation to process past events (e.g., last week, two weeks ago)

### 1.3 Text Cleanup
- Removes out-of-character markers: `((text))` and `[[text]]`
- Normalizes smart quotes (curly quotes) to standard straight quotes
- Converts ellipsis characters (...) to three periods (...)
- Ensures proper sentence-ending punctuation

### 1.4 Dialogue Formatting
- Formats say-channel text as: `Character says, "dialogue text"`
- Formats emote-channel text as: `Character performs action` or quoted dialogue
- Maintains proper attribution to each character

### 1.5 AI-Powered Corrections
- Optional grammar and spelling corrections using AI language models
- Preserves character names and fantasy terminology exactly as written
- Gracefully falls back to original text if AI service is unavailable
- Generates before/after comparison (diff) showing all changes made
- Option to retain the unedited version alongside the corrected output

---

## 2. Event Preset System

### 2.1 Built-in Presets
- Four pre-configured event types for common roleplay schedules
- Each preset includes: day of week, start time, duration, timezone, and filename prefix
- Built-in presets cannot be modified or deleted (protected)

### 2.2 Custom Presets
- Create unlimited custom presets for any recurring event
- Configure: name, day of week, timezone, start time, duration, filename prefix
- Set default "weeks ago" value per preset
- Edit existing custom presets
- Delete custom presets when no longer needed
- Custom presets persist across sessions

---

## 3. Output Options

### 3.1 Output File Naming
- Automatic filename generation using preset prefix and date (e.g., `conv-101125.txt`)
- Custom filename override option
- Output to specific directory option
- Dry-run mode to preview what would be processed without creating files

### 3.2 Output Format
- Clean plain text files in UTF-8 encoding
- Formatted dialogue with proper character attribution
- No timestamps, system messages, or out-of-character content in output

### 3.3 Diff Generation
- Side-by-side comparison of original vs. AI-corrected text
- Color-coded additions and deletions
- Option to suppress diff generation
- Option to keep the original (unedited) version

---

## 4. AI Model Integration

### 4.1 OpenRouter Integration
- Connects to OpenRouter API for AI language model access
- Supports dozens of AI models from multiple providers
- Secure API key storage using operating system keychain

### 4.2 Model Selection
- **Automatic mode**: System selects the best available model
- **Curated list mode**: Choose from pre-vetted models with quality ratings
- **Manual mode**: Enter any OpenRouter model identifier
- Filter to show only free models
- Display pricing information (cost per million tokens in/out)
- Display AI quality scores (AAII - AI Alignment Improvement Index)
- Model tier indicators: Free, Cheap

### 4.3 OAuth Authentication
- Secure OAuth login flow for OpenRouter
- No manual API key copying required
- Automatic token storage and retrieval

---

## 5. Command-Line Interface (CLI)

### 5.1 Main Processing Command
- Process chat logs with sensible defaults (zero-configuration possible)
- Override any setting via command-line flags
- Specify input file location
- Specify output file or directory
- Enable/disable individual processing stages

### 5.2 Event Selection Flags
- `--rsm7`: Tuesday 7pm event
- `--rsm8`: Tuesday 8pm event
- `--tp6`: Friday 6pm event
- `--preset <name>`: Any preset by name
- `--last <N>`: Go back N weeks

### 5.3 Time Override Flags
- `--start <datetime>`: Custom start date/time
- `--end <datetime>`: Custom end date/time
- `--1h`: Force 1-hour duration
- `--2h`: Force 2-hour duration
- `--duration-hours <N>`: Custom duration (supports decimals like 1.5)

### 5.4 Processing Flags
- `--cleanup`: Enable/disable OOC removal and text normalization
- `--llm`: Enable/disable AI corrections
- `--keep-orig`: Retain unedited version after AI corrections
- `--no-diff`: Skip diff generation
- `--dry-run`: Preview without writing files

### 5.5 Model Flags
- `--model <id>`: Select specific AI model
- `--list-curated`: Display all available curated models
- `--free-models-only`: Filter to free models only

### 5.6 Preset Management Subcommand
- `preset list`: View all presets with their configurations
- `preset show`: View details of a specific preset
- `preset create`: Create a new custom preset
- `preset update`: Modify an existing preset
- `preset delete`: Remove a custom preset

### 5.7 Secret Management Subcommand
- `secret set-openrouter-key`: Store API key securely (with hidden input)
- `secret clear-openrouter-key`: Remove stored API key

---

## 6. Graphical User Interface (GUI)

### 6.1 Mode Selection
- **Chat Log Mode**: Process raw ESO chat logs with automatic date filtering
- **Processed Input Mode**: Process pre-filtered text files

### 6.2 Configuration Panel
- Preset dropdown with all built-in and custom presets
- Visual preview of schedule (day, time, timezone, duration)
- "Weeks Ago" selector for processing past events
- Duration override with preset options (1 hour, 2 hours, custom)

### 6.3 Processing Options
- Toggle: Enable cleanup (OOC removal, punctuation normalization)
- Toggle: Enable dialogue formatting
- Toggle: Enable AI corrections
- Toggle: Keep original file when using AI
- Toggle: Show diff after processing

### 6.4 Input/Output Configuration
- Text input for chat log file path with path expansion
- Auto-detection of default ESO logs location
- Toggle between file output and directory output modes
- File browser dialogs for selecting locations
- Display of default output location

### 6.5 Model Selection Panel
- Three selection modes: Automatic, Curated List, Manual Entry
- Curated model dropdown with details
- Pricing display for selected model
- Quality score display (AAII)
- "Free models only" filter checkbox

### 6.6 Authentication
- OAuth Login button for OpenRouter
- Secure webview-based login flow
- Status indicator showing when API key is saved
- No manual key entry required

### 6.7 Processing Controls
- Start Processing button
- Dry-run mode checkbox
- Real-time progress log
- Collapsible technical log view
- Auto-scroll toggle for log
- Live diff preview during processing

### 6.8 Results Display
- Success notifications with output file path
- Error messages with detailed explanations
- Progress tracking with stage names and elapsed times
- Color-coded diff preview (additions in green, deletions in red)
- Clickable output file paths

### 6.9 Preset Management
- Create new preset form with all configuration options
- Edit existing preset in-place
- Delete preset with confirmation
- Protection against modifying built-in presets

### 6.10 Theme Support
- Dark mode (default)
- Light mode
- System preference mode (follows OS setting)
- Consistent theming across all UI elements

### 6.11 Settings Persistence
- All configuration automatically saved
- Remembers: active preset, weeks ago, all toggles, paths, model selection
- Remembers: theme preference, log visibility, auto-scroll setting
- Settings survive application restarts

---

## 7. Security Features

### 7.1 API Key Storage
- **Primary**: Operating system keychain (macOS Keychain, Windows Credential Manager, Linux Secret Service)
- **Fallback**: AES-256 encrypted local storage with device-specific master key
- No plaintext secrets in configuration files

### 7.2 OAuth Security
- Industry-standard PKCE (Proof Key for Code Exchange) flow
- Secure callback handling
- Session state tracking with timeout cleanup

---

## 8. Configuration Storage

### 8.1 Configuration File
- Automatic configuration file in standard user config directory
- Automatic migration from legacy formats
- Human-readable format for advanced users

### 8.2 Stored Settings
- Chat log file path
- Active preset selection
- Weeks ago default
- All processing toggles (cleanup, AI, keep original, show diff)
- Output file/directory preferences
- Model selection and free-only filter
- Theme preference
- All custom presets

---

## 9. Validation & Error Handling

### 9.1 Input Validation
- File existence checks with clear error messages
- Date range validation
- Duration constraints (minimum 1 hour, positive values)
- Preset name uniqueness enforcement
- Mutually exclusive option detection

### 9.2 Processing Feedback
- "No data in range" warnings with searched dates displayed
- Empty output detection with remediation suggestions
- API failure notifications with fallback behavior
- Detailed error messages for troubleshooting

### 9.3 Constraint Enforcement
- Cannot combine conflicting event type flags
- Cannot combine conflicting duration flags
- Cannot use event flags with custom start/end times
- Cannot output to directory when file output is configured

---

## 10. Advanced Features

### 10.1 Pre-Filtered File Processing
- Skip date filtering and process already-exported text
- Useful for re-processing or processing logs from other sources

### 10.2 Custom Date Ranges
- Override any preset with specific start and end times
- Full ISO 8601 date/time format support

### 10.3 Flexible Duration
- Override default duration for any event
- Support for fractional hours (e.g., 1.5 hours)

### 10.4 Curated Model System
- Pre-vetted model list with quality assessments
- Embedded fallback snapshot when network unavailable
- Remote snapshot updates with timeout handling
- Multiple provider support through OpenRouter

---

## 11. Environment Customization

- Override HTTP port for local API server
- Override working directory for output files
- Custom model snapshot location for testing

---

## 12. Output Characteristics

- **Format**: Plain text (.txt)
- **Encoding**: UTF-8
- **Naming**: Configurable prefix with automatic date stamping
- **Content**: Clean, formatted roleplay dialogue only
- **Preserved**: Character names, fantasy terminology, dialogue structure
- **Removed**: Timestamps, OOC markers, system messages, non-roleplay chat
