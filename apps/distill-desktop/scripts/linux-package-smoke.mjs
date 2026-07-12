#!/usr/bin/env node

/**
 * Smoke-test the installed Linux Tauri package under an Xvfb display.
 *
 * The CI wrapper installs the .deb before invoking this script. The script resolves
 * controls through AT-SPI and drives the real packaged window with xdotool, then verifies the same
 * chosen-home, export, restart, and Fixture-containment contracts as macOS.
 */

import { execFile, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { promises as fs } from "node:fs";
import os from "node:os";
import path from "node:path";
import { promisify } from "node:util";

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

async function accessibleBounds(name, contains = false) {
  const script = path.join(appRoot, "scripts/linux-atspi-bounds.py");
  const args = [script, "--name", name, "--interactive", "--timeout", "20"];
  if (contains) args.push("--contains");
  return JSON.parse((await command("python3", args)).stdout);
}

async function clickAccessible(windowId, name, contains = false) {
  const bounds = await accessibleBounds(name, contains);
  await xdotool(["windowactivate", "--sync", windowId]);
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

async function waitForWindow(processHandle, label) {
  for (let attempt = 0; attempt < 60; attempt += 1) {
    if (processHandle.launchError) throw processHandle.launchError;
    const result = await xdotool(["search", "--onlyvisible", "--name", "^Distill$"], {
      allowFailure: true,
    });
    const windowId = result.stdout.trim().split("\n").filter(Boolean).at(-1);
    if (windowId) {
      await xdotool(["windowactivate", "--sync", windowId]);
      await xdotool(["windowsize", windowId, "880", "720"]);
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
  for (let attempt = 0; attempt < 60; attempt += 1) {
    try {
      await fs.access(filePath);
      return;
    } catch {
      await sleep(250);
    }
  }
  fail(`expected packaged artifact did not appear: ${path.basename(filePath)}`);
}

async function runUiJourney(binary, home, fixtureRoot) {
  const processHandle = launch(binary, "initial");
  try {
    const windowId = await waitForWindow(processHandle, "initial");
    await typeIntoAccessible(windowId, "Distill home", home);
    await typeIntoAccessible(windowId, "Fixture root", fixtureRoot);
    await clickAccessible(windowId, "Run Fixture journey");
    await waitForFile(path.join(home, "distill.db"));

    await clickAccessible(windowId, "Load sessions");
    await typeIntoAccessible(windowId, "Search sessions", "smoke");
    await key(windowId, "Enter");
    await clickAccessible(windowId, "Linux Package Smoke", true);
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
const fixtureRoot = path.join(base, "fixture");
await fs.mkdir(path.join(fixtureRoot, "captures"), { recursive: true });
const sessionTitle = "Linux Package Smoke";
await fs.writeFile(
  path.join(fixtureRoot, "distill.fixture.json"),
  JSON.stringify({
    version: 1,
    captures: [
      {
        id: "linux-smoke",
        kind: "file",
        relative_path: "captures/linux-smoke.jsonl",
        external_session_id: "linux-package-smoke",
        title: sessionTitle,
      },
    ],
  }),
);
await fs.writeFile(
  path.join(fixtureRoot, "captures/linux-smoke.jsonl"),
  [
    JSON.stringify({
      record_type: "session_meta",
      title: sessionTitle,
      summary: "packaged smoke",
    }),
    JSON.stringify({ record_type: "message", role: "user", text: "smoke search" }),
    JSON.stringify({ record_type: "message", role: "assistant", text: "smoke response" }),
  ].join("\n") + "\n",
);

const beforeFixture = await collectFiles(fixtureRoot);
const beforeFixtureHashes = await collectFileHashes(fixtureRoot);
const beforeBase = await collectFiles(base);
await runUiJourney(binary, home, fixtureRoot);
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
const afterFixture = await collectFiles(fixtureRoot);
const afterFixtureHashes = await collectFileHashes(fixtureRoot);
if (
  JSON.stringify(beforeFixture) !== JSON.stringify(afterFixture) ||
  JSON.stringify(beforeFixtureHashes) !== JSON.stringify(afterFixtureHashes)
) {
  fail("Linux packaged smoke changed Fixture source files");
}
const afterBase = await collectFiles(base);
for (const file of afterBase.filter((entry) => !beforeBase.includes(entry))) {
  const absolute = path.join(base, file);
  if (!isWithin(absolute, home) && !isWithin(absolute, fixtureRoot)) {
    fail(`Linux packaged write escaped chosen home/Fixture roots: ${file}`);
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
      home,
      fixture_root: fixtureRoot,
      home_files: homeFiles,
      export_files: exportFiles,
      non_claims: [
        "installed Ubuntu smoke only; no migration, crash-recovery, privacy, scale, export-atomicity, or screen-reader claim",
      ],
    },
    null,
    2,
  ),
);
