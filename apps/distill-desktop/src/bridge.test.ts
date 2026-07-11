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
});
