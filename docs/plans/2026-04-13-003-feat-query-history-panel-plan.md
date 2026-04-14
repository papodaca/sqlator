---
title: "feat: Query History Panel"
type: feat
status: active
date: 2026-04-13
---

# feat: Query History Panel

## Overview

Add a persistent query history system to sqlator that records every executed SQL query — including metadata (timestamp, connection/database, duration, row count) and a capped result preview — and surfaces it in a browsable, searchable, re-runnable panel in both the Tauri desktop UI and TUI.

"Never throw away data" means append-only storage with a high cap (e.g., 50,000 entries) and explicit user-initiated clearing only. No automatic pruning.

---

## Problem Statement

Users frequently re-execute queries they've already written but don't have a way to find them after closing a tab or session. The current tab persistence only saves the *latest* SQL text per tab — not a log of every executed statement. There is no way to see what ran, when, how long it took, or what it returned without re-running it.

---

## Proposed Solution

1. **Storage layer**: A dedicated `~/.config/sqlator/history.db` SQLite database opened at app startup via a new `HistoryManager` in `core`. This avoids bloating `connections.json` and sidesteps the full-file-rewrite cost of the existing `ConfigManager` pattern.

2. **Capture hook**: Record each query at the point where its terminal event (`Done`, `RowsAffected`, or `Error`) is received — capturing SQL text, connection metadata, timing, and a capped row preview.

3. **Desktop UI**: A new History panel accessible as a special query-tab variant (following the existing `tableBrowse` / `schemaDdl` discriminant pattern) plus a dedicated `HistoryPanel.svelte` component with search, filter, and re-run.

4. **TUI**: A new `AppMode::History` (full-screen overlay) following the same `AppMode::NewConnection` blueprint from the in-flight `2026-04-13-001` plan.

---

## Technical Approach

### Architecture

```
User executes SQL
      │
      ▼
execute_query (Tauri command / TUI)
      │  captures sql, connection_id, timestamp_start
      │
      ▼
QueryEvent stream  ──→  Done { row_count, duration_ms }
      │                  RowsAffected { count, duration_ms }
      │                  Error { message }
      │
      ▼
HistoryManager::record()
  └─ INSERT INTO query_history (...)
        ~/.config/sqlator/history.db
```

### Database Schema (`history.db`)

```sql
CREATE TABLE IF NOT EXISTS query_history (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    sql_text    TEXT    NOT NULL,
    conn_name   TEXT    NOT NULL,
    db_type     TEXT    NOT NULL,     -- "postgres", "mysql", "sqlite", etc.
    database    TEXT,                 -- active database/schema name if known
    executed_at TEXT    NOT NULL,     -- ISO 8601 UTC, e.g. "2026-04-13T14:23:01Z"
    duration_ms INTEGER,              -- NULL if errored before completion
    row_count   INTEGER,              -- NULL for DML / error
    rows_affected INTEGER,            -- NULL for SELECT / error
    status      TEXT    NOT NULL CHECK(status IN ('ok','error','rows_affected')),
    error_msg   TEXT,                 -- NULL unless status = 'error'
    result_preview TEXT               -- JSON blob, first 100 rows × all columns; NULL if no rows
);

CREATE INDEX IF NOT EXISTS idx_history_executed_at ON query_history(executed_at DESC);
CREATE INDEX IF NOT EXISTS idx_history_conn       ON query_history(conn_name);
```

**Cap enforcement**: On INSERT, if `COUNT(*) >= 50000`, delete the oldest `COUNT(*) - 50000 + 1` rows. This is the only automatic pruning mechanism; otherwise data is never thrown away.

**Result preview**: Store first 100 rows as a compact JSON array of arrays (not objects, to save space). On re-run the live result replaces this preview.

### Core (`core/src/history.rs`) — new file

