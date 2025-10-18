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

Visit the [Releases page](https://github.com/allquixotic/convocations/releases) and grab the build for your platform:

- **Windows**: `rconv-windows-x86_64.zip`
- **macOS**: `rconv-macos-universal.tar.gz`

Unzip the archive, place the app anywhere you like, then double‑click it. (macOS may ask you to approve the app in **System Settings → Privacy & Security** the first time.)

## Start Here (Desktop App)

1. Launch Convocations.  
2. Leave **Chat Log** selected if you want the latest Saturday session, or choose **Processed Input** if you already exported a chat log.  
3. Decide where the cleaned text should go:
   - Pick **Output File** (default) to save a single document. The app suggests a friendly filename and, when possible, points it to your **Documents** folder.
   - Switch to **Output Directory** if you prefer to drop the results into a folder; the label under the toggle updates instantly.
4. Click **Start Processing**. Watch the progress log for status updates and, if AI corrections are on, a live diff preview of every change.

You can reopen the previous configuration at any time; the app remembers your choices, including the Output File/Directory toggle.

## Optional: Connect OpenRouter (AI Clean‑up)

AI corrections are entirely optional. To enable them:

1. Press **OAuth Login** inside the app.  
2. A Convocations browser window opens on the OpenRouter website. Sign in and click **Authorize**.  
3. When the success message appears, close the window. You’re done—the key is stored securely for future runs.

Convocations saves secrets in your operating system keyring (Keychain on macOS, Credential Manager on Windows, Secret Service on Linux). If a keyring isn’t available, the app encrypts the secret locally before writing to disk.  

Prefer the command line? Run:
```bash
convocations secret set-openrouter-key
```
Paste your key when prompted (input stays hidden). Remove it later with:
```bash
convocations secret clear-openrouter-key
```

## Power Users: Command Line

You can still run everything from a terminal:

```bash
convocations --last 1          # last week’s Saturday event
convocations --rsm7            # Tuesday 7 pm event
convocations --process-file exported.txt
convocations --outfile ~/Documents/conv-output.txt
convocations --llm=false       # skip AI clean-up
```

The CLI writes files to your current working directory unless you give `--outfile` or set the `CONVOCATIONS_WORKING_DIR` environment variable to a folder of your choice.

## Output Files

By default the app produces tidy filenames based on the preset you picked:
- `conv-101125.txt` - Saturday sessions
- `rsm7-101425.txt` - RSM7 Tuesday events
- `rsm8-101425.txt` - RSM8 Tuesday events  
- `tp6-101025.txt` - TP6 Friday events

The date format is MMDDYY (month, day, year).

## AI Features

If you turn on AI helpers, the tool can ask your preferred OpenRouter model to fix spelling and grammar mistakes while keeping:
- Character names exactly as written
- Fantasy/game terms unchanged
- The dialogue format intact

After processing, you’ll see a colorized diff preview in the app that highlights the exact edits. The original `_unedited` file is cleaned up automatically unless you choose **Keep original output**.

## Tips

- The tool looks in your ESO Documents folder for `ChatLog.log` automatically.
- Only “say” and “emote” channels are kept—guild, party, and system chatter are ignored.
- Times are converted to your local timezone automatically.
- Toggle **Show technical log** if you’re curious; it streams every step the processor takes.

## Troubleshooting

**"No log data found"**: The dates might not match when you actually played. Try using `--dry-run` to see what dates it's looking for.

**Empty file**: Make sure you have Save Chat in your ESO settings enabled.

**AI not working**: The tool still works without AI - it just won't fix spelling/grammar.

**OAuth login errors**: If the OpenRouter page shows an error, close the window and try again. You can always paste a key manually with the CLI command above.

## Getting Help

For technical details or advanced features, see `WARP.md` in the project folder.

For issues or questions, visit: https://github.com/allquixotic/convocations
