---
title: "feat: Open Schema DDL Tab from Right-Click Context Menu"
type: feat
status: active
date: 2026-04-12
origin: docs/plans/2026-04-04-005-feat-database-schema-browser-plan.md
---

# ✨ Open Schema DDL Tab from Right-Click Context Menu

Add a right-click "Open Schema" option to table names in the schema browser tree. Selecting it opens a new tab containing the `CREATE TABLE` (or `CREATE VIEW`) DDL statement, displayed read-only with syntax highlighting.

---

## Overview

This feature extends the schema browser with a right-click context menu on table nodes, allowing users to view the DDL (Data Definition Language) for any table or view. The DDL is fetched from the database and displayed in a dedicated read-only tab with a Copy button.

**Core Capabilities:**
1. **Right-click context menu** on table nodes in `SchemaNode.svelte`
2. **DDL retrieval** via a new `get_ddl` Tauri command with per-database strategies
3. **DDL viewer tab** — read-only, syntax-highlighted, with Copy and Refresh buttons
4. **Tab deduplication** — reuses existing DDL tab for the same table

---

## Problem Statement

The schema browser shows table names and column metadata, but users cannot see the full `CREATE TABLE` statement. Understanding DDL is essential for:
- Checking constraints, indexes, and default values
- Reviewing column types with full precision (e.g., `numeric(10,2)` vs simplified `decimal`)
- Understanding partitioning, storage engine, and charset settings
- Copying DDL for documentation or migration scripts

Other SQL clients (DataGrip, DBeaver, Beekeeper Studio) all offer "Show DDL" or "Show Create Table" functionality. SQLator needs this feature matching the existing Tauri + Svelte 5 architecture.

---

## Proposed Solution

A three-part implementation:

1. **Context menu on SchemaNode** — reuse `ContextMenu.svelte` component
2. **`get_ddl` backend command** — per-database DDL retrieval (native `SHOW CREATE TABLE` where available, catalog reconstruction for PostgreSQL/MSSQL)
3. **DDL tab in the tab system** — new `schemaDdl` discriminant on `QueryTab`, rendered as a read-only CodeMirror viewer

---

## Technical Considerations

### DDL Retrieval Strategy Per Database

| Database | Strategy | Command / Query | Completeness |
|----------|----------|-----------------|--------------|
| **MySQL** | Native | `SHOW CREATE TABLE schema.table` | Full DDL (indexes, constraints, charset, engine) |
| **SQLite** | Native | `SELECT sql FROM sqlite_master WHERE name = ?` | Original DDL as stored by SQLite |
| **ClickHouse** | Native | `SHOW CREATE TABLE db.table` | Full DDL (engine, partition key, order by) |
| **Oracle** | Native | `SELECT DBMS_METADATA.GET_DDL('TABLE', name, schema) FROM dual` | Full DDL (may include storage/tablespace clauses) |
| **PostgreSQL** | Reconstruct | Query `pg_catalog` for columns, constraints, indexes | Columns + NOT NULL + defaults + PK + FK (indexes as future enhancement) |
| **MSSQL** | Reconstruct | Query `sys.*` catalog views | Columns + NOT NULL + defaults + PK + FK (indexes as future enhancement) |

**Decision:** Use native commands where available; reconstruct from catalog queries for PostgreSQL and MSSQL. Reconstructed DDL may not be byte-identical to original but will be functionally equivalent. (see origin: `docs/plans/2026-04-04-005-feat-database-schema-browser-plan.md:949` — DDL generation was listed as a future consideration)

**PostgreSQL DDL reconstruction scope (Phase 1):**
- Column names, types, NOT NULL, defaults
- PRIMARY KEY constraints
- FOREIGN KEY constraints
- UNIQUE constraints
- CHECK constraints

**Out of scope for initial implementation:**
- Index definitions (can be added later)
- Partitioning clauses
- Triggers, rules
- Storage parameters

### Tab Type Design

