---
date: 2026-04-13
topic: unified-grid-infinite-scroll
---

# Unified Grid with Infinite Scroll, CTE Pagination, and Post-Query Filter/Sort

## Problem Frame

The app currently has two separate grid components with diverging capabilities:

- **EnhancedGrid** (table browse): LIMIT/OFFSET pagination via a "Load more" button, server-side filter/sort, editable cells.
- **ResultGrid** (ad-hoc query results): Single fetch hard-capped at 1,000 rows, no pagination, no post-query filter/sort, virtualised rendering.

This split creates inconsistent UX and duplicated code. Users hitting the 1,000-row limit in the query result pane have no recourse. And applying a filter or sort to query results requires rewriting and re-running the original SQL.

## Requirements

- **R1. Unified grid component** — Merge `EnhancedGrid` and `ResultGrid` into a single grid component used in both table browse and query result contexts. Editing features (cell editing, save/discard controls) are enabled only in table browse mode; disabled for ad-hoc query results.

- **R2. Infinite scroll replaces "Load more" button** — Both contexts auto-fetch the next page of rows when the user scrolls near the bottom of the loaded data. The "Load more" button is removed. A subtle loading indicator appears at the bottom while fetching.

- **R3. CTE-based transparent pagination for query results** — When the user runs a SELECT-type query, the backend wraps it in a CTE and applies `LIMIT/OFFSET` pagination on top. The user's original SQL is unchanged and unaware of the wrapping. Each scroll-triggered fetch increments the offset. The base query executes once; subsequent pages re-execute the CTE wrapper with a new offset.

  Example transformation:
  ```sql
  -- User writes:
  SELECT * FROM orders WHERE status = 'pending' ORDER BY created_at DESC

  -- Backend wraps as (page 2, page size 200):
  WITH __sqlator_q AS (
    SELECT * FROM orders WHERE status = 'pending' ORDER BY created_at DESC
  )
  SELECT * FROM __sqlator_q LIMIT 200 OFFSET 200
  ```

- **R4. Post-query filter and sort via CTE** — Filters and sort applied interactively by the user in the query result grid are pushed into the CTE wrapper as additional `WHERE` / `ORDER BY` clauses appended outside the base CTE. This resets the offset to 0 and re-fetches. The UI matches the current EnhancedGrid style: column header click to sort, filter input row beneath the headers.

  Example with user-applied filter and sort:
  ```sql
  WITH __sqlator_q AS (
    SELECT * FROM orders WHERE status = 'pending' ORDER BY created_at DESC
  )
  SELECT * FROM __sqlator_q
  WHERE customer_name ILIKE '%acme%'
  ORDER BY total_amount DESC
  LIMIT 200 OFFSET 0
  ```

- **R5. 50,000-row cap** — The hard row limit is raised from 1,000 to 50,000 across all contexts. When the cap is reached, display a clear "50,000 row limit reached" message and stop fetching. The cap is enforced in the backend.

- **R6. Non-wrappable queries fall back gracefully** — Queries that cannot be safely CTE-wrapped (DML, DDL, `EXPLAIN`, `SHOW`, `DESCRIBE`, multi-statement batches, or queries where wrapping is detected as unsafe) are executed as today: single-fetch streaming, no CTE wrapping, no post-query filter/sort. The result pane behaves as the current `ResultPane` for these cases.

- **R7. Table browse is unaffected by CTE wrapping** — Table browse already has server-side filter/sort via the `query_table` backend command. It does not use CTE wrapping; pagination, filter, and sort continue to go through `query_table` with its existing `LIMIT/OFFSET/filters/sort` parameters. Only the "Load more" button is replaced with auto-scroll (R2).

## Success Criteria

- A user running a query that returns 10,000 rows can scroll through all of them without clicking anything or rewriting SQL.
- A user can sort or filter the result of any SELECT query interactively without re-running it.
- The codebase has one grid component, not two.
- The "Showing first 1,000 of X rows" notice is gone; replaced with live "Loaded N of X rows" or similar.

## Scope Boundaries

- **Out of scope:** Editing cells in the ad-hoc query result pane (disabled per R1).
- **Out of scope:** Exporting the full result set beyond 50k rows.
- **Out of scope:** TUI grid — different rendering system entirely.
- **Out of scope:** SSH / Docker connection handling — no change.
- **Out of scope:** The `query_table` backend command and its server-side filter/sort logic — table browse server-side behavior is unchanged.

## Key Decisions

- **Always automatic CTE wrapping:** No opt-in toggle. Every SELECT result silently gets CTE-based pagination and filter/sort capability. Users never see the wrapper SQL.
- **50k row cap:** Safety ceiling retained but raised from 1k to 50k. Uncapped scrolling was rejected to avoid accidental full-table loads on large datasets.
- **Filter/sort UI:** Column header sort + filter row, matching the current EnhancedGrid style. No new UI paradigm introduced.
- **Unified component:** One grid replaces two. Edit mode is a prop, not a separate component.
- **CTE name:** `__sqlator_q` (double-underscore prefix reduces collision risk with user-defined CTE names).

## Dependencies / Assumptions

- All supported databases (PostgreSQL, MySQL 8+, SQLite, MSSQL, Oracle, ClickHouse) support CTEs. Older MySQL versions may not — this needs verification during planning.
- `fetch_schema_metadata` currently marks `WITH`-prefixed queries as non-editable. CTE-wrapped table browse queries must not be routed through this path (R7 ensures they aren't).
- The `@tanstack/svelte-virtual` virtualizer already in `ResultGrid` is the right scroll primitive for infinite scroll detection.

## Outstanding Questions

### Resolve Before Planning

_(none — all product decisions resolved)_

### Deferred to Planning

- **[Affects R2][Technical]** What scroll position threshold triggers the next page fetch — e.g. within 5 rows of the bottom of loaded data?
- **[Affects R2, R3][Technical]** What is the right page size per fetch (100? 200? 500?) for the query result pane?
- **[Affects R3][Needs research]** Does CTE wrapping add measurable query planning overhead for simple queries on the supported databases?
- **[Affects R3][Needs research]** How to detect that a user's query already contains a top-level `LIMIT` or `OFFSET` — should wrapping be skipped or allowed to override?
- **[Affects R3][Needs research]** MySQL version compatibility for CTEs — minimum supported version.
- **[Affects R4][Technical]** How are column types inferred from the CTE result for filter input rendering (e.g. date picker vs text box)?
- **[Affects R1][Technical]** Exact props interface for the unified grid component (edit mode flag, connection id, etc.).

## Next Steps

→ `/ce:plan` for structured implementation planning
