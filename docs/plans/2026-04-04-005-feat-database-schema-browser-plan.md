---
title: "feat: Database Schema Browser with Enhanced Data Grid"
type: feat
status: completed
date: 2026-04-04
origin: docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md
---

# ✨ Database Schema Browser with Enhanced Data Grid

A left sidebar schema browser with expandable table/column tree, database/schema selector for Postgres, and an enhanced data grid with server-side sorting and filtering.

---

## Overview

Build a schema browser feature that allows users to visually explore their database structure and browse table data with Excel-like sorting and filtering capabilities. This feature extends the MVP's connection manager and query execution foundation.

**Core Capabilities:**
1. **Schema Tree** — Left sidebar showing databases/schemas (Postgres), tables, and columns
2. **Tree Navigation** — Expandable nodes showing column names and types
3. **Data Grid Launch** — Double-click table to open `SELECT *` results
4. **Enhanced Grid** — Server-side sorting and filtering per column

---

## Problem Statement

The MVP provides a SQL editor and basic result grid, but users must know table names and column structures to write queries. Real-world database exploration requires:
- Visual discovery of available tables and schemas
- Quick inspection of table structure (columns, types, constraints)
- Easy data browsing without writing `SELECT *` queries
- Excel-like interaction for sorting and filtering results

Existing SQL clients (DataGrip, Beekeeper Studio) solve this, but SQLator needs its own implementation matching the Tauri + Svelte 5 architecture.

---

## Proposed Solution

A three-panel layout with schema tree in the left sidebar:

```
┌─────────────┬─────────────────────────────────────────────────┐
│  Sidebar    │              Main Content Area                  │
│ ┌─────────┐ │ ┌─────────────────────────────────────────────┐ │
│ │Schema   │ │ │  Editor Toolbar                             │ │
│ │Dropdown │ │ │  [Run] [Cancel]  Status: connected  42ms    │ │
│ └─────────┘ │ └─────────────────────────────────────────────┘ │
│ ┌─────────┐ │ ┌─────────────────────────────────────────────┐ │
│ │[Refresh]│ │ │                                             │ │
│ └─────────┘ │ │  SQL Editor (CodeMirror 6)                  │ │
│ ┌─────────┐ │ │                                             │ │
│ │ Tables  │ │ │                                             │ │
│ │ ├ users │ │ └─────────────────────────────────────────────┘ │
│ │ │├ id    │ │ ┌─────────────────────────────────────────────┐ │
│ │ │├ name  │ │ │  Data Grid (TanStack Table)                │ │
│ │ │└ email │ │ │  ┌────────┬────────┬────────┬────────┐     │ │
│ │ ├ orders│ │ │  │ id ▼   │ name   │ email  │ status │     │ │
│ │ │├ id    │ │ │  ├────────┼────────┼────────┼────────┤     │ │
│ │ │├ total │ │ │  │ 1      │ Alice  │ a@b.c  │ active │     │ │
│ │ │└ date  │ │ │  │ 2      │ Bob    │ d@e.f  │ active │     │ │
│ │ └ products│ │  └────────┴────────┴────────┴────────┘     │ │
│ └─────────┘ │ │  [Load More]  Showing 1-50 of 1,247        │ │
│             │ └─────────────────────────────────────────────┘ │
└─────────────┴─────────────────────────────────────────────────┘
```

---

## Technical Approach

### Architecture

The schema browser extends the MVP architecture with new Rust commands and Svelte components:

```
┌──────────────────────────────────────────────────────────────┐
│                    Svelte 5 Frontend                          │
│  ┌──────────────┐  ┌───────────────┐  ┌──────────────────┐  │
│  │ SchemaTree   │  │ SqlEditor     │  │ EnhancedGrid     │  │
│  │ (Lazy Load)  │  │ (CodeMirror)  │  │ (TanStack Table) │  │
│  └──────┬───────┘  └───────┬───────┘  └────────┬─────────┘  │
│         │                  │                   │             │
│         └──────────── invoke() (IPC) ──────────┘             │
└──────────────────────────────────────────────────────────────┘
                              │
┌─────────────────────────────┼────────────────────────────────┐
│            Tauri 2 Rust Backend                               │
│                              │                                │
│  ┌───────────────────────────▼────────────────────────────┐  │
│  │ New Commands:                                          │  │
│  │  get_schemas, get_tables, get_columns,                 │  │
│  │  query_table (with sort/filter/pagination)             │  │
│  └──────────┬─────────────────────────────────────────────┘  │
│             │                                                 │
│  ┌──────────▼────────┐    ┌──────────────────────────────┐  │
│  │  sqlx AnyPool     │    │  Schema Cache (DashMap)      │  │
│  │  (information_    │    │  tables, columns, fks        │  │
│  │   schema queries) │    │  per connection_id           │  │
│  └───────────────────┘    └──────────────────────────────┘  │
└──────────────────────────────────────────────────────────────┘
```

### Schema Introspection Strategy

