#!/usr/bin/env node
const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
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

if (process.platform !== "win32") {
  const source =
    process.env.REPOVOW_BIN || path.join(__dirname, "..", "vendor", "repovow");
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "repovow-shim-"));
  const nonExecutable = path.join(tempDir, "repovow");

  try {
    fs.copyFileSync(source, nonExecutable);
    fs.chmodSync(nonExecutable, 0o644);
    const modeResult = spawnSync(process.execPath, [shim, "--version"], {
      encoding: "utf8",
      env: { ...process.env, REPOVOW_BIN: nonExecutable },
    });

    if (modeResult.status !== 0 || modeResult.stdout.trim() !== `repovow ${version}`) {
      console.error(modeResult.stderr || modeResult.stdout);
      process.exit(modeResult.status || 1);
    }
    if ((fs.statSync(nonExecutable).mode & 0o111) === 0) {
      console.error("shim did not restore executable permissions");
      process.exit(1);
    }
    console.log("shim permissions: ok");
  } finally {
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
}
