/**
 * Executable seam for hermetic packaged legacy Electron-home seeding (#50).
 *
 * Proves the smoke helper creates a host/CLI-shaped temporary legacy home with
 * `distill.db`, empty blobs/exports, and WAL sidecars without touching Electron
 * product sources or a live user Distill home.
 */

import assert from "node:assert/strict";
import { promises as fs } from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import {
  planHermeticLegacyHome,
  seedHermeticLegacyHome,
} from "./packaged-hermetic-legacy-home.mjs";

describe("packaged hermetic legacy Electron home", () => {
  it("seeds a host-shaped legacy home with journal sidecar paths under a temp base", async () => {
    const base = await fs.mkdtemp(path.join(os.tmpdir(), "distill-hermetic-50-"));
    const labels = {
      sessionTitle: "Packaged Legacy Session",
      externalSessionId: "packaged-legacy-1",
      searchQuery: "packaged legacy",
    };
    const planned = planHermeticLegacyHome(base, labels);
    assert.equal(planned.legacyHome, path.join(base, "legacy-home"));
    assert.equal(planned.expectedCaptures, 1);
    assert.equal(planned.expectedSessions, 1);

    const seeded = await seedHermeticLegacyHome(base, labels);
    assert.equal(seeded.legacyHome, planned.legacyHome);

    await fs.access(path.join(seeded.legacyHome, "distill.db"));
    await fs.access(path.join(seeded.legacyHome, "distill.db-wal"));
    await fs.access(path.join(seeded.legacyHome, "distill.db-shm"));
    const blobs = await fs.stat(path.join(seeded.legacyHome, "blobs"));
    const exportsDir = await fs.stat(path.join(seeded.legacyHome, "exports"));
    assert.equal(blobs.isDirectory(), true);
    assert.equal(exportsDir.isDirectory(), true);

    // Sibling of the native smoke home only — never nested under destination.
    assert.equal(path.dirname(seeded.legacyHome), base);
  });
});
