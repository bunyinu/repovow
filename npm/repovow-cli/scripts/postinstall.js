#!/usr/bin/env node
const { spawnSync } = require("node:child_process");
const path = require("node:path");

if (
  process.env.REPOVOW_SKIP_GLOBAL_HOOKS === "1" ||
  process.env.KEEL_SKIP_GLOBAL_HOOKS === "1"
) {
  process.exit(0);
}

const shim = path.join(__dirname, "..", "bin", "repovow.js");
const result = spawnSync(process.execPath, [shim, "agents", "install"], {
  stdio: "inherit",
  env: process.env,
});

if (result.error) {
  console.error(`repovow: automatic agent integration failed: ${result.error.message}`);
  process.exit(1);
}

process.exit(result.status ?? 1);
