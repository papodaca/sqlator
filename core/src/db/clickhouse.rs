/// ClickHouse database support via the HTTP JSON API.
///
/// ClickHouse exposes an HTTP interface on port 8123. We POST SQL with
/// `FORMAT JSONCompact` appended so that responses include column metadata
/// and typed values without needing a schema at compile time.
///
/// Connection URL format: clickhouse://user:pass@host:8123/database
use crate::error::CoreError;
use crate::models::{QueryEvent, SchemaColumnInfo, SchemaInfo, TableInfo};
use std::sync::Arc;
use std::time::Instant;

pub struct ClickHouseClient {
    http: reqwest::Client,
    base_url: String,
    user: String,
    password: String,
    pub database: String,
}

pub type ClickHousePool = Arc<ClickHouseClient>;

pub async fn create_pool(url: &str) -> Result<ClickHousePool, CoreError> {
    let parsed = url::Url::parse(url).map_err(|e| CoreError {
        message: format!("Invalid ClickHouse URL: {}", e),
        code: "INVALID_URL".into(),
    })?;

    let host = parsed.host_str().unwrap_or("localhost");
    let port = parsed.port().unwrap_or(8123);
    let user = parsed.username();
    let password = parsed.password().unwrap_or("");
    let database_raw = parsed.path().trim_start_matches('/');
    let database = if database_raw.is_empty() { "default" } else { database_raw };

    let http = reqwest::Client::builder()
        .build()
        .map_err(|e| CoreError { message: e.to_string(), code: "CONNECTION_FAILED".into() })?;

    let client = Arc::new(ClickHouseClient {
        http,
        base_url: format!("http://{}:{}/", host, port),
        user: user.to_string(),
        password: password.to_string(),
        database: database.to_string(),
    });

    // Test connectivity
    send_query(&client, "SELECT 1 FORMAT JSONCompact").await.map_err(|e| CoreError {
        message: format!("ClickHouse connection test failed: {}", e.message),
        code: "CONNECTION_FAILED".into(),
    })?;

    Ok(client)
}

/// POST a SQL string to ClickHouse and return the parsed JSON response body.
async fn send_query(
    client: &ClickHouseClient,
    sql: &str,
) -> Result<serde_json::Value, CoreError> {
    let response = client
        .http
        .post(&client.base_url)
        .query(&[
            ("user", client.user.as_str()),
            ("password", client.password.as_str()),
            ("database", client.database.as_str()),
        ])
        .body(sql.to_string())
        .send()
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "CONNECTION_FAILED".into() })?;

    if !response.status().is_success() {
        let msg = response.text().await.unwrap_or_else(|_| "Unknown ClickHouse error".into());
        return Err(CoreError { message: msg.trim().to_string(), code: "DATABASE_ERROR".into() });
    }

    response
        .json::<serde_json::Value>()
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "PARSE_ERROR".into() })
}

/// POST a DML/DDL statement and return (written_rows, error_body).
async fn send_statement(
    client: &ClickHouseClient,
    sql: &str,
) -> Result<u64, CoreError> {
    let response = client
        .http
        .post(&client.base_url)
        .query(&[
            ("user", client.user.as_str()),
            ("password", client.password.as_str()),
            ("database", client.database.as_str()),
            ("wait_end_of_query", "1"),
        ])
        .body(sql.to_string())
        .send()
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "CONNECTION_FAILED".into() })?;

    if !response.status().is_success() {
        let msg = response.text().await.unwrap_or_else(|_| "Unknown ClickHouse error".into());
        return Err(CoreError { message: msg.trim().to_string(), code: "DATABASE_ERROR".into() });
    }

    // X-ClickHouse-Summary header carries written_rows for DML
    let written_rows: u64 = response
        .headers()
        .get("X-ClickHouse-Summary")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|j| j["written_rows"].as_str().map(str::to_string))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    Ok(written_rows)
}

/// Append `FORMAT JSONCompact` if the query doesn't already specify a FORMAT.
fn with_json_format(sql: &str) -> String {
    let trimmed = sql.trim_end_matches(';').trim();
    if trimmed.to_uppercase().contains(" FORMAT ") {
        trimmed.to_string()
    } else {
        format!("{} FORMAT JSONCompact", trimmed)
    }
}

pub async fn execute_select(
    pool: &ClickHousePool,
    sql: &str,
    sender: tokio::sync::mpsc::Sender<QueryEvent>,
    start: Instant,
) -> Result<(), CoreError> {
    let formatted = with_json_format(sql);

    let result = match send_query(pool, &formatted).await {
        Ok(r) => r,
        Err(e) => {
            let _ = sender.send(QueryEvent::Error { message: e.message }).await;
            return Ok(());
        }
    };

    // JSONCompact: {"meta":[{"name":"col","type":"UInt32"}],"data":[[v,…],…],"rows":N}
    let meta = result["meta"].as_array().cloned().unwrap_or_default();
    let names: Vec<String> =
        meta.iter().filter_map(|m| m["name"].as_str().map(String::from)).collect();

    if !names.is_empty() {
        let _ = sender.send(QueryEvent::Columns { names }).await;
    }

    let data = result["data"].as_array().cloned().unwrap_or_default();
    let total_rows = result["rows"].as_u64().unwrap_or(data.len() as u64) as usize;
    let max_rows = 1000usize;

    for (i, row) in data.into_iter().enumerate() {
        if i >= max_rows {
            break;
        }
        let values: Vec<serde_json::Value> = match row {
            serde_json::Value::Array(arr) => arr,
            other => vec![other],
        };
        let _ = sender.send(QueryEvent::Row { values }).await;
    }

    let duration_ms = start.elapsed().as_millis() as u64;
    let _ = sender.send(QueryEvent::Done { row_count: total_rows, duration_ms }).await;
    Ok(())
}

