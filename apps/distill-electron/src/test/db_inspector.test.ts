import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import test from "node:test";
import { openDistillDatabase } from "../distill/db";
import {
  browseDbTable,
  chooseDefaultBrowseSort,
  ensureSingleStatementSql,
  getDbExplorerSnapshot,
  quoteIdentifier,
  runDbQuery
} from "../distill/db_inspector";
import { DbBrowseResult, DbFilter, DbQueryResult } from "../shared/types";

function withTempDistill(fn: (db: ReturnType<typeof openDistillDatabase>["db"]) => void): void {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "distill-db-inspector-"));
  const previous = process.env.DISTILL_HOME;
  process.env.DISTILL_HOME = path.join(tempRoot, ".distill");

  try {
    const distillDb = openDistillDatabase();
    try {
      fn(distillDb.db);
    } finally {
      distillDb.close();
    }
  } finally {
    if (previous === undefined) {
      delete process.env.DISTILL_HOME;
    } else {
      process.env.DISTILL_HOME = previous;
    }
    fs.rmSync(tempRoot, { recursive: true, force: true });
  }
}

function insertSource(db: ReturnType<typeof openDistillDatabase>["db"], id = 1): void {
  db.prepare(`
    INSERT INTO sources (id, kind, display_name, install_status, detected_at, metadata_json)
    VALUES (?, 'codex', 'Codex', 'installed', '2026-03-30T00:00:00Z', '{}')
  `).run(id);
}

function findColumnIndex(columns: DbBrowseResult["columns"] | DbQueryResult["columns"], name: string): number {
  const index = columns.findIndex((column) => column.name === name);
  assert.notEqual(index, -1, `Expected to find column "${name}"`);
  return index;
}

function rowCellDetail(
  columns: DbBrowseResult["columns"] | DbQueryResult["columns"],
  row: DbBrowseResult["rows"][number] | DbQueryResult["rows"][number],
  columnName: string
): string {
  return row.cells[findColumnIndex(columns, columnName)]?.detail ?? "";
}

test("db inspector discovers grouped tables and excludes sqlite internals", () => {
  withTempDistill(() => {
    const snapshot = getDbExplorerSnapshot();

    assert.equal(snapshot.databaseExists, true);
    assert.equal(snapshot.defaultTableName, "sessions");
    assert(snapshot.coreTables.some((table) => table.name === "sessions"));
    assert(snapshot.advancedTables.some((table) => table.name === "message_fts"));
    assert.equal(
      [...snapshot.coreTables, ...snapshot.advancedTables].some((table) => table.name.startsWith("sqlite_")),
      false
    );
  });
});

test("db inspector picks the default browse sort and paginates session rows", () => {
  withTempDistill((db) => {
    insertSource(db);

    const insertSession = db.prepare(`
      INSERT INTO sessions (
        source_id,
        external_session_id,
        title,
        updated_at,
        message_count,
        raw_capture_count,
        metadata_json
      ) VALUES (?, ?, ?, ?, 0, 1, '{}')
    `);

    for (let index = 0; index < 60; index += 1) {
      insertSession.run(
        1,
        `session-${index}`,
        `Session ${index}`,
        `2026-03-30T12:${String(index).padStart(2, "0")}:00Z`
      );
    }

    const columns = [
      {
        name: "id",
        type: "INTEGER",
        filterKind: "numeric" as const,
        isNullable: false,
        isPrimaryKey: true,
        isHidden: false
      },
      {
        name: "updated_at",
        type: "TEXT",
        filterKind: "date" as const,
        isNullable: true,
        isPrimaryKey: false,
        isHidden: false
      }
    ];
    const defaultSort = chooseDefaultBrowseSort(columns);
    assert.deepEqual(defaultSort, { column: "updated_at", direction: "desc" });

    const result = browseDbTable({
      tableName: "sessions",
      filters: [],
      page: 2,
      pageSize: 25
    });

    assert.deepEqual(result.sort, { column: "updated_at", direction: "desc" });
    assert.equal(result.totalRows, 60);
    assert.equal(result.page, 2);
    assert.equal(result.pageSize, 25);
    assert.equal(result.rows.length, 25);
    assert.equal(rowCellDetail(result.columns, result.rows[0], "title"), "Session 34");
  });
});

