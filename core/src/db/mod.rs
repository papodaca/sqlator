mod any;
mod mysql;
mod postgres;
mod sqlite;

use crate::error::CoreError;
use crate::models::{BatchResult, BatchError, ColumnMeta, PrimaryKeyMeta, QueryEvent, SqlBatch, TableMeta,
    SchemaInfo, TableInfo, SchemaColumnInfo, FilterSpec, SortSpec, TableQueryParams, TableQueryResult};
use dashmap::DashMap;
use sqlx::{AnyPool, MySqlPool, PgPool, Row, SqlitePool};
use std::collections::HashMap;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseType {
    Postgres,
    MySql,
    Sqlite,
}

#[derive(Clone)]
pub enum DatabasePool {
    Postgres(PgPool),
    MySql(MySqlPool),
    Sqlite(SqlitePool),
    Any(AnyPool),
}

pub struct DbManager {
    pools: DashMap<String, DatabasePool>,
}

impl DbManager {
    pub fn new() -> Self {
        sqlx::any::install_default_drivers();
        Self {
            pools: DashMap::new(),
        }
    }

    pub async fn test_connection(url: &str) -> Result<String, CoreError> {
        let pool = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            create_pool_for_url(url),
        )
        .await
        .map_err(|_| CoreError {
            message: "Connection timed out after 5 seconds".into(),
            code: "TIMEOUT".into(),
        })?
        .map_err(CoreError::from)?;

        close_pool(pool).await;
        Ok("Connected successfully".to_string())
    }

    pub async fn connect(&self, connection_id: &str, url: &str) -> Result<(), CoreError> {
        if let Some((_, old_pool)) = self.pools.remove(connection_id) {
            close_pool(old_pool).await;
        }

        let pool = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            create_pool_for_url(url),
        )
        .await
        .map_err(|_| CoreError {
            message: "Connection timed out after 5 seconds".into(),
            code: "TIMEOUT".into(),
        })?
        .map_err(CoreError::from)?;

        self.pools.insert(connection_id.to_string(), pool);
        Ok(())
    }

    pub async fn disconnect(&self, connection_id: &str) {
        if let Some((_, pool)) = self.pools.remove(connection_id) {
            close_pool(pool).await;
        }
    }

    pub fn is_connected(&self, connection_id: &str) -> bool {
        self.pools.contains_key(connection_id)
    }

    pub async fn execute_query(
        &self,
        connection_id: &str,
        sql: &str,
        sender: tokio::sync::mpsc::Sender<QueryEvent>,
    ) -> Result<(), CoreError> {
        let pool = self
            .pools
            .get(connection_id)
            .ok_or_else(|| CoreError {
                message: "Not connected".into(),
                code: "NO_CONNECTION".into(),
            })?
            .clone();

        let start = Instant::now();
        let sql_trimmed = sql.trim();

        let is_select = sql_trimmed
            .split_whitespace()
            .next()
            .map(|w| {
                let upper = w.to_uppercase();
                upper == "SELECT"
                    || upper == "WITH"
                    || upper == "EXPLAIN"
                    || upper == "SHOW"
                    || upper == "DESCRIBE"
            })
            .unwrap_or(false);

        match pool {
            DatabasePool::Postgres(p) => {
                if is_select {
                    postgres::execute_select(&p, sql_trimmed, sender, start).await
                } else {
                    execute_statement_pg(&p, sql_trimmed, sender, start).await
                }
            }
            DatabasePool::MySql(p) => {
                if is_select {
                    mysql::execute_select(&p, sql_trimmed, sender, start).await
                } else {
                    execute_statement_mysql(&p, sql_trimmed, sender, start).await
                }
            }
            DatabasePool::Sqlite(p) => {
                if is_select {
                    sqlite::execute_select(&p, sql_trimmed, sender, start).await
                } else {
                    execute_statement_sqlite(&p, sql_trimmed, sender, start).await
                }
            }
            DatabasePool::Any(p) => {
                if is_select {
                    any::execute_select(&p, sql_trimmed, sender, start).await
                } else {
                    execute_statement_any(&p, sql_trimmed, sender, start).await
                }
            }
        }
    }

    pub async fn fetch_schema_metadata(
        &self,
        connection_id: &str,
        table_name: &str,
        schema_name: Option<&str>,
    ) -> Result<TableMeta, CoreError> {
        let pool = self
            .pools
            .get(connection_id)
            .ok_or_else(|| CoreError { message: "Not connected".into(), code: "NO_CONNECTION".into() })?
            .clone();

        match pool {
            DatabasePool::Postgres(p) => fetch_schema_postgres(&p, table_name, schema_name).await,
            DatabasePool::MySql(p) => fetch_schema_mysql(&p, table_name).await,
            DatabasePool::Sqlite(p) => fetch_schema_sqlite(&p, table_name).await,
            DatabasePool::Any(_) => Err(CoreError {
                message: "Schema metadata not supported for this connection type".into(),
                code: "UNSUPPORTED".into(),
            }),
        }
    }

    pub async fn get_schemas(
        &self,
        connection_id: &str,
    ) -> Result<Vec<SchemaInfo>, CoreError> {
        let pool = self.pools.get(connection_id)
            .ok_or_else(|| CoreError { message: "Not connected".into(), code: "NO_CONNECTION".into() })?
            .clone();
        match pool {
            DatabasePool::Postgres(p) => get_schemas_postgres(&p).await,
            DatabasePool::MySql(p) => get_schemas_mysql(&p).await,
            DatabasePool::Sqlite(_) => Ok(vec![SchemaInfo { name: "main".into(), is_default: true }]),
            DatabasePool::Any(_) => Err(CoreError {
                message: "Schema browsing not supported for this connection type".into(),
                code: "UNSUPPORTED".into(),
            }),
        }
    }

    pub async fn get_tables(
        &self,
        connection_id: &str,
        schema: Option<&str>,
    ) -> Result<Vec<TableInfo>, CoreError> {
        let pool = self.pools.get(connection_id)
            .ok_or_else(|| CoreError { message: "Not connected".into(), code: "NO_CONNECTION".into() })?
            .clone();
        match pool {
            DatabasePool::Postgres(p) => get_tables_postgres(&p, schema).await,
            DatabasePool::MySql(p) => get_tables_mysql(&p, schema).await,
            DatabasePool::Sqlite(p) => get_tables_sqlite(&p).await,
            DatabasePool::Any(_) => Err(CoreError {
                message: "Schema browsing not supported for this connection type".into(),
                code: "UNSUPPORTED".into(),
            }),
        }
    }

    pub async fn get_columns(
        &self,
        connection_id: &str,
        table_name: &str,
        schema: Option<&str>,
    ) -> Result<Vec<SchemaColumnInfo>, CoreError> {
        let pool = self.pools.get(connection_id)
            .ok_or_else(|| CoreError { message: "Not connected".into(), code: "NO_CONNECTION".into() })?
            .clone();
        match pool {
            DatabasePool::Postgres(p) => get_columns_postgres(&p, table_name, schema).await,
            DatabasePool::MySql(p) => get_columns_mysql(&p, table_name).await,
            DatabasePool::Sqlite(p) => get_columns_sqlite(&p, table_name).await,
            DatabasePool::Any(_) => Err(CoreError {
                message: "Schema browsing not supported for this connection type".into(),
                code: "UNSUPPORTED".into(),
            }),
        }
    }

    pub async fn query_table(
        &self,
        connection_id: &str,
        params: &TableQueryParams,
    ) -> Result<TableQueryResult, CoreError> {
        let pool = self.pools.get(connection_id)
            .ok_or_else(|| CoreError { message: "Not connected".into(), code: "NO_CONNECTION".into() })?
            .clone();

        // First fetch column info so we can validate sort/filter column names
        let columns_info = match &pool {
            DatabasePool::Postgres(p) => get_columns_postgres(p, &params.table_name, params.schema.as_deref()).await?,
            DatabasePool::MySql(p) => get_columns_mysql(p, &params.table_name).await?,
            DatabasePool::Sqlite(p) => get_columns_sqlite(p, &params.table_name).await?,
            DatabasePool::Any(_) => return Err(CoreError {
                message: "Table query not supported for this connection type".into(),
                code: "UNSUPPORTED".into(),
            }),
        };

        let valid_columns: Vec<&str> = columns_info.iter().map(|c| c.name.as_str()).collect();
        let col_names: Vec<String> = columns_info.iter().map(|c| c.name.clone()).collect();
        let col_types: Vec<String> = columns_info.iter().map(|c| c.data_type.clone()).collect();

        match pool {
            DatabasePool::Postgres(p) => query_table_postgres(&p, params, &valid_columns, col_names, col_types).await,
            DatabasePool::MySql(p) => query_table_mysql(&p, params, &valid_columns, col_names, col_types).await,
            DatabasePool::Sqlite(p) => query_table_sqlite(&p, params, &valid_columns, col_names, col_types).await,
            DatabasePool::Any(_) => unreachable!(),
        }
    }

    pub async fn execute_batch(
        &self,
        connection_id: &str,
        batch: &SqlBatch,
    ) -> Result<BatchResult, CoreError> {
        let pool = self
            .pools
            .get(connection_id)
            .ok_or_else(|| CoreError { message: "Not connected".into(), code: "NO_CONNECTION".into() })?
            .clone();

        match pool {
            DatabasePool::Postgres(p) => execute_batch_postgres(&p, batch).await,
            DatabasePool::MySql(p) => execute_batch_mysql(&p, batch).await,
            DatabasePool::Sqlite(p) => execute_batch_sqlite(&p, batch).await,
            DatabasePool::Any(p) => execute_batch_any(&p, batch).await,
        }
    }
}

