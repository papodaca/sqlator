---
title: "feat: Add URL-based connection creation to TUI"
type: feat
status: active
date: 2026-04-13
---

# feat: Add URL-based Connection Creation to TUI

## Overview

The TUI currently has no way to create connections — the connection list screen is read-only and the empty state explicitly directs users to the desktop app. This feature adds an interactive "add connection" form triggered from the connection list, where the user pastes or types a database connection URL to save a new connection directly in the TUI.

## Problem Statement / Motivation

Users running sqlator in headless or server environments cannot create connections without the Tauri desktop GUI. Adding URL-based creation to the TUI makes the tool self-contained — a user can `ssh` into a machine, run `sqlator`, and add a new connection without switching to a graphical environment.

## Proposed Solution

Add a fourth `AppMode::NewConnection` to the TUI's state machine. When the user presses `a` from the connection list, a two-field form appears: a URL field (pre-focused) and a name field (auto-derived from the URL, editable). Pressing `Enter` parses the URL, constructs a `SavedConnection`, persists it via `ConfigManager`, reloads the list, and returns to `ConnectionList` mode with the new entry selected. `Esc` cancels at any point.

## Technical Considerations

### Key Design Decisions

| Decision | Choice | Reason |
|---|---|---|
| `id` generation | `uuid::Uuid::new_v4().to_string()` | Consistent with how Tauri backend creates connections (`src-tauri/src/commands.rs:26`) |
| `color_id` | Empty string `""` | No color picker yet; consistent with what happens when GUI omits it |
| `connection_type` | `ConnectionType::Direct` | URL-based implies a direct connection; SSH/Docker remain GUI-only for now |
| Port when URL omits it | `url.port_or_known_default().unwrap_or(0)` | The `url` crate returns 5432 for `postgres://`, 3306 for `mysql://`, 0 for unknown — acceptable for display |
| Duplicate URLs | Silently allowed | `config.save_connection` is insert-by-UUID; matching the existing Tauri behavior |
| Password in input | Plain text visible | Consistent with how passwords are stored in `connections.json` today; no keyring integration for DB URLs yet |
| SQLite URL format | Only `sqlite://` scheme supported | `detect_database_type` splits on `://`; `sqlite:./path` (no double-slash) returns `None` — document the limitation |
| Field navigation | `Tab` to move URL → Name, `Enter` to save from either field, `Esc` to cancel | `Tab` is safe in `NewConnection` mode since Workspace tab-switching is a different mode |

### Refactor: URL Parsing Moves to `sqlator-core`

The URL→`SavedConnection` logic currently lives inline in `src-tauri/src/commands.rs:26–68`. To avoid duplication, this logic is extracted into `sqlator-core` before the TUI feature is built on top of it.

**New function in `core/src/models.rs` (or `core/src/connection_builder.rs`):**

```rust
// core/src/models.rs (or core/src/connection_builder.rs)

/// Build a SavedConnection from a raw connection URL.
/// Returns Err if the URL cannot be parsed.
pub fn connection_from_url(url: &str, name: Option<String>) -> Result<SavedConnection, url::ParseError> {
    let parsed = url::Url::parse(url)?;
    let db_type = crate::db::detect_database_type(url)
        .unwrap_or_else(|| "unknown".to_string());
    let host     = parsed.host_str().unwrap_or("").to_string();
    let port     = parsed.port_or_known_default().unwrap_or(0) as i64;
    let database = parsed.path().trim_start_matches('/').to_string();
    let username = parsed.username().to_string();
    let derived_name = name.unwrap_or_else(|| {
        if username.is_empty() { format!("{host}/{database}") }
        else                   { format!("{username}@{host}/{database}") }
    });
    Ok(SavedConnection {
        id: uuid::Uuid::new_v4().to_string(),
        name: derived_name,
        color_id: "".to_string(),
        db_type,
        host,
        port,
        database,
        username,
        url: url.to_string(),
        ssh_profile_id: None,
        group_id: None,
        connection_type: ConnectionType::Direct,
        container_name: None,
        container_port: None,
    })
}
```

**Tauri backend update** — `src-tauri/src/commands.rs:26–68` is replaced with a call to `connection_from_url`:

```rust
// src-tauri/src/commands.rs (simplified after refactor)
let conn = sqlator_core::models::connection_from_url(&config.url, Some(config.name))?;
state.config.save_connection(conn)?;
```

**TUI usage** — `tui-app/src/app.rs` calls the same function:

```rust
// tui-app/src/app.rs (save_new_connection method)
let conn = sqlator_core::models::connection_from_url(&url_str, Some(name_str))?;
self.config.save_connection(conn)?;
self.connections = self.config.get_connections();
```

No new dependencies are needed in `tui-app` — `uuid` already lives in `core`.

### uuid Dependency

`uuid` is already in `core/Cargo.toml` (used by `connection_from_url`). The TUI calls core, so no additional `uuid` dependency is needed in `tui-app/Cargo.toml`.

