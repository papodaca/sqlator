---
title: "feat: SQLator — Desktop SQL Client MVP"
type: feat
status: completed
date: 2026-04-04
---

# ✨ SQLator — Desktop SQL Client MVP

A minimal viable desktop SQL client built with **Tauri 2** (Rust backend) + **Svelte 5** + **Tailwind CSS v4**. Inspired by DataGrip, Beekeeper Studio, and DBngin — but lean and focused.

---

## Overview

SQLator lets developers save database connections and run SQL queries against them from a native desktop app. The MVP delivers three core capabilities:

1. **Connection Manager** — Save named, color-coded database connections by URL
2. **SQL Editor** — CodeMirror 6-powered editor with SQL syntax highlighting and Ctrl/Cmd+Enter to execute
3. **Result Grid** — Virtualized data grid showing query results with light/dark mode support

---

## Problem Statement

Existing SQL clients are either too heavyweight (DataGrip requires a JVM and subscription), browser-based (losing native performance and credential security), or too bare-bones. SQLator delivers a fast, native, privacy-respecting SQL tool that keeps credentials in the OS keychain and stays out of the way.

---

## Proposed Solution

A Cargo Workspace supporting both a Tauri 2 desktop app and a future Ratatui TUI:
- **Core Library (Rust):** `sqlx 0.8` with `AnyPool` for Postgres/MySQL/SQLite; `keyring 3.3` for OS keychain; custom file-based config manager for metadata (framework-agnostic)
- **Tauri App:** Thin wrapper around the Core Library exposing IPC commands
- **Svelte 5 frontend:** Runes-based reactive state; CodeMirror 6 SQL editor; TanStack Virtual result grid
- **Tailwind v4:** CSS-first theming with `@tailwindcss/vite`; `@custom-variant dark` for light/dark toggling

---

## Technical Approach

### Architecture (Cargo Workspace)

To support both a Desktop GUI and a future Terminal UI (TUI), the project is structured as a Cargo workspace, strictly decoupling business logic from Tauri.

```
┌─────────────────────────────────────────────────────────┐
│                   Svelte 5 Frontend                      │
│  ┌──────────────┐  ┌─────────────────┐  ┌────────────┐ │
│  │  Sidebar     │  │  SQL Editor     │  │  Result    │ │
│  │  Connection  │  │  (CodeMirror 6) │  │  Grid      │ │
│  │  List        │  │                 │  │  (Virtual) │ │
│  └──────┬───────┘  └────────┬────────┘  └─────┬──────┘ │
│         │                   │                  │        │
│         └──────────── invoke() (IPC) ──────────┘        │
└─────────────────────────────────────────────────────────┘
                              │
┌─────────────────────────────┼───────────────────────────┐
│           Tauri 2 App (Thin Wrapper)                     │
│  ┌──────────────────────────▼────────────────────────┐  │
│  │  Commands: connect_db, execute_query, test_conn   │  │
│  │  (Translates Core MPSC channels to Tauri Channels)│  │
│  └──────────────────────────┬────────────────────────┘  │
└─────────────────────────────┼───────────────────────────┘
                              │
┌─────────────────────────────┼───────────────────────────┐
│           Core Library (Pure Rust)                       │
│  ┌──────────────────────────▼────────────────────────┐  │
│  │  Core API (Connection Manager, Query Execution)   │  │
│  └──────────┬─────────────────────────┬──────────────┘  │
│             │                         │                 │
│  ┌──────────▼────────┐    ┌───────────▼─────────────┐  │
│  │  sqlx AnyPool     │    │  Config Manager (fs)    │  │
│  │  (Postgres/MySQL/ │    │  connections.json       │  │
│  │  SQLite)          │    │  (name, host, color)    │  │
│  └───────────────────┘    └─────────────────────────┘  │
│                                                          │
│  ┌─────────────────────────────────────────────────┐    │
│  │  keyring 3.3 (OS Keychain / DPAPI / libsecret)  │    │
│  │  passwords stored per connection ID             │    │
│  └─────────────────────────────────────────────────┘    │
└─────────────────────────────┬───────────────────────────┘
                              │ (Future)
┌─────────────────────────────▼───────────────────────────┐
│           TUI App (Ratatui)                              │
│  Terminal Interface consuming the exact same Core API    │
└──────────────────────────────────────────────────────────┘
```

