---
title: "feat: Terminal UI (TUI) MVP"
type: feat
status: active
date: 2026-04-04
origin: docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md
---

# ✨ Terminal UI (TUI) MVP

## Overview

A lightweight, purely terminal-based SQL client that acts as a first-class citizen alongside the Tauri desktop application. The TUI leverages the project's decoupled `core/` Rust library (providing identical connection management, secure credential handling, and database execution logic) but replaces the Svelte GUI with a keyboard-driven, fast-booting `ratatui` interface.

**Scope Note:** This is an MVP. It **explicitly excludes** the editable datagrid (Plan 004) and the VS Code-style tabbed interface (Plan 003) to ensure a focused, rapid delivery of core SQL querying capabilities in the terminal.

## Problem Statement

While the Tauri desktop application is highly performant and secure, developers and system administrators often work extensively within terminal multiplexers (tmux, Zellijg) or over remote SSH sessions. Firing up a full GUI client to run a quick `SELECT` query or inspect a table schema interrupts the terminal workflow. A native, keyboard-centric TUI built on the same robust Rust core satisfies this workflow without requiring dual-maintenance of business logic.

## Proposed Solution

Introduce a new Cargo workspace member (`tui-app/`) built with `ratatui` and `crossterm`. 

**Key Capabilities:**
1. **Connection Manager:** List and select connections saved by the GUI (reading from the `core/` file-based config manager).
2. **Schema Browser:** A simple tree-view in the left pane showing databases, schemas, tables, and columns.
3. **Query Editor:** A text area for writing raw SQL queries.
4. **Result Grid:** A terminal-based table widget to stream and display query results.

## Technical Approach

### Architecture

The TUI directly consumes the `core/` library API, bypassing any Tauri-specific IPC or plugins.