### Inline Validation

On every keystroke in the URL field, call `detect_database_type(&input)`. If the input is non-empty and the result is `None`, show a `"⚠ Unsupported or invalid URL scheme"` message beneath the input. This gives real-time feedback without blocking submission.

## System-Wide Impact

- **No other modes affected** — `AppMode` dispatch is a match statement; adding a new arm is additive.
- **`config.save_connection` is synchronous** — no async spawn needed; the call completes inline in the key handler.
- **Connection list refresh** — `self.connections = self.config.get_connections()` is the same reload call used by `disconnect_and_return_to_list()`. No new codepath.
- **`connections.json` is a flat JSON array** — no schema migration needed.
- **Tauri desktop app unaffected** — it reads the same `connections.json` file; any connection added via TUI will appear in the GUI on next load.

## Acceptance Criteria

- [ ] `connection_from_url` function lives in `sqlator-core` and is used by both the Tauri backend and the TUI (no duplication)
- [ ] Tauri connection creation still works after the core refactor
- [ ] Pressing `a` from the connection list opens the new connection form
- [ ] URL input field is focused by default
- [ ] A name is auto-derived from the URL as the user types
- [ ] `Tab` moves focus from URL field to Name field (and back)
- [ ] `Enter` saves the connection when URL is valid and non-empty
- [ ] `Esc` cancels and returns to the connection list without saving
- [ ] On save, the new connection appears in the list and the cursor moves to it
- [ ] Inline validation shows an error hint for unrecognized URL schemes
- [ ] Empty URL field on `Enter` shows an error hint, does not save
- [ ] All supported schemes work: `postgres://`, `postgresql://`, `mysql://`, `mariadb://`, `sqlite://`, `mssql://`, `sqlserver://`, `oracle://`, `clickhouse://`
- [ ] The connection list footer hint bar shows `a Add` alongside existing hints
- [ ] The empty-state message is updated (or supplemented) to mention the `a` shortcut
- [ ] `uuid` crate added to `tui-app/Cargo.toml` and project compiles cleanly

## Implementation Phases

### Phase 1 — Core Refactor (prerequisite)

Extract URL parsing logic from Tauri into `sqlator-core` so it's shared:

| File | Change |
|---|---|
| `core/src/models.rs` | Add `pub fn connection_from_url(url: &str, name: Option<String>) -> Result<SavedConnection, url::ParseError>` |
| `src-tauri/src/commands.rs:26–68` | Replace inline URL parsing with a call to `connection_from_url` |

Verify the Tauri app still compiles and existing connection creation still works after this refactor.

### Phase 2 — TUI New Connection Form

| File | Change |
|---|---|
| `tui-app/src/app.rs` | Add `NewConnection` variant to `AppMode`; add `new_conn_url`, `new_conn_name` `TextArea` fields to `App`; add `new_conn_error: Option<String>`; add `handle_new_connection_key()` and `save_new_connection()` methods; update `handle_key()` dispatch; update `handle_connection_list_key()` for `a` key |
| `tui-app/src/ui.rs` | Add `draw_new_connection()` function; update `draw_connection_list()` footer to include `a Add`; update empty-state message |

## Known Limitations (Scope Exclusions)

- **SQLite `sqlite:./path` format** (no double slash) is not detected by `detect_database_type` — only `sqlite://` works. Document in the UI as: `"SQLite format: sqlite:///path/to/file.db"`.
- **No color picker** — `color_id` defaults to `""` (no color dot in the list).
- **No connection test before save** — connection is saved immediately on Enter without verifying it can connect. A future enhancement could add async validation.
- **SSH tunnel / Docker container connections** not supported via this form — remain GUI-only.
- **No duplicate URL check** — two entries with the same URL but different IDs are silently permitted.
- **Password masking** — the URL input shows the full URL including embedded passwords in plain text.

## Dependencies & Risks

- **Merged entrypoints plan** (`docs/plans/2026-04-12-002-feat-merged-entrypoints-plan.md`) — if that plan lands first and exposes a `tui::run(opts)` API, URL-based creation might also be added as a `--add <url>` CLI flag. This plan focuses on the interactive TUI form only; CLI flag support is orthogonal and can be layered on top.
- **`tui-textarea` API** — already used for the SQL editor in `app.rs`. The same `TextArea::new([])`, `.input(key)`, and `.lines()[0]` access pattern applies.

## Sources & References

- `tui-app/src/app.rs` — `AppMode` enum, `App` struct, `handle_key`, `start_connect`
- `tui-app/src/ui.rs` — `draw_connection_list`, rendering patterns
- `core/src/models.rs:77` — `SavedConnection` struct fields
- `core/src/config.rs:73` — `save_connection` signature
- `core/src/db/mod.rs:371` — `detect_database_type` (public, reusable)
- `src-tauri/src/commands.rs:26–68` — reference implementation for URL→SavedConnection
- `docs/plans/2026-04-12-002-feat-merged-entrypoints-plan.md` — related CLI unification work
