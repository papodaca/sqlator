---
title: "feat: Unified Grid with Infinite Scroll and CTE-based Pagination"
type: feat
status: active
date: 2026-04-13
origin: docs/brainstorms/2026-04-13-unified-grid-infinite-scroll-requirements.md
---

# feat: Unified Grid with Infinite Scroll and CTE-based Pagination

## Overview

Two separate grid components (`EnhancedGrid.svelte` for table browse, `ResultGrid.svelte` for query results) are merged into one. Ad-hoc query results gain transparent CTE-based pagination, infinite scroll, and interactive filter/sort — all without the user touching their SQL. The hard row cap rises from 1,000 to 50,000. Table browse gains infinite scroll in place of the "Load more" button.

(see origin: docs/brainstorms/2026-04-13-unified-grid-infinite-scroll-requirements.md)

## Problem Statement

- `ResultGrid` has a virtualizer but no pagination, no filter/sort, and a 1,000-row hard cap.
- `EnhancedGrid` has filter/sort and load-more pagination but no virtualizer (all rows live in the DOM simultaneously) and the same 1,000-row cap.
- Users hitting 1,000 rows in the query result pane can only work around it by rewriting their SQL.
- The two codebases diverge over time and must be maintained separately.

## Proposed Solution

### Architecture

```
┌─────────────────────────────────────────────────────┐
│             UnifiedGrid.svelte (new)                │
│  • @tanstack/svelte-virtual                         │
│  • Sort headers + filter row (from EnhancedGrid)    │
│  • Cell editing (from ResultGrid, disabled via prop)│
│  • Infinite scroll sentinel row                     │
│  Props: rows, columns, editMode, hasMore,           │
│         isFetchingMore, filters, sort, onLoadMore   │
└─────────────────────────────────────────────────────┘
         ↑                          ↑
  ResultPane.svelte           TableBrowser context
  (query results,             (table browse,
   editMode=false)             editMode=true)
```

**Backend new command:**

```
execute_query_paged(connectionId, sql, limit, offset, filters, sort) → Stream<QueryEvent>
```

The backend wraps `sql` in `WITH __sqlator_q AS (...sql...) SELECT * FROM __sqlator_q [WHERE ...] [ORDER BY ...] LIMIT N OFFSET M` using AST-safe query classification. Non-wrappable queries (DML, DDL, multi-statement) fall back to the existing `execute_query` path.

---

## Technical Approach

### Phase 1 — Core: Query Classification and CTE Wrapping

**New file: `core/src/db/sql_classify.rs`**

Add the `sqlparser` crate (`sqlparser = "0.55"` in `core/Cargo.toml`) for AST-based classification.

```rust
// core/src/db/sql_classify.rs

use sqlparser::{ast::Statement, dialect::*, parser::Parser};

pub enum QueryClass {
    /// Safe to wrap in CTE for pagination
    WrappableSelect { already_has_limit: bool },
    /// DML — never wrap, emit rows_affected
    Dml,
    /// DDL — never wrap
    Ddl,
    /// EXPLAIN / SHOW / DESCRIBE — stream as-is
    MetaCommand,
    /// Multiple semicolon-separated statements
    MultiStatement,
    /// Parse failed — fall back to first-word heuristic
    Unknown,
}

pub fn classify_sql(sql: &str, db_type: DatabaseType) -> QueryClass { ... }
```

**CTE wrapping — `core/src/db/sql_classify.rs`:**