Add `schemaDdl` as a new optional discriminant on `QueryTab`, parallel to the existing `tableBrowse`:

```typescript
// src/lib/types.ts (addition)

export interface SchemaDdlState {
  tableName: string;
  schema?: string;
  connectionId: string;
  ddl: string | null;
  isLoading: boolean;
  error: string | null;
}
```

The `QueryTab` interface becomes:

```typescript
export interface QueryTab {
  id: string;
  label: string;
  sql: string;
  isDirty: boolean;
  result: ResultPaneState;
  isExecuting: boolean;
  tableBrowse?: TableBrowseState;
  schemaDdl?: SchemaDdlState;     // NEW
}
```

**Tab rendering dispatch in `TabbedEditor.svelte`:**
```
if (tableBrowse)    → EnhancedGrid
else if (schemaDdl) → SchemaDdlViewer  (NEW)
else                → SqlEditor + ResultPane
```

**Deduplication:** DDL tabs deduplicate independently from browse tabs. Matching on `schemaDdl.tableName + schemaDdl.schema`. A DDL tab and a browse tab for the same table coexist as separate tabs.

### Read-Only DDL Viewer

The DDL viewer uses a read-only CodeMirror 6 instance with SQL syntax highlighting (already a project dependency: `@codemirror/lang-sql`). This provides:
- Syntax highlighting for SQL
- Monospace font with proper indentation
- Line numbers
- Text selection for manual copying

Additionally, a toolbar provides:
- **Copy button** — copies DDL to clipboard
- **Refresh button** — re-fetches DDL from database
- **"Open in Editor" button** — creates a new SQL editor tab with DDL pre-populated (future enhancement, tracked in Future Considerations)

### Context Menu Design

Reuse the existing `ContextMenu.svelte` component. Menu items for table nodes:

| Action | Label | Always Available |
|--------|-------|-----------------|
| `open-table` | Open Table | Yes |
| `open-schema` | Open Schema | Yes |

For view nodes:

| Action | Label | Always Available |
|--------|-------|-----------------|
| `open-table` | Open View Data | Yes |
| `open-schema` | Open Schema | Yes |

Single "Open Schema" label for both tables and views — the DDL content adapts automatically (`CREATE TABLE` vs `CREATE VIEW`).

---

## System-Wide Impact

### Interaction Graph

1. **Right-click on table node:**
   - User right-clicks → `oncontextmenu` handler in `SchemaNode.svelte` → builds `ContextMenuItem[]` → shows `ContextMenu` at cursor position → user clicks "Open Schema" → `onopenschema(table)` callback fires

2. **DDL tab creation:**
   - `onopenschema` → bubbles through `SchemaTree → SchemaBrowser → Sidebar` → `tabs.openSchemaDdl(connectionId, table)` → creates `QueryTab` with `schemaDdl` state → tab becomes active

3. **DDL fetch:**
   - Tab renders `SchemaDdlViewer` → `onMount` calls `invoke('get_ddl', { connectionId, tableName, schema })` → Rust dispatches to per-database DDL retrieval → returns DDL string → `SchemaDdlState.ddl` updated → viewer displays

4. **DDL refresh:**
   - User clicks Refresh → `invoke('get_ddl', ...)` again → DDL updated in state

5. **Copy to clipboard:**
   - User clicks Copy → `navigator.clipboard.writeText(ddl)` → toast "DDL copied to clipboard"

### Error Propagation

| Error | Origin | Handling |
|-------|--------|----------|
| Permission denied for DDL query | `sqlx::Error::Database` | Show error in DDL tab: "Insufficient permissions to read table DDL" |
| Table dropped between tree load and DDL fetch | Query returns empty | Show error: "Table not found. It may have been dropped." |
| Oracle `DBMS_METADATA` not accessible | `sqlx::Error::Database` | Show error: "DBMS_METADATA access required for DDL retrieval" |
| PostgreSQL catalog query fails | `sqlx::Error` | Show error with message + retry button |
| Connection lost during fetch | `sqlx::Error::Io` | Show "Connection lost" in DDL tab |