**PostgreSQL:**
- Use `information_schema` for portability, `pg_catalog` for performance-critical paths
- Query `pg_namespace` for schemas, filter by user permissions
- Support `current_database()` for database context

**MySQL:**
- Query `information_schema.tables` for tables, `information_schema.columns` for columns
- Use `table_schema` for database selection (MySQL has databases, not schemas)
- Foreign keys via `KEY_COLUMN_USAGE`

**SQLite:**
- Use `sqlite_master` for tables
- `PRAGMA table_info(table_name)` for columns
- `PRAGMA foreign_key_list(table_name)` for foreign keys

### Data Grid Server-Side Operations

**Sorting:**
```sql
SELECT * FROM users ORDER BY name ASC NULLS LAST LIMIT 50 OFFSET 0
```

**Filtering:**
```sql
SELECT * FROM users 
WHERE name ILIKE '%alice%' AND status = 'active'
ORDER BY id LIMIT 50 OFFSET 0
```

**Pagination:**
- Default page size: 50 rows
- Hard limit: 1000 rows per query
- "Load More" button for additional pages
- Use offset pagination for simplicity (cursor-based for future)

---

## Implementation Phases

### Phase 1: Schema Tree Foundation

**Goal:** Display database tables in a tree view with lazy-loaded columns.

**Rust commands:**
```rust
// src-tauri/src/commands/schema.rs

#[tauri::command]
pub async fn get_schemas(
    state: State<'_, AppState>,
    connection_id: String,
) -> Result<Vec<SchemaInfo>, CommandError>

#[tauri::command]
pub async fn get_tables(
    state: State<'_, AppState>,
    connection_id: String,
    schema: Option<String>,
) -> Result<Vec<TableInfo>, CommandError>

#[tauri::command]
pub async fn get_columns(
    state: State<'_, AppState>,
    connection_id: String,
    table: String,
    schema: Option<String>,
) -> Result<Vec<ColumnInfo>, CommandError>
```

**Frontend components:**
- `src/lib/components/SchemaBrowser.svelte` — Container with dropdown + tree
- `src/lib/components/SchemaTree.svelte` — Tree view with lazy-loaded nodes
- `src/lib/components/SchemaNode.svelte` — Individual tree node (table/column)
- `src/lib/components/SchemaDropdown.svelte` — Database/schema selector (Postgres)
- `src/lib/stores/schema.svelte.ts` — `$state` for schema tree

**Data types:**

```typescript
// src/lib/types.ts (additions)

export interface SchemaInfo {
  name: string;
  database?: string;  // For Postgres multi-database
  isDefault: boolean;
}

export interface TableInfo {
  name: string;
  schema?: string;
  type: 'table' | 'view';
  fullName: string;  // schema.table or just table
}

export interface ColumnInfo {
  name: string;
  type: string;
  nullable: boolean;
  defaultValue?: string;
  isPrimaryKey: boolean;
  isForeignKey: boolean;
  foreignTable?: string;
  foreignColumn?: string;
  ordinalPosition: number;
}
```

**Rust models:**

```rust
// src-tauri/src/models.rs (additions)

#[derive(Serialize, Clone)]
pub struct SchemaInfo {
    pub name: String,
    pub database: Option<String>,
    pub is_default: bool,
}

#[derive(Serialize, Clone)]
pub struct TableInfo {
    pub name: String,
    pub schema: Option<String>,
    #[serde(rename = "type")]
    pub table_type: String,
    pub full_name: String,
}

#[derive(Serialize, Clone)]
pub struct ColumnInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub data_type: String,
    pub nullable: bool,
    pub default_value: Option<String>,
    pub is_primary_key: bool,
    pub is_foreign_key: bool,
    pub foreign_table: Option<String>,
    pub foreign_column: Option<String>,
    pub ordinal_position: i32,
}
```

**Tasks:**
- [ ] Add `get_schemas` command in `src-tauri/src/commands/schema.rs`
- [ ] Add `get_tables` command with database-specific queries
- [ ] Add `get_columns` command with FK/PK detection
- [ ] Create `SchemaBrowser.svelte` component with layout
- [ ] Create `SchemaTree.svelte` with lazy-loaded expandable nodes
- [ ] Create `SchemaNode.svelte` with table/column rendering
- [ ] Create `SchemaDropdown.svelte` for Postgres schema selection
- [ ] Create `schema.svelte.ts` store with `$state` runes
- [ ] Register schema commands in `lib.rs` invoke_handler
- [ ] Add refresh button that clears cache and reloads schema
- [ ] Handle connection state: gray out tree when disconnected

**Success criteria:**
- [ ] On connection, schema tree loads automatically in sidebar
- [ ] Tables display with expand/collapse icons
- [ ] Clicking expand loads columns (lazy)
- [ ] Column nodes show name and type (e.g., `id: uuid`, `name: varchar(255)`)
- [ ] PK columns show key icon, FK columns show link icon
- [ ] Postgres connections show schema dropdown (public, user schemas)
- [ ] MySQL shows database dropdown
- [ ] SQLite shows tables directly (no dropdown)
- [ ] Refresh button reloads schema from database
- [ ] Tree grays out with "Reconnect to refresh" when connection lost

