#!/usr/bin/env node

/**
 * Smoke-test the installed Linux Tauri package under an Xvfb display.
 *
 * The CI wrapper installs the .deb before invoking this script. The script resolves
 * controls through AT-SPI and drives the real packaged window with xdotool, then verifies the same
 * chosen-home, export, restart, and Fixture-containment contracts as macOS.
 * It also probes repair-dialog focus containment via AT-SPI (not screen-reader conformance)
 * and drives hermetic multi-Source Detect Sources + Start Sync Run before the Fixture
 * search/detail/Attempt-history/renormalize/curation/export journey.
 */

import { execFile, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { promises as fs } from "node:fs";
import os from "node:os";
import path from "node:path";
import { promisify } from "node:util";

import {
  DETECT_SIBLING_SECRET,
  hermeticSourceRoots,
  seedHermeticMultisourceRoots,
} from "./packaged-hermetic-multisource.mjs";

const execFileAsync = promisify(execFile);
const appRoot = path.resolve(import.meta.dirname, "..");
const repoRoot = path.resolve(appRoot, "../..");
const tauriRoot = path.join(appRoot, "src-tauri");

function fail(message) {
  throw new Error(message);
}

async function command(commandName, args, options = {}) {
  try {
    const result = await execFileAsync(commandName, args, {
      cwd: repoRoot,
      maxBuffer: 4 * 1024 * 1024,
      ...options,
    });
    return { ...result, code: 0 };
  } catch (error) {
    if (options.allowFailure) {
      return {
        stdout: error.stdout ?? "",
        stderr: error.stderr ?? error.message,
        code: error.code ?? 1,
      };
    }
    throw error;
  }
}

async function requireCommand(commandName) {
  await command("sh", ["-c", `command -v ${commandName}`]);
}

async function collectFiles(root) {
  const files = [];
  async function visit(current) {
    const entries = await fs.readdir(current, { withFileTypes: true });
    for (const entry of entries) {
      const next = path.join(current, entry.name);
      if (entry.isDirectory()) await visit(next);
      else files.push(path.relative(root, next));
    }
  }
  await visit(root);
  return files.sort();
}

async function collectFileHashes(root) {
  const hashes = {};
  async function visit(current) {
    const entries = await fs.readdir(current, { withFileTypes: true });
    for (const entry of entries) {
      const next = path.join(current, entry.name);
      if (entry.isDirectory()) {
        await visit(next);
      } else {
        hashes[path.relative(root, next)] = createHash("sha256")
          .update(await fs.readFile(next))
          .digest("hex");
      }
    }
  }
  await visit(root);
  return hashes;
}

function isWithin(candidate, root) {
  const relative = path.relative(root, candidate);
  return relative === "" || (!relative.startsWith(`..${path.sep}`) && relative !== "..");
}

async function findArtifact(directory, suffix) {
  const entries = await fs.readdir(directory).catch(() => []);
  const artifact = entries
    .filter((entry) => entry.endsWith(suffix))
    .sort()
    .at(-1);
  return artifact ? path.join(directory, artifact) : null;
}

async function sleep(milliseconds) {
  await new Promise((resolve) => setTimeout(resolve, milliseconds));
}

async function xdotool(args, options = {}) {
  return command("xdotool", args, options);
}

async function key(windowId, value) {
  await xdotool(["key", "--window", windowId, value]);
}

async function focusWindow(windowId) {
  const focused = await xdotool(["windowfocus", "--sync", windowId], {
    allowFailure: true,
  });
  if (focused.code !== 0) {
    await xdotool(["windowactivate", "--sync", windowId], { allowFailure: true });
  }
}

async function accessibleBounds(name, contains = false) {
  const script = path.join(appRoot, "scripts/linux-atspi-bounds.py");
  const args = [script, "--name", name, "--interactive", "--timeout", "20"];
  if (contains) args.push("--contains");
  return JSON.parse((await command("python3", args)).stdout);
}

/**
 * Run the AT-SPI focus helper and parse its JSON stdout.
 * @param {string[]} args - helper flags after the script path
 * @param {string} failureLabel - typed failure prefix when the helper exits non-zero
 */
async function atspiFocus(args, failureLabel) {
  const script = path.join(appRoot, "scripts/linux-atspi-focus.py");
  const result = await command("python3", [script, ...args], { allowFailure: true });
  if (result.code !== 0) {
    const detail = String(result.stderr || result.stdout || "").trim();
    fail(`${failureLabel}: ${detail || `exit ${result.code}`}`);
  }
  try {
    return JSON.parse(result.stdout);
  } catch {
    fail(`${failureLabel}: helper returned non-JSON stdout`);
  }
}

/**
 * Assert a named accessible is focused (polling inside the helper).
 * @param {string} name - exact accessible name
 */
async function assertAccessibleFocused(name) {
  return atspiFocus(
    ["--assert-focused", "--name", name, "--timeout", "20"],
    `dialog-focus: expected focused accessible "${name}"`,
  );
}

/**
 * Assert a named dialog has at least one focused descendant.
 * @param {string} name - dialog accessible name
 */
async function assertDialogHasFocusedDescendant(name) {
  const report = await atspiFocus(
    ["--dialog-focus", "--name", name, "--timeout", "20"],
    `dialog-focus: expected focused descendant inside "${name}"`,
  );
  if (!Array.isArray(report.focused) || report.focused.length === 0) {
    fail(`dialog-focus: dialog "${name}" reported no focused descendants`);
  }
  return report;
}

async function clickAccessible(windowId, name, contains = false) {
  const bounds = await accessibleBounds(name, contains);
  await focusWindow(windowId);
  await xdotool([
    "mousemove",
    "--sync",
    String(Math.round(bounds.x + bounds.width / 2)),
    String(Math.round(bounds.y + bounds.height / 2)),
  ]);
  await xdotool(["click", "1"]);
}

async function typeIntoAccessible(windowId, name, value) {
  await clickAccessible(windowId, name);
  await key(windowId, "ctrl+a");
  await xdotool([
    "type",
    "--window",
    windowId,
    "--clearmodifiers",
    "--delay",
    "1",
    value,
  ]);
}

/**
 * Wait for any AT-SPI accessible name (including static status text).
 * @param {string} name - exact or substring match
 * @param {boolean} contains - substring match when true
 */
async function waitForAccessibleText(name, contains = false) {
  const script = path.join(appRoot, "scripts/linux-atspi-find.py");
  const args = [script, "--name", name, "--timeout", "30"];
  if (contains) args.push("--contains");
  const result = await command("python3", args, { allowFailure: true });
  if (result.code !== 0) {
    const detail = String(result.stderr || result.stdout || "").trim();
    fail(`accessible-text: ${detail || `exit ${result.code}`}`);
  }
  return JSON.parse(result.stdout);
}

/**
 * Fail if any AT-SPI accessible name contains a forbidden fragment.
 * @param {string} fragment
 */
async function assertAccessibleNameOmits(fragment) {
  const script = path.join(appRoot, "scripts/linux-atspi-find.py");
  const result = await command(
    "python3",
    [script, "--name", fragment, "--contains", "--timeout", "2"],
    { allowFailure: true },
  );
  if (result.code === 0) {
    fail(`accessible-text: leaked forbidden fragment "${fragment}"`);
  }
  const detail = String(result.stderr || result.stdout || "").trim();
  if (result.code !== 1 || !detail.startsWith("AT-SPI accessible not found:")) {
    fail(`accessible-text: redaction probe failed: ${detail || `exit ${result.code}`}`);
  }
}

/**
 * Enable one Source preference draft and type its configured root.
 * @param {string} windowId
 * @param {string} kind - Source kind label used in accessible names
 * @param {string} root
 */
async function configureSourceDraft(windowId, kind, root) {
  await clickAccessible(windowId, `Enable ${kind}`);
  await typeIntoAccessible(windowId, `${kind} source root`, root);
}

async function waitForWindow(processHandle, label) {
  for (let attempt = 0; attempt < 60; attempt += 1) {
    if (processHandle.launchError) throw processHandle.launchError;
    const result = await xdotool(["search", "--onlyvisible", "--name", "^Distill$"], {
      allowFailure: true,
    });
    const windowId = result.stdout.trim().split("\n").filter(Boolean).at(-1);
    if (windowId) {
      await focusWindow(windowId);
      await xdotool(["windowsize", windowId, "1000", "900"]);
      return windowId;
    }
    await sleep(250);
  }
  if (processHandle.launchError) throw processHandle.launchError;
  fail(`${label} Distill window did not appear under Xvfb`);
}

function launch(binary, label) {
  const processHandle = spawn(binary, [], {
    cwd: repoRoot,
    env: {
      ...process.env,
      WEBKIT_DISABLE_COMPOSITING_MODE: "1",
      GDK_BACKEND: process.env.GDK_BACKEND ?? "x11",
    },
    stdio: "ignore",
  });
  processHandle.launchError = null;
  processHandle.once("error", (error) => {
    processHandle.launchError = new Error(
      `${label} packaged host failed to launch: ${error.message}`,
    );
  });
  return processHandle;
}

async function stopProcess(processHandle) {
  if (!processHandle || processHandle.exitCode !== null) return;
  processHandle.kill("SIGTERM");
  await sleep(750);
  if (processHandle.exitCode === null) processHandle.kill("SIGKILL");
}

async function waitForFile(filePath) {
  for (let attempt = 0; attempt < 120; attempt += 1) {
    try {
      await fs.access(filePath);
      return;
    } catch {
      await sleep(250);
    }
  }
  fail(`expected packaged artifact did not appear: ${path.basename(filePath)}`);
}

async function runUiJourney(binary, home, roots) {
  const processHandle = launch(binary, "initial");
  try {
    const windowId = await waitForWindow(processHandle, "initial");
    await typeIntoAccessible(windowId, "Distill home", home);
    await typeIntoAccessible(windowId, "Fixture root", roots.fixtureRoot);

    // Packaged repair-dialog focus containment (AT-SPI FOCUSED state only; not SR).
    await clickAccessible(windowId, "Repair library");
    await assertDialogHasFocusedDescendant("Confirm destructive repair");
    await key(windowId, "Tab");
    await assertDialogHasFocusedDescendant("Confirm destructive repair");
    await key(windowId, "Escape");
    await assertAccessibleFocused("Repair library");

    // Hermetic multi-Source Detect: missing Codex sibling must warn without leaking the secret.
    await configureSourceDraft(windowId, "codex", roots.missingSiblingRoot);
    await configureSourceDraft(windowId, "claude_code", roots.claudeRoot);
    await configureSourceDraft(windowId, "opencode", roots.opencodeRoot);
    await configureSourceDraft(windowId, "droid", roots.droidRoot);
    await clickAccessible(windowId, "Detect Sources");
    await waitForAccessibleText("Status: warning", true);
    await waitForAccessibleText("codex: unhealthy", true);
    for (const expected of [
      "fixture: ok",
      "claude_code: unavailable",
      "opencode: ok",
      "droid: ok",
    ]) {
      await waitForAccessibleText(expected, true);
    }
    // Detect result copy must stay redacted; clear the missing path before scanning names.
    await typeIntoAccessible(windowId, "codex source root", roots.codexRoot);
    await assertAccessibleNameOmits(DETECT_SIBLING_SECRET);

    // Sync Fixture + hermetic providers through Start Sync Run (not Run Fixture journey).
    await clickAccessible(windowId, "Start Sync Run");
    await waitForFile(path.join(home, "distill.db"));
    await waitForAccessibleText("Status: success", true);
    for (const kind of ["fixture", "codex", "claude_code", "opencode", "droid"]) {
      await waitForAccessibleText(`${kind}: completed`, true);
    }

    await clickAccessible(windowId, "Load sessions");
    await typeIntoAccessible(windowId, "Search sessions", "smoke");
    await key(windowId, "Enter");
    await clickAccessible(windowId, roots.fixtureSessionTitle, true);

    // Attempt history and same-Capture renormalize stay bridge-only: the UI discovers
    // the Capture through Activity, then exposes immutable Attempt summaries and the
    // Distill-owned retry report without parser-version or provider-root controls.
    await clickAccessible(windowId, "Load Attempt history");
    await waitForAccessibleText("Status: ready", true);
    await waitForAccessibleText("Capture 1", true);
    await waitForAccessibleText("#1", true);
    await waitForAccessibleText("fixture/1.0.0", true);
    await waitForAccessibleText("succeeded", true);
    await clickAccessible(windowId, "Renormalize Capture");
    await waitForAccessibleText("Renormalize: ready", true);
    await waitForAccessibleText("Capture 1", true);
    await waitForAccessibleText("attempt 2", true);
    await waitForAccessibleText("#2", true);
    await waitForAccessibleText("fixture/1.0.0", true);
    await waitForAccessibleText("succeeded", true);

    await clickAccessible(windowId, "train");
    await clickAccessible(windowId, "Preview export");
    await clickAccessible(windowId, "Publish export");
    const exportDirectory = path.join(home, "exports");
    for (let attempt = 0; attempt < 60; attempt += 1) {
      const exportFiles = (await fs.readdir(exportDirectory).catch(() => [])).filter(
        (file) => file.endsWith(".jsonl"),
      );
      if (exportFiles.length > 0) return;
      await sleep(250);
    }
    fail("packaged Linux export did not appear");
  } finally {
    await stopProcess(processHandle);
  }
}

if (process.platform !== "linux") fail("desktop:smoke:linux requires Linux");
await requireCommand("xdotool");
await requireCommand("xvfb-run");
await requireCommand("python3");
const debDirectory = path.join(repoRoot, "target/release/bundle/deb");
const appImageDirectory = path.join(repoRoot, "target/release/bundle/appimage");
const debPath =
  process.env.DISTILL_LINUX_DEB ?? (await findArtifact(debDirectory, ".deb"));
const appImagePath =
  process.env.DISTILL_LINUX_APPIMAGE ??
  (await findArtifact(appImageDirectory, ".AppImage"));
if (!debPath || !appImagePath)
  fail("Linux package artifacts are missing; run desktop:package:linux");
await fs.access(debPath);
await fs.access(appImagePath);
await requireCommand("dpkg-deb");
const debDepends = (await command("dpkg-deb", ["-f", debPath, "Depends"])).stdout.trim();
const dependencyNames = debDepends.split(",").map((dependency) => dependency.trim());
const hasWebKitDependency = dependencyNames.some((dependency) =>
  dependency.startsWith("libwebkit2gtk-4.1-0"),
);
const hasGtkDependency = dependencyNames.some((dependency) =>
  dependency.startsWith("libgtk-3-0"),
);
if (!hasWebKitDependency || !hasGtkDependency) {
  fail(`Linux .deb Depends metadata is incomplete: ${debDepends || "<empty>"}`);
}

const binary = process.env.DISTILL_LINUX_BINARY ?? "/usr/bin/distill-desktop";
const binaryStat = await fs.stat(binary);
if ((binaryStat.mode & 0o111) === 0)
  fail(`installed Linux host is not executable: ${binary}`);
const capabilityPath = path.join(tauriRoot, "capabilities/default.json");
const capability = JSON.parse(await fs.readFile(capabilityPath, "utf8"));
if (JSON.stringify(capability.permissions) !== JSON.stringify(["core:event:default"])) {
  fail("Linux packaged capability source is broader than core:event:default");
}

const base = await fs.mkdtemp(path.join(os.tmpdir(), "distill-linux-smoke-"));
const home = path.join(base, "home");
const sessionTitle = "Linux Package Smoke";
const roots = await seedHermeticMultisourceRoots(base, {
  fixtureSessionTitle: sessionTitle,
  fixtureExternalSessionId: "linux-package-smoke",
});
const sourceRoots = hermeticSourceRoots(roots);

const beforeSourceSnapshots = {};
for (const root of sourceRoots) {
  beforeSourceSnapshots[root] = {
    files: await collectFiles(root),
    hashes: await collectFileHashes(root),
  };
}
const beforeBase = await collectFiles(base);
await runUiJourney(binary, home, roots);
const beforeRestart = await collectFiles(home);
const restartProcess = launch(binary, "restart");
try {
  await waitForWindow(restartProcess, "restart");
  const afterRestart = await collectFiles(home);
  if (JSON.stringify(beforeRestart) !== JSON.stringify(afterRestart)) {
    fail("Linux packaged restart changed the chosen home artifact set");
  }
} finally {
  await stopProcess(restartProcess);
}

const homeFiles = await collectFiles(home);
const exportFiles = (await fs.readdir(path.join(home, "exports"))).filter((file) =>
  file.endsWith(".jsonl"),
);
if (!homeFiles.includes("distill.db") || exportFiles.length === 0) {
  fail("Linux packaged home is missing distill.db or a JSONL export");
}
const exportContents = await Promise.all(
  exportFiles.map((file) => fs.readFile(path.join(home, "exports", file), "utf8")),
);
const exportRecords = exportContents.flatMap((contents) =>
  contents
    .split("\n")
    .filter((line) => line.trim().length > 0)
    .map((line) => JSON.parse(line)),
);
if (
  !exportRecords.some(
    (record) =>
      record.external_session_id === "linux-package-smoke" &&
      Array.isArray(record.labels) &&
      record.labels.includes("train"),
  )
) {
  fail("Linux packaged export did not contain the curated Fixture session in train");
}
for (const root of sourceRoots) {
  const afterFiles = await collectFiles(root);
  const afterHashes = await collectFileHashes(root);
  if (
    JSON.stringify(beforeSourceSnapshots[root].files) !== JSON.stringify(afterFiles) ||
    JSON.stringify(beforeSourceSnapshots[root].hashes) !== JSON.stringify(afterHashes)
  ) {
    fail(
      `Linux packaged smoke changed hermetic source files under ${path.basename(root)}`,
    );
  }
}
const afterBase = await collectFiles(base);
for (const file of afterBase.filter((entry) => !beforeBase.includes(entry))) {
  const absolute = path.join(base, file);
  const allowed = [home, ...sourceRoots].some((root) => isWithin(absolute, root));
  if (!allowed) {
    fail(`Linux packaged write escaped chosen home/hermetic source roots: ${file}`);
  }
}

console.log(
  JSON.stringify(
    {
      machine: `${os.platform()} ${os.arch()}`,
      deb: path.basename(debPath),
      appimage: path.basename(appImagePath),
      deb_depends: debDepends,
      installed_binary: binary,
      capabilities: capability.permissions,
      ui: "passed",
      restart: "passed",
      hermetic_multisource: "passed",
      detect_sibling_isolation: "passed",
      attempt_history_renormalize: "passed",
      home,
      fixture_root: roots.fixtureRoot,
      hermetic_roots: {
        codex: roots.codexRoot,
        claude_code: roots.claudeRoot,
        opencode: roots.opencodeRoot,
        droid: roots.droidRoot,
      },
      home_files: homeFiles,
      export_files: exportFiles,
      non_claims: [
        "installed Ubuntu smoke only; hermetic temp roots only — no host-installed provider claim",
        "no migration, crash-recovery, privacy, scale, export-atomicity, or screen-reader claim",
      ],
    },
    null,
    2,
  ),
);
