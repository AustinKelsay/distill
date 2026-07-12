/**
 * First-run Distill UI: home/Fixture inputs and source/sync/session/health results.
 */

import { type FormEvent, useEffect, useState } from "react";
import type {
  DistillBridge,
  FixtureJourneyPhase,
  FixtureJourneyResult,
  HealthReport,
  HostError,
  RepairReport,
} from "./types";

/** Explicit UI lifecycle for the first-run Fixture journey. */
export type UiStatus = "idle" | "running" | "success" | "error";

type AppProps = {
  bridge: DistillBridge;
};

/**
 * Render the minimal first-run Fixture caller surface.
 * @param props - injected Distill bridge (real Tauri or typed fake)
 */
export function App({ bridge }: AppProps) {
  const [home, setHome] = useState("");
  const [fixtureRoot, setFixtureRoot] = useState("");
  const [status, setStatus] = useState<UiStatus>("idle");
  const [phase, setPhase] = useState<FixtureJourneyPhase | null>(null);
  const [result, setResult] = useState<FixtureJourneyResult | null>(null);
  const [standaloneHealth, setStandaloneHealth] = useState<HealthReport | null>(null);
  const [repairReport, setRepairReport] = useState<RepairReport | null>(null);
  const [confirmRepair, setConfirmRepair] = useState(false);
  const [error, setError] = useState<HostError | null>(null);

  useEffect(() => {
    return bridge.onProgress((nextPhase) => {
      setPhase(nextPhase);
    });
  }, [bridge]);

  /**
   * Submit the first-run form through the typed bridge only.
   * @param event - form submit event
   */
  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setStatus("running");
    setError(null);
    setResult(null);
    setStandaloneHealth(null);
    setRepairReport(null);
    setPhase(null);
    try {
      const next = await bridge.runFixtureJourney({ home, fixtureRoot });
      setResult(next);
      setStatus("success");
    } catch (caught) {
      setError(normalizeError(caught));
      setStatus("error");
    }
  }

  /**
   * Refresh typed health for the chosen Distill home.
   */
  async function onCheckHealth() {
    setError(null);
    setRepairReport(null);
    try {
      const health = await bridge.health(home);
      setStandaloneHealth(health);
    } catch (caught) {
      setError(normalizeError(caught));
    }
  }

  /**
   * Run explicit repair only when the confirmation checkbox is checked.
   */
  async function onRepair() {
    setError(null);
    if (!confirmRepair) {
      setError({
        code: "validation",
        message: "repair requires explicit confirmation",
      });
      return;
    }
    try {
      const report = await bridge.repair(home, true);
      setRepairReport(report);
      setStandaloneHealth(report.health_after);
    } catch (caught) {
      setError(normalizeError(caught));
    }
  }

  const health = standaloneHealth ?? result?.health ?? null;

  return (
    <main className="app">
      <header className="hero">
        <p className="brand">Distill</p>
        <h1>First-run Fixture sync</h1>
        <p className="lede">
          Provide a home directory and Fixture root. The sandboxed UI asks the host to run
          the Library journey; it has no filesystem or Node authority.
        </p>
      </header>

      <form className="form" onSubmit={onSubmit}>
        <label htmlFor="distill-home">Distill home</label>
        <input
          id="distill-home"
          name="home"
          value={home}
          onChange={(event) => setHome(event.target.value)}
          placeholder="/tmp/distill-home"
          required
        />
        <label htmlFor="fixture-root">Fixture root</label>
        <input
          id="fixture-root"
          name="fixtureRoot"
          value={fixtureRoot}
          onChange={(event) => setFixtureRoot(event.target.value)}
          placeholder="/path/to/fixture"
          required
        />
        <button type="submit" disabled={status === "running"}>
          {status === "running" ? "Running…" : "Run Fixture journey"}
        </button>
      </form>

      <section className="form" aria-label="Library health and repair">
        <button type="button" onClick={onCheckHealth} disabled={!home.trim()}>
          Check health
        </button>
        <label htmlFor="confirm-repair">
          <input
            id="confirm-repair"
            type="checkbox"
            checked={confirmRepair}
            onChange={(event) => setConfirmRepair(event.target.checked)}
          />{" "}
          Confirm destructive repair
        </label>
        <button
          type="button"
          onClick={onRepair}
          disabled={!home.trim() || !confirmRepair}
        >
          Repair library
        </button>
      </section>

      <section aria-live="polite" className="status">
        <p>
          Status: <strong data-testid="status">{status}</strong>
          {phase ? ` · phase: ${phase}` : null}
        </p>
      </section>

      {error ? (
        <section className="panel error" data-testid="error-panel">
          <h2>Error</h2>
          <p>
            <code>{error.code}</code>: {error.message}
          </p>
        </section>
      ) : null}

      {result ? (
        <div className="results">
          <section className="panel" data-testid="source-panel">
            <h2>Source</h2>
            <dl>
              <dt>Kind</dt>
              <dd>{result.source.kind}</dd>
              <dt>Name</dt>
              <dd>{result.source.display_name}</dd>
              <dt>Root</dt>
              <dd>{result.source.data_root}</dd>
              <dt>Parser</dt>
              <dd>
                {result.source.parser_id}@{result.source.parser_version}
              </dd>
            </dl>
          </section>

          <section className="panel" data-testid="sync-panel">
            <h2>Sync</h2>
            <dl>
              <dt>Accepted captures</dt>
              <dd>{result.sync.accepted_captures}</dd>
              <dt>Successful attempts</dt>
              <dd>{result.sync.successful_attempts}</dd>
              <dt>Failed attempts</dt>
              <dd>{result.sync.failed_attempts}</dd>
              <dt>Skipped duplicates</dt>
              <dd>{result.sync.skipped_duplicates}</dd>
            </dl>
          </section>

          <section className="panel" data-testid="session-panel">
            <h2>Session</h2>
            {result.session ? (
              <dl>
                <dt>Identity</dt>
                <dd>
                  {result.session.summary.source_kind}:
                  {result.session.summary.external_session_id}
                </dd>
                <dt>Title</dt>
                <dd>{result.session.summary.title ?? "(untitled)"}</dd>
                <dt>Accepted Capture count</dt>
                <dd>{result.session.summary.accepted_capture_count}</dd>
                <dt>Normalization Attempt count</dt>
                <dd>{result.session.summary.normalization_attempt_count}</dd>
                <dt>Successful projection generation</dt>
                <dd>{result.session.summary.successful_projection_generation}</dd>
                <dt>Messages</dt>
                <dd>
                  <ol>
                    {result.session.messages.map((message) => (
                      <li key={message.id}>
                        <strong>{message.role}</strong>: {message.text}
                      </li>
                    ))}
                  </ol>
                </dd>
              </dl>
            ) : (
              <p>No Session Projection was produced.</p>
            )}
          </section>
        </div>
      ) : null}

      {health ? (
        <section className="panel" data-testid="health-panel">
          <h2>Health</h2>
          <dl>
            <dt>OK</dt>
            <dd>{String(health.ok)}</dd>
            <dt>Schema</dt>
            <dd>{health.schema_status}</dd>
            <dt>Content</dt>
            <dd>{health.content_status}</dd>
            <dt>FTS</dt>
            <dd>{health.fts_status}</dd>
            <dt>Staging</dt>
            <dd>{health.staging_status}</dd>
            <dt>Orphan</dt>
            <dd>{health.orphan_status}</dd>
            <dt>Incomplete</dt>
            <dd>{health.incomplete_status}</dd>
            <dt>Operations</dt>
            <dd>{health.operations_status}</dd>
          </dl>
          {health.issues.length > 0 ? (
            <ul data-testid="health-issues">
              {health.issues.map((issue) => (
                <li key={`${issue.code}-${issue.summary}`}>
                  <code>{issue.code}</code> ({issue.severity}/{issue.category}):{" "}
                  {issue.summary}
                </li>
              ))}
            </ul>
          ) : null}
        </section>
      ) : null}

      {repairReport ? (
        <section className="panel" data-testid="repair-panel">
          <h2>Repair</h2>
          <ul>
            {repairReport.actions.map((action) => (
              <li key={action.name}>
                {action.name}: {action.count}
              </li>
            ))}
          </ul>
        </section>
      ) : null}
    </main>
  );
}

/**
 * Normalize unknown bridge failures into a typed HostError.
 * @param caught - thrown value from the bridge
 */
function normalizeError(caught: unknown): HostError {
  if (
    typeof caught === "object" &&
    caught !== null &&
    "code" in caught &&
    "message" in caught &&
    typeof (caught as HostError).code === "string" &&
    typeof (caught as HostError).message === "string"
  ) {
    return caught as HostError;
  }
  return {
    code: "unknown",
    message: caught instanceof Error ? caught.message : String(caught),
  };
}