```rust
pub fn wrap_for_pagination(
    user_sql: &str,
    limit: i64,
    offset: i64,
    db_type: DatabaseType,
    filters: &[FilterSpec],  // from the UI's filter row
    sort: &[SortSpec],       // from the UI's sort headers
    valid_columns: &[&str],  // column names from the first-page result, for injection safety
) -> String {
    let clean = user_sql.trim().trim_end_matches(';').trim();

    // Dialect-specific outer pagination clause
    let pagination = match db_type {
        DatabaseType::Mssql => format!(
            "ORDER BY (SELECT NULL) OFFSET {} ROWS FETCH NEXT {} ROWS ONLY", offset, limit
        ),
        DatabaseType::Oracle => format!(
            "OFFSET {} ROWS FETCH NEXT {} ROWS ONLY", offset, limit
        ),
        _ => format!("LIMIT {} OFFSET {}", limit, offset),
    };

    // Filter + sort on outer SELECT (reuses existing build_where_clause / build_order_by logic)
    let where_clause = build_outer_where(filters, db_type, valid_columns);
    let order_clause = build_outer_order(sort, db_type, valid_columns);

    format!(
        "WITH __sqlator_q AS (\n{}\n) SELECT * FROM __sqlator_q{}{}\n{}",
        clean, where_clause, order_clause, pagination
    )
}
```

Key wrapping safety rules (see origin: Requirements §R6):
- `MultiStatement` → fall back (do not wrap), UI shows "Multi-statement queries are not paginated"
- `Dml` / `Ddl` → fall back to `execute_query`
- `MetaCommand` → fall back to `execute_query`
- `Unknown` → fall back to `execute_query` (parse failure is non-fatal)
- `WrappableSelect { already_has_limit: true }` → still wrap; outer LIMIT takes precedence; document this behavior

### Phase 2 — Backend: New Paginated Command and Raised Row Cap

**New Tauri command: `execute_query_paged`** in `src-tauri/src/commands.rs`:

```rust
#[tauri::command]
pub async fn execute_query_paged(
    state: State<'_, AppState>,
    connection_id: String,
    sql: String,          // always the original user SQL, never the wrapped version
    limit: i64,
    offset: i64,
    filters: Vec<FilterSpec>,
    sort: Vec<SortSpec>,
    on_event: Channel<QueryEvent>,
) -> CmdResult<()>
```

Logic:
1. Classify `sql` via `classify_sql(&sql, db_type)`
2. If `WrappableSelect`: call `wrap_for_pagination(...)` → execute the wrapped SQL via the existing streaming path
3. Otherwise (DML/DDL/meta/multi/unknown): delegate to `execute_query` unchanged
4. Stream `QueryEvent` back via `on_event`

Column injection safety for outer filter/sort: On offset=0 (first page), capture the returned column names from the `Columns` event and store them in a per-request variable. Pass them as `valid_columns` to `wrap_for_pagination`. For offset>0, the frontend passes known column names from the result state (already validated on page 0).

**Raise `max_rows` from 1,000 to 50,000 — 7 files:**

| File | Location | Change |
|---|---|---|
| `core/src/db/postgres.rs` | line 18 | `let max_rows: usize = 50_000` |
| `core/src/db/mysql.rs` | line 18 | `let max_rows: usize = 50_000` |
| `core/src/db/sqlite.rs` | line 18 | `let max_rows: usize = 50_000` |
| `core/src/db/any.rs` | line 18 | `let max_rows: usize = 50_000` |
| `core/src/db/mssql.rs` | line 82 | `let max_rows: usize = 50_000` |
| `core/src/db/oracle.rs` | line 73 | `let max_rows: usize = 50_000` |
| `core/src/db/clickhouse.rs` | line 161 | `if i >= 50_000 { break }` |

**Raise `query_table` cap from 1,000 in `core/src/db/mod.rs` and driver files:**

| File | Pattern | Change |
|---|---|---|
| `core/src/db/mod.rs` | `params.limit.min(1000)` × 3 (lines 969, 1033, 1087) | `.min(50_000)` |
| `core/src/db/mssql.rs` | same pattern | `.min(50_000)` |
| `core/src/db/oracle.rs` | same pattern | `.min(50_000)` |
| `core/src/db/clickhouse.rs` | same pattern | `.min(50_000)` |

Register `execute_query_paged` in Tauri's `invoke_handler` in `src-tauri/src/main.rs` (alongside existing commands).

### Phase 3 — Frontend: Types and State

**`src/lib/types.ts` — extend `ResultPaneState.results`:**

