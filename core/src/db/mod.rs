mod any;
mod mysql;
mod postgres;
mod sqlite;

use crate::error::CoreError;
use crate::models::QueryEvent;
use dashmap::DashMap;
use sqlx::{AnyPool, MySqlPool, PgPool, SqlitePool};
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
