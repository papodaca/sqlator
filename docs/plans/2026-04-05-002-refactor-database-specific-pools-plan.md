---
title: Refactor to Database-Specific Connection Pools
type: refactor
status: completed
date: 2026-04-05
---

# Refactor to Database-Specific Connection Pools

## Overview

Replace `AnyPool` with database-specific pools (`PgPool`, `MySqlPool`, `SqlitePool`) to enable native type handling for database-specific types like JSONB, arrays, and geometry. Keep `AnyPool` as fallback for unknown database types.

## Problem Statement

Current implementation uses `sqlx::AnyPool` which has significant limitations:

1. **JSONB decode failures**: PostgreSQL JSONB columns cause runtime errors
2. **Trial-and-error type coercion**: `row_value_to_json()` attempts multiple type conversions, falling back to `<TYPE_NAME>` for unsupported types
3. **No database-specific features**: Cannot use native JSON, array, or geometry types
4. **User friction**: Users must manually cast columns like `props::text` in queries

**User goal**: Simple queries like `SELECT * FROM table` should "just work" without type coercion knowledge.

## Proposed Solution

### Architecture: Enum-Based Pool Storage

```rust
// core/src/db.rs
use sqlx::{PgPool, MySqlPool, SqlitePool, AnyPool};

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
```

### Database Type Detection

URL scheme detection already exists in `commands.rs:22-29`. Move this to core library:

```rust
// core/src/db.rs
pub fn detect_database_type(url: &str) -> Option<DatabaseType> {
    let scheme = url.split("://").next()?;
    match scheme {
        "postgres" | "postgresql" => Some(DatabaseType::Postgres),
        "mysql" | "mariadb" => Some(DatabaseType::MySql),
        "sqlite" => Some(DatabaseType::Sqlite),
        _ => None,
    }
}
```

### Query Execution Dispatch

Match on pool variant and delegate to database-specific handlers:

```rust
impl DbManager {
    pub async fn execute_query(
        &self,
        connection_id: &str,
        sql: &str,
        sender: tokio::sync::mpsc::Sender<QueryEvent>,
    ) -> Result<(), CoreError> {
        let pool = self.pools.get(connection_id).ok_or(...)?;
        
        match pool.value() {
            DatabasePool::Postgres(p) => self.execute_select_pg(p, sql, sender).await,
            DatabasePool::MySql(p) => self.execute_select_mysql(p, sql, sender).await,
            DatabasePool::Sqlite(p) => self.execute_select_sqlite(p, sql, sender).await,
            DatabasePool::Any(p) => self.execute_select_any(p, sql, sender).await,
        }
    }
}
```

### Type Handling Per Database

| Database | Native Types | Handling Strategy |
|----------|-------------|-------------------|
| PostgreSQL | JSONB, arrays, geo, UUID | Native decoding via `sqlx::types::Json`, `Vec<T>` |
| MySQL | JSON, datetime | Native decoding via `sqlx::types::Json` |
| SQLite | Limited types | Fallback to text representation |
| Any (fallback) | Basic types only | Existing trial-and-error coercion |

## Technical Considerations

### Database-Specific Type Coercion

**PostgreSQL** (`core/src/db/postgres.rs`):
```rust
fn pg_row_to_json(row: &PgRow, index: usize) -> serde_json::Value {
    // Try native types first
    if let Ok(v) = row.try_get::<Option<sqlx::types::Json<serde_json::Value>>, _>(index) {
        return v.map(|j| j.0).unwrap_or(serde_json::Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<Vec<String>>, _>(index) {
        return serde_json::json!(v);
    }
    // Fallback to standard types
    // ...
}
```

**MySQL** (`core/src/db/mysql.rs`):
```rust
fn mysql_row_to_json(row: &MySqlRow, index: usize) -> serde_json::Value {
    if let Ok(v) = row.try_get::<Option<sqlx::types::Json<serde_json::Value>>, _>(index) {
        return v.map(|j| j.0).unwrap_or(serde_json::Value::Null);
    }
    // ...
}
```

### MariaDB Handling

MariaDB uses `mysql://` scheme and shares MySQL protocol. Treat as MySQL with `MySqlPool`, but store `db_type: "mariadb"` for UI display if detected from connection string.

### Connection Timeout Consistency

Apply 5s timeout to `connect()` matching `test_connection()`:

```rust
pub async fn connect(&self, connection_id: &str, url: &str) -> Result<(), CoreError> {
    let pool = tokio::time::timeout(
        Duration::from_secs(5),
        self.create_pool(url)
    ).await.map_err(|_| CoreError::timeout())??;
    
    self.pools.insert(connection_id.to_string(), pool);
    Ok(())
}
```

### Unknown Scheme Fallback

When scheme is unrecognized, create `AnyPool` with warning:

