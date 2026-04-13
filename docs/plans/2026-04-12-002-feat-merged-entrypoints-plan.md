---
title: "feat: Merge entrypoints into single sqlator binary"
type: feat
status: active
date: 2026-04-12
origin: docs/brainstorms/2026-04-12-merged-entrypoints-requirements.md
---

# feat: Merge entrypoints into single sqlator binary

## Overview

Merge the three separate binaries (`sqlator-app`, `sqlator-tui`, `sqlator-web`) into a single `sqlator` binary with subcommands: `sqlator` (GUI), `sqlator tui`, `sqlator web`. This simplifies distribution and makes the product feel unified.

## Problem Statement / Motivation

Three separate binaries that share `sqlator-core` complicates distribution and adds cognitive overhead. Users need to know which binary to install and run. A single binary with subcommands is simpler to ship and use. (see origin: `docs/brainstorms/2026-04-12-merged-entrypoints-requirements.md`)

## Proposed Solution

Absorb CLI dispatch into `src-tauri/src/main.rs`. The `src-tauri` crate adds clap and depends on `tui-app` and `web-server` as libraries. Both sub-crates are refactored to expose `pub fn run()` entrypoints. The binary name changes from `sqlator-app` to `sqlator`.

**Why absorb into src-tauri rather than a new crate?** Tauri's build pipeline (`tauri build`) specifically targets the `src-tauri/` crate. A separate crate would require custom build configuration to tell Tauri where to find the binary. Keeping dispatch in `src-tauri` is the simplest path that preserves Tauri compatibility. (Resolves outstanding question from origin: "Should the unified entrypoint be a new workspace member, or should src-tauri absorb the dispatch logic?")

## Technical Considerations

- **Tauri `generate_context!()`** embeds the frontend dist at compile time — it's always included in the binary even when running `tui` or `web` mode. This is acceptable per the brainstorm decision "binary size is acceptable".
- **`windows_subsystem = "windows"`** is a compile-time attribute that's a no-op on macOS/Linux. Keep it with a comment documenting the Windows CLI limitation. (see origin: Scope Boundaries — Windows is out of scope)
- **Runtime mismatch**: TUI creates its own `tokio::Runtime` (synchronous), web uses `#[tokio::main]`. The merged `main()` must handle both patterns — TUI's `run()` stays synchronous, web's `run()` is async and called from a tokio runtime.
- **`--static-dir` default**: When the binary is inside a Tauri bundle (e.g., `/Applications/SQLator.app/Contents/MacOS/SQLator`), `./build` won't resolve correctly. Default should resolve relative to the binary location via `std::env::current_exe().parent()`, looking for a sibling `build/` or `../Resources/build/`, falling back to `./build`. (Resolves outstanding question from origin)
- **Headless environments**: When Tauri fails to launch (no display server), print a helpful message suggesting `sqlator tui` or `sqlator web` and exit with code 1. (Gap identified by SpecFlow — not in original requirements)
- **Exit codes**: 0 = success, 1 = application error, 2 = CLI usage error (clap default).
- **`gui` subcommand**: Add as an explicit optional subcommand that behaves identically to running with no arguments. Costs nothing, makes the CLI self-documenting.

## System-Wide Impact

- **Interaction graph**: `main.rs` → clap dispatch → one of three entrypoints. No shared state between modes.
- **Error propagation**: Refactored `run()` functions return `Result` types. The merged `main()` handles errors with clean messages and appropriate exit codes.
- **State lifecycle risks**: None — each mode runs independently with its own state, same as today.
- **API surface parity**: The web server's current `Cli` struct (clap) is replaced by the merged binary's subcommand definitions. All existing CLI flags are preserved.

## Acceptance Criteria

