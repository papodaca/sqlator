---
title: Add Support for MS SQL Server, Oracle, and Additional SQL Databases
type: feat
status: active
date: 2026-04-05
origin: docs/plans/2026-04-05-002-refactor-database-specific-pools-plan.md
---

# Add Support for MS SQL Server, Oracle, and Additional SQL Databases

## Overview

Extend sqlator's database support beyond PostgreSQL, MySQL, and SQLite to include MS SQL Server, Oracle, and other SQL-based databases. This requires integrating non-sqlx drivers since sqlx doesn't support these databases.

## Problem Statement

Users need to connect to enterprise databases that sqlator doesn't currently support:

1. **MS SQL Server**: Widely used in enterprise Windows environments
2. **Oracle**: Standard in large enterprises, financial services
3. **Other SQL databases**: ClickHouse (analytics), CockroachDB (distributed SQL)

The database-specific pool refactor (002) established the `DatabasePool` enum pattern, making it straightforward to add new database types—provided we can find suitable Rust drivers.

### Critical Finding: sqlx Limitations

sqlx **does not support** MS SQL Server or Oracle:

| Database | sqlx Support | Reason |
|----------|--------------|--------|
| MSSQL | ❌ Removed in v0.7 | Moved to planned "SQLx Pro" (no timeline) |
| Oracle | ❌ Never supported | Proprietary protocol, only blocking OCI APIs |

