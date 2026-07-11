/**
 * Production Tauri bridge. Uses only the explicit invoke/event APIs.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  DistillBridge,
  FixtureJourneyInput,
  FixtureJourneyPhase,
  FixtureJourneyResult,
} from "./types";

const PROGRESS_EVENT = "fixture-journey-progress";

/**
 * Create the real Tauri Distill bridge.
 */
export function createTauriBridge(): DistillBridge {
  return {
    async runFixtureJourney(input: FixtureJourneyInput): Promise<FixtureJourneyResult> {
      return invoke<FixtureJourneyResult>("run_fixture_journey_command", {
        home: input.home,
        fixtureRoot: input.fixtureRoot,
      });
    },
    onProgress(listener: (phase: FixtureJourneyPhase) => void) {
      let unlisten: (() => void) | undefined;
      let disposed = false;
      void listen<FixtureJourneyPhase>(PROGRESS_EVENT, (event) => {
        listener(event.payload);
      }).then((fn) => {
        if (disposed) {
          fn();
        } else {
          unlisten = fn;
        }
      });
      return () => {
        disposed = true;
        unlisten?.();
      };
    },
  };
}
