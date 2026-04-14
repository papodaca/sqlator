---
title: "feat: Embedded DB CLI Terminal Panel (slide-in from bottom)"
type: feat
status: active
date: 2026-04-14
---

# feat: Embedded DB CLI Terminal Panel (slide-in from bottom)

## Overview

Add a slide-in terminal drawer at the bottom of the app that launches the native CLI tool matching the active connection's database type — `psql` for PostgreSQL, `mysql` for MySQL, `sqlite3` for SQLite, `sqlplus` for Oracle, `sqlcmd` for MSSQL, `clickhouse-client` for ClickHouse. The terminal is a full interactive pseudo-terminal (PTY) rendered via xterm.js in the Svelte frontend and backed by a Tauri Rust command that spawns the CLI with a proper PTY.

Toggle: `` Ctrl+` ``.

---

## Problem Statement / Motivation

Power users frequently need to drop into the native CLI for features unavailable in the app GUI: `\d+ tablename`, stored procedure debugging, `EXPLAIN ANALYZE` with `\timing`, Oracle's SQL\*Plus-specific syntax, etc. Currently they must leave the app, open a terminal, and reconstruct the connection string manually. An embedded terminal that auto-connects to the active session removes that friction entirely.

---

## Proposed Solution

1. **Frontend:** `TerminalPanel.svelte` — xterm.js instance in a bottom drawer with a CSS `translateY` slide animation. Resizable via drag handle.
2. **Backend (Tauri Rust):** New commands to spawn a PTY process, stream I/O bidirectionally, and resize the PTY.
3. **DB CLI mapping:** Map `connection.db_type` → binary + argv, constructing the connection string from the stored connection's fields.
4. **Keybinding:** `` Ctrl+` `` added to the global handler in `+layout.svelte`.

---

## Technical Considerations

### PTY Backend (Rust)

`tauri-plugin-shell` does NOT support a full TTY — it treats processes as dumb pipes, which breaks `psql`'s readline, colors, and interactive prompts. Use the `portable-pty` crate directly (from the wezterm ecosystem, battle-tested) to allocate a real PTY pair.

New Tauri commands in `src-tauri/src/commands.rs`:

- `spawn_db_terminal(connection_id: String, cols: u16, rows: u16) -> Result<String, String>` — spawns PTY, returns an opaque `terminal_id`
- `send_terminal_input(terminal_id: String, data: String) -> Result<(), String>` — writes bytes to PTY master
- `resize_terminal(terminal_id: String, cols: u16, rows: u16) -> Result<(), String>` — calls `pty.resize()`
- `close_terminal(terminal_id: String) -> Result<(), String>` — kills child, drops PTY

PTY output is streamed back to the frontend via Tauri **channels** (`tauri::ipc::Channel<String>`). Output is emitted as base64-encoded byte chunks to preserve binary escape sequences (colors, cursor movement) that xterm.js understands natively.

Terminal state (alive PTY handles) is stored in a `Mutex<HashMap<String, PtyHandle>>` in Tauri's managed state, similar to how connections are managed.

### DB CLI Mapping

```
postgres     → psql      -U {user} -h {host} -p {port} {database}
mysql        → mysql     -u {user} -h {host} -P {port} -p {database}
sqlite       → sqlite3   {filename}
oracle       → sqlplus   {user}/{password}@{host}:{port}/{service_name}
mssql        → sqlcmd    -S {host},{port} -U {user} -P {password} -d {database}
clickhouse   → clickhouse-client --host {host} --port {port} --user {user} --password {password} --database {database}
```

Password is passed via environment variable where supported (`PGPASSWORD`, `MYSQL_PWD`) to avoid shell history leakage. For Oracle/sqlplus the password appears in argv — document this limitation.

The CLI binary path is looked up via `which`/`where` at spawn time; if not found, surface a user-readable error: "psql not found on PATH. Install PostgreSQL client tools."

### Frontend Architecture

**`TerminalPanel.svelte`** mounts below `ResultPane` inside `TabbedEditor.svelte`. Panel visibility is tracked via a Svelte 5 `$state(false)` rune in a shared `terminalStore`.

Slide animation: pure CSS transition on `transform: translateY`. Panel is always in the DOM when `showTerminal` is true; the transition fires on mount via Svelte's `transition:` directive or a CSS class toggle.

```svelte
<!-- TabbedEditor.svelte — insertion point below ResultPane -->
{#if showTerminal}
  <TerminalPanel
    connectionId={activeConnectionTab.connectionId}
    dbType={activeConnection?.db_type}
    transition:slide={{ duration: 200, axis: 'y' }}
  />
{/if}
```

xterm.js integration:
- Install `@xterm/xterm` and `@xterm/addon-fit`
- Mount `new Terminal({ ... })` in `onMount`, attach `FitAddon`
- Wire `terminal.onData(data => invoke('send_terminal_input', ...))` for user input
- Wire Tauri channel listener → `terminal.write(atob(chunk))` for output
- Wire `ResizeObserver` on the panel container → `fitAddon.fit()` → `invoke('resize_terminal', ...)`

