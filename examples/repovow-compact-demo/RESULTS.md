# RepoVow vs No RepoVow — Compaction Demo (clean run)

**Recorded:** 2026-06-22  
**Global `~/.claude/settings.json` hooks:** removed  
**RepoVow:** installed only in `with-repovow/` via `repovow init`

## Watch the recording

- **GIF:** [demo.gif](./demo.gif) (~1 MB, terminal screen recording)
- **Asciinema cast:** [demo.cast](./demo.cast) — replay with `asciinema play demo.cast`

## What the demo does

1. Creates two identical `greet-api` repos
2. **without-repovow:** plain git repo (no `.repovow`, no `.claude` hooks)
3. **with-repovow:** `repovow init` + goal (secret port **8842**, constraints, acceptance)
4. For each: Claude Code implements `server.js` → **`/compact`** → recall test without re-reading files

## Results

| | Without RepoVow | With RepoVow |
|--|--------------|-----------|
| `.repovow/` exists | **No** | Yes |
| Port after compact | **3000** (default/guess) | **8842** |
| Recalls constraints | No — "no requirement source exists" | Yes — avoids 3000/8080 |
| `server.js` | `PORT = 3000` | `PORT = 8842` |

### Without RepoVow (phase 3)
> 3000 is the default I chose, not a recovered requirement. The "correct port from project requirements" remains unknown, since no requirement source exists in this directory.

### With RepoVow (phase 3)
> `server.js` already uses `PORT = 8842` … matches the acceptance criteria and avoids the forbidden ports.

## Restore global hooks (if needed)

Backup saved at `~/.claude/settings.json.bak.before-repovow-removal-*`

## Re-run

```bash
bash examples/repovow-compact-demo/demo.sh
```