### Credential Security Model

> **Critical:** `Core Config Manager` writes **plaintext JSON** to disk. Passwords MUST go to the OS keychain.

When a user saves a connection URL (e.g., `postgres://admin:secret@host:5432/mydb`):

1. Parse the URL in Rust — extract credentials, host, port, database
2. Store non-sensitive fields in Core Config Manager: `{ id, name, color, host, port, database, username, dbType }`
3. Store password in OS keychain via `keyring 3.3`: key = `("sqlator", <connection-id>)`
4. Display masked URL in UI: `postgres://admin:***@host:5432/mydb`
5. On connect: assemble full URL in Rust only — never pass plaintext password through IPC

### Connection State Machine

```
 [idle] ──click──► [connecting] ──success──► [connected]
                        │                         │
                      error                    query
                        │                      runs
                        ▼                         │
                  [error banner]            [executing]
                  + retry button                  │
                                          success/error
                                                  │
                                            [idle, results shown]
```

One active connection at a time. Pool created on selection, torn down on switch.

### IPC Command Surface

| Command | Direction | Purpose |
|---------|-----------|---------|
| `test_connection` | FE → Rust | Attempt connection with 5s timeout |
| `save_connection` | FE → Rust | Parse URL, persist metadata + keychain |
| `get_connections` | FE → Rust | Load saved connections list |
| `update_connection` | FE → Rust | Edit name/color/URL/credentials |
| `delete_connection` | FE → Rust | Remove metadata + keychain entry |
| `connect_database` | FE → Rust | Create AnyPool, store in DashMap state |
| `execute_query` | FE → Rust | Run SQL, return QueryResult via Channel |
| `cancel_query` | FE → Rust | Abort in-flight query |

### Query Execution Flow

For non-blocking large result sets, the Core library yields results through a generic `tokio::sync::mpsc` channel. 
The Tauri app wrapper consumes this channel and forwards it to the frontend using `tauri::ipc::Channel` for streaming:

```rust
// Core (Pure Rust)
pub async fn execute_query(
    pool: &AnyPool,
    sql: &str,
    sender: tokio::sync::mpsc::Sender<QueryEvent>,
) -> Result<(), CoreError> { ... }

// Tauri Wrapper
#[tauri::command]
async fn execute_query(
    state: State<'_, AppState>,
    connection_id: String,
    sql: String,
    on_event: Channel<QueryEvent>,
) -> Result<(), CommandError> {
    // Bridges the Core MPSC to the Tauri IPC Channel
}
```

Frontend batches row events at 50ms intervals to avoid thousands of reactive updates (use `$state.raw` + interval flush).

Result types:
- **SELECT**: columns + rows array → data grid
- **INSERT/UPDATE/DELETE/DDL**: rows_affected count → "Query OK, N rows affected (Xms)" message

---

## Implementation Phases

### Phase 1: Project Scaffold & Foundation

**Goal:** Running Tauri 2 + Svelte 5 + Tailwind v4 app with basic window.

**Tasks:**
- [ ] Scaffold with `npm create tauri-app@latest` (Svelte + TypeScript template)
- [ ] Add `@tailwindcss/vite` plugin (before svelte plugin in `vite.config.ts`)
- [ ] Set up `src/app.css` with `@import "tailwindcss"` + `@theme {}` tokens
- [ ] Configure `tauri.conf.json`: `productName: "SQLator"`, min size 900×600, center on launch
- [ ] Implement pure-Rust file-based configuration manager in `core/` (replacing Core Config Manager)
- [ ] Set up Cargo workspace (`core`, `tauri-app`, `tui-app`)
- [ ] Add `keyring 3.3` to `Cargo.toml`
- [ ] Add `sqlx 0.8` with features: `runtime-tokio, tls-rustls, postgres, mysql, sqlite, any, json`
- [ ] Add `dashmap 6` and `futures 0.3` to `Cargo.toml`
- [ ] Set up `src-tauri/capabilities/main.json` with store permissions
- [ ] Add dark mode support: `@custom-variant dark (&:where(.dark, .dark *))` in CSS
- [ ] Set up light/dark toggle that persists to Core Config Manager
- [ ] Create basic layout: left sidebar (240px) + main content area