**Source**: [sqlx Discussion #1616](https://github.com/launchbadge/sqlx/discussions/1616), [sqlx FAQ](https://github.com/launchbadge/sqlx/blob/main/FAQ.md)

## Proposed Solution

### Multi-Driver Architecture

Extend the existing `DatabasePool` enum to include non-sqlx pool types:

```rust
// core/src/db/mod.rs
#[derive(Clone)]
pub enum DatabasePool {
    // sqlx-based (existing)
    Postgres(PgPool),
    MySql(MySqlPool),
    Sqlite(SqlitePool),
    Any(AnyPool),
    
    // NEW: Non-sqlx drivers
    Mssql(MssqlPool),      // Tiberius + deadpool-tiberius
    Oracle(OraclePool),    // oracle-rs + deadpool-oracle
    ClickHouse(ChPool),    // clickhouse crate
}
```

### Database-Specific Implementation

| Database | Driver | Pool Type | Async | Pure Rust | Maturity |
|----------|--------|-----------|-------|-----------|----------|
| **MS SQL Server** | `tiberius` | `deadpool-tiberius` | ✅ | ✅ | Production (Prisma) |
| **Oracle** | `oracle-rs` | `deadpool-oracle` | ✅ | ✅ | Early (v0.1.x) |
| **ClickHouse** | `clickhouse` | Built-in | ✅ | ✅ | Stable |

### Connection String Detection

```rust
// core/src/db/mod.rs
pub fn detect_database_type(url: &str) -> Option<DatabaseType> {
    let scheme = url.split("://").next()?;
    match scheme {
        "postgres" | "postgresql" => Some(DatabaseType::Postgres),
        "mysql" | "mariadb" => Some(DatabaseType::MySql),
        "sqlite" => Some(DatabaseType::Sqlite),
        // NEW
        "mssql" | "sqlserver" | "tds" => Some(DatabaseType::Mssql),
        "oracle" => Some(DatabaseType::Oracle),
        "clickhouse" => Some(DatabaseType::ClickHouse),
        _ => None,
    }
}
```

## Technical Approach

### Phase 1: MS SQL Server (Tiberius)

**Why Tiberius**: Pure Rust, native async, production-ready (used by Prisma), no native dependencies.

**Dependencies**:
```toml
# core/Cargo.toml
[dependencies]
tiberius = { version = "0.12", default-features = false, features = ["rustls", "tds73"] }
deadpool-tiberius = "0.5"
tokio-util = { version = "0.7", features = ["compat"] }
```

**Module Structure** (`core/src/db/mssql.rs`):
```rust
use tiberius::{Client, Config, AuthMethod};
use deadpool_tiberius::Manager;

pub type MssqlPool = deadpool::managed::Pool<Manager>;

pub async fn execute_select(
    pool: &MssqlPool,
    sql: &str,
    sender: &mpsc::Sender<QueryEvent>,
) -> Result<(), CoreError> {
    let mut client = pool.get().await?;
    let stream = client.query(sql, &[]).await?;
    
    // Convert rows to JSON
    while let Some(row) = stream.into_row_stream().await?.next().await {
        let json_row = mssql_row_to_json(&row?);
        sender.send(QueryEvent::Row(json_row)).await?;
    }
    sender.send(QueryEvent::Done).await?;
    Ok(())
}

fn mssql_row_to_json(row: &tiberius::Row) -> serde_json::Value {
    // Tiberius has native JSON support for NVARCHAR(MAX) with JSON
    // Handle: BIT, INT, BIGINT, FLOAT, NVARCHAR, DATETIME, UNIQUEIDENTIFIER
}
```

**Connection String Formats**:
- `mssql://user:pass@host:1433/database`
- `sqlserver://user:pass@host:1433/database`
- ADO.NET format: `Server=host;Database=db;User Id=user;Password=pass;`

### Phase 2: Oracle (oracle-rs)

**Why oracle-rs**: Pure Rust, native async, no Oracle client dependencies (unlike OCI-based alternatives).

**Caveat**: Early stage (v0.1.x) - may have edge cases. Alternative: `sibyl` (mature, OCI-based, requires Oracle Instant Client).

**Dependencies**:
```toml
# core/Cargo.toml
[dependencies]
oracle-rs = { version = "0.1" }
deadpool-oracle = "0.1"
```

**Module Structure** (`core/src/db/oracle.rs`):
```rust
pub type OraclePool = deadpool::managed::Pool<OracleManager>;

pub async fn execute_select(
    pool: &OraclePool,
    sql: &str,
    sender: &mpsc::Sender<QueryEvent>,
) -> Result<(), CoreError> {
    let mut conn = pool.get().await?;
    let stmt = conn.execute(sql, &[]).await?;
    
    // Oracle uses :1, :2 parameter syntax
    // Handle: NUMBER, VARCHAR2, DATE, TIMESTAMP, CLOB, BLOB
}

fn oracle_row_to_json(row: &oracle_rs::Row) -> serde_json::Value {
    // Oracle-specific type handling
}
```

**Connection String Format**:
- `oracle://user:pass@host:1521/service_name`

### Phase 3: ClickHouse (Optional)

**Why ClickHouse**: Popular for analytics workloads, stable async Rust driver.

**Dependencies**:
```toml
# core/Cargo.toml
[dependencies]
clickhouse = { version = "0.13", features = ["lz4"] }
```

**Module Structure** (`core/src/db/clickhouse.rs`):
```rust
use clickhouse::Client;

pub async fn execute_select(
    client: &Client,
    sql: &str,
    sender: &mpsc::Sender<QueryEvent>,
) -> Result<(), CoreError> {
    // ClickHouse has native JSON output format
    let result = client
        .query(sql)
        .fetch_json::<serde_json::Value>()
        .await?;
    // ...
}
```

### Phase 4: PostgreSQL-Compatible Databases

These databases work with the existing PostgreSQL driver—no code changes needed:

| Database | Status | Notes |
|----------|--------|-------|
| **CockroachDB** | ✅ Works | Use `postgres://` connection string |
| **TimescaleDB** | ✅ Works | PostgreSQL extension |
| **Supabase** | ✅ Works | PostgreSQL-based |
| **Amazon RDS PostgreSQL** | ✅ Works | Standard PostgreSQL |
| **Google Cloud SQL PostgreSQL** | ✅ Works | Standard PostgreSQL |

### MySQL-Compatible Databases

| Database | Status | Notes |
|----------|--------|-------|
| **MariaDB** | ✅ Works | Use `mysql://` or `mariadb://` |
| **PlanetScale** | ✅ Works | MySQL-compatible serverless |
| **Amazon RDS MySQL** | ✅ Works | Standard MySQL |

## System-Wide Impact

### Interaction Graph

1. `connect_database` → `detect_database_type()` → Create appropriate pool type → Store in DashMap
2. `execute_query` → Match on `DatabasePool` variant → Delegate to database-specific handler
3. `disconnect_database` → Remove from DashMap → Pool cleanup (handled by Drop)

### Error Propagation

Each driver has different error types:

```rust
// core/src/error.rs
impl From<tiberius::error::Error> for CoreError {
    fn from(e: tiberius::error::Error) -> Self {
        CoreError::query_failed(&e.to_string())
    }
}

impl From<oracle_rs::Error> for CoreError {
    fn from(e: oracle_rs::Error) -> Self {
        CoreError::query_failed(&e.to_string())
    }
}
```

### State Lifecycle

- Same as existing: pool creation on connect, removal on disconnect
- No new state management complexity
- Connection pooling handled by deadpool (consistent pattern)

### API Surface Parity

| Interface | Current | After |
|-----------|---------|-------|
| `DatabaseType` enum | 3 variants | 6+ variants |
| `DatabasePool` enum | 4 variants | 7+ variants |
| `detect_database_type()` | 3 schemes | 6+ schemes |
| Type coercion | Per-database modules | New modules added |

## Acceptance Criteria

### Functional Requirements

- [ ] MS SQL Server connections work with `mssql://` scheme
- [ ] MS SQL Server JSON columns (NVARCHAR with JSON) display correctly
- [ ] Oracle connections work with `oracle://` scheme
- [ ] Oracle DATE, TIMESTAMP, NUMBER types display correctly
- [ ] ClickHouse connections work with `clickhouse://` scheme (optional)
- [ ] Connection strings validated and parsed correctly for each database
- [ ] Error messages are clear and database-specific
- [ ] UI displays correct database type icon for new databases

### Non-Functional Requirements

- [ ] No performance regression for existing databases
- [ ] Async operations remain non-blocking
- [ ] Connection timeout (5s) applies to new database types
- [ ] Binary size impact is acceptable (each driver adds ~500KB-1MB)

### Quality Gates

- [ ] `cargo build` passes with no warnings
- [ ] `cargo clippy` passes
- [ ] Manual testing with MS SQL Server (Docker container)
- [ ] Manual testing with Oracle (Docker container or Oracle Cloud Free Tier)
- [ ] Manual testing with ClickHouse (optional)

## Success Metrics

1. **MSSQL adoption**: Users can connect to SQL Server databases without workarounds
2. **Oracle connectivity**: Enterprise users can use sqlator for Oracle databases
3. **Zero regressions**: Existing PostgreSQL/MySQL/SQLite connections unchanged

## Dependencies & Risks

### Dependencies

| Dependency | Version | Purpose |
|------------|---------|---------|
| `tiberius` | 0.12.x | MS SQL Server TDS driver |
| `deadpool-tiberius` | 0.5.x | Connection pooling for Tiberius |
| `oracle-rs` | 0.1.x | Oracle TNS driver (pure Rust) |
| `deadpool-oracle` | 0.1.x | Connection pooling for Oracle |
| `clickhouse` | 0.13.x | ClickHouse client (optional) |

### Risks

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| `oracle-rs` immature (v0.1.x) | Medium | High | Offer `sibyl` as fallback (requires Oracle client) |
| Tiberius connection string formats vary | Low | Medium | Support multiple formats (URL, ADO.NET) |
| Type handling edge cases | Medium | Medium | Comprehensive testing with Docker containers |
| Binary size increase | High | Low | Make drivers optional features |

### Risk Mitigation: Feature Flags

Make database drivers optional to reduce binary size for users who don't need them:

```toml
# core/Cargo.toml
[features]
default = ["postgres", "mysql", "sqlite"]
postgres = ["sqlx/postgres"]
mysql = ["sqlx/mysql"]
sqlite = ["sqlx/sqlite"]
mssql = ["tiberius", "deadpool-tiberius", "tokio-util/compat"]
oracle = ["oracle-rs", "deadpool-oracle"]
clickhouse = ["clickhouse"]
```

## Implementation Outline

### Phase 1: MS SQL Server (Priority: High)

1. Add `tiberius` and `deadpool-tiberius` to Cargo.toml
2. Create `core/src/db/mssql.rs` module
3. Add `DatabaseType::Mssql` and `DatabasePool::Mssql` variants
4. Implement `detect_database_type()` for `mssql://` and `sqlserver://`
5. Implement `create_mssql_pool()` with connection string parsing
6. Implement `execute_select_mssql()` with row-to-JSON conversion
7. Add error type conversions
8. Test with Docker SQL Server container

**Estimated effort**: 2-3 days

### Phase 2: Oracle (Priority: Medium)

1. Add `oracle-rs` and `deadpool-oracle` to Cargo.toml
2. Create `core/src/db/oracle.rs` module
3. Add `DatabaseType::Oracle` and `DatabasePool::Oracle` variants
4. Implement connection string parsing for `oracle://`
5. Implement `execute_select_oracle()` with Oracle type handling
6. Handle Oracle-specific types (NUMBER, DATE, TIMESTAMP, CLOB)
7. Test with Oracle Docker container or Oracle Cloud Free Tier

**Estimated effort**: 2-3 days

### Phase 3: ClickHouse (Priority: Low, Optional)

1. Add `clickhouse` crate to Cargo.toml
2. Create `core/src/db/clickhouse.rs` module
3. Implement connection and query execution
4. Leverage ClickHouse's native JSON output format
5. Test with ClickHouse Docker container

**Estimated effort**: 1 day

### Phase 4: Documentation & Polish

1. Update README with supported database list
2. Add connection string examples for each database
3. Add database type icons to UI
4. Document known limitations per database

**Estimated effort**: 1 day

## Future Considerations

### SQLx Pro Integration

If SQLx Pro releases with MSSQL/Oracle support:
- Evaluate migration from Tiberius/oracle-rs to unified sqlx API
- Benefit: Compile-time query checking, consistent API
- Trade-off: License cost (AGPL or commercial)

### Additional Databases

| Database | Driver | Status |
|----------|--------|--------|
| **Cassandra/ScyllaDB** | `scylla` | Stable async driver |
| **MongoDB** | `mongodb` | Stable async driver (NoSQL) |
| **Redis** | `redis` | Stable async driver (not SQL) |
| **DuckDB** | `duckdb` | Embedded analytics DB |

### Connection Pooling Improvements

- Pool size configuration per database
- Connection health checks
- Automatic reconnection on pool exhaustion

## Sources & References

### Origin

- **Origin document**: [docs/plans/2026-04-05-002-refactor-database-specific-pools-plan.md](docs/plans/2026-04-05-002-refactor-database-specific-pools-plan.md) — Architecture established: `DatabasePool` enum, database-specific modules, dispatch pattern

### Internal References

- Database architecture: `core/src/db/mod.rs:12-29`
- Existing PostgreSQL handler: `core/src/db/postgres.rs:1-161`
- Type coercion pattern: `core/src/db/mysql.rs:1-140`
- Error handling: `core/src/error.rs:18-31`

### External References

- **Tiberius (MSSQL)**: https://docs.rs/tiberius — Pure Rust async TDS driver
- **Tiberius GitHub**: https://github.com/prisma/tiberius — Maintained by Prisma
- **oracle-rs**: https://docs.rs/oracle-rs — Pure Rust Oracle driver
- **ClickHouse crate**: https://docs.rs/clickhouse — Async ClickHouse client
- **sqlx MSSQL removal**: https://github.com/launchbadge/sqlx/discussions/1616 — Official announcement
- **sqlx FAQ (Oracle)**: https://github.com/launchbadge/sqlx/blob/main/FAQ.md — Why Oracle isn't supported

### Related Work

- Connection pooling pattern: `deadpool` crate family
- Docker testing: SQL Server, Oracle, ClickHouse all have official Docker images
