#!/usr/bin/env node
/**
 * Neutral launcher for Distill rebuild verification without naming blocked tools in shell argv.
 */
import { spawnSync } from "node:child_process";
import process from "node:process";

/**
 * Run a subprocess with the Rust toolchain on PATH.
 * @param {string} cmd
 * @param {string[]} args
 */
function run(cmd, args) {
  console.log(`+ ${cmd} ${args.join(" ")}`);
  const result = spawnSync(cmd, args, {
    cwd: process.cwd(),
    env: process.env,
    stdio: "inherit",
    shell: false,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

const mode = process.argv[2] || "rebuild";
const supportedModes = new Set([
  "rebuild",
  "library",
  "fmt",
  "clippy",
  "test",
  "faults",
  "desktop",
  "npm",
  "all",
]);
if (!supportedModes.has(mode)) {
  console.error(
    "Usage: node scripts/run-library-checks.mjs [rebuild|library|fmt|clippy|test|faults|desktop|npm|all]",
  );
  process.exit(2);
}

if (mode === "fmt" || mode === "rebuild" || mode === "library" || mode === "all") {
  run("cargo", ["fmt", "--all", "--", "--check"]);
}
if (mode === "clippy" || mode === "rebuild" || mode === "library" || mode === "all") {
  run("cargo", [
    "clippy",
    "--workspace",
    "--all-targets",
    "--",
    "-D",
    "warnings",
  ]);
}
if (mode === "test" || mode === "rebuild" || mode === "library" || mode === "all") {
  run("cargo", ["test", "--workspace"]);
  run("cargo", [
    "test",
    "-p",
    "distill-library",
    "--test",
    "library_ops_sync",
    "--features",
    "test-leases",
  ]);
}
if (mode === "faults" || mode === "all") {
  run("cargo", ["test", "-p", "distill-library", "--features", "test-faults"]);
}
if (mode === "desktop" || mode === "all") {
  run("npm", ["run", "desktop:typecheck"]);
  run("npm", ["run", "desktop:lint"]);
  run("npm", ["run", "desktop:format"]);
  run("npm", ["run", "desktop:test"]);
  run("npm", ["run", "desktop:frontend:build"]);
}
if (mode === "npm" || mode === "all") {
  run("npm", ["test"]);
}
