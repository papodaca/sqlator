mod any;
mod clickhouse;
mod mssql;
mod mysql;
mod oracle;
mod postgres;
mod sql_classify;
mod sqlite;

pub use sql_classify::PagedQueryOutcome;

use crate::error::CoreError;
use crate::models::{
    BatchError, BatchResult, ColumnMeta, FilterSpec, PrimaryKeyMeta, QueryEvent, SchemaColumnInfo,
    SchemaInfo, SortSpec, SqlBatch, TableInfo, TableMeta, TableQueryParams, TableQueryResult,
};
use dashmap::DashMap;
use sql_classify::{PagedDialect, PaginationClassification};
use sqlx::{AnyPool, MySqlPool, PgPool, Row, SqlitePool};
use std::collections::HashMap;
use std::time::Instant;

/// Engine-side ceiling for paged query runs (KTD-5 / R5). Offsets at or above
/// this are refused; the GTK UI shows the "50,000 row limit reached" message.
pub const PAGED_ROW_CEILING: usize = 50_000;

/// Sentinel-safe page-size bound (KTD-4): the wrapper fetches `limit + 1`
/// rows and every driver's streaming path suppresses sends past 1,000, so the
/// sentinel only works when `limit + 1 <= 1000`.
const PAGED_MAX_PAGE_SIZE: usize = 999;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseType {
    Postgres,
    MySql,
    Sqlite,
    Mssql,
    Oracle,
    ClickHouse,
}

#[derive(Clone)]
pub enum DatabasePool {
    Postgres(PgPool),
    MySql(MySqlPool),
    Sqlite(SqlitePool),
    Any(AnyPool),
    Mssql(mssql::MssqlPool),
    Oracle(oracle::OraclePool),
    ClickHouse(clickhouse::ClickHousePool),
}

/// Server-side cancel target for an in-flight query (sqlator-9g1.4).
#[derive(Debug, Clone, Copy)]
enum BackendCancelId {
    Postgres(i32),
    MySql(u64),
}

pub struct DbManager {
    pools: DashMap<String, DatabasePool>,
    /// Latest backend id per connection for [`Self::cancel_query`].
    active_backends: DashMap<String, BackendCancelId>,
}

impl DbManager {
    pub fn new() -> Self {
        sqlx::any::install_default_drivers();
        Self {
            pools: DashMap::new(),
            active_backends: DashMap::new(),
        }
    }

    /// Best-effort server-side cancel of the latest query on `connection_id`.
    ///
    /// PostgreSQL: `pg_cancel_backend`. MySQL/MariaDB: `KILL QUERY`. Other
    /// engines: no-op (client-side cancel still drops the waiting future).
    ///
    /// Uses a separate pool checkout so cancel does not wait on the query's
    /// connection (requires pool `max_connections` ≥ 2 — the default).
    pub async fn cancel_query(&self, connection_id: &str) -> Result<(), CoreError> {
        let Some(backend) = self.active_backends.get(connection_id).map(|v| *v) else {
            return Ok(());
        };
        let Some(pool) = self.pools.get(connection_id).map(|p| p.clone()) else {
            return Ok(());
        };
        match (backend, pool) {
            (BackendCancelId::Postgres(pid), DatabasePool::Postgres(p)) => {
                sqlx::query("SELECT pg_cancel_backend($1)")
                    .bind(pid)
                    .execute(&p)
                    .await
                    .map_err(|e| CoreError {
                        message: format!("pg_cancel_backend failed: {e}"),
                        code: "CANCEL_FAILED".into(),
                    })?;
            }
            (BackendCancelId::MySql(id), DatabasePool::MySql(p)) => {
                // CONNECTION_ID() is a server-assigned integer; safe to embed.
                sqlx::query(&format!("KILL QUERY {id}"))
                    .execute(&p)
                    .await
                    .map_err(|e| CoreError {
                        message: format!("KILL QUERY failed: {e}"),
                        code: "CANCEL_FAILED".into(),
                    })?;
            }
            _ => {}
        }
        Ok(())
    }

    pub async fn test_connection(url: &str) -> Result<String, CoreError> {
        let pool =
            tokio::time::timeout(std::time::Duration::from_secs(5), create_pool_for_url(url))
                .await
                .map_err(|_| CoreError {
                    message: "Connection timed out after 5 seconds".into(),
                    code: "TIMEOUT".into(),
                })??;

        close_pool(pool).await;
        Ok("Connected successfully".to_string())
    }

    pub async fn connect(&self, connection_id: &str, url: &str) -> Result<(), CoreError> {
        if let Some((_, old_pool)) = self.pools.remove(connection_id) {
            close_pool(old_pool).await;
        }

        let pool =
            tokio::time::timeout(std::time::Duration::from_secs(5), create_pool_for_url(url))
                .await
                .map_err(|_| CoreError {
                    message: "Connection timed out after 5 seconds".into(),
                    code: "TIMEOUT".into(),
                })??;

        self.pools.insert(connection_id.to_string(), pool);
        Ok(())
    }