```typescript
// Before (partial):
| { kind: "results"; columns: string[]; rows: ...; rowCount: number; durationMs: number }

// After:
| {
    kind: "results";
    columns: string[];
    columnTypes: string[];          // NEW — inferred from first page for filter UI
    rows: Record<string, unknown>[];
    rowCount: number;
    durationMs: number;
    // Pagination state:
    originalSql: string;            // NEW — the unwrapped user SQL, for re-fetching pages
    offset: number;                 // NEW — rows already loaded
    hasMore: boolean;               // NEW — server may have more rows
    isFetchingMore: boolean;        // NEW — in-flight page request
    // Interactive filter/sort applied on top of base query:
    activeFilters: FilterSpec[];    // NEW
    activeSort: SortSpec[];         // NEW
    isFiltered: boolean;            // NEW — true when any filter/sort is active
  }
```

**`src/lib/stores/tabs.svelte.ts` — new functions:**

- `executeQueryFirstPage(tabId, sql)`: calls `execute_query_paged` with `offset=0`, replaces `ResultPaneState`; captures column names for `columnTypes`
- `executeQueryNextPage(tabId)`: increments offset, calls `execute_query_paged` again, **appends** rows to existing state, sets `isFetchingMore`
- `applyResultFilter(tabId, filters, sort)`: resets offset to 0, sets `activeFilters`/`activeSort`, calls `executeQueryFirstPage` (first page with new filters)

Column type inference for the filter row (since we have no schema metadata for ad-hoc queries): infer from the first non-null value in each column of the first-page result. Helper `inferColumnType(values: unknown[]) → 'text' | 'number' | 'date' | 'boolean'` — same categories used by `getColTypeCategory` in `EnhancedGrid.svelte` (line 149).

### Phase 4 — Frontend: UnifiedGrid Component

**New file: `src/lib/components/UnifiedGrid.svelte`**

Merges capabilities of both existing grids.

**Props interface:**

```typescript
interface Props {
  columns: string[];
  columnTypes: string[];     // for filter operator selection
  rows: Record<string, unknown>[];
  editMode: boolean;         // enables cell editing and save/discard controls

  // Infinite scroll
  hasMore?: boolean;
  isFetchingMore?: boolean;
  onLoadMore?: () => void;

  // Filter/sort (emitted to parent, applied via CTE or query_table)
  activeFilters?: FilterSpec[];
  activeSort?: SortSpec[];
  onFiltersChange?: (filters: FilterSpec[], sort: SortSpec[]) => void;

  // Edit callbacks (only called when editMode=true)
  onSave?: () => void;
}
```

**Virtualizer setup** (extends `ResultGrid.svelte` pattern):

```typescript
// +1 when hasMore: the extra slot is the loading sentinel
let rowCount = $derived(hasMore ? displayRows.length + 1 : displayRows.length);

let virtualizer = $derived(
    scrollEl
        ? createVirtualizer({
            count: rowCount,
            getScrollElement: () => scrollEl!,
            estimateSize: () => 36,
            overscan: 10,
          })
        : null,
);
```

**Infinite scroll detection** (`$effect` in `UnifiedGrid.svelte`):

Three-layer guard prevents double-fetches (see origin: Requirements §R2):

```typescript
let fetchInFlight = $state(false);

$effect(() => {
    const items = virtualizer ? get(virtualizer).getVirtualItems() : [];
    const lastItem = items[items.length - 1];
    if (!lastItem || !onLoadMore) return;

    const nearEnd = lastItem.index >= displayRows.length - 1;
    if (nearEnd && hasMore && !isFetchingMore && !fetchInFlight) {
        fetchInFlight = true;
        onLoadMore();
    }
    if (!isFetchingMore) {
        fetchInFlight = false;
    }
});
```

**Loading sentinel row** (inside `{#each virtualItems}`):