```rust
match detect_database_type(url) {
    Some(DatabaseType::Postgres) => {
        let pool = PgPool::connect(url).await?;
        Ok(DatabasePool::Postgres(pool))
    }
    // ... other known types
    None => {
        // Log warning: using AnyPool fallback
        let pool = AnyPool::connect(url).await?;
        Ok(DatabasePool::Any(pool))
    }
}
```

## System-Wide Impact

### Interaction Graph

1. `connect_database` command → `DbManager::connect()` → URL parsing → Pool creation → DashMap insert
2. `execute_query` command → `DbManager::execute_query()` → Pool lookup → Match on variant → Database-specific execution
3. `disconnect_database` command → `DbManager::disconnect()` → DashMap remove → Pool close

### Error Propagation

- Pool creation errors → `CoreError::connection_failed()` with database-specific message
- Type decode errors → Wrapped in `CoreError` with consistent formatting
- Timeout errors → `CoreError::timeout()` (5s limit)

### State Lifecycle

- **Connection switch**: Old pool closed, new pool created, DashMap updated atomically
- **Partial failure**: If pool creation fails, DashMap remains unchanged (no orphan state)
- **Pool exhaustion**: sqlx queues requests; UI shows "executing..." state

### API Surface Parity

| Interface | Current | After Refactor |
|-----------|---------|----------------|
| `DbManager::connect()` | Takes URL, creates AnyPool | Takes URL, detects type, creates typed pool |
| `DbManager::execute_query()` | Single code path | Dispatch by pool variant |
| `commands.rs` | Detects db_type for storage | Pass db_type to core (optional optimization) |

## Acceptance Criteria

### Functional Requirements

- [ ] PostgreSQL JSONB columns display correctly without manual casting
- [ ] PostgreSQL array columns display as JSON arrays
- [ ] MySQL JSON columns display correctly
- [ ] SQLite queries work as before
- [ ] Unknown database schemes fall back to AnyPool
- [ ] MariaDB connections work via MySQL protocol
- [ ] Connection timeout is consistent (5s) across all methods

### Non-Functional Requirements

- [ ] No performance regression for basic queries
- [ ] Error messages remain consistent and user-friendly
- [ ] UI shows database type correctly in connection list

### Quality Gates

- [ ] `cargo build` passes with no warnings
- [ ] `npm run check` passes with no errors
- [ ] Manual testing with PostgreSQL (JSONB, arrays)
- [ ] Manual testing with MySQL/MariaDB
- [ ] Manual testing with SQLite

## Success Metrics

1. **Zero JSONB errors**: No `PgTypeInfo(Jsonb)` decode errors
2. **Simpler queries**: Users can run `SELECT *` without type coercion knowledge
3. **Fallback works**: Unknown schemes don't block connections

## Dependencies & Risks

### Dependencies

- sqlx 0.8.6 already includes all required pool types (`postgres`, `mysql`, `sqlite`, `any` features)
- No new dependencies required

### Risks

| Risk | Mitigation |
|------|------------|
| Code duplication between database handlers | Extract common patterns to helper functions |
| Different error formats per database | Wrap in CoreError with consistent formatting |
| AnyPool fallback limitations | Document limitations, show warning in UI |

## Implementation Outline

### Phase 1: Core Infrastructure

1. Create `DatabasePool` enum in `core/src/db.rs`
2. Create `detect_database_type()` function
3. Create database-specific modules: `core/src/db/postgres.rs`, `mysql.rs`, `sqlite.rs`, `any.rs`
4. Refactor `DbManager::connect()` to use typed pools
5. Refactor `DbManager::execute_query()` to dispatch by variant

### Phase 2: Type Handling

1. Implement `pg_row_to_json()` with native JSONB/array support
2. Implement `mysql_row_to_json()` with native JSON support
3. Move existing `row_value_to_json()` to `any.rs` module
4. Add connection timeout to `connect()`

### Phase 3: Integration

1. Update `commands.rs` to use core's `detect_database_type()`
2. Remove duplicate URL parsing from Tauri commands
3. Test with PostgreSQL, MySQL, SQLite
4. Test AnyPool fallback with unknown scheme

## Sources & References

### Internal References

- Current DbManager: `core/src/db.rs:10-237`
- URL scheme detection: `src-tauri/src/commands.rs:22-29`
- Type coercion: `core/src/db.rs:203-237`
- Models: `core/src/models.rs:3-14`

### External References

- sqlx Pool documentation: https://docs.rs/sqlx/0.8.6/sqlx/
- sqlx PostgreSQL types: https://docs.rs/sqlx/0.8.6/sqlx/postgres/types/index.html
- sqlx MySQL types: https://docs.rs/sqlx/0.8.6/sqlx/mysql/types/index.html
- sqlx JSON type: https://docs.rs/sqlx/0.8.6/sqlx/types/struct.Json.html

### Key Learnings Applied

- **DashMap pattern** (from planning docs): Pool is already Send+Sync, use DashMap for concurrent access
- **install_default_drivers()** (from planning docs): Still required for AnyPool fallback
- **Enum pattern** (from research): Recommended approach for heterogeneous pool storage
