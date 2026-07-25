#!/usr/bin/env node
/** Sync version across npm/repovow-cli and npm/platforms/* */
const fs = require("node:fs");
const path = require("node:path");

const version = process.argv[2];
if (!version) {
  console.error("usage: sync-version.js <version>");
  process.exit(1);
}

const root = path.join(__dirname, "..", "..");
const cliPkg = path.join(root, "repovow-cli", "package.json");
const cli = JSON.parse(fs.readFileSync(cliPkg, "utf8"));
cli.version = version;

const opt = cli.optionalDependencies || {};
for (const name of Object.keys(opt)) {
  opt[name] = version;
}
cli.optionalDependencies = opt;
fs.writeFileSync(cliPkg, JSON.stringify(cli, null, 2) + "\n");

const platformsDir = path.join(root, "platforms");
for (const dir of fs.readdirSync(platformsDir)) {
  const pkgPath = path.join(platformsDir, dir, "package.json");
  if (!fs.existsSync(pkgPath)) continue;
  const pkg = JSON.parse(fs.readFileSync(pkgPath, "utf8"));
  pkg.version = version;
  fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");
}

// Cargo.toml
const cargoPath = path.join(root, "..", "..", "Cargo.toml");
if (fs.existsSync(cargoPath)) {
  let cargo = fs.readFileSync(cargoPath, "utf8");
  cargo = cargo.replace(/^version = ".*"$/m, `version = "${version}"`);
  fs.writeFileSync(cargoPath, cargo);
}

console.log(`Synced version ${version}`);
