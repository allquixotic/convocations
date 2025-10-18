# OpenRouter OAuth PKCE Notes

This page tracks the OAuth2 PKCE integration that Convocations uses to let users link their OpenRouter account from the desktop UI.

## Flow Summary

1. The frontend asks the Tauri backend (`/api/openrouter/oauth/start`) to initiate the OAuth flow.
2. The backend generates a PKCE verifier/challenge pair, persists the verifier in the in-memory session store, and opens an embedded Tauri webview pointed at OpenRouter's `/auth` endpoint.
3. Once the user authorizes Convocations, OpenRouter redirects back to the callback handler (`/api/openrouter/oauth/callback`), which exchanges the authorization code for an API key and stores it in the config file. The backend then emits an `openrouter-auth-complete` event for the frontend to show success/failure.

## Root Cause of the 409 "Failed to create or update app while creating auth code"

OpenRouter rejects authorization attempts if the callback/referrer combination does not match the application configuration stored on their side. When we originally launched the in-app browser we bound our internal HTTP server to an ephemeral port and passed that changing port to OpenRouter. Their backend responded with HTTP `409` and logged the error shown above because every login attempt appeared to originate from a new unrecognized redirect URL.

## Fix

- The backend now prefers binding the local API server to `http://localhost:3000` (overridable via the `RCONV_HTTP_PORT` environment variable). This gives the OAuth flow a stable `callback_url` and `referrer`.
- If port `3000` is unavailable Convocations still falls back to a random high port, but in that case you must register the alternate URL with OpenRouter (or free the preferred port) before OAuth will succeed.
- The generated `/auth` URL now always includes:
  - `client_id=convocations`
  - `callback_url=http://localhost:<port>/api/openrouter/oauth/callback`
  - `referrer=http://localhost:<port>`
  - PKCE values (`code_challenge`, `code_challenge_method`).

With a consistent host/port combo OpenRouter accepts the authorize action and redirects back to the Tauri callback, allowing the key exchange to complete.

## Secret Storage

- After exchanging the authorization code, Convocations tries to store the OpenRouter key in the operating system keyring (macOS Keychain, Windows Credential Manager, Linux Secret Service, etc.).
- If the keyring is unavailable, the key is encrypted with a randomly generated master key that lives in `~/.config/convocations/secret.key` (permissions `0600`). The encrypted blob is written to `config.toml`; no plaintext secrets are persisted.
- The runtime config now serializes `openrouter_api_key` as a `{ backend = "keyring" | "local-encrypted", ... }` object so callers know how to retrieve the secret without ever exposing the raw value.

## CLI Helpers

- `convocations secret set-openrouter-key` – prompts for the key (input is hidden) and stores it using the same secure workflow as the GUI.
- `convocations secret clear-openrouter-key` – removes the stored secret from the keyring/encrypted store and clears the config reference.

## Troubleshooting

- **HTTP 409 continues to appear**  
  Ensure the Convocations process is actually listening on the expected port. You can override the port with `RCONV_HTTP_PORT=XXXX cargo gui-dev` and update the registered callback/referrer URLs in your OpenRouter application settings to match.

- **Window closes without saving the key**  
  Check the backend logs for `OAuth webview navigating to ...` lines. If you never see the callback URL, the authorization probably failed upstream; inspect the in-window dev tools Console for the exact OpenRouter error.

- **Port 3000 already in use**  
  Set `RCONV_HTTP_PORT` to an available port before launching `cargo gui-dev`. Update your OpenRouter app's allowed callback and referrer URLs accordingly.