impl Default for DbManager {
    fn default() -> Self {
        Self::new()
    }
}

pub fn detect_database_type(url: &str) -> Option<DatabaseType> {
    let scheme = url.split("://").next()?;
    match scheme {
        "postgres" | "postgresql" => Some(DatabaseType::Postgres),
        "mysql" | "mariadb" => Some(DatabaseType::MySql),
        "sqlite" => Some(DatabaseType::Sqlite),
        _ => None,
    }
}

async fn create_pool_for_url(url: &str) -> Result<DatabasePool, sqlx::Error> {
    match detect_database_type(url) {
        Some(DatabaseType::Postgres) => {
            let pool = PgPool::connect(url).await?;
            Ok(DatabasePool::Postgres(pool))
        }
        Some(DatabaseType::MySql) => {
            let pool = MySqlPool::connect(url).await?;
            Ok(DatabasePool::MySql(pool))
        }
        Some(DatabaseType::Sqlite) => {
            let pool = SqlitePool::connect(url).await?;
            Ok(DatabasePool::Sqlite(pool))
        }
        None => {
            let pool = AnyPool::connect(url).await?;
            Ok(DatabasePool::Any(pool))
        }
    }
}

async fn close_pool(pool: DatabasePool) {
    match pool {
        DatabasePool::Postgres(p) => p.close().await,
        DatabasePool::MySql(p) => p.close().await,
        DatabasePool::Sqlite(p) => p.close().await,
        DatabasePool::Any(p) => p.close().await,
    }
}

async fn execute_statement_pg(
    pool: &PgPool,
    sql: &str,
    sender: tokio::sync::mpsc::Sender<QueryEvent>,
    start: Instant,
) -> Result<(), CoreError> {
    match sqlx::query(sql).execute(pool).await {
        Ok(result) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let _ = sender
                .send(QueryEvent::RowsAffected {
                    count: result.rows_affected(),
                    duration_ms,
                })
                .await;
        }
        Err(e) => {
            let _ = sender.send(QueryEvent::Error { message: e.to_string() }).await;
        }
    }
    Ok(())
}

async fn execute_statement_mysql(
    pool: &MySqlPool,
    sql: &str,
    sender: tokio::sync::mpsc::Sender<QueryEvent>,
    start: Instant,
) -> Result<(), CoreError> {
    match sqlx::query(sql).execute(pool).await {
        Ok(result) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let _ = sender
                .send(QueryEvent::RowsAffected {
                    count: result.rows_affected(),
                    duration_ms,
                })
                .await;
        }
        Err(e) => {
            let _ = sender.send(QueryEvent::Error { message: e.to_string() }).await;
        }
    }
    Ok(())
}

async fn execute_statement_sqlite(
    pool: &SqlitePool,
    sql: &str,
    sender: tokio::sync::mpsc::Sender<QueryEvent>,
    start: Instant,
) -> Result<(), CoreError> {
    match sqlx::query(sql).execute(pool).await {
        Ok(result) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let _ = sender
                .send(QueryEvent::RowsAffected {
                    count: result.rows_affected(),
                    duration_ms,
                })
                .await;
        }
        Err(e) => {
            let _ = sender.send(QueryEvent::Error { message: e.to_string() }).await;
        }
    }
    Ok(())
}

// ── Schema browser helpers ─────────────────────────────────────────────────────

async fn get_schemas_postgres(pool: &PgPool) -> Result<Vec<SchemaInfo>, CoreError> {
    let rows = sqlx::query(
        r#"SELECT schema_name,
            (schema_name = current_schema()) AS is_default
        FROM information_schema.schemata
        WHERE schema_name NOT IN ('information_schema', 'pg_catalog', 'pg_toast', 'pg_temp_1', 'pg_toast_temp_1')
            AND schema_name NOT LIKE 'pg_%'
        ORDER BY is_default DESC, schema_name"#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    Ok(rows.iter().map(|r| SchemaInfo {
        name: r.get("schema_name"),
        is_default: r.try_get::<bool, _>("is_default").unwrap_or(false),
    }).collect())
}

async fn get_schemas_mysql(pool: &MySqlPool) -> Result<Vec<SchemaInfo>, CoreError> {
    let rows = sqlx::query(
        r#"SELECT SCHEMA_NAME,
            (SCHEMA_NAME = DATABASE()) AS is_default
        FROM information_schema.SCHEMATA
        WHERE SCHEMA_NAME NOT IN ('information_schema', 'performance_schema', 'mysql', 'sys')
        ORDER BY is_default DESC, SCHEMA_NAME"#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    Ok(rows.iter().map(|r| SchemaInfo {
        name: r.get("SCHEMA_NAME"),
        is_default: r.try_get::<i64, _>("is_default").map(|v| v != 0).unwrap_or(false),
    }).collect())
}

