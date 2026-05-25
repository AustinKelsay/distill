import fs from "node:fs";
import { DatabaseSync, constants } from "node:sqlite";
import {
  DbBrowseRequest,
  DbBrowseResult,
  DbCellValue,
  DbColumnFilterKind,
  DbColumnInfo,
  DbExplorerSnapshot,
  DbFilter,
  DbFilterOperator,
  DbForeignKeyInfo,
  DbQueryRequest,
  DbQueryResult,
  DbRowCount,
  DbResultColumn,
  DbResultRow,
  DbSort,
  DbSortDirection,
  DbTableSummary
} from "../shared/types";
import { getDistillDatabasePath } from "./paths";

type SQLitePrimitive = null | number | bigint | string | Uint8Array;

type SqliteMasterRow = {
  name: string;
  sql: string | null;
};

type TableXInfoRow = {
  name: string;
  type: string | null;
  notnull: number;
  dflt_value: string | null;
  pk: number;
  hidden: number;
};

type ForeignKeyRow = {
  id: number;
  seq: number;
  table: string;
  from: string;
  to: string | null;
  on_update: string;
  on_delete: string;
  match: string;
};

type StatementColumnMetadata = {
  name: string;
  column: string | null;
  table: string | null;
  database: string | null;
  type: string | null;
};

const CORE_TABLE_NAMES = [
  "sources",
  "captures",
  "sessions",
  "capture_records",
  "messages",
  "artifacts",
  "tags",
  "tag_assignments",
  "labels",
  "label_assignments"
] as const;

const ADVANCED_TABLE_NAMES = [
  "activity_events",
  "jobs",
  "exports",
  "user_preferences",
  "message_fts"
] as const;

const CORE_TABLE_SET = new Set<string>(CORE_TABLE_NAMES);
const ADVANCED_TABLE_SET = new Set<string>(ADVANCED_TABLE_NAMES);
const PRAGMA_ALLOWLIST = new Set([
  "table_info",
  "table_xinfo",
  "table_list",
  "index_list",
  "index_xinfo",
  "foreign_key_list"
]);
const ALLOWED_ACTION_CODES = new Set<number>([
  constants.SQLITE_SELECT,
  constants.SQLITE_READ,
  constants.SQLITE_FUNCTION,
  constants.SQLITE_RECURSIVE
]);
const MAX_RESULT_ROWS = 100;
const RESULT_PROBE_ROWS = MAX_RESULT_ROWS + 1;
const MAX_CELL_DETAIL_LENGTH = 8 * 1024;
const MAX_CELL_PREVIEW_LENGTH = 140;
const PAGE_SIZE_OPTIONS = new Set([25, 50, 100]);
const FILTER_OPERATORS = new Set<DbFilterOperator>([
  "contains",
  "equals",
  "not_equals",
  "starts_with",
  "ends_with",
  "eq",
  "neq",
  "gt",
  "gte",
  "lt",
  "lte",
  "is_null",
  "is_not_null"
]);
const SORT_DIRECTIONS = new Set<DbSortDirection>(["asc", "desc"]);

export function quoteIdentifier(identifier: string): string {
  if (!identifier.trim()) {
    throw new Error("Identifier cannot be empty.");
  }

  return `"${identifier.replace(/"/g, "\"\"")}"`;
}

export function chooseDefaultBrowseSort(columns: DbColumnInfo[]): DbSort {
  const visibleColumns = columns.filter((column) => !column.isHidden);
  const preferredDescending = [
    "updated_at",
    "updated_recorded_at",
    "created_at",
    "created_recorded_at",
    "id"
  ];

  for (const name of preferredDescending) {
    if (visibleColumns.some((column) => column.name === name)) {
      return {
        column: name,
        direction: "desc"
      };
    }
  }

  const fallbackColumn = visibleColumns[0];
  if (!fallbackColumn) {
    throw new Error("The selected table does not expose any visible columns.");
  }

  return {
    column: fallbackColumn.name,
    direction: "asc"
  };
}