```rust
// core/src/history.rs
pub struct HistoryManager {
    pool: sqlx::SqlitePool,
}

pub struct HistoryEntry {
    pub id: i64,
    pub sql_text: String,
    pub conn_name: String,
    pub db_type: String,
    pub database: Option<String>,
    pub executed_at: String,     // ISO 8601 UTC
    pub duration_ms: Option<i64>,
    pub row_count: Option<i64>,
    pub rows_affected: Option<i64>,
    pub status: String,
    pub error_msg: Option<String>,
    pub result_preview: Option<String>,  // JSON
}

impl HistoryManager {
    pub async fn open() -> Result<Self, sqlx::Error>;
    pub async fn record(&self, entry: &HistoryEntry) -> Result<i64, sqlx::Error>;
    pub async fn list(&self, limit: i64, offset: i64, search: Option<&str>) -> Result<Vec<HistoryEntry>, sqlx::Error>;
    pub async fn get(&self, id: i64) -> Result<Option<HistoryEntry>, sqlx::Error>;
    pub async fn delete(&self, id: i64) -> Result<(), sqlx::Error>;
    pub async fn clear(&self) -> Result<u64, sqlx::Error>;
    pub async fn count(&self) -> Result<i64, sqlx::Error>;
}
```

`HistoryManager::open()` resolves `dirs::config_dir()/sqlator/history.db`, creates the file and runs `CREATE TABLE IF NOT EXISTS` on first open.

### Tauri State (`src-tauri/src/state.rs`)

Add `history: HistoryManager` to `AppState`. Initialize in `tauri::Builder::setup` alongside `DbManager`.

### Tauri Commands (`src-tauri/src/commands.rs`)

New commands to register in `src-tauri/src/lib.rs`:

```rust
// src-tauri/src/commands.rs

#[tauri::command]
pub async fn get_history(
    state: State<'_, AppState>,
    limit: i64,
    offset: i64,
    search: Option<String>,
) -> Result<Vec<HistoryEntry>, String>;

#[tauri::command]
pub async fn get_history_count(
    state: State<'_, AppState>,
    search: Option<String>,
) -> Result<i64, String>;

#[tauri::command]
pub async fn delete_history_entry(
    state: State<'_, AppState>,
    id: i64,
) -> Result<(), String>;

#[tauri::command]
pub async fn clear_history(
    state: State<'_, AppState>,
) -> Result<(), String>;
```

**Recording history** is done inside the existing `execute_query` Tauri command (around line 355 of `commands.rs`) after the mpsc channel drains and the terminal event is processed — not as a separate command.

### Frontend Store (`src/lib/stores/history.svelte.ts`) — new file

```typescript
// src/lib/stores/history.svelte.ts
interface HistoryEntry { /* mirrors Rust struct */ }

class HistoryStore {
  entries = $state<HistoryEntry[]>([]);
  total   = $state(0);
  search  = $state('');
  page    = $state(0);
  pageSize = 50;

  async load(): Promise<void>;         // fetches current page
  async loadMore(): Promise<void>;     // increments page, appends
  async delete(id: number): Promise<void>;
  async clear(): Promise<void>;
  setSearch(q: string): void;          // debounced, resets page
}

export const history = new HistoryStore();
```

After `tabs.executeQuery` completes (in `tabs.svelte.ts` around line 362–416), call `history.prepend(newEntry)` to keep the panel live without a full reload.

### Frontend Components

#### `HistoryPanel.svelte` — new file

Layout:
- **Header bar**: search input (debounced 300ms), "Clear All" button (with confirmation), entry count
- **Entry list** (virtualized via `@tanstack/svelte-virtual`): each row shows timestamp, connection badge, db_type icon, sql snippet (first 120 chars), duration chip, row count badge, status indicator, "Re-run" button, "Delete" button
- **Detail drawer** (slide-out or expanded row): full SQL text (CodeMirror read-only), result preview table (first 100 rows), full metadata

#### Integration with QueryTab system

Add `history: HistoryState` as a new discriminant on the `QueryTab` union in `src/lib/types.ts`:

```typescript
type HistoryState = {
  search: string;
  selectedEntryId: number | null;
};
```