async fn get_tables_postgres(pool: &PgPool, schema: Option<&str>) -> Result<Vec<TableInfo>, CoreError> {
    let schema = schema.unwrap_or("public");
    let rows = sqlx::query(
        r#"SELECT table_name, table_type
        FROM information_schema.tables
        WHERE table_schema = $1
            AND table_type IN ('BASE TABLE', 'VIEW')
        ORDER BY table_name"#,
    )
    .bind(schema)
    .fetch_all(pool)
    .await
    .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    Ok(rows.iter().map(|r| {
        let name: String = r.get("table_name");
        let raw_type: String = r.get("table_type");
        let table_type = if raw_type == "VIEW" { "view".into() } else { "table".into() };
        TableInfo {
            full_name: format!("{}.{}", schema, name),
            name,
            schema: Some(schema.into()),
            table_type,
        }
    }).collect())
}

async fn get_tables_mysql(pool: &MySqlPool, schema: Option<&str>) -> Result<Vec<TableInfo>, CoreError> {
    let (sql, schema_val) = if let Some(s) = schema {
        (
            r#"SELECT TABLE_NAME, TABLE_TYPE, TABLE_SCHEMA
            FROM information_schema.TABLES
            WHERE TABLE_SCHEMA = ? AND TABLE_TYPE IN ('BASE TABLE', 'VIEW')
            ORDER BY TABLE_NAME"#,
            s.to_string(),
        )
    } else {
        (
            r#"SELECT TABLE_NAME, TABLE_TYPE, TABLE_SCHEMA
            FROM information_schema.TABLES
            WHERE TABLE_SCHEMA = DATABASE() AND TABLE_TYPE IN ('BASE TABLE', 'VIEW')
            ORDER BY TABLE_NAME"#,
            String::new(),
        )
    };

    let rows = if schema.is_some() {
        sqlx::query(sql).bind(&schema_val).fetch_all(pool).await
    } else {
        sqlx::query(sql).fetch_all(pool).await
    }
    .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    Ok(rows.iter().map(|r| {
        let name: String = r.get("TABLE_NAME");
        let raw_type: String = r.get("TABLE_TYPE");
        let schema_name: String = r.get("TABLE_SCHEMA");
        let table_type = if raw_type == "VIEW" { "view".into() } else { "table".into() };
        TableInfo {
            full_name: format!("`{}`.`{}`", schema_name, name),
            name,
            schema: Some(schema_name),
            table_type,
        }
    }).collect())
}

async fn get_tables_sqlite(pool: &SqlitePool) -> Result<Vec<TableInfo>, CoreError> {
    let rows = sqlx::query(
        r#"SELECT name, type FROM sqlite_master
        WHERE type IN ('table', 'view')
            AND name NOT LIKE 'sqlite_%'
        ORDER BY name"#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    Ok(rows.iter().map(|r| {
        let name: String = r.get("name");
        let table_type: String = r.get("type");
        TableInfo {
            full_name: format!("\"{}\"", name),
            name: name.clone(),
            schema: None,
            table_type,
        }
    }).collect())
}

async fn get_columns_postgres(
    pool: &PgPool,
    table_name: &str,
    schema: Option<&str>,
) -> Result<Vec<SchemaColumnInfo>, CoreError> {
    let schema = schema.unwrap_or("public");

    let rows = sqlx::query(
        r#"SELECT
            c.column_name,
            c.data_type,
            c.is_nullable,
            c.column_default,
            c.ordinal_position,
            CASE WHEN pk.column_name IS NOT NULL THEN true ELSE false END AS is_primary_key,
            CASE WHEN fk.column_name IS NOT NULL THEN true ELSE false END AS is_foreign_key,
            fk.foreign_table_name,
            fk.foreign_column_name
        FROM information_schema.columns c
        LEFT JOIN (
            SELECT kcu.column_name
            FROM information_schema.table_constraints tc
            JOIN information_schema.key_column_usage kcu
                ON tc.constraint_name = kcu.constraint_name
                AND tc.table_schema = kcu.table_schema
            WHERE tc.constraint_type = 'PRIMARY KEY'
                AND tc.table_schema = $1 AND tc.table_name = $2
        ) pk ON pk.column_name = c.column_name
        LEFT JOIN (
            SELECT kcu.column_name, ccu.table_name AS foreign_table_name, ccu.column_name AS foreign_column_name
            FROM information_schema.table_constraints tc
            JOIN information_schema.key_column_usage kcu
                ON tc.constraint_name = kcu.constraint_name
                AND tc.table_schema = kcu.table_schema
            JOIN information_schema.constraint_column_usage ccu
                ON ccu.constraint_name = tc.constraint_name
            WHERE tc.constraint_type = 'FOREIGN KEY'
                AND tc.table_schema = $1 AND tc.table_name = $2
        ) fk ON fk.column_name = c.column_name
        WHERE c.table_schema = $1 AND c.table_name = $2
        ORDER BY c.ordinal_position"#,
    )
    .bind(schema)
    .bind(table_name)
    .fetch_all(pool)
    .await
    .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    Ok(rows.iter().map(|r| SchemaColumnInfo {
        name: r.get("column_name"),
        data_type: map_pg_type(&r.get::<String, _>("data_type")),
        nullable: r.get::<&str, _>("is_nullable") == "YES",
        default_value: r.get("column_default"),
        is_primary_key: r.try_get("is_primary_key").unwrap_or(false),
        is_foreign_key: r.try_get("is_foreign_key").unwrap_or(false),
        foreign_table: r.try_get("foreign_table_name").ok().flatten(),
        foreign_column: r.try_get("foreign_column_name").ok().flatten(),
        ordinal_position: r.get::<i32, _>("ordinal_position"),
    }).collect())
}