### State Lifecycle Risks

- **Stale DDL** — User alters table after viewing DDL. Mitigation: Refresh button re-fetches. DDL is not cached.
- **DDL tab + browse tab coexistence** — Two tabs for same table. Mitigation: Different tab labels and icons; independent deduplication.
- **Table dropped mid-session** — DDL tab still shows old content. Mitigation: DDL is a point-in-time snapshot; Refresh will detect the drop.

### API Surface Parity

- **`get_ddl`** (new) — Returns DDL string for a table/view. Follows same `connection_id + table_name + schema` pattern as `get_columns`, `query_table`.

---

## Acceptance Criteria

- [ ] **AC-01** Right-clicking a table name in the schema tree shows a context menu with "Open Schema" option
- [ ] **AC-02** Right-clicking a view name in the schema tree shows a context menu with "Open Schema" option
- [ ] **AC-03** Clicking "Open Schema" opens a new tab with the table/view DDL
- [ ] **AC-04** DDL tab displays `CREATE TABLE ...` for tables and `CREATE VIEW ...` for views
- [ ] **AC-05** DDL tab shows read-only, syntax-highlighted SQL with line numbers
- [ ] **AC-06** DDL tab has a "Copy" button that copies DDL to clipboard
- [ ] **AC-07** DDL tab has a "Refresh" button that re-fetches DDL from database
- [ ] **AC-08** Opening schema for an already-open table reuses the existing DDL tab
- [ ] **AC-09** A DDL tab and a browse tab for the same table coexist as separate tabs
- [ ] **AC-10** MySQL returns DDL via `SHOW CREATE TABLE`
- [ ] **AC-11** SQLite returns DDL via `sqlite_master`
- [ ] **AC-12** ClickHouse returns DDL via `SHOW CREATE TABLE`
- [ ] **AC-13** Oracle returns DDL via `DBMS_METADATA.GET_DDL`
- [ ] **AC-14** PostgreSQL returns reconstructed DDL from `pg_catalog` (columns, NOT NULL, defaults, PK, FK, UNIQUE, CHECK)
- [ ] **AC-15** MSSQL returns reconstructed DDL from `sys.*` catalog views
- [ ] **AC-16** Permission errors show inline error in DDL tab with retry option
- [ ] **AC-17** Table-not-found errors show inline error: "Table may have been dropped"
- [ ] **AC-18** DDL tab label shows `DDL: schema.table` format (or `DDL: table` for SQLite)
- [ ] **AC-19** Context menu also includes "Open Table" action for convenience
- [ ] **AC-20** Loading state shows spinner while DDL is being fetched
- [ ] **AC-21** Table names with special characters (quotes, spaces, reserved words) are handled correctly in DDL queries

---

## MVP

### `core/src/db/mod.rs`

```rust
pub async fn get_ddl(
    &self,
    connection_id: &str,
    table_name: &str,
    schema: Option<&str>,
) -> Result<String, CoreError> {
    let pool = self.pools.get(connection_id)
        .ok_or_else(|| CoreError { message: "Not connected".into(), code: "NO_CONNECTION".into() })?
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
```

### `core/src/db/mysql.rs` — MySQL DDL