**Success criteria:** App opens, shows a two-panel layout, dark mode toggle works.

**Key files:**
- `src-tauri/Cargo.toml`
- `src-tauri/src/lib.rs`
- `src-tauri/tauri.conf.json`
- `src-tauri/capabilities/main.json`
- `vite.config.ts`
- `src/app.css`
- `src/lib/stores/theme.svelte.ts`

---

### Phase 2: Connection Manager

**Goal:** Users can add, test, edit, delete, and select connections.

**Rust commands to implement:**
- `test_connection(url: String) -> Result<String, CommandError>` — 5-second timeout, returns `"Connected"` or error detail
- `save_connection(config: ConnectionConfig) -> Result<String, CommandError>` — parses URL, extracts credentials, stores in keychain + plugin-store, returns generated UUID
- `get_connections() -> Result<Vec<SavedConnection>, CommandError>` — loads from Core Config
- `update_connection(id: String, config: ConnectionConfig) -> Result<(), CommandError>`
- `delete_connection(id: String) -> Result<(), CommandError>` — removes from store + keychain

**Frontend components:**
- `src/lib/components/Sidebar.svelte` — connection list with color dots
- `src/lib/components/ConnectionForm.svelte` — add/edit dialog with:
  - Name input
  - Connection URL input (password masked after entry)
  - Color picker (10 canned colors)
  - "Test Connection" button → inline success/error
  - "Save" button (enabled even if test not run)
- `src/lib/components/ConnectionItem.svelte` — list item with right-click context menu (Edit, Delete)
- `src/lib/stores/connections.svelte.ts` — `$state` store for connection list + active connection

**10 canned colors:**

```typescript
// src/lib/constants/colors.ts
export const CONNECTION_COLORS = [
  { id: 'red',    hex: '#ef4444', label: 'Red' },
  { id: 'orange', hex: '#f97316', label: 'Orange' },
  { id: 'yellow', hex: '#eab308', label: 'Yellow' },
  { id: 'green',  hex: '#22c55e', label: 'Green' },
  { id: 'teal',   hex: '#14b8a6', label: 'Teal' },
  { id: 'blue',   hex: '#3b82f6', label: 'Blue' },
  { id: 'violet', hex: '#8b5cf6', label: 'Violet' },
  { id: 'pink',   hex: '#ec4899', label: 'Pink' },
  { id: 'slate',  hex: '#64748b', label: 'Slate' },
  { id: 'white',  hex: '#f8fafc', label: 'White' },
] as const;
```

**Data types:**

```typescript
// src/lib/types.ts
export interface SavedConnection {
  id: string;           // UUID
  name: string;
  colorId: ConnectionColorId;
  dbType: 'postgres' | 'mysql' | 'sqlite';
  host: string;
  port: number;
  database: string;
  username: string;
  maskedUrl: string;    // e.g. postgres://admin:***@host/db
}

export interface ConnectionConfig {
  name: string;
  colorId: ConnectionColorId;
  url: string;          // raw URL including password, only used for test/save
}
```

**Rust state:**

```rust
// src-tauri/src/state.rs
use dashmap::DashMap;
use sqlx::AnyPool;

pub struct AppState {
    pub pools: DashMap<String, AnyPool>,
    pub active_connection: tokio::sync::Mutex<Option<String>>,
}
```

