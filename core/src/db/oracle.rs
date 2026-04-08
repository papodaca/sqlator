/// Oracle database support (experimental) via oracle-rs pure-Rust TNS driver.
///
/// Requires Oracle Database 12.1+. No Oracle Instant Client or OCI libraries needed.
/// Connection URL format: oracle://user:pass@host:1521/service_name
///
/// Tested against Oracle Free 23c (gvenzl/oracle-free Docker image).
use crate::error::CoreError;
use crate::models::{QueryEvent, SchemaColumnInfo, SchemaInfo, TableInfo};
use deadpool_oracle::PoolBuilder;
use oracle_rs::{Config, LobValue, Value};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

pub type OraclePool = deadpool_oracle::Pool;

pub async fn create_pool(url: &str) -> Result<OraclePool, CoreError> {
    let config = parse_url(url)?;
    PoolBuilder::new(config)
        .max_size(10)
        .build()
        .map_err(|e| CoreError { message: e.to_string(), code: "CONNECTION_FAILED".into() })
}

fn parse_url(url: &str) -> Result<Config, CoreError> {
    let parsed = url::Url::parse(url).map_err(|e| CoreError {
        message: format!("Invalid Oracle URL: {}", e),
        code: "INVALID_URL".into(),
    })?;

    let host = parsed.host_str().unwrap_or("localhost");
    let port = parsed.port().unwrap_or(1521);
    let service = parsed.path().trim_start_matches('/');
    let user = parsed.username();
    let password = parsed.password().unwrap_or("");

    if service.is_empty() {
        return Err(CoreError {
            message: "Oracle URL must include a service name, e.g. oracle://user:pass@host:1521/FREEPDB1".into(),
            code: "INVALID_URL".into(),
        });
    }

    Ok(Config::new(host, port, service, user, password))
}

pub async fn execute_select(
    pool: &OraclePool,
    sql: &str,
    sender: tokio::sync::mpsc::Sender<QueryEvent>,
    start: Instant,
) -> Result<(), CoreError> {
    let conn = pool.get().await.map_err(|e| CoreError {
        message: e.to_string(),
        code: "CONNECTION_FAILED".into(),
    })?;

    let result = match conn.query(sql, &[]).await {
        Ok(r) => r,
        Err(e) => {
            let _ = sender.send(QueryEvent::Error { message: e.to_string() }).await;
            return Ok(());
        }
    };

    let mut row_count: usize = 0;
    let max_rows: usize = 1000;

    if !result.columns.is_empty() {
        let names: Vec<String> = result.columns.iter().map(|c| c.name.clone()).collect();
        let _ = sender.send(QueryEvent::Columns { names }).await;
    }

    for row in &result.rows {
        if row_count < max_rows {
            let values: Vec<serde_json::Value> =
                row.values().iter().map(oracle_value_to_json).collect();
            let _ = sender.send(QueryEvent::Row { values }).await;
        }
        row_count += 1;
    }

    let duration_ms = start.elapsed().as_millis() as u64;
    let _ = sender.send(QueryEvent::Done { row_count, duration_ms }).await;
    Ok(())
}

