/**
 * Renderer seam: React first-run UI against one typed Distill bridge fake.
 */

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { App } from "./App";
import type {
  DistillBridge,
  FixtureJourneyPhase,
  FixtureJourneyResult,
  HostError,
} from "./types";

/**
 * Build a successful Fixture journey fixture for the typed bridge fake.
 */
function sampleResult(): FixtureJourneyResult {
  return {
    source: {
      kind: "fixture",
      display_name: "Fixture",
      data_root: "/tmp/fixture",
      parser_id: "fixture",
      parser_version: "1.0.0",
    },
    sync: {
      accepted_captures: 1,
      skipped_duplicates: 0,
      successful_attempts: 1,
      failed_attempts: 0,
      capture_ids: [1],
      session_identities: [
        {
          source_kind: "fixture",
          external_session_id: "fixture-session-ui",
        },
      ],
    },
    session: {
      summary: {
        id: 1,
        source_kind: "fixture",
        external_session_id: "fixture-session-ui",
        title: "UI Fixture",
        accepted_capture_count: 1,
        normalization_attempt_count: 1,
        successful_projection_generation: 1,
      },
      messages: [
        {
          id: 1,
          ordinal: 0,
          role: "user",
          message_kind: "text",
          text: "Hello from UI fixture",
        },
      ],
      artifacts: [],
      metadata_json: "{}",
    },
    health: {
      ok: true,
      schema_status: "ok",
      content_status: "ok",
      fts_status: "ok",
      issues: [],
    },
  };
}

/**
 * Create one typed bridge fake with controllable success/error behavior.
 */
function createFakeBridge(options?: {
  result?: FixtureJourneyResult;
  error?: HostError;
  phases?: FixtureJourneyPhase[];
}): DistillBridge {
  const listeners = new Set<(phase: FixtureJourneyPhase) => void>();
  return {
    async runFixtureJourney() {
      for (const phase of options?.phases ?? [
        "detecting_source",
        "syncing_captures",
        "loading_session",
        "checking_health",
      ]) {
        listeners.forEach((listener) => listener(phase));
      }
      if (options?.error) throw options.error;
      return options?.result ?? sampleResult();
    },
    onProgress(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}

describe("first-run Fixture UI", () => {
  it("renders idle input and success source/sync/session/health panels", async () => {
    const user = userEvent.setup();
    const bridge = createFakeBridge();
    render(<App bridge={bridge} />);

    expect(screen.getByTestId("status")).toHaveTextContent("idle");
    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.type(
      screen.getByRole("textbox", { name: "Fixture root" }),
      "/tmp/fixture",
    );
    await user.click(screen.getByRole("button", { name: /run fixture journey/i }));

    await waitFor(() => {
      expect(screen.getByTestId("status")).toHaveTextContent("success");
    });
    expect(screen.getByTestId("source-panel")).toHaveTextContent("fixture");
    expect(screen.getByTestId("sync-panel")).toHaveTextContent("Accepted captures");
    expect(screen.getByTestId("sync-panel")).toHaveTextContent("1");
    expect(screen.getByTestId("session-panel")).toHaveTextContent(
      "fixture:fixture-session-ui",
    );
    expect(screen.getByTestId("session-panel")).toHaveTextContent(
      "Hello from UI fixture",
    );
    expect(screen.getByTestId("health-panel")).toHaveTextContent("true");
  });

  it("renders typed error state from the bridge", async () => {
    const user = userEvent.setup();
    const bridge = createFakeBridge({
      error: { code: "validation", message: "fixture root must not be empty" },
    });
    render(<App bridge={bridge} />);

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.type(
      screen.getByRole("textbox", { name: "Fixture root" }),
      "/tmp/fixture",
    );
    await user.click(screen.getByRole("button", { name: /run fixture journey/i }));

    await waitFor(() => {
      expect(screen.getByTestId("status")).toHaveTextContent("error");
    });
    expect(screen.getByTestId("error-panel")).toHaveTextContent("validation");
    expect(screen.getByTestId("error-panel")).toHaveTextContent(
      "fixture root must not be empty",
    );
  });

  it("shows running status while the bridge journey is in flight", async () => {
    const user = userEvent.setup();
    let resolveJourney!: (value: FixtureJourneyResult) => void;
    const pending = new Promise<FixtureJourneyResult>((resolve) => {
      resolveJourney = resolve;
    });
    const bridge: DistillBridge = {
      runFixtureJourney: () => pending,
      onProgress: () => () => undefined,
    };
    render(<App bridge={bridge} />);

    await user.type(screen.getByRole("textbox", { name: "Distill home" }), "/tmp/home");
    await user.type(
      screen.getByRole("textbox", { name: "Fixture root" }),
      "/tmp/fixture",
    );
    await user.click(screen.getByRole("button", { name: /run fixture journey/i }));

    expect(screen.getByTestId("status")).toHaveTextContent("running");
    resolveJourney(sampleResult());
    await waitFor(() => {
      expect(screen.getByTestId("status")).toHaveTextContent("success");
    });
  });
});