**Success criteria:**
- [ ] Can add a connection with URL, name, and color
- [ ] "Test Connection" shows inline success or failure within 5s
- [ ] Saved connections appear in sidebar with correct color dot
- [ ] Right-click opens Edit/Delete menu
- [ ] Delete requires confirmation; removes from store and keychain
- [ ] Passwords never appear in plaintext in the UI or `connections.json`

---

### Phase 3: SQL Editor

**Goal:** CodeMirror 6 SQL editor with dialect-aware syntax highlighting and keyboard shortcut execution.

**Frontend components:**
- `src/lib/components/SqlEditor.svelte` — wraps CodeMirror 6
- `src/lib/components/EditorToolbar.svelte` — "Run" button, connection status badge, execution time

**CodeMirror 6 setup:**

```bash
pnpm add codemirror @codemirror/lang-sql @codemirror/theme-one-dark @codemirror/view @codemirror/state
```

Key extensions:
- `sql({ dialect: dialectMap[activeConnection.dbType] })` — PostgreSQL/MySQL/SQLite dialects
- `Prec.highest(keymap.of([{ key: 'Mod-Enter', run: executeQuery }]))` — Ctrl/Cmd+Enter
- `EditorView.lineWrapping`
- `oneDark` / custom light theme matching app theme

**Editor state persistence:**
- Last query text per connection persisted to Core Config Manager under key `query:<connection-id>`
- Restored when connection is (re-)selected

**Empty editor behavior:**
- Ctrl+Enter with empty editor is a no-op — no error shown

**Success criteria:**
- [ ] SQL syntax highlighted correctly for active connection dialect
- [ ] Ctrl+Enter (Windows/Linux) and Cmd+Enter (macOS) trigger execution
- [ ] Last query restored on connection switch
- [ ] Editor is disabled/grayed when no connection is selected

---

### Phase 4: Query Execution & Result Grid

**Goal:** Execute queries and show results in a virtualized grid.

**Rust command:**

```rust
#[derive(Serialize, Clone)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
enum QueryEvent {
    Columns { names: Vec<String> },
    Row { values: Vec<serde_json::Value> },
    Done { row_count: usize, duration_ms: u64 },
    RowsAffected { count: u64, duration_ms: u64 },
    Error { message: String, position: Option<u32> },
}
```

**Result display components:**
- `src/lib/components/ResultPane.svelte` — container; switches between states
- `src/lib/components/ResultGrid.svelte` — TanStack Virtual table; max 1000 rows
- `src/lib/components/ExecutionMessage.svelte` — "Query OK, N rows affected" for DML/DDL
- `src/lib/components/ErrorDisplay.svelte` — red error box with message + optional position
- `src/lib/components/LoadingState.svelte` — spinner + "Cancel" button

**Result pane states:**
| State | Shown When |
|-------|-----------|
| Idle (empty) | No query run yet |
| Loading | Query in flight |
| Result Grid | SELECT returned rows |
| Empty Set | SELECT returned 0 rows |
| Rows Affected | DML/DDL succeeded |
| Error | DB error or connection error |

**Virtual grid setup:**

```bash
pnpm add @tanstack/svelte-virtual
```

- `createVirtualizer` (not deprecated `useVirtualizer`) for Svelte 5
- Row height estimate: 36px
- Overscan: 10 rows
- Max rows rendered: 1000 (warn with notice above grid)
- NULL display: muted italic `NULL` text, distinct from empty string

**Row batching for streaming:**

```typescript
// Buffer rows and flush every 50ms to avoid thousands of reactive updates
let buffer: Row[] = [];
const flushInterval = setInterval(() => {
  if (buffer.length > 0) {
    streamedRows = [...streamedRows, ...buffer];
    buffer = [];
  }
}, 50);
```

**Cancel query:**

```rust
#[tauri::command]
async fn cancel_query(
    state: State<'_, AppState>,
    connection_id: String,
) -> Result<(), CommandError> {
    // Signal cancellation via tokio CancellationToken stored per connection
}
```

