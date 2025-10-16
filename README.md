# Convocations - ESO Chat Log Formatter

A simple tool that turns your Elder Scrolls Online chat logs into clean, readable posts.

## What Does It Do?

After your roleplay sessions, this tool:
- Takes your ESO chat log
- Extracts just the roleplay conversations (say and emote)
- Formats it like a book with proper dialogue
- Fixes spelling and grammar mistakes using AI
- Saves it as a text file you can share or archive

**Before**: Raw chat log with timestamps, channels, and system messages  
**After**: Clean narrative like "Valandil says, 'The moon is beautiful tonight.'"

## Download

Go to the [Releases page](https://github.com/allquixotic/convocations/releases) and download:
- **Windows**: `rconv-windows-x86_64.zip`
- **Mac**: `rconv-macos-universal.tar.gz`

## Installation

### Windows
1. Unzip the downloaded file
2. Put `rconv-windows-x86_64.exe` wherever you like
3. Double-click to run it (or use PowerShell or Terminal if you prefer)

### Mac
1. Double-click the `.tar.gz` file to unzip it
2. Put the `rconv-macos-universal` file wherever you like
3. Open Terminal and drag the file in to run it

**Note**: Mac users may need to allow the app in System Settings > Privacy & Security if you get a warning about the developer.

## Quick Start

The simplest way to use the tool:

1. Open your terminal (Command Prompt, PowerShell or Terminal app on Windows, Terminal on Mac)
2. Navigate to where you put the program
3. Run: `convocations` (or `./rconv-macos-universal` on Mac)

This will automatically:
- Find last Saturday's chat session (10pm-12:25am)
- Process the chat log
- Save it as `conv-MMDDYY.txt` in the current folder

You can change the behavior of the tool with some of these optional parameters:

### Last Week's Session
```
convocations --last 1
```

### Two Weeks Ago
```
convocations --last 2
```

### Tuesday Night Events (RSM at 7pm)
```
convocations --rsm7
```

### Tuesday Night Events (RSM at 8pm)
```
convocations --rsm8
```

### Friday Events (TP at 6pm)
```
convocations --tp6
```

### Longer Events
```
convocations --2h
```
Use `--2h` for 2-hour events, or `--1h` for 1-hour events. The default is 1-hour for --rsm7, --rsm8 or --tp6, and 2-hour for Convocations, which is the default.

### Without AI Corrections
```
convocations --llm=false
```
If you don't want AI to fix spelling/grammar. There won't be any AI invocation unless you configure it, anyway.

### See What It Would Do
```
convocations --dry-run
```
Shows what it would process without actually creating a file.

## Output Files

The tool creates files with names like:
- `conv-101125.txt` - Saturday sessions
- `rsm7-101425.txt` - RSM7 Tuesday events
- `rsm8-101425.txt` - RSM8 Tuesday events  
- `tp6-101025.txt` - TP6 Friday events

The date format is MMDDYY (month, day, year).

## AI Features

If you have it set up, the tool uses Google's AI to fix spelling and grammar mistakes while keeping:
- Character names exactly as written
- Fantasy/game terms unchanged
- The dialogue format intact

After processing, it shows you a colorized diff highlighting what was changed, then deletes the unedited version. If you want to keep both versions, add `--keep-orig`.

## Tips

- The tool looks in your ESO Documents folder for `ChatLog.log` automatically
- It only captures "say" and "emote" channels (your roleplay)
- All times are converted to your local timezone automatically
- Files are saved in whatever folder you run the command from

## Troubleshooting

**"No log data found"**: The dates might not match when you actually played. Try using `--dry-run` to see what dates it's looking for.

**Empty file**: Make sure you have Save Chat in your ESO settings enabled.

**AI not working**: The tool still works without AI - it just won't fix spelling/grammar.

## Getting Help

For technical details or advanced features, see `WARP.md` in the project folder.

For issues or questions, visit: https://github.com/allquixotic/convocations