    pub async fn disconnect(&self, connection_id: &str) {
        self.active_backends.remove(connection_id);
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

        let result = match pool {
            DatabasePool::Postgres(p) => {
                let mut register = |pid: i32| {
                    self.active_backends
                        .insert(connection_id.to_string(), BackendCancelId::Postgres(pid));
                };
                if is_select {
                    postgres::execute_select(&p, sql_trimmed, sender, start, Some(&mut register))
                        .await
                } else {
                    execute_statement_pg(&p, sql_trimmed, sender, start, Some(&mut register)).await
                }
            }
            DatabasePool::MySql(p) => {
                let mut register = |id: u64| {
                    self.active_backends
                        .insert(connection_id.to_string(), BackendCancelId::MySql(id));
                };
                if is_select {
                    mysql::execute_select(&p, sql_trimmed, sender, start, Some(&mut register)).await
                } else {
                    execute_statement_mysql(&p, sql_trimmed, sender, start, Some(&mut register))
                        .await
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
            DatabasePool::Mssql(p) => {
                if is_select {
                    mssql::execute_select(&p, sql_trimmed, sender, start).await
                } else {
                    mssql::execute_statement(&p, sql_trimmed, sender, start).await
                }
            }
            DatabasePool::Oracle(p) => {
                if is_select {
                    oracle::execute_select(&p, sql_trimmed, sender, start).await
                } else {
                    oracle::execute_statement(&p, sql_trimmed, sender, start).await
                }
            }
            DatabasePool::ClickHouse(p) => {
                if is_select {
                    clickhouse::execute_select(&p, sql_trimmed, sender, start).await
                } else {
                    clickhouse::execute_statement(&p, sql_trimmed, sender, start).await
                }
            }
        };
        self.active_backends.remove(connection_id);
        result
    }

    /// Execute an ad-hoc query as one offset-driven page of rows (U1: R4/R5/R6).
    ///
    /// Wrappable queries (per [`sql_classify::classify_pagination`]) run inside
    /// the transparent CTE wrapper with a `limit + 1` sentinel fetch (KTD-2);
    /// events forwarded to `sender` keep the exact `Columns → Row* → Done`
    /// sequencing of [`Self::execute_query`], with at most `limit` row events
    /// and a `Done` whose `row_count` is the forwarded count. Passthrough
    /// queries delegate to [`Self::execute_query`] unchanged.
    ///
    /// `sortable_columns` is the whitelist for sort rendering: pass the column
    /// names from the page-one run of the same SQL (a sorted re-query has them
    /// on screen already); pass `&[]` on a fresh first run — every sort spec is
    /// then silently dropped, never string-interpolated. `limit` is clamped to
    /// the sentinel-safe bound (KTD-4). Offsets ≥ [`PAGED_ROW_CEILING`]
    /// short-circuit with a `capped` outcome and no driver call (R5).
    // Flat param list mirrors execute_query + page spec (gtk browse.rs carries
    // the same allow for its fetch signature).
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_query_paged(
        &self,
        connection_id: &str,
        sql: &str,
        sort: &[SortSpec],
        sortable_columns: &[String],
        limit: usize,
        offset: usize,
        sender: tokio::sync::mpsc::Sender<QueryEvent>,
    ) -> Result<PagedQueryOutcome, CoreError> {
        let pool = self
            .pools
            .get(connection_id)
            .ok_or_else(|| CoreError {
                message: "Not connected".into(),
                code: "NO_CONNECTION".into(),
            })?
            .clone();

        let dialect = paged_dialect_for_pool(&pool);

        match sql_classify::classify_pagination(sql, dialect) {
            PaginationClassification::Passthrough(reason) => {
                tracing::debug!(
                    connection_id,
                    reason = reason.as_str(),
                    "paged execution: passthrough"
                );
                self.execute_query(connection_id, sql, sender).await?;
                return Ok(PagedQueryOutcome::passthrough());
            }
            PaginationClassification::Wrappable => {}
        }

        // Ceiling short-circuit: refuse the page entirely (no driver call).
        if offset_at_ceiling(offset) {
            return Ok(PagedQueryOutcome {
                row_count: 0,
                has_more: false,
                capped: true,
                paged: true,
            });
        }

        let limit = clamp_page_size(limit);
        let valid: Vec<&str> = sortable_columns.iter().map(String::as_str).collect();
        let wrapped = sql_classify::wrap_for_pagination(sql, dialect, sort, &valid, limit, offset);

        // Driver streams the wrapped SQL into an internal channel; we forward
        // events through the sentinel gate while the driver runs (single-owner
        // select! + mpsc bridge — no shared locks).
        let (inner_tx, mut inner_rx) = tokio::sync::mpsc::channel::<QueryEvent>(64);
        let start = Instant::now();
        let driver =
            self.execute_wrapped_page_select(connection_id, pool, &wrapped, inner_tx, start);
        tokio::pin!(driver);

        let mut driver_result: Option<Result<(), CoreError>> = None;
        let mut gate = SentinelPageGate::new(limit);

        loop {
            tokio::select! {
                res = &mut driver, if driver_result.is_none() => {
                    driver_result = Some(res);
                    // The driver sends everything before returning, so all
                    // remaining events are already buffered — drain and stop.
                    while let Ok(ev) = inner_rx.try_recv() {
                        if let Some(ev) = gate.observe(ev) {
                            let _ = sender.send(ev).await;
                        }
                    }
                    break;
                }
                maybe = inner_rx.recv() => {
                    match maybe {
                        Some(ev) => {
                            if let Some(ev) = gate.observe(ev) {
                                let _ = sender.send(ev).await;
                            }
                        }
                        // Channel closed ⟹ the driver future returned (its
                        // sender dropped on completion); its result is
                        // recovered below if this arm beat the driver arm in
                        // the final select!.
                        None => break,
                    }
                }
            }
        }

        // The loop can exit via the channel-close arm while the driver arm was
        // ready but not selected; a closed channel means the driver future
        // already returned (its sender is dropped), so this await resolves
        // immediately and only recovers the result — never losing an error.
        let driver_result = match driver_result {
            Some(res) => res,
            None => driver.await,
        };
        driver_result?;

        let row_count = gate.forwarded_count();
        Ok(PagedQueryOutcome {
            row_count,
            has_more: gate.has_more(),
            capped: page_end_capped(offset, row_count),
            paged: true,
        })
    }

    /// Run the wrapped page SELECT through the same per-driver `execute_select`
    /// functions [`Self::execute_query`] uses, including `active_backends`
    /// registration so server-side cancel works (mirrors `execute_query`).
    async fn execute_wrapped_page_select(
        &self,
        connection_id: &str,
        pool: DatabasePool,
        sql: &str,
        sender: tokio::sync::mpsc::Sender<QueryEvent>,
        start: Instant,
    ) -> Result<(), CoreError> {
        let result = match pool {
            DatabasePool::Postgres(p) => {
                let mut register = |pid: i32| {
                    self.active_backends
                        .insert(connection_id.to_string(), BackendCancelId::Postgres(pid));
                };
                postgres::execute_select(&p, sql, sender, start, Some(&mut register)).await
            }
            DatabasePool::MySql(p) => {
                let mut register = |id: u64| {
                    self.active_backends
                        .insert(connection_id.to_string(), BackendCancelId::MySql(id));
                };
                mysql::execute_select(&p, sql, sender, start, Some(&mut register)).await
            }
            DatabasePool::Sqlite(p) => sqlite::execute_select(&p, sql, sender, start).await,
            DatabasePool::Any(p) => any::execute_select(&p, sql, sender, start).await,
            DatabasePool::Mssql(p) => mssql::execute_select(&p, sql, sender, start).await,
            DatabasePool::Oracle(p) => oracle::execute_select(&p, sql, sender, start).await,
            DatabasePool::ClickHouse(p) => clickhouse::execute_select(&p, sql, sender, start).await,
        };
        self.active_backends.remove(connection_id);
        result
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
            .ok_or_else(|| CoreError {
                message: "Not connected".into(),
                code: "NO_CONNECTION".into(),
            })?
            .clone();

        match pool {
            DatabasePool::Postgres(p) => fetch_schema_postgres(&p, table_name, schema_name).await,
            DatabasePool::MySql(p) => fetch_schema_mysql(&p, table_name).await,
            DatabasePool::Sqlite(p) => fetch_schema_sqlite(&p, table_name).await,
            DatabasePool::Any(_)
            | DatabasePool::Mssql(_)
            | DatabasePool::Oracle(_)
            | DatabasePool::ClickHouse(_) => Err(CoreError {
                message: "Schema metadata not supported for this connection type".into(),
                code: "UNSUPPORTED".into(),
            }),
        }
    }

    pub async fn get_schemas(&self, connection_id: &str) -> Result<Vec<SchemaInfo>, CoreError> {
        let pool = self
            .pools
            .get(connection_id)
            .ok_or_else(|| CoreError {
                message: "Not connected".into(),
                code: "NO_CONNECTION".into(),
            })?
            .clone();
        match pool {
            DatabasePool::Postgres(p) => get_schemas_postgres(&p).await,
            DatabasePool::MySql(p) => get_schemas_mysql(&p).await,
            DatabasePool::Sqlite(_) => Ok(vec![SchemaInfo {
                name: "main".into(),
                is_default: true,
            }]),
            DatabasePool::Mssql(p) => mssql::get_schemas(&p).await,
            DatabasePool::Oracle(p) => oracle::get_schemas(&p).await,
            DatabasePool::ClickHouse(p) => clickhouse::get_schemas(&p).await,
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
        let pool = self
            .pools
            .get(connection_id)
            .ok_or_else(|| CoreError {
                message: "Not connected".into(),
                code: "NO_CONNECTION".into(),
            })?
            .clone();
        match pool {
            DatabasePool::Postgres(p) => get_tables_postgres(&p, schema).await,
            DatabasePool::MySql(p) => get_tables_mysql(&p, schema).await,
            DatabasePool::Sqlite(p) => get_tables_sqlite(&p).await,
            DatabasePool::Mssql(p) => mssql::get_tables(&p, schema).await,
            DatabasePool::Oracle(p) => oracle::get_tables(&p, schema).await,
            DatabasePool::ClickHouse(p) => clickhouse::get_tables(&p, schema).await,
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
        let pool = self
            .pools
            .get(connection_id)
            .ok_or_else(|| CoreError {
                message: "Not connected".into(),
                code: "NO_CONNECTION".into(),
            })?
            .clone();
        match pool {
            DatabasePool::Postgres(p) => get_columns_postgres(&p, table_name, schema).await,
            DatabasePool::MySql(p) => get_columns_mysql(&p, table_name).await,
            DatabasePool::Sqlite(p) => get_columns_sqlite(&p, table_name).await,
            DatabasePool::Mssql(p) => mssql::get_columns(&p, table_name, schema).await,
            DatabasePool::Oracle(p) => oracle::get_columns(&p, table_name, schema).await,
            DatabasePool::ClickHouse(p) => clickhouse::get_columns(&p, table_name, schema).await,
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
        let pool = self
            .pools
            .get(connection_id)
            .ok_or_else(|| CoreError {
                message: "Not connected".into(),
                code: "NO_CONNECTION".into(),
            })?
            .clone();

        // First fetch column info so we can validate sort/filter column names
        let columns_info = match &pool {
            DatabasePool::Postgres(p) => {
                get_columns_postgres(p, &params.table_name, params.schema.as_deref()).await?
            }
            DatabasePool::MySql(p) => get_columns_mysql(p, &params.table_name).await?,
            DatabasePool::Sqlite(p) => get_columns_sqlite(p, &params.table_name).await?,
            DatabasePool::Mssql(p) => {
                mssql::get_columns(p, &params.table_name, params.schema.as_deref()).await?
            }
            DatabasePool::ClickHouse(p) => {
                clickhouse::get_columns(p, &params.table_name, params.schema.as_deref()).await?
            }
            DatabasePool::Oracle(p) => {
                oracle::get_columns(p, &params.table_name, params.schema.as_deref()).await?
            }
            DatabasePool::Any(_) => {
                return Err(CoreError {
                    message: "Table query not supported for this connection type".into(),
                    code: "UNSUPPORTED".into(),
                })
            }
        };

        let valid_columns: Vec<&str> = columns_info.iter().map(|c| c.name.as_str()).collect();
        let col_names: Vec<String> = columns_info.iter().map(|c| c.name.clone()).collect();
        let col_types: Vec<String> = columns_info.iter().map(|c| c.data_type.clone()).collect();

        match pool {
            DatabasePool::Postgres(p) => {
                query_table_postgres(&p, params, &valid_columns, col_names, col_types).await
            }
            DatabasePool::MySql(p) => {
                query_table_mysql(&p, params, &valid_columns, col_names, col_types).await
            }
            DatabasePool::Sqlite(p) => {
                query_table_sqlite(&p, params, &valid_columns, col_names, col_types).await
            }
            DatabasePool::Mssql(p) => {
                mssql::query_table(&p, params, &valid_columns, col_names, col_types).await
            }
            DatabasePool::ClickHouse(p) => {
                clickhouse::query_table(&p, params, &valid_columns, col_names, col_types).await
            }
            DatabasePool::Oracle(p) => {
                oracle::query_table(&p, params, &valid_columns, col_names, col_types).await
            }
            DatabasePool::Any(_) => unreachable!(),
        }
    }

    pub async fn get_ddl(
        &self,
        connection_id: &str,
        table_name: &str,
        schema: Option<&str>,
    ) -> Result<String, CoreError> {
        let pool = self
            .pools
            .get(connection_id)
            .ok_or_else(|| CoreError {
                message: "Not connected".into(),
                code: "NO_CONNECTION".into(),
            })?
            .clone();
        match pool {
            DatabasePool::Postgres(p) => postgres::get_ddl(&p, table_name, schema).await,
            DatabasePool::MySql(p) => mysql::get_ddl(&p, table_name, schema).await,
            DatabasePool::Sqlite(p) => sqlite::get_ddl(&p, table_name).await,
            DatabasePool::Mssql(p) => mssql::get_ddl(&p, table_name, schema).await,
            DatabasePool::Oracle(p) => oracle::get_ddl(&p, table_name, schema).await,
            DatabasePool::ClickHouse(p) => clickhouse::get_ddl(&p, table_name, schema).await,
            DatabasePool::Any(_) => Err(CoreError {
                message: "DDL retrieval not supported for this connection type".into(),
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
            .ok_or_else(|| CoreError {
                message: "Not connected".into(),
                code: "NO_CONNECTION".into(),
            })?
            .clone();

        match pool {
            DatabasePool::Postgres(p) => execute_batch_postgres(&p, batch).await,
            DatabasePool::MySql(p) => execute_batch_mysql(&p, batch).await,
            DatabasePool::Sqlite(p) => execute_batch_sqlite(&p, batch).await,
            DatabasePool::Any(p) => execute_batch_any(&p, batch).await,
            DatabasePool::Mssql(_) => Err(CoreError {
                message: "Batch execution not yet supported for MS SQL Server".into(),
                code: "UNSUPPORTED".into(),
            }),
            DatabasePool::Oracle(_) => Err(CoreError {
                message: "Batch execution not yet supported for Oracle".into(),
                code: "UNSUPPORTED".into(),
            }),
            DatabasePool::ClickHouse(_) => Err(CoreError {
                message: "Batch execution not yet supported for ClickHouse".into(),
                code: "UNSUPPORTED".into(),
            }),
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
        "mssql" | "sqlserver" | "tds" => Some(DatabaseType::Mssql),
        "oracle" => Some(DatabaseType::Oracle),
        "clickhouse" => Some(DatabaseType::ClickHouse),
        _ => None,
    }
}

async fn create_pool_for_url(url: &str) -> Result<DatabasePool, CoreError> {
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
        Some(DatabaseType::Mssql) => {
            let pool = mssql::create_pool(url).await?;
            Ok(DatabasePool::Mssql(pool))
        }
        Some(DatabaseType::Oracle) => {
            let pool = oracle::create_pool(url).await?;
            Ok(DatabasePool::Oracle(pool))
        }
        Some(DatabaseType::ClickHouse) => {
            let pool = clickhouse::create_pool(url).await?;
            Ok(DatabasePool::ClickHouse(pool))
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
        DatabasePool::Mssql(_) => {} // Arc<Mutex<Client>> drops naturally; tiberius sends logout on drop
        DatabasePool::Oracle(_) => {} // deadpool Pool drops naturally; connections closed on drop
        DatabasePool::ClickHouse(_) => {} // Arc<ClickHouseClient> drops naturally; reqwest Client is shared
    }
}

// ── Paged query execution helpers (U1) ───────────────────────────────────────

fn paged_dialect_for_pool(pool: &DatabasePool) -> PagedDialect {
    match pool {
        DatabasePool::Postgres(_) => PagedDialect::Postgres,
        DatabasePool::MySql(_) => PagedDialect::MySql,
        DatabasePool::Sqlite(_) => PagedDialect::Sqlite,
        DatabasePool::Any(_) => PagedDialect::Any,
        DatabasePool::Mssql(_) => PagedDialect::Mssql,
        DatabasePool::Oracle(_) => PagedDialect::Oracle,
        DatabasePool::ClickHouse(_) => PagedDialect::ClickHouse,
    }
}

/// Sentinel-row gate for paged execution (KTD-2): forwards the first `limit`
/// row events and passes `Columns`/`Error` through unchanged, drops the
/// `limit + 1`-th (sentinel) row, and rewrites `Done`'s `row_count` to the
/// forwarded count so the caller's event contract matches an unpaged run.
struct SentinelPageGate {
    limit: usize,
    seen_rows: usize,
    forwarded_rows: usize,
}

impl SentinelPageGate {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            seen_rows: 0,
            forwarded_rows: 0,
        }
    }

    /// Consume one driver event; `Some` means forward it, `None` means drop it
    /// (the sentinel row).
    fn observe(&mut self, ev: QueryEvent) -> Option<QueryEvent> {
        match ev {
            QueryEvent::Row { values } => {
                self.seen_rows += 1;
                if self.seen_rows <= self.limit {
                    self.forwarded_rows += 1;
                    Some(QueryEvent::Row { values })
                } else {
                    None
                }
            }
            QueryEvent::Done { duration_ms, .. } => Some(QueryEvent::Done {
                row_count: self.forwarded_rows,
                duration_ms,
            }),
            other => Some(other),
        }
    }

    /// Has-more iff the sentinel row arrived (stream exceeded `limit`).
    fn has_more(&self) -> bool {
        self.seen_rows > self.limit
    }

    fn forwarded_count(&self) -> usize {
        self.forwarded_rows
    }
}

/// Clamp the requested page size to the sentinel-safe bound (KTD-4).
fn clamp_page_size(limit: usize) -> usize {
    limit.clamp(1, PAGED_MAX_PAGE_SIZE)
}

/// Offsets at or beyond the ceiling are refused without a driver call (R5).
fn offset_at_ceiling(offset: usize) -> bool {
    offset >= PAGED_ROW_CEILING
}

/// A page whose end reaches or crosses the ceiling reports `capped` (R5).
fn page_end_capped(offset: usize, forwarded_rows: usize) -> bool {
    offset + forwarded_rows >= PAGED_ROW_CEILING
}

async fn execute_statement_pg(
    pool: &PgPool,
    sql: &str,
    sender: tokio::sync::mpsc::Sender<QueryEvent>,
    start: Instant,
    mut register_backend: Option<&mut (dyn FnMut(i32) + Send)>,
) -> Result<(), CoreError> {
    let mut conn = pool.acquire().await.map_err(|e| CoreError {
        message: e.to_string(),
        code: "ACQUIRE".into(),
    })?;

    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *conn)
        .await
        .map_err(|e| CoreError {
            message: e.to_string(),
            code: "BACKEND_PID".into(),
        })?;
    if let Some(reg) = register_backend.as_mut() {
        reg(pid);
    }

    match sqlx::query(sql).execute(&mut *conn).await {
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

async fn execute_statement_mysql(
    pool: &MySqlPool,
    sql: &str,
    sender: tokio::sync::mpsc::Sender<QueryEvent>,
    start: Instant,
    mut register_backend: Option<&mut (dyn FnMut(u64) + Send)>,
) -> Result<(), CoreError> {
    let mut conn = pool.acquire().await.map_err(|e| CoreError {
        message: e.to_string(),
        code: "ACQUIRE".into(),
    })?;

    let id: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()")
        .fetch_one(&mut *conn)
        .await
        .map_err(|e| CoreError {
            message: e.to_string(),
            code: "CONNECTION_ID".into(),
        })?;
    if let Some(reg) = register_backend.as_mut() {
        reg(id);
    }

    match sqlx::query(sql).execute(&mut *conn).await {
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
            let _ = sender
                .send(QueryEvent::Error {
                    message: e.to_string(),
                })
                .await;
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

    Ok(rows
        .iter()
        .map(|r| SchemaInfo {
            name: r.get("schema_name"),
            is_default: r.try_get::<bool, _>("is_default").unwrap_or(false),
        })
        .collect())
}

async fn get_schemas_mysql(pool: &MySqlPool) -> Result<Vec<SchemaInfo>, CoreError> {
    // MySQL 8 returns information_schema strings as VARBINARY via prepared statements;
    // CAST to CHAR so SQLx can decode them as String.
    let rows = sqlx::query(
        r#"SELECT CAST(SCHEMA_NAME AS CHAR) AS schema_name,
            (SCHEMA_NAME = DATABASE()) AS is_default
        FROM information_schema.SCHEMATA
        WHERE SCHEMA_NAME NOT IN ('information_schema', 'performance_schema', 'mysql', 'sys')
        ORDER BY is_default DESC, SCHEMA_NAME"#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| CoreError {
        message: e.to_string(),
        code: "SCHEMA_QUERY".into(),
    })?;

    Ok(rows
        .iter()
        .map(|r| SchemaInfo {
            name: r.get("schema_name"),
            is_default: r
                .try_get::<i64, _>("is_default")
                .map(|v| v != 0)
                .unwrap_or(false),
        })
        .collect())
}

async fn get_tables_postgres(
    pool: &PgPool,
    schema: Option<&str>,
) -> Result<Vec<TableInfo>, CoreError> {
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
    .map_err(|e| CoreError {
        message: e.to_string(),
        code: "SCHEMA_QUERY".into(),
    })?;

    Ok(rows
        .iter()
        .map(|r| {
            let name: String = r.get("table_name");
            let raw_type: String = r.get("table_type");
            let table_type = if raw_type == "VIEW" {
                "view".into()
            } else {
                "table".into()
            };
            TableInfo {
                full_name: format!("{}.{}", schema, name),
                name,
                schema: Some(schema.into()),
                table_type,
            }
        })
        .collect())
}

async fn get_tables_mysql(
    pool: &MySqlPool,
    schema: Option<&str>,
) -> Result<Vec<TableInfo>, CoreError> {
    // CAST to CHAR: MySQL 8 returns information_schema strings as VARBINARY via prepared stmts.
    let (sql, schema_val) = if let Some(s) = schema {
        (
            r#"SELECT CAST(TABLE_NAME AS CHAR) AS table_name,
                CAST(TABLE_TYPE AS CHAR) AS table_type,
                CAST(TABLE_SCHEMA AS CHAR) AS table_schema
            FROM information_schema.TABLES
            WHERE TABLE_SCHEMA = ? AND TABLE_TYPE IN ('BASE TABLE', 'VIEW')
            ORDER BY TABLE_NAME"#,
            s.to_string(),
        )
    } else {
        (
            r#"SELECT CAST(TABLE_NAME AS CHAR) AS table_name,
                CAST(TABLE_TYPE AS CHAR) AS table_type,
                CAST(TABLE_SCHEMA AS CHAR) AS table_schema
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
    .map_err(|e| CoreError {
        message: e.to_string(),
        code: "SCHEMA_QUERY".into(),
    })?;

    Ok(rows
        .iter()
        .map(|r| {
            let name: String = r.get("table_name");
            let raw_type: String = r.get("table_type");
            let schema_name: String = r.get("table_schema");
            let table_type = if raw_type == "VIEW" {
                "view".into()
            } else {
                "table".into()
            };
            TableInfo {
                full_name: format!("`{}`.`{}`", schema_name, name),
                name,
                schema: Some(schema_name),
                table_type,
            }
        })
        .collect())
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
    .map_err(|e| CoreError {
        message: e.to_string(),
        code: "SCHEMA_QUERY".into(),
    })?;

    Ok(rows
        .iter()
        .map(|r| {
            let name: String = r.get("name");
            let table_type: String = r.get("type");
            TableInfo {
                full_name: format!("\"{}\"", name),
                name: name.clone(),
                schema: None,
                table_type,
            }
        })
        .collect())
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

    Ok(rows
        .iter()
        .map(|r| SchemaColumnInfo {
            name: r.get("column_name"),
            data_type: map_pg_type(&r.get::<String, _>("data_type")),
            nullable: r.get::<&str, _>("is_nullable") == "YES",
            default_value: r.get("column_default"),
            is_primary_key: r.try_get("is_primary_key").unwrap_or(false),
            is_foreign_key: r.try_get("is_foreign_key").unwrap_or(false),
            foreign_table: r.try_get("foreign_table_name").ok().flatten(),
            foreign_column: r.try_get("foreign_column_name").ok().flatten(),
            ordinal_position: r.get::<i32, _>("ordinal_position"),
        })
        .collect())
}

async fn get_columns_mysql(
    pool: &MySqlPool,
    table_name: &str,
) -> Result<Vec<SchemaColumnInfo>, CoreError> {
    // CAST to CHAR: MySQL 8 returns information_schema strings as VARBINARY via prepared stmts.
    let col_rows = sqlx::query(
        r#"SELECT
            CAST(c.COLUMN_NAME AS CHAR) AS col_name,
            CAST(c.DATA_TYPE AS CHAR) AS data_type,
            CAST(c.IS_NULLABLE AS CHAR) AS is_nullable,
            CAST(c.COLUMN_DEFAULT AS CHAR) AS col_default,
            c.ORDINAL_POSITION AS ordinal_pos,
            (c.COLUMN_KEY = 'PRI') AS is_primary_key,
            (c.COLUMN_KEY = 'MUL') AS is_foreign_key
        FROM information_schema.COLUMNS c
        WHERE c.TABLE_SCHEMA = DATABASE() AND c.TABLE_NAME = ?
        ORDER BY c.ORDINAL_POSITION"#,
    )
    .bind(table_name)
    .fetch_all(pool)
    .await
    .map_err(|e| CoreError {
        message: e.to_string(),
        code: "SCHEMA_QUERY".into(),
    })?;

    // Fetch FK references
    let fk_rows = sqlx::query(
        r#"SELECT CAST(COLUMN_NAME AS CHAR) AS col_name,
            CAST(REFERENCED_TABLE_NAME AS CHAR) AS ref_table,
            CAST(REFERENCED_COLUMN_NAME AS CHAR) AS ref_col
        FROM information_schema.KEY_COLUMN_USAGE
        WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ?
            AND REFERENCED_TABLE_NAME IS NOT NULL"#,
    )
    .bind(table_name)
    .fetch_all(pool)
    .await
    .map_err(|e| CoreError {
        message: e.to_string(),
        code: "SCHEMA_QUERY".into(),
    })?;

    let fk_map: HashMap<String, (String, String)> = fk_rows
        .iter()
        .filter_map(|r| {
            let col: String = r.get("col_name");
            let ref_table: Option<String> = r.get("ref_table");
            let ref_col: Option<String> = r.get("ref_col");
            if let (Some(t), Some(c)) = (ref_table, ref_col) {
                Some((col, (t, c)))
            } else {
                None
            }
        })
        .collect();

    Ok(col_rows
        .iter()
        .map(|r| {
            let col_name: String = r.get("col_name");
            let fk = fk_map.get(&col_name);
            SchemaColumnInfo {
                name: col_name.clone(),
                data_type: map_mysql_type(&r.get::<String, _>("data_type")),
                nullable: r.get::<String, _>("is_nullable") == "YES",
                default_value: r.get("col_default"),
                is_primary_key: r
                    .try_get::<i64, _>("is_primary_key")
                    .map(|v| v != 0)
                    .unwrap_or(false),
                is_foreign_key: fk.is_some(),
                foreign_table: fk.map(|(t, _)| t.clone()),
                foreign_column: fk.map(|(_, c)| c.clone()),
                ordinal_position: r.try_get::<i64, _>("ordinal_pos").unwrap_or(0) as i32,
            }
        })
        .collect())
}

async fn get_columns_sqlite(
    pool: &SqlitePool,
    table_name: &str,
) -> Result<Vec<SchemaColumnInfo>, CoreError> {
    let safe_name = table_name.replace('"', "\"\"");
    let pragma_sql = format!("PRAGMA table_info(\"{}\")", safe_name);
    let rows = sqlx::query(&pragma_sql)
        .fetch_all(pool)
        .await
        .map_err(|e| CoreError {
            message: e.to_string(),
            code: "SCHEMA_QUERY".into(),
        })?;

    let fk_sql = format!("PRAGMA foreign_key_list(\"{}\")", safe_name);
    let fk_rows = sqlx::query(&fk_sql)
        .fetch_all(pool)
        .await
        .unwrap_or_default();

    let fk_map: HashMap<String, (String, String)> = fk_rows
        .iter()
        .filter_map(|r| {
            let from: String = r.try_get("from").ok()?;
            let table: String = r.try_get("table").ok()?;
            let to: String = r.try_get("to").ok()?;
            Some((from, (table, to)))
        })
        .collect();

    Ok(rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
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
        })
        .collect())
}

// ── Query Table helpers ────────────────────────────────────────────────────────

pub(crate) fn validate_column(name: &str, valid: &[&str]) -> bool {
    valid.contains(&name)
}

pub(crate) fn build_order_by_pg(sort: &[SortSpec], valid: &[&str]) -> String {
    if sort.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = sort
        .iter()
        .filter(|s| validate_column(&s.column, valid))
        .map(|s| {
            format!(
                "\"{}\" {}",
                s.column.replace('"', "\"\""),
                if s.desc {
                    "DESC NULLS LAST"
                } else {
                    "ASC NULLS LAST"
                }
            )
        })
        .collect();
    if parts.is_empty() {
        return String::new();
    }
    format!(" ORDER BY {}", parts.join(", "))
}

pub(crate) fn build_order_by_generic(sort: &[SortSpec], valid: &[&str], quote: char) -> String {
    if sort.is_empty() {
        return String::new();
    }
    let q = quote;
    let parts: Vec<String> = sort
        .iter()
        .filter(|s| validate_column(&s.column, valid))
        .map(|s| {
            format!(
                "{}{}{} {}",
                q,
                s.column.replace(q, &format!("{}{}", q, q)),
                q,
                if s.desc { "DESC" } else { "ASC" }
            )
        })
        .collect();
    if parts.is_empty() {
        return String::new();
    }
    format!(" ORDER BY {}", parts.join(", "))
}

// Build WHERE clause with positional placeholders, returns (clause, values)
fn build_where_clause(
    filters: &[FilterSpec],
    valid: &[&str],
    placeholder_start: usize,
    positional: bool, // true for PG ($1), false for ?
) -> (String, Vec<serde_json::Value>) {
    let active: Vec<&FilterSpec> = filters
        .iter()
        .filter(|f| validate_column(&f.column, valid))
        .collect();

    if active.is_empty() {
        return (String::new(), vec![]);
    }

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
                let ph = if positional {
                    format!("${}", idx)
                } else {
                    "?".into()
                };
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

    if parts.is_empty() {
        return (String::new(), vec![]);
    }
    (format!(" WHERE {}", parts.join(" AND ")), values)
}

// Non-ILIKE version for MySQL/SQLite
fn build_where_clause_like(
    filters: &[FilterSpec],
    valid: &[&str],
    quote: char,
) -> (String, Vec<serde_json::Value>) {
    let active: Vec<&FilterSpec> = filters
        .iter()
        .filter(|f| validate_column(&f.column, valid))
        .collect();

    if active.is_empty() {
        return (String::new(), vec![]);
    }

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
                    "equals" => {
                        parts.push(format!("{} = ?", col));
                        values.push(val.clone());
                    }
                    "gt" => {
                        parts.push(format!("{} > ?", col));
                        values.push(val.clone());
                    }
                    "gte" => {
                        parts.push(format!("{} >= ?", col));
                        values.push(val.clone());
                    }
                    "lt" => {
                        parts.push(format!("{} < ?", col));
                        values.push(val.clone());
                    }
                    "lte" => {
                        parts.push(format!("{} <= ?", col));
                        values.push(val.clone());
                    }
                    _ => continue,
                }
            }
        }
    }

    if parts.is_empty() {
        return (String::new(), vec![]);
    }
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
    let table_quoted = format!(
        "\"{}\".\"{}\"",
        schema.replace('"', "\"\""),
        params.table_name.replace('"', "\"\"")
    );

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
                if let Some(i) = n.as_i64() {
                    q.bind(i)
                } else if let Some(f) = n.as_f64() {
                    q.bind(f)
                } else {
                    q.bind(n.to_string())
                }
            }
            serde_json::Value::String(s) => q.bind(s.clone()),
            _ => q.bind(val.to_string()),
        };
    }
    q = q.bind(limit).bind(params.offset);

    let rows = q.fetch_all(pool).await.map_err(|e| CoreError {
        message: e.to_string(),
        code: "QUERY_TABLE".into(),
    })?;

    let has_more = rows.len() as i64 > params.limit.min(1000);
    let rows_to_use = if has_more {
        &rows[..rows.len() - 1]
    } else {
        &rows[..]
    };

    let result_rows: Vec<serde_json::Value> = rows_to_use
        .iter()
        .map(|row| {
            let mut obj = serde_json::Map::new();
            for (i, col) in col_names.iter().enumerate() {
                obj.insert(col.clone(), postgres::pg_row_to_json(row, i));
            }
            serde_json::Value::Object(obj)
        })
        .collect();

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
        format!(
            "`{}`.`{}`",
            schema.replace('`', "``"),
            params.table_name.replace('`', "``")
        )
    };

