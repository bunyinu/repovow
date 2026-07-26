#!/usr/bin/env node
/**
 * RepoVow npm shim — resolves the native binary for the current platform.
 */
const { spawn } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const PLATFORM_PACKAGE_BY_TARGET = {
  "linux-x64": "repovow-linux-x64-gnu",
  "linux-arm64": "repovow-linux-arm64-gnu",
  "darwin-x64": "repovow-darwin-x64",
  "darwin-arm64": "repovow-darwin-arm64",
};

function platformKey() {
  return `${process.platform}-${process.arch}`;
}

function resolveFromOptionalPackage() {
  const pkgName = PLATFORM_PACKAGE_BY_TARGET[platformKey()];
  if (!pkgName) return null;

  try {
    const pkgJson = require.resolve(`${pkgName}/package.json`);
    const binPath = path.join(path.dirname(pkgJson), "bin", "repovow");
    if (fs.existsSync(binPath)) return binPath;
  } catch {
    // optional dependency not installed
  }
  return null;
}

function resolveVendorBinary() {
  const vendor = path.join(__dirname, "..", "vendor", "repovow");
  if (fs.existsSync(vendor)) return vendor;
  const dev = path.join(__dirname, "..", "..", "..", "target", "release", "repovow");
  if (fs.existsSync(dev)) return dev;
  return null;
}

function resolveBinary() {
  if (process.env.REPOVOW_BIN || process.env.KEEL_BIN) {
    return process.env.REPOVOW_BIN || process.env.KEEL_BIN;
  }
  return resolveFromOptionalPackage() || resolveVendorBinary() || "repovow";
}

function ensureExecutable(bin) {
  if (process.platform === "win32" || !fs.existsSync(bin)) return;

  const mode = fs.statSync(bin).mode & 0o777;
  if ((mode & 0o111) === 0) {
    fs.chmodSync(bin, mode | 0o111);
  }
}

function main() {
  const bin = resolveBinary();
  try {
    ensureExecutable(bin);
  } catch (err) {
    console.error(`repovow: cannot make native binary executable: ${err.message}`);
    process.exit(1);
  }

  const child = spawn(bin, process.argv.slice(2), {
    stdio: "inherit",
    env: {
      ...process.env,
      REPOVOW_HOOK_BIN: process.env.REPOVOW_HOOK_BIN || path.resolve(__filename),
    },
  });

  child.on("error", (err) => {
    if (err.code === "ENOENT") {
      console.error(
        "repovow: native binary not found.\n" +
          "Install with: npm install -g repovow\n" +
          "Or build from source: cargo install --path ."
      );
    } else {
      console.error(`repovow: ${err.message}`);
    }
    process.exit(1);
  });

  child.on("close", (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal);
      return;
    }
    process.exit(code ?? 1);
  });
}

main();
