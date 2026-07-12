#!/usr/bin/env node

/**
 * Inspect and smoke-test the packaged macOS `.app`.
 *
 * The static checks are always automated. The short UI journey uses macOS
 * Accessibility via System Events; set DISTILL_MACOS_ALLOW_MANUAL=1 only when
 * the runner has no Accessibility permission and a human will complete the
 * same checklist. That mode is explicit and never claims a packaged journey.
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
const expectedIdentifier = "dev.distill.desktop";

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

async function findBundle() {
  if (process.env.DISTILL_MACOS_APP) return process.env.DISTILL_MACOS_APP;
  const candidates = [
    path.join(tauriRoot, "target/release/bundle/macos/Distill.app"),
    path.join(repoRoot, "target/release/bundle/macos/Distill.app"),
    path.join(appRoot, "target/release/bundle/macos/Distill.app"),
  ];
  for (const candidate of candidates) {
    try {
      const stat = await fs.stat(candidate);
      if (stat.isDirectory()) return candidate;
    } catch {
      // Keep looking in the known Tauri target locations.
    }
  }
  fail(
    `Distill.app not found; run npm run desktop:package:macos first. Tried: ${candidates.join(", ")}`,
  );
}

async function readPlist(appPath) {
  const plistPath = path.join(appPath, "Contents/Info.plist");
  const result = await command("plutil", ["-convert", "json", "-o", "-", plistPath]);
  return JSON.parse(result.stdout);
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

async function runPackagedUiJourney(appPath, home, fixtureRoot, sessionTitle) {
  const script = `
tell application "Distill" to activate
tell application "System Events"
  tell process "distill-desktop"
    repeat 30 times
      if (exists window 1) then exit repeat
      delay 0.25
    end repeat
    if not (exists window 1) then error "Distill window did not appear"
    set frontmost to true
    -- The native window appears before WebKit has populated its AX tree.
    set uiReady to false
    repeat 40 times
      try
        set nodes to entire contents of window 1
        repeat with node in nodes
          try
            if role of node is "AXTextField" and name of node is "Distill home" then
              set uiReady to true
              exit repeat
            end if
          end try
        end repeat
      end try
      if uiReady then exit repeat
      delay 0.25
    end repeat
    if uiReady is false then error "packaged renderer Accessibility tree did not become ready"
    set nodes to entire contents of window 1
    set homeFound to false
    set fixtureFound to false
    repeat with node in nodes
      try
        if role of node is "AXTextField" and name of node is "Distill home" then
          set the clipboard to ${JSON.stringify(home)}
          click node
          keystroke "a" using {command down}
          keystroke "v" using {command down}
          set homeFound to true
          exit repeat
        end if
      end try
    end repeat
    set nodes to entire contents of window 1
    repeat with node in nodes
      try
        if role of node is "AXTextField" and name of node is "Fixture root" then
          set the clipboard to ${JSON.stringify(fixtureRoot)}
          click node
          keystroke "a" using {command down}
          keystroke "v" using {command down}
          set fixtureFound to true
          exit repeat
        end if
      end try
    end repeat
    if homeFound is false or fixtureFound is false then error "packaged renderer text fields are not accessible"
    set nodes to entire contents of window 1
    set runFound to false
    repeat with node in nodes
      try
        if role of node is "AXButton" and name of node is "Run Fixture journey" then
          click node
          set runFound to true
          exit repeat
        end if
      end try
    end repeat
    if runFound is false then error "Run Fixture journey button is not accessible"
    delay 3
    set nodes to entire contents of window 1
    set loadFound to false
    repeat with node in nodes
      try
        if role of node is "AXButton" and name of node is "Load sessions" then
          click node
          set loadFound to true
          exit repeat
        end if
      end try
    end repeat
    if loadFound is false then error "Load sessions button is not accessible"
    delay 2
    set nodes to entire contents of window 1
    set sessionFound to false
    repeat with node in nodes
      try
        -- WebKit maps aria-pressed session rows to AXCheckBox controls.
        if (role of node is "AXCheckBox" or role of node is "AXButton") and name of node contains ${JSON.stringify(sessionTitle)} then
          click node
          set sessionFound to true
          exit repeat
        end if
      end try
    end repeat
    if sessionFound is false then error "packaged session row was not accessible"
    delay 2
    set nodes to entire contents of window 1
    set favoriteFound to false
    repeat with node in nodes
      try
        -- aria-pressed label controls can be AXButton or AXCheckBox in WebKit.
        if (role of node is "AXButton" or role of node is "AXCheckBox") and name of node is "train" then
          click node
          set favoriteFound to true
          exit repeat
        end if
      end try
    end repeat
    if favoriteFound is false then error "favorite curation button is not accessible"
    delay 1
    set nodes to entire contents of window 1
    set previewFound to false
    repeat with node in nodes
      try
        if role of node is "AXButton" and name of node is "Preview export" then
          click node
          set previewFound to true
          exit repeat
        end if
      end try
    end repeat
    if previewFound is false then error "Preview export button is not accessible"
    delay 2
    set nodes to entire contents of window 1
    set publishFound to false
    repeat with node in nodes
      try
        if role of node is "AXButton" and name of node is "Publish export" then
          click node
          set publishFound to true
          exit repeat
        end if
      end try
    end repeat
    if publishFound is false then error "Publish export button is not accessible"
    delay 3
    -- Keyboard pass: focus the search control and submit with Enter.
    set nodes to entire contents of window 1
    set searchFound to false
    repeat with node in nodes
      try
        if role of node is "AXTextField" and name of node is "Search sessions" then
          click node
          keystroke "smoke"
          key code 36
          set searchFound to true
          exit repeat
        end if
      end try
    end repeat
    if searchFound is false then error "Search sessions field is not accessible"
    delay 1
    set nodes to entire contents of window 1
    set searchResultFound to false
    repeat with node in nodes
      try
        if (role of node is "AXCheckBox" or role of node is "AXButton") and name of node contains ${JSON.stringify(sessionTitle)} then
          set searchResultFound to true
          exit repeat
        end if
      end try
    end repeat
    if searchResultFound is false then error "packaged search did not retain the Fixture session result"
  end tell
end tell
`;
  await command("osascript", ["-e", script]);
}

async function stopApp() {
  await command("osascript", ["-e", 'tell application "Distill" to quit'], {
    allowFailure: true,
  });
  await new Promise((resolve) => setTimeout(resolve, 750));
}

async function waitForPackagedWindow() {
  const script = `
tell application "System Events"
  tell process "distill-desktop"
    repeat 30 times
      if (exists window 1) then exit repeat
      delay 0.25
    end repeat
    if not (exists window 1) then error "Distill restart window did not appear"
  end tell
end tell
`;
  await command("osascript", ["-e", script]);
}

if (process.platform !== "darwin") fail("desktop:smoke:macos requires macOS");

const appPath = await findBundle();
const plist = await readPlist(appPath);
if (plist.CFBundleIdentifier !== expectedIdentifier) {
  fail(`unexpected bundle identifier: ${plist.CFBundleIdentifier}`);
}
if (plist.CFBundleName !== "Distill" || plist.CFBundleDisplayName !== "Distill") {
  fail(`unexpected bundle name: ${plist.CFBundleName ?? plist.CFBundleDisplayName}`);
}
if (plist.LSMinimumSystemVersion !== "12.0") {
  fail(`unexpected minimum macOS version: ${plist.LSMinimumSystemVersion}`);
}
const executable = path.join(appPath, "Contents/MacOS/distill-desktop");
const icon = path.join(appPath, "Contents/Resources/icon.icns");
await fs.access(executable);
await fs.access(icon);

const capabilityPath = path.join(tauriRoot, "capabilities/default.json");
const capability = JSON.parse(await fs.readFile(capabilityPath, "utf8"));
if (JSON.stringify(capability.permissions) !== JSON.stringify(["core:event:default"])) {
  fail("packaged capability source is broader than core:event:default");
}

const codesign = await command("codesign", ["-dv", "--verbose=4", appPath], {
  allowFailure: true,
});
const signingText = `${codesign.stdout}\n${codesign.stderr}`;
const signing = signingText.includes("adhoc")
  ? "adhoc"
  : codesign.code === 0
    ? "signed"
    : "unsigned";
if (!["adhoc", "unsigned"].includes(signing)) {
  fail(`local package unexpectedly reports ${signing} signing; expected ad-hoc/unsigned`);
}
const base = await fs.mkdtemp(path.join(os.tmpdir(), "distill-macos-smoke-"));
const home = path.join(base, "home");
const fixtureRoot = path.join(base, "fixture");
const sessionTitle = "macOS Package Smoke";
await fs.mkdir(path.join(fixtureRoot, "captures"), { recursive: true });
await fs.writeFile(
  path.join(fixtureRoot, "distill.fixture.json"),
  JSON.stringify({
    version: 1,
    captures: [
      {
        id: "macos-smoke",
        kind: "file",
        relative_path: "captures/macos-smoke.jsonl",
        external_session_id: "macos-package-smoke",
        title: sessionTitle,
      },
    ],
  }),
);
await fs.writeFile(
  path.join(fixtureRoot, "captures/macos-smoke.jsonl"),
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
const evidence = {
  machine: `${os.platform()} ${os.arch()}`,
  app_path: appPath,
  bundle_identifier: plist.CFBundleIdentifier,
  bundle_name: plist.CFBundleName,
  minimum_system_version: plist.LSMinimumSystemVersion,
  icon: path.relative(appPath, icon),
  capabilities: capability.permissions,
  signing,
  notarization: "not assessed as notarized by the local unsigned gate",
  ui: "pending",
  home,
  fixture_root: fixtureRoot,
};

let uiError = null;
let restartError = null;
const appProcess = spawn("open", ["-n", appPath], { stdio: "ignore" });
try {
  try {
    await runPackagedUiJourney(appPath, home, fixtureRoot, sessionTitle);
    evidence.ui = "passed";
  } catch (error) {
    uiError = error;
    evidence.ui = "manual_required";
    evidence.ui_error = error instanceof Error ? error.message : String(error);
  }

  if (!uiError) {
    const beforeRestart = await collectFiles(home);
    let restartProcess = null;
    try {
      await stopApp();
      restartProcess = spawn("open", ["-n", appPath], { stdio: "ignore" });
      await waitForPackagedWindow();
      const afterRestart = await collectFiles(home);
      if (
        !afterRestart.some(
          (file) => file.startsWith("exports/") && file.endsWith(".jsonl"),
        )
      ) {
        fail("packaged restart did not preserve a JSONL export under the chosen home");
      }
      if (JSON.stringify(beforeRestart) !== JSON.stringify(afterRestart)) {
        fail("packaged restart changed the chosen home artifact set");
      }
      evidence.restart = "passed";
    } catch (error) {
      restartError = error;
      evidence.restart = "failed";
      evidence.restart_error = error instanceof Error ? error.message : String(error);
    } finally {
      await stopApp();
      restartProcess?.kill();
    }
  } else {
    evidence.restart = "not_run";
  }
} finally {
  await stopApp();
  appProcess.kill();
}

if (restartError) throw restartError;
if (uiError && process.env.DISTILL_MACOS_ALLOW_MANUAL !== "1") {
  console.error(JSON.stringify(evidence, null, 2));
  throw uiError;
}

const homeFiles = await collectFiles(home).catch(() => []);
const exportsDir = path.join(home, "exports");
const exported = await fs.readdir(exportsDir).catch(() => []);
const exportFiles = exported.filter((name) => name.endsWith(".jsonl"));
if (evidence.ui === "passed" && exportFiles.length === 0) {
  fail("packaged UI journey did not leave a JSONL export under the chosen home");
}
if (evidence.ui === "passed" && !homeFiles.includes("distill.db")) {
  fail("packaged UI journey did not leave distill.db under the chosen home");
}
if (evidence.ui === "passed") {
  const exportBytes = await Promise.all(
    exportFiles.map((name) => fs.readFile(path.join(exportsDir, name), "utf8")),
  );
  if (!exportBytes.some((contents) => contents.trim().length > 0)) {
    fail("packaged UI journey left only empty JSONL export artifacts");
  }
  const exportRecords = exportBytes.flatMap((contents) =>
    contents
      .split("\n")
      .filter((line) => line.trim().length > 0)
      .map((line) => JSON.parse(line)),
  );
  if (
    !exportRecords.some(
      (record) =>
        record.external_session_id === "macos-package-smoke" &&
        Array.isArray(record.labels) &&
        record.labels.includes("train"),
    )
  ) {
    fail("packaged export did not contain the curated Fixture session in train");
  }
}
const afterFixture = await collectFiles(fixtureRoot);
if (JSON.stringify(beforeFixture) !== JSON.stringify(afterFixture)) {
  fail("packaged smoke modified the Fixture source root");
}
const afterFixtureHashes = await collectFileHashes(fixtureRoot);
if (JSON.stringify(beforeFixtureHashes) !== JSON.stringify(afterFixtureHashes)) {
  fail("packaged smoke changed Fixture source contents");
}
const afterBase = await collectFiles(base);
const newBaseFiles = afterBase.filter((file) => !beforeBase.includes(file));
for (const file of newBaseFiles) {
  const absolute = path.join(base, file);
  if (!isWithin(absolute, home) && !isWithin(absolute, fixtureRoot)) {
    fail(`packaged write escaped chosen home/Fixture roots: ${file}`);
  }
}
for (const file of homeFiles) {
  if (!isWithin(path.join(home, file), home))
    fail(`home write escaped chosen home: ${file}`);
}

evidence.home_files = homeFiles;
evidence.export_files = exportFiles;
evidence.non_claims = [
  "unsigned local .app only; no Developer ID or notarization claim",
  "no migration, crash-recovery, privacy, scale, export-atomicity, or VoiceOver claim",
];
console.log(JSON.stringify(evidence, null, 2));
