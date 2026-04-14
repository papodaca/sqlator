use crate::error::CoreError;
use crate::models::QueryEvent;
use sqlx::sqlite::SqliteRow;
use sqlx::{Column, Row, SqlitePool, TypeInfo};
use std::time::Instant;

pub async fn execute_select(
    pool: &SqlitePool,
    sql: &str,
    sender: tokio::sync::mpsc::Sender<QueryEvent>,
    start: Instant,
) -> Result<(), CoreError> {
    use futures::StreamExt;

    let mut stream = sqlx::query(sql).fetch(pool);
    let mut row_count: usize = 0;
    let mut columns_sent = false;
    let max_rows: usize = 1000;

    while let Some(result) = stream.next().await {
        match result {
            Ok(row) => {
                if !columns_sent {
                    let names: Vec<String> = row
                        .columns()
                        .iter()
                        .map(|c| c.name().to_string())
                        .collect();
                    let _ = sender.send(QueryEvent::Columns { names }).await;
                    columns_sent = true;
                }

                if row_count < max_rows {
                    let values: Vec<serde_json::Value> = row
                        .columns()
                        .iter()
                        .enumerate()
                        .map(|(i, _)| sqlite_row_to_json(&row, i))
                        .collect();
                    let _ = sender.send(QueryEvent::Row { values }).await;
                }
                row_count += 1;
            }
            Err(e) => {
                let _ = sender.send(QueryEvent::Error { message: e.to_string() }).await;
                return Ok(());
            }
        }
    }

    let duration_ms = start.elapsed().as_millis() as u64;
    let _ = sender
        .send(QueryEvent::Done {
            row_count,
            duration_ms,
        })
        .await;
    Ok(())
}

pub async fn get_ddl(pool: &SqlitePool, table_name: &str) -> Result<String, CoreError> {
    let safe_name = table_name.replace('"', "\"\"");
    let sql = format!("SELECT sql FROM sqlite_master WHERE name = \"{}\"", safe_name);
    let row = sqlx::query(&sql)
        .fetch_one(pool)
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "DDL_QUERY".into() })?;
    let raw = row.try_get::<String, _>("sql").unwrap_or_default();
    let mut result = String::with_capacity(raw.len());
    let mut prev_newline = false;
    for ch in raw.chars() {
        if ch == '\n' {
            if prev_newline {
                continue;
            }
            prev_newline = true;
        } else {
            prev_newline = false;
        }
        result.push(ch);
    }
    Ok(result)
}

pub(super) fn sqlite_row_to_json(row: &SqliteRow, index: usize) -> serde_json::Value {
    if let Ok(v) = row.try_get::<Option<String>, _>(index) {
        return match v {
            Some(s) => serde_json::Value::String(s),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<i64>, _>(index) {
        return match v {
            Some(n) => serde_json::json!(n),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<f64>, _>(index) {
        return match v {
            Some(n) => serde_json::json!(n),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<bool>, _>(index) {
        return match v {
            Some(b) => serde_json::json!(b),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<Vec<u8>>, _>(index) {
        return match v {
            Some(bytes) => serde_json::json!(format!("<binary: {} bytes>", bytes.len())),
            None => serde_json::Value::Null,
        };
    }

    let type_name = row.column(index).type_info().name().to_string();
    serde_json::Value::String(format!("<{}>", type_name))
}
