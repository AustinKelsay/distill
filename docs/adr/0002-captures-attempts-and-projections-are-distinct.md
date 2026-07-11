# Separate Captures, Normalization Attempts, and Session Projections

A Capture is immutable checksum-verified source content owned by Distill. Parser execution is recorded as a versioned Normalization Attempt with immutable Capture Facts, while the current Session Projection points to the latest successful attempt and is atomically replaced. This makes parser upgrades and retries possible without mutating evidence or destroying the last good conversation view.

## Considered Options

- Mutating a capture status from captured to normalized or failed was simpler, but made identical failed bytes terminal and blurred immutable evidence with processing history.
- Keeping only projections was smaller, but lost replay, provenance, and recovery.

## Consequences

- The same Capture may have multiple Normalization Attempts across parser versions.
- A failed attempt never changes the current Session Projection.
- Capture counts and attempt counts are separate and named explicitly.
- Projection messages and artifacts are generation-owned derived state, not history.