- [ ] `sqlator` (no args) launches the Tauri GUI with identical behavior to the current `sqlator-app` binary (R2)
- [ ] `sqlator gui` behaves identically to `sqlator` with no args (explicit subcommand alias)
- [ ] `sqlator tui` launches the terminal UI with identical behavior to the current `sqlator-tui` binary (R3)
- [ ] `sqlator web` launches the web server with all existing flags (`--port`, `--host`, `--config`, `--static-dir`) (R4)
- [ ] `sqlator --help` displays usage information listing all available subcommands (R7)
- [ ] `sqlator --version` displays a single version string
- [ ] Tauri packaging produces a working `.app` bundle on macOS and AppImage on Linux (R5)
- [ ] The packaged binary supports all subcommands when run from a terminal (R6)
- [ ] A symlink from `/usr/local/bin/sqlator` to the binary inside the `.app` bundle works on macOS (R6)
- [ ] `sqlator web --static-dir` defaults to the correct path when the binary is inside a Tauri bundle
- [ ] `sqlator` on a headless machine (no display server) prints a helpful message suggesting `tui` or `web` and exits with code 1
- [ ] `sqlator tui` in a non-TTY environment exits with a clear error message
- [ ] The standalone `sqlator-tui` and `sqlator-web` binaries are removed from the workspace (R8)
- [ ] `cargo build -p sqlator-app` (or the renamed crate) produces a single `sqlator` binary
- [ ] No functionality is lost compared to the current separate binaries

## Success Metrics

- Single binary replaces three — `cargo build` produces one `sqlator` binary
- All three modes work identically to the current standalone binaries
- Tauri packaging pipeline produces working installers without custom configuration

## Dependencies & Risks

- **Risk**: Tauri's build pipeline may be sensitive to the Cargo package name change (`sqlator-app` → `sqlator`). Mitigation: test `tauri build` early in implementation.
- **Risk**: Adding `tui-app` and `web-server` as dependencies of `src-tauri` increases compile time. Mitigation: acceptable per brainstorm decision.
- **Dependency**: Both `tui-app` and `web-server` must be refactored to expose library entrypoints before the merged binary can be built.

## Implementation Phases

### Phase 1: Refactor sub-crates into libraries

- [ ] **1.1** Add `lib.rs` to `tui-app/` exposing `pub fn run() -> Result<(), Box<dyn std::error::Error>>`
  - Move terminal setup/teardown logic from `main.rs` into the `run()` function
  - Keep `app.rs` and `ui.rs` as internal modules
  - `main.rs` becomes a thin wrapper: `fn main() { sqlator_tui::run().unwrap() }`
  - File: `tui-app/src/lib.rs` (new), `tui-app/src/main.rs` (simplified)
  - File: `tui-app/Cargo.toml` — add `[lib]` section

- [ ] **1.2** Add `lib.rs` to `web-server/` exposing `pub async fn run(config: WebConfig) -> Result<(), Box<dyn std::error::Error>>`
  - Create `WebConfig` struct to hold `port`, `host`, `config`, `static_dir`
  - Move server setup from `main.rs` into the `run()` function
  - Keep `handlers.rs`, `state.rs`, `ws_query.rs` as internal modules
  - `main.rs` becomes a thin wrapper that parses CLI and calls `run()`
  - Files: `web-server/src/lib.rs` (new), `web-server/src/main.rs` (simplified)
  - File: `web-server/Cargo.toml` — add `[lib]` section
  - File: `web-server/src/lib.rs` — export `WebConfig` struct

- [ ] **1.3** Verify both refactored crates still build and run correctly as standalone binaries
  - `cargo build -p sqlator-tui && cargo run -p sqlator-tui`
  - `cargo build -p sqlator-web && cargo run -p sqlator-web -- --port 3000`

### Phase 2: Merge entrypoints in src-tauri

- [ ] **2.1** Add dependencies to `src-tauri/Cargo.toml`
  - Add `sqlator-tui = { path = "../tui-app" }`
  - Add `sqlator-web = { path = "../web-server" }`
  - Add `clap = { version = "4.5", features = ["derive"] }`

- [ ] **2.2** Add clap subcommand definitions to `src-tauri/src/main.rs`
  - Define `Cli` enum with subcommands: `Gui`, `Tui`, `Web`
  - `Gui` subcommand: no additional args (alias for default behavior)
  - `Tui` subcommand: no additional args
  - `Web` subcommand: `--port`, `--host`, `--config`, `--static-dir` (same as current `sqlator-web`)
  - Default (no subcommand) dispatches to GUI
  - File: `src-tauri/src/main.rs`

- [ ] **2.3** Implement dispatch logic in `src-tauri/src/main.rs`
  - Parse args with clap
  - `Gui` or no subcommand → call `sqlator_app_lib::run()`
  - `Tui` → call `sqlator_tui::run()`
  - `Web` → create tokio runtime, call `sqlator_web::run(config)`
  - Handle Tauri launch failure with helpful headless message
  - File: `src-tauri/src/main.rs`

