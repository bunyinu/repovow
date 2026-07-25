#!/usr/bin/env bash
# Deploy RepoVow Cloud to Render (requires git remote + RENDER_API in env).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ -z "${RENDER_API:-}" ]]; then
  echo "Set RENDER_API to your Render API key" >&2
  exit 1
fi

OWNER_ID="${RENDER_OWNER_ID:-tea-d6vircs50q8c739ltq6g}"
SERVICE_NAME="${RENDER_SERVICE_NAME:-repovow-cloud}"

echo "==> Checking for existing service..."
EXISTING=$(curl -sS -H "Authorization: Bearer $RENDER_API" \
  "https://api.render.com/v1/services?limit=50" | \
  python3 -c "import sys,json; d=json.load(sys.stdin); print(next((x['service']['id'] for x in d if x.get('service',{}).get('name')=='${SERVICE_NAME}'), ''))" 2>/dev/null || true)

if [[ -n "$EXISTING" ]]; then
  echo "Service exists: $EXISTING"
  echo "Trigger deploy: https://dashboard.render.com/web/$EXISTING"
  curl -sS -X POST -H "Authorization: Bearer $RENDER_API" \
    "https://api.render.com/v1/services/${EXISTING}/deploys" \
    -H "Content-Type: application/json" \
    -d '{"clearCache":"do_not_clear"}' | python3 -m json.tool 2>/dev/null || true
  exit 0
fi

REMOTE=$(git remote get-url origin 2>/dev/null || true)
if [[ -z "$REMOTE" ]]; then
  echo "No git remote. To deploy on Render:"
  echo "  1. Push this repo to GitHub"
  echo "  2. Connect repo at https://dashboard.render.com/blueprints"
  echo "  3. Select render.yaml (Blueprint)"
  echo ""
  echo "Or run locally: cargo run --release --bin repovow-server"
  exit 1
fi

# Normalize github URL for Render
REPO_URL="$REMOTE"
REPO_URL="${REPO_URL%.git}"
REPO_URL="${REPO_URL/git@github.com:/https://github.com/}"

echo "==> Creating Render web service from $REPO_URL ..."
curl -sS -X POST -H "Authorization: Bearer $RENDER_API" \
  -H "Content-Type: application/json" \
  "https://api.render.com/v1/services" \
  -d "$(python3 - <<EOF
import json
print(json.dumps({
  "type": "web_service",
  "name": "${SERVICE_NAME}",
  "ownerId": "${OWNER_ID}",
  "repo": "${REPO_URL}",
  "branch": "main",
  "runtime": "docker",
  "plan": "free",
  "serviceDetails": {
    "env": "docker",
    "healthCheckPath": "/health",
    "disk": {
      "name": "repovow-data",
      "mountPath": "/data",
      "sizeGB": 1
    },
    "envVars": [
      {"key": "REPOVOW_DB_PATH", "value": "/data/repovow.db"},
      {"key": "RUST_LOG", "value": "info"}
    ]
  }
}))
EOF
)" | python3 -m json.tool

echo "Deploy started. Check https://dashboard.render.com"
