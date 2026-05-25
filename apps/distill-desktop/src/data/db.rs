use anyhow::{Result, bail};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, ToSql};

use crate::view_models::{
    DbBrowseVm, DbColumnHeaderVm, DbExplorerVm, DbGridRowVm, DbQueryVm, DbSchemaColumnVm,
    DbTableVm,
};

use super::{
    DbBrowseRequestVm, DesktopDataSource, FILTER_OPERATORS, PAGE_SIZE, cell_to_text,
    escape_like_pattern, guard_read_only_sql, quote_identifier, quote_sql_string, truncate_inline,
};

#[derive(Clone, Debug)]
pub(super) struct TableSummary {
    pub name: String,
    pub kind: String,
}

#[derive(Clone, Debug)]
struct TableColumn {
    name: String,
    type_label: String,
    hidden: bool,
    primary_key: bool,
    nullable: bool,
}

impl DesktopDataSource {
    pub fn load_db_snapshot(&self, selected_table_name: Option<&str>) -> Result<DbExplorerVm> {
        let mut vm = DbExplorerVm::default();

        if !self.database_exists() {
            vm.browse.error = format!(
                "Database file not found at {}",
                self.database_path().display()
            );
            return Ok(vm);
        }

        let conn = self.open_read_only()?;
        let tables = self.list_tables(&conn)?;
        let selected = selected_table_name
            .filter(|candidate| tables.iter().any(|table| table.name == *candidate))
            .map(ToOwned::to_owned)
            .or_else(|| tables.first().map(|table| table.name.clone()));

        vm.tables = tables
            .iter()
            .map(|table| DbTableVm {
                name: table.name.clone(),
                kind: table.kind.clone(),
                selected: selected.as_deref() == Some(table.name.as_str()),
            })
            .collect();
        vm.selected_table_name = selected.clone();

        if let Some(selected) = selected {
            let browse = self.browse_db_table(DbBrowseRequestVm {
                table_name: selected,
                page: 1,
                ..DbBrowseRequestVm::default()
            })?;
            vm.filter_column = browse.filter_columns.first().cloned().unwrap_or_default();
            vm.filter_operator = "contains".to_string();
            vm.sort_column = browse
                .sort_columns
                .iter()
                .find(|column| {
                    [
                        "updated_at",
                        "updated_recorded_at",
                        "created_at",
                        "created_recorded_at",
                        "id",
                    ]
                    .contains(&column.as_str())
                })
                .cloned()
                .unwrap_or_else(|| {
                    browse.sort_columns.first().cloned().unwrap_or_default()
                });
            vm.sort_direction = "desc".to_string();
            vm.browse = browse;
        }

        Ok(vm)
    }