test("db inspector preserves bigint browse counts", () => {
  withTempDistill((db) => {
    insertSource(db);

    db.prepare(`
      INSERT INTO sessions (
        source_id,
        external_session_id,
        title,
        updated_at,
        message_count,
        raw_capture_count,
        metadata_json
      ) VALUES (1, 'session-bigint', 'Bigint session', '2026-03-30T12:00:00Z', 0, 1, '{}')
    `).run();

    const originalPrepare = DatabaseSync.prototype.prepare;
    const hugeCount = BigInt(Number.MAX_SAFE_INTEGER) + 1n;

    DatabaseSync.prototype.prepare = function patchedPrepare(sql: string, ...args: unknown[]) {
      const statement = originalPrepare.call(this, sql, ...args as []) as {
        get: (...params: unknown[]) => unknown;
      };

      if (sql.includes("COUNT(*) AS total_rows")) {
        const originalGet = statement.get.bind(statement);
        statement.get = (...params: unknown[]) => {
          const row = originalGet(...params) as { total_rows: number } | undefined;
          return row ? { ...row, total_rows: hugeCount } : row;
        };
      }

      return statement as ReturnType<typeof originalPrepare>;
    };

    try {
      const result = browseDbTable({
        tableName: "sessions",
        filters: [],
        page: 1,
        pageSize: 25
      });

      assert.equal(typeof result.totalRows, "bigint");
      assert.equal(result.totalRows, hugeCount);
    } finally {
      DatabaseSync.prototype.prepare = originalPrepare;
    }
  });
});

test("db inspector supports every structured filter operator and NULL handling", () => {
  withTempDistill((db) => {
    db.exec(`
      CREATE TABLE filter_cases (
        id INTEGER PRIMARY KEY,
        name TEXT,
        score INTEGER,
        created_at TEXT,
        note TEXT
      );
    `);

    const insert = db.prepare(`
      INSERT INTO filter_cases (id, name, score, created_at, note)
      VALUES (?, ?, ?, ?, ?)
    `);

    insert.run(1, "alpha", 10, "2026-03-01T00:00:00Z", null);
    insert.run(2, "alphabet", 20, "2026-03-02T00:00:00Z", "ready");
    insert.run(3, "beta", 20, "2026-03-03T00:00:00Z", "done");
    insert.run(4, null, null, null, null);

    const runIds = (filters: DbFilter[]): number[] => {
      const result = browseDbTable({
        tableName: "filter_cases",
        filters,
        page: 1,
        pageSize: 50
      });

      return result.rows.map((row) => Number(rowCellDetail(result.columns, row, "id")));
    };

    assert.deepEqual(runIds([{ column: "name", operator: "contains", value: "pha" }]), [2, 1]);
    assert.deepEqual(runIds([{ column: "name", operator: "equals", value: "alpha" }]), [1]);
    assert.deepEqual(runIds([{ column: "name", operator: "not_equals", value: "alpha" }]), [3, 2]);
    assert.deepEqual(runIds([{ column: "name", operator: "starts_with", value: "alp" }]), [2, 1]);
    assert.deepEqual(runIds([{ column: "name", operator: "ends_with", value: "ta" }]), [3]);
    assert.deepEqual(runIds([{ column: "score", operator: "eq", value: "20" }]), [3, 2]);
    assert.deepEqual(runIds([{ column: "score", operator: "neq", value: "20" }]), [1]);
    assert.deepEqual(runIds([{ column: "score", operator: "gt", value: "10" }]), [3, 2]);
    assert.deepEqual(runIds([{ column: "score", operator: "gte", value: "20" }]), [3, 2]);
    assert.deepEqual(runIds([{ column: "score", operator: "lt", value: "20" }]), [1]);
    assert.deepEqual(runIds([{ column: "score", operator: "lte", value: "10" }]), [1]);
    assert.deepEqual(runIds([{ column: "score", operator: "is_null" }]), [4]);
    assert.deepEqual(runIds([{ column: "score", operator: "is_not_null" }]), [3, 2, 1]);
    assert.deepEqual(runIds([{ column: "created_at", operator: "gte", value: "2026-03-02T00:00:00Z" }]), [3, 2]);
    assert.deepEqual(runIds([{ column: "note", operator: "is_null" }]), [1, 4]);
    assert.deepEqual(runIds([{ column: "note", operator: "is_not_null" }]), [3, 2]);
  });
});

test("db inspector safely quotes identifiers and binds filter values", () => {
  withTempDistill((db) => {
    assert.equal(quoteIdentifier("value\"col"), "\"value\"\"col\"");

    db.exec(`
      CREATE TABLE "danger""name" (
        "id" INTEGER PRIMARY KEY,
        "value""col" TEXT
      );
    `);

    const insert = db.prepare(`
      INSERT INTO "danger""name" ("id", "value""col")
      VALUES (?, ?)
    `);

    insert.run(1, "literal %_ marker");
    insert.run(2, "plain marker");

    const result = browseDbTable({
      tableName: "danger\"name",
      filters: [
        {
          column: "value\"col",
          operator: "contains",
          value: "%_"
        }
      ],
      page: 1,
      pageSize: 50
    });

    assert.equal(result.rows.length, 1);
    assert.equal(rowCellDetail(result.columns, result.rows[0], "value\"col"), "literal %_ marker");
  });
});

