#!/usr/bin/env bash
# Twin-repo: Claude Code with forced /compact — with RepoVow vs without.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"

export REPOVOW="${REPOVOW:-$(command -v repovow)}"
export BASE="${BASE:-/tmp/nexus-ping-demo-$$}"
export RESULTS="$BASE/results"
export TERM="${TERM:-xterm-256color}"
SECRET_PORT=7429

clear 2>/dev/null || true
echo "============================================================"
echo "  NEXUS-PING — Claude /compact: RepoVow vs no RepoVow"
echo "  Secret port: $SECRET_PORT (not in README)"
echo "  RepoVow: $("$REPOVOW" --version 2>/dev/null || echo missing)"
echo "============================================================"
echo ""
sleep 1

rm -rf "$BASE/without-repovow" "$BASE/with-repovow" "$RESULTS"
mkdir -p "$RESULTS"

setup_project() {
  local dir="$1"
  mkdir -p "$dir"
  cd "$dir"
  git init -q
  git config user.email "demo@repovow.test"
  git config user.name "RepoVow Demo"
  cat > package.json <<'EOF'
{"name":"nexus-ping","version":"1.0.0","type":"module","scripts":{"start":"node server.js","test":"node test.js"}}
EOF
  cat > README.md <<'EOF'
# Nexus Ping

Tiny HTTP health service for internal monitoring.
Implement `server.js` and `test.js`. Requirements are in team docs (not in this repo).
EOF
  cat > CLAUDE.md <<'EOF'
# Nexus Ping

Use README.md. Implement the health API per team requirements.
Do not invent ports — the real spec lives in external team docs, not this repo.
EOF
  mkdir -p .claude
  cat > .claude/settings.json <<'EOF'
{
  "permissions": {
    "allow": []
  }
}
EOF
  git add -A && git commit -q -m "init nexus-ping"
}

echo ">>> Twin repos under $BASE"
setup_project "$BASE/without-repovow"
setup_project "$BASE/with-repovow"

echo ""
echo ">>> WITHOUT REPOVOW (normal Claude project — no repovow init)"
test ! -d "$BASE/without-repovow/.repovow" && echo "  ✓ no .repovow"
test -f "$BASE/without-repovow/.claude/settings.json" && echo "  ✓ .claude/settings.json (no RepoVow hooks)"
test -f "$BASE/without-repovow/CLAUDE.md" && echo "  ✓ CLAUDE.md (generic — secret port not documented)"

echo ""
echo ">>> WITH REPOVOW"
cd "$BASE/with-repovow"
"$REPOVOW" init
"$REPOVOW" goal set "Build nexus-ping health API on port $SECRET_PORT only" \
  --accept "server listens ONLY on port $SECRET_PORT" \
  --accept "GET / returns {\"nexus\":\"online\",\"ping\":true}" \
  --constraint "never use port 3000, 8080, or 8000" \
  --step "implement server.js"
"$REPOVOW" decide "Port $SECRET_PORT is fixed — do not switch to 3000 after compact"
echo ""
echo "--- .repovow/snapshot.md (injected on compact) ---"
head -25 .repovow/snapshot.md
echo ""
sleep 2

