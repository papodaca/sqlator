---
date: 2026-04-08
topic: oracle-driver-fix
---

# Oracle Database Driver — Fix and Enable

## Problem Frame

The Oracle driver (`core/src/db/oracle.rs`) is fully scaffolded but intentionally disabled via a hard-coded error in `create_pool`. When the guard was removed and the driver was tested against Oracle Free (FREEPDB1), two problems surfaced: the schema browser returned an error and `SELECT` queries returned zero rows. The goal is to diagnose and fix these bugs so Oracle becomes a first-class supported database in SQLator, on par with PostgreSQL, MySQL, and MSSQL.

**Crates in use:** `oracle-rs` (pure-Rust TNS implementation) + `deadpool-oracle` (connection pool).  
**Reference material:** Node.js `oracledb` package docs for Oracle protocol behavior, type handling, and query patterns.  
**Test target:** `gvenzl/oracle-free:latest-faststart` — Oracle 23c Free Edition via docker-compose on port 1522, service `FREEPDB1`, password `Sqlator123!`.

## Requirements

- R1. `create_pool` must establish a real connection to Oracle using `oracle-rs`/`deadpool-oracle` rather than returning a hard-coded error.
- R2. The schema browser (`get_schemas`) must return the list of user-owned schemas without error. Connected as `system`, at minimum the current user's schema and any non-system schemas must be visible.
- R3. `get_tables` must return tables and views for a given schema. The `UNION ALL` query using positional bind parameters (`:1`) passed twice must be validated against oracle-rs's actual parameter binding API.
- R4. `execute_select` must return rows for a user-created table. The row iteration pattern over `result.rows` must match oracle-rs's actual result set API — streaming vs. batch collection must be verified.
- R5. `execute_statement` (INSERT, UPDATE, DELETE, DDL) must report rows affected without error.
- R6. `get_columns` must return column metadata (name, type, nullability, PK/FK) for a given table.
- R7. Oracle must be enabled in the UI — the table view must work and the connection must not be greyed out or disabled.

## Out of Scope

- `query_table` filtered/sorted pagination for Oracle (the structured table view command) — wire this up only if straightforward; otherwise defer.
- LOB streaming beyond what oracle-rs already handles inline.
- Oracle versions older than 12c (the `oracle_maintained` column filter requires 12.1+; Oracle Free is 23c).
- Writing a from-scratch TNS protocol implementation — the goal is to make oracle-rs work, not replace it.

## Success Criteria

- Connecting to `oracle://system:Sqlator123!@localhost:1522/FREEPDB1` succeeds.
- Schema browser shows at least the `SYSTEM` schema (or the connected user's schema).
- A user-created table (e.g. `SYSTEM.PERSONS`) appears in the table list and returns its rows when queried.
- A `SELECT 1 FROM DUAL` returns one row.
- An INSERT followed by a SELECT on the same connection reflects the inserted row.

## Key Decisions

- **Use oracle-rs + deadpool-oracle as the foundation:** Avoids depending on Oracle's OCI C libraries, keeping the build fully pure-Rust and cross-platform.
- **Use oracledb (Node.js) as protocol/behavior reference only:** Not integrated into the binary — consulted for documentation on how Oracle query results, bind params, and types are expected to behave.
- **Target Oracle Free (23c) via docker-compose:** The existing `gvenzl/oracle-free` container is the test environment; no Oracle XE or full EE setup needed.

## Outstanding Questions

### Resolve Before Planning

_(none — scope is clear enough to plan)_

### Deferred to Planning

- [Affects R3, R4][Needs research] Does oracle-rs support repeated positional parameters (`:1` used twice in a single query)? What is the correct API for binding `Value::String` parameters?
- [Affects R4][Needs research] Does `conn.query()` in oracle-rs return a fully buffered `QueryResult` with a `.rows` vec, or does it return a stream/cursor? The current code iterates `result.rows` — verify this matches the actual oracle-rs 0.1.x API.
- [Affects R2][Needs research] Does `oracle_maintained = 'N'` filter work correctly on Oracle Free 23c when connected as `system`? It may exclude `SYSTEM` itself since it is a maintained user.
- [Affects R7][Technical] The UI currently disables Oracle in the table view — confirm which frontend component gate needs to be removed or updated.

## Next Steps

→ `/ce:plan` for structured implementation planning
