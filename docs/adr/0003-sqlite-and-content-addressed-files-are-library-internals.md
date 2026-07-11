# Keep SQLite and content-addressed files inside the Library

The Library owns a versioned SQLite schema, SQLite FTS5, and a content-addressed file store under one Distill home. They are internal implementation dependencies rather than public repository interfaces: SQLite is a retained local-first constraint, while filesystem publication uses staging, checksums, atomic same-volume rename, explicit recovery states, and orphan repair.

## Considered Options

- Public storage ports would make hypothetical database swaps easier, but no second product storage adapter exists and the extra interfaces would expose transaction sequencing.
- Database-only binary storage simplified atomicity but made large captures and exports expensive to inspect, stream, and recover independently.

## Consequences

- Contract tests use real temporary SQLite databases and real temporary homes through the Library interface.
- Ordered checksummed migrations, foreign keys, `CHECK` constraints, and required fields enforce durable invariants.
- A file can be safely orphaned before database acceptance, but referenced missing or corrupt content is a health failure.
- Export publication has a durable lifecycle and restart repair because SQLite cannot atomically commit a filesystem rename.