```text
┌─────────────────────────────────────────────────────────┐
│                   TUI App (Ratatui)                     │
│  ┌──────────────┐  ┌─────────────────┐  ┌────────────┐  │
│  │  Left Pane   │  │  Top Pane       │  │  Bottom    │  │
│  │  Connections │  │  SQL Editor     │  │  Pane      │  │
│  │  & Schema    │  │  (tui-textarea) │  │  Results   │  │
│  └──────┬───────┘  └────────┬────────┘  └─────┬──────┘  │
│         │                   │                 │         │
│         └──────────── Core Library API ───────┘         │
└─────────────────────────────────────────────────────────┘
                              │
┌─────────────────────────────▼───────────────────────────┐
│           Core Library (Pure Rust)                       │
│  ┌───────────────────────────────────────────────────┐  │
│  │  Core API (Config, DB Pools, Query Execution)     │  │
│  └──────────┬─────────────────────────┬──────────────┘  │
│             │                         │                 │
│  ┌──────────▼────────┐    ┌───────────▼─────────────┐  │
│  │  sqlx AnyPool     │    │  Config Manager (fs)    │  │
│  │  (Postgres/MySQL/ │    │  connections.json       │  │
│  │  SQLite)          │    │  (name, host, color)    │  │
│  └───────────────────┘    └─────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

### UI Layout

The TUI will use a standard 3-pane layout, which changes context based on the application state:

**State 1: Connection Selection**
* Full-screen list of saved connections (read from `core::config`).
* Pressing `Enter` connects to the database via `core::db::connect()`.

**State 2: Connected Workspace**
* **Left Sidebar (20-30% width):** Schema Browser (Tables & Columns).
* **Right Top (40% height):** SQL Editor (`tui-textarea` crate).
* **Right Bottom (60% height):** Query Result Grid (`ratatui::widgets::Table`).
* **Footer:** Keyboard shortcuts reference (e.g., `Ctrl+E` to execute, `Tab` to switch panes).

## Implementation Phases

### Phase 1: TUI Scaffold & Core Integration
* **Tasks:**
  - Create `tui-app/` workspace.
  - Set up `ratatui` with the `crossterm` backend.
  - Implement the main event loop (drawing frames, handling async ticks).
  - Link the `core/` crate and verify ability to read the file-based connection config.
* **Success Criteria:** TUI boots, draws a basic layout, and successfully parses `connections.json` using the core logic.

### Phase 2: Connection Manager View
* **Tasks:**
  - Build a `List` widget showing saved connections.
  - Implement Up/Down arrow navigation.
  - Implement `Enter` to initiate a database connection using the `core` library.
  - Show a loading spinner/throbber while connecting.
  - Securely fetch passwords from the OS keyring (via `core::security`).
* **Success Criteria:** User can select a connection, authenticate seamlessly via the keyring, and transition to the Connected Workspace state.

### Phase 3: Schema Browser
* **Tasks:**
  - Query table and column metadata using `core::db::schema` functions (implemented in Plan 005).
  - Render a collapsible tree structure using `tui-tree-widget` (or a custom nested list).
* **Success Criteria:** The left pane displays tables and their respective columns after successfully connecting.

### Phase 4: SQL Editor
* **Tasks:**
  - Integrate `tui-textarea` for multi-line SQL input.
  - Support basic editing (typing, backspace, cursor movement).
  - Bind `Ctrl+E` (or `Ctrl+Enter` depending on terminal capability) to trigger query execution.
* **Success Criteria:** User can type a raw SQL query and dispatch it to the core execution engine.

### Phase 5: Query Execution & Result Grid
* **Tasks:**
  - Listen to the `tokio::sync::mpsc` channel exposed by `core::db::execute_query`.
  - Buffer streamed `QueryEvent` rows (similar to the Svelte frontend, but stored in TUI application state).
  - Render a horizontally scrollable `Table` widget.
  - Enforce a 1000-row limit in memory to prevent terminal lag/OOM issues.
  - Show status messages for DDL/DML operations (e.g., "Query OK, 5 rows affected").
* **Success Criteria:** Executing a `SELECT` displays rows in the bottom pane. Executing an `UPDATE` shows the affected row count.

## Acceptance Criteria

### Functional Requirements
- [ ] User can launch the TUI executable (`sqlator-tui` or `cargo run -p tui-app`).
- [ ] User can view and navigate a list of saved connections.
- [ ] User can establish a connection using stored keyring credentials without re-entering them.
- [ ] Connected workspace shows a 3-pane layout: Schema (left), Editor (top right), Results (bottom right).
- [ ] User can navigate between panes using `Tab` and `Shift+Tab`.
- [ ] User can type a SQL query in the editor and execute it using a keyboard shortcut (`Ctrl+E`).
- [ ] Query results are streamed from the `core` channel and rendered in a `ratatui` Table.
- [ ] Schema pane displays tables and columns fetched from the database.
- [ ] Application can be exited cleanly using `Ctrl+C` or `q` (when not in editor mode).

### Exclusions (Explicitly out of scope for MVP)
- [ ] No inline cell editing in the result grid.
- [ ] No multi-level tabbed interfaces (a single active connection and query context at a time).
- [ ] No advanced syntax highlighting in the SQL editor (plain text or basic keyword highlighting only).

## Dependencies & Risks

### TUI Ecosystem Crates
```toml
# tui-app/Cargo.toml
[dependencies]
core = { path = "../core" }
ratatui = "0.26"
crossterm = "0.27"
tokio = { version = "1", features = ["full"] }
tui-textarea = "0.4"    # For the SQL editor
tui-tree-widget = "0.2" # For the schema browser
```

### Risk Mitigation
| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| Terminal Event Blocking | High | High | Run the TUI draw loop and the Crossterm event listener on a separate thread/task from the `core` async database operations. |
| Streaming Large Results | Medium | High | Rely on the `mpsc` channel chunking. Cap the TUI table model at 1000 rows to prevent rendering massive tables that tank FPS. |
| Key Binding Conflicts | High | Low | Terminal emulators often trap keys (e.g., `Ctrl+W`, `Ctrl+T`). Stick to universally safe bindings (`Ctrl+E` for execute, `Tab` for focus). |

## Sources & References

- **Origin document:** [docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md](docs/plans/2026-04-04-001-feat-sql-client-desktop-mvp-plan.md)
  - Decision carried forward: *Architecture decoupled into `core/`, `tauri-app/`, and `tui-app/` workspaces to share pure Rust logic.*
  - Decision carried forward: *Core library yields results through a generic `tokio::sync::mpsc` channel for non-blocking large result sets.*
- **Ratatui Docs:** [https://ratatui.rs/](https://ratatui.rs/)
- **tui-textarea Docs:** [https://github.com/rhysd/tui-textarea](https://github.com/rhysd/tui-textarea)