```svelte
{#if item.index > displayRows.length - 1}
    <tr class="sentinel-row" style="height: {item.size}px">
        <td colspan={columns.length} class="sentinel-cell">
            {#if isFetchingMore}
                <span class="loading-more-text">Loading…</span>
            {/if}
        </td>
    </tr>
{:else}
    <!-- normal row -->
{/if}
```

**Sort headers** (from `EnhancedGrid.svelte` lines 81–93): click cycles `null → asc → desc → null`; calls `onFiltersChange(activeFilters, newSort)`.

**Filter row** (from `EnhancedGrid.svelte` lines 223–269): `<select>` for operator, `<input>` for value, debounced 300ms; calls `onFiltersChange(newFilters, activeSort)`.

**Cell editing** (from `ResultGrid.svelte` lines 260–300): only rendered when `editMode=true`.

**50k row status footer** (replaces "Showing first 1,000 of X rows" notice in `ResultPane.svelte`):

```svelte
{#if result.rowCount > result.rows.length}
    <div class="row-limit-notice">
        Loaded {result.rows.length.toLocaleString()} of {result.rowCount.toLocaleString()} rows
        {#if result.rows.length >= 50_000}— 50,000 row limit reached{/if}
    </div>
{/if}
```

### Phase 5 — Migration: Replace Old Components

**`src/lib/components/ResultPane.svelte`:**
- Replace `<ResultGrid ...>` with `<UnifiedGrid editMode={false} onLoadMore={...} hasMore={result.hasMore} isFetchingMore={result.isFetchingMore} onFiltersChange={...} ...>`
- Pass `executeQueryNextPage(tabId)` as `onLoadMore`
- Pass `applyResultFilter(tabId, filters, sort)` as `onFiltersChange`
- Remove the "Showing first 1,000 of N rows" notice; add the new status footer above

**Table browse context** (wherever `EnhancedGrid` is used — search for `EnhancedGrid` import sites):
- Replace `<EnhancedGrid ...>` with `<UnifiedGrid editMode={true} onLoadMore={handleTableLoadMore} ...>`
- `handleTableLoadMore` calls `fetchData(currentOffset)` (existing table browse fetch logic, unchanged)
- Remove `<LoadMoreButton>` usage (it becomes redundant)

**Delete when migration is complete:**
- `src/lib/components/EnhancedGrid.svelte`
- `src/lib/components/ResultGrid.svelte`
- `src/lib/components/LoadMoreButton.svelte`

---

## Implementation Phases Summary

### Phase 1 — Core classification + wrapping (Rust)
- `core/Cargo.toml`: add `sqlparser = "0.55"`
- Create `core/src/db/sql_classify.rs`: `classify_sql`, `wrap_for_pagination`, `build_outer_where`, `build_outer_order`
- Raise `max_rows` to 50,000 (7 driver files + `mod.rs`)

### Phase 2 — New backend command (Rust + Tauri)
- Add `execute_query_paged` to `src-tauri/src/commands.rs`
- Register in `src-tauri/src/main.rs` invoke handler
- Update `query_table` cap to 50,000 in all locations

### Phase 3 — Types and state (TypeScript)
- Extend `ResultPaneState` in `src/lib/types.ts`
- Add `executeQueryFirstPage`, `executeQueryNextPage`, `applyResultFilter` to `src/lib/stores/tabs.svelte.ts`
- Add `inferColumnType` helper

### Phase 4 — UnifiedGrid component (Svelte)
- Create `src/lib/components/UnifiedGrid.svelte`
- Carries virtualizer, sort headers, filter row, cell editing, infinite scroll sentinel, 50k status footer

### Phase 5 — Migration
- Update `ResultPane.svelte` to use `UnifiedGrid`
- Update table browse context to use `UnifiedGrid`
- Delete `EnhancedGrid.svelte`, `ResultGrid.svelte`, `LoadMoreButton.svelte`

---

## System-Wide Impact

### Interaction Graph