export function ensureSingleStatementSql(sql: string): string {
  if (!sql.trim()) {
    throw new Error("SQL query is required.");
  }

  let mode:
    | "normal"
    | "single_quote"
    | "double_quote"
    | "backtick"
    | "bracket"
    | "line_comment"
    | "block_comment" = "normal";

  for (let index = 0; index < sql.length; index += 1) {
    const current = sql[index];
    const next = sql[index + 1];

    if (mode === "line_comment") {
      if (current === "\n") {
        mode = "normal";
      }
      continue;
    }

    if (mode === "block_comment") {
      if (current === "*" && next === "/") {
        mode = "normal";
        index += 1;
      }
      continue;
    }

    if (mode === "single_quote") {
      if (current === "'" && next === "'") {
        index += 1;
        continue;
      }
      if (current === "'") {
        mode = "normal";
      }
      continue;
    }

    if (mode === "double_quote") {
      if (current === "\"" && next === "\"") {
        index += 1;
        continue;
      }
      if (current === "\"") {
        mode = "normal";
      }
      continue;
    }

    if (mode === "backtick") {
      if (current === "`" && next === "`") {
        index += 1;
        continue;
      }
      if (current === "`") {
        mode = "normal";
      }
      continue;
    }

    if (mode === "bracket") {
      if (current === "]") {
        mode = "normal";
      }
      continue;
    }

    if (current === "-" && next === "-") {
      mode = "line_comment";
      index += 1;
      continue;
    }

    if (current === "/" && next === "*") {
      mode = "block_comment";
      index += 1;
      continue;
    }

    if (current === "'") {
      mode = "single_quote";
      continue;
    }

    if (current === "\"") {
      mode = "double_quote";
      continue;
    }

    if (current === "`") {
      mode = "backtick";
      continue;
    }

    if (current === "[") {
      mode = "bracket";
      continue;
    }

    if (current === ";") {
      for (let remainderIndex = index + 1; remainderIndex < sql.length; remainderIndex += 1) {
        const remainder = sql[remainderIndex];
        const remainderNext = sql[remainderIndex + 1];

        if (/\s/.test(remainder)) {
          continue;
        }

        if (remainder === "-" && remainderNext === "-") {
          remainderIndex += 2;
          while (remainderIndex < sql.length && sql[remainderIndex] !== "\n") {
            remainderIndex += 1;
          }
          continue;
        }

        if (remainder === "/" && remainderNext === "*") {
          remainderIndex += 2;
          while (remainderIndex < sql.length - 1) {
            if (sql[remainderIndex] === "*" && sql[remainderIndex + 1] === "/") {
              remainderIndex += 1;
              break;
            }
            remainderIndex += 1;
          }
          continue;
        }

        throw new Error("Only one SQL statement is allowed per query.");
      }
    }
  }

  return sql.trim();
}

export function getDbExplorerSnapshot(): DbExplorerSnapshot {
  const databasePath = getDistillDatabasePath();
  if (!fs.existsSync(databasePath)) {
    return {
      databasePath,
      databaseExists: false,
      coreTables: [],
      advancedTables: []
    };
  }

  const db = openInspectorDatabase(databasePath);
  try {
    const tables = listTables(db);
    const coreTables = tables.filter((table) => table.isCore);
    const advancedTables = tables.filter((table) => !table.isCore);
    const defaultTableName =
      tables.find((table) => table.name === "sessions")?.name
      ?? coreTables[0]?.name
      ?? advancedTables[0]?.name;

    return {
      databasePath,
      databaseExists: true,
      coreTables,
      advancedTables,
      defaultTableName
    };
  } finally {
    db.close();
  }
}