async fn get_columns_mysql(pool: &MySqlPool, table_name: &str) -> Result<Vec<SchemaColumnInfo>, CoreError> {
    let col_rows = sqlx::query(
        r#"SELECT
            c.COLUMN_NAME, c.DATA_TYPE, c.IS_NULLABLE, c.COLUMN_DEFAULT, c.ORDINAL_POSITION,
            (c.COLUMN_KEY = 'PRI') AS is_primary_key,
            (c.COLUMN_KEY = 'MUL') AS is_foreign_key
        FROM information_schema.COLUMNS c
        WHERE c.TABLE_SCHEMA = DATABASE() AND c.TABLE_NAME = ?
        ORDER BY c.ORDINAL_POSITION"#,
    )
    .bind(table_name)
    .fetch_all(pool)
    .await
    .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    // Fetch FK references
    let fk_rows = sqlx::query(
        r#"SELECT COLUMN_NAME, REFERENCED_TABLE_NAME, REFERENCED_COLUMN_NAME
        FROM information_schema.KEY_COLUMN_USAGE
        WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ?
            AND REFERENCED_TABLE_NAME IS NOT NULL"#,
    )
    .bind(table_name)
    .fetch_all(pool)
    .await
    .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    let fk_map: HashMap<String, (String, String)> = fk_rows.iter().filter_map(|r| {
        let col: String = r.get("COLUMN_NAME");
        let ref_table: Option<String> = r.get("REFERENCED_TABLE_NAME");
        let ref_col: Option<String> = r.get("REFERENCED_COLUMN_NAME");
        if let (Some(t), Some(c)) = (ref_table, ref_col) {
            Some((col, (t, c)))
        } else {
            None
        }
    }).collect();

    Ok(col_rows.iter().map(|r| {
        let col_name: String = r.get("COLUMN_NAME");
        let fk = fk_map.get(&col_name);
        SchemaColumnInfo {
            name: col_name.clone(),
            data_type: map_mysql_type(&r.get::<String, _>("DATA_TYPE")),
            nullable: r.get::<&str, _>("IS_NULLABLE") == "YES",
            default_value: r.get("COLUMN_DEFAULT"),
            is_primary_key: r.try_get::<i64, _>("is_primary_key").map(|v| v != 0).unwrap_or(false),
            is_foreign_key: fk.is_some(),
            foreign_table: fk.map(|(t, _)| t.clone()),
            foreign_column: fk.map(|(_, c)| c.clone()),
            ordinal_position: r.get::<i32, _>("ORDINAL_POSITION"),
        }
    }).collect())
}

async fn get_columns_sqlite(pool: &SqlitePool, table_name: &str) -> Result<Vec<SchemaColumnInfo>, CoreError> {
    let safe_name = table_name.replace('"', "\"\"");
    let pragma_sql = format!("PRAGMA table_info(\"{}\")", safe_name);
    let rows = sqlx::query(&pragma_sql)
        .fetch_all(pool)
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    let fk_sql = format!("PRAGMA foreign_key_list(\"{}\")", safe_name);
    let fk_rows = sqlx::query(&fk_sql)
        .fetch_all(pool)
        .await
        .unwrap_or_default();

    let fk_map: HashMap<String, (String, String)> = fk_rows.iter().filter_map(|r| {
        let from: String = r.try_get("from").ok()?;
        let table: String = r.try_get("table").ok()?;
        let to: String = r.try_get("to").ok()?;
        Some((from, (table, to)))
    }).collect();

    Ok(rows.iter().enumerate().map(|(i, r)| {
        let name: String = r.get("name");
        let type_str: String = r.try_get("type").unwrap_or_default();
        let pk_order: i64 = r.try_get("pk").unwrap_or(0);
        let notnull: i64 = r.try_get("notnull").unwrap_or(0);
        let fk = fk_map.get(&name);
        SchemaColumnInfo {
            name: name.clone(),
            data_type: map_sqlite_type(&type_str),
            nullable: notnull == 0,
            default_value: r.try_get::<Option<String>, _>("dflt_value").unwrap_or(None),
            is_primary_key: pk_order > 0,
            is_foreign_key: fk.is_some(),
            foreign_table: fk.map(|(t, _)| t.clone()),
            foreign_column: fk.map(|(_, c)| c.clone()),
            ordinal_position: (i + 1) as i32,
        }
    }).collect())
}

// ── Query Table helpers ────────────────────────────────────────────────────────

fn validate_column(name: &str, valid: &[&str]) -> bool {
    valid.contains(&name)
}

fn build_order_by_pg(sort: &[SortSpec], valid: &[&str]) -> String {
    if sort.is_empty() { return String::new(); }
    let parts: Vec<String> = sort.iter()
        .filter(|s| validate_column(&s.column, valid))
        .map(|s| format!("\"{}\" {}", s.column.replace('"', "\"\""), if s.desc { "DESC NULLS LAST" } else { "ASC NULLS LAST" }))
        .collect();
    if parts.is_empty() { return String::new(); }
    format!(" ORDER BY {}", parts.join(", "))
}

fn build_order_by_generic(sort: &[SortSpec], valid: &[&str], quote: char) -> String {
    if sort.is_empty() { return String::new(); }
    let q = quote;
    let parts: Vec<String> = sort.iter()
        .filter(|s| validate_column(&s.column, valid))
        .map(|s| format!("{}{}{} {}", q, s.column.replace(q, &format!("{}{}", q, q)), q, if s.desc { "DESC" } else { "ASC" }))
        .collect();
    if parts.is_empty() { return String::new(); }
    format!(" ORDER BY {}", parts.join(", "))
}

// Build WHERE clause with positional placeholders, returns (clause, values)
fn build_where_clause(
    filters: &[FilterSpec],
    valid: &[&str],
    placeholder_start: usize,
    positional: bool, // true for PG ($1), false for ?
) -> (String, Vec<serde_json::Value>) {
    let active: Vec<&FilterSpec> = filters.iter()
        .filter(|f| validate_column(&f.column, valid))
        .collect();

    if active.is_empty() { return (String::new(), vec![]); }

    let mut parts = Vec::new();
    let mut values: Vec<serde_json::Value> = Vec::new();
    let mut idx = placeholder_start;

    for f in active {
        let col = format!("\"{}\"", f.column.replace('"', "\"\""));
        match f.operator.as_str() {
            "isNull" => parts.push(format!("{} IS NULL", col)),
            "isNotNull" => parts.push(format!("{} IS NOT NULL", col)),
            _ => {
                let Some(val) = &f.value else { continue };
                let ph = if positional { format!("${}", idx) } else { "?".into() };
                match f.operator.as_str() {
                    "contains" => {
                        let like_val = format!("%{}%", val_to_str(val));
                        parts.push(format!("{} ILIKE {}", col, ph));
                        values.push(serde_json::Value::String(like_val));
                    }
                    "startsWith" => {
                        let like_val = format!("{}%", val_to_str(val));
                        parts.push(format!("{} ILIKE {}", col, ph));
                        values.push(serde_json::Value::String(like_val));
                    }
                    "endsWith" => {
                        let like_val = format!("%{}", val_to_str(val));
                        parts.push(format!("{} ILIKE {}", col, ph));
                        values.push(serde_json::Value::String(like_val));
                    }
                    "equals" => {
                        parts.push(format!("{} = {}", col, ph));
                        values.push(val.clone());
                    }
                    "gt" => {
                        parts.push(format!("{} > {}", col, ph));
                        values.push(val.clone());
                    }
                    "gte" => {
                        parts.push(format!("{} >= {}", col, ph));
                        values.push(val.clone());
                    }
                    "lt" => {
                        parts.push(format!("{} < {}", col, ph));
                        values.push(val.clone());
                    }
                    "lte" => {
                        parts.push(format!("{} <= {}", col, ph));
                        values.push(val.clone());
                    }
                    _ => continue,
                }
                idx += 1;
            }
        }
    }

    if parts.is_empty() { return (String::new(), vec![]); }
    (format!(" WHERE {}", parts.join(" AND ")), values)
}