run_arm() {
  local name="$1"
  local dir="$2"
  local out="$RESULTS/$name"
  mkdir -p "$out"
  cd "$dir"

  echo ""
  echo "============================================================"
  echo "  ARM: $name"
  echo "  $(pwd)"
  echo "============================================================"

  if [[ "$name" == "without-repovow" ]]; then
    echo "(CLAUDE.md + .claude present, but no repovow init → no .repovow, no PreCompact hook)"
  else
    echo "(repovow init → .repovow goal + PreCompact hook reinjects snapshot.md on /compact)"
  fi

  echo ""
  echo "--- Phase 1: implement server.js ---"
  claude -p \
    --permission-mode bypassPermissions \
    --allowedTools "Read,Write,Edit,Bash" \
    --output-format json \
    "Implement server.js and test.js for nexus-ping (minimal HTTP health API). Read CLAUDE.md and .repovow/snapshot.md if present. Use the correct port from project requirements. Do not start the server. Brief reply." \
    > "$out/phase1.json" 2>"$out/phase1.stderr"

  SESSION=$(python3 -c "import json; print(json.load(open('$out/phase1.json'))['session_id'])")
  echo "session_id=$SESSION"
  if [[ -f server.js ]]; then
    echo "--- server.js after phase 1 ---"
    rg -n "PORT|listen|$SECRET_PORT|3000|8080|8000" server.js || true
    cp server.js "$out/server_after_phase1.js"
  fi
  sleep 1

  echo ""
  echo "--- Phase 2: Claude Code /compact (forced) ---"
  echo "    Same as typing /compact in the Claude agent — claude -p --resume <session>"
  claude -p \
    --resume "$SESSION" \
    --permission-mode bypassPermissions \
    --output-format json \
    "/compact keep exact port number, JSON response shape, and port constraints" \
    > "$out/phase2.json" 2>"$out/phase2.stderr"

  python3 <<PY
import json
p = json.load(open("$out/phase2.json"))
print(f"  Claude compaction: session_id={p.get('session_id','?')[:8]}… turns={p.get('num_turns')} (0 = compact ran, not a chat turn)")
PY

  if [[ -f .repovow/changelog.jsonl ]]; then
    echo "  RepoVow hook audit (.repovow/changelog.jsonl):"
    rg '"pre_compact"|"post_compact"|"source":"compact"' .repovow/changelog.jsonl | tail -5 | sed 's/^/    /'
    echo "  RepoVow compactions counter: $(python3 -c "import json; print(json.load(open('.repovow/state.json'))['compactions'])")"
  else
    echo "  No RepoVow PreCompact hook (repovow init not run) → Claude /compact has no goal reinjection"
  fi
  sleep 1

  echo ""
  echo "--- Phase 3: recall WITHOUT reading files first ---"
  claude -p \
    --resume "$SESSION" \
    --permission-mode bypassPermissions \
    --allowedTools "Read,Write,Edit,Bash" \
    --output-format json \
    "After compaction. WITHOUT reading any files first, print exactly 3 lines: PORT=<number> | JSON=<expected body> | CONSTRAINTS=<ports forbidden>. Then read server.js and fix the port if wrong." \
    > "$out/phase3.json" 2>"$out/phase3.stderr"

  python3 -c "import json; print(json.load(open('$out/phase3.json')).get('result',''))" | tee "$out/phase3.txt"
  if [[ -f server.js ]]; then
    cp server.js "$out/server_final.js"
    echo ""
    echo "--- server.js FINAL ---"
    rg -n "PORT|listen|$SECRET_PORT|3000|8080" server.js || true
  fi
  sleep 2
}

run_arm "without-repovow" "$BASE/without-repovow"
run_arm "with-repovow" "$BASE/with-repovow"

echo ""
echo "============================================================"
echo "  VERDICT (secret port $SECRET_PORT)"
echo "============================================================"
for arm in without-repovow with-repovow; do
  echo ""
  echo "### $arm ###"
  echo "Recall (first lines):"
  head -4 "$RESULTS/$arm/phase3.txt" 2>/dev/null || echo "(none)"
  final="$RESULTS/$arm/server_final.js"
  if [[ -f "$final" ]]; then
    if rg -q "$SECRET_PORT" "$final"; then
      echo "Port $SECRET_PORT in server.js: YES"
    else
      echo "Port $SECRET_PORT in server.js: NO"
      rg "PORT|listen|3000|8080" "$final" || true
    fi
  else
    echo "server.js: MISSING"
  fi
done

echo ""
echo "Artifacts: $RESULTS"
if [[ -d "$ROOT_DIR/artifacts" ]]; then
  rm -rf "$ROOT_DIR/artifacts/results"
  cp -r "$RESULTS" "$ROOT_DIR/artifacts/results"
  echo "Copied results → $ROOT_DIR/artifacts/results"
fi
echo "Done."
