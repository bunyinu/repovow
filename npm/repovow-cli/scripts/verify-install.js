#!/usr/bin/env node
const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const packageRoot = path.join(__dirname, "..");
const postinstall = path.join(__dirname, "postinstall.js");
const expectedShim = path.join(packageRoot, "bin", "repovow.js");
const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "repovow-install-"));
const home = path.join(tempDir, "home");

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

try {
  fs.mkdirSync(home, { recursive: true });
  fs.writeFileSync(
    path.join(home, "CLAUDE.md"),
    "## Keel (agent state)\n\nRead `.keel/snapshot.md`.\n\n## User rule\n\nKeep Claude rule.\n"
  );
  fs.writeFileSync(
    path.join(home, "AGENTS.md"),
    "## Keel (agent state)\n\nRun `keel progress`.\n\n## Tools\n\nKeep Codex rule.\n"
  );

  const env = {
    ...process.env,
    HOME: home,
    CODEX_HOME: path.join(home, ".codex"),
  };
  delete env.REPOVOW_BIN;
  delete env.REPOVOW_HOOK_BIN;
  delete env.KEEL_BIN;

  for (let attempt = 0; attempt < 2; attempt += 1) {
    const result = spawnSync(process.execPath, [postinstall], {
      encoding: "utf8",
      env,
    });
    assert(
      result.status === 0,
      result.stderr || result.stdout || "postinstall failed"
    );
  }

  const claude = fs.readFileSync(path.join(home, "CLAUDE.md"), "utf8");
  const agents = fs.readFileSync(path.join(home, "AGENTS.md"), "utf8");
  const claudeHooks = fs.readFileSync(
    path.join(home, ".claude", "settings.json"),
    "utf8"
  );
  const codexHooks = fs.readFileSync(
    path.join(home, ".codex", "hooks.json"),
    "utf8"
  );

  assert(!claude.includes("## Keel"), "Claude legacy section was not migrated");
  assert(!agents.includes("## Keel"), "Codex legacy section was not migrated");
  assert(claude.includes("Keep Claude rule."), "Claude user content was removed");
  assert(agents.includes("Keep Codex rule."), "Codex user content was removed");
  assert(
    (claude.match(/## RepoVow/g) || []).length === 1,
    "Claude managed section is not idempotent"
  );
  assert(
    (agents.match(/## RepoVow/g) || []).length === 1,
    "Codex managed section is not idempotent"
  );
  assert(claudeHooks.includes(expectedShim), "Claude hooks do not use installed shim");
  assert(codexHooks.includes(expectedShim), "Codex hooks do not use installed shim");

  console.log("automatic agent install: ok");
} finally {
  fs.rmSync(tempDir, { recursive: true, force: true });
}
