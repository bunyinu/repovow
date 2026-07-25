# RepoVow policy (signed goal)

_Cryptographic policy for title, acceptance, and constraints only. Agent-written progress and failures live in `snapshot.md` and are **not** signed._

Algorithm: **ecdsa-p256** (default for new installs)

Policy signing: **off**

## Goal
**Ship RepoVow distribution**

### Acceptance
- CI passes
- npm shim works
- hooks use PATH repovow

_Verify: `repovow policy verify` · Re-sign: `repovow policy sign` · Mode: off_