// Non-ILIKE version for MySQL/SQLite
fn build_where_clause_like(
    filters: &[FilterSpec],
    valid: &[&str],
    quote: char,
) -> (String, Vec<serde_json::Value>) {
    let active: Vec<&FilterSpec> = filters.iter()
        .filter(|f| validate_column(&f.column, valid))
        .collect();

    if active.is_empty() { return (String::new(), vec![]); }

    let q = quote;
    let mut parts = Vec::new();
    let mut values: Vec<serde_json::Value> = Vec::new();

    for f in active {
        let col = format!("{}{}{}", q, f.column.replace(q, &format!("{}{}", q, q)), q);
        match f.operator.as_str() {
            "isNull" => parts.push(format!("{} IS NULL", col)),
            "isNotNull" => parts.push(format!("{} IS NOT NULL", col)),
            _ => {
                let Some(val) = &f.value else { continue };
                match f.operator.as_str() {
                    "contains" => {
                        parts.push(format!("{} LIKE ?", col));
                        values.push(serde_json::Value::String(format!("%{}%", val_to_str(val))));
                    }
                    "startsWith" => {
                        parts.push(format!("{} LIKE ?", col));
                        values.push(serde_json::Value::String(format!("{}%", val_to_str(val))));
                    }
                    "endsWith" => {
                        parts.push(format!("{} LIKE ?", col));
                        values.push(serde_json::Value::String(format!("%{}", val_to_str(val))));
                    }
                    "equals" => { parts.push(format!("{} = ?", col)); values.push(val.clone()); }
                    "gt" => { parts.push(format!("{} > ?", col)); values.push(val.clone()); }
                    "gte" => { parts.push(format!("{} >= ?", col)); values.push(val.clone()); }
                    "lt" => { parts.push(format!("{} < ?", col)); values.push(val.clone()); }
                    "lte" => { parts.push(format!("{} <= ?", col)); values.push(val.clone()); }
                    _ => continue,
                }
            }
        }
    }

    if parts.is_empty() { return (String::new(), vec![]); }
    (format!(" WHERE {}", parts.join(" AND ")), values)
}

fn val_to_str(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        _ => v.to_string(),
    }
}

async fn query_table_postgres(
    pool: &PgPool,
    params: &TableQueryParams,
    valid_columns: &[&str],
    col_names: Vec<String>,
    col_types: Vec<String>,
) -> Result<TableQueryResult, CoreError> {
    let schema = params.schema.as_deref().unwrap_or("public");
    let table_quoted = format!("\"{}\".\"{}\"",
        schema.replace('"', "\"\""),
        params.table_name.replace('"', "\"\""));

    let (where_clause, filter_vals) = build_where_clause(&params.filters, valid_columns, 1, true);
    let order_clause = build_order_by_pg(&params.sort, valid_columns);
    let limit = params.limit.min(1000) + 1;
    let ph_limit = filter_vals.len() + 1;
    let ph_offset = filter_vals.len() + 2;

    let sql = format!(
        "SELECT * FROM {}{}{} LIMIT ${} OFFSET ${}",
        table_quoted, where_clause, order_clause, ph_limit, ph_offset
    );

    let mut q = sqlx::query(&sql);
    for val in &filter_vals {
        q = match val {
            serde_json::Value::Null => q.bind(None::<String>),
            serde_json::Value::Bool(b) => q.bind(*b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() { q.bind(i) }
                else if let Some(f) = n.as_f64() { q.bind(f) }
                else { q.bind(n.to_string()) }
            }
            serde_json::Value::String(s) => q.bind(s.clone()),
            _ => q.bind(val.to_string()),
        };
    }
    q = q.bind(limit).bind(params.offset);

    let rows = q.fetch_all(pool).await
        .map_err(|e| CoreError { message: e.to_string(), code: "QUERY_TABLE".into() })?;

    let has_more = rows.len() as i64 > params.limit.min(1000);
    let rows_to_use = if has_more { &rows[..rows.len()-1] } else { &rows[..] };

    let result_rows: Vec<serde_json::Value> = rows_to_use.iter().map(|row| {
        let mut obj = serde_json::Map::new();
        for (i, col) in col_names.iter().enumerate() {
            obj.insert(col.clone(), postgres::pg_row_to_json(row, i));
        }
        serde_json::Value::Object(obj)
    }).collect();

    Ok(TableQueryResult {
        columns: col_names,
        column_types: col_types,
        rows: result_rows,
        has_more,
        total_returned: rows_to_use.len(),
    })
}

async fn query_table_mysql(
    pool: &MySqlPool,
    params: &TableQueryParams,
    valid_columns: &[&str],
    col_names: Vec<String>,
    col_types: Vec<String>,
) -> Result<TableQueryResult, CoreError> {
    let schema = params.schema.as_deref().unwrap_or("");
    let table_quoted = if schema.is_empty() {
        format!("`{}`", params.table_name.replace('`', "``"))
    } else {
        format!("`{}`.`{}`", schema.replace('`', "``"), params.table_name.replace('`', "``"))
    };

    let (where_clause, filter_vals) = build_where_clause_like(&params.filters, valid_columns, '`');
    let order_clause = build_order_by_generic(&params.sort, valid_columns, '`');
    let limit = params.limit.min(1000) + 1;

    let sql = format!("SELECT * FROM {}{}{} LIMIT ? OFFSET ?", table_quoted, where_clause, order_clause);

    let mut q = sqlx::query(&sql);
    for val in &filter_vals {
        q = match val {
            serde_json::Value::Null => q.bind(None::<String>),
            serde_json::Value::Bool(b) => q.bind(*b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() { q.bind(i) }
                else if let Some(f) = n.as_f64() { q.bind(f) }
                else { q.bind(n.to_string()) }
            }
            serde_json::Value::String(s) => q.bind(s.clone()),
            _ => q.bind(val.to_string()),
        };
    }
    q = q.bind(limit).bind(params.offset);

    let rows = q.fetch_all(pool).await
        .map_err(|e| CoreError { message: e.to_string(), code: "QUERY_TABLE".into() })?;

    let has_more = rows.len() as i64 > params.limit.min(1000);
    let rows_to_use = if has_more { &rows[..rows.len()-1] } else { &rows[..] };

    let result_rows: Vec<serde_json::Value> = rows_to_use.iter().map(|row| {
        let mut obj = serde_json::Map::new();
        for (i, col) in col_names.iter().enumerate() {
            obj.insert(col.clone(), mysql::mysql_row_to_json(row, i));
        }
        serde_json::Value::Object(obj)
    }).collect();

    Ok(TableQueryResult {
        columns: col_names,
        column_types: col_types,
        rows: result_rows,
        has_more,
        total_returned: rows_to_use.len(),
    })
}

