// Oracle database support via oracle-rs pure-Rust TNS driver.
// Connection URL format: oracle://user:pass@host:1521/service_name
use crate::error::CoreError;
use crate::models::{QueryEvent, SchemaColumnInfo, SchemaInfo, TableInfo, TableQueryParams, TableQueryResult};
use deadpool_oracle::PoolBuilder;
use oracle_rs::{Config, LobValue, Value};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

pub type OraclePool = deadpool_oracle::Pool;

pub async fn create_pool(url: &str) -> Result<OraclePool, CoreError> {
    let config = parse_url(url)?;
    let pool = PoolBuilder::new(config)
        .max_size(4)
        .build()
        .map_err(|e| CoreError {
            message: format!("Failed to create Oracle connection pool: {}", e),
            code: "POOL_CREATE_FAILED".into(),
        })?;

    // Verify the connection works by getting one from the pool
    let _conn = pool.get().await.map_err(|e| CoreError {
        message: format!("Oracle connection failed: {}", e),
        code: "CONNECTION_FAILED".into(),
    })?;

    Ok(pool)
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
            // Oracle auto-commit is off by default; commit DML so changes persist
            // across pooled connections (deadpool recycles via rollback).
            if let Err(e) = conn.commit().await {
                let _ = sender.send(QueryEvent::Error { message: format!("Commit failed: {}", e) }).await;
                return Ok(());
            }
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

    // Resolve current user with a simple DUAL query (avoids SYS_CONTEXT in the main query
    // which can trigger protocol issues on Oracle Free)
    let current_user = conn
        .query("SELECT USER FROM DUAL", &[])
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?
        .rows
        .first()
        .and_then(|r| r.get_string(0))
        .unwrap_or("")
        .to_uppercase();

    // oracle_maintained = 'N' filters out built-in Oracle system schemas (Oracle 12.1+).
    // Always include the current user even if they are oracle-maintained (e.g. SYSTEM).
    let result = conn
        .query(
            "SELECT username FROM all_users \
             WHERE oracle_maintained = 'N' OR username = :1 \
             ORDER BY username",
            &[Value::String(current_user.clone())],
        )
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;

    Ok(result
        .rows
        .iter()
        .map(|r| {
            let name = r.get_string(0).unwrap_or("unknown").to_string();
            let is_default = name.to_uppercase() == current_user;
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
             SELECT view_name, 'view' FROM all_views WHERE owner = :2 \
             ORDER BY 1",
            &[Value::String(schema_val.clone()), Value::String(schema_val.clone())],
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
    // Resolve schema using a dedicated connection that is dropped immediately after.
    // Each query uses its own connection to avoid oracle-rs leaving the connection
    // in a bad protocol state after complex multi-JOIN queries.
    let schema_val = if let Some(s) = schema {
        s.to_string()
    } else {
        let conn = pool.get().await.map_err(|e| CoreError {
            message: e.to_string(),
            code: "CONNECTION_FAILED".into(),
        })?;
        let r = conn
            .query("SELECT USER FROM DUAL", &[])
            .await
            .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;
        r.rows.first().and_then(|row| row.get_string(0)).unwrap_or("UNKNOWN").to_string()
    };

    let schema_esc = schema_val.replace('\'', "''");
    let table_esc = table_name.replace('\'', "''");

    // Primary key columns — fresh connection, dropped before next query
    let pk_columns: HashSet<String> = {
        let conn = pool.get().await.map_err(|e| CoreError {
            message: e.to_string(),
            code: "CONNECTION_FAILED".into(),
        })?;
        let pk_sql = format!(
            "SELECT acc.column_name \
             FROM all_constraints ac \
             JOIN all_cons_columns acc \
                 ON ac.constraint_name = acc.constraint_name AND ac.owner = acc.owner \
             WHERE ac.constraint_type = 'P' AND ac.owner = '{}' AND ac.table_name = '{}' \
             ORDER BY acc.position",
            schema_esc, table_esc
        );
        let result = conn
            .query(&pk_sql, &[])
            .await
            .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;
        result.rows.iter().filter_map(|r| r.get_string(0).map(String::from)).collect()
    };

    // Foreign key columns — fresh connection
    let fk_map: HashMap<String, (String, String)> = {
        let conn = pool.get().await.map_err(|e| CoreError {
            message: e.to_string(),
            code: "CONNECTION_FAILED".into(),
        })?;
        let fk_sql = format!(
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
             WHERE ac.constraint_type = 'R' AND ac.owner = '{}' AND ac.table_name = '{}'",
            schema_esc, table_esc
        );
        let result = conn
            .query(&fk_sql, &[])
            .await
            .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?;
        let mut map = HashMap::new();
        for r in &result.rows {
            if let (Some(col), Some(ref_owner), Some(ref_table), Some(ref_col)) = (
                r.get_string(0),
                r.get_string(1),
                r.get_string(2),
                r.get_string(3),
            ) {
                map.insert(
                    col.to_string(),
                    (format!("{}.{}", ref_owner, ref_table), ref_col.to_string()),
                );
            }
        }
        map
    };

    // Column metadata — fresh connection
    let col_result = {
        let conn = pool.get().await.map_err(|e| CoreError {
            message: e.to_string(),
            code: "CONNECTION_FAILED".into(),
        })?;
        // Exclude data_default (LONG type) — oracle-rs cannot decode Oracle LONG columns,
        // causing the entire result to come back empty. Default values are not displayed
        // in the schema browser so omitting them is safe.
        let col_sql = format!(
            "SELECT column_name, data_type, nullable, column_id \
             FROM all_tab_columns \
             WHERE owner = '{}' AND table_name = '{}' \
             ORDER BY column_id",
            schema_esc, table_esc
        );
        conn
            .query(&col_sql, &[])
            .await
            .map_err(|e| CoreError { message: e.to_string(), code: "SCHEMA_QUERY".into() })?
    };

    Ok(col_result
        .rows
        .iter()
        .map(|r| {
            let name = r.get_string(0).unwrap_or("unknown").to_string();
            let data_type = r.get_string(1).unwrap_or("unknown");
            let nullable = r.get_string(2).map(|v| v == "Y").unwrap_or(true);
            let ordinal = r.get_i64(3).unwrap_or(0) as i32;
            let is_fk = fk_map.contains_key(&name);
            let (foreign_table, foreign_column) = fk_map
                .get(&name)
                .map(|(t, c)| (Some(t.clone()), Some(c.clone())))
                .unwrap_or((None, None));
            SchemaColumnInfo {
                name: name.clone(),
                data_type: map_oracle_type(data_type),
                nullable,
                default_value: None,
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

pub async fn query_table(
    pool: &OraclePool,
    params: &TableQueryParams,
    valid_columns: &[&str],
    col_names: Vec<String>,
    col_types: Vec<String>,
) -> Result<TableQueryResult, CoreError> {
    let schema = params.schema.as_deref().unwrap_or("").to_uppercase();
    let table_upper = params.table_name.to_uppercase();
    let table_quoted = if schema.is_empty() {
        format!("\"{}\"", table_upper.replace('"', "\"\""))
    } else {
        format!(
            "\"{}\".\"{}\"",
            schema.replace('"', "\"\""),
            table_upper.replace('"', "\"\"")
        )
    };

    let where_clause = build_where_oracle(&params.filters, valid_columns);
    let order_clause = build_order_by_oracle(&params.sort, valid_columns);

    let limit = params.limit.min(1000) + 1;
    let sql = format!(
        "SELECT * FROM {}{}{} OFFSET {} ROWS FETCH NEXT {} ROWS ONLY",
        table_quoted, where_clause, order_clause, params.offset, limit
    );

    let conn = pool.get().await.map_err(|e| CoreError {
        message: e.to_string(),
        code: "CONNECTION_FAILED".into(),
    })?;

    let result = conn.query(&sql, &[]).await.map_err(|e| CoreError {
        message: e.to_string(),
        code: "QUERY_TABLE".into(),
    })?;

    let has_more = result.rows.len() as i64 > params.limit.min(1000);
    let rows_slice = if has_more {
        &result.rows[..result.rows.len() - 1]
    } else {
        &result.rows[..]
    };

    let result_rows: Vec<serde_json::Value> = rows_slice
        .iter()
        .map(|row| {
            let values: Vec<serde_json::Value> = row.values().iter().map(oracle_value_to_json).collect();
            let mut obj = serde_json::Map::new();
            for (i, col) in col_names.iter().enumerate() {
                let val = values.get(i).cloned().unwrap_or(serde_json::Value::Null);
                obj.insert(col.clone(), val);
            }
            serde_json::Value::Object(obj)
        })
        .collect();

    Ok(TableQueryResult {
        columns: col_names,
        column_types: col_types,
        rows: result_rows,
        has_more,
        total_returned: rows_slice.len(),
    })
}

fn format_sql_literal(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::Null => "NULL".to_string(),
        serde_json::Value::Bool(b) => if *b { "1" } else { "0" }.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => format!("'{}'", s.replace('\'', "''")),
        _ => format!("'{}'", val.to_string().replace('\'', "''")),
    }
}

fn build_where_oracle(filters: &[crate::models::FilterSpec], valid: &[&str]) -> String {
    let parts: Vec<String> = filters
        .iter()
        .filter(|f| valid.contains(&f.column.as_str()))
        .filter_map(|f| {
            let col = format!("\"{}\"", f.column.replace('"', "\"\""));
            match f.operator.as_str() {
                "isNull" => Some(format!("{} IS NULL", col)),
                "isNotNull" => Some(format!("{} IS NOT NULL", col)),
                _ => {
                    let val = f.value.as_ref()?;
                    let lit = format_sql_literal(val);
                    match f.operator.as_str() {
                        "contains" => {
                            let s = match val {
                                serde_json::Value::String(s) => s.replace('\'', "''"),
                                _ => val.to_string(),
                            };
                            Some(format!("{} LIKE '%{}%'", col, s))
                        }
                        "startsWith" => {
                            let s = match val {
                                serde_json::Value::String(s) => s.replace('\'', "''"),
                                _ => val.to_string(),
                            };
                            Some(format!("{} LIKE '{}%'", col, s))
                        }
                        "endsWith" => {
                            let s = match val {
                                serde_json::Value::String(s) => s.replace('\'', "''"),
                                _ => val.to_string(),
                            };
                            Some(format!("{} LIKE '%{}'", col, s))
                        }
                        "equals" => Some(format!("{} = {}", col, lit)),
                        "gt" => Some(format!("{} > {}", col, lit)),
                        "gte" => Some(format!("{} >= {}", col, lit)),
                        "lt" => Some(format!("{} < {}", col, lit)),
                        "lte" => Some(format!("{} <= {}", col, lit)),
                        _ => None,
                    }
                }
            }
        })
        .collect();

    if parts.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", parts.join(" AND "))
    }
}

fn build_order_by_oracle(sort: &[crate::models::SortSpec], valid: &[&str]) -> String {
    let parts: Vec<String> = sort
        .iter()
        .filter(|s| valid.contains(&s.column.as_str()))
        .map(|s| {
            let col = format!("\"{}\"", s.column.replace('"', "\"\""));
            format!("{} {}", col, if s.desc { "DESC" } else { "ASC" })
        })
        .collect();

    if parts.is_empty() {
        String::new()
    } else {
        format!(" ORDER BY {}", parts.join(", "))
    }
}
