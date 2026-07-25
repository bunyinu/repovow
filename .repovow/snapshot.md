# RepoVow state snapshot

_Agent-written progress and failures below are **not** cryptographically signed._
_Trusted goal policy (title, acceptance, constraints): read `.repovow/policy.md` and verify with `repovow policy verify`._

_Compactions: 0 · Sessions: 1 · Last agent: claude_

Hooks inject a compact context packet automatically. Read this full snapshot only when the packet omitted needed detail or hooks were unavailable.

## Goal
**Ship RepoVow distribution**

### Acceptance
- CI passes
- npm shim works
- hooks use PATH repovow

## Progress

**Current step:** Repeat benchmark across diverse repositories before broad product claims

### Done
- implemented budgeted context packets, compaction deduplication, and working-set tracking
- embedded compact-first skill and quality-reserved section retrieval
- Ran reproducible 2x2 large-repository benchmark: Codex RepoVow treatment reduced gross input 20.5% at equal quality; Claude treatment increased gross input 8.6%, exposing 34 duplicate policy warnings and one full snapshot reread
- V2 2x2 benchmark verified token efficiency at equal hidden quality: Codex gross input -23.0%, fresh -15.8%, time -11.0%; Claude gross input -27.0%, fresh -33.3%, time -26.7%, cost -28.4%; zero Claude snapshot reads or policy warnings
- Installed persistent Claude/Codex hook routers with hot project activation, one-time mid-session context delivery, safe no-op outside .repovow, and duplicate-event suppression
- Added zero-command agent bootstrap: npm installs global routers, Git repos initialize on agent events, and the first prompt creates goal plus acceptance context automatically
- Validated zero-touch Claude and Codex activation on fresh repositories; fixed Codex hook trust registration; 41 unit and 17 integration tests pass; report written to /home/lus/repovow-real-proof/zero-touch-new-project-test/REPORT.md

## Do NOT retry (already failed)
_These approaches failed. Try a different strategy._

- **Bash:** `bash:npm test`
  - tests failed