1. User scrolls near the bottom of `UnifiedGrid` → `$effect` fires `onLoadMore()`
2. `onLoadMore` calls `executeQueryNextPage(tabId)` in `tabs.svelte.ts`
3. `executeQueryNextPage` invokes `execute_query_paged` via Tauri IPC with `offset += pageSize`
4. Rust: `classify_sql` → `wrap_for_pagination` → streaming execute → `QueryEvent` channel
5. Frontend: accumulates events, appends rows to `ResultPaneState.rows`, sets `isFetchingMore=false`
6. Svelte reactivity propagates new `rows` and `isFetchingMore=false` to `UnifiedGrid`
7. `$effect` fires again; `fetchInFlight` resets; if user is still near bottom, fires again

### Error & Failure Propagation

- `execute_query_paged` emits `QueryEvent::Error { message }` on failure (same as `execute_query`)
- The streaming path in `tabs.svelte.ts` sets `ResultPaneState` to `{ kind: "error" }` on error event
- A mid-scroll error (e.g. connection lost on page 3) leaves rows 1-2 intact; the error replaces the loading state in `ResultPane.svelte`
- Wrapping failures (sqlparser errors) fall back silently — the original SQL is executed unchanged; the user sees results without pagination/filter/sort, not an error

### State Lifecycle Risks

- **Stale `originalSql`**: `ResultPaneState.originalSql` must be set exactly once per top-level execution. If the user re-runs the query tab while a page fetch is in flight, `isFetchingMore=true` must be cancelled. Add a tab-level `abortController` pattern or sequence token to discard stale responses (same approach as the existing 50ms flush interval in `tabs.svelte.ts`).
- **Filter reset on re-execute**: When the user presses "Run" again, `activeFilters` and `activeSort` must be cleared; the new execution starts fresh.
- **`MAX_ROWS` in `EnhancedGrid` and `LoadMoreButton` constants**: Remove `MAX_ROWS = 1000` from the deleted files. The 50,000 cap is now enforced entirely in the Rust backend; no frontend constant is needed.

### API Surface Parity

| Context | Old command | New command |
|---|---|---|
| Query result, first page | `execute_query` | `execute_query_paged(offset=0)` |
| Query result, next pages | N/A | `execute_query_paged(offset=N)` |
| Query result with filter/sort | N/A | `execute_query_paged(offset=0, filters, sort)` |
| Table browse | `query_table` | `query_table` (unchanged) |
| TUI | `execute_query` (streaming) | unchanged — TUI uses its own path |

### Integration Test Scenarios

1. **Large result set**: Run `SELECT * FROM large_table` (100k rows). Scroll to bottom; expect rows to keep loading until 50k cap. At 50k, expect "50,000 row limit reached" footer and no further fetches.
2. **Filter on result**: Run any SELECT; apply a text filter. Expect offset to reset to 0 and filtered rows to replace the unfiltered set. Scroll to bottom of filtered set; expect infinite scroll to continue with filter intact.
3. **Non-wrappable fallback**: Run `DELETE FROM table RETURNING *`. Expect the result pane to show rows-affected, not a paginated grid. No CTE wrapping applied.
4. **Re-execute mid-scroll**: While page 3 of a large result is loading, press "Run" again. Expect the in-progress fetch to be abandoned and the result pane to start fresh.
5. **Table browse auto-scroll**: Open table browse for a 200-row table. Scroll to bottom. Expect rows 51–100 to load automatically without clicking "Load more".

---

## Acceptance Criteria

