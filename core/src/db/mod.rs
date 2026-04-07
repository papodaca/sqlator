mod any;
mod mysql;
mod postgres;
mod sqlite;

use crate::error::CoreError;
use crate::models::{BatchResult, BatchError, ColumnMeta, PrimaryKeyMeta, QueryEvent, SqlBatch, TableMeta};
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