```rust
pub async fn get_ddl(pool: &MySqlPool, table_name: &str, schema: Option<&str>) -> Result<String, CoreError> {
    let qualified = match schema {
        Some(s) => format!("`{}`.`{}`", s.replace('`', "``"), table_name.replace('`', "``")),
        None => format!("`{}`", table_name.replace('`', "``")),
    };
    let sql = format!("SHOW CREATE TABLE {}", qualified);
    let row = sqlx::query(&sql)
        .fetch_one(pool)
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "DDL_QUERY".into() })?;
    // MySQL returns column "Create Table"
    Ok(row.try_get::<String, _>("Create Table")
        .or_else(|_| row.try_get::<String, _>(1))
        .unwrap_or_default())
}
```

### `core/src/db/sqlite.rs` — SQLite DDL

```rust
pub async fn get_ddl(pool: &SqlitePool, table_name: &str) -> Result<String, CoreError> {
    let safe_name = table_name.replace('"', "\"\"");
    let sql = format!("SELECT sql FROM sqlite_master WHERE name = \"{}\"", safe_name);
    let row = sqlx::query(&sql)
        .fetch_one(pool)
        .await
        .map_err(|e| CoreError { message: e.to_string(), code: "DDL_QUERY".into() })?;
    Ok(row.try_get::<String, _>("sql").unwrap_or_default())
}
```

### `core/src/db/clickhouse.rs` — ClickHouse DDL

```rust
pub async fn get_ddl(pool: &ClickHousePool, table_name: &str, schema: Option<&str>) -> Result<String, CoreError> {
    // ClickHouse uses SHOW CREATE TABLE db.table
    // Implementation follows existing clickhouse.rs patterns
}
```

### `core/src/db/oracle.rs` — Oracle DDL

```rust
pub async fn get_ddl(pool: &OraclePool, table_name: &str, schema: Option<&str>) -> Result<String, CoreError> {
    // SELECT DBMS_METADATA.GET_DDL('TABLE', :table_name, :schema) FROM dual
    // For views: DBMS_METADATA.GET_DDL('VIEW', ...)
}
```

### `core/src/db/postgres.rs` — PostgreSQL DDL (reconstructed)

```rust
pub async fn get_ddl(pool: &PgPool, table_name: &str, schema: Option<&str>) -> Result<String, CoreError> {
    let schema = schema.unwrap_or("public");
    let safe_table = table_name.replace('"', "\"\"");
    let safe_schema = schema.replace('"', "\"\"");
    let qualified = format!("\"{}\".\"{}\"", safe_schema, safe_table);

    // 1. Fetch columns from pg_catalog
    // 2. Fetch PK constraint
    // 3. Fetch FK constraints
    // 4. Fetch UNIQUE constraints
    // 5. Fetch CHECK constraints
    // 6. Assemble CREATE TABLE statement
    //    CREATE TABLE schema.table (
    //      col1 type NOT NULL DEFAULT val,
    //      col2 type,
    //      CONSTRAINT pk_name PRIMARY KEY (col1),
    //      CONSTRAINT fk_name FOREIGN KEY (col2) REFERENCES other(col),
    //      CONSTRAINT uq_name UNIQUE (col1, col2),
    //      CONSTRAINT ck_name CHECK (...)
    //    );
}
```

### `core/src/db/mssql.rs` — MSSQL DDL (reconstructed)

```rust
pub async fn get_ddl(pool: &MssqlPool, table_name: &str, schema: Option<&str>) -> Result<String, CoreError> {
    // Query sys.columns, sys.indexes, sys.foreign_keys, sys.check_constraints
    // Assemble CREATE TABLE statement
}
```

### `src-tauri/src/commands.rs`

```rust
#[tauri::command]
pub async fn get_ddl(
    state: State<'_, AppState>,
    connection_id: String,
    table_name: String,
    schema: Option<String>,
) -> Result<String, CommandError> {
    state.db_manager
        .get_ddl(&connection_id, &table_name, schema.as_deref())
        .await
        .map_err(|e| CommandError { message: e.message, code: e.code })
}
```

### `src/lib/types.ts`

```typescript
export interface SchemaDdlState {
  tableName: string;
  schema?: string;
  connectionId: string;
  ddl: string | null;
  isLoading: boolean;
  error: string | null;
}
```

### `src/lib/stores/tabs.svelte.ts`

```typescript
openSchemaDdl(connectionId: string, table: TableInfo) {
  const ct = connectionTabs.find((t) => t.connectionId === connectionId);
  if (!ct) return;

  // Reuse existing DDL tab for same table
  const existing = ct.queryTabs.find(
    (t) => t.schemaDdl?.tableName === table.name && t.schemaDdl?.schema === table.schema
  );
  if (existing) {
    connectionTabs = connectionTabs.map((t) =>
      t.connectionId === connectionId ? { ...t, activeQueryTabId: existing.id } : t
    );
    return;
  }

  const ddlState: SchemaDdlState = {
    tableName: table.name,
    schema: table.schema,
    connectionId,
    ddl: null,
    isLoading: true,
    error: null,
  };

  const label = table.schema
    ? `DDL: ${table.schema}.${table.name}`
    : `DDL: ${table.name}`;

  const newTab: QueryTab = {
    id: crypto.randomUUID(),
    label,
    sql: "",
    isDirty: false,
    result: { kind: "idle" },
    isExecuting: false,
    schemaDdl: ddlState,
  };

  connectionTabs = connectionTabs.map((t) =>
    t.connectionId === connectionId
      ? { ...t, queryTabs: [...t.queryTabs, newTab], activeQueryTabId: newTab.id }
      : t
  );
},

