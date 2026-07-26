# RepoVow — Frontend design brief

**For:** Product designer / UI engineer building RepoVow Cloud + marketing site
**Product:** RepoVow — repo-local agent state that survives Claude Code compaction
**Live reference (functional, not pretty):** https://keel-cloud.onrender.com
**Version:** v0.5.1
**Date:** 2026-06-23

---

## 1. One-sentence product

**RepoVow embeds repo-owned task state into coding-agent lifecycles, reinjecting a quality-preserving context index after compaction with optional team dashboard and CI gates.**

Not “another memory file.” The wedge is: **goal survives compaction; hooks enforce it.**

---

## 2. What you are designing

RepoVow has **two surfaces**. The CLI/hooks are Rust (not your job). You are designing **RepoVow Cloud** — the web product.

| Surface | User | Your scope |
|---------|------|------------|
| **Marketing site** | Visitors, evaluators | Landing, pricing, trust, demo |
| **App (logged-in)** | Eng leads, PMs, devs | Account, fleet, project dashboard, goal editor |

**Out of scope for v1 web:** In-browser terminal, agent chat, hook configuration UI (that stays in CLI / `repovow init`), full policy key management (CLI-first).

---

## 3. Users & jobs-to-be-done

### Persona A — Solo developer
- **Job:** “My agent forgets the task after `/compact`.”
- **Flow:** Install CLI → maybe one cloud project → edit goal in browser occasionally.
- **Success:** Sees goal in dashboard match what agent recalls after compact.

### Persona B — Eng lead (primary paid buyer)
- **Job:** “I run 5–50 repos with agents; I need to see who’s stuck and gate merges.”
- **Flow:** Team account → fleet table → drill into project → edit acceptance criteria → devs run `repovow check` in CI.
- **Success:** Fleet view shows goal title + current step + last sync per repo.

### Persona C — PM / tech lead (non-terminal)
- **Job:** “Set acceptance criteria without opening the IDE.”
- **Flow:** Web goal editor → save → engineer runs `repovow cloud pull`.
- **Success:** Clear form: goal, step, acceptance lines, constraint lines.

---

## 4. Brand & visual direction

### Tone
- **Trustworthy infrastructure**, not playful chatbot.
- **Precise, engineer-native** — like Linear, Vercel, or Render — not consumer social.
- **Honest** — we block tools and compaction amnesia is real; no “AI magic” fluff.

### Current palette (reference only — improve it)

| Token | Hex | Use |
|-------|-----|-----|
| Background | `#0b0f14` | Page |
| Surface / card | `#141c28` | Cards, inputs |
| Border | `#2a3548` | Dividers |
| Text primary | `#e8eef7` | Headings |
| Text muted | `#8fa3bf` | Meta, labels |
| Accent | `#3d7eff` | Primary CTA |
| Success | `#7dcea0` | Saved, valid |
| Error | `#ff8a8a` | Failures |
| Warning | `#f0b429` | Keys, one-time secrets |

### Typography
- Today: system-ui stack. Designer may propose one display + one mono (for API keys, CLI commands, snapshot preview).

### Motion
- Subtle. Terminal demo GIF is the hero motion asset — don’t compete with it.

### Illustration / imagery
- **Hero:** terminal screen recording (`demo.gif`) — same task, forced `/compact`, with RepoVow vs without.
- Avoid generic AI brain / robot stock art.
- Optional: simple diagram — chat context wiped → budgeted RepoVow context packet reinjected via hook.

---

## 5. Information architecture

```
Marketing
├── Home (/)
├── Pricing (/pricing)
├── Trust & security (/trust)
└── Get started (/start) → Account

App (opaque HttpOnly browser session)
├── Account hub (/account)
│   ├── Create project
│   ├── Link existing project
│   └── Fleet table (all projects)
├── Project dashboard (/dashboard/:id)
│   ├── Connect CLI (copy commands)
│   ├── Snapshot preview (read-only markdown)
│   └── Link to edit goal
└── Goal editor (/dashboard/:id/edit)
    ├── Goal title
    ├── Current step
    ├── Acceptance criteria (multiline)
    └── Constraints (multiline)
```

**Auth model (important):** Email + account key login creates an opaque, expiring server-side session in an `HttpOnly`, `SameSite=Strict` cookie. Account and project keys are never stored in browser storage. Project API keys (`repovow_…`) are shown once at creation for CLI setup.

---

## 6. Screen-by-screen requirements

### 6.1 Home / Landing

**Purpose:** Convert visitors in 10 seconds.

