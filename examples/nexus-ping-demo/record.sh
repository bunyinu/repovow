#!/usr/bin/env bash
# Record demo.sh with asciinema; optional GIF via agg (github.com/asciinema/agg).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"
OUT_DIR="$ROOT/artifacts"
mkdir -p "$OUT_DIR"
CAST="$OUT_DIR/demo.cast"
GIF="$ROOT/demo.gif"
LOG="$OUT_DIR/record.log"

export REPOVOW="${REPOVOW:-$(command -v repovow)}"
export TERM="${TERM:-xterm-256color}"

echo "Recording to $CAST (this takes ~10–20 min — 6 Claude calls)..."
asciinema rec --overwrite -q -c "bash '$ROOT/demo.sh'" "$CAST" </dev/null 2>&1 | tee "$LOG"

if command -v agg >/dev/null 2>&1; then
  echo "Rendering GIF..."
  agg "$CAST" "$GIF"
  echo "GIF: $GIF"
else
  echo "Install agg for GIF: cargo install --git https://github.com/asciinema/agg"
  echo "Replay: asciinema play $CAST"
fi