---

### Phase 2: Data Grid Launch

**Goal:** Double-click table to open `SELECT *` results in enhanced data grid.

**Rust command:**

```rust
// src-tauri/src/commands/queries.rs (addition)

#[derive(Deserialize)]
pub struct TableQueryParams {
    pub connection_id: String,
    pub table_name: String,
    pub schema: Option<String>,
    pub columns: Vec<String>,      // For SELECT clause
    pub sort: Vec<SortSpec>,       // ORDER BY
    pub filters: Vec<FilterSpec>,  // WHERE clause
    pub limit: i64,
    pub offset: i64,
}

#[derive(Serialize, Clone)]
pub struct TableQueryResult {
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub total_count: i64,
    pub has_more: bool,
}

#[tauri::command]
pub async fn query_table(
    state: State<'_, AppState>,
    params: TableQueryParams,
    on_event: Channel<QueryEvent>,
) -> Result<(), CommandError>
```

**Frontend components:**
- `src/lib/components/EnhancedGrid.svelte` — TanStack Table with sort/filter
- `src/lib/components/GridToolbar.svelte` — Column filter controls
- `src/lib/components/LoadMoreButton.svelte` — Pagination button
- `src/lib/stores/grid.svelte.ts` — `$state` for grid data and filters

**Event flow:**
1. User double-clicks table in `SchemaTree`
2. `SchemaNode.svelte` emits `table-selected` event with `TableInfo`
3. `App.svelte` or parent catches event, opens grid
4. `EnhancedGrid.svelte` calls `invoke('query_table', params)`
5. Results stream via `Channel<QueryEvent>`
6. Grid displays with sort/filter controls

**Tasks:**
- [ ] Add `query_table` command with sort/filter support
- [ ] Implement SQL builder with `QueryBuilder` for safe query construction
- [ ] Create `EnhancedGrid.svelte` wrapping TanStack Table
- [ ] Set up TanStack Table with `manualSorting`, `manualFiltering`, `manualPagination`
- [ ] Create `GridToolbar.svelte` with filter dropdowns per column
- [ ] Create `LoadMoreButton.svelte` for pagination
- [ ] Wire double-click event from `SchemaNode` to grid open
- [ ] Replace `ResultGrid.svelte` with `EnhancedGrid.svelte` for table queries
- [ ] Add "Open Table" context menu item as alternative to double-click

**Success criteria:**
- [ ] Double-clicking table opens data grid with `SELECT *` results
- [ ] Grid shows up to 50 rows initially
- [ ] "Load More" button fetches next 50 rows
- [ ] Hard limit of 1000 rows enforced with notice
- [ ] Column headers display with sort indicators
- [ ] Empty tables show "No data" message

---

### Phase 3: Server-Side Sorting

**Goal:** Sort grid columns with server-side `ORDER BY` queries.

**Sort specification:**

```typescript
// src/lib/types.ts (addition)

export interface SortSpec {
  id: string;        // Column name
  desc: boolean;     // Sort direction
  nullsFirst?: boolean;  // NULL position (Postgres-specific)
}

export type SortDirection = 'asc' | 'desc' | null;
```

**Rust sort handling:**

```rust
// In query_table implementation

fn apply_sorting(
    builder: &mut QueryBuilder<Postgres>,
    sort: &[SortSpec],
    valid_columns: &[&str],
) {
    if sort.is_empty() { return; }
    
    builder.push(" ORDER BY ");
    for (i, s) in sort.iter().enumerate() {
        if i > 0 { builder.push(", "); }
        
        // Validate column name against allowlist
        if valid_columns.contains(&s.id.as_str()) {
            builder.push(&s.id);
            builder.push(if s.desc { " DESC" } else { " ASC" });
            
            // PostgreSQL NULLS FIRST/LAST
            if let Some(nulls_first) = s.nulls_first {
                builder.push(if nulls_first { " NULLS FIRST" } else { " NULLS LAST" });
            }
        }
    }
}
```

**Frontend sort state:**

```svelte
<!-- EnhancedGrid.svelte -->
<script lang="ts">
  import { writable } from 'svelte/store';
  import { createSvelteTable, getCoreRowModel } from '@tanstack/svelte-table';
  import type { SortingState, Updater } from '@tanstack/table-core';

  let sorting = $state<SortingState>([]);
  let isLoading = $state(false);

  function setSorting(updater: Updater<SortingState>) {
    sorting = updater instanceof Function ? updater(sorting) : updater;
    fetchSortedData();
  }

  async function fetchSortedData() {
    isLoading = true;
    try {
      const result = await invoke('query_table', {
        params: {
          connection_id: connectionId,
          table_name: tableName,
          schema,
          sort: sorting,
          limit: 50,
          offset: 0,
        }
      });
      // Update grid data
    } finally {
      isLoading = false;
    }
  }

  const options = writable({
    columns: columnDefs,
    data: rows,
    state: { get sorting() { return sorting; } },
    onSortingChange: setSorting,
    manualSorting: true,
    getCoreRowModel: getCoreRowModel(),
  });

  const table = createSvelteTable(options);
</script>
```