Expose via a **"History" button** in the connection toolbar (alongside Schema, New Query). Opening history uses `tabs.openHistory(connectionId)` following the `openTableBrowse`/`openSchemaDdl` pattern in `tabs.svelte.ts`.

Re-run from history:
1. Find or create a SQL editor `QueryTab` for this connection
2. Set its `sql` to the selected entry's `sql_text`
3. Auto-execute (`tabs.executeQuery(...)`) if user clicked "Re-run" vs just inserting the SQL

### TUI (`tui-app/src/app.rs` and `tui-app/src/ui.rs`)

**After merging the in-flight `AppMode::NewConnection` plan (`2026-04-13-001`):**

Add `AppMode::History` to the `AppMode` enum.

State fields on `App`:
```rust
history_entries: Vec<HistoryEntry>,
history_total: usize,
history_selected: usize,
history_search: TextArea<'static>,
history_search_active: bool,
history_loaded: bool,
```

Keyboard shortcuts (in `handle_history_key`):
- `Esc` → `AppMode::Workspace`
- `↑`/`↓` → navigate list
- `/` → focus search input
- `Enter` → re-run selected (populate editor + execute)
- `d` → delete selected entry
- `C` → clear all (with confirmation prompt)
- `?` → show help overlay

Access trigger: `Ctrl+H` in workspace mode opens history. The History panel is **global** (all connections) with a `conn_name` column visible.

`draw_history` function in `ui.rs`: full-screen layout:
```
┌─ Query History ───────────────────────────────────┐
│  [/] Search: _________________________  [50,000]  │
├──────────────────────────────────────────────────-┤
│  TIME        │ CONN       │ STATUS │ DUR │ ROWS   │
│  2026-04-13  │ prod-pg    │  ✓     │ 45ms│ 1,200  │
│  …                                                │
├───────────────────────────────────────────────────┤
│  SELECT * FROM users WHERE active = true LIMIT… ◄ │
└───────────────────────────────────────────────────┘
```

History is loaded asynchronously when `AppMode::History` is first entered (same pattern as schema loading).

---

## Implementation Phases

### Phase 1: Storage & Capture Backend

- [ ] Create `core/src/history.rs` with `HistoryManager` and `HistoryEntry`
- [ ] Add `history` to `core/src/lib.rs` module exports
- [ ] Add `history: HistoryManager` to `AppState` in `src-tauri/src/state.rs`
- [ ] Initialize `HistoryManager` in Tauri `setup` closure
- [ ] Capture and persist history in `execute_query` command (`src-tauri/src/commands.rs`)
- [ ] Add result preview serialization (first 100 rows → compact JSON)
- [ ] Register new Tauri commands in `src-tauri/src/lib.rs`

**Files:**
- `core/src/history.rs` ← new
- `core/src/lib.rs` ← add `pub mod history`
- `src-tauri/src/state.rs` ← add field
- `src-tauri/src/commands.rs` ← capture + 4 new commands
- `src-tauri/src/lib.rs` ← register commands

### Phase 2: Desktop UI Panel

- [ ] Add `HistoryEntry` TypeScript type to `src/lib/types.ts`
- [ ] Add `get_history`, `get_history_count`, `delete_history_entry`, `clear_history` to `src/lib/api/tauri-adapter.ts`
- [ ] Create `src/lib/stores/history.svelte.ts` with paginated load + search
- [ ] Wire `tabs.executeQuery` to prepend new entry to history store on completion
- [ ] Create `src/lib/components/HistoryPanel.svelte` (list + search + detail drawer)
- [ ] Add `history?: HistoryState` discriminant to `QueryTab` in `src/lib/types.ts`
- [ ] Add `openHistory` / `closeHistory` to `tabs.svelte.ts`
- [ ] Add History button to connection toolbar
- [ ] Re-run action: populate editor tab + optional auto-execute