async fn query_table_sqlite(
    pool: &SqlitePool,
    params: &TableQueryParams,
    valid_columns: &[&str],
    col_names: Vec<String>,
    col_types: Vec<String>,
) -> Result<TableQueryResult, CoreError> {
    let table_quoted = format!("\"{}\"", params.table_name.replace('"', "\"\""));

    let (where_clause, filter_vals) = build_where_clause_like(&params.filters, valid_columns, '"');
    let order_clause = build_order_by_generic(&params.sort, valid_columns, '"');
    let limit = params.limit.min(1000) + 1;

    let sql = format!("SELECT * FROM {}{}{} LIMIT ? OFFSET ?", table_quoted, where_clause, order_clause);

    let mut q = sqlx::query(&sql);
    for val in &filter_vals {
        q = match val {
            serde_json::Value::Null => q.bind(None::<String>),
            serde_json::Value::Bool(b) => q.bind(*b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() { q.bind(i) }
                else if let Some(f) = n.as_f64() { q.bind(f) }
                else { q.bind(n.to_string()) }
            }
            serde_json::Value::String(s) => q.bind(s.clone()),
            _ => q.bind(val.to_string()),
        };
    }
    q = q.bind(limit).bind(params.offset);

    let rows = q.fetch_all(pool).await
        .map_err(|e| CoreError { message: e.to_string(), code: "QUERY_TABLE".into() })?;

    let has_more = rows.len() as i64 > params.limit.min(1000);
    let rows_to_use = if has_more { &rows[..rows.len()-1] } else { &rows[..] };

    let result_rows: Vec<serde_json::Value> = rows_to_use.iter().map(|row| {
        let mut obj = serde_json::Map::new();
        for (i, col) in col_names.iter().enumerate() {
            obj.insert(col.clone(), sqlite::sqlite_row_to_json(row, i));
        }
        serde_json::Value::Object(obj)
    }).collect();

    Ok(TableQueryResult {
        columns: col_names,
        column_types: col_types,
        rows: result_rows,
        has_more,
        total_returned: rows_to_use.len(),
    })
}

// ── Schema metadata helpers ────────────────────────────────────────────────────

fn map_pg_type(type_name: &str) -> String {
    match type_name.to_lowercase().as_str() {
        "integer" | "int4" | "int" => "integer".into(),
        "bigint" | "int8" => "bigint".into(),
        "smallint" | "int2" => "smallint".into(),
        "numeric" | "decimal" => "decimal".into(),
        "real" | "float4" => "float".into(),
        "double precision" | "float8" => "double".into(),
        "character varying" | "varchar" => "varchar".into(),
        "text" => "text".into(),
        "character" | "char" | "bpchar" => "char".into(),
        "boolean" | "bool" => "boolean".into(),
        "date" => "date".into(),
        "time" | "time without time zone" | "time with time zone" => "time".into(),
        "timestamp" | "timestamp without time zone" => "timestamp".into(),
        "timestamp with time zone" | "timestamptz" => "timestamp".into(),
        "json" => "json".into(),
        "jsonb" => "jsonb".into(),
        "uuid" => "uuid".into(),
        "USER-DEFINED" | "user-defined" => "enum".into(),
        _ => "unknown".into(),
    }
}

fn map_mysql_type(type_name: &str) -> String {
    match type_name.to_lowercase().as_str() {
        "int" | "integer" => "integer".into(),
        "bigint" => "bigint".into(),
        "smallint" | "tinyint" => "smallint".into(),
        "decimal" | "numeric" => "decimal".into(),
        "float" => "float".into(),
        "double" => "double".into(),
        "varchar" => "varchar".into(),
        "text" | "mediumtext" | "longtext" | "tinytext" => "text".into(),
        "char" => "char".into(),
        "boolean" | "bool" | "tinyint(1)" => "boolean".into(),
        "date" => "date".into(),
        "time" => "time".into(),
        "datetime" => "datetime".into(),
        "timestamp" => "timestamp".into(),
        "json" => "json".into(),
        "enum" => "enum".into(),
        _ => "unknown".into(),
    }
}

fn map_sqlite_type(type_name: &str) -> String {
    let t = type_name.to_uppercase();
    if t.contains("INT") { return "integer".into(); }
    if t.contains("CHAR") || t.contains("CLOB") || t.contains("TEXT") { return "text".into(); }
    if t.contains("BLOB") || t.is_empty() { return "unknown".into(); }
    if t.contains("REAL") || t.contains("FLOA") || t.contains("DOUB") { return "float".into(); }
    if t.contains("BOOL") { return "boolean".into(); }
    if t.contains("DATE") || t.contains("TIME") { return "timestamp".into(); }
    if t.contains("NUMERIC") || t.contains("DECIMAL") { return "decimal".into(); }
    "unknown".into()
}

async fn fetch_schema_postgres(
    pool: &PgPool,
    table_name: &str,
    schema_name: Option<&str>,
) -> Result<TableMeta, CoreError> {
    let schema = schema_name.unwrap_or("public");

    // Fetch columns
    let col_rows = sqlx::query(
        r#"SELECT
            c.column_name,
            c.data_type,
            c.is_nullable,
            c.column_default,
            COALESCE(c.is_identity = 'YES', false) AS is_identity,
            CASE WHEN c.is_generated = 'ALWAYS' THEN true ELSE false END AS is_generated
        FROM information_schema.columns c
        WHERE c.table_schema = $1 AND c.table_name = $2
        ORDER BY c.ordinal_position"#,
    )
    .bind(schema)
    .bind(table_name)
    .fetch_all(pool)
    .await
    .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    if col_rows.is_empty() {
        return Ok(TableMeta {
            table_name: table_name.to_string(),
            schema: Some(schema.to_string()),
            columns: vec![],
            primary_key: PrimaryKeyMeta { columns: vec![], exists: false },
            is_editable: false,
            editability_reason: Some("Table not found or no columns".into()),
        });
    }

    let mut columns: Vec<ColumnMeta> = col_rows
        .iter()
        .map(|r| {
            let data_type: String = r.get("data_type");
            let is_identity: bool = r.try_get("is_identity").unwrap_or(false);
            let is_generated: bool = r.get("is_generated");
            let is_updatable = !is_identity && !is_generated;
            ColumnMeta {
                name: r.get("column_name"),
                column_type: map_pg_type(&data_type),
                nullable: r.get::<&str, _>("is_nullable") == "YES",
                is_auto_increment: is_identity,
                is_generated,
                is_updatable,
                default_value: r.get("column_default"),
            }
        })
        .collect();

    // Fetch primary keys
    let pk_rows = sqlx::query(
        r#"SELECT kcu.column_name
        FROM information_schema.table_constraints tc
        JOIN information_schema.key_column_usage kcu
            ON tc.constraint_name = kcu.constraint_name
            AND tc.table_schema = kcu.table_schema
        WHERE tc.constraint_type = 'PRIMARY KEY'
            AND tc.table_schema = $1
            AND tc.table_name = $2
        ORDER BY kcu.ordinal_position"#,
    )
    .bind(schema)
    .bind(table_name)
    .fetch_all(pool)
    .await
    .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    let pk_columns: Vec<String> = pk_rows.iter().map(|r| r.get("column_name")).collect();
    let pk_exists = !pk_columns.is_empty();

    let primary_key = PrimaryKeyMeta { columns: pk_columns, exists: pk_exists };

    // Mark PK columns as not updatable
    for col in &mut columns {
        if primary_key.columns.contains(&col.name) {
            col.is_updatable = false;
        }
    }

    Ok(TableMeta {
        table_name: table_name.to_string(),
        schema: Some(schema.to_string()),
        columns,
        primary_key,
        is_editable: pk_exists,
        editability_reason: if pk_exists { None } else { Some("No primary key detected".into()) },
    })
}