**Tasks:**
- [ ] Add sort state to `EnhancedGrid.svelte` with TanStack Table controlled state
- [ ] Implement `setSorting` handler that calls `query_table` with sort params
- [ ] Add `apply_sorting` function in Rust for safe `ORDER BY` construction
- [ ] Validate sort column names against table schema (prevent SQL injection)
- [ ] Add sort direction indicators (arrows) to column headers
- [ ] Show "Sorting..." spinner while sort query runs
- [ ] Handle NULL sorting per database conventions
- [ ] Preserve sort state when "Load More" is clicked

**Success criteria:**
- [ ] Clicking column header sorts data ascending
- [ ] Second click sorts descending
- [ ] Third click clears sort
- [ ] Sort indicator (arrow) shows in column header
- [ ] NULL values sort consistently (follow database default)
- [ ] Sort persists when loading more rows
- [ ] Multi-column sort supported (Shift+click)

---

### Phase 4: Server-Side Filtering

**Goal:** Filter grid columns with server-side `WHERE` clauses.

**Filter specification:**

```typescript
// src/lib/types.ts (addition)

export type FilterOperator = 
  | 'contains'    // Text: ILIKE '%value%'
  | 'equals'      // Text: = 'value' / Number: = value
  | 'startsWith'  // Text: ILIKE 'value%'
  | 'endsWith'    // Text: ILIKE '%value'
  | 'gt'          // Number: > value
  | 'gte'         // Number: >= value
  | 'lt'          // Number: < value
  | 'lte'         // Number: <= value
  | 'between'     // Number: BETWEEN a AND b
  | 'isNull'      // IS NULL
  | 'isNotNull';  // IS NOT NULL

export interface FilterSpec {
  id: string;            // Column name
  operator: FilterOperator;
  value: string | number | [number, number];  // Single value or range
}

export interface ColumnFilterState {
  columnId: string;
  enabled: boolean;
  operator: FilterOperator;
  value: string | number;
}
```

**Filter UI per column type:**

| Column Type | Filter Operators | UI Component |
|-------------|-----------------|--------------|
| `varchar`, `text`, `char` | contains, equals, startsWith, endsWith | Text input + dropdown |
| `int`, `bigint`, `smallint`, `decimal` | equals, gt, gte, lt, lte, between | Number input(s) + dropdown |
| `timestamp`, `date`, `time` | equals, gt, lt, between | Date picker + dropdown |
| `boolean` | equals, isNull | Checkbox / dropdown |
| `uuid` | equals, contains | Text input |
| All types | isNull, isNotNull | Checkbox |

**Rust filter handling:**

```rust
// In query_table implementation

fn apply_filters(
    builder: &mut QueryBuilder<Postgres>,
    filters: &[FilterSpec],
    valid_columns: &[&str],
) {
    if filters.is_empty() { return; }
    
    builder.push(" WHERE ");
    for (i, f) in filters.iter().enumerate() {
        if i > 0 { builder.push(" AND "); }
        
        // Validate column name
        if !valid_columns.contains(&f.id.as_str()) { continue; }
        
        match f.operator.as_str() {
            "contains" => {
                builder.push(&f.id);
                builder.push(" ILIKE ");
                builder.push_bind(format!("%{}%", f.value));
            }
            "equals" => {
                builder.push(&f.id);
                builder.push(" = ");
                builder.push_bind(&f.value);
            }
            "gt" => {
                builder.push(&f.id);
                builder.push(" > ");
                builder.push_bind(&f.value);
            }
            "isNull" => {
                builder.push(&f.id);
                builder.push(" IS NULL");
            }
            "between" => {
                if let Value::Array(arr) = &f.value {
                    builder.push(&f.id);
                    builder.push(" BETWEEN ");
                    builder.push_bind(&arr[0]);
                    builder.push(" AND ");
                    builder.push_bind(&arr[1]);
                }
            }
            // ... other operators
            _ => {}
        }
    }
}
```

**Frontend filter UI:**