export function browseDbTable(request: unknown): DbBrowseResult {
  const parsedRequest = parseBrowseRequest(request);
  const databasePath = getDistillDatabasePath();
  if (!fs.existsSync(databasePath)) {
    throw new Error(`Database file not found at ${databasePath}.`);
  }

  const db = openInspectorDatabase(databasePath);
  try {
    const table = getTableSummary(db, parsedRequest.tableName);
    const schemaColumns = getTableSchemaColumns(db, table.name);
    const foreignKeys = getTableForeignKeys(db, table.name);
    const visibleColumns = schemaColumns.filter((column) => !column.isHidden);
    if (visibleColumns.length === 0) {
      throw new Error(`Table "${table.name}" does not expose any visible columns.`);
    }

    const filters = normalizeFilters(parsedRequest.filters, visibleColumns);
    const sort = normalizeSort(parsedRequest.sort, visibleColumns);
    const pageSize = normalizePageSize(parsedRequest.pageSize);
    const page = normalizePage(parsedRequest.page);
    const where = buildWhereClause(filters, visibleColumns);
    const selectSql = buildBrowseSelectSql(table.name, visibleColumns, where.sql, sort, pageSize, page);
    const countSql = buildBrowseCountSql(table.name, where.sql);
    const rows = runBrowseSelect(db, selectSql, where.params, pageSize, page);
    const totalRows = getBrowseCount(db, countSql, where.params);

    return {
      databasePath,
      table,
      schemaColumns,
      foreignKeys,
      appliedFilters: filters,
      sort,
      page,
      pageSize,
      totalRows,
      columns: visibleColumns.map((column): DbResultColumn => ({
        name: column.name,
        sourceColumn: column.name,
        table: table.name,
        database: "main",
        type: column.type
      })),
      rows
    };
  } finally {
    db.close();
  }
}

export function runDbQuery(request: unknown): DbQueryResult {
  const snapshot = getDbExplorerSnapshot();
  if (!snapshot.databaseExists) {
    throw new Error(`Database file not found at ${snapshot.databasePath}.`);
  }

  const { sql: rawSql } = parseQueryRequest(request);
  const sql = ensureSingleStatementSql(rawSql);
  const db = openInspectorDatabase(snapshot.databasePath);
  const startedAt = Date.now();

  try {
    const statement = db.prepare(sql, { returnArrays: true });
    const executedSql = statement.sourceSQL.trim();
    const columns = serializeColumns(statement.columns() as StatementColumnMetadata[]);
    const rows: DbResultRow[] = [];
    let rowIndex = 0;

    for (const row of statement.iterate() as unknown as Iterable<SQLitePrimitive[]>) {
      rows.push(serializeRow(rowIndex, row));
      rowIndex += 1;
      if (rows.length >= RESULT_PROBE_ROWS) {
        break;
      }
    }

    const truncated = rows.length > MAX_RESULT_ROWS;
    const visibleRows = truncated ? rows.slice(0, MAX_RESULT_ROWS) : rows;

    return {
      databasePath: snapshot.databasePath,
      executedSql,
      durationMs: Date.now() - startedAt,
      columns,
      rows: visibleRows,
      rowCount: visibleRows.length,
      truncated
    };
  } catch (error) {
    throw normalizeInspectorError(error);
  } finally {
    db.close();
  }
}

function openInspectorDatabase(databasePath: string): DatabaseSync {
  const db = new DatabaseSync(databasePath, {
    readOnly: true,
    allowExtension: false,
    timeout: 2000,
    defensive: true
  });

  db.setAuthorizer((actionCode, arg1) => {
    if (ALLOWED_ACTION_CODES.has(actionCode)) {
      return constants.SQLITE_OK;
    }

    if (actionCode === constants.SQLITE_PRAGMA && arg1 && PRAGMA_ALLOWLIST.has(arg1)) {
      return constants.SQLITE_OK;
    }

    return constants.SQLITE_DENY;
  });

  return db;
}