**Must include:**
- Headline: agent state survives compaction
- Subhead: goal lives in `.repovow/` in the repo; hooks restore after Claude/Codex/Cursor compact
- Primary CTA: Get started
- Secondary: Pricing, Trust
- **Demo GIF** (full width, captioned): with RepoVow port survives / without RepoVow agent guesses
- How it works (5 steps): account → npm install → `repovow onboard` → optional cloud link → use agent
- Social proof placeholder (logos/quotes — empty for now)

**Do not:** Feature matrix overload on hero.

---

### 6.2 Pricing

**Purpose:** Free vs Team ($15/mo) conversion.

| Plan | Price | Bullets |
|------|-------|---------|
| **Free** | $0 | 1 cloud project, full CLI, web goal editor |
| **Team** | $15/mo | Fleet dashboard, 50 repos, CI / `repovow check` story |

**Must include:**
- Stripe subscribe CTA (external link)
- **Activate Pro** form: team license + upgrade code (post-payment)
- Link back to account if already on free

---

### 6.3 Trust & security

**Purpose:** SOC-2-minded eng lead sign-off.

**Sections (content exists in `web/trust.html`):**
- What we store (local vs cloud)
- What we don’t do (no training on goals)
- Hooks can block tools — intentional
- Signed policy (P-256) — what’s signed vs agent-written
- Cloud access control (API key required, not public by URL alone)
- Data deletion

**Tone:** Plain language, no legal wall of text. Scannable headings.

---

### 6.4 Start / Create account (`/start`)

**Purpose:** Onboard new team (free tier starts here).

**Fields:**
- Create account: name / team name → returns **account key** (show once banner)
- Returning: paste account key → open account

**States:** loading, error, success redirect to `/account`

---

### 6.5 Account hub (`/account`) — **core app screen**

**Purpose:** Fleet home for eng lead.

**Header:**
- Team name
- Plan badge: `free` | `pro`
- Account key hint (last 6 chars)
- Sign out

**Actions:**
- **New project** — name field + create (respects plan limit; 402 → upsell pricing)
- **Link existing project** — project ID + API key

**Fleet table (Team plan showcase):**

| Column | Source |
|--------|--------|
| Project name | link to dashboard |
| Goal title | from synced state (or “no goal”) |
| Current step | optional, pro fleet |
| Compactions | count from state |
| Last updated | ISO timestamp |

Today only shows name, goal, updated — **design should add step + compaction + health indicator** (e.g. stale sync > 24h).

**Empty state:** “No projects — create one or link CLI”

**One-time key banners:** Account key + project API key — high-visibility, dismissible, copy button.

---

### 6.6 Project dashboard (`/dashboard/:id`)

**Purpose:** Single-repo control panel.

**If there is no valid account session:** redirect to account sign-in with `?link={id}`.

**Content:**
- Project name (editable in v2?)
- Project ID (mono, copy)
- Last updated
- **Connect your repo** — code block:
  ```
  npm install -g repovow
  repovow cloud link --url … --project … --key …
  repovow onboard "…" --accept "…"
  ```
- **Snapshot preview** — rendered markdown or styled pre (goal, acceptance, constraints, progress, do-not-retry)
- CTA: Edit goal

**Future widgets (v2):**
- Policy status badge: valid / unsigned / tampered
- Acceptance gate status
- Recent changelog events (compact, tool blocks)

---

### 6.7 Goal editor (`/dashboard/:id/edit`)

**Purpose:** Non-developer edits mission.

**Form fields:**

| Field | Type | Maps to |
|-------|------|---------|
| Goal | single line | `state.goal.title` |
| Current step | single line | `state.progress.current_step` |
| Acceptance criteria | textarea, one per line | `state.goal.acceptance[]` |
| Constraints | textarea, one per line | `state.goal.constraints[]` |

**Actions:** Save (primary), Back to dashboard
**Success message:** “Saved. Run `repovow cloud pull` in your repo.”
**Errors:** inline, red

**Design note:** Frame acceptance/constraints as **intentional requirements** (trusted), not chat injection — subtle trust chrome for v0.4 policy story.

---

## 7. Key user flows (wire these)

### Flow 1 — First-time visitor → using RepoVow
```
Home → Get started → Create account → (save account key) → Account
  → Create project → (save project API key) → Dashboard
  → Copy CLI commands → terminal (out of band)
```

### Flow 2 — Eng lead checks fleet
```
Account → fleet table → click project → dashboard → snapshot
```

### Flow 3 — PM updates acceptance criteria
```
Dashboard → Edit goal → change acceptance → Save
  → (engineer) repovow cloud pull → agent sees new snapshot on next compact
```

### Flow 4 — Upgrade to Team
```
Pricing → Stripe → email with upgrade code → Pricing activate form
  → Account shows pro badge → create up to 50 projects
```

### Flow 5 — Returning user new browser
```
Start → paste account key → server creates HttpOnly session → Account
```