**Files:**
- `src/lib/types.ts` ← add `HistoryEntry`, `HistoryState`
- `src/lib/api/tauri-adapter.ts` ← 4 new invoke calls
- `src/lib/stores/history.svelte.ts` ← new
- `src/lib/stores/tabs.svelte.ts` ← wire prepend
- `src/lib/components/HistoryPanel.svelte` ← new
- `src/lib/components/TabbedEditor.svelte` ← render history panel variant
- `src/lib/components/QueryTabBar.svelte` ← History button

### Phase 3: TUI History Mode

- [ ] Add `AppMode::History` to `AppMode` enum
- [ ] Add history state fields to `App` struct
- [ ] Implement `load_history()` async fn (calls `HistoryManager` directly — TUI shares `core`)
- [ ] Implement `handle_history_key()` in `app.rs`
- [ ] Implement `draw_history()` in `ui.rs`
- [ ] Capture history in TUI's `handle_query_event` when `Done`/`RowsAffected`/`Error` fires
- [ ] Add `Ctrl+H` binding in `AppMode::Workspace` key handler

**Files:**
- `tui-app/src/app.rs` ← `AppMode::History`, state fields, handlers, capture
- `tui-app/src/ui.rs` ← `draw_history()`

### Phase 4: Web Server Mode (if applicable)

- [ ] Expose `GET /api/history?search=&limit=&offset=`, `DELETE /api/history/:id`, `DELETE /api/history` HTTP endpoints in `web-server`
- [ ] Share `HistoryManager` from `core` (same pattern as DB pools)

---

## Design Decisions (SpecFlow Gap Resolution)

The following decisions resolve ambiguities surfaced during spec-flow analysis:

| Question | Decision |
|---|---|
| What counts as "result data"? | Store **first 100 rows as compact JSON** (`[[v1,v2,...], ...]`) + column names. Not the full result set — unbounded storage growth is unacceptable. Re-run fetches fresh data. |
| "Never throw away data" — auto-pruning vs. user delete? | No automatic TTL. Cap at 50,000 entries (oldest removed on overflow). Users **can** delete individual entries or clear all — "never throw away" means no silent expiry, not prohibition on user action. |
| Are error queries recorded? | **Yes.** `status='error'`, `error_msg` non-null, `duration_ms` null (error may fire before `Done`), `row_count` null. Failed queries are part of the audit trail. |
| History scope: global or per-connection? | **Global**, with a connection-name filter in the panel. Cross-connection history is more useful; scoped-per-connection history would hide queries when no connection is active. |
| Re-run semantics | "Re-run" button: populate editor tab with SQL **and execute immediately**. "Copy to editor" button: populate without executing. This gives users two clear affordances and avoids silent destructive execution. |
| Re-run against a closed connection | Show inline error in the history panel: "Connection '[name]' is not open. Connect first, then re-run." Do not auto-reconnect. |
| What SQL is stored when unified-grid CTE-wraps the query? | Store the **user's original SQL only** (before any CTE wrapping). The re-run path goes through the normal execute flow which re-applies CTE wrapping if applicable. |
| Connection name vs. ID in history record | Store **both**: `conn_id` (for re-run lookup) and `conn_name` + `db_type` + `database` (snapshot at execution time for display even if connection is later renamed or deleted). |
| SQLite journal mode | **WAL mode** enabled via `PRAGMA journal_mode=WAL` at pool init. Prevents `SQLITE_BUSY` under concurrent multi-connection workloads. |
| Web-server multi-user isolation | Phase 4 (web-server endpoints) deferred. If implemented, history is **per-process** (single `history.db` for the server process). No user identity isolation — web-server mode is single-user. |
| TUI search support | TUI history includes a search bar (reuse `tui-textarea` pattern from the editor). Press `/` to focus, `Esc` to blur. Results filter as-you-type. |
| Desktop panel location | History is a **special QueryTab variant** (following `tableBrowse` / `schemaDdl` pattern), not a sidebar drawer. Accessible via "History" button in the connection toolbar. One history tab per connection tab — filtered to all connections by default. |

---

