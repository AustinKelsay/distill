/**
 * Hermetic multi-Source fixture seeding for packaged macOS/Linux smokes.
 *
 * Seeds temporary file-backed Codex, Claude Code, OpenCode, and Droid roots plus
 * a missing sibling path used only for Detect Sources isolation/redaction. OpenCode
 * uses a checked-in-style no-op stub under `{opencodeRoot}/bin/opencode` so the
 * packaged host never needs a host-installed provider.
 */

import { promises as fs } from "node:fs";
import path from "node:path";

/** Secret fragment used only inside a missing Detect sibling path. */
export const DETECT_SIBLING_SECRET = "secret-token-packaged-48";

/**
 * @typedef {object} HermeticMultisourceRoots
 * @property {string} fixtureRoot
 * @property {string} codexRoot
 * @property {string} claudeRoot
 * @property {string} opencodeRoot
 * @property {string} droidRoot
 * @property {string} missingSiblingRoot
 * @property {string} fixtureSessionTitle
 * @property {string} fixtureExternalSessionId
 * @property {string} opencodeBin
 */

/**
 * Build the absolute root map under a temporary smoke workspace.
 * @param {string} base - temporary parent directory owned by the smoke
 * @param {{ fixtureSessionTitle: string, fixtureExternalSessionId: string }} labels
 * @returns {HermeticMultisourceRoots}
 */
export function planHermeticMultisourceRoots(base, labels) {
  const opencodeRoot = path.join(base, "opencode-home");
  return {
    fixtureRoot: path.join(base, "fixture"),
    codexRoot: path.join(base, "codex-home"),
    claudeRoot: path.join(base, "claude-home"),
    opencodeRoot,
    droidRoot: path.join(base, "factory-sessions"),
    missingSiblingRoot: path.join(base, `${DETECT_SIBLING_SECRET}-missing-root`),
    fixtureSessionTitle: labels.fixtureSessionTitle,
    fixtureExternalSessionId: labels.fixtureExternalSessionId,
    opencodeBin: path.join(opencodeRoot, "bin", "opencode"),
  };
}

/**
 * Write the retained Fixture manifest + capture used by packaged export assertions.
 * @param {HermeticMultisourceRoots} roots
 */
async function writeFixtureRoot(roots) {
  const captures = path.join(roots.fixtureRoot, "captures");
  await fs.mkdir(captures, { recursive: true });
  await fs.writeFile(
    path.join(roots.fixtureRoot, "distill.fixture.json"),
    JSON.stringify({
      version: 1,
      captures: [
        {
          id: "packaged-smoke",
          kind: "file",
          relative_path: "captures/packaged-smoke.jsonl",
          external_session_id: roots.fixtureExternalSessionId,
          title: roots.fixtureSessionTitle,
        },
      ],
    }),
  );
  await fs.writeFile(
    path.join(captures, "packaged-smoke.jsonl"),
    [
      JSON.stringify({
        record_type: "session_meta",
        title: roots.fixtureSessionTitle,
        summary: "packaged hermetic smoke",
      }),
      JSON.stringify({ record_type: "message", role: "user", text: "smoke search" }),
      JSON.stringify({
        record_type: "message",
        role: "assistant",
        text: "smoke response",
      }),
    ].join("\n") + "\n",
  );
}

/**
 * Write a minimal Codex live session under a synthetic Codex home.
 * @param {string} root
 */
async function writeCodexRoot(root) {
  const relative =
    "sessions/2026/07/12/rollout-2026-07-12T12-00-00-abc12345-1111-2222-3333-abcdefabcdef.jsonl";
  const sessionPath = path.join(root, relative);
  await fs.mkdir(path.dirname(sessionPath), { recursive: true });
  await fs.writeFile(
    sessionPath,
    [
      JSON.stringify({
        timestamp: "2026-07-12T12:00:00.000Z",
        type: "session_meta",
        payload: {
          id: "abc12345-1111-2222-3333-abcdefabcdef",
          timestamp: "2026-07-12T12:00:00.000Z",
          cwd: "/tmp/packaged-codex",
        },
      }),
      JSON.stringify({
        timestamp: "2026-07-12T12:00:01.000Z",
        type: "response_item",
        payload: {
          type: "message",
          role: "user",
          content: [{ type: "input_text", text: "hello packaged codex" }],
        },
      }),
      JSON.stringify({
        timestamp: "2026-07-12T12:00:02.000Z",
        type: "response_item",
        payload: {
          type: "message",
          role: "assistant",
          content: [{ type: "output_text", text: "codex packaged reply" }],
        },
      }),
    ].join("\n") + "\n",
  );
  await fs.writeFile(
    path.join(root, "session_index.jsonl"),
    '{"id":"abc12345-1111-2222-3333-abcdefabcdef","thread_name":"Packaged Codex","updated_at":"2026-07-12T12:01:00.000Z"}\n',
  );
}

/**
 * Write a minimal Claude Code project session under a synthetic Claude home.
 * @param {string} root
 */