    pub fn browse_db_table(&self, request: DbBrowseRequestVm) -> Result<DbBrowseVm> {
        if !self.database_exists() {
            return Ok(DbBrowseVm {
                error: format!(
                    "Database file not found at {}",
                    self.database_path().display()
                ),
                ..DbBrowseVm::default()
            });
        }

        let conn = self.open_read_only()?;
        let columns = self.table_columns(&conn, &request.table_name)?;
        let visible_columns = columns
            .iter()
            .filter(|column| !column.hidden)
            .map(|column| column.name.clone())
            .collect::<Vec<_>>();
        if visible_columns.is_empty() {
            bail!(
                "table \"{}\" does not expose any visible columns",
                request.table_name
            );
        }

        let sort_column = if request.sort_column.trim().is_empty() {
            visible_columns
                .iter()
                .find(|column| {
                    [
                        "updated_at",
                        "updated_recorded_at",
                        "created_at",
                        "created_recorded_at",
                        "id",
                    ]
                    .contains(&column.as_str())
                })
                .cloned()
                .unwrap_or_else(|| visible_columns[0].clone())
        } else {
            request.sort_column.clone()
        };
        let sort_direction = if request.sort_direction.eq_ignore_ascii_case("asc") {
            "ASC"
        } else {
            "DESC"
        };

        let page = request.page.max(1);
        let offset = (page - 1) * PAGE_SIZE;
        let mut params: Vec<String> = Vec::new();
        let mut where_clauses = Vec::new();
        if !request.filter_column.trim().is_empty() && !request.filter_operator.trim().is_empty() {
            let quoted_column = quote_identifier(&request.filter_column)?;
            let operator = request.filter_operator.as_str();
            if !FILTER_OPERATORS.contains(&operator) {
                bail!("unsupported filter operator: {operator}");
            }

            match operator {
                "contains" => {
                    where_clauses.push(format!("CAST({quoted_column} AS TEXT) LIKE ? ESCAPE '\\'"));
                    params.push(format!(
                        "%{}%",
                        escape_like_pattern(request.filter_value.trim())
                    ));
                }
                "equals" => {
                    where_clauses.push(format!("CAST({quoted_column} AS TEXT) = ?"));
                    params.push(request.filter_value.trim().to_string());
                }
                "not_equals" => {
                    where_clauses.push(format!("CAST({quoted_column} AS TEXT) <> ?"));
                    params.push(request.filter_value.trim().to_string());
                }
                "starts_with" => {
                    where_clauses.push(format!("CAST({quoted_column} AS TEXT) LIKE ? ESCAPE '\\'"));
                    params.push(format!(
                        "{}%",
                        escape_like_pattern(request.filter_value.trim())
                    ));
                }
                "ends_with" => {
                    where_clauses.push(format!("CAST({quoted_column} AS TEXT) LIKE ? ESCAPE '\\'"));
                    params.push(format!(
                        "%{}",
                        escape_like_pattern(request.filter_value.trim())
                    ));
                }
                "eq" => {
                    where_clauses.push(format!("{quoted_column} = ?"));
                    params.push(request.filter_value.trim().to_string());
                }
                "neq" => {
                    where_clauses.push(format!("{quoted_column} <> ?"));
                    params.push(request.filter_value.trim().to_string());
                }
                "gt" => {
                    where_clauses.push(format!("{quoted_column} > ?"));
                    params.push(request.filter_value.trim().to_string());
                }
                "gte" => {
                    where_clauses.push(format!("{quoted_column} >= ?"));
                    params.push(request.filter_value.trim().to_string());
                }
                "lt" => {
                    where_clauses.push(format!("{quoted_column} < ?"));
                    params.push(request.filter_value.trim().to_string());
                }
                "lte" => {
                    where_clauses.push(format!("{quoted_column} <= ?"));
                    params.push(request.filter_value.trim().to_string());
                }
                "is_null" => where_clauses.push(format!("{quoted_column} IS NULL")),
                "is_not_null" => where_clauses.push(format!("{quoted_column} IS NOT NULL")),
                _ => unreachable!(),
            }
        }

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };

        let quoted_table = quote_identifier(&request.table_name)?;
        let select_sql = format!(
            "SELECT {} FROM {} {} ORDER BY {} {} LIMIT {} OFFSET {}",
            visible_columns
                .iter()
                .map(|column| quote_identifier(column))
                .collect::<Result<Vec<_>>>()?
                .join(", "),
            quoted_table,
            where_sql,
            quote_identifier(&sort_column)?,
            sort_direction,
            PAGE_SIZE,
            offset,
        );
        let count_sql = format!("SELECT COUNT(*) FROM {} {}", quoted_table, where_sql);

        let params_refs = params
            .iter()
            .map(|value| value as &dyn ToSql)
            .collect::<Vec<_>>();
        let total_rows = conn.query_row(&count_sql, params_refs.as_slice(), |row| {
            row.get::<_, i64>(0)
        })?;

        let mut statement = conn.prepare(&select_sql)?;
        let column_names = statement
            .column_names()
            .into_iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>();
        let mut rows = statement.query(params_refs.as_slice())?;
        let mut result_rows = Vec::new();
        let mut row_index = offset;
        while let Some(row) = rows.next()? {
            let values = (0..column_names.len())
                .map(|index| cell_to_text(row.get_ref(index).unwrap_or(ValueRef::Null)))
                .collect::<Vec<_>>();
            let preview = values
                .iter()
                .zip(column_names.iter())
                .take(3)
                .map(|(value, column)| format!("{column}={}", truncate_inline(value, 36)))
                .collect::<Vec<_>>()
                .join(" · ");
            let detail = column_names
                .iter()
                .zip(values.iter())
                .map(|(column, value)| format!("{column}: {value}"))
                .collect::<Vec<_>>()
                .join("\n");
            result_rows.push(DbGridRowVm {
                key: format!("row-{row_index}"),
                preview,
                detail,
                cells: values,
                selected: false,
            });
            row_index += 1;
        }