## Alternative Approaches Considered

| Approach | Verdict |
|---|---|
| Store history in `connections.json` via `ConfigManager` | ❌ Full-file-rewrite on every INSERT is O(n) I/O. Bloats primary config. |
| Store history in an in-memory `Vec` only | ❌ Doesn't survive app restarts. Contradicts "never throw away data". |
| Use a separate JSON file | ❌ Same rewrite cost as `connections.json`. Hard to search/paginate. |
| SQLite with FTS5 full-text index | ⏸ Valuable for fuzzy search across large history. Deferred to v2 — SQLite `LIKE` is sufficient initially. |
| Store complete result rows (all rows) | ❌ Can be hundreds of MB for large queries. Store first 100 rows as preview + metadata only. |

---

## System-Wide Impact

### Interaction Graph

`tabs.executeQuery` → `tauriAdapter.executeQueryStream` → Tauri `execute_query` command → `DbManager::execute_query` → streams `QueryEvent` back → `execute_query` command assembles result → **NEW: `HistoryManager::record()`** → writes to `history.db`. Frontend receives `Done` event → `tabs.svelte.ts` updates `QueryTab.resultPane` → **NEW: `history.prepend(entry)`** updates panel live.

### Error & Failure Propagation

- If `HistoryManager::record()` fails (disk full, I/O error): log the error, do NOT fail the query result delivery to the user. History recording is best-effort.
- If `history.db` cannot be opened at startup: continue without history; show a one-time warning banner in the UI. Do not block the app from launching.
- If `execute_query` errors (`QueryEvent::Error`): still record the history entry with `status = 'error'` and `error_msg`. Failed queries are part of the audit trail.

### State Lifecycle Risks

- History entries are written before the frontend receives the final result. If the user closes the app mid-stream, a partial entry could remain unwritten — the `execute_query` command only records on terminal events, so partial executions (no `Done`/`Error` received) leave no orphaned rows.
- SQLite WAL mode recommended to allow concurrent reads during writes.

### API Surface Parity

- Tauri desktop: 4 new IPC commands
- TUI: direct in-process calls to `HistoryManager`
- Web server: 3 new HTTP endpoints (Phase 4)

### Integration Test Scenarios

1. **Execute SELECT → verify history entry**: Run `SELECT 1`, then `get_history(limit=1, offset=0)` → entry has correct `sql_text`, `status='ok'`, non-null `duration_ms`, `row_count=1`, `result_preview` containing `[[1]]`.
2. **Execute failing query → error recorded**: Run `SELECT * FROM nonexistent_table` → entry has `status='error'`, `error_msg` non-null, `duration_ms` non-null.
3. **Search filters correctly**: Insert 10 entries with varied SQL, search for "users" → only matching entries returned.
4. **Re-run populates editor**: Click re-run on a history entry → correct SQL appears in the active editor tab.
5. **Cap enforcement**: Insert 50,001 entries → `COUNT(*)` never exceeds 50,000; oldest entry is gone.

---

## Acceptance Criteria

### Functional Requirements

- [ ] Every executed SQL query (SELECT, DML, DDL, error) is persisted to `~/.config/sqlator/history.db` with: SQL text, connection name, db type, database name, timestamp (UTC), duration_ms, row count or rows_affected, status, error message
- [ ] History survives app restarts
- [ ] History is browsable in a dedicated panel in the desktop UI (accessible via "History" button in the connection toolbar)
- [ ] History is searchable by SQL text (case-insensitive substring match on `sql_text`)
- [ ] Each history entry shows: relative time ("2 minutes ago"), connection badge, status icon, duration chip, row count
- [ ] Clicking a history entry reveals the full SQL and result preview (up to 100 rows)
- [ ] One-click "Re-run" populates the SQL editor and executes the query
- [ ] Individual history entries can be deleted
- [ ] All history can be cleared via a "Clear All" action with a confirmation dialog
- [ ] History panel paginates (50 entries per page, infinite scroll / load more)
- [ ] TUI exposes history via `Ctrl+H` → `AppMode::History` with search, navigation, and re-run
- [ ] History recording does not block or delay query result delivery to the user
- [ ] If `history.db` cannot be opened, the app starts normally and shows a warning (not an error)