```svelte
<!-- GridToolbar.svelte -->
<script lang="ts">
  let { column, onFilterChange } = $props();
  let filterValue = $state('');
  let filterOperator = $state<FilterOperator>('contains');

  function handleFilterChange() {
    onFilterChange({
      id: column.id,
      operator: filterOperator,
      value: filterValue,
    });
  }

  $effect(() => {
    handleFilterChange();
  });
</script>

<div class="filter-control">
  <select bind:value={filterOperator}>
    {#if column.type === 'text'}
      <option value="contains">Contains</option>
      <option value="equals">Equals</option>
      <option value="startsWith">Starts with</option>
      <option value="endsWith">Ends with</option>
    {:else if column.type === 'number'}
      <option value="equals">Equals</option>
      <option value="gt">Greater than</option>
      <option value="lt">Less than</option>
      <option value="between">Between</option>
    {/if}
    <option value="isNull">Is null</option>
    <option value="isNotNull">Is not null</option>
  </select>
  
  {#if filterOperator !== 'isNull' && filterOperator !== 'isNotNull'}
    <input 
      type={column.type === 'number' ? 'number' : 'text'}
      bind:value={filterValue}
      placeholder="Filter value..."
    />
  {/if}
</div>
```

**Tasks:**
- [ ] Add filter state to `EnhancedGrid.svelte` with TanStack Table
- [ ] Create `GridToolbar.svelte` with per-column filter dropdowns
- [ ] Add filter operator types to `src/lib/types.ts`
- [ ] Implement `apply_filters` in Rust for safe `WHERE` construction
- [ ] Validate filter column names against table schema
- [ ] Debounce filter input (300ms) to avoid excessive queries
- [ ] Show "Filtering..." spinner while filter query runs
- [ ] Add "Clear filters" button
- [ ] Handle filter + sort combination
- [ ] Handle filter validation errors (e.g., non-numeric in number field)

**Success criteria:**
- [ ] Filter dropdown appears in column header or toolbar
- [ ] Typing filter value debounces and triggers query
- [ ] Text columns support contains, equals, starts/ends with
- [ ] Number columns support equals, comparison, between
- [ ] Date columns support date picker with before/after
- [ ] `IS NULL` / `IS NOT NULL` filters available for all columns
- [ ] Multiple filters combine with AND logic
- [ ] Clear filters button resets all filters
- [ ] Invalid filter input shows inline validation error

---

### Phase 5: Polish & Edge Cases

**Goal:** Handle edge cases, error states, and UX polish.

**Connection state handling:**

```svelte
<!-- SchemaBrowser.svelte -->
<script lang="ts">
  import { connectionStore } from '$lib/stores/connections.svelte';

  let isConnected = $derived(connectionStore.status === 'connected');
</script>

<div class="schema-browser" class:disabled={!isConnected}>
  {#if !isConnected}
    <div class="disconnected-banner">
      Connection lost. Reconnect to refresh schema.
    </div>
  {/if}
  
  <!-- Schema tree grays out when !isConnected -->
</div>
```

**Error states:**

| Error | UI Response |
|-------|-------------|
| Schema fetch timeout | Inline error in tree: "Schema load timed out" + Retry button |
| Permission denied | "Insufficient permissions" + partial tree (accessible tables only) |
| Table dropped mid-session | "Table no longer exists" + auto-refresh tree |
| Query timeout (sort/filter) | "Query timed out" + retry button in grid |
| Connection lost during operation | "Connection lost" banner + reconnect prompt |
| Invalid filter value | Inline validation error, don't submit query |

**Loading states:**

| Operation | UI Indicator |
|-----------|--------------|
| Schema tree load | Spinner in tree area |
| Column load (expand) | Spinner on table node |
| Data grid load | Skeleton grid + spinner |
| Sort/filter operation | "Sorting..." / "Filtering..." in toolbar |
| Load more | Spinner on button |

**Performance optimizations:**

1. **Schema cache** — Cache tables/columns in `DashMap<String, SchemaCache>` per connection
2. **Lazy column load** — Only fetch columns when table expanded
3. **Debounced filters** — 300ms debounce on filter input
4. **Query cancellation** — Cancel in-flight query when new sort/filter applied
5. **Virtual scrolling** — TanStack Virtual for large result sets

**Tasks:**
- [ ] Add connection state check to schema tree (gray out when disconnected)
- [ ] Add schema cache in Rust (`DashMap<String, SchemaCache>`)
- [ ] Implement cache invalidation on manual refresh
- [ ] Add error boundary for schema fetch failures
- [ ] Add loading spinners for all async operations
- [ ] Add skeleton UI for grid while loading
- [ ] Implement query cancellation for sort/filter
- [ ] Add keyboard navigation for tree (arrow keys, Enter, Escape)
- [ ] Add search box above tree for large schemas
- [ ] Add table row count display (optional, behind setting)
- [ ] Persist sort/filter state in `Core Config Manager`
- [ ] Handle long table/column names with tooltip truncation
- [ ] Add "Table dropped" error detection and auto-refresh

**Success criteria:**
- [ ] Tree grays out when connection lost
- [ ] Schema fetch errors show inline with retry
- [ ] All loading states have spinners
- [ ] Filter input debounced (no flicker)
- [ ] Keyboard navigation works (Tab, arrows, Enter)
- [ ] Long names truncated with tooltip
- [ ] Cache cleared on refresh button click

---

## Alternative Approaches Considered

### TanStack Virtual vs TanStack Table

**TanStack Virtual** (MVP plan) only handles virtualization. **TanStack Table** provides sorting, filtering, pagination, column resizing, and more.

