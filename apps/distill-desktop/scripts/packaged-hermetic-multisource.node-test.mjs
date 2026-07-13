/**
 * Executable seam for hermetic packaged multi-Source fixture seeding (#48).
 *
 * Proves the smoke helper creates file-backed Codex/Claude/OpenCode/Droid roots,
 * installs a local OpenCode stub (never a host binary), and leaves the Detect
 * sibling path absent so packaged Detect can exercise isolation/redaction.
 */

import assert from "node:assert/strict";
import { promises as fs } from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import {
  DETECT_SIBLING_SECRET,
  hermeticSourceRoots,
  seedHermeticMultisourceRoots,
} from "./packaged-hermetic-multisource.mjs";

describe("packaged hermetic multi-Source fixtures", () => {
  it("seeds file-backed provider roots and a missing Detect sibling", async () => {
    const base = await fs.mkdtemp(path.join(os.tmpdir(), "distill-hermetic-48-"));
    const roots = await seedHermeticMultisourceRoots(base, {
      fixtureSessionTitle: "Hermetic Fixture",
      fixtureExternalSessionId: "hermetic-fixture",
    });

    for (const root of hermeticSourceRoots(roots)) {
      const stat = await fs.stat(root);
      assert.equal(stat.isDirectory(), true);
    }

    await assert.rejects(() => fs.access(roots.missingSiblingRoot), /ENOENT/);
    assert.match(roots.missingSiblingRoot, new RegExp(DETECT_SIBLING_SECRET));

    await fs.access(path.join(roots.fixtureRoot, "distill.fixture.json"));
    await fs.access(
      path.join(
        roots.codexRoot,
        "sessions/2026/07/12/rollout-2026-07-12T12-00-00-abc12345-1111-2222-3333-abcdefabcdef.jsonl",
      ),
    );
    await fs.access(
      path.join(
        roots.claudeRoot,
        "projects/packaged-demo/123e4567-e89b-12d3-a456-426614174000.jsonl",
      ),
    );
    await fs.access(
      path.join(
        roots.droidRoot,
        "ws-packaged/123e4567-e89b-12d3-a456-426614174000.jsonl",
      ),
    );

    const opencodeStat = await fs.stat(roots.opencodeBin);
    assert.equal(opencodeStat.isFile(), true);
    assert.equal((opencodeStat.mode & 0o111) !== 0, true);
    await fs.access(path.join(roots.opencodeRoot, "sessions.json"));
    await fs.access(path.join(roots.opencodeRoot, "exports/ses_packaged.json"));
  });
});
