#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import process from "node:process";

const root = process.cwd();

function read(relativePath) {
  return fs.readFileSync(path.join(root, relativePath), "utf8");
}

function readJson(relativePath) {
  return JSON.parse(read(relativePath));
}

function fail(message) {
  console.error(`release check failed: ${message}`);
  process.exit(1);
}

const rootPackage = readJson("package.json");
const desktopPackage = readJson("apps/distill-desktop/package.json");
const tauri = readJson("apps/distill-desktop/src-tauri/tauri.conf.json");
const windowsBetaTauri = readJson(
  "apps/distill-desktop/src-tauri/tauri.windows.beta.conf.json",
);
const cargo = read("Cargo.toml");
const cargoVersion = cargo.match(/^version = "([^"]+)"$/m)?.[1];
const expected = rootPackage.version;

if (!expected?.includes("-beta.")) {
  fail(`expected a beta version, received ${expected ?? "<missing>"}`);
}

const expectedWindowsMsiVersion = expected.replace(/-beta\.(\d+)$/, "-$1");
if (windowsBetaTauri.version !== expectedWindowsMsiVersion) {
  fail(
    `Windows beta Tauri version ${windowsBetaTauri.version} does not map ${expected} to the numeric MSI prerelease ${expectedWindowsMsiVersion}`,
  );
}

const versions = {
  root: rootPackage.version,
  desktop: desktopPackage.version,
  tauri: tauri.version,
  cargo: cargoVersion,
};
for (const [name, version] of Object.entries(versions)) {
  if (version !== expected)
    fail(`${name} version ${version} does not match ${expected}`);
}

for (const requiredPath of [
  "docs/release/first-beta.md",
  ".github/workflows/beta-release.yml",
  "apps/distill-desktop/src-tauri/tauri.linux.conf.json",
  "apps/distill-desktop/src-tauri/tauri.windows.beta.conf.json",
  "apps/distill-desktop/src-tauri/icons/icon.ico",
]) {
  if (!fs.existsSync(path.join(root, requiredPath)))
    fail(`missing ${requiredPath}`);
}

const smokeActivation =
  process.env.VITE_DISTILL_SMOKE_DOM_ACTIVATE?.trim().toLowerCase();
if (["1", "true", "yes", "on"].includes(smokeActivation)) {
  fail(
    "smoke-only renderer activation must not be enabled for release packaging",
  );
}

console.log(`release metadata OK: ${expected}`);