test("db inspector allows read-only custom query shapes", () => {
  withTempDistill((db) => {
    insertSource(db);
    db.prepare(`
      INSERT INTO sessions (
        source_id,
        external_session_id,
        title,
        message_count,
        raw_capture_count,
        metadata_json
      ) VALUES (1, 'session-1', 'Query target', 0, 1, '{}')
    `).run();

    const selectResult = runDbQuery({
      sql: "SELECT title FROM sessions ORDER BY title ASC"
    });
    assert.equal(rowCellDetail(selectResult.columns, selectResult.rows[0], "title"), "Query target");

    const cteResult = runDbQuery({
      sql: "WITH sample AS (SELECT 1 AS value) SELECT value FROM sample"
    });
    assert.equal(rowCellDetail(cteResult.columns, cteResult.rows[0], "value"), "1");

    const pragmaResult = runDbQuery({
      sql: "PRAGMA table_info('sessions')"
    });
    assert(pragmaResult.columns.length > 0);
    assert(pragmaResult.rows.length > 0);

    const explainResult = runDbQuery({
      sql: "EXPLAIN QUERY PLAN SELECT * FROM sessions"
    });
    assert(explainResult.columns.length > 0);
    assert(explainResult.rows.length > 0);

    assert.equal(ensureSingleStatementSql("SELECT 1; -- trailing comment"), "SELECT 1; -- trailing comment");
  });
});

test("db inspector blocks mutating and multi-statement custom queries", () => {
  withTempDistill(() => {
    const blockedQueries = [
      "INSERT INTO sessions (id) VALUES (1)",
      "UPDATE sessions SET title = 'x'",
      "DELETE FROM sessions",
      "CREATE TABLE blocked (id INTEGER)",
      "DROP TABLE sessions",
      "ALTER TABLE sessions RENAME TO moved_sessions",
      "ATTACH DATABASE ':memory:' AS other_db",
      "DETACH DATABASE main",
      "PRAGMA writable_schema = ON"
    ];

    for (const sql of blockedQueries) {
      assert.throws(
        () => runDbQuery({ sql }),
        /Query blocked\. Only read-only single-statement queries are allowed\./
      );
    }

    assert.throws(
      () => runDbQuery({ sql: "SELECT 1; DELETE FROM sessions" }),
      /Only one SQL statement is allowed per query\./
    );
  });
});

test("db inspector validates runtime browse and query request shapes", () => {
  withTempDistill(() => {
    assert.throws(
      () => runDbQuery({} as unknown as { sql: string }),
      /Invalid DB query request\./
    );

    assert.throws(
      () =>
        browseDbTable({
          tableName: "sessions",
          filters: "invalid",
          page: 1,
          pageSize: 50
        } as unknown as Parameters<typeof browseDbTable>[0]),
      /Invalid DB browse request\./
    );

    assert.throws(
      () =>
        browseDbTable({
          tableName: "sessions",
          filters: [
            {
              column: "title",
              operator: "DROP TABLE"
            }
          ],
          page: 1,
          pageSize: 50
        } as unknown as Parameters<typeof browseDbTable>[0]),
      /Invalid DB browse request\./
    );
  });
});

test("db inspector preserves duplicate custom query column names", () => {
  withTempDistill(() => {
    const result = runDbQuery({
      sql: "SELECT 1 AS value, 2 AS value"
    });

    assert.equal(result.columns.length, 2);
    assert.equal(result.columns[0]?.name, "value");
    assert.equal(result.columns[1]?.name, "value");
    assert.equal(result.rows[0]?.cells[0]?.detail, "1");
    assert.equal(result.rows[0]?.cells[1]?.detail, "2");
  });
});

test("db inspector truncates large result sets and summarizes large cells", () => {
  withTempDistill((db) => {
    db.exec(`
      CREATE TABLE payload_rows (
        id INTEGER PRIMARY KEY,
        content TEXT,
        payload BLOB
      );
    `);

    const insert = db.prepare(`
      INSERT INTO payload_rows (id, content, payload)
      VALUES (?, ?, ?)
    `);

    for (let index = 1; index <= 105; index += 1) {
      insert.run(
        index,
        index === 1 ? "x".repeat(9000) : `row-${index}`,
        index === 1 ? Buffer.alloc(1536, 7) : Buffer.alloc(8, index)
      );
    }

    const result = runDbQuery({
      sql: "SELECT content, payload FROM payload_rows ORDER BY id ASC"
    });

    assert.equal(result.truncated, true);
    assert.equal(result.rowCount, 100);
    assert.equal(result.rows.length, 100);

    const contentCell = result.rows[0]?.cells[0];
    const payloadCell = result.rows[0]?.cells[1];

    assert.equal(contentCell?.kind, "text");
    assert.equal(contentCell?.detailTruncated, true);
    assert.equal(contentCell?.previewTruncated, true);
    assert.match(contentCell?.detail ?? "", /…$/);
    assert.equal(payloadCell?.kind, "blob");
    assert.equal(payloadCell?.preview, "[blob 1536 bytes]");
    assert.equal(payloadCell?.detail, "[blob 1536 bytes]");
  });
});
