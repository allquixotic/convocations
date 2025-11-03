#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: set-macos-secrets.sh --p12 <path> --p12-password <password> \
                            --api-key-file <path> --api-key-id <id> \
                            --api-key-issuer <issuer-id> --team-id <team-id>
       [--repo <owner/name>]

Requires the GitHub CLI to be authenticated with permission to write secrets
for the target repository (default: allquixotic/convocations).
USAGE
}

REPO="allquixotic/convocations"
P12=""
P12_PASSWORD=""
API_KEY_FILE=""
API_KEY_ID=""
API_KEY_ISSUER=""
TEAM_ID=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo)
      REPO="$2"
      shift 2
      ;;
    --p12)
      P12="$2"
      shift 2
      ;;
    --p12-password)
      P12_PASSWORD="$2"
      shift 2
      ;;
    --api-key-file)
      API_KEY_FILE="$2"
      shift 2
      ;;
    --api-key-id)
      API_KEY_ID="$2"
      shift 2
      ;;
    --api-key-issuer)
      API_KEY_ISSUER="$2"
      shift 2
      ;;
    --team-id)
      TEAM_ID="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ -z "$P12" || -z "$P12_PASSWORD" || -z "$API_KEY_FILE" || -z "$API_KEY_ID" || -z "$API_KEY_ISSUER" || -z "$TEAM_ID" ]]; then
  echo "Missing required arguments" >&2
  usage >&2
  exit 1
fi

if [[ ! -f "$P12" ]]; then
  echo "Developer ID certificate file not found: $P12" >&2
  exit 1
fi

if [[ ! -f "$API_KEY_FILE" ]]; then
  echo "Apple API key file not found: $API_KEY_FILE" >&2
  exit 1
fi

if ! command -v gh >/dev/null 2>&1; then
  echo "gh CLI is required" >&2
  exit 1
fi

CERT_B64=$(base64 <"$P12" | tr -d '\n')
API_KEY_B64=$(base64 <"$API_KEY_FILE" | tr -d '\n')

# Set secrets via GitHub CLI
GH_REPO_FLAG=(--repo "$REPO")

echo "Setting GitHub secrets for $REPO"

gh secret set APPLE_CERTIFICATE "${GH_REPO_FLAG[@]}" --body "$CERT_B64"

gh secret set APPLE_CERTIFICATE_PASSWORD "${GH_REPO_FLAG[@]}" --body "$P12_PASSWORD"

gh secret set APPLE_API_KEY "${GH_REPO_FLAG[@]}" --body "$API_KEY_ID"

gh secret set APPLE_API_KEY_BASE64 "${GH_REPO_FLAG[@]}" --body "$API_KEY_B64"

# Team ID and issuer are not secret but we keep them alongside the rest

gh secret set APPLE_API_ISSUER "${GH_REPO_FLAG[@]}" --body "$API_KEY_ISSUER"

gh secret set APPLE_TEAM_ID "${GH_REPO_FLAG[@]}" --body "$TEAM_ID"

echo "All macOS signing secrets have been stored in GitHub." 