updateSchemaDdlState(connectionId: string, queryTabId: string, patch: Partial<SchemaDdlState>) {
  connectionTabs = connectionTabs.map((ct) =>
    ct.connectionId === connectionId
      ? {
          ...ct,
          queryTabs: ct.queryTabs.map((qt) =>
            qt.id === queryTabId && qt.schemaDdl
              ? { ...qt, schemaDdl: { ...qt.schemaDdl, ...patch } }
              : qt
          ),
        }
      : ct
  );
},
```

### `src/lib/components/SchemaNode.svelte`

```svelte
<script lang="ts">
  import ContextMenu, { type ContextMenuItem } from "./ContextMenu.svelte";
  import type { TableInfo, SchemaColumnInfo } from "$lib/types";

  let {
    table,
    columns,
    isExpanded = false,
    isLoadingColumns = false,
    onexpand,
    onopen,
    onopenschema,
  }: {
    table: TableInfo;
    columns: SchemaColumnInfo[] | null;
    isExpanded?: boolean;
    isLoadingColumns?: boolean;
    onexpand: (table: TableInfo) => void;
    onopen: (table: TableInfo) => void;
    onopenschema: (table: TableInfo) => void;
  } = $props();

  let contextMenu = $state<{ x: number; y: number } | null>(null);

  const contextItems: ContextMenuItem[] = [
    { label: "Open Table", action: "open-table" },
    { label: "Open Schema", action: "open-schema" },
  ];

  function handleContextMenu(e: MouseEvent) {
    e.preventDefault();
    contextMenu = { x: e.clientX, y: e.clientY };
  }

  function handleContextSelect(action: string) {
    if (action === "open-table") onopen(table);
    if (action === "open-schema") onopenschema(table);
    contextMenu = null;
  }

  // ... existing handlers
</script>