        let schema_columns = columns
            .iter()
            .map(|column| DbSchemaColumnVm {
                name: column.name.clone(),
                type_label: column.type_label.clone(),
                flags_text: [
                    column.primary_key.then_some("pk"),
                    (!column.nullable).then_some("required"),
                    column.hidden.then_some("hidden"),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · "),
                hidden: column.hidden,
            })
            .collect::<Vec<_>>();
        let result_columns = columns
            .iter()
            .filter(|column| !column.hidden)
            .map(|column| DbColumnHeaderVm {
                name: column.name.clone(),
                type_label: column.type_label.clone(),
            })
            .collect::<Vec<_>>();

        Ok(DbBrowseVm {
            schema_columns,
            result_columns,
            result_rows,
            summary: format!(
                "{} rows · page {} · sorted by {} {}",
                total_rows,
                page,
                sort_column,
                sort_direction.to_lowercase()
            ),
            error: String::new(),
            page_label: format!("Page {}", page),
            filter_columns: visible_columns.clone(),
            sort_columns: visible_columns,
            row_detail: String::new(),
        })
    }

    pub fn run_read_only_query(&self, sql: &str) -> Result<DbQueryVm> {
        if !self.database_exists() {
            return Ok(DbQueryVm {
                sql: sql.to_string(),
                error: format!(
                    "Database file not found at {}",
                    self.database_path().display()
                ),
                ..DbQueryVm::default()
            });
        }

        let guarded_sql = guard_read_only_sql(sql)?;
        let conn = self.open_read_only()?;
        let mut statement = conn.prepare(&guarded_sql)?;
        let column_names = statement
            .column_names()
            .into_iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>();
        let mut rows = statement.query([])?;
        let mut previews = Vec::new();
        let mut row_count = 0usize;
        while let Some(row) = rows.next()? {
            let values = (0..column_names.len())
                .map(|index| cell_to_text(row.get_ref(index).unwrap_or(ValueRef::Null)))
                .collect::<Vec<_>>();
            previews.push(
                values
                    .iter()
                    .zip(column_names.iter())
                    .map(|(value, column)| format!("{column}={}", truncate_inline(value, 48)))
                    .collect::<Vec<_>>()
                    .join(" · "),
            );
            row_count += 1;
            if row_count >= PAGE_SIZE {
                break;
            }
        }

        Ok(DbQueryVm {
            sql: guarded_sql,
            summary: format!(
                "{} columns · {} visible rows",
                column_names.len(),
                row_count
            ),
            preview: if previews.is_empty() {
                "Query completed without visible rows.".to_string()
            } else {
                previews.join("\n\n")
            },
            error: String::new(),
        })
    }

    pub(super) fn list_tables(&self, conn: &Connection) -> Result<Vec<TableSummary>> {
        let mut statement = conn.prepare(
            r#"
            SELECT name, sql
            FROM sqlite_master
            WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
            ORDER BY name COLLATE NOCASE ASC
            "#,
        )?;
        let mut rows = statement.query([])?;
        let mut tables = Vec::new();
        while let Some(row) = rows.next()? {
            let name: String = row.get(0)?;
            let sql: Option<String> = row.get(1)?;
            tables.push(TableSummary {
                kind: if sql
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_uppercase()
                    .contains("CREATE VIRTUAL TABLE")
                {
                    "virtual".to_string()
                } else {
                    "table".to_string()
                },
                name,
            });
        }
        tables.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(tables)
    }

    fn table_columns(&self, conn: &Connection, table_name: &str) -> Result<Vec<TableColumn>> {
        let sql = format!("PRAGMA table_xinfo({})", quote_sql_string(table_name));
        let mut statement = conn.prepare(&sql)?;
        let mut rows = statement.query([])?;
        let mut columns = Vec::new();
        while let Some(row) = rows.next()? {
            let name: String = row.get(1)?;
            let type_label: Option<String> = row.get(2)?;
            let notnull: i64 = row.get(3)?;
            let pk: i64 = row.get(5)?;
            let hidden: i64 = row.get(6)?;
            columns.push(TableColumn {
                name,
                type_label: type_label.unwrap_or_else(|| "ANY".to_string()),
                hidden: hidden != 0,
                primary_key: pk != 0,
                nullable: notnull == 0,
            });
        }
        Ok(columns)
    }
}