function parseBrowseRequest(request: unknown): DbBrowseRequest {
  if (!isRecord(request)) {
    throw new Error("Invalid DB browse request.");
  }

  if (typeof request.tableName !== "string" || !request.tableName.trim()) {
    throw new Error("Invalid DB browse request.");
  }

  if (!Array.isArray(request.filters)) {
    throw new Error("Invalid DB browse request.");
  }

  const filters = request.filters.map(parseFilter);
  const sort = request.sort === undefined ? undefined : parseSort(request.sort);

  if (typeof request.page !== "number" || !Number.isFinite(request.page)) {
    throw new Error("Invalid DB browse request.");
  }

  if (typeof request.pageSize !== "number" || !Number.isFinite(request.pageSize)) {
    throw new Error("Invalid DB browse request.");
  }

  return {
    tableName: request.tableName.trim(),
    filters,
    sort,
    page: request.page,
    pageSize: request.pageSize
  };
}

function parseFilter(filter: unknown): DbFilter {
  if (!isRecord(filter)) {
    throw new Error("Invalid DB browse request.");
  }

  if (typeof filter.column !== "string" || !filter.column.trim()) {
    throw new Error("Invalid DB browse request.");
  }

  if (typeof filter.operator !== "string" || !FILTER_OPERATORS.has(filter.operator as DbFilterOperator)) {
    throw new Error("Invalid DB browse request.");
  }

  if (filter.value !== undefined && typeof filter.value !== "string") {
    throw new Error("Invalid DB browse request.");
  }

  return {
    column: filter.column.trim(),
    operator: filter.operator as DbFilterOperator,
    value: filter.value
  };
}

function parseSort(sort: unknown): DbSort {
  if (!isRecord(sort)) {
    throw new Error("Invalid DB browse request.");
  }

  if (typeof sort.column !== "string" || !sort.column.trim()) {
    throw new Error("Invalid DB browse request.");
  }

  if (typeof sort.direction !== "string" || !SORT_DIRECTIONS.has(sort.direction as DbSortDirection)) {
    throw new Error("Invalid DB browse request.");
  }

  return {
    column: sort.column.trim(),
    direction: sort.direction as DbSortDirection
  };
}

