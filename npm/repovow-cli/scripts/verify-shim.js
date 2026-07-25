#!/usr/bin/env node
const { spawnSync } = require("node:child_process");
const path = require("node:path");
const { version } = require("../package.json");

const shim = path.join(__dirname, "..", "bin", "repovow.js");
const result = spawnSync(process.execPath, [shim, "--version"], {
  encoding: "utf8",
  env: { ...process.env, REPOVOW_BIN: process.env.REPOVOW_BIN || undefined },
});

if (result.status !== 0) {
  console.error(result.stderr || result.stdout);
  process.exit(result.status || 1);
}

if (result.stdout.trim() !== `repovow ${version}`) {
  console.error("unexpected version output:", result.stdout);
  process.exit(1);
}

console.log("shim ok:", result.stdout.trim());