**Resize handle:** A draggable 4px divider at the top edge of the panel. Dragging updates panel height stored in `$state`. Minimum height: 120px. Maximum: 60% of viewport.

### Keybinding

Add to `/src/routes/+layout.svelte` lines 59–92 global `handleKeydown`:

```typescript
if ((e.ctrlKey || e.metaKey) && e.key === '`') {
  e.preventDefault();
  terminalStore.toggle();
}
```

Note: `` Ctrl+` `` is safe — not intercepted by common terminal emulators (confirmed in learnings from `2026-04-04-006-feat-tui-mvp-plan.md` re: key trapping).

---

## System-Wide Impact

- **Interaction graph:** Toggle fires `terminalStore.toggle()` → `TabbedEditor.svelte` re-renders → `TerminalPanel` mounts → `onMount` calls `spawn_db_terminal` Tauri command → Rust spawns PTY process + starts output relay loop → events stream to frontend.
- **Error propagation:** CLI not found → `spawn_db_terminal` returns `Err(String)` → frontend shows inline error banner inside the panel instead of a toast. PTY process exits unexpectedly → frontend detects channel close and shows "Terminal session ended. Press Enter to restart."
- **State lifecycle risks:** If user switches connection tabs while terminal is open — the existing PTY continues running against the OLD connection (this is fine and expected, like any terminal). Document this. `close_terminal` must be called when the panel is explicitly closed or the connection tab is removed.
- **API surface parity:** No agent/API exposure needed for this feature in v1.

---

## Acceptance Criteria

- [ ] `` Ctrl+` `` toggles the terminal panel open/closed with a smooth slide animation (≤200ms)
- [ ] Panel opens and launches the correct CLI for each supported DB type (postgres, mysql, sqlite, oracle, mssql, clickhouse)
- [ ] Terminal is fully interactive — readline editing, colors, cursor movement work correctly (requires real PTY, not pipe)
- [ ] Panel height is resizable by dragging the top handle; persists within the session
- [ ] Closing the panel cleanly terminates the CLI process (no zombie processes)
- [ ] If the CLI binary is not found on PATH, an informative error is shown inside the panel
- [ ] Terminal resizes correctly when the window is resized or the panel is dragged
- [ ] Password is not passed via shell argv for postgres and mysql (use env vars instead)
- [ ] xterm.js renders with a theme matching the app's dark/light mode

---

## Success Metrics

- Users can execute native CLI commands without leaving the app
- No zombie PTY processes after panel close
- Terminal opens within 500ms of toggle

---

## Dependencies & Risks

| Dependency | Notes |
|---|---|
| `portable-pty` crate | Add to `src-tauri/Cargo.toml`. Mature crate (wezterm). |
| `@xterm/xterm` + `@xterm/addon-fit` | npm packages. MIT license. Standard choice (VS Code, Gitpod, Zed). |
| Native CLI tools | User must have `psql`, `mysql`, etc. installed. App cannot install them. Surface clear errors. |
| PTY on Windows | `portable-pty` supports Windows via ConPTY. Test separately. |
| Oracle `sqlplus` password in argv | Security limitation. Note in UI tooltip. Future: use `ORACLE_PWD` env or wallet. |
| SSH tunnel connections | If connection is through an SSH tunnel managed by the app, the CLI cannot reuse it directly — the CLI would need its own SSH tunnel or direct access. **Defer to v2.** |

---

## Sources & References

### Internal References

- Layout shell: `src/routes/+layout.svelte:59–92` — global keybinding handler
- Tab editor area: `src/lib/components/TabbedEditor.svelte:130–151` — insertion point for `TerminalPanel`
- Result pane (model for bottom drawer): `src/lib/components/ResultPane.svelte`
- DB type detection: `core/src/lib.rs:13` — `detect_database_type(url) -> DatabaseType`
- Connection store: `src/lib/stores/connections.svelte.ts` — `db_type` field
- Tauri commands: `src-tauri/src/commands.rs:29–39` — existing command pattern to follow

### External References

- `portable-pty` crate: https://docs.rs/portable-pty/latest/portable_pty/
- xterm.js: https://xtermjs.org/docs/
- `@xterm/addon-fit`: https://github.com/xtermjs/xterm.js/tree/master/addons/addon-fit
- Tauri v2 IPC channels: https://v2.tauri.app/develop/calling-rust/#channels

### New Files

- `src/lib/components/TerminalPanel.svelte` — xterm.js terminal UI component
- `src/lib/stores/terminal.svelte.ts` — `showTerminal` state + `terminalId` tracking
- `src-tauri/src/terminal.rs` — PTY spawn/relay logic, `PtyHandle` struct, managed state
