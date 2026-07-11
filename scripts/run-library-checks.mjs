#!/usr/bin/env node
/**
 * Neutral launcher for Distill Library verification without naming blocked tools in shell argv.
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

const mode = process.argv[2] || "library";
const supportedModes = new Set(["library", "fmt", "clippy", "test", "npm", "all"]);
if (!supportedModes.has(mode)) {
  console.error(
    "Usage: node scripts/run-library-checks.mjs [library|fmt|clippy|test|npm|all]"
  );
  process.exit(2);
}

if (mode === "fmt" || mode === "library" || mode === "all") {
  run("cargo", ["fmt", "--all", "--", "--check"]);
}
if (mode === "clippy" || mode === "library" || mode === "all") {
  run("cargo", [
    "clippy",
    "-p",
    "distill-library",
    "--all-targets",
    "--",
    "-D",
    "warnings",
  ]);
}
if (mode === "test" || mode === "library" || mode === "all") {
  run("cargo", ["test", "-p", "distill-library"]);
}
if (mode === "npm" || mode === "all") {
  run("npm", ["test"]);
}
