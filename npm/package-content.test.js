#!/usr/bin/env node
"use strict";

const assert = require("assert");
const fs = require("fs");
const path = require("path");
const { supportedPlatforms, bin, files } = require("./package.json");

assert.strictEqual(bin.gd, "run.js");
assert.strictEqual(bin.grad, undefined);
assert(files.includes("install.js"));
assert(files.includes("platform.js"));
assert(files.includes("run.js"));

for (const [target, platform] of Object.entries(supportedPlatforms)) {
  assert(platform.artifact.startsWith(`grad-${target}`), `${target} artifact should match target`);
  assert(platform.binary, `${target} missing gd binary`);
  assert.strictEqual(platform.binary, target.includes("windows") ? "gd.exe" : "gd");
}

for (const workflow of ["release.yml", "homebrew.yml"]) {
  const text = fs.readFileSync(path.join(__dirname, "..", ".github", "workflows", workflow), "utf8");
  assert(text.includes("gd"), `${workflow} must package/install gd`);
}