function parseQueryRequest(request: unknown): DbQueryRequest {
  if (!isRecord(request) || typeof request.sql !== "string") {
    throw new Error("Invalid DB query request.");
  }

  return {
    sql: request.sql
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function listTables(db: DatabaseSync): DbTableSummary[] {
  const rows = db.prepare(`
    SELECT name, sql
    FROM sqlite_master
    WHERE type = 'table'
      AND name NOT LIKE 'sqlite_%'
    ORDER BY name COLLATE NOCASE ASC
  `).all() as SqliteMasterRow[];

  return rows
    .map((row): DbTableSummary => ({
      name: row.name,
      kind: isVirtualTableSql(row.sql) ? "virtual" : "table",
      isCore: CORE_TABLE_SET.has(row.name)
    }))
    .sort(compareTableSummaries);
}

function compareTableSummaries(left: DbTableSummary, right: DbTableSummary): number {
  if (left.isCore !== right.isCore) {
    return left.isCore ? -1 : 1;
  }

  const leftRank = tableSortRank(left.name, left.isCore);
  const rightRank = tableSortRank(right.name, right.isCore);

  if (leftRank !== rightRank) {
    return leftRank - rightRank;
  }

  return left.name.localeCompare(right.name);
}

function tableSortRank(name: string, isCore: boolean): number {
  const source = isCore ? CORE_TABLE_NAMES : ADVANCED_TABLE_NAMES;
  const index = source.indexOf(name as never);
  return index === -1 ? source.length + 100 : index;
}

function getTableSummary(db: DatabaseSync, tableName: string): DbTableSummary {
  const table = listTables(db).find((entry) => entry.name === tableName);
  if (!table) {
    throw new Error(`Unknown table "${tableName}".`);
  }

  return table;
}

function getTableSchemaColumns(db: DatabaseSync, tableName: string): DbColumnInfo[] {
  const sql = `PRAGMA table_xinfo(${quoteSqlString(tableName)})`;
  const rows = db.prepare(sql).all() as TableXInfoRow[];

  return rows.map((row): DbColumnInfo => ({
    name: row.name,
    type: cleanType(row.type),
    filterKind: deriveFilterKind(row.name, row.type),
    isNullable: row.notnull === 0 && row.pk === 0,
    isPrimaryKey: row.pk > 0,
    isHidden: row.hidden !== 0,
    primaryKeyOrdinal: row.pk > 0 ? row.pk : undefined,
    defaultValue: row.dflt_value ?? undefined
  }));
}

function getTableForeignKeys(db: DatabaseSync, tableName: string): DbForeignKeyInfo[] {
  const sql = `PRAGMA foreign_key_list(${quoteSqlString(tableName)})`;
  const rows = db.prepare(sql).all() as ForeignKeyRow[];

  return rows.map((row): DbForeignKeyInfo => ({
    id: row.id,
    seq: row.seq,
    table: row.table,
    from: row.from,
    to: row.to ?? undefined,
    onUpdate: row.on_update,
    onDelete: row.on_delete,
    match: row.match
  }));
}

function buildBrowseSelectSql(
  tableName: string,
  columns: DbColumnInfo[],
  whereSql: string,
  sort: DbSort,
  pageSize: number,
  page: number
): string {
  const offset = (page - 1) * pageSize;
  const selectColumns = columns.map((column) => quoteIdentifier(column.name)).join(", ");

  return `
    SELECT ${selectColumns}
    FROM ${quoteIdentifier(tableName)}
    ${whereSql}
    ORDER BY ${quoteIdentifier(sort.column)} ${sort.direction.toUpperCase()}
    LIMIT ${pageSize}
    OFFSET ${offset}
  `;
}

function buildBrowseCountSql(tableName: string, whereSql: string): string {
  return `
    SELECT COUNT(*) AS total_rows
    FROM ${quoteIdentifier(tableName)}
    ${whereSql}
  `;
}

function runBrowseSelect(
  db: DatabaseSync,
  sql: string,
  params: string[],
  pageSize: number,
  page: number
): DbResultRow[] {
  const rows = db.prepare(sql, { returnArrays: true }).all(...params) as unknown as SQLitePrimitive[][];

  return rows.slice(0, MAX_RESULT_ROWS).map((row, index) =>
    serializeRow(index + ((page - 1) * pageSize), row)
  );
}

function getBrowseCount(db: DatabaseSync, sql: string, params: string[]): DbRowCount {
  const row = db.prepare(sql).get(...params) as { total_rows: DbRowCount } | undefined;
  if (!row) {
    return 0;
  }

  return row.total_rows;
}

function normalizeFilters(filters: DbFilter[], visibleColumns: DbColumnInfo[]): DbFilter[] {
  const columnMap = new Map(visibleColumns.map((column) => [column.name, column]));
  const normalizedFilters: DbFilter[] = [];

  for (const filter of filters) {
    const column = columnMap.get(filter.column);
    if (!column) {
      throw new Error(`Unknown filter column "${filter.column}".`);
    }

    if (!isOperatorAllowedForColumn(column, filter.operator)) {
      throw new Error(`Operator "${filter.operator}" is not valid for column "${column.name}".`);
    }

    if (requiresFilterValue(filter.operator) && !filter.value?.trim()) {
      continue;
    }

    normalizedFilters.push({
      column: column.name,
      operator: filter.operator,
      value: filter.value?.trim()
    });
  }

  return normalizedFilters;
}

function buildWhereClause(
  filters: DbFilter[],
  visibleColumns: DbColumnInfo[]
): { sql: string; params: string[] } {
  if (filters.length === 0) {
    return {
      sql: "",
      params: []
    };
  }

  const columnMap = new Map(visibleColumns.map((column) => [column.name, column]));
  const clauses: string[] = [];
  const params: string[] = [];

  for (const filter of filters) {
    const column = columnMap.get(filter.column);
    if (!column) {
      continue;
    }

    const quotedColumn = quoteIdentifier(column.name);

    switch (filter.operator) {
      case "contains":
        clauses.push(`CAST(${quotedColumn} AS TEXT) LIKE ? ESCAPE '\\'`);
        params.push(`%${escapeLikePattern(filter.value ?? "")}%`);
        break;
      case "equals":
        clauses.push(`CAST(${quotedColumn} AS TEXT) = ?`);
        params.push(filter.value ?? "");
        break;
      case "not_equals":
        clauses.push(`CAST(${quotedColumn} AS TEXT) <> ?`);
        params.push(filter.value ?? "");
        break;
      case "starts_with":
        clauses.push(`CAST(${quotedColumn} AS TEXT) LIKE ? ESCAPE '\\'`);
        params.push(`${escapeLikePattern(filter.value ?? "")}%`);
        break;
      case "ends_with":
        clauses.push(`CAST(${quotedColumn} AS TEXT) LIKE ? ESCAPE '\\'`);
        params.push(`%${escapeLikePattern(filter.value ?? "")}`);
        break;
      case "eq":
        clauses.push(`${quotedColumn} = ?`);
        params.push(filter.value ?? "");
        break;
      case "neq":
        clauses.push(`${quotedColumn} <> ?`);
        params.push(filter.value ?? "");
        break;
      case "gt":
        clauses.push(`${quotedColumn} > ?`);
        params.push(filter.value ?? "");
        break;
      case "gte":
        clauses.push(`${quotedColumn} >= ?`);
        params.push(filter.value ?? "");
        break;
      case "lt":
        clauses.push(`${quotedColumn} < ?`);
        params.push(filter.value ?? "");
        break;
      case "lte":
        clauses.push(`${quotedColumn} <= ?`);
        params.push(filter.value ?? "");
        break;
      case "is_null":
        clauses.push(`${quotedColumn} IS NULL`);
        break;
      case "is_not_null":
        clauses.push(`${quotedColumn} IS NOT NULL`);
        break;
    }
  }

  return {
    sql: clauses.length ? `WHERE ${clauses.join(" AND ")}` : "",
    params
  };
}

function normalizeSort(sort: DbSort | undefined, visibleColumns: DbColumnInfo[]): DbSort {
  if (!sort) {
    return chooseDefaultBrowseSort(visibleColumns);
  }

  const column = visibleColumns.find((entry) => entry.name === sort.column);
  if (!column) {
    return chooseDefaultBrowseSort(visibleColumns);
  }

  return {
    column: column.name,
    direction: normalizeSortDirection(sort.direction)
  };
}

function normalizeSortDirection(direction: DbSortDirection | string | undefined): DbSortDirection {
  return direction?.toLowerCase() === "asc" ? "asc" : "desc";
}

function normalizePageSize(pageSize: number): number {
  return PAGE_SIZE_OPTIONS.has(pageSize) ? pageSize : 50;
}

function normalizePage(page: number): number {
  return Number.isInteger(page) && page > 0 ? page : 1;
}

function deriveFilterKind(name: string, type: string | null): DbColumnFilterKind {
  const normalizedType = type?.trim().toUpperCase() ?? "";
  const normalizedName = name.toLowerCase();

  if (/_at$/.test(normalizedName) || normalizedType.includes("DATE") || normalizedType.includes("TIME")) {
    return "date";
  }

  if (
    normalizedType.includes("INT")
    || normalizedType.includes("REAL")
    || normalizedType.includes("FLOA")
    || normalizedType.includes("DOUB")
    || normalizedType.includes("NUM")
    || normalizedType.includes("DEC")
    || normalizedType.includes("BOOL")
  ) {
    return "numeric";
  }

  if (
    !normalizedType
    || normalizedType.includes("CHAR")
    || normalizedType.includes("CLOB")
    || normalizedType.includes("TEXT")
    || normalizedType.includes("JSON")
  ) {
    return "text";
  }

  return "other";
}

function isOperatorAllowedForColumn(column: DbColumnInfo, operator: DbFilterOperator): boolean {
  if (operator === "is_null" || operator === "is_not_null") {
    return true;
  }

  if (column.filterKind === "text") {
    return [
      "contains",
      "equals",
      "not_equals",
      "starts_with",
      "ends_with"
    ].includes(operator);
  }

  if (column.filterKind === "numeric" || column.filterKind === "date") {
    return [
      "eq",
      "neq",
      "gt",
      "gte",
      "lt",
      "lte"
    ].includes(operator);
  }

  return ["equals", "not_equals", "eq", "neq"].includes(operator);
}

function requiresFilterValue(operator: DbFilterOperator): boolean {
  return operator !== "is_null" && operator !== "is_not_null";
}

function escapeLikePattern(value: string): string {
  return value.replace(/[\\%_]/g, "\\$&");
}

function serializeColumns(columns: StatementColumnMetadata[]): DbResultColumn[] {
  return columns.map((column): DbResultColumn => ({
    name: column.name,
    sourceColumn: column.column ?? undefined,
    table: column.table ?? undefined,
    database: column.database ?? undefined,
    type: cleanType(column.type)
  }));
}

function serializeRow(index: number, row: SQLitePrimitive[]): DbResultRow {
  return {
    key: String(index),
    cells: row.map(serializeCellValue)
  };
}

function serializeCellValue(value: SQLitePrimitive): DbCellValue {
  if (value === null) {
    return {
      kind: "null",
      preview: "NULL",
      detail: "NULL",
      previewTruncated: false,
      detailTruncated: false
    };
  }

  if (typeof value === "number" || typeof value === "bigint") {
    const text = String(value);
    return {
      kind: "number",
      preview: text,
      detail: text,
      previewTruncated: false,
      detailTruncated: false
    };
  }

  if (typeof value === "string") {
    const detailTruncated = value.length > MAX_CELL_DETAIL_LENGTH;
    const detailBase = detailTruncated ? value.slice(0, MAX_CELL_DETAIL_LENGTH) : value;
    const detail = detailTruncated ? `${detailBase}…` : detailBase;
    const previewTruncated = detail.length > MAX_CELL_PREVIEW_LENGTH;
    const previewBase = previewTruncated ? detail.slice(0, MAX_CELL_PREVIEW_LENGTH) : detail;
    const preview = previewTruncated ? `${previewBase}…` : previewBase;

    return {
      kind: "text",
      preview,
      detail,
      previewTruncated,
      detailTruncated
    };
  }

  const byteLength = value.byteLength;
  const summary = `[blob ${byteLength} bytes]`;
  return {
    kind: "blob",
    preview: summary,
    detail: summary,
    previewTruncated: false,
    detailTruncated: false,
    byteLength
  };
}

function isVirtualTableSql(sql: string | null): boolean {
  return /CREATE\s+VIRTUAL\s+TABLE/i.test(sql ?? "");
}

function cleanType(type: string | null | undefined): string | undefined {
  const trimmed = type?.trim();
  return trimmed ? trimmed : undefined;
}

function quoteSqlString(value: string): string {
  return `'${value.replace(/'/g, "''")}'`;
}

function normalizeInspectorError(error: unknown): Error {
  if (error instanceof Error) {
    const message = error.message.toLowerCase();
    if (
      message.includes("not authorized")
      || message.includes("readonly database")
      || message.includes("attempt to write")
    ) {
      return new Error("Query blocked. Only read-only single-statement queries are allowed.");
    }

    return error;
  }

  return new Error("DB inspector query failed.");
}
