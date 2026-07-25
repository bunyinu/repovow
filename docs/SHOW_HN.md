# Show HN — draft (you post)

**Title:** Show HN: RepoVow – repo-local agent goals that survive Claude Code compaction

**URL:** https://keel-cloud.onrender.com

---

## Comment (post as first reply)

Hi HN — I built RepoVow because my agents kept forgetting the task after `/compact`.

**The problem:** Compaction summarizes chat history. Architectural decisions, ports, constraints, and “do not retry” lists fall out unless something durable puts them back.

**What RepoVow does:**

1. You set a goal in the repo: `repovow onboard "Ship auth" --accept "tests pass"`
2. State lives in `.repovow/snapshot.md` (git-committable)
3. Hooks for Claude Code, Codex, and Cursor re-inject that snapshot after compact
4. Optional guardrails: block `npm install`, block stop until tests pass, loop breaker

**Demo (same machine, forced `/compact`):**

- **With RepoVow:** agent keeps secret port 8842 from the repo goal
- **Without RepoVow:** agent defaults to port 3000 and admits it guessed

GIF on the homepage: https://keel-cloud.onrender.com/demo.gif

**Install:**

```bash
npm install -g repovow
cd your-repo
repovow onboard "your task" --accept "tests pass"
```

**Not trying to replace** Claude’s Tasks API or Agentpack — RepoVow is for teams that want a **repo-owned, enforceable goal file** shared across Claude, Codex, and Cursor, with optional CI gate (`repovow check`).

**Team plan ($15/mo):** fleet dashboard + 50 repos — https://keel-cloud.onrender.com/pricing

I'd love feedback on: (1) whether you'd commit `.repovow/` to git, (2) Cursor hook UX, (3) what acceptance gate command you'd use in CI.

Apache-2.0 CLI: https://github.com/bunyinu/repovow

---

## Checklist before posting

- [ ] Deploy latest server (demo.gif + trust page live)
- [ ] Tag npm release if onboard/Cursor not published yet
- [ ] Post Tuesday–Thursday morning US time
- [ ] Reply to every comment for 4 hours
- [ ] Link demo GIF in comment, not only homepage

## What I cannot post for you

- The Show HN submission itself (your account)
- Pilot team quotes
- Live Stripe keys (set `REPOVOW_STRIPE_PAYMENT_LINK` on Render)