**Success criteria:**
- [ ] SELECT results appear in scrollable grid with column headers
- [ ] Spinner and Cancel button shown during execution
- [ ] Cancel aborts the query without crashing the app
- [ ] INSERT/UPDATE/DELETE shows "Query OK, N rows affected (Xms)"
- [ ] DB errors shown in red inline error box with message
- [ ] NULL cells show `NULL` in muted italic
- [ ] "Showing first 1,000 rows" notice when result set exceeds limit
- [ ] Execution time shown in toolbar after completion

---

### Phase 5: Polish & Light/Dark Mode

**Goal:** Solid UX, theme consistency, keyboard shortcuts, and empty states.

**Tasks:**
- [ ] Light/dark mode: follows OS preference on launch, manual toggle persisted
- [ ] Empty state for connection list: "No connections yet — add one to get started"
- [ ] Empty state for editor: hint text "Select a connection to start querying"
- [ ] Connection status badge in editor toolbar (connected / disconnected / error)
- [ ] Keyboard shortcuts documented in-app (? key or Help menu)
- [ ] Error recovery: "Retry" button on connection error banner
- [ ] App title bar updates to `SQLator — <connection name>` when connected
- [ ] Minimum window size enforced (900×600 in `tauri.conf.json`)
- [ ] All interactive elements keyboard-accessible (Tab navigation)
- [ ] Result grid column widths: auto-fit to content, minimum 80px

**Theme tokens (app.css):**

```css
@import "tailwindcss";

@custom-variant dark (&:where(.dark, .dark *));

@theme {
  --font-mono: 'JetBrains Mono', 'Fira Code', ui-monospace, monospace;
  --font-sans: 'Inter', system-ui, sans-serif;
}

:root {
  color-scheme: light;
  --color-bg:          oklch(0.97 0 0);
  --color-surface:     oklch(0.99 0 0);
  --color-surface-2:   oklch(0.94 0 0);
  --color-border:      oklch(0.87 0 0);
  --color-text:        oklch(0.15 0 0);
  --color-text-muted:  oklch(0.50 0 0);
  --color-accent:      oklch(0.55 0.20 263);
}

.dark {
  color-scheme: dark;
  --color-bg:          oklch(0.13 0 0);
  --color-surface:     oklch(0.17 0 0);
  --color-surface-2:   oklch(0.21 0 0);
  --color-border:      oklch(0.28 0 0);
  --color-text:        oklch(0.92 0 0);
  --color-text-muted:  oklch(0.58 0 0);
  --color-accent:      oklch(0.65 0.18 263);
}
```

---

## Alternative Approaches Considered

### Monaco Editor vs CodeMirror 6