pub async fn execute_statement(
    pool: &OraclePool,
    sql: &str,
    sender: tokio::sync::mpsc::Sender<QueryEvent>,
    start: Instant,
) -> Result<(), CoreError> {
    let conn = pool.get().await.map_err(|e| CoreError {
        message: e.to_string(),
        code: "CONNECTION_FAILED".into(),
    })?;

    match conn.execute(sql, &[]).await {
        Ok(result) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let _ = sender
                .send(QueryEvent::RowsAffected {
                    count: result.rows_affected,
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

pub async fn get_schemas(pool: &OraclePool) -> Result<Vec<SchemaInfo>, CoreError> {
    let conn = pool.get().await.map_err(|e| CoreError {
        message: e.to_string(),
        code: "CONNECTION_FAILED".into(),
    })?;

    // oracle_maintained = 'N' filters out built-in Oracle system schemas (Oracle 12.1+)
    let result = conn
        .query(
            "SELECT username, \
                 CASE WHEN username = SYS_CONTEXT('USERENV', 'CURRENT_USER') THEN 1 ELSE 0 END \
             FROM all_users \
             WHERE oracle_maintained = 'N' \
             ORDER BY username",
            &[],
        )
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    Ok(result
        .rows
        .iter()
        .map(|r| {
            let name = r.get_string(0).unwrap_or("unknown").to_string();
            let is_default = r.get_i64(1).unwrap_or(0) != 0;
            SchemaInfo { name, is_default }
        })
        .collect())
}

pub async fn get_tables(
    pool: &OraclePool,
    schema: Option<&str>,
) -> Result<Vec<TableInfo>, CoreError> {
    let conn = pool.get().await.map_err(|e| CoreError {
        message: e.to_string(),
        code: "CONNECTION_FAILED".into(),
    })?;

    // Determine schema: fall back to current user if not specified
    let schema_val = if let Some(s) = schema {
        s.to_string()
    } else {
        let r = conn
            .query("SELECT USER FROM DUAL", &[])
            .await
            .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;
        r.rows.first().and_then(|row| row.get_string(0)).unwrap_or("UNKNOWN").to_string()
    };

    let result = conn
        .query(
            "SELECT table_name, 'table' AS obj_type FROM all_tables WHERE owner = :1 \
             UNION ALL \
             SELECT view_name, 'view' FROM all_views WHERE owner = :1 \
             ORDER BY 1",
            &[Value::String(schema_val.clone())],
        )
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    Ok(result
        .rows
        .iter()
        .map(|r| {
            let name = r.get_string(0).unwrap_or("unknown").to_string();
            let table_type = r.get_string(1).unwrap_or("table").to_string();
            TableInfo {
                full_name: format!("{}.{}", schema_val, name),
                name,
                schema: Some(schema_val.clone()),
                table_type,
            }
        })
        .collect())
}

pub async fn get_columns(
    pool: &OraclePool,
    table_name: &str,
    schema: Option<&str>,
) -> Result<Vec<SchemaColumnInfo>, CoreError> {
    let conn = pool.get().await.map_err(|e| CoreError {
        message: e.to_string(),
        code: "CONNECTION_FAILED".into(),
    })?;

    // Resolve schema
    let schema_val = if let Some(s) = schema {
        s.to_string()
    } else {
        let r = conn
            .query("SELECT USER FROM DUAL", &[])
            .await
            .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;
        r.rows.first().and_then(|row| row.get_string(0)).unwrap_or("UNKNOWN").to_string()
    };

    // Primary key columns
    let pk_result = conn
        .query(
            "SELECT acc.column_name \
             FROM all_constraints ac \
             JOIN all_cons_columns acc \
                 ON ac.constraint_name = acc.constraint_name AND ac.owner = acc.owner \
             WHERE ac.constraint_type = 'P' AND ac.owner = :1 AND ac.table_name = :2 \
             ORDER BY acc.position",
            &[Value::String(schema_val.clone()), Value::String(table_name.to_string())],
        )
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    let pk_columns: HashSet<String> = pk_result
        .rows
        .iter()
        .filter_map(|r| r.get_string(0).map(String::from))
        .collect();

    // Foreign key columns
    let fk_result = conn
        .query(
            "SELECT acc.column_name, ac2.owner, ac2.table_name, acc2.column_name \
             FROM all_constraints ac \
             JOIN all_cons_columns acc \
                 ON ac.constraint_name = acc.constraint_name AND ac.owner = acc.owner \
             JOIN all_constraints ac2 \
                 ON ac.r_constraint_name = ac2.constraint_name AND ac.r_owner = ac2.owner \
             JOIN all_cons_columns acc2 \
                 ON ac2.constraint_name = acc2.constraint_name \
                 AND ac2.owner = acc2.owner \
                 AND acc.position = acc2.position \
             WHERE ac.constraint_type = 'R' AND ac.owner = :1 AND ac.table_name = :2",
            &[Value::String(schema_val.clone()), Value::String(table_name.to_string())],
        )
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    let mut fk_map: HashMap<String, (String, String)> = HashMap::new();
    for r in &fk_result.rows {
        if let (Some(col), Some(ref_owner), Some(ref_table), Some(ref_col)) = (
            r.get_string(0),
            r.get_string(1),
            r.get_string(2),
            r.get_string(3),
        ) {
            fk_map.insert(
                col.to_string(),
                (format!("{}.{}", ref_owner, ref_table), ref_col.to_string()),
            );
        }
    }

    // Column metadata
    let col_result = conn
        .query(
            "SELECT column_name, data_type, nullable, data_default, column_id \
             FROM all_tab_columns \
             WHERE owner = :1 AND table_name = :2 \
             ORDER BY column_id",
            &[Value::String(schema_val), Value::String(table_name.to_string())],
        )
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    Ok(col_result
        .rows
        .iter()
        .map(|r| {
            let name = r.get_string(0).unwrap_or("unknown").to_string();
            let data_type = r.get_string(1).unwrap_or("unknown");
            let nullable = r.get_string(2).map(|v| v == "Y").unwrap_or(true);
            let default_value = r.get(3).and_then(|v| {
                if v.is_null() { None } else { Some(v.to_string()) }
            });
            let ordinal = r.get_i64(4).unwrap_or(0) as i32;
            let is_fk = fk_map.contains_key(&name);
            let (foreign_table, foreign_column) = fk_map
                .get(&name)
                .map(|(t, c)| (Some(t.clone()), Some(c.clone())))
                .unwrap_or((None, None));
            SchemaColumnInfo {
                name: name.clone(),
                data_type: map_oracle_type(data_type),
                nullable,
                default_value,
                is_primary_key: pk_columns.contains(&name),
                is_foreign_key: is_fk,
                foreign_table,
                foreign_column,
                ordinal_position: ordinal,
            }
        })
        .collect())
}

fn map_oracle_type(type_name: &str) -> String {
    let lower = type_name.to_lowercase();
    // Oracle types often have precision/scale appended, e.g. "NUMBER(10,2)"
    let base = lower.split('(').next().unwrap_or(&lower).trim();
    match base {
        "number" | "integer" | "int" | "smallint" | "binary_integer" | "pls_integer" => "number",
        "float" | "binary_float" | "binary_double" => "float",
        "varchar2" | "varchar" | "nvarchar2" | "char" | "nchar" | "clob" | "nclob" | "long" => {
            "text"
        }
        "raw" | "long raw" | "blob" | "bfile" => "binary",
        "date" => "datetime",
        t if t.starts_with("timestamp") => "datetime",
        "interval year" | "interval day" => "interval",
        "boolean" => "boolean",
        "json" => "json",
        "xmltype" => "xml",
        "rowid" | "urowid" => "rowid",
        other => other,
    }
    .to_string()
}

fn oracle_value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Boolean(b) => serde_json::json!(b),
        Value::Integer(n) => serde_json::json!(n),
        Value::Float(f) => serde_json::json!(f),
        Value::Number(n) => {
            if n.is_integer {
                n.to_i64()
                    .map(|v| serde_json::json!(v))
                    .unwrap_or_else(|_| serde_json::json!(n.as_str()))
            } else {
                n.to_f64()
                    .map(|v| serde_json::json!(v))
                    .unwrap_or_else(|_| serde_json::json!(n.as_str()))
            }
        }
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::Bytes(b) => serde_json::json!(format!("<binary: {} bytes>", b.len())),
        Value::Date(d) => serde_json::json!(format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            d.year, d.month, d.day, d.hour, d.minute, d.second
        )),
        Value::Timestamp(ts) => {
            if ts.has_timezone() {
                serde_json::json!(format!(
                    "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:06}{:+03}:{:02}",
                    ts.year,
                    ts.month,
                    ts.day,
                    ts.hour,
                    ts.minute,
                    ts.second,
                    ts.microsecond,
                    ts.tz_hour_offset,
                    ts.tz_minute_offset.unsigned_abs()
                ))
            } else {
                serde_json::json!(format!(
                    "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:06}",
                    ts.year, ts.month, ts.day, ts.hour, ts.minute, ts.second, ts.microsecond
                ))
            }
        }
        Value::Json(v) => v.clone(),
        Value::RowId(r) => serde_json::Value::String(r.to_string().unwrap_or_default()),
        Value::Lob(lob) => match lob {
            LobValue::Null => serde_json::Value::Null,
            LobValue::Empty => serde_json::Value::String(String::new()),
            LobValue::Inline(data) => match std::str::from_utf8(data) {
                Ok(s) => serde_json::Value::String(s.to_string()),
                Err(_) => serde_json::json!(format!("<binary: {} bytes>", data.len())),
            },
            LobValue::Locator(loc) => {
                serde_json::json!(format!("<lob: {} bytes>", loc.size()))
            }
        },
        Value::Vector(_) => serde_json::Value::String("<vector>".to_string()),
        Value::Cursor(_) => serde_json::Value::String("<cursor>".to_string()),
        Value::Collection(_) => serde_json::Value::String("<collection>".to_string()),
    }
}