async fn fetch_schema_mysql(pool: &MySqlPool, table_name: &str) -> Result<TableMeta, CoreError> {
    let col_rows = sqlx::query(
        r#"SELECT
            c.COLUMN_NAME,
            c.DATA_TYPE,
            c.IS_NULLABLE,
            c.COLUMN_DEFAULT,
            (c.EXTRA LIKE '%auto_increment%') AS is_auto_increment,
            (c.EXTRA LIKE '%GENERATED%') AS is_generated
        FROM information_schema.COLUMNS c
        WHERE c.TABLE_SCHEMA = DATABASE() AND c.TABLE_NAME = ?
        ORDER BY c.ORDINAL_POSITION"#,
    )
    .bind(table_name)
    .fetch_all(pool)
    .await
    .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    if col_rows.is_empty() {
        return Ok(TableMeta {
            table_name: table_name.to_string(),
            schema: None,
            columns: vec![],
            primary_key: PrimaryKeyMeta { columns: vec![], exists: false },
            is_editable: false,
            editability_reason: Some("Table not found or no columns".into()),
        });
    }

    let mut columns: Vec<ColumnMeta> = col_rows
        .iter()
        .map(|r| {
            let data_type: String = r.get("DATA_TYPE");
            let is_auto: bool = r.try_get::<bool, _>("is_auto_increment").unwrap_or(false);
            let is_generated: bool = r.try_get::<bool, _>("is_generated").unwrap_or(false);
            ColumnMeta {
                name: r.get("COLUMN_NAME"),
                column_type: map_mysql_type(&data_type),
                nullable: r.get::<&str, _>("IS_NULLABLE") == "YES",
                is_auto_increment: is_auto,
                is_generated,
                is_updatable: !is_auto && !is_generated,
                default_value: r.get("COLUMN_DEFAULT"),
            }
        })
        .collect();

    let pk_rows = sqlx::query(
        r#"SELECT kcu.COLUMN_NAME
        FROM information_schema.TABLE_CONSTRAINTS tc
        JOIN information_schema.KEY_COLUMN_USAGE kcu
            ON tc.CONSTRAINT_NAME = kcu.CONSTRAINT_NAME
            AND tc.TABLE_SCHEMA = kcu.TABLE_SCHEMA
        WHERE tc.CONSTRAINT_TYPE = 'PRIMARY KEY'
            AND tc.TABLE_SCHEMA = DATABASE()
            AND tc.TABLE_NAME = ?
        ORDER BY kcu.ORDINAL_POSITION"#,
    )
    .bind(table_name)
    .fetch_all(pool)
    .await
    .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    let pk_columns: Vec<String> = pk_rows.iter().map(|r| r.get("COLUMN_NAME")).collect();
    let pk_exists = !pk_columns.is_empty();
    let primary_key = PrimaryKeyMeta { columns: pk_columns, exists: pk_exists };

    for col in &mut columns {
        if primary_key.columns.contains(&col.name) {
            col.is_updatable = false;
        }
    }

    Ok(TableMeta {
        table_name: table_name.to_string(),
        schema: None,
        columns,
        primary_key,
        is_editable: pk_exists,
        editability_reason: if pk_exists { None } else { Some("No primary key detected".into()) },
    })
}

async fn fetch_schema_sqlite(pool: &SqlitePool, table_name: &str) -> Result<TableMeta, CoreError> {
    // PRAGMA table_info returns: cid, name, type, notnull, dflt_value, pk
    let pragma_sql = format!("PRAGMA table_info(\"{}\")", table_name.replace('"', "\"\""));
    let rows = sqlx::query(&pragma_sql)
        .fetch_all(pool)
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    if rows.is_empty() {
        return Ok(TableMeta {
            table_name: table_name.to_string(),
            schema: None,
            columns: vec![],
            primary_key: PrimaryKeyMeta { columns: vec![], exists: false },
            is_editable: false,
            editability_reason: Some("Table not found or no columns".into()),
        });
    }

    let mut pk_columns: Vec<(i64, String)> = vec![];
    let mut columns: Vec<ColumnMeta> = rows
        .iter()
        .map(|r| {
            let name: String = r.get("name");
            let type_str: String = r.try_get("type").unwrap_or_default();
            let notnull: bool = r.try_get::<i64, _>("notnull").unwrap_or(0) != 0;
            let pk_order: i64 = r.try_get("pk").unwrap_or(0);
            if pk_order > 0 {
                pk_columns.push((pk_order, name.clone()));
            }
            // Check for INTEGER PRIMARY KEY (SQLite rowid alias — auto-increment)
            let is_auto = type_str.to_uppercase() == "INTEGER" && pk_order > 0;
            ColumnMeta {
                name,
                column_type: map_sqlite_type(&type_str),
                nullable: !notnull,
                is_auto_increment: is_auto,
                is_generated: false,
                is_updatable: !is_auto,
                default_value: r.try_get::<Option<String>, _>("dflt_value").unwrap_or(None),
            }
        })
        .collect();

    pk_columns.sort_by_key(|(order, _)| *order);
    let pk_col_names: Vec<String> = pk_columns.into_iter().map(|(_, n)| n).collect();
    let pk_exists = !pk_col_names.is_empty();
    let primary_key = PrimaryKeyMeta { columns: pk_col_names, exists: pk_exists };

    for col in &mut columns {
        if primary_key.columns.contains(&col.name) {
            col.is_updatable = false;
        }
    }

    Ok(TableMeta {
        table_name: table_name.to_string(),
        schema: None,
        columns,
        primary_key,
        is_editable: pk_exists,
        editability_reason: if pk_exists { None } else { Some("No primary key detected".into()) },
    })
}

// ── Batch execution helpers ────────────────────────────────────────────────────

fn bind_params_to_query<'q>(
    mut q: sqlx::query::Query<'q, sqlx::Any, sqlx::any::AnyArguments<'q>>,
    params: &'q [serde_json::Value],
) -> sqlx::query::Query<'q, sqlx::Any, sqlx::any::AnyArguments<'q>> {
    for p in params {
        match p {
            serde_json::Value::Null => q = q.bind(None::<String>),
            serde_json::Value::Bool(b) => q = q.bind(*b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    q = q.bind(i);
                } else if let Some(f) = n.as_f64() {
                    q = q.bind(f);
                } else {
                    q = q.bind(n.to_string());
                }
            }
            serde_json::Value::String(s) => q = q.bind(s.as_str()),
            serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                q = q.bind(p.to_string())
            }
        }
    }
    q
}

