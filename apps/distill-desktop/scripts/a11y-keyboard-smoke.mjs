/**
 * Lightweight post-build smoke: run renderer a11y/state Vitest suites.
 * Does not claim signed packaged WebView focus or screen-reader verification.
 */

import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

/**
 * Run a command in the desktop package and exit non-zero on failure.
 * @param label - human-readable step name
 * @param args - npm script arguments
 */
function run(label, args) {
  console.log(`[a11y-smoke] ${label}`);
  const result = spawnSync("npm", args, {
    cwd: root,
    stdio: "inherit",
    shell: process.platform === "win32",
  });
  if (result.status !== 0) {
    console.error(`[a11y-smoke] failed: ${label}`);
    process.exit(result.status ?? 1);
  }
}

run("frontend build", ["run", "build"]);
run("renderer a11y/state tests", [
  "run",
  "test",
  "--",
  "src/App.a11y.test.tsx",
  "src/App.states.test.tsx",
  "src/styles.a11y.test.ts",
]);

console.log(
  "[a11y-smoke] renderer keyboard/state suites passed after build (not packaged WebView/SR).",
);
