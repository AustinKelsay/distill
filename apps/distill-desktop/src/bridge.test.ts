/** Production bridge contract: exact Tauri command arguments and listener cleanup. */

import { beforeEach, describe, expect, it, vi } from "vitest";
import { createTauriBridge } from "./bridge";

const tauri = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: tauri.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: tauri.listen }));

describe("Tauri bridge", () => {
  beforeEach(() => {
    tauri.invoke.mockReset();
    tauri.listen.mockReset();
  });

  it("translates renderer input to the Tauri command camel-case arguments", async () => {
    tauri.invoke.mockResolvedValue({ health: { ok: true } });
    const bridge = createTauriBridge();

    await bridge.runFixtureJourney({
      home: "/tmp/distill-home",
      fixtureRoot: "/tmp/fixture-root",
    });

    expect(tauri.invoke).toHaveBeenCalledWith("run_fixture_journey_command", {
      home: "/tmp/distill-home",
      fixtureRoot: "/tmp/fixture-root",
    });
  });

  it("invokes health and repair commands with confirm flag", async () => {
    tauri.invoke.mockResolvedValue({ ok: true });
    const bridge = createTauriBridge();

    await bridge.health("/tmp/distill-home");
    expect(tauri.invoke).toHaveBeenCalledWith("health_command", {
      home: "/tmp/distill-home",
    });

    await bridge.repair("/tmp/distill-home", true);
    expect(tauri.invoke).toHaveBeenCalledWith("repair_command", {
      home: "/tmp/distill-home",
      confirm: true,
    });
  });

  it("invokes sync and source preference commands", async () => {
    tauri.invoke.mockResolvedValue({ ok: true });
    const bridge = createTauriBridge();

    await bridge.setSourcePreference("/tmp/home", "fixture", true, "/tmp/fixture");
    expect(tauri.invoke).toHaveBeenCalledWith("set_source_preference_command", {
      home: "/tmp/home",
      kind: "fixture",
      enabled: true,
      configuredRoot: "/tmp/fixture",
    });

    await bridge.startSync("/tmp/home", ["fixture"]);
    expect(tauri.invoke).toHaveBeenCalledWith("sync_start_command", {
      home: "/tmp/home",
      sourceKinds: ["fixture"],
    });

    await bridge.cancelSync("/tmp/home", 7);
    expect(tauri.invoke).toHaveBeenCalledWith("sync_cancel_command", {
      home: "/tmp/home",
      syncRunId: 7,
    });

    await bridge.listSources("/tmp/home");
    expect(tauri.invoke).toHaveBeenCalledWith("list_sources_command", {
      home: "/tmp/home",
    });

    await bridge.syncStatus("/tmp/home", 7);
    expect(tauri.invoke).toHaveBeenCalledWith("sync_status_command", {
      home: "/tmp/home",
      syncRunId: 7,
    });

    await bridge.listSessions("/tmp/home", {
      query: "café",
      lane: "all",
      limit: 20,
      cursor: null,
    });
    expect(tauri.invoke).toHaveBeenCalledWith("sessions_list_command", {
      home: "/tmp/home",
      request: { query: "café", lane: "all", limit: 20, cursor: null },
    });

    await bridge.sessionDetail("/tmp/home", {
      source_kind: "fixture",
      external_session_id: "session-1",
      message_limit: 20,
      artifact_limit: 20,
      message_cursor: null,
      artifact_cursor: null,
    });
    expect(tauri.invoke).toHaveBeenCalledWith("session_detail_command", {
      home: "/tmp/home",
      request: {
        source_kind: "fixture",
        external_session_id: "session-1",
        message_limit: 20,
        artifact_limit: 20,
        message_cursor: null,
        artifact_cursor: null,
      },
    });

    await bridge.addSessionTag("/tmp/home", {
      source_kind: "fixture",
      external_session_id: "session-1",
      name: "research",
    });
    expect(tauri.invoke).toHaveBeenCalledWith("add_session_tag_command", {
      home: "/tmp/home",
      request: {
        source_kind: "fixture",
        external_session_id: "session-1",
        name: "research",
      },
    });

    await bridge.removeSessionTag("/tmp/home", {
      source_kind: "fixture",
      external_session_id: "session-1",
      name: "research",
    });
    expect(tauri.invoke).toHaveBeenCalledWith("remove_session_tag_command", {
      home: "/tmp/home",
      request: {
        source_kind: "fixture",
        external_session_id: "session-1",
        name: "research",
      },
    });

    await bridge.toggleSessionLabel("/tmp/home", {
      source_kind: "fixture",
      external_session_id: "session-1",
      name: "train",
    });
    expect(tauri.invoke).toHaveBeenCalledWith("toggle_session_label_command", {
      home: "/tmp/home",
      request: {
        source_kind: "fixture",
        external_session_id: "session-1",
        name: "train",
      },
    });

    await bridge.previewExport("/tmp/home", "train");
    expect(tauri.invoke).toHaveBeenCalledWith("export_preview_command", {
      home: "/tmp/home",
      dataset: "train",
    });

    await bridge.publishExport("/tmp/home", "holdout");
    expect(tauri.invoke).toHaveBeenCalledWith("export_publish_command", {
      home: "/tmp/home",
      dataset: "holdout",
    });

    await bridge.cancelExport("/tmp/home", "holdout");
    expect(tauri.invoke).toHaveBeenCalledWith("export_cancel_command", {
      home: "/tmp/home",
      dataset: "holdout",
    });
  });

  it("unsubscribes when cleanup happens before async listener registration finishes", async () => {
    let finishRegistration!: (unlisten: () => void) => void;
    tauri.listen.mockReturnValue(
      new Promise<() => void>((resolve) => {
        finishRegistration = resolve;
      }),
    );
    const unlisten = vi.fn();
    const cleanup = createTauriBridge().onProgress(() => undefined);

    cleanup();
    finishRegistration(unlisten);
    await Promise.resolve();

    expect(unlisten).toHaveBeenCalledOnce();
  });

  it("uses only invoke/listen and never ambient filesystem/process/sql/shell APIs", async () => {
    const { readFileSync } = await import("node:fs");
    const { dirname, join } = await import("node:path");
    const { fileURLToPath } = await import("node:url");
    const bridgeSource = readFileSync(
      join(dirname(fileURLToPath(import.meta.url)), "bridge.ts"),
      "utf8",
    );
    expect(bridgeSource).toContain('from "@tauri-apps/api/core"');
    expect(bridgeSource).toContain('from "@tauri-apps/api/event"');
    for (const denied of [
      "@tauri-apps/plugin-fs",
      "@tauri-apps/plugin-shell",
      "@tauri-apps/plugin-sql",
      "@tauri-apps/plugin-process",
      "@tauri-apps/plugin-dialog",
      "node:child_process",
      "dangerouslySetInnerHTML",
    ]) {
      expect(bridgeSource).not.toContain(denied);
    }
    expect(bridgeSource).toContain("no application encryption");
    expect(bridgeSource).toContain("secure-forget");

    tauri.invoke.mockResolvedValue({ items: [] });
    const bridge = createTauriBridge();
    await bridge.listActivity("/tmp/home", { limit: 10, cursor: null });
    expect(tauri.invoke).toHaveBeenCalledWith("activity_list_command", {
      home: "/tmp/home",
      request: { limit: 10, cursor: null },
    });
    expect(tauri.listen).not.toHaveBeenCalled();
  });
});