<div class="schema-node">
  <button
    class="table-row"
    class:expanded={isExpanded}
    onclick={handleToggle}
    ondblclick={handleDblClick}
    oncontextmenu={handleContextMenu}
    title="Single-click to expand, double-click to open table"
  >
    <!-- ... existing content -->
  </button>

  <!-- ... column list -->

  {#if contextMenu}
    <ContextMenu
      x={contextMenu.x}
      y={contextMenu.y}
      items={contextItems}
      onselect={handleContextSelect}
      onclose={() => contextMenu = null}
    />
  {/if}
</div>
```

### `src/lib/components/SchemaDdlViewer.svelte`

```svelte
<script lang="ts">
  import { EditorView } from "@codemirror/view";
  import { EditorState } from "@codemirror/state";
  import { sql } from "@codemirror/lang-sql";
  import { oneDark } from "@codemirror/theme-one-dark";
  import { api } from "$lib/api";
  import type { SchemaDdlState } from "$lib/types";

  let {
    ddlState,
    onStateChange,
  }: {
    ddlState: SchemaDdlState;
    onStateChange: (patch: Partial<SchemaDdlState>) => void;
  } = $props();

  let editorContainer: HTMLElement | undefined = $state();
  let view: EditorView | undefined = $state();
  let copyFeedback = $state(false);

  async function fetchDdl() {
    onStateChange({ isLoading: true, error: null });
    try {
      const ddl = await api.invoke<string>("get_ddl", {
        connectionId: ddlState.connectionId,
        tableName: ddlState.tableName,
        schema: ddlState.schema,
      });
      onStateChange({ ddl, isLoading: false });
    } catch (e) {
      onStateChange({ error: String(e), isLoading: false });
    }
  }

  $effect(() => {
    if (!editorContainer) return;
    if (view) view.destroy();
    if (!ddlState.ddl) return;

    view = new EditorView({
      state: EditorState.create({
        doc: ddlState.ddl,
        extensions: [
          sql(),
          oneDark,
          EditorView.editable.of(false),
          EditorView.lineWrapping,
        ],
      }),
      parent: editorContainer,
    });

    return () => view?.destroy();
  });

  async function handleCopy() {
    if (!ddlState.ddl) return;
    await navigator.clipboard.writeText(ddlState.ddl);
    copyFeedback = true;
    setTimeout(() => copyFeedback = false, 2000);
  }

  $effect(() => {
    if (ddlState.isLoading && !ddlState.ddl) fetchDdl();
  });
</script>

<div class="ddl-viewer">
  <div class="ddl-toolbar">
    <span class="ddl-title">🔧 {ddlState.schema ? `${ddlState.schema}.${ddlState.tableName}` : ddlState.tableName}</span>
    <div class="ddl-actions">
      <button class="ddl-btn" onclick={fetchDdl} disabled={ddlState.isLoading}>Refresh</button>
      <button class="ddl-btn" onclick={handleCopy} disabled={!ddlState.ddl}>
        {copyFeedback ? "Copied!" : "Copy"}
      </button>
    </div>
  </div>

  {#if ddlState.isLoading && !ddlState.ddl}
    <div class="ddl-loading"><div class="spinner"></div> Loading DDL...</div>
  {:else if ddlState.error}
    <div class="ddl-error">
      <p>{ddlState.error}</p>
      <button class="ddl-btn" onclick={fetchDdl}>Retry</button>
    </div>
  {:else if ddlState.ddl}
    <div class="ddl-editor" bind:this={editorContainer}></div>
  {/if}
</div>
```

### `src/lib/components/TabbedEditor.svelte` (modification)

Add a new branch to the tab rendering dispatch:

```svelte
{#if activeQueryTab.tableBrowse}
  <!-- Table browse mode -->
{:else if activeQueryTab.schemaDdl}
  <SchemaDdlViewer
    ddlState={activeQueryTab.schemaDdl}
    onStateChange={(patch) => {
      tabs.updateSchemaDdlState(
        activeConnectionTab.connectionId,
        activeQueryTab.id,
        patch
      );
    }}
  />
{:else}
  <!-- SQL editor mode -->
{/if}
```

### `src/lib/components/QueryTabBar.svelte` (modification)

Add a visual indicator to distinguish DDL tabs from browse/query tabs:

- DDL tab label: `🔧 DDL: schema.table`
- Browse tab label: `📋 schema.table` (existing)
- Query tab label: `Query N` (existing)

---

## Dependencies & Risks

### Dependencies

- **CodeMirror 6** — Already in `package.json` (`codemirror`, `@codemirror/lang-sql`, `@codemirror/theme-one-dark`). Read-only mode via `EditorView.editable.of(false)`.
- **ContextMenu.svelte** — Already implemented and reusable.
- **`get_columns` / `get_tables` infrastructure** — PostgreSQL and MSSQL DDL reconstruction reuses the same `information_schema` / `pg_catalog` / `sys.*` query patterns.

### Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| PostgreSQL DDL reconstruction incomplete (missing indexes, partitioning) | High | Medium | Start with columns + constraints; indexes as follow-up. Show note in UI: "Excludes index definitions" |
| Oracle `DBMS_METADATA` returns verbose storage clauses | Medium | Low | Show verbatim; users can copy and edit |
| MySQL `SHOW CREATE TABLE` returns `VARBINARY` columns (MySQL 8 gotcha) | Medium | Medium | Use `CAST(... AS CHAR)` for DDL query, same pattern as `get_columns_mysql` |
| SQLite `sqlite_master.sql` returns `null` for auto-indexes | Low | Low | Already filtered by `name NOT LIKE 'sqlite_%'` in `get_tables_sqlite` |
| MSSQL DDL reconstruction from `sys.*` complex | Medium | Medium | Follow same incremental approach as PostgreSQL; start with columns + PK/FK |
| Table name quoting issues (special chars, reserved words) | Medium | High | Reuse existing quoting functions from `query_table_*` implementations |

---

## Future Considerations

- **Index definitions in reconstructed DDL** — Add `pg_indexes` / `sys.indexes` queries for PostgreSQL/MSSQL
- **"Open in Editor" button** — Create a new SQL editor tab with DDL pre-populated for editing
- **ALTER TABLE diff** — Compare DDL between two tables/schemas
- **Materialized view DDL** — `CREATE MATERIALIZED VIEW` for PostgreSQL, Oracle, ClickHouse
- **Function/procedure DDL** — Extend beyond tables and views
- **DDL for partitioned tables** — Include `PARTITION BY` clauses
- **Simplified Oracle DDL** — Option to strip storage/tablespace clauses from `DBMS_METADATA` output

---

## Sources & References

### Origin

- **Origin document:** [docs/plans/2026-04-04-005-feat-database-schema-browser-plan.md](docs/plans/2026-04-04-005-feat-database-schema-browser-plan.md) — DDL generation listed as future consideration (line 949)
- Key decisions carried forward:
  - Tauri 2 + Svelte 5 + Tailwind v4 stack
  - `sqlx 0.8` with per-database pool types for multi-DB support
  - `$state` runes for reactive state management
  - `ContextMenu.svelte` for right-click interactions
  - `QueryTab` discriminant pattern for tab type dispatch

### Internal References

- Tab system: `src/lib/stores/tabs.svelte.ts:98-140` (openTableBrowse pattern to follow)
- Tab dispatch: `src/lib/components/TabbedEditor.svelte:95-111`
- Context menu: `src/lib/components/ContextMenu.svelte`
- SchemaNode click flow: `src/lib/components/SchemaNode.svelte:20-27`
- Schema tree event wiring: `src/lib/components/Sidebar.svelte` (handleTableOpen)
- MySQL VARBINARY gotcha: `core/src/db/mod.rs:497-513` (CAST AS CHAR pattern)
- SQLite PRAGMA sanitization: `core/src/db/mod.rs:735-736`
- PostgreSQL column query: `core/src/db/mod.rs:611-670`
- PostgreSQL FK query: `core/src/db/mod.rs:639-649`

### External References

- [MySQL SHOW CREATE TABLE](https://dev.mysql.com/doc/refman/8.0/en/show-create-table.html)
- [PostgreSQL pg_catalog](https://www.postgresql.org/docs/current/catalogs.html)
- [SQLite sqlite_master](https://www.sqlite.org/schematab.html)
- [Oracle DBMS_METADATA.GET_DDL](https://docs.oracle.com/en/database/oracle/oracle-database/19/arpls/DBMS_METADATA.html)
- [ClickHouse SHOW CREATE TABLE](https://clickhouse.com/docs/en/sql-reference/statements/show#create-table)
- [MSSQL sys.tables catalog](https://learn.microsoft.com/en-us/sql/relational-databases/system-catalog-views/sys-tables-transact-sql)
- [CodeMirror 6 EditorView.editable](https://codemirror.net/docs/ref/#view.EditorView.editable)
