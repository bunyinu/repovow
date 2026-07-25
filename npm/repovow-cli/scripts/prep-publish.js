#!/usr/bin/env node
/** Rewrite optionalDependencies to registry versions before npm publish. */
const fs = require("node:fs");
const path = require("node:path");

const version = process.argv[2];
if (!version) {
  console.error("usage: prep-publish.js <version>");
  process.exit(1);
}

const pkgPath = path.join(__dirname, "..", "package.json");
const pkg = JSON.parse(fs.readFileSync(pkgPath, "utf8"));

pkg.optionalDependencies = {
  "repovow-linux-x64-gnu": version,
  "repovow-linux-arm64-gnu": version,
  "repovow-darwin-x64": version,
  "repovow-darwin-arm64": version,
};

fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");
console.log("Prepared repovow for npm publish");