    let (where_clause, filter_vals) = build_where_clause_like(&params.filters, valid_columns, '`');
    let order_clause = build_order_by_generic(&params.sort, valid_columns, '`');
    let limit = params.limit.min(1000) + 1;

    let sql = format!(
        "SELECT * FROM {}{}{} LIMIT ? OFFSET ?",
        table_quoted, where_clause, order_clause
    );

    let mut q = sqlx::query(&sql);
    for val in &filter_vals {
        q = match val {
            serde_json::Value::Null => q.bind(None::<String>),
            serde_json::Value::Bool(b) => q.bind(*b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    q.bind(i)
                } else if let Some(f) = n.as_f64() {
                    q.bind(f)
                } else {
                    q.bind(n.to_string())
                }
            }
            serde_json::Value::String(s) => q.bind(s.clone()),
            _ => q.bind(val.to_string()),
        };
    }
    q = q.bind(limit).bind(params.offset);

    let rows = q.fetch_all(pool).await.map_err(|e| CoreError {
        message: e.to_string(),
        code: "QUERY_TABLE".into(),
    })?;

    let has_more = rows.len() as i64 > params.limit.min(1000);
    let rows_to_use = if has_more {
        &rows[..rows.len() - 1]
    } else {
        &rows[..]
    };

    let result_rows: Vec<serde_json::Value> = rows_to_use
        .iter()
        .map(|row| {
            let mut obj = serde_json::Map::new();
            for (i, col) in col_names.iter().enumerate() {
                obj.insert(col.clone(), mysql::mysql_row_to_json(row, i));
            }
            serde_json::Value::Object(obj)
        })
        .collect();

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

    let sql = format!(
        "SELECT * FROM {}{}{} LIMIT ? OFFSET ?",
        table_quoted, where_clause, order_clause
    );

    let mut q = sqlx::query(&sql);
    for val in &filter_vals {
        q = match val {
            serde_json::Value::Null => q.bind(None::<String>),
            serde_json::Value::Bool(b) => q.bind(*b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    q.bind(i)
                } else if let Some(f) = n.as_f64() {
                    q.bind(f)
                } else {
                    q.bind(n.to_string())
                }
            }
            serde_json::Value::String(s) => q.bind(s.clone()),
            _ => q.bind(val.to_string()),
        };
    }
    q = q.bind(limit).bind(params.offset);

    let rows = q.fetch_all(pool).await.map_err(|e| CoreError {
        message: e.to_string(),
        code: "QUERY_TABLE".into(),
    })?;

    let has_more = rows.len() as i64 > params.limit.min(1000);
    let rows_to_use = if has_more {
        &rows[..rows.len() - 1]
    } else {
        &rows[..]
    };

    let result_rows: Vec<serde_json::Value> = rows_to_use
        .iter()
        .map(|row| {
            let mut obj = serde_json::Map::new();
            for (i, col) in col_names.iter().enumerate() {
                obj.insert(col.clone(), sqlite::sqlite_row_to_json(row, i));
            }
            serde_json::Value::Object(obj)
        })
        .collect();

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
    if t.contains("INT") {
        return "integer".into();
    }
    if t.contains("CHAR") || t.contains("CLOB") || t.contains("TEXT") {
        return "text".into();
    }
    if t.contains("BLOB") || t.is_empty() {
        return "unknown".into();
    }
    if t.contains("REAL") || t.contains("FLOA") || t.contains("DOUB") {
        return "float".into();
    }
    if t.contains("BOOL") {
        return "boolean".into();
    }
    if t.contains("DATE") || t.contains("TIME") {
        return "timestamp".into();
    }
    if t.contains("NUMERIC") || t.contains("DECIMAL") {
        return "decimal".into();
    }
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
    .map_err(|e| CoreError {
        message: e.to_string(),
        code: "SCHEMA_QUERY".into(),
    })?;

    if col_rows.is_empty() {
        return Ok(TableMeta {
            table_name: table_name.to_string(),
            schema: Some(schema.to_string()),
            columns: vec![],
            primary_key: PrimaryKeyMeta {
                columns: vec![],
                exists: false,
            },
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
    .map_err(|e| CoreError {
        message: e.to_string(),
        code: "SCHEMA_QUERY".into(),
    })?;

    let pk_columns: Vec<String> = pk_rows.iter().map(|r| r.get("column_name")).collect();
    let pk_exists = !pk_columns.is_empty();

    let primary_key = PrimaryKeyMeta {
        columns: pk_columns,
        exists: pk_exists,
    };

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
        editability_reason: if pk_exists {
            None
        } else {
            Some("No primary key detected".into())
        },
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
    .map_err(|e| CoreError {
        message: e.to_string(),
        code: "SCHEMA_QUERY".into(),
    })?;

    if col_rows.is_empty() {
        return Ok(TableMeta {
            table_name: table_name.to_string(),
            schema: None,
            columns: vec![],
            primary_key: PrimaryKeyMeta {
                columns: vec![],
                exists: false,
            },
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
    .map_err(|e| CoreError {
        message: e.to_string(),
        code: "SCHEMA_QUERY".into(),
    })?;

    let pk_columns: Vec<String> = pk_rows.iter().map(|r| r.get("COLUMN_NAME")).collect();
    let pk_exists = !pk_columns.is_empty();
    let primary_key = PrimaryKeyMeta {
        columns: pk_columns,
        exists: pk_exists,
    };

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
        editability_reason: if pk_exists {
            None
        } else {
            Some("No primary key detected".into())
        },
    })
}

async fn fetch_schema_sqlite(pool: &SqlitePool, table_name: &str) -> Result<TableMeta, CoreError> {
    // PRAGMA table_info returns: cid, name, type, notnull, dflt_value, pk
    let pragma_sql = format!("PRAGMA table_info(\"{}\")", table_name.replace('"', "\"\""));
    let rows = sqlx::query(&pragma_sql)
        .fetch_all(pool)
        .await
        .map_err(|e| CoreError {
            message: e.to_string(),
            code: "SCHEMA_QUERY".into(),
        })?;

    if rows.is_empty() {
        return Ok(TableMeta {
            table_name: table_name.to_string(),
            schema: None,
            columns: vec![],
            primary_key: PrimaryKeyMeta {
                columns: vec![],
                exists: false,
            },
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
    let primary_key = PrimaryKeyMeta {
        columns: pk_col_names,
        exists: pk_exists,
    };

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
        editability_reason: if pk_exists {
            None
        } else {
            Some("No primary key detected".into())
        },
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
            serde_json::Value::Array(_) | serde_json::Value::Object(_) => q = q.bind(p.to_string()),
        }
    }
    q
}

async fn execute_batch_postgres(pool: &PgPool, batch: &SqlBatch) -> Result<BatchResult, CoreError> {
    let mut tx = pool.begin().await.map_err(|e| CoreError {
        message: e.to_string(),
        code: "TX_BEGIN".into(),
    })?;
    let mut executed = 0;
    let inserted_ids: HashMap<String, serde_json::Value> = HashMap::new();
    let total = batch.statements.len();

    for stmt in &batch.statements {
        let mut q = sqlx::query(&stmt.sql);
        for p in &stmt.params {
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

    tx.commit().await.map_err(|e| CoreError {
        message: e.to_string(),
        code: "TX_COMMIT".into(),
    })?;
    Ok(BatchResult {
        success: true,
        executed_count: executed,
        total_statements: total,
        error: None,
        inserted_ids,
    })
}

async fn execute_batch_mysql(pool: &MySqlPool, batch: &SqlBatch) -> Result<BatchResult, CoreError> {
    let mut tx = pool.begin().await.map_err(|e| CoreError {
        message: e.to_string(),
        code: "TX_BEGIN".into(),
    })?;
    let mut executed = 0;
    let total = batch.statements.len();

    for stmt in &batch.statements {
        let mut q = sqlx::query(&stmt.sql);
        for p in &stmt.params {
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

    tx.commit().await.map_err(|e| CoreError {
        message: e.to_string(),
        code: "TX_COMMIT".into(),
    })?;
    Ok(BatchResult {
        success: true,
        executed_count: executed,
        total_statements: total,
        error: None,
        inserted_ids: HashMap::new(),
    })
}

async fn execute_batch_sqlite(
    pool: &SqlitePool,
    batch: &SqlBatch,
) -> Result<BatchResult, CoreError> {
    let mut tx = pool.begin().await.map_err(|e| CoreError {
        message: e.to_string(),
        code: "TX_BEGIN".into(),
    })?;
    let mut executed = 0;
    let total = batch.statements.len();

    for stmt in &batch.statements {
        let mut q = sqlx::query(&stmt.sql);
        for p in &stmt.params {
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

    tx.commit().await.map_err(|e| CoreError {
        message: e.to_string(),
        code: "TX_COMMIT".into(),
    })?;
    Ok(BatchResult {
        success: true,
        executed_count: executed,
        total_statements: total,
        error: None,
        inserted_ids: HashMap::new(),
    })
}

async fn execute_batch_any(pool: &AnyPool, batch: &SqlBatch) -> Result<BatchResult, CoreError> {
    let mut tx = pool.begin().await.map_err(|e| CoreError {
        message: e.to_string(),
        code: "TX_BEGIN".into(),
    })?;
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

    tx.commit().await.map_err(|e| CoreError {
        message: e.to_string(),
        code: "TX_COMMIT".into(),
    })?;
    Ok(BatchResult {
        success: true,
        executed_count: executed,
        total_statements: total,
        error: None,
        inserted_ids: HashMap::new(),
    })
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
            let _ = sender
                .send(QueryEvent::Error {
                    message: e.to_string(),
                })
                .await;
        }
    }
    Ok(())
}

#[cfg(test)]
mod group_a_tests {
    use super::*;

    #[test]
    fn detect_database_type_every_scheme_alias() {
        assert_eq!(
            detect_database_type("postgres://h/db"),
            Some(DatabaseType::Postgres)
        );
        assert_eq!(
            detect_database_type("postgresql://h/db"),
            Some(DatabaseType::Postgres)
        );
        assert_eq!(
            detect_database_type("mysql://h/db"),
            Some(DatabaseType::MySql)
        );
        assert_eq!(
            detect_database_type("mariadb://h/db"),
            Some(DatabaseType::MySql)
        );
        assert_eq!(
            detect_database_type("sqlite:///tmp/x.db"),
            Some(DatabaseType::Sqlite)
        );
        assert_eq!(
            detect_database_type("mssql://h/db"),
            Some(DatabaseType::Mssql)
        );
        assert_eq!(
            detect_database_type("sqlserver://h/db"),
            Some(DatabaseType::Mssql)
        );
        assert_eq!(
            detect_database_type("tds://h/db"),
            Some(DatabaseType::Mssql)
        );
        assert_eq!(
            detect_database_type("oracle://h/db"),
            Some(DatabaseType::Oracle)
        );
        assert_eq!(
            detect_database_type("clickhouse://h/db"),
            Some(DatabaseType::ClickHouse)
        );
    }

    #[test]
    fn detect_database_type_unknown_scheme_is_none() {
        assert_eq!(detect_database_type("redis://h/db"), None);
        assert_eq!(detect_database_type("http://h/db"), None);
    }

    #[test]
    fn detect_database_type_malformed_without_scheme_separator() {
        // No "://" → split yields the whole string as scheme → unknown → None
        assert_eq!(detect_database_type("not-a-url"), None);
        assert_eq!(detect_database_type(""), None);
    }
}

#[cfg(test)]
mod group_b_tests {
    use super::*;
    use crate::models::QueryEvent;
    use std::time::{Duration, Instant};

    fn sqlite_url(path: &std::path::Path) -> String {
        // Absolute path + mode=rwc so sqlx creates the file if missing
        format!("sqlite://{}?mode=rwc", path.display())
    }

    async fn exec_sql(mgr: &DbManager, id: &str, sql: &str) -> Vec<QueryEvent> {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<QueryEvent>(32);
        mgr.execute_query(id, sql, tx)
            .await
            .unwrap_or_else(|e| panic!("execute_query failed: {e:?}"));
        let mut events = Vec::new();
        while let Some(ev) = rx.recv().await {
            events.push(ev);
        }
        events
    }

    // ── Connect timeout ───────────────────────────────────────────────────────

    /// Unreachable/blackholed host must fail with `code == "TIMEOUT"` in ~5s.
    /// Gated so bare `cargo test --workspace` stays fast; run with
    /// `SQLATOR_SLOW_TESTS=1`.
    #[tokio::test]
    async fn connect_unreachable_host_times_out_with_timeout_code() {
        if std::env::var("SQLATOR_SLOW_TESTS").ok().as_deref() != Some("1") {
            eprintln!("skipping connect timeout test; set SQLATOR_SLOW_TESTS=1 (~5s)");
            return;
        }

        let mgr = DbManager::new();
        // TEST-NET-1 — should not respond (blackhole / no route)
        let url = "postgres://sqlator@192.0.2.1:5432/sqlator";
        let start = Instant::now();
        let err = mgr
            .connect("timeout-probe", url)
            .await
            .expect_err("must time out");
        let elapsed = start.elapsed();

        assert_eq!(err.code, "TIMEOUT");
        assert!(
            elapsed >= Duration::from_secs(4) && elapsed < Duration::from_secs(10),
            "expected ~5s timeout, got {elapsed:?}"
        );
        assert!(!mgr.is_connected("timeout-probe"));
    }

    // ── Pool replacement ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn connect_same_id_replaces_old_pool() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path_a = dir.path().join("a.db");
        let path_b = dir.path().join("b.db");
        let url_a = sqlite_url(&path_a);
        let url_b = sqlite_url(&path_b);

        let mgr = DbManager::new();
        mgr.connect("c1", &url_a).await.expect("connect A");
        exec_sql(&mgr, "c1", "CREATE TABLE only_in_a (id INTEGER)").await;

        // Second connect on same id closes old pool and opens B
        mgr.connect("c1", &url_b).await.expect("connect B");
        assert!(mgr.is_connected("c1"));

        // Table from A must not be visible on B.
        // Note: sqlite SELECT errors are delivered as QueryEvent::Error with Ok(()) return.
        let events = exec_sql(&mgr, "c1", "SELECT * FROM only_in_a").await;
        assert!(
            events.iter().any(|e| matches!(e, QueryEvent::Error { .. })),
            "after pool replace, only_in_a must be missing on B; events={events:?}"
        );

        exec_sql(&mgr, "c1", "CREATE TABLE only_in_b (id INTEGER)").await;

        // Reconnect to A — only_in_a exists, proving A was a separate pool/file
        mgr.connect("c1", &url_a).await.expect("reconnect A");
        let events = exec_sql(&mgr, "c1", "SELECT * FROM only_in_a").await;
        assert!(
            events.iter().any(|e| matches!(e, QueryEvent::Done { .. })),
            "reconnected A must still have only_in_a; events={events:?}"
        );
        let events = exec_sql(&mgr, "c1", "SELECT * FROM only_in_b").await;
        assert!(
            events.iter().any(|e| matches!(e, QueryEvent::Error { .. })),
            "only_in_b must not exist on A; events={events:?}"
        );
    }
}

#[cfg(test)]
mod paged_tests {
    use super::*;
    use crate::models::QueryEvent;

    fn row(v: i64) -> QueryEvent {
        QueryEvent::Row {
            values: vec![serde_json::json!(v)],
        }
    }

    fn done(row_count: usize) -> QueryEvent {
        QueryEvent::Done {
            row_count,
            duration_ms: 7,
        }
    }

    // ── Sentinel-row gating (KTD-2) ───────────────────────────────────────────

    #[test]
    fn gate_forwards_exactly_limit_rows_when_stream_has_limit_plus_two() {
        let mut gate = SentinelPageGate::new(2);
        // Fake driver stream: Columns + 4 rows (limit+2) + Done.
        let columns = gate
            .observe(QueryEvent::Columns {
                names: vec!["n".into()],
            })
            .expect("Columns forwards");
        assert!(matches!(columns, QueryEvent::Columns { .. }));

        let mut forwarded = 0;
        for v in 0..4 {
            if gate.observe(row(v)).is_some() {
                forwarded += 1;
            }
        }
        assert_eq!(forwarded, 2, "sentinel rows must be dropped");
        assert!(gate.has_more(), "stream exceeded limit → has_more");
        assert_eq!(gate.forwarded_count(), 2);

        let done_ev = gate.observe(done(4)).expect("Done forwards");
        match done_ev {
            QueryEvent::Done {
                row_count,
                duration_ms,
            } => {
                assert_eq!(row_count, 2, "Done is rewritten to the forwarded count");
                assert_eq!(duration_ms, 7, "duration preserved");
            }
            _ => panic!("expected Done"),
        }
    }

    #[test]
    fn gate_forwards_all_when_stream_has_exactly_limit_rows() {
        let mut gate = SentinelPageGate::new(2);
        assert!(gate.observe(row(1)).is_some());
        assert!(gate.observe(row(2)).is_some());
        assert!(!gate.has_more(), "sentinel never arrived → no more pages");
        assert_eq!(gate.forwarded_count(), 2);
    }

    #[test]
    fn gate_empty_stream_forwards_zero_rows_no_columns_no_more() {
        let mut gate = SentinelPageGate::new(500);
        let done_ev = gate.observe(done(0)).expect("Done forwards");
        match done_ev {
            QueryEvent::Done { row_count, .. } => assert_eq!(row_count, 0),
            _ => panic!("expected Done"),
        }
        assert!(!gate.has_more(), "empty first page → has_more false");
        assert_eq!(gate.forwarded_count(), 0);
    }

    #[test]
    fn gate_forwards_error_events_unchanged() {
        let mut gate = SentinelPageGate::new(500);
        let ev = gate
            .observe(QueryEvent::Error {
                message: "boom".into(),
            })
            .expect("Error forwards");
        assert!(matches!(ev, QueryEvent::Error { .. }));
        assert!(!gate.has_more());
    }

    // ── Page-size clamp (KTD-4: page + sentinel must stay under driver cap) ───

    #[test]
    fn page_size_clamped_to_sentinel_safe_bound() {
        assert_eq!(clamp_page_size(500), 500);
        assert_eq!(clamp_page_size(999), 999);
        assert_eq!(
            clamp_page_size(1000),
            999,
            "limit+1 must fit the driver send-cap"
        );
        assert_eq!(clamp_page_size(usize::MAX), 999);
        assert_eq!(clamp_page_size(0), 1, "zero page size would loop forever");
    }

    // ── Ceiling (KTD-5 / R5) ──────────────────────────────────────────────────

    #[test]
    fn offset_at_or_over_ceiling_short_circuits() {
        assert!(!offset_at_ceiling(PAGED_ROW_CEILING - 1));
        assert!(offset_at_ceiling(PAGED_ROW_CEILING));
        assert!(offset_at_ceiling(PAGED_ROW_CEILING + 1));
        assert_eq!(PAGED_ROW_CEILING, 50_000);
    }

    #[test]
    fn page_end_reaching_or_crossing_ceiling_is_capped() {
        // Rows land exactly on the ceiling → capped.
        assert!(page_end_capped(49_500, 500));
        // Rows cross the ceiling → capped.
        assert!(page_end_capped(49_900, 500));
        // Page fully below the ceiling → not capped.
        assert!(!page_end_capped(0, 500));
        assert!(!page_end_capped(49_499, 500));
    }
}