async function writeClaudeRoot(root) {
  const sessionId = "123e4567-e89b-12d3-a456-426614174000";
  const sessionPath = path.join(root, "projects", "packaged-demo", `${sessionId}.jsonl`);
  await fs.mkdir(path.dirname(sessionPath), { recursive: true });
  await fs.writeFile(
    sessionPath,
    [
      JSON.stringify({
        type: "user",
        uuid: "u1",
        sessionId,
        timestamp: "2026-07-12T12:10:00.000Z",
        cwd: "/tmp/packaged-claude",
        message: {
          role: "user",
          content: [{ type: "text", text: "hello packaged claude" }],
        },
      }),
      JSON.stringify({
        type: "assistant",
        uuid: "a1",
        parentUuid: "u1",
        sessionId,
        timestamp: "2026-07-12T12:10:01.000Z",
        cwd: "/tmp/packaged-claude",
        message: {
          role: "assistant",
          content: [{ type: "text", text: "claude packaged reply" }],
        },
      }),
    ].join("\n") + "\n",
  );
  await fs.writeFile(
    path.join(root, "history.jsonl"),
    `{"display":"Packaged Claude","sessionId":"${sessionId}"}\n`,
  );
}

/**
 * Write a minimal Droid session under a synthetic Factory sessions root.
 * @param {string} root
 */
async function writeDroidRoot(root) {
  const sessionId = "123e4567-e89b-12d3-a456-426614174000";
  const sessionPath = path.join(root, "ws-packaged", `${sessionId}.jsonl`);
  await fs.mkdir(path.dirname(sessionPath), { recursive: true });
  await fs.writeFile(
    sessionPath,
    [
      JSON.stringify({
        type: "session_start",
        id: sessionId,
        title: "Packaged Droid",
        owner: "packaged",
        cwd: "/tmp/packaged-droid",
      }),
      JSON.stringify({
        type: "message",
        id: "u1",
        timestamp: "2026-07-12T12:20:00.000Z",
        message: {
          role: "user",
          content: [{ type: "text", text: "hello packaged droid" }],
        },
      }),
      JSON.stringify({
        type: "message",
        id: "a1",
        timestamp: "2026-07-12T12:20:01.000Z",
        message: {
          role: "assistant",
          content: [{ type: "text", text: "droid packaged reply" }],
        },
      }),
    ].join("\n") + "\n",
  );
}

/**
 * Install a hermetic fake `opencode` under `{root}/bin` with one virtual session.
 * @param {string} root
 */
async function installFakeOpencode(root) {
  const binDir = path.join(root, "bin");
  const exportDir = path.join(root, "exports");
  await fs.mkdir(binDir, { recursive: true });
  await fs.mkdir(exportDir, { recursive: true });
  await fs.writeFile(
    path.join(root, "sessions.json"),
    '[{"id":"ses_packaged","title":"Packaged OpenCode","directory":"/tmp/packaged-opencode","version":"1.0.0","time_created":1774543194067,"time_updated":1774543475213,"time_archived":null}]\n',
  );
  const exportBody = [
    "Exporting session: ses_packaged",
    JSON.stringify({
      info: {
        id: "ses_packaged",
        slug: "packaged-wizard",
        projectID: "global",
        directory: "/tmp/packaged-opencode",
        title: "Packaged OpenCode",
        version: "1.0.0",
        time: { created: 1774543194067, updated: 1774543475213 },
      },
      messages: [
        {
          info: { id: "msg_user", role: "user", time: { created: 1774543194080 } },
          parts: [{ id: "part_user", type: "text", text: "hello packaged opencode" }],
        },
        {
          info: {
            id: "msg_assistant",
            role: "assistant",
            parentID: "msg_user",
            time: { created: 1774543194090 },
          },
          parts: [{ id: "part_text", type: "text", text: "opencode packaged reply" }],
        },
      ],
    }),
  ].join("\n");
  await fs.writeFile(path.join(exportDir, "ses_packaged.json"), `${exportBody}\n`);
  const script = path.join(binDir, "opencode");
  const scriptBody = `#!/bin/sh
set -eu
ROOT="${root}"
case "\${1:-}" in
  db)
    if [ "\${2:-}" = "path" ]; then
      printf '%s\\n' "$ROOT/opencode.db"
      exit 0
    fi
    cat "$ROOT/sessions.json"
    exit 0
    ;;
  export)
    cat "$ROOT/exports/\${2:-}.json"
    exit 0
    ;;
esac
printf 'unsupported fake opencode command\\n' >&2
exit 1
`;
  await fs.writeFile(script, scriptBody, { mode: 0o755 });
}

/**
 * Seed all hermetic multi-Source roots used by the packaged smoke journey.
 * Does not create the missing sibling directory — Detect must observe it as absent.
 *
 * @param {string} base - temporary parent directory owned by the smoke
 * @param {{ fixtureSessionTitle: string, fixtureExternalSessionId: string }} labels
 * @returns {Promise<HermeticMultisourceRoots>}
 */
export async function seedHermeticMultisourceRoots(base, labels) {
  const roots = planHermeticMultisourceRoots(base, labels);
  await fs.mkdir(roots.fixtureRoot, { recursive: true });
  await fs.mkdir(roots.codexRoot, { recursive: true });
  await fs.mkdir(roots.claudeRoot, { recursive: true });
  await fs.mkdir(roots.opencodeRoot, { recursive: true });
  await fs.mkdir(roots.droidRoot, { recursive: true });
  await writeFixtureRoot(roots);
  await writeCodexRoot(roots.codexRoot);
  await writeClaudeRoot(roots.claudeRoot);
  await installFakeOpencode(roots.opencodeRoot);
  await writeDroidRoot(roots.droidRoot);
  return roots;
}

/**
 * Provider and Fixture roots that must remain byte-stable across the packaged smoke.
 * @param {HermeticMultisourceRoots} roots
 * @returns {string[]}
 */
export function hermeticSourceRoots(roots) {
  return [
    roots.fixtureRoot,
    roots.codexRoot,
    roots.claudeRoot,
    roots.opencodeRoot,
    roots.droidRoot,
  ];
}