### Non-Functional Requirements

- [ ] Recording latency: `HistoryManager::record()` completes in < 10ms for typical queries (SQLite INSERT is fast)
- [ ] History list load: first 50 entries load in < 100ms from `history.db`
- [ ] Result preview: capped at 100 rows stored as compact JSON; total stored size per entry < 50KB for typical queries
- [ ] No data loss: history is flushed to disk before the query result is returned to the UI (SQLite transaction)
- [ ] SQLite WAL mode enabled for concurrent read performance

### Quality Gates

- [ ] Unit tests for `HistoryManager` (record, list, search, delete, clear, cap enforcement)
- [ ] No regressions in existing query execution flow
- [ ] Desktop UI tested: Chrome DevTools shows no layout issues on narrow/wide windows
- [ ] TUI history mode tested with keyboard navigation and re-run

---

## Success Metrics

- Users can find and re-run any previously executed query within 2 interactions
- History panel loads and is searchable without perceptible lag
- Zero data loss: every completed `QueryEvent::Done`/`RowsAffected`/`Error` event has a corresponding `history.db` row

---

## Dependencies & Prerequisites

- `sqlx` with `sqlite` feature already present in `core/Cargo.toml` (used by `core/src/db/sqlite.rs`)
- `dirs` crate already used by `ConfigManager` — same dep resolves history DB path
- `@tanstack/svelte-virtual` already used in results grid — reuse for history list virtualization
- **Coordinate with `2026-04-13-001-feat-tui-url-connection-add-plan.md`** — that plan adds `AppMode::NewConnection` and modifies `App` struct / `handle_key`. Rebase or apply `AppMode::History` on top to avoid conflicts in `app.rs:16–21` and `handle_key` dispatch.

---

## Risk Analysis & Mitigation

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| `history.db` grows unbounded | Low | Medium | Cap at 50,000 rows; auto-delete oldest on insert |
| SQLite write contention if queries fire rapidly | Low | Low | WAL mode; writes are sequential per-connection anyway |
| TUI plan merge conflict (`AppMode` enum) | High | Low | Coordinate with `2026-04-13-001`; apply after it merges |
| Result preview JSON bloat for wide tables | Medium | Medium | Cap at 100 rows × 50 columns; truncate values > 1KB |
| Startup failure if `history.db` path unwritable | Low | Low | Catch error, log, set `history_available = false`; never panic |

---

## Future Considerations

- **FTS5 full-text search** over `sql_text` for fuzzy/token search across large history
- **Starred / bookmarked queries** promoted out of history into a named collection
- **Per-connection history view** (filter by connection in the panel)
- **Export history** as CSV or SQL script
- **Statistics view**: most-run queries, slowest queries, error rate over time

---

## Sources & References

### Internal References

- Query execution pipeline: `core/src/db/mod.rs:94–176`, `src-tauri/src/commands.rs:334–362`
- `QueryEvent` enum: `core/src/models.rs:132–138`
- `ResultPaneState` type: `src/lib/types.ts:94`
- `tabs.executeQuery` hook point: `src/lib/stores/tabs.svelte.ts:325–416`
- `ConfigManager` pattern (and its limitations): `core/src/config.rs:13–65`
- `AppState` struct: `src-tauri/src/state.rs:10–57`
- `AppMode` enum and cycle_focus: `tui-app/src/app.rs:16–28`
- In-flight TUI URL connection plan (AppMode pattern to follow): `docs/plans/2026-04-13-001-feat-tui-url-connection-add-plan.md`
- Existing SQLite adapter (reuse as reference): `core/src/db/sqlite.rs`
- UI panel tab patterns: `src/lib/components/TabbedEditor.svelte:96–152`, `src/lib/components/QueryTabBar.svelte`