**Decision: TanStack Table** — The enhanced grid needs sort/filter/pagination. TanStack Table includes virtualization plus these features. The MVP's `ResultGrid.svelte` can be replaced with `EnhancedGrid.svelte` using TanStack Table.

### Offset Pagination vs Cursor-Based Pagination

**Offset pagination** (`LIMIT 50 OFFSET 100`) is simple but inefficient for large offsets. **Cursor-based** (`WHERE id > last_id`) is faster but requires ordered unique column.

**Decision: Offset pagination for Phase 1** — Simpler implementation, works for most use cases. Cursor-based can be added later for performance-critical paths.

### Client-Side vs Server-Side Filter UI

**Client-side filtering** (TanStack default) filters loaded rows only. **Server-side** sends `WHERE` clauses to database.

**Decision: Server-side filtering** — User requested this for large tables. The grid should support filtering millions of rows, not just the loaded 1000.

### Inline Filter vs Toolbar Filter

**Inline filter** (filter input in column header) saves space. **Toolbar filter** (separate row above grid) provides more room for operators.

**Decision: Hybrid** — Simple text filter inline in header. Clicking filter icon opens dropdown with operator selection. This matches Excel's auto-filter UX.

---

## System-Wide Impact

### Interaction Graph

1. **Schema load on connect:**
   - User connects → `connect_database` → `get_schemas` → `get_tables` → `SchemaBrowser` populates → tables render
   - State: `$state({ schemas, tables, isLoading })`

2. **Tree expand:**
   - User clicks expand → `get_columns(table)` → Rust queries `information_schema` → columns streamed back → `SchemaNode` renders children

3. **Table open:**
   - User double-clicks → `query_table(params)` → Rust builds `SELECT * FROM table LIMIT 50` → results streamed via `Channel<QueryEvent>` → `EnhancedGrid` displays

4. **Sort operation:**
   - User clicks header → `setSorting([...])` → `query_table({ sort })` → Rust adds `ORDER BY` → new results stream → grid updates
   - **Cancel previous query** before starting new one

5. **Filter operation:**
   - User types in filter → debounce 300ms → `query_table({ filters })` → Rust adds `WHERE` → new results stream → grid updates

### Error Propagation

| Error Type | Origin | Handling |
|-----------|--------|---------|
| Schema query timeout | `sqlx::Error::Io` | Inline error in tree + retry button |
| Permission denied | `sqlx::Error::Database` | "Insufficient permissions" + partial tree |
| Table dropped | Query returns 0 | "Table no longer exists" + auto-refresh |
| Sort/filter timeout | `tokio::time::timeout` | "Query timed out" in grid toolbar + retry |
| Invalid filter value | Frontend validation | Inline validation error, no query sent |
| Column not found | `sqlx::Error::ColumnNotFound` | "Schema changed" + auto-refresh |

### State Lifecycle Risks

- **Stale schema cache** — User creates table externally, refresh needed. Mitigation: manual refresh button, document that schema is cached.
- **Sort/filter race conditions** — User clicks sort twice quickly. Mitigation: cancel previous query before starting new one.
- **Filter state on table switch** — Filters applied to `users`, user opens `orders`. Mitigation: clear filters on table switch, or persist per-table.

### API Surface Parity

- **`execute_query`** (MVP) — Runs arbitrary SQL, returns via Channel
- **`query_table`** (new) — Runs `SELECT` with sort/filter/pagination, returns via Channel

Both should use the same `Channel<QueryEvent>` pattern. `query_table` is a specialized version for table browsing.

### Integration Test Scenarios

1. **Happy path — Postgres schema browse:** Connect → schema dropdown shows `public`, `user_schema` → select `public` → tables list → expand `users` → columns show with types → double-click → grid shows data

2. **Filter + sort combination:** Open `orders` table → filter `status = 'pending'` → sort by `created_at DESC` → verify `WHERE status = 'pending' ORDER BY created_at DESC` in logs

3. **Large table pagination:** Open table with 10,000 rows → grid shows 50 rows → click "Load More" → next 50 rows appear → continue until 1000 limit → "Maximum rows reached" notice

4. **Connection lost recovery:** Open table → simulate network drop → tree grays out → reconnect → tree reloads → previous table/grid restored

5. **Schema refresh:** Connect → external process creates new table → click refresh → new table appears in tree

---

## Acceptance Criteria

### Functional Requirements

