#!/usr/bin/env node

/**
 * Inspect and smoke-test the packaged macOS `.app`.
 *
 * The static checks are always automated. The short UI journey uses macOS
 * Accessibility via System Events; set DISTILL_MACOS_ALLOW_MANUAL=1 only when
 * the runner has no Accessibility permission and a human will complete the
 * same checklist. That mode is explicit and never claims a packaged journey.
 * The journey seeds hermetic multi-Source roots, drives Detect Sources + Start
 * Sync Run, then retains the Fixture search/detail/curation/export/restart path.
 * Attempt-history/renormalize is not automated by this System Events script;
 * PKG-006 is an explicit manual AX checklist when the packaged window is exposed.
 * Hermetic legacy Electron-home import (PKG-007) is likewise manual-required on
 * Darwin: seed a temporary host/CLI-shaped legacy home, drive Import legacy home,
 * assert ok/captures/sessions/reused=false, search the migrated session, and
 * compare before/after source-home hashes including distill.db-wal/shm.
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

async function runPackagedUiJourney(home, roots) {
  const script = `
tell application "Distill" to activate
tell application "System Events"
  tell process "distill-desktop"
    set tabKeyCode to 48
    set escapeKeyCode to 53
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
          set the clipboard to ${JSON.stringify(roots.fixtureRoot)}
          click node
          keystroke "a" using {command down}
          keystroke "v" using {command down}
          set fixtureFound to true
          exit repeat
        end if
      end try
    end repeat
    if homeFound is false or fixtureFound is false then error "packaged renderer text fields are not accessible"
    -- Packaged repair-dialog focus containment (AX focused/AXFocused only; not VoiceOver).
    delay 0.5
    set nodes to entire contents of window 1
    set repairFound to false
    repeat with node in nodes
      try
        if role of node is "AXButton" and name of node is "Repair library" then
          click node
          set repairFound to true
          exit repeat
        end if
      end try
    end repeat
    if repairFound is false then error "Repair library button is not accessible"
    set dialogFocusReady to false
    repeat 40 times
      set dialogNamed to false
      set focusInside to false
      set nodes to entire contents of window 1
      repeat with node in nodes
        try
          if name of node is "Confirm destructive repair" then
            set dialogNamed to true
            try
              if focused of node is true then set focusInside to true
            end try
            try
              if (value of attribute "AXFocused" of node) is true then set focusInside to true
            end try
            try
              set dialogKids to entire contents of node
              repeat with kid in dialogKids
                try
                  if focused of kid is true then set focusInside to true
                end try
                try
                  if (value of attribute "AXFocused" of kid) is true then set focusInside to true
                end try
                if focusInside then exit repeat
              end repeat
            end try
          end if
        end try
      end repeat
      if dialogNamed and focusInside then
        set dialogFocusReady to true
        exit repeat
      end if
      delay 0.25
    end repeat
    if dialogFocusReady is false then error "dialog-focus: expected focused accessible inside Confirm destructive repair"
    key code tabKeyCode
    delay 0.25
    set dialogNamed to false
    set focusInside to false
    set nodes to entire contents of window 1
    repeat with node in nodes
      try
        if name of node is "Confirm destructive repair" then
          set dialogNamed to true
          try
            if focused of node is true then set focusInside to true
          end try
          try
            if (value of attribute "AXFocused" of node) is true then set focusInside to true
          end try
          try
            set dialogKids to entire contents of node
            repeat with kid in dialogKids
              try
                if focused of kid is true then set focusInside to true
              end try
              try
                if (value of attribute "AXFocused" of kid) is true then set focusInside to true
              end try
              if focusInside then exit repeat
            end repeat
          end try
        end if
      end try
    end repeat
    if dialogNamed is false or focusInside is false then error "dialog-focus: Tab moved focus outside Confirm destructive repair"
    key code escapeKeyCode
    set escapeSettled to false
    repeat 40 times
      set dialogPresent to false
      set dialogActionPresent to false
      set dialogHasFocus to false
      set repairFocused to false
      set nodes to entire contents of window 1
      repeat with node in nodes
        try
          if role of node is "AXButton" and (name of node is "Cancel repair" or name of node is "Confirm repair") then
            set dialogActionPresent to true
            try
              if focused of node is true then set dialogHasFocus to true
            end try
            try
              if (value of attribute "AXFocused" of node) is true then set dialogHasFocus to true
            end try
          end if
        end try
        try
          if name of node is "Confirm destructive repair" then
            set dialogPresent to true
            try
              if focused of node is true then set dialogHasFocus to true
            end try
            try
              if (value of attribute "AXFocused" of node) is true then set dialogHasFocus to true
            end try
            try
              set dialogKids to entire contents of node
              repeat with kid in dialogKids
                try
                  if focused of kid is true then set dialogHasFocus to true
                end try
                try
                  if (value of attribute "AXFocused" of kid) is true then set dialogHasFocus to true
                end try
                if dialogHasFocus then exit repeat
              end repeat
            end try
          end if
        end try
        try
          if role of node is "AXButton" and name of node is "Repair library" then
            try
              if focused of node is true then set repairFocused to true
            end try
            try
              if (value of attribute "AXFocused" of node) is true then set repairFocused to true
            end try
          end if
        end try
      end repeat
      -- Closed dialog: actions gone, no dialog focus containment, trigger focused again.
      if dialogPresent is false and dialogHasFocus is false and dialogActionPresent is false and repairFocused then
        set escapeSettled to true
        exit repeat
      end if
      delay 0.25
    end repeat
    if escapeSettled is false then error "dialog-focus: expected Confirm destructive repair closed with Repair library focused"
    -- Hermetic multi-Source Detect Sources (sibling failure) then Start Sync Run.
    set nodes to entire contents of window 1
    repeat with node in nodes
      try
        if (role of node is "AXCheckBox" or role of node is "AXButton") and name of node is "Enable codex" then
          click node
          exit repeat
        end if
      end try
    end repeat
    set nodes to entire contents of window 1
    repeat with node in nodes
      try
        if role of node is "AXTextField" and name of node is "codex source root" then
          set the clipboard to ${JSON.stringify(roots.missingSiblingRoot)}
          click node
          keystroke "a" using {command down}
          keystroke "v" using {command down}
          exit repeat
        end if
      end try
    end repeat
    set nodes to entire contents of window 1
    repeat with node in nodes
      try
        if (role of node is "AXCheckBox" or role of node is "AXButton") and name of node is "Enable claude_code" then
          click node
          exit repeat
        end if
      end try
    end repeat
    set nodes to entire contents of window 1
    repeat with node in nodes
      try
        if role of node is "AXTextField" and name of node is "claude_code source root" then
          set the clipboard to ${JSON.stringify(roots.claudeRoot)}
          click node
          keystroke "a" using {command down}
          keystroke "v" using {command down}
          exit repeat
        end if
      end try
    end repeat
    set nodes to entire contents of window 1
    repeat with node in nodes
      try
        if (role of node is "AXCheckBox" or role of node is "AXButton") and name of node is "Enable opencode" then
          click node
          exit repeat
        end if
      end try
    end repeat
    set nodes to entire contents of window 1
    repeat with node in nodes
      try
        if role of node is "AXTextField" and name of node is "opencode source root" then
          set the clipboard to ${JSON.stringify(roots.opencodeRoot)}
          click node
          keystroke "a" using {command down}
          keystroke "v" using {command down}
          exit repeat
        end if
      end try
    end repeat
    set nodes to entire contents of window 1
    repeat with node in nodes
      try
        if (role of node is "AXCheckBox" or role of node is "AXButton") and name of node is "Enable droid" then
          click node
          exit repeat
        end if
      end try
    end repeat
    set nodes to entire contents of window 1
    repeat with node in nodes
      try
        if role of node is "AXTextField" and name of node is "droid source root" then
          set the clipboard to ${JSON.stringify(roots.droidRoot)}
          click node
          keystroke "a" using {command down}
          keystroke "v" using {command down}
          exit repeat
        end if
      end try
    end repeat
    set nodes to entire contents of window 1
    set detectFound to false
    repeat with node in nodes
      try
        if role of node is "AXButton" and name of node is "Detect Sources" then
          click node
          set detectFound to true
          exit repeat
        end if
      end try
    end repeat
    if detectFound is false then error "Detect Sources button is not accessible"
    set detectWarned to false
    repeat 40 times
      set sawWarning to false
      set sawUnhealthy to false
      set nodes to entire contents of window 1
      repeat with node in nodes
        try
          set nodeName to name of node
          if nodeName contains "Status: warning" then set sawWarning to true
          if nodeName contains "codex: unhealthy" then set sawUnhealthy to true
        end try
      end repeat
      if sawWarning and sawUnhealthy then
        set detectWarned to true
        exit repeat
      end if
      delay 0.25
    end repeat
    if detectWarned is false then error "Detect Sources did not surface sibling-failure warning status"
    repeat with expectedStatus in {"fixture: ok", "claude_code: unavailable", "opencode: ok", "droid: ok"}
      set statusFound to false
      repeat 40 times
        set nodes to entire contents of window 1
        repeat with node in nodes
          try
            if name of node contains expectedStatus then
              set statusFound to true
              exit repeat
            end if
          end try
        end repeat
        if statusFound then exit repeat
        delay 0.25
      end repeat
      if statusFound is false then error "Detect Sources did not preserve sibling status " & expectedStatus
    end repeat
    set nodes to entire contents of window 1
    repeat with node in nodes
      try
        if role of node is "AXTextField" and name of node is "codex source root" then
          set the clipboard to ${JSON.stringify(roots.codexRoot)}
          click node
          keystroke "a" using {command down}
          keystroke "v" using {command down}
          exit repeat
        end if
      end try
    end repeat
    delay 0.25
    set nodes to entire contents of window 1
    repeat with node in nodes
      try
        if name of node contains ${JSON.stringify(DETECT_SIBLING_SECRET)} then error "detect diagnostics leaked sibling secret token"
      end try
    end repeat
    set nodes to entire contents of window 1
    set syncFound to false
    repeat with node in nodes
      try
        if role of node is "AXButton" and name of node is "Start Sync Run" then
          click node
          set syncFound to true
          exit repeat
        end if
      end try
    end repeat
    if syncFound is false then error "Start Sync Run button is not accessible"
    set syncReady to false
    repeat 80 times
      set nodes to entire contents of window 1
      repeat with node in nodes
        try
          if name of node contains "Status: success" then
            set syncReady to true
            exit repeat
          end if
        end try
      end repeat
      if syncReady then exit repeat
      delay 0.25
    end repeat
    if syncReady is false then error "Start Sync Run did not reach success status"
    repeat with expectedKind in {"fixture", "codex", "claude_code", "opencode", "droid"}
      set sourceReady to false
      repeat 40 times
        set nodes to entire contents of window 1
        repeat with node in nodes
          try
            if name of node contains (expectedKind & ": completed") then
              set sourceReady to true
              exit repeat
            end if
          end try
        end repeat
        if sourceReady then exit repeat
        delay 0.25
      end repeat
      if sourceReady is false then error "Start Sync Run did not complete source " & expectedKind
    end repeat
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
        if (role of node is "AXCheckBox" or role of node is "AXButton") and name of node contains ${JSON.stringify(roots.fixtureSessionTitle)} then
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
        if (role of node is "AXCheckBox" or role of node is "AXButton") and name of node contains ${JSON.stringify(roots.fixtureSessionTitle)} then
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
const sessionTitle = "macOS Package Smoke";
const roots = await seedHermeticMultisourceRoots(base, {
  fixtureSessionTitle: sessionTitle,
  fixtureExternalSessionId: "macos-package-smoke",
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
  hermetic_multisource: "pending",
  detect_sibling_isolation: "pending",
  home,
  fixture_root: roots.fixtureRoot,
  hermetic_roots: {
    codex: roots.codexRoot,
    claude_code: roots.claudeRoot,
    opencode: roots.opencodeRoot,
    droid: roots.droidRoot,
  },
};

let uiError = null;
let restartError = null;
const appProcess = spawn("open", ["-n", appPath], { stdio: "ignore" });
try {
  try {
    await runPackagedUiJourney(home, roots);
    evidence.ui = "passed";
    evidence.hermetic_multisource = "passed";
    evidence.detect_sibling_isolation = "passed";
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
for (const root of sourceRoots) {
  const afterFiles = await collectFiles(root);
  const afterHashes = await collectFileHashes(root);
  if (
    JSON.stringify(beforeSourceSnapshots[root].files) !== JSON.stringify(afterFiles) ||
    JSON.stringify(beforeSourceSnapshots[root].hashes) !== JSON.stringify(afterHashes)
  ) {
    fail(`packaged smoke changed hermetic source files under ${path.basename(root)}`);
  }
}
const afterBase = await collectFiles(base);
const newBaseFiles = afterBase.filter((file) => !beforeBase.includes(file));
for (const file of newBaseFiles) {
  const absolute = path.join(base, file);
  const allowed = [home, ...sourceRoots].some((root) => isWithin(absolute, root));
  if (!allowed) {
    fail(`packaged write escaped chosen home/hermetic source roots: ${file}`);
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
  "hermetic temp roots only — no host-installed provider claim",
  "Attempt-history/renormalize automation is manual-required on macOS when AX exposes the packaged window; no screen-reader claim",
  "hermetic legacy Electron-home import (PKG-007) is manual-required on macOS when AX exposes the packaged window; no live-user-home or Electron-product edit claim",
  "no crash-recovery, privacy, scale, export-atomicity, or VoiceOver claim",
];
console.log(JSON.stringify(evidence, null, 2));