---

## 8. Data the UI displays (API contract)

Base URL: `https://keel-cloud.onrender.com`

| Endpoint | Auth | Returns |
|----------|------|---------|
| `GET /api/projects/{id}` | Browser session or bearer project API key | `name`, `snapshot`, `state`, `updated_at` |
| `PUT /api/projects/{id}/goal` | Browser session or bearer | updates goal fields |
| `GET /api/teams/projects` | Browser session or bearer account key | redacted `team` + `projects[]`; never access keys |
| `POST /api/projects` | Browser session | new `id`, one-time `api_key`, `dashboard_url` |
| `POST /api/billing/upgrade` | Browser session + configured code | activates pro |

**state.goal shape:**
```json
{
  "title": "Build API on port 7429",
  "acceptance": ["tests pass", "GET / returns {...}"],
  "constraints": ["no new deps", "never port 3000"]
}
```

---

## 9. Components to design (component library)

| Component | Notes |
|-----------|-------|
| Site header | Logo, nav, active state |
| Primary / secondary button | |
| Card | Default + pro highlight (pricing) |
| Form field + label + help text | |
| Code block + copy button | CLI connect snippet |
| Secret key banner | One-time display, warning styling |
| Fleet table | Sortable in v2; responsive → cards on mobile |
| Snapshot viewer | Markdown-rendered or structured sections |
| Plan badge | free / pro |
| Status pill | ok / err / warn / muted |
| Empty state | Illustration optional |
| Toast / inline status | Save success, API errors |
| Demo figure | GIF + caption |

---

## 10. Responsive & accessibility

- **Mobile:** Account + fleet must work on phone (eng lead checks status on the go).
- **Keyboard:** All forms, focus rings visible on dark bg.
- **Contrast:** WCAG AA on text and CTAs.
- **Screen readers:** Key banners are alerts; table headers proper.

---

## 11. Copy cheatsheet (use on site)

| Instead of | Say |
|------------|-----|
| Memory platform | Goal survives compaction |
| AI assistant | Claude Code, Codex, Cursor |
| Prompt engineering | Hooks reinject `.repovow/snapshot.md` |
| Guaranteed | Reduces amnesia / enforces (honest) |

**Tagline options:**
- “Agent state that survives compaction”
- “The task ticket your repo keeps”
- “Goal survives `/compact`”

---

## 12. MVP vs phase 2

### MVP (ship first)
- Unified design system across all pages (today pricing/trust are inconsistent)
- Landing + pricing + trust + account + dashboard + goal editor
- Key custody banners
- Fleet table with goal + updated
- Mobile-responsive fleet + editor

### Phase 2
- Policy status UI (`valid` / `unsigned` / `tampered`)
- Changelog timeline (compactions, tool blocks)
- Project health (stale sync, no goal set)
- Onboarding checklist on dashboard (init, link, first push)
- Email-based auth (replace raw keys) — **product decision pending**
- Dark/light toggle (dark default)

### Not web (stay CLI)
- `repovow init`, hook install, `repovow policy sign`, `repovow check` config

---

## 13. Competitive context (for hero messaging)

RepoVow is **not** Claude Tasks API or a chat UI. Comparison for footer or `/pricing`:

| | RepoVow | Claude Tasks | Spreadsheet + CLAUDE.md |
|--|------|--------------|-------------------------|
| Lives in repo | ✓ | ✗ | partial |
| Survives compact | hooks | tasks on disk | manual |
| Blocks bad tools | ✓ | ✗ | ✗ |
| Team fleet | ✓ | ✗ | ✗ |

---

## 14. Assets provided

| Asset | Path |
|-------|------|
| Demo GIF | `web/demo.gif` |
| Fair demo write-up | `examples/nexus-ping-demo/RESULTS.md` |
| Existing HTML wireframes | `web/*.html` |
| CSS tokens (starter) | `web/site.css` |

---

## 15. Deliverables expected from designer

1. **Figma (or similar):** all screens desktop + mobile key breakpoints
2. **Design system:** color, type, spacing, components
3. **Prototype:** Home → account → dashboard → edit goal
4. **Handoff:** specs for engineer implementing in `web/` or a future React/Vue app

**Engineering note:** Backend is REST + static HTML today. Designer is free to propose SPA stack; API contract in §8 should remain stable.

---

## 16. Questions for product (designer can flag)

1. Email auth vs key-based auth for v2?
2. Show snapshot as raw markdown or structured cards?
3. Fleet: table only or kanban by “current step”?
4. In-app Stripe checkout vs external link?
5. Light mode?

---

*Brief aligns with RepoVow v0.5.1 codebase and `web/` as deployed on Render.*