- [ ] **AC-01** Schema tree loads automatically when a connection is established
- [ ] **AC-02** Postgres connections show a schema dropdown (public, user schemas)
- [ ] **AC-03** MySQL connections show a database dropdown
- [ ] **AC-04** SQLite connections show tables directly (no dropdown)
- [ ] **AC-05** Tables display in alphabetical order in the tree
- [ ] **AC-06** Table nodes are expandable; expanding loads column info
- [ ] **AC-07** Column nodes show name and type (e.g., `id: uuid`, `total: numeric(10,2)`)
- [ ] **AC-08** Primary key columns show a key icon
- [ ] **AC-09** Foreign key columns show a link icon with tooltip showing referenced table
- [ ] **AC-10** Double-clicking a table opens a data grid with `SELECT *` results
- [ ] **AC-11** Data grid shows up to 50 rows initially
- [ ] **AC-12** "Load More" button fetches the next 50 rows
- [ ] **AC-13** Hard limit of 1000 rows enforced with "Maximum rows reached" notice
- [ ] **AC-14** Column headers display with sort indicators (arrows)
- [ ] **AC-15** Clicking a column header sorts data ascending
- [ ] **AC-16** Second click sorts descending
- [ ] **AC-17** Third click clears the sort
- [ ] **AC-18** Multi-column sort supported via Shift+click
- [ ] **AC-19** NULL values sort consistently (follow database default)
- [ ] **AC-20** Filter UI available per column (dropdown + input)
- [ ] **AC-21** Text columns support: contains, equals, starts with, ends with
- [ ] **AC-22** Number columns support: equals, greater than, less than, between
- [ ] **AC-23** Date columns support: before, after, between
- [ ] **AC-24** All columns support: is null, is not null
- [ ] **AC-25** Multiple filters combine with AND logic
- [ ] **AC-26** "Clear filters" button resets all filters
- [ ] **AC-27** Refresh button reloads schema from database
- [ ] **AC-28** Schema tree grays out when connection is lost
- [ ] **AC-29** "Reconnect to refresh" banner shown when disconnected
- [ ] **AC-30** Empty tables show "No data" message in grid
- [ ] **AC-31** Filter with no matches shows "No results match your filter"
- [ ] **AC-32** Invalid filter input shows inline validation error

### Non-Functional Requirements

- [ ] Schema tree loads in under 2 seconds for schemas with < 100 tables
- [ ] Column expand loads in under 500ms per table
- [ ] Sort/filter queries complete in under 5 seconds for tables with < 100K rows
- [ ] Grid renders 1000 rows at 60fps (virtual scrolling)
- [ ] Filter input debounced at 300ms to avoid excessive queries
- [ ] All async operations show loading indicators
- [ ] Keyboard navigation works for tree (arrows, Enter, Escape)

### Quality Gates

- [ ] No SQL injection vulnerabilities in sort/filter parameters (parameterized queries)
- [ ] Sort/filter column names validated against table schema
- [ ] All IPC commands return typed errors via `CommandError`
- [ ] Schema cache cleared on manual refresh
- [ ] Connection state checked before all schema operations

---

## Dependencies & Prerequisites

### Rust (Cargo.toml additions)

```toml
[dependencies]
# Existing dependencies from MVP...
tauri = { version = "2", features = ["protocol-asset"] }
Core Config Manager = "2"
sqlx = { version = "0.8", features = [
    "runtime-tokio", "tls-rustls",
    "postgres", "mysql", "sqlite", "any", "json"
] }
keyring = "3.3"
dashmap = "6"
futures = "0.3"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
uuid = { version = "1", features = ["v4"] }
url = "2"

# New for schema browser
tokio-util = "0.7"  # For CancellationToken
```

### Frontend (package.json additions)

```json
{
  "dependencies": {
    "@tauri-apps/api": "^2",
    "@tauri-apps/plugin-store": "^2"
  },
  "devDependencies": {
    "svelte": "^5",
    "@sveltejs/vite-plugin-svelte": "^5",
    "vite": "^6",
    "tailwindcss": "^4",
    "@tailwindcss/vite": "^4",
    "codemirror": "^6.0.1",
    "@codemirror/lang-sql": "^6.8.0",
    "@codemirror/theme-one-dark": "^6",
    "@codemirror/view": "^6",
    "@codemirror/state": "^6",
    "@tanstack/svelte-table": "^8.21",
    "@tanstack/svelte-virtual": "^3.13",
    "typescript": "^5"
  }
}
```

### Prerequisites

- MVP implementation complete (connection manager, SQL editor, basic result grid)
- Rust 1.77.2+
- Node.js 20+ / pnpm 9+
- Tauri CLI v2

---

## Risk Analysis & Mitigation

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| SQL injection in sort/filter params | Medium | High | Use parameterized queries via `QueryBuilder`; validate column names against schema allowlist |
| Schema query timeout on slow connections | Medium | Medium | Add 10-second timeout; show inline error with retry |
| Large schema (1000+ tables) slow to render | Medium | Medium | Virtualize tree rendering; lazy-load table children; show search box |
| Filter UI complexity for all data types | Low | Medium | Start with text/number/date filters; defer advanced types |
| TanStack Table Svelte 5 compatibility | Low | High | Use latest `@tanstack/svelte-table@8.21+`; test early in Phase 2 |
| Schema cache memory leak | Low | Medium | Use `DashMap` with cleanup on connection close; limit cache size |
| Filter + sort + pagination query complexity | Medium | Medium | Use `QueryBuilder` pattern; test with various combinations |
| Permission denied on information_schema | Medium | Low | Show partial tree with accessible tables; inline error message |