async fn execute_batch_postgres(pool: &PgPool, batch: &SqlBatch) -> Result<BatchResult, CoreError> {
    let mut tx = pool.begin().await.map_err(|e| CoreError { message: e.to_string(), code: "TX_BEGIN".into() })?;
    let mut executed = 0;
    let mut inserted_ids: HashMap<String, serde_json::Value> = HashMap::new();
    let total = batch.statements.len();

    for stmt in &batch.statements {
        let mut q = sqlx::query(&stmt.sql);
        for p in &stmt.params {
            match p {
                serde_json::Value::Null => q = q.bind(None::<String>),
                serde_json::Value::Bool(b) => q = q.bind(*b),
                serde_json::Value::Number(n) => {
                    if let Some(i) = n.as_i64() { q = q.bind(i); }
                    else if let Some(f) = n.as_f64() { q = q.bind(f); }
                    else { q = q.bind(n.to_string()); }
                }
                serde_json::Value::String(s) => q = q.bind(s.as_str()),
                _ => q = q.bind(p.to_string()),
            }
        }

        match q.execute(&mut *tx).await {
            Ok(_) => {
                executed += 1;
            }
            Err(e) => {
                let _ = tx.rollback().await;
                let code = extract_pg_error_code(&e);
                return Ok(BatchResult {
                    success: false,
                    executed_count: executed,
                    total_statements: total,
                    error: Some(BatchError {
                        statement_index: executed,
                        message: format_db_error(&e),
                        code,
                    }),
                    inserted_ids: HashMap::new(),
                });
            }
        }
    }

    tx.commit().await.map_err(|e| CoreError { message: e.to_string(), code: "TX_COMMIT".into() })?;
    Ok(BatchResult { success: true, executed_count: executed, total_statements: total, error: None, inserted_ids })
}

async fn execute_batch_mysql(pool: &MySqlPool, batch: &SqlBatch) -> Result<BatchResult, CoreError> {
    let mut tx = pool.begin().await.map_err(|e| CoreError { message: e.to_string(), code: "TX_BEGIN".into() })?;
    let mut executed = 0;
    let total = batch.statements.len();

    for stmt in &batch.statements {
        let mut q = sqlx::query(&stmt.sql);
        for p in &stmt.params {
            match p {
                serde_json::Value::Null => q = q.bind(None::<String>),
                serde_json::Value::Bool(b) => q = q.bind(*b),
                serde_json::Value::Number(n) => {
                    if let Some(i) = n.as_i64() { q = q.bind(i); }
                    else if let Some(f) = n.as_f64() { q = q.bind(f); }
                    else { q = q.bind(n.to_string()); }
                }
                serde_json::Value::String(s) => q = q.bind(s.as_str()),
                _ => q = q.bind(p.to_string()),
            }
        }
        match q.execute(&mut *tx).await {
            Ok(_) => executed += 1,
            Err(e) => {
                let _ = tx.rollback().await;
                return Ok(BatchResult {
                    success: false,
                    executed_count: executed,
                    total_statements: total,
                    error: Some(BatchError {
                        statement_index: executed,
                        message: format_db_error(&e),
                        code: extract_mysql_error_code(&e),
                    }),
                    inserted_ids: HashMap::new(),
                });
            }
        }
    }

    tx.commit().await.map_err(|e| CoreError { message: e.to_string(), code: "TX_COMMIT".into() })?;
    Ok(BatchResult { success: true, executed_count: executed, total_statements: total, error: None, inserted_ids: HashMap::new() })
}

async fn execute_batch_sqlite(pool: &SqlitePool, batch: &SqlBatch) -> Result<BatchResult, CoreError> {
    let mut tx = pool.begin().await.map_err(|e| CoreError { message: e.to_string(), code: "TX_BEGIN".into() })?;
    let mut executed = 0;
    let total = batch.statements.len();

    for stmt in &batch.statements {
        let mut q = sqlx::query(&stmt.sql);
        for p in &stmt.params {
            match p {
                serde_json::Value::Null => q = q.bind(None::<String>),
                serde_json::Value::Bool(b) => q = q.bind(*b),
                serde_json::Value::Number(n) => {
                    if let Some(i) = n.as_i64() { q = q.bind(i); }
                    else if let Some(f) = n.as_f64() { q = q.bind(f); }
                    else { q = q.bind(n.to_string()); }
                }
                serde_json::Value::String(s) => q = q.bind(s.as_str()),
                _ => q = q.bind(p.to_string()),
            }
        }
        match q.execute(&mut *tx).await {
            Ok(_) => executed += 1,
            Err(e) => {
                let _ = tx.rollback().await;
                return Ok(BatchResult {
                    success: false,
                    executed_count: executed,
                    total_statements: total,
                    error: Some(BatchError {
                        statement_index: executed,
                        message: format_db_error(&e),
                        code: None,
                    }),
                    inserted_ids: HashMap::new(),
                });
            }
        }
    }

    tx.commit().await.map_err(|e| CoreError { message: e.to_string(), code: "TX_COMMIT".into() })?;
    Ok(BatchResult { success: true, executed_count: executed, total_statements: total, error: None, inserted_ids: HashMap::new() })
}

async fn execute_batch_any(pool: &AnyPool, batch: &SqlBatch) -> Result<BatchResult, CoreError> {
    let mut tx = pool.begin().await.map_err(|e| CoreError { message: e.to_string(), code: "TX_BEGIN".into() })?;
    let mut executed = 0;
    let total = batch.statements.len();

    for stmt in &batch.statements {
        let q = bind_params_to_query(sqlx::query(&stmt.sql), &stmt.params);
        match q.execute(&mut *tx).await {
            Ok(_) => executed += 1,
            Err(e) => {
                let _ = tx.rollback().await;
                return Ok(BatchResult {
                    success: false,
                    executed_count: executed,
                    total_statements: total,
                    error: Some(BatchError {
                        statement_index: executed,
                        message: format_db_error(&e),
                        code: None,
                    }),
                    inserted_ids: HashMap::new(),
                });
            }
        }
    }

    tx.commit().await.map_err(|e| CoreError { message: e.to_string(), code: "TX_COMMIT".into() })?;
    Ok(BatchResult { success: true, executed_count: executed, total_statements: total, error: None, inserted_ids: HashMap::new() })
}

fn format_db_error(e: &sqlx::Error) -> String {
    match e {
        sqlx::Error::Database(db_err) => {
            let msg = db_err.message();
            // Try to extract constraint name for user-friendly messages
            if let Some(constraint) = db_err.constraint() {
                format!("{} (constraint: {})", msg, constraint)
            } else {
                msg.to_string()
            }
        }
        _ => e.to_string(),
    }
}

fn extract_pg_error_code(e: &sqlx::Error) -> Option<String> {
    if let sqlx::Error::Database(db_err) = e {
        db_err.code().map(|c| c.to_string())
    } else {
        None
    }
}

fn extract_mysql_error_code(e: &sqlx::Error) -> Option<String> {
    if let sqlx::Error::Database(db_err) = e {
        db_err.code().map(|c| c.to_string())
    } else {
        None
    }
}

async fn execute_statement_any(
    pool: &AnyPool,
    sql: &str,
    sender: tokio::sync::mpsc::Sender<QueryEvent>,
    start: Instant,
) -> Result<(), CoreError> {
    match sqlx::query(sql).execute(pool).await {
        Ok(result) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let _ = sender
                .send(QueryEvent::RowsAffected {
                    count: result.rows_affected(),
                    duration_ms,
                })
                .await;
        }
        Err(e) => {
            let _ = sender.send(QueryEvent::Error { message: e.to_string() }).await;
        }
    }
    Ok(())
}
