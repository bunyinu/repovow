# RepoVow — Full Handoff (rebuild from scratch)

**Version:** 0.5.3
**Repo:** https://github.com/bunyinu/repovow
**npm:** `repovow` (publisher `bunyinu`)
**Cloud:** https://keel-cloud.onrender.com (legacy immutable Render hostname; service name is `repovow-cloud`)
**Last updated:** 2026-06-23
**Purpose:** Enough detail that a new engineer can **rebuild, deploy, and ship** RepoVow as it exists today — CLI, npm, and cloud — without oral history.

---

## Table of contents

1. [What you are rebuilding (3 artifacts)](#1-what-you-are-rebuilding-3-artifacts)
2. [Prerequisites & accounts](#2-prerequisites--accounts)
3. [Repo map](#3-repo-map)
4. [Rebuild the Rust CLI](#4-rebuild-the-rust-cli)
5. [Rebuild the npm distribution](#5-rebuild-the-npm-distribution)
6. [Rebuild RepoVow Cloud (server)](#6-rebuild-repovow-cloud-server)
7. [Deploy RepoVow Cloud to Render](#7-deploy-repovow-cloud-to-render) ← **deployment**
8. [Release pipeline (tag → npm + GitHub)](#8-release-pipeline-tag--npm--github)
9. [Secrets & environment variables](#9-secrets--environment-variables)
10. [End-to-end verification checklist](#10-end-to-end-verification-checklist)
11. [Cloud HTTP API](#11-cloud-http-api)
12. [SQLite schema](#12-sqlite-schema)
13. [Hook wiring (agent integration)](#13-hook-wiring-agent-integration)
14. [Product summary](#14-product-summary)
15. [Demos & proof assets](#15-demos--proof-assets)
16. [Sales narrative](#16-sales-narrative)
17. [Failed approaches (do not retry)](#17-failed-approaches-do-not-retry)
18. [Open decisions](#18-open-decisions)
19. [Key links](#19-key-links)

---

## 1. What you are rebuilding (3 artifacts)

RepoVow is **not one binary**. It is three shipped artifacts:

| # | Artifact | What it is | How users get it |
|---|----------|------------|------------------|
| **A** | `repovow` CLI | Rust binary: goals, hooks, policy, cloud sync | `npm install -g repovow` |
| **B** | npm packages | Node shim + 4 platform native binaries | Published on tag via GitHub Actions |
| **C** | `repovow-server` | Rust Axum server + SQLite + static `web/` | Docker on Render |

**Data flow:**

```
Developer repo                    RepoVow Cloud (Render)
─────────────                    ───────────────────
.repovow/state.json  ──push/pull──►  SQLite projects.state_json
.repovow/snapshot.md                 projects.snapshot_md
.claude/settings.json             (not stored — local hooks only)
     │
     └── hooks call `repovow hook …` on compact / tool / stop
```

**Core product idea (do not deviate):** Task context lives in the **git repo** (`.repovow/`), not the chat. Hooks **reinject** on Claude `/compact` and **block** bad tools / premature stop. Optional cloud = fleet dashboard + sync.

---

## 2. Prerequisites & accounts

### Machine

| Tool | Version | Why |
|------|---------|-----|
| Rust | stable (2021 edition) | CLI + server |
| Node.js | 18+ | npm shim, publish scripts |
| cargo | comes with Rust | build |
| git | any | release tags |
| Docker | optional | local server smoke test |

### Accounts (production)

| Service | Used for |
|---------|----------|
| **GitHub** | Source repo, Actions release, GitHub Releases |
| **npm** | Unscoped `repovow` package + 4 unscoped platform packages |
| **Render** | Host `repovow-cloud` (Docker web service + persistent disk) |
| **Stripe** | Team plan payment link (env on Render) |

### npm packages to create (one-time)

If rebuilding npm from zero, publish these packages from the `bunyinu` account:

- `repovow` — main package (shim only)
- `repovow-linux-x64-gnu`
- `repovow-linux-arm64-gnu`
- `repovow-darwin-x64`
- `repovow-darwin-arm64`

All five packages use npm trusted publishing for `bunyinu/repovow` and
`.github/workflows/release.yml`. GitHub Actions exchanges its OIDC identity for a
short-lived publish credential and records provenance; no npm token is stored in
GitHub.

---

## 3. Repo map

```
compo1/  (repovow)
├── Cargo.toml              # version source of truth; two bins: repovow, repovow-server
├── src/
│   ├── main.rs             # CLI entry
│   ├── lib.rs              # module exports
│   ├── bin/repovow_server.rs  # cloud server entry
│   ├── install.rs          # repovow init — hooks + CLAUDE.md merge
│   ├── hooks.rs            # repovow hook <event> — agent callback
│   ├── state.rs            # RepoVowState, RepoVowConfig
│   ├── snapshot.rs         # complete snapshot.md renderer
│   ├── context.rs          # token-budgeted agent context compiler
│   ├── policy.rs           # signed goals (ECDSA P-256 default)
│   ├── constraints.rs      # PreToolUse constraint guard
│   ├── loop_breaker.rs     # PreToolUse retry block
│   ├── acceptance.rs       # Stop hook gate
│   ├── check.rs            # repovow check (CI)
│   ├── cloud.rs            # push/pull to RepoVow Cloud
│   ├── server/             # Axum routes + db.rs (SQLite)
│   └── …
├── web/                    # Static HTML/CSS served by repovow-server
│   ├── index.html          # landing
│   ├── start.html          # sign-in / create project
│   ├── pricing.html, trust.html, team.html
│   ├── dashboard.html, dashboard-edit.html
│   ├── demo.gif            # homepage embed
│   └── site.css
├── npm/
│   ├── repovow-cli/           # repovow — bin/repovow.js shim
│   └── platforms/*/        # per-OS native binary packages
├── scripts/
│   ├── stage-npm.sh        # copy release repovow → npm/platforms
│   ├── release.sh          # local: test + stage + optional global install
│   └── deploy-render.sh    # trigger Render deploy via API
├── Dockerfile              # builds repovow-server for Render
├── render.yaml             # Render Blueprint spec
├── .github/workflows/
│   ├── ci.yml              # PR: fmt, clippy, test, npm shim verify
│   └── release.yml         # tag v*.*.* → binaries + npm publish
└── examples/
    ├── nexus-ping-demo/    # fair compaction A/B (use this for sales)
    ├── repovow-compact-demo/  # legacy demo
    └── github-repovow-check.yml
```

---

## 4. Rebuild the Rust CLI

### Step 1 — Clone and build

```bash
git clone https://github.com/bunyinu/repovow.git
cd repovow
cargo build --release
```

Produces:

- `target/release/repovow` — CLI + hooks
- `target/release/repovow-server` — cloud server

### Step 2 — Run tests

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

Or use the helper:

```bash
./scripts/release.sh          # test + stage npm
./scripts/release.sh --skip-tests --install-global
```

### Step 3 — Use in a project

```bash
cd /path/to/your-app
/path/to/repovow/target/release/repovow init
repovow onboard "My task" --accept "tests pass" --constraint "no new deps"
repovow config set --acceptance "npm test"
```

### What `repovow init` writes

| Path | Action |
|------|--------|
| `.repovow/config.json` | Defaults (loop breaker, snapshot limits) |
| `.repovow/state.json` | Empty goal until `repovow goal set` |
| `.repovow/snapshot.md` | Generated from state |
| `.claude/settings.json` | **Merges** RepoVow hooks (does not delete yours) |
| `.codex/hooks.json` | Merges RepoVow hooks |
| `.cursor/hooks.json` | Merges RepoVow hooks |
| `.agents/skills/repovow/SKILL.md` | Managed, progressively disclosed agent workflow |
| `CLAUDE.md` / `AGENTS.md` | **Appends** `## RepoVow` snippet if missing |

### Default `.repovow/config.json` (after init)

```json
{
  "loop_breaker": { "max_same_failure": 2, "window_minutes": 60 },
  "acceptance_gate": { "enabled": false, "command": "" },
  "policy": { "mode": "off" },
  "snapshot_max_lines": 120,
  "snapshot_max_decisions": 8,
  "snapshot_max_failures": 6,
  "context": {
    "max_tokens": 500,
    "prompt_reminder": false
  }
}
```

---

## 5. Rebuild the npm distribution

### How it works

`repovow` is **not** the Rust binary. It is a **Node shim** (`npm/repovow-cli/bin/repovow.js`) that:

1. Resolves the matching `repovow-<platform>` optional dependency, OR
2. Falls back to `npm/repovow-cli/vendor/repovow` (local dev), OR
3. Falls back to `target/release/repovow` (dev), OR
4. Uses `REPOVOW_BIN` env override

**Critical:** `bin/repovow.js` must stay a **JavaScript shim**. Never commit a compiled ELF as `repovow.js` (v0.4.0 bug).

### Stage locally

```bash
./scripts/stage-npm.sh
# copies target/release/repovow → npm/platforms/<host>/bin/repovow
# copies → npm/repovow-cli/vendor/repovow
# syncs version from Cargo.toml

node npm/repovow-cli/scripts/verify-shim.js
```

### Install globally from local tree

```bash
npm install -g ./npm/repovow-cli
repovow --version   # must match Cargo.toml
repovow policy --help
```

### Platform package layout

Each `npm/platforms/linux-x64-gnu/package.json`:

```json
{
  "name": "repovow-linux-x64-gnu",
  "version": "0.5.3",
  "os": ["linux"],
  "cpu": ["x64"],
  "files": ["bin/repovow"]
}
```

Only `bin/repovow` (native executable) is published in platform packages.

---

## 6. Rebuild RepoVow Cloud (server)

### Run locally

```bash
export PORT=8080
export REPOVOW_DB_PATH=/tmp/repovow-local.db
# optional:
export REPOVOW_STRIPE_PAYMENT_LINK=https://buy.stripe.com/...
export REPOVOW_CREATE_SECRET=my-secret-for-create
export REPOVOW_UPGRADE_CODES=promo1,promo2
export REPOVOW_COOKIE_SECURE=false # local HTTP only; defaults to true

cargo run --release --bin repovow-server
```

Open http://localhost:8080

### Docker (same as Render)

```bash
docker build -t repovow-server .
docker run -p 8080:8080 \
  -e REPOVOW_DB_PATH=/data/repovow.db \
  -v repovow-data:/data \
  repovow-server
```

### What the server does

- Serves static pages from `web/` (embedded fallback for `demo.gif` in binary)
- SQLite at `REPOVOW_DB_PATH` (teams + projects + hashed browser sessions)
- REST API for project create, sync, goal edit, team fleet, billing upgrade
- Browser auth uses an expiring `HttpOnly`, `SameSite=Strict` cookie; CLI auth remains bearer-key based
- Same-origin browser access only; no permissive CORS layer
- Health check at `GET /health` → `{"ok":true,"service":"repovow-cloud"}`

### Server entry (`src/bin/repovow_server.rs`)

- Reads `PORT` (Render sets this; default 8080)
- Tries `REPOVOW_DB_PATH`, falls back to `/tmp/repovow.db` if `/data` fails
- Listens `0.0.0.0:PORT`

---

## 7. Deploy RepoVow Cloud to Render

This is the **production deployment path**. CLI/npm do **not** auto-deploy; only the server runs on Render.

### Architecture on Render

```
GitHub push (main) ──► Render Web Service "repovow-cloud"
                         runtime: docker
                         Dockerfile → repovow-server
                         disk: 1GB mounted at /data
                         SQLite: /data/repovow.db
                         health: GET /health
                         URL: https://keel-cloud.onrender.com
```

### Files involved

| File | Role |
|------|------|
| [`render.yaml`](render.yaml) | Blueprint: service name, env vars, disk, health check |
| [`Dockerfile`](Dockerfile) | Multi-stage Rust build → debian-slim + `repovow-server` + `web/` |
| [`scripts/deploy-render.sh`](scripts/deploy-render.sh) | API trigger for redeploy |

### First-time deploy (Blueprint)

1. Push repo to GitHub (`bunyinu/repovow` or your fork).
2. Log in to https://dashboard.render.com
3. **New → Blueprint** → connect GitHub repo
4. Render reads `render.yaml` and creates:
   - Web service `repovow-cloud`
   - Docker build from `Dockerfile`
   - Persistent disk `repovow-data` → `/data`
5. In Render dashboard → **Environment**, set secrets (see [§9](#9-secrets--environment-variables)):
   - `REPOVOW_STRIPE_PAYMENT_LINK`
   - `REPOVOW_UPGRADE_CODES`
   - `REPOVOW_CREATE_SECRET`
6. Wait for deploy. Verify:
   ```bash
   curl https://keel-cloud.onrender.com/health
   ```

### Redeploy after code changes

**Option A — Git auto-deploy (recommended):**
Push to `main` → Render rebuilds Docker image.

**Option B — API script:**

```bash
export RENDER_API=rnd_xxxxxxxxxxxx
# optional: export RENDER_OWNER_ID=tea-xxxxx
# optional: export RENDER_SERVICE_NAME=repovow-cloud
./scripts/deploy-render.sh
```

Script behavior:

- If service exists → `POST /v1/services/{id}/deploys`
- If not → prints Blueprint instructions

### Dockerfile notes (why it looks weird)

- **Runs as root** so Render persistent disk at `/data` is writable
- `mkdir -p /data` in image
- `COPY web` for static assets
- Only builds `--bin repovow-server` (not `repovow` CLI)

### Render free tier caveats

- Cold starts on free plan
- Single instance + SQLite — not HA
- Disk persists across deploys; backup `repovow.db` before risky migrations

### Connect a local repo to cloud (after deploy)

```bash
# On website: https://keel-cloud.onrender.com/start → create project → copy id + api_key

repovow cloud link \
  --url https://keel-cloud.onrender.com \
  --project YOUR_PROJECT_ID \
  --key YOUR_API_KEY

repovow cloud push
```

Creates `.repovow/cloud.json` (usually gitignored).

---

## 8. Release pipeline (tag → npm + GitHub)

### Trigger

```bash
# bump version in Cargo.toml first (source of truth)
git commit -am "Release v0.5.3"
git tag v0.5.3
git push origin main
git push origin v0.5.3
```

### GitHub Actions (`.github/workflows/release.yml`)

On tag `v*.*.*`:

1. **Matrix build** (4 targets):
   - `x86_64-unknown-linux-gnu` → `repovow-linux-x64-gnu`
   - `aarch64-unknown-linux-gnu` → `repovow-linux-arm64-gnu`
   - `x86_64-apple-darwin` → `repovow-darwin-x64`
   - `aarch64-apple-darwin` → `repovow-darwin-arm64`
2. `./scripts/stage-npm.sh --target … --npm-pkg …`
3. Upload artifacts
4. **publish job:**
   - Merge platform packages
   - `node npm/repovow-cli/scripts/sync-version.js $VERSION`
   - `node npm/repovow-cli/scripts/prep-publish.js $VERSION`
   - Publish each missing platform package, then `repovow`, with provenance
   - Create the GitHub Release only after npm publishing succeeds

The publish step is resumable: an existing package version is skipped, so a
failed release can be rerun after correcting registry access.

### npm authorization

Each package trusts the GitHub-hosted `.github/workflows/release.yml` workflow in
`bunyinu/repovow` for `npm publish`. The workflow has `id-token: write` permission,
uses an OIDC-capable npm CLI, and fails if trusted publishing is unavailable.

### Post-release verification (mandatory)

```bash
npm install -g repovow@0.5.3
which repovow
repovow --version          # must show 0.5.3
file $(which repovow)      # must be node script or symlink to it — NOT ELF
repovow policy --help      # must exist on 0.4+
```

### CI on every PR (`.github/workflows/ci.yml`)

- `cargo fmt`, `clippy`, `test`, `release build`
- `./scripts/stage-npm.sh` + `verify-shim.js`

**Cloud is NOT deployed by CI.** Deploy server separately via Render.

---

## 9. Secrets & environment variables

### RepoVow Cloud (Render dashboard)

| Variable | Required | Example | Purpose |
|----------|----------|---------|---------|
| `PORT` | Auto | `10000` | Set by Render |
| `REPOVOW_DB_PATH` | Yes | `/data/repovow.db` | SQLite path on persistent disk |
| `RUST_LOG` | No | `info` | Logging |
| `REPOVOW_FREE_PROJECT_LIMIT` | No | `1` | Free tier project cap |
| `REPOVOW_PRO_PROJECT_LIMIT` | No | `50` | Team tier project cap |
| `REPOVOW_STRIPE_PAYMENT_LINK` | For billing | `https://buy.stripe.com/...` | Pricing page CTA |
| `REPOVOW_UPGRADE_CODES` | For billing | `code1,code2` | Redeem after Stripe payment |
| `REPOVOW_CREATE_SECRET` | Optional | random string | Require a user-supplied signup code for `POST /api/teams`; never rendered into HTML |
| `REPOVOW_COOKIE_SECURE` | Production | `true` | Mark browser session cookies `Secure`; set `false` only for local HTTP |

In `render.yaml`, billing/create secrets use `sync: false` — you set them manually in Render UI.

### Local CLI

| Variable | Purpose |
|----------|---------|
| `REPOVOW_BIN` | Override binary path in installed hooks |

### Local server dev

Same as Render vars; use `/tmp/repovow.db` if no disk.

---

## 10. End-to-end verification checklist

Run this after any rebuild or deploy. Every step should pass.

### CLI

```bash
repovow --version
repovow doctor
mkdir /tmp/repovow-smoke && cd /tmp/repovow-smoke
git init && repovow init
repovow goal set "smoke test" --accept "ok"
test -f .repovow/snapshot.md
test -f .claude/settings.json
rg "repovow hook" .claude/settings.json
repovow check
```

### Hooks (manual)

```bash
repovow hook session-start --agent claude < /dev/null
# should print snapshot text
```

### Server local

```bash
curl -s localhost:8080/health | jq .
curl -s -o /dev/null -w "%{http_code}" localhost:8080/pricing   # 200
```

### Server production

```bash
curl -s https://keel-cloud.onrender.com/health
curl -s -o /dev/null -w "%{http_code}" https://keel-cloud.onrender.com/demo.gif
```

### npm shim (after publish)

```bash
npm install -g repovow@latest
repovow --version
repovow policy verify   # in a repo with policy
```

### Compaction demo (proof)

```bash
bash examples/nexus-ping-demo/demo.sh
# without-repovow: no port 7429
# with-repovow: port 7429 after /compact
```

---

## 11. Cloud HTTP API

Base URL: `https://keel-cloud.onrender.com`

| Method | Path | Auth | Purpose |
|--------|------|------|---------|
| GET | `/health` | none | Health check |
| GET | `/`, `/pricing`, `/trust`, `/start`, … | none | Static HTML |
| GET | `/demo.gif` | none | Demo asset |
| POST | `/api/teams` | none | Create team |
| POST | `/api/projects` | `X-RepoVow-Create-Secret` if configured | Create project → returns `id`, `api_key` |
| GET | `/api/projects/{id}` | `Bearer {api_key}` | Get project state |
| POST | `/api/projects/{id}/sync` | Bearer | Push `state` + `snapshot` |
| PUT | `/api/projects/{id}/goal` | Bearer | Web goal editor |
| POST | `/api/teams/projects/link` | team license | Link project to team |
| GET | `/api/teams/projects` | `?license=` | Fleet list |
| POST | `/api/billing/upgrade` | body: `team_license`, `code` | Free → Pro |

CLI sync implementation: `src/cloud.rs` (`push_state`, `pull_state`).

---

## 12. SQLite schema

Created in `src/server/db.rs` on first boot:

```sql
CREATE TABLE teams (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    plan TEXT NOT NULL DEFAULT 'free',      -- 'free' | 'pro'
    license_key TEXT NOT NULL UNIQUE,
    max_projects INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL
);

CREATE TABLE projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    api_key TEXT NOT NULL UNIQUE,           -- repovow_{uuid}
    team_id TEXT,
    state_json TEXT NOT NULL DEFAULT '{}',
    snapshot_md TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL
);
```

Limits: `REPOVOW_FREE_PROJECT_LIMIT` (default 1), `REPOVOW_PRO_PROJECT_LIMIT` (default 50).

---

## 13. Hook wiring (agent integration)

Installed into both `.claude/settings.json` and the persistent user-level router at `~/.claude/settings.json` by `repovow init`:

| Event | Matcher | Command |
|-------|---------|---------|
| PreCompact | all | `repovow hook pre-compact --agent claude` |
| SessionStart | `startup\|resume\|clear\|compact` | `repovow hook session-start --agent claude` |
| PreToolUse | Bash, Edit, Write, ApplyPatch | `repovow hook pre-tool-use --agent claude` |
| PostToolUse | same | `repovow hook post-tool-use --agent claude` |
| UserPromptSubmit | all | `repovow hook user-prompt-submit --agent claude` |
| Stop | all | `repovow hook stop --agent claude` |

**PreCompact** prints a prioritized, token-budgeted context packet. The latest completed item is kept in the header, critical sections receive reserved space before extra items, and agents fetch only genuinely omitted detail with `repovow context --section NAME`. The complete `snapshot.md` stays on disk as a human-readable fallback but is not reread after packet delivery. A marker prevents the following compact `SessionStart` hook from injecting the same packet twice.

`repovow init` also installs `.agents/skills/repovow/SKILL.md`. This embeds the compact-first workflow: do not reread packet sections or the full snapshot, batch independent targeted discovery, and use at most one checkpoint near a meaningful transition.

The persistent Claude/Codex routers resolve the working repository on every event. When the agent opens a Git repository without `.repovow/config.json`, the router creates minimal repo-local state without modifying `CLAUDE.md`, `AGENTS.md`, or project hook files. The first submitted prompt becomes the active goal and receives one compact packet. Non-Git directories, `.repovow-disabled` repositories, and `REPOVOW_AUTO_INIT=0` remain untouched. Claude Code watches settings files and can apply a newly installed router live. RepoVow registers normalized Codex trust fingerprints for only its own handlers while preserving unrelated hook/config state; a Codex process that predates first installation may still need one restart. New repositories require no RepoVow command, restart, or `/hooks` review. Project-local and user-level duplicate delivery is claimed once through `.repovow/hook-dedup/`. Use `repovow agents install` to repair the routers and `repovow agents status` to verify both installation and Codex trust.

`UserPromptSubmit` normally logs the prompt without adding context. If RepoVow was initialized after the current session started, it injects one compact packet and records delivery in `.repovow/context-sessions/`; later prompts are silent. Enable the legacy per-prompt reminder with `repovow config set --prompt-reminder on`.

**PreToolUse** can return `decision: block` (loop breaker, constraints, signed policy).

**Stop** runs acceptance gate shell command; exit 2 blocks session end (Claude).

Codex: same events in `.codex/hooks.json`; installation registers trust for RepoVow-owned handlers only.
Cursor: `.cursor/hooks.json` — less battle-tested.

---

## 14. Product summary

### What RepoVow is

Repo-local **task ticket** (goal, acceptance, constraints, progress, failures) + **hook-layer enforcement** across Claude Code, Codex, Cursor.

### What RepoVow is not

- Replacement for `CLAUDE.md` (house rules stay in your md; `repovow init` appends a small RepoVow section)
- Replacement for Claude Tasks API or Agentpack
- Bulletproof vs prompt injection

### vs “good CLAUDE.md + skills + loop”

| | md + skills | RepoVow |
|--|-------------|------|
| Survives `/compact` | Only if your loop re-reads files | PreCompact injects snapshot automatically |
| Block bad commands | Advisory | PreToolUse deny |
| Block “done” with failing tests | Advisory | Stop hook |
| CI signed goal | DIY | `repovow policy` + `repovow check` |

### Pricing

| Tier | Price |
|------|-------|
| Free CLI + 1 cloud project | $0 |
| Team (fleet, 50 repos) | $15/mo |

---

## 15. Demos & proof assets

### Primary — `examples/nexus-ping-demo/` (fair baseline)

Both arms have `CLAUDE.md` + `.claude/`. Only difference: `repovow init` or not.

| Arm | After Claude `/compact` |
|-----|-------------------------|
| without-repovow | `process.env.PORT` — cannot ship secret **7429** |
| with-repovow | **`PORT = 7429`**, correct JSON |

```bash
bash examples/nexus-ping-demo/demo.sh
bash examples/nexus-ping-demo/record.sh   # asciinema + GIF
```

Artifacts: `demo.gif`, `demo.cast`, `artifacts/results/`, `RESULTS.md`
Homepage: `web/demo.gif`

### Legacy — `examples/repovow-compact-demo/`

Port 8842 vs 3000; without-repovow had no `.claude` (less fair).

### CI example

`examples/github-repovow-check.yml` — run `repovow check` on PR.

---

## 16. Sales narrative

**Wedge:** *Goal survives Claude `/compact`; CI enforces it.*

**One sentence:** RepoVow Team is the control plane for AI agents in your repos — see every goal, gate merges with `repovow check`, same guardrails in Claude, Codex, and Cursor.

**Buyer:** Eng lead, 3–15 devs, multiple repos, Claude Code or Codex.

**Proof:** Run nexus-ping demo or show `demo.gif`.

**Show HN draft:** `docs/SHOW_HN.md`

---

## 17. Failed approaches (do not retry)

| Approach | Why |
|----------|-----|
| Commit ELF as `npm/repovow-cli/bin/repovow.js` | v0.4.0 shipped wrong version, no `policy` cmd |
| `claude --bare` in demos | Breaks Claude auth |
| without-repovow with no `.claude` | Unrealistic baseline |
| Injecting global-hook context before the first prompt | Empty snapshots; automatic bootstrap must wait for the prompt-derived goal |
| `npm test` as acceptance gate before tests pass | Infinite fail |
| asciinema without `--overwrite` | Re-record aborts |
| Expect GHA to deploy cloud | Only Render deploys server |

---

## 18. Open decisions

1. Cursor: first-class vs documented manual hooks?
2. Product telemetry vs privacy-blind?
3. Windows npm platform package?
4. Retire greet-api demo in favor of nexus-ping only?
5. Add “power user md + loop” third demo arm?

---

## 19. Key links

| Resource | URL / path |
|----------|------------|
| GitHub | https://github.com/bunyinu/repovow |
| Cloud | https://keel-cloud.onrender.com |
| npm | `repovow` |
| Handoff (this file) | `docs/HANDOFF.md` |
| Deploy blueprint | `render.yaml` |
| Deploy script | `scripts/deploy-render.sh` |
| Docker | `Dockerfile` |
| Release workflow | `.github/workflows/release.yml` |
| Fair demo | `examples/nexus-ping-demo/` |
| Show HN | `docs/SHOW_HN.md` |

---

## Quick rebuild order (TL;DR for a new engineer)

1. `cargo test && cargo build --release` — CLI works
2. `./scripts/stage-npm.sh && npm install -g ./npm/repovow-cli` — npm works
3. `cargo run --release --bin repovow-server` — cloud works locally
4. Push to GitHub → Render Blueprint from `render.yaml` — cloud live
5. Set Render secrets (Stripe, upgrade codes, create secret)
6. `git tag vX.Y.Z && git push origin vX.Y.Z` — npm published
7. `npm install -g repovow@X.Y.Z && repovow --version` — verify shim
8. `bash examples/nexus-ping-demo/demo.sh` — verify product proof

---

*End of handoff. If something fails, start at §10 verification checklist and trace which artifact (CLI / npm / server) broke.*