---

## Future Considerations

- **Views and materialized views** — Show in tree with different icon
- **Functions and procedures** — Add to tree or separate panel
- **Table row counts** — Optional display (requires `COUNT(*)` query)
- **Schema diff** — Compare schema across environments
- **DDL generation** — Generate `CREATE TABLE` statements
- **Tabbed data grids** — Multiple tables open simultaneously
- **Cursor-based pagination** — For tables with billions of rows
- **Column resize/reorder** — Persist column preferences
- **Export filtered data** — CSV/JSON export of grid results
- **Advanced filters** — Regex, date ranges, multi-select

---

## File Structure

```
sqlator/
├── core/
│   └── src/
│       ├── db/                  # Core DB introspection
├── tauri-app/
│   ├── src-tauri/
│   │   ├── Cargo.toml
│   │   ├── tauri.conf.json
│   │   └── src/
│       ├── lib.rs
│       ├── state.rs
│       ├── commands/
│       │   ├── mod.rs
│       │   ├── connections.rs
│       │   ├── queries.rs       # Add query_table here
│       │   └── schema.rs        # NEW: get_schemas, get_tables, get_columns
│       ├── models.rs            # Add SchemaInfo, TableInfo, ColumnInfo, SortSpec, FilterSpec
│       └── error.rs
├── src/
│   ├── app.css
│   ├── main.ts
│   ├── App.svelte
│   ├── lib/
│   │   ├── constants/
│   │   │   └── colors.ts
│   │   ├── types.ts             # Add schema-related types
│   │   ├── stores/
│   │   │   ├── connections.svelte.ts
│   │   │   ├── query.svelte.ts
│   │   │   ├── theme.svelte.ts
│   │   │   ├── schema.svelte.ts     # NEW
│   │   │   └── grid.svelte.ts       # NEW
│   │   └── components/
│   │       ├── Sidebar.svelte
│   │       ├── ConnectionItem.svelte
│   │       ├── ConnectionForm.svelte
│   │       ├── SqlEditor.svelte
│   │       ├── EditorToolbar.svelte
│   │       ├── ResultPane.svelte
│   │       ├── ResultGrid.svelte       # Keep for custom SQL queries
│   │       ├── EnhancedGrid.svelte      # NEW: TanStack Table
│   │       ├── GridToolbar.svelte       # NEW
│   │       ├── LoadMoreButton.svelte    # NEW
│   │       ├── SchemaBrowser.svelte     # NEW
│   │       ├── SchemaTree.svelte        # NEW
│   │       ├── SchemaNode.svelte        # NEW
│   │       ├── SchemaDropdown.svelte    # NEW
│   │       └── ThemeToggle.svelte
│   └── vite-env.d.ts
├── vite.config.ts
├── package.json
└── tsconfig.json
```

---

## Sources & References

### Origin

- **Origin document:** [docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md](docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md)
- Key decisions carried forward:
  - Tauri 2 + Svelte 5 + Tailwind v4 stack
  - `sqlx 0.8` with `AnyPool` for multi-database support
  - `tauri::ipc::Channel` for streaming large results
  - `$state` runes for reactive state management
  - 1000-row hard limit with warning notice
  - `CommandError` enum for typed error handling

### Internal References

- Architecture patterns: `docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md:43-74`
- IPC command patterns: `docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md:107-117`
- Streaming pattern: `docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md:120-131`
- State management: `docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md:656-658`
- Error handling: `docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md:477-485`

### External References

- [TanStack Table Svelte](https://tanstack.com/table/latest/docs/framework/svelte/overview) — Server-side sorting/filtering
- [TanStack Virtual](https://tanstack.com/virtual/latest) — Virtual scrolling
- [PostgreSQL information_schema](https://www.postgresql.org/docs/current/information-schema.html) — Schema introspection
- [MySQL information_schema](https://dev.mysql.com/doc/refman/8.0/en/information-schema.html) — Schema introspection
- [SQLite schema](https://www.sqlite.org/schematab.html) — sqlite_master table
- [sqlx QueryBuilder](https://docs.rs/sqlx/latest/sqlx/query_builder/struct.QueryBuilder.html) — Dynamic query construction

### Key Gotchas

1. **Validate all column names** in sort/filter against table schema to prevent SQL injection
2. **Use parameterized queries** via `QueryBuilder` — never interpolate user input
3. **Cancel previous query** before starting new sort/filter to avoid race conditions
4. **Debounce filter input** (300ms) to avoid excessive database queries
5. **Cache schema metadata** to avoid repeated `information_schema` queries
6. **TanStack Table requires `manualSorting`, `manualFiltering`, `manualPagination`** for server-side ops
7. **Svelte 5 uses `createSvelteTable`** with writable store for options
8. **SQLite PRAGMA doesn't support parameterized table names** — validate against allowlist