**Monaco** was considered (it's what VS Code uses; excellent SQL support) but rejected for Tauri because:
- Monaco uses Web Workers for language services
- Tauri 2's CSP and WebView restrict `blob:` and `data:` URIs required by Monaco workers
- Requires complex CSP overrides that are fragile across platforms
- Bundle size: 5-10MB vs CodeMirror's ~450KB

**Decision: CodeMirror 6** — no worker dependency, modular, first-class Tauri compatibility.

### sqlx vs sea-orm vs diesel

- **Diesel:** compile-time only, no async, no multi-DB runtime switching — wrong for a client app
- **SeaORM:** ORM abstractions not needed when executing arbitrary user SQL
- **sqlx 0.8 with AnyPool:** runtime multi-DB, async-first, best-in-class — chosen

### `Mutex<Pool>` vs `DashMap<Pool>`

`sqlx::Pool` is already thread-safe and manages its own internal connection pool. Wrapping it in `Mutex` is an anti-pattern (locks the entire map for each query). Use `DashMap<String, AnyPool>` to allow concurrent queries across different connections.

### tauri-plugin-stronghold vs keyring

- **Stronghold:** Encrypted vault on disk, requires user-set master password, significant UX complexity
- **keyring 3.3:** OS-native keychain (Keychain on macOS, DPAPI on Windows, libsecret on Linux) — seamless UX, no extra password required

**Decision: keyring 3.3** — simpler, native OS integration, no additional user friction.

---

## System-Wide Impact

### Interaction Graph

1. User saves connection → `save_connection` command → URL parsed in Rust → non-sensitive fields → `Core Config Manager` → password → `keyring` → UUID returned to frontend → connection list updates via `$state`

2. User clicks connection → `connect_database` command → full URL assembled in Rust (keyring fetch) → `AnyPool::connect()` → pool stored in `DashMap` under connection ID → frontend reactive state updated to `connected`

3. User presses Ctrl+Enter → `execute_query` command → pool fetched from `DashMap` → sqlx `query().fetch()` stream → `Channel<QueryEvent>` → frontend receives events → batch flush every 50ms → `$state.raw` array updated → TanStack Virtual re-renders visible rows

4. User clicks Cancel → `cancel_query` command → `CancellationToken::cancel()` → sqlx stream aborted → `Done` event with partial row count sent → frontend resets loading state

### Error Propagation

| Error Type | Origin | Handling |
|-----------|--------|---------|
| Connection refused | `sqlx::Error::Io` | `CommandError { code: "CONNECTION_FAILED" }` → red banner in editor |
| Auth failure | `sqlx::Error::Database` | Same as above, error message shown |
| SQL syntax error | `sqlx::Error::Database` | `QueryEvent::Error { message, position }` → inline red box below editor |
| Query timeout | `tokio::time::timeout` expired | `CommandError { code: "TIMEOUT" }` → timeout message + retry |
| IPC payload limit | Tauri internal | Not a risk — using `Channel` streaming, not buffering |
| Keychain unavailable | `keyring::Error` | `CommandError { code: "KEYCHAIN_ERROR" }` → prompt user |

### State Lifecycle Risks

- **Orphaned pools:** If the app crashes while a pool is in `DashMap`, the pool is dropped by the OS. No cleanup needed.
- **Partial saves:** If `save_connection` succeeds writing to plugin-store but keyring write fails, the connection is saved without credentials. On next connect, `keyring::get_password` fails with `CommandError` → user sees "Credentials missing, please re-edit connection." Connection should be marked with a warning icon.
- **Concurrent edit/delete:** If user edits a connection while a query is running on it, disable Edit/Delete buttons when `status === 'executing'`.

### Integration Test Scenarios

1. **Happy path — Postgres SELECT:** Add connection → test → save → click → type `SELECT 1` → Ctrl+Enter → grid shows `1` in column `?column?`
2. **Auth failure:** Save connection with wrong password → click → error banner shown → fix credentials via Edit → retry → connects
3. **Long query cancel:** Execute `SELECT pg_sleep(60)` → Cancel button → query aborts within 1s → result pane shows "Query cancelled"
4. **App restart persistence:** Add 3 connections → close app → reopen → all 3 connections present in sidebar
5. **Credential security:** Save connection with password → inspect `connections.json` on disk → password NOT present in file

---

## Acceptance Criteria

### Functional Requirements

- [ ] **AC-01** User can add a named, color-coded database connection by entering a connection URL
- [ ] **AC-02** A "Test Connection" button attempts connection with 5s timeout, showing inline pass/fail
- [ ] **AC-03** Passwords from connection URLs are stored in the OS keychain, never in plaintext on disk
- [ ] **AC-04** Saved connections display with masked URL (`postgres://user:***@host/db`)
- [ ] **AC-05** Connections are restored from disk on app restart
- [ ] **AC-06** Right-click on a connection opens Edit and Delete options; Delete requires confirmation
- [ ] **AC-07** Clicking a connection opens a SQL editor with CodeMirror 6 syntax highlighting
- [ ] **AC-08** Ctrl+Enter (Win/Linux) and Cmd+Enter (macOS) execute the current SQL
- [ ] **AC-09** SELECT results display in a scrollable, virtualized data grid with column headers
- [ ] **AC-10** INSERT/UPDATE/DELETE/DDL shows "Query OK, N rows affected (Xms)" instead of a grid
- [ ] **AC-11** A loading spinner and Cancel button appear while a query is executing
- [ ] **AC-12** Cancel aborts the in-flight query
- [ ] **AC-13** DB errors (syntax errors, etc.) show in a red inline box below the editor with the error message
- [ ] **AC-14** Connection errors show a red banner in the editor area with a Retry button
- [ ] **AC-15** NULL cells display as muted italic `NULL`, visually distinct from empty string
- [ ] **AC-16** Result sets exceeding 1,000 rows show only the first 1,000 with a notice
- [ ] **AC-17** Last query text per connection is persisted and restored on connection re-selection
- [ ] **AC-18** Ctrl+Enter with empty editor is a no-op (no error shown)
- [ ] **AC-19** Light/dark mode follows OS preference on launch; a toggle persists manual override
- [ ] **AC-20** App enforces minimum window size of 900×600

### Non-Functional Requirements

- [ ] First query result renders within 200ms for results under 100 rows on localhost DB
- [ ] 1,000-row result grid scrolls at 60fps (virtual rendering)
- [ ] Connection list loads from disk in under 100ms on app startup
- [ ] App binary size under 50MB for release builds

### Quality Gates

- [ ] No plaintext passwords in `connections.json` (automated check in CI)
- [ ] Keyboard-only navigation works for all primary flows
- [ ] Dark mode tested on all three platforms (macOS, Windows, Linux)

---

## Dependencies & Prerequisites

### Rust (Cargo.toml)

```toml
[dependencies]
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

[build-dependencies]
tauri-build = { version = "2", features = [] }
```

### Frontend (package.json)

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
    "@tanstack/svelte-virtual": "^3.13",
    "typescript": "^5"
  }
}
```

### Prerequisites

- Rust 1.77.2+
- Node.js 20+ / pnpm 9+
- Tauri CLI v2: `cargo install tauri-cli --version "^2"`
- Platform build deps: Xcode (macOS), Visual Studio Build Tools (Windows), `libwebkit2gtk-4.1-dev` (Linux)

---

## Risk Analysis & Mitigation

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| `sqlx` `AnyPool` driver conflicts at compile time | Medium | High | Use `any` feature with all drivers; call `install_default_drivers()` before first connect |
| `keyring` OS keychain unavailable (headless Linux) | Low | High | Graceful error: prompt user to enter password each session; log warning |
| TanStack Virtual Svelte 5 compatibility regression | Low | Medium | Pin to `@tanstack/svelte-virtual@3.13.x`; test early in Phase 4 |
| Tauri IPC 1MB payload limit hit for large results | Medium | High | Use `Channel` streaming from day one (Phase 4 design); never buffer full result in Rust |
| `Core Config Manager` `autoSave` race condition | Low | Low | Use `LazyStore` pattern; call `.save()` explicitly after mutations |
| CSP blocking CodeMirror assets | Low | Medium | Set `"csp": null` in `tauri.conf.json` for development; tighten for production |
| Multi-statement SQL behavior varies by DB driver | High | Medium | Execute statement at cursor position only; detect and warn on semicolons |

---

## Future Considerations (v2+)

- **Multiple tabs** — multiple open connections with tab bar
- **Query history** — persisted per connection with timestamps
- **Export** — CSV, JSON, TSV export of result sets
- **Schema browser** — left panel tree of databases/tables/columns
- **SSH tunnel support** — connection via SSH jump host
- **SSL/TLS certificate config** — upload PEM certs per connection
- **Auto-complete** — schema-aware CodeMirror completions from fetched table/column metadata
- **Query formatter** — SQL pretty-printer (e.g., `sql-formatter` npm package)
- **Keyboard shortcut config** — user-definable key bindings

---

## File Structure

```
sqlator/
├── core/                   # Pure Rust core logic
│   ├── Cargo.toml
│   └── src/
│       ├── config.rs           # File-based config manager
│       ├── db.rs               # sqlx AnyPool management
│       ├── models.rs           # Serde types
│       └── error.rs            # CoreError enum
├── tauri-app/              # Desktop GUI
│   ├── src-tauri/
│   │   ├── Cargo.toml
│   │   ├── tauri.conf.json
│   │   └── src/
│   │       ├── lib.rs          # Tauri builder, bridging Core
│   │       ├── state.rs        # Tauri wrapper for Core state
│   │       └── commands/       # Tauri command wrappers
│   └── src/                    # Svelte frontend
├── tui-app/                # Terminal UI
│   ├── Cargo.toml
│   └── src/
│       └── main.rs             # Ratatui TUI entry point
├── package.json
│   ├── app.css                 # @import "tailwindcss"; @theme {}; dark tokens
│   ├── main.ts                 # mount App
│   ├── App.svelte              # root layout: sidebar + main
│   ├── lib/
│   │   ├── constants/
│   │   │   └── colors.ts       # CONNECTION_COLORS array
│   │   ├── types.ts            # SavedConnection, ConnectionConfig, QueryResult
│   │   ├── stores/
│   │   │   ├── connections.svelte.ts   # $state: connections[], activeConnectionId
│   │   │   ├── query.svelte.ts         # $state: result, isExecuting, streamedRows
│   │   │   └── theme.svelte.ts         # $state: isDark + OS watcher
│   │   └── components/
│   │       ├── Sidebar.svelte
│   │       ├── ConnectionItem.svelte
│   │       ├── ConnectionForm.svelte   # add/edit dialog
│   │       ├── ColorPicker.svelte
│   │       ├── SqlEditor.svelte        # CodeMirror 6 wrapper
│   │       ├── EditorToolbar.svelte    # run button, status badge, exec time
│   │       ├── ResultPane.svelte       # state router
│   │       ├── ResultGrid.svelte       # TanStack Virtual table
│   │       ├── ExecutionMessage.svelte # "Query OK, N rows affected"
│   │       ├── ErrorDisplay.svelte     # inline SQL error
│   │       └── ThemeToggle.svelte
│   └── vite-env.d.ts
├── vite.config.ts
├── package.json
└── tsconfig.json
```

---

## Sources & References

### External References

- [Tauri 2 Documentation](https://v2.tauri.app/) — IPC, capabilities, plugin system
- [Tauri 2 `tauri::ipc::Channel`](https://v2.tauri.app/develop/calling-rust/#channels) — streaming pattern
- [sqlx 0.8 AnyPool](https://docs.rs/sqlx/latest/sqlx/pool/struct.Pool.html) — multi-driver runtime pool
- [keyring 3.3](https://docs.rs/keyring/latest/keyring/) — OS keychain integration
- [CodeMirror 6 SQL language](https://codemirror.net/docs/ref/#lang-sql) — `@codemirror/lang-sql`
- [TanStack Virtual v3](https://tanstack.com/virtual/latest) — `createVirtualizer` for Svelte 5
- [Tailwind CSS v4 Vite setup](https://tailwindcss.com/docs/installation/vite) — `@tailwindcss/vite` plugin
- [Core Config Manager v2](https://v2.tauri.app/plugin/store/) — `LazyStore`, capabilities

### Key Gotchas (from research)

1. `sqlx::any::install_default_drivers()` MUST be called before any `AnyPool::connect()` — panics silently otherwise
2. `@tailwindcss/vite` MUST be listed before `svelte()` in `vite.config.ts` plugins array
3. `Core Config Manager` is **plaintext JSON** — never store passwords there
4. Use `tokio::sync::Mutex` (not `std::sync::Mutex`) for state accessed across `.await` points
5. `$state.raw` + 50ms batch flush for large streaming result sets — avoid deep proxy overhead
6. TanStack Virtual requires `createVirtualizer` in Svelte 5 (not `useVirtualizer`)
7. Set `"csp": null` in `tauri.conf.json` during development to avoid asset-blocking CSP issues
8. `dashmap` eliminates the `Mutex<HashMap>` anti-pattern — `Pool` is already thread-safe
