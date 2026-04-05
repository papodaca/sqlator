use crate::error::CoreError;
use crate::models::QueryEvent;
use sqlx::postgres::PgRow;
use sqlx::{Column, PgPool, Row, TypeInfo};
use std::time::Instant;

pub async fn execute_select(
    pool: &PgPool,
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
                        .map(|(i, _)| pg_row_to_json(&row, i))
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

fn pg_row_to_json(row: &PgRow, index: usize) -> serde_json::Value {
    if let Ok(v) = row.try_get::<Option<sqlx::types::Json<serde_json::Value>>, _>(index) {
        return v.map(|j| j.0).unwrap_or(serde_json::Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<Vec<String>>, _>(index) {
        return serde_json::json!(v);
    }
    if let Ok(v) = row.try_get::<Option<Vec<i32>>, _>(index) {
        return serde_json::json!(v);
    }
    if let Ok(v) = row.try_get::<Option<Vec<i64>>, _>(index) {
        return serde_json::json!(v);
    }
    if let Ok(v) = row.try_get::<Option<Vec<f64>>, _>(index) {
        return serde_json::json!(v);
    }
    if let Ok(v) = row.try_get::<Option<Vec<bool>>, _>(index) {
        return serde_json::json!(v);
    }
    if let Ok(v) = row.try_get::<Option<uuid::Uuid>, _>(index) {
        return match v {
            Some(u) => serde_json::json!(u.to_string()),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(index) {
        return match v {
            Some(dt) => serde_json::json!(dt.to_rfc3339()),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<chrono::NaiveDateTime>, _>(index) {
        return match v {
            Some(dt) => serde_json::json!(dt.to_string()),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<chrono::NaiveDate>, _>(index) {
        return match v {
            Some(d) => serde_json::json!(d.to_string()),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<chrono::NaiveTime>, _>(index) {
        return match v {
            Some(t) => serde_json::json!(t.to_string()),
            None => serde_json::Value::Null,
        };
    }
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
    if let Ok(v) = row.try_get::<Option<i32>, _>(index) {
        return match v {
            Some(n) => serde_json::json!(n),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<i16>, _>(index) {
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
    if let Ok(v) = row.try_get::<Option<f32>, _>(index) {
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
