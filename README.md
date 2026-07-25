# RepoVow

**Repo-local agent state for Claude Code and Codex — written in Rust.**

RepoVow keeps task context in your repository — not in a chat transcript — so agents survive compaction, session resets, and switching between tools.

## Install & update (everyone)

**One standard method** — npm, same pattern as Codex CLI. Requires [Node.js 18+](https://nodejs.org/).

```bash
# Install
npm install -g repovow

# Update (after you have 0.2.2+)
repovow update

# Or update without the repovow command
npm install -g repovow@latest
```

The npm installation automatically installs or updates the Claude Code and Codex user-level routers while preserving other hooks.

### Upgrading from Keel

Install `repovow` once. Its postinstall replaces managed Keel hooks, and the first
RepoVow command or agent event renames `.keel/` to `.repovow/` atomically. Managed
agent instructions and skills are rebranded during `repovow init`; existing cloud
environment variables, browser sessions, and `/data/keel.db` remain compatible
during the transition.

Verify:

```bash
repovow --version
```

Then use your agent normally:

```bash
cd your-repo
claude # or: codex
```

RepoVow creates minimal `.repovow/` state automatically and uses the first submitted task as the goal. `repovow init`, `repovow onboard`, and `repovow tui` remain available for explicit acceptance criteria, constraints, or team-shared project hooks.

> **Contributors** building from this repo use `./scripts/release.sh --install-global` — not required for normal users.

## RepoVow Cloud (hosted)

**Live:** https://keel-cloud.onrender.com · **Pricing:** https://keel-cloud.onrender.com/pricing

The Render service is named `repovow-cloud`; the older generated hostname is retained because Render does not rename an existing service URL.

| Plan | Price | What you sell |
|------|-------|----------------|
| **Free** | $0 | 1 repo on cloud + full local CLI |
| **Team** | $15/mo | Fleet dashboard + 50 repos + `repovow check` in CI |

Pro activation: pay via Stripe on `/pricing`, then redeem upgrade code with your team license.

### What you’re selling (one sentence)

> **RepoVow Team is the control plane for AI agents in your repos** — see every goal, gate merges with `repovow check`, same guardrails in Claude and Codex.

**Buyer:** eng lead / small team (3–15 devs) using Claude Code or Codex on multiple repos.  
**Pain:** agents forget goals after compaction, skip tests, install deps, no visibility across repos.  
**Proof:** `repovow check` fails CI until goal + tests pass; fleet dashboard shows which repo is stuck.

Example CI workflow: [`examples/github-repovow-check.yml`](examples/github-repovow-check.yml)

1. Open **https://keel-cloud.onrender.com** → create a project → copy your API key
2. In your repo:

```bash
repovow cloud link --url https://keel-cloud.onrender.com --project YOUR_PROJECT_ID --key YOUR_API_KEY
repovow init
```

3. Use Claude Code or Codex — state syncs automatically.

**Dashboard:** `https://keel-cloud.onrender.com/dashboard/YOUR_PROJECT_ID`  
**Edit goal in browser:** `.../dashboard/YOUR_PROJECT_ID/edit`

## Set your goal (CLI, TUI, or web)

| Method | Command / URL |
|--------|----------------|
| CLI | `repovow goal set "..." --accept "..." --constraint "..." --step "..."` |
| TUI | `repovow tui` |
| Web | `https://keel-cloud.onrender.com/dashboard/YOUR_PROJECT_ID/edit` |

After editing on the web: `repovow cloud pull` in your repo.

## Commands

```bash
repovow init
repovow onboard "My task" --accept "tests pass"   # recommended: init + goal
repovow doctor                       # diagnose setup
repovow check                        # CI: goal + acceptance gate (if enabled)
repovow check --cloud                # also verify cloud link
repovow agents status               # persistent Claude/Codex router status
repovow agents install              # repair or upgrade the routers
repovow update                       # npm users: upgrade to latest
repovow goal set / show
repovow tui
repovow config set --acceptance "npm test"   # gate on agent stop
repovow config set --acceptance off
repovow config set --context-tokens 500      # injected context budget
repovow config set --prompt-reminder off     # default: no per-prompt token tax
repovow config show
repovow progress --step "..." --done "..." --blocker "..."
repovow decide "We chose Postgres"
repovow status
repovow snapshot --print
repovow context --stats               # preview injection + estimated savings
repovow context --section acceptance  # retrieve only one omitted section
repovow cloud link / push / pull
```

## v0.3 guardrails

| Feature | When | What |
|---------|------|------|
| **Constraint guard** | `PreToolUse` | Blocks deps install, file edits (read-only), banned keywords from `--constraint` |
| **Acceptance gate** | `Stop` | Runs your command (e.g. `npm test`) before agent ends session |
| **Loop breaker** | `PreToolUse` | Blocks repeated failed commands (unchanged) |
| **Context compiler** | compaction / session start | Injects prioritized state under an approximate token budget |

Constraints are matched from `repovow goal set --constraint "..."`. Examples:

- `no new deps` → blocks `npm install`, `cargo add`, etc.
- `read-only` → blocks Write/Edit
- `no payment SDK` → blocks stripe/paypal in commands

## What it does

1. **Survives compaction** — hooks restore a compact packet after compact/resume
2. **Cross-tool** — same `.repovow/` for Claude Code and Codex
3. **Loop breaker** — blocks repeated failed Bash/edit attempts
4. **Failure detection** — reads `exit_code`, `stderr`, `tool_result.is_error`

## Token-efficient context

RepoVow keeps the complete human-readable state in `.repovow/snapshot.md`, but does not inject that entire file on every chat:

- Goal, current step, latest completed work, constraints, acceptance, blockers, failures, decisions, and working-set paths are prioritized.
- Every non-empty critical section gets a reserved slot before remaining budget is distributed, so a long constraint list cannot hide acceptance criteria or the working set.
- The default injected budget is approximately 500 tokens (`repovow config set --context-tokens N`).
- Successful edit/write hooks remember recent files, so the next chat starts with a targeted working set.
- Git-tracked modifications are included as paths, not file contents.
- The packet explicitly directs agents to use the working set and targeted search instead of scanning the repository.
- Agents do not reread packet sections or the full snapshot. Only genuinely omitted details are retrieved by section (`repovow context --section acceptance`, `constraints`, `working-set`, `blockers`, `failures`, or `decisions`).
- The embedded workflow batches independent reads and searches into fewer model turns and limits RepoVow checkpointing to meaningful transitions.
- If the complete snapshot is already smaller, RepoVow uses it instead; injection never grows solely because of the compiler.
- Compaction context is injected once; the following compact-session hook uses a short acknowledgement.
- Per-prompt reminders are off by default, avoiding a repeated token charge on every message.

Run `repovow context --stats` to preview exactly what an agent receives and compare it with the full snapshot.

## Embedded agent workflow

`repovow init` installs lifecycle hooks for Claude Code, Codex, and Cursor and a repo-scoped `.agents/skills/repovow/SKILL.md` workflow. The hook injects live state automatically; the skill teaches agents to retrieve only missing sections, batch targeted discovery from the working set, and checkpoint meaningful transitions. Unsigned policy enforcement is off by default; `repovow policy init` creates signing keys and switches enforcement to required. Skill metadata is always small, while the workflow body uses progressive disclosure and loads only when the repository task needs it.

## RepoVow vs Claude Tasks API vs Agentpack

Nobody ships *exactly* RepoVow as a first-party product. These are the closest alternatives today.

| | **RepoVow** | **Claude Code Tasks API** | **[Agentpack](https://github.com/ihorponom/agentpack)** |
|---|----------|---------------------------|----------------------------------------------------------|
| **Who** | Third-party (you) | Anthropic (native) | Third-party OSS |
| **Agents** | Claude Code + Codex + Cursor | Claude Code only | Any MCP client (Claude, Codex, Cursor, …) |
| **Where state lives** | `.repovow/` **in the repo** (git-committable) | `~/.claude/tasks/` (home dir) | `.agentpack/` (local; gitignored by default) |
| **Who writes state** | You (`repovow goal set`, TUI, web) + hooks | Agent (`TaskCreate` / `TaskUpdate`) | Agent via MCP tools + checkpoints |
| **Survives compaction** | ✓ hooks inject a budgeted context packet | ✓ tasks on disk, agent queries `TaskList` | ✓ ledger + `load_context` / export |
| **Multi-session sync** | ✓ git + optional RepoVow Cloud | ✓ `CLAUDE_CODE_TASK_LIST_ID` | ✓ shared ledger / handoff export |
| **Acceptance criteria** | ✓ explicit in goal + optional **Stop gate** | ✗ task status only | ✓ evidence / decisions (no Stop gate) |
| **Enforcement** | ✓ constraint guard, loop breaker, acceptance gate | ✗ reminders + task status (no tool blocks) | ✗ continuity layer (no tool blocks) |
| **Install** | `npm install -g repovow` | Built into Claude Code v2.1.16+ | `pip` / MCP server |
| **Hosted team UI** | ✓ RepoVow Cloud (free / Pro) | ✗ | ✗ |

### When to use which

- **Claude Code Tasks API** — you live in Claude only, want native task lists with dependencies and multi-terminal sync. Best default *inside* Claude Code since v2.1.19.
- **Agentpack** — you want a rich task ledger (decisions, dead ends, evidence, file hashes) and already run MCP across multiple agents.
- **RepoVow** — you want **repo-owned** goal state in git, the **same file** in Claude *and* Codex, **hard guardrails** (block deps, block stop until tests pass), and optional cloud/team dashboard without running an MCP server.

RepoVow complements Tasks API and Agentpack; it does not replace Claude memory, `CLAUDE.md`, or a full ledger. Use Tasks or Agentpack for deep task graphs; use RepoVow when the team needs a shared, enforceable goal file in the repo.

## Building workflow dependency (stickiness)

RepoVow should not trap users — it should **accumulate value** so removing it hurts workflow, not data.

| Layer | What compounds | Switching cost |
|-------|----------------|----------------|
| **Git** | Commit `.repovow/state.json` + `snapshot.md` — goal becomes part of PR review | Team process references RepoVow goal in tickets/PRs |
| **Hooks** | Constraint guard + loop breaker + Stop gate run every session | Agents behave differently without hooks |
| **History** | `decisions`, `attempts.jsonl`, `changelog.jsonl` — “do not retry” list grows | Lose failure memory if you uninstall |
| **Cloud** | Pro team links many repos; web goal editor | Re-link every repo + lose dashboard |
| **CI** | `repovow check` in GitHub Actions / pre-merge | Pipeline fails without RepoVow |

**CI example** (add after `repovow config set --acceptance "npm test"`):

```yaml
- name: RepoVow acceptance
  run: repovow check
```

`repovow check` verifies: RepoVow initialized → active goal → acceptance command passes (same gate as agent Stop). Use `repovow check --cloud` when linked to RepoVow Cloud.

## Layout

```
.repovow/
  state.json        # goal, progress, decisions
  snapshot.md       # complete state; read on demand
  attempts.jsonl    # tool attempts (loop breaker)
  changelog.jsonl   # lifecycle audit log
  config.json       # thresholds
  cloud.json        # optional RepoVow Cloud link
```

## Why Rust

| Python v0.1 | Rust v0.2 |
|-------------|-----------|
| Requires Python runtime | **Single static binary** (~2MB) |
| ~50ms hook cold start | **Sub-ms hook latency** |
| pip install | **npm install -g repovow** |

## Develop (contributors only)

```bash
cargo test
./scripts/stage-npm.sh
./scripts/release.sh --install-global
```

## CI / Release

- **CI** — fmt, clippy, test, npm shim verify
- **Release** — tag `v*.*.*` → GitHub binaries + npm publish

```bash
git tag vX.Y.Z && git push origin vX.Y.Z
```

## Environment

| Variable | Purpose |
|----------|---------|
| `REPOVOW_BIN` | Override repovow binary path in installed hooks |
| `REPOVOW_SKIP_GLOBAL_HOOKS=1` | Skip user-level router installation in isolated automation |
| `REPOVOW_AUTO_INIT=0` | Disable automatic `.repovow/` bootstrap |

## Hooks

RepoVow installs persistent user-level hook routers into:

**Claude Code** — `~/.claude/settings.json`
**Codex** — `${CODEX_HOME:-~/.codex}/hooks.json`

The routers resolve the repository on every lifecycle or tool event. In a Git repository without `.repovow/config.json`, the first event creates minimal `.repovow/` state; the first user prompt becomes the active goal and receives one compact context packet. Later prompts add no context unless reminders are explicitly enabled. Non-Git directories and repositories containing `.repovow-disabled` remain untouched. Existing project-local hook files remain as portable fallbacks; RepoVow deduplicates delivery when both layers fire.

Claude Code watches settings files and applies the router live. During installation, RepoVow registers Codex trust fingerprints for only its own seven handlers and preserves all unrelated hook/config state. A Codex process that was already open before the persistent router was installed for the first time may require one restart; new processes need no `/hooks` review. `repovow agents status` reports a router unhealthy when its installed fingerprint is not trusted.

## Commit `.repovow/`?

Commit `state.json` and `snapshot.md` for team-shared task state. Optional-gitignore `attempts.jsonl` and `changelog.jsonl`.

## License

Apache-2.0
