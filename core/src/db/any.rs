use crate::error::CoreError;
use crate::models::QueryEvent;
use sqlx::any::AnyRow;
use sqlx::{AnyPool, Column, Row, TypeInfo};
use std::time::Instant;

pub async fn execute_select(
    pool: &AnyPool,
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
                let row: AnyRow = row;
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
                        .map(|(i, _)| row_value_to_json(&row, i))
                        .collect();
                    let _ = sender.send(QueryEvent::Row { values }).await;
                }
                row_count += 1;
            }
            Err(e) => {
                let msg = e.to_string();
                let hint = if msg.contains("Jsonb") || msg.contains("JSONB") {
                    format!(
                        "{}\n\nTip: Cast JSONB columns to text in your query:\n  SELECT jsonb_col::text FROM table",
                        msg
                    )
                } else {
                    msg
                };
                let _ = sender.send(QueryEvent::Error { message: hint }).await;
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

fn row_value_to_json(row: &AnyRow, index: usize) -> serde_json::Value {
    let type_name = row.column(index).type_info().name().to_string();

    if let Ok(v) = row.try_get::<Option<String>, _>(index) {
        return match v {
            Some(s) => serde_json::Value::String(s),
            None => return serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<i64>, _>(index) {
        return match v {
            Some(n) => serde_json::json!(n),
            None => return serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<f64>, _>(index) {
        return match v {
            Some(n) => serde_json::json!(n),
            None => return serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<bool>, _>(index) {
        return match v {
            Some(b) => serde_json::json!(b),
            None => return serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<Vec<u8>>, _>(index) {
        return match v {
            Some(bytes) => serde_json::json!(format!("<binary: {} bytes>", bytes.len())),
            None => return serde_json::Value::Null,
        };
    }
    serde_json::Value::String(format!("<{}>", type_name))
}
