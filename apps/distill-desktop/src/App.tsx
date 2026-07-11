/**
 * First-run Distill UI: home/Fixture inputs and source/sync/session/health results.
 */

import { type FormEvent, useEffect, useState } from "react";
import type {
  DistillBridge,
  FixtureJourneyPhase,
  FixtureJourneyResult,
  HostError,
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

          <section className="panel" data-testid="health-panel">
            <h2>Health</h2>
            <dl>
              <dt>OK</dt>
              <dd>{String(result.health.ok)}</dd>
              <dt>Schema</dt>
              <dd>{result.health.schema_status}</dd>
              <dt>Content</dt>
              <dd>{result.health.content_status}</dd>
              <dt>FTS</dt>
              <dd>{result.health.fts_status}</dd>
            </dl>
          </section>
        </div>
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
