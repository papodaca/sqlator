use crate::error::CoreError;
use crate::models::QueryEvent;
use dashmap::DashMap;
use sqlx::any::AnyRow;
use sqlx::{AnyPool, Column, Row};
use std::time::Instant;

/// Manages database connection pools.
/// Thread-safe via DashMap — no Mutex needed since Pool is already Send+Sync.
pub struct DbManager {
    pools: DashMap<String, AnyPool>,
}

impl DbManager {
    pub fn new() -> Self {
        sqlx::any::install_default_drivers();
        Self {
            pools: DashMap::new(),
        }
    }

    pub async fn test_connection(url: &str) -> Result<String, CoreError> {
        let pool: AnyPool = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            AnyPool::connect(url),
        )
        .await
        .map_err(|_| CoreError {
            message: "Connection timed out after 5 seconds".into(),
            code: "TIMEOUT".into(),
        })?
        .map_err(CoreError::from)?;

        pool.close().await;
        Ok("Connected successfully".to_string())
    }

    pub async fn connect(&self, connection_id: &str, url: &str) -> Result<(), CoreError> {
        // Close existing pool if any
        if let Some((_, old_pool)) = self.pools.remove(connection_id) {
            old_pool.close().await;
        }

        let pool: AnyPool = AnyPool::connect(url).await.map_err(CoreError::from)?;
        self.pools.insert(connection_id.to_string(), pool);
        Ok(())
    }

    pub async fn disconnect(&self, connection_id: &str) {
        if let Some((_, pool)) = self.pools.remove(connection_id) {
            pool.close().await;
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

        // Determine if this is a SELECT-like query or a DML/DDL statement
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

        if is_select {
            self.execute_select(&pool, sql_trimmed, sender, start)
                .await
        } else {
            self.execute_statement(&pool, sql_trimmed, sender, start)
                .await
        }
    }

    async fn execute_select(
        &self,
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
                            .map(|(i, _col)| row_value_to_json(&row, i))
                            .collect();
                        let _ = sender.send(QueryEvent::Row { values }).await;
                    }
                    row_count += 1;
                }
                Err(e) => {
                    let _ = sender
                        .send(QueryEvent::Error {
                            message: e.to_string(),
                        })
                        .await;
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

    async fn execute_statement(
        &self,
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
                let _ = sender
                    .send(QueryEvent::Error {
                        message: e.to_string(),
                    })
                    .await;
            }
        }
        Ok(())
    }
}

impl Default for DbManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract a value from an AnyRow at the given index, converting to serde_json::Value.
/// Tries multiple types since AnyRow doesn't support dynamic type introspection well.
fn row_value_to_json(row: &AnyRow, index: usize) -> serde_json::Value {
    // Try string first (most types can be read as string from Any driver)
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
    serde_json::Value::Null
}