### Functional
- [ ] `UnifiedGrid.svelte` exists; `EnhancedGrid.svelte`, `ResultGrid.svelte`, `LoadMoreButton.svelte` are deleted
- [ ] Query result pane uses `UnifiedGrid` with `editMode=false`
- [ ] Table browse uses `UnifiedGrid` with `editMode=true`
- [ ] Scrolling to the bottom of a result set automatically loads the next page
- [ ] The "Load more" button no longer exists in any context
- [ ] Interactive filter and sort are available in the query result pane (matching EnhancedGrid's current style)
- [ ] Applying a filter or sort resets to page 0 and fetches from the beginning
- [ ] DML queries (INSERT/UPDATE/DELETE) continue to show rows-affected, no CTE wrapping applied
- [ ] Multi-statement queries execute without wrapping; UI indicates pagination is unavailable
- [ ] 50,000 row cap is enforced; a "50,000 row limit reached" message appears when hit
- [ ] `execute_query_paged` Tauri command is registered and works for all 6 DB drivers
- [ ] `sqlparser` classifies all wrappable/non-wrappable queries correctly for the Postgres dialect (unit tested)

### Non-Functional
- [ ] No visible scroll jank when loading next page — sentinel row appears smoothly
- [ ] Double-fetch prevention works: scrolling fast does not produce duplicate page requests
- [ ] `max_rows` = 50,000 across all 6 driver files and `query_table` cap locations

---

## Dependencies & Risks

- **`sqlparser` crate**: Adds a compile-time dependency. Version 0.55 must support all 6 dialects. Oracle dialect support in sqlparser is partial; the `GenericDialect` fallback covers it.
- **MSSQL pagination syntax**: `ORDER BY ... OFFSET ... FETCH NEXT` is required. If the user's inner query already has `ORDER BY`, the outer wrapper may produce a duplicate `ORDER BY` — test this case; may need to suppress the `ORDER BY (SELECT NULL)` fallback when sort specs are present.
- **Svelte 5 + TanStack Virtual compatibility**: The `$derived`-gated virtualizer pattern in `ResultGrid.svelte` (which `UnifiedGrid` inherits) is the known workaround for TanStack/virtual issue #866. Do not change this pattern.
- **`EnhancedGrid` has no virtualizer today** — table browse with 1,000 accumulated rows works fine with plain DOM rendering, but at 50,000 rows it will be slow without a virtualizer. `UnifiedGrid` uses the virtualizer for all contexts, which fixes this.
- **Column type inference for filter row**: Inferred types may be wrong for mixed-type columns or all-null columns. Use `'text'` as the fallback to avoid breaking the filter row.

## Sources & References

### Origin
- **Origin document:** [docs/brainstorms/2026-04-13-unified-grid-infinite-scroll-requirements.md](docs/brainstorms/2026-04-13-unified-grid-infinite-scroll-requirements.md) — key decisions carried forward: (1) always-automatic CTE wrapping, (2) 50k row cap, (3) unified grid with edit mode prop

### Internal References
- `src/lib/components/EnhancedGrid.svelte` — filter row UI (lines 223–269), sort logic (lines 81–93), `fetchData` (lines 36–74)
- `src/lib/components/ResultGrid.svelte` — virtualizer setup (lines 68–79), cell editing (lines 260–300)
- `src/lib/components/ResultPane.svelte` — 1,000-row notice (lines 74–77), ResultGrid usage (lines 85–89)
- `src/lib/components/LoadMoreButton.svelte` — props and render logic (lines 2–38)
- `src/lib/stores/tabs.svelte.ts` — `executeQuery` streaming accumulation (lines 325–416)
- `src/lib/types.ts` — `FilterSpec`, `TableQueryParams`, `ResultPaneState`, `TableBrowseState`
- `core/src/db/mod.rs` — `execute_query` is_select detection (lines 112–123), `query_table` SQL generation (lines 955–1087), `build_where_clause` (line 826), `build_order_by_pg` (line 804)
- `core/src/db/postgres.rs` — `max_rows` enforcement pattern (line 18)
- `src-tauri/src/commands.rs` — `execute_query` command (lines 333–362), `query_table` command (lines 1699–1706)

### External References
- [TanStack Virtual Svelte Infinite Scroll Example](https://tanstack.com/virtual/v3/docs/framework/svelte/examples/infinite-scroll)
- [sqlparser-rs Statement enum](https://docs.rs/sqlparser/latest/sqlparser/ast/enum.Statement.html)
- Svelte 5 + TanStack Virtual compatibility: [TanStack/virtual issue #866](https://github.com/TanStack/virtual/issues/866)