pub async fn execute_statement(
    pool: &ClickHousePool,
    sql: &str,
    sender: tokio::sync::mpsc::Sender<QueryEvent>,
    start: Instant,
) -> Result<(), CoreError> {
    match send_statement(pool, sql).await {
        Ok(written_rows) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let _ = sender
                .send(QueryEvent::RowsAffected { count: written_rows, duration_ms })
                .await;
        }
        Err(e) => {
            let _ = sender.send(QueryEvent::Error { message: e.message }).await;
        }
    }
    Ok(())
}

pub async fn get_schemas(pool: &ClickHousePool) -> Result<Vec<SchemaInfo>, CoreError> {
    // In ClickHouse, databases are the top-level namespaces (analogous to schemas).
    // Filter out built-in system databases.
    let sql = format!(
        "SELECT name, name = currentDatabase() AS is_default \
         FROM system.databases \
         WHERE name NOT IN ('system', 'information_schema', 'INFORMATION_SCHEMA') \
         ORDER BY name \
         FORMAT JSONCompact"
    );

    let result = send_query(pool, &sql).await?;
    let data = result["data"].as_array().cloned().unwrap_or_default();

    Ok(data
        .iter()
        .filter_map(|row| {
            let arr = row.as_array()?;
            let name = arr.first()?.as_str()?.to_string();
            let is_default = arr.get(1).and_then(|v| v.as_u64()).unwrap_or(0) != 0;
            Some(SchemaInfo { name, is_default })
        })
        .collect())
}

pub async fn get_tables(
    pool: &ClickHousePool,
    schema: Option<&str>,
) -> Result<Vec<TableInfo>, CoreError> {
    let db = schema.unwrap_or(&pool.database);
    let db_escaped = escape_str(db);

    let sql = format!(
        "SELECT name, \
                multiIf(engine IN ('View','MaterializedView','LiveView','WindowView'), 'view', 'table') \
                AS table_type \
         FROM system.tables \
         WHERE database = '{db_escaped}' \
         ORDER BY name \
         FORMAT JSONCompact"
    );

    let result = send_query(pool, &sql).await?;
    let data = result["data"].as_array().cloned().unwrap_or_default();

    Ok(data
        .iter()
        .filter_map(|row| {
            let arr = row.as_array()?;
            let name = arr.first()?.as_str()?.to_string();
            let table_type = arr.get(1)?.as_str()?.to_string();
            Some(TableInfo {
                full_name: format!("{}.{}", db, name),
                name,
                schema: Some(db.to_string()),
                table_type,
            })
        })
        .collect())
}

pub async fn get_columns(
    pool: &ClickHousePool,
    table_name: &str,
    schema: Option<&str>,
) -> Result<Vec<SchemaColumnInfo>, CoreError> {
    let db = schema.unwrap_or(&pool.database);
    let db_escaped = escape_str(db);
    let tbl_escaped = escape_str(table_name);

    let sql = format!(
        "SELECT name, type, default_expression, is_in_primary_key, position \
         FROM system.columns \
         WHERE database = '{db_escaped}' AND table = '{tbl_escaped}' \
         ORDER BY position \
         FORMAT JSONCompact"
    );

    let result = send_query(pool, &sql).await?;
    let data = result["data"].as_array().cloned().unwrap_or_default();

    Ok(data
        .iter()
        .filter_map(|row| {
            let arr = row.as_array()?;
            let name = arr.first()?.as_str()?.to_string();
            let raw_type = arr.get(1)?.as_str()?.to_string();
            let default_value = arr
                .get(2)
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from);
            let is_pk = arr.get(3).and_then(|v| v.as_u64()).unwrap_or(0) != 0;
            let ordinal = arr.get(4).and_then(|v| v.as_u64()).unwrap_or(0) as i32;

            // Nullable(T) wraps the inner type; detect nullable from the type string
            let nullable = raw_type.starts_with("Nullable(");
            let data_type = map_clickhouse_type(&raw_type);

            Some(SchemaColumnInfo {
                name,
                data_type,
                nullable,
                default_value,
                is_primary_key: is_pk,
                is_foreign_key: false, // ClickHouse has no FK constraints
                foreign_table: None,
                foreign_column: None,
                ordinal_position: ordinal,
            })
        })
        .collect())
}

fn map_clickhouse_type(raw: &str) -> String {
    // Strip Nullable(...) wrapper for type mapping
    let inner = if let Some(stripped) = raw.strip_prefix("Nullable(").and_then(|s| s.strip_suffix(')')) {
        stripped
    } else {
        raw
    };

    let base = inner.split('(').next().unwrap_or(inner).trim();

    match base {
        "UInt8" | "UInt16" | "UInt32" | "UInt64" | "UInt128" | "UInt256"
        | "Int8" | "Int16" | "Int32" | "Int64" | "Int128" | "Int256" => "integer",
        "Float32" | "Float64" => "float",
        "Decimal" | "Decimal32" | "Decimal64" | "Decimal128" | "Decimal256" => "decimal",
        "String" | "FixedString" => "text",
        "Date" | "Date32" => "date",
        "DateTime" | "DateTime64" => "datetime",
        "Bool" | "Boolean" => "boolean",
        "UUID" => "uuid",
        "JSON" | "Object" => "json",
        "Array" => "array",
        "Tuple" => "tuple",
        "Map" => "map",
        "IPv4" | "IPv6" => "text",
        other => other,
    }
    .to_string()
}

/// Minimal SQL string escaping: replace `'` with `''` and `\` with `\\`.
/// Used only for schema/table names in internal metadata queries.
fn escape_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "''")
}