- [ ] **2.4** Implement smart `--static-dir` default for web subcommand
  - Use `std::env::current_exe()` to find binary location
  - On macOS: check for `.app/Contents/MacOS/` path pattern, resolve to `../Resources/build/`
  - On Linux: check for AppImage mount path, resolve to sibling `build/` directory
  - Fall back to `./build` if not in a bundle
  - File: `src-tauri/src/main.rs` or a helper module

- [ ] **2.5** Rename binary from `sqlator-app` to `sqlator`
  - Update `src-tauri/Cargo.toml` package name or add `[[bin]] name = "sqlator"`
  - Verify `tauri build` still works with the new name
  - File: `src-tauri/Cargo.toml`

- [ ] **2.6** Update `windows_subsystem` attribute with documentation comment
  - Keep `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`
  - Add comment explaining it's a no-op on macOS/Linux and blocks Windows CLI usage
  - File: `src-tauri/src/main.rs`

### Phase 3: Remove standalone binaries and update workspace

- [ ] **3.1** Remove `main.rs` from `tui-app/` — it's no longer a standalone binary
  - Remove `[[bin]]` or auto-detected binary from `tui-app/Cargo.toml`
  - The crate becomes library-only
  - File: `tui-app/src/main.rs` (delete), `tui-app/Cargo.toml` (update)

- [ ] **3.2** Remove `main.rs` from `web-server/` — it's no longer a standalone binary
  - Remove `[[bin]]` section from `web-server/Cargo.toml`
  - The crate becomes library-only
  - File: `web-server/src/main.rs` (delete), `web-server/Cargo.toml` (update)

- [ ] **3.3** Update workspace `Cargo.toml` if needed
  - Verify all workspace members still build: `cargo build --workspace`
  - File: `Cargo.toml`

### Phase 4: CLI wrapper for Tauri package

- [ ] **4.1** Create macOS symlink strategy
  - Document that installing via `.dmg` or `.app` should include a symlink from `/usr/local/bin/sqlator` to `/Applications/SQLator.app/Contents/MacOS/SQLator`
  - This can be a post-install script in the `.dmg` or a manual step in the README
  - File: README.md or installation docs

- [ ] **4.2** Verify AppImage CLI access on Linux
  - AppImage supports CLI mode by default: `./SQLator.AppImage tui` or `./SQLator.AppImage web`
  - Symlink from `/usr/local/bin/sqlator` to the AppImage path
  - Document in README

### Phase 5: Verification

- [ ] **5.1** Build and test all modes from the single binary
  - `cargo build -p sqlator-app` (or renamed crate)
  - `./target/debug/sqlator --help` — shows subcommands
  - `./target/debug/sqlator --version` — shows version
  - `./target/debug/sqlator` — launches GUI (manual test)
  - `./target/debug/sqlator gui` — launches GUI (manual test)
  - `./target/debug/sqlator tui` — launches TUI (manual test)
  - `./target/debug/sqlator web --port 3000` — launches web server (manual test)
  - `./target/debug/sqlator web --config config.json` — launches in single-db mode (manual test)

- [ ] **5.2** Test Tauri packaging
  - `tauri build` on macOS → verify `.app` bundle and `.dmg`
  - `tauri build` on Linux → verify AppImage
  - Run subcommands from the packaged binary: `/Applications/SQLator.app/Contents/MacOS/SQLator tui`
  - Run subcommands via symlink: `sqlator web --port 3000`

- [ ] **5.3** Test error scenarios
  - `sqlator` on headless environment → helpful error message
  - `sqlator tui` with no TTY → clear error
  - `sqlator web --config nonexistent.json` → clear error
  - `sqlator invalid-subcommand` → clap error message

## Sources & References

- **Origin document:** [docs/brainstorms/2026-04-12-merged-entrypoints-requirements.md](docs/brainstorms/2026-04-12-merged-entrypoints-requirements.md) — Key decisions carried forward: always GUI by default, binary size acceptable, Tauri package + CLI wrapper, macOS + Linux primary
- Tauri lib/bin split pattern: `src-tauri/src/lib.rs:7` (`pub fn run()`)
- Web server clap CLI: `web-server/src/main.rs:17-37`
- TUI entrypoint: `tui-app/src/main.rs:6-31`
- Workspace structure: `Cargo.toml:1-3`
- Tauri config: `src-tauri/tauri.conf.json`
- Web version plan: `docs/plans/2026-04-04-007-feat-web-version-server-mode-plan.md`
- TUI plan: `docs/plans/2026-04-04-006-feat-tui-mvp-plan.md`
