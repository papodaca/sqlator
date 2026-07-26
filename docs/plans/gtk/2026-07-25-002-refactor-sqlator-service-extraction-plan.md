---
title: "refactor: Extract sqlator-service shared application layer"
type: refactor
status: active
date: 2026-07-25
---

# refactor: Extract sqlator-service shared application layer

## Overview

Extract the application logic currently duplicated between `src-tauri/src/commands.rs` and
`web-server/src/handlers.rs` into a new `sqlator-service` crate sitting between
`sqlator-core` and the frontends. Migrate Tauri onto it first, then web, then the TUI.

This is a prerequisite for the GTK frontend, but it stands on its own merit: it removes
~1,000 duplicated lines and closes five known divergence bugs — four by adopting Tauri's
behavior, one by adopting the corrected table extraction from
[plan 007](../2026-07-25-007-fix-tauri-table-extraction-editability-plan.md) in place of
either existing copy.

---

## Problem Statement / Motivation

`sqlator-core` stops at the driver boundary. Everything above it — deciding *how* to connect
given a `connection_type`, standing up and tearing down SSH tunnels, resolving credentials
out of keyring or vault, inspecting Docker containers, caching schema metadata, resolving
import collisions — lives in the frontends, twice.

Roughly **1,000–1,100 of `commands.rs`'s 1,727 lines have a near-verbatim twin in
`handlers.rs`**:

| Logic | Tauri | Web |
|---|---|---|
| Export builder | `commands.rs:1014-1079` | `handlers.rs:789-854` |
| Import (3-pass group re-parenting, rename) | `commands.rs:1101-1243` | `handlers.rs:873-993` |
| `extract_single_table` + regex fallback | `commands.rs:1530-1615` | `handlers.rs:1093-1152` |
| Schema metadata + 5min TTL cache | `commands.rs:1618-1668` | `handlers.rs:1154-1197` |
| `build_auth_config_for_profile` | `commands.rs:772-796` | `handlers.rs:1068-1089` |
| `build_jump_hosts_for_profile` | `commands.rs:745-770` | `handlers.rs:1369-1393` |
| Tunnel create / close / list | `commands.rs:431-513` | `handlers.rs:553-618` |
| Docker discover / list / test | `commands.rs:1303-1450` | `handlers.rs:1244-1350` |
| Vault + storage mode | `commands.rs:800-875` | `handlers.rs:620-666` |
| Groups CRUD | `commands.rs:877-944` | `handlers.rs:668-725` |
| `resolve_connection_type`, `unique_name`, `build_url_no_password`, `parse_auth_method` | `commands.rs:1245-1299` | `handlers.rs:995-1012`, `:1354`, `:429` |

There is a **third** copy of credential resolution: `resolve_ssh_auth` at
`src-tauri/src/terminal.rs:364` is `build_auth_config_for_profile` again.

The copies have drifted, and the drift is user-visible:

1. **Web's `connect_database` (`handlers.rs:339-358`) ignores `connection_type` entirely.**
   It calls `state.db.connect(&id, &conn.url)` on the raw URL — no tunnel, no Docker inspect,
   no credential lookup. SSH-tunneled and Docker connections that work on the desktop fail or
   connect to the wrong host in the browser. `disconnect_database` (`:360`) correspondingly
   never closes a tunnel, unlike Tauri's (`commands.rs:323`).
2. **`get_ddl` is not implemented in web.** The shared Svelte UI calls it
   (`SchemaDdlViewer.svelte:25`), the web adapter maps it (`web-adapter.ts:7`), and `handle()`
   returns `unknown command`. The DDL viewer is broken in web mode.
3. **Web's `test_connection_with_ssh` passes `vec![]` for jump hosts (`:1051`)** where Tauri
   passes `build_jump_hosts_for_profile` (`commands.rs:714`). Proxy-jump profiles connect
   wrong.
4. **Web's import hardcodes `local_port_binding` and `keepalive_interval` to `None`
   (`:948-949`)**; Tauri preserves them (`commands.rs:1190-1191`).
5. **Tauri's regex fallback rejects any SQL containing a comma (`commands.rs:1601`); web's
   does not (`:1142`).** Same query, different editability verdict, different UI. Neither copy
   is correct — web's accepts an implicit comma join as editable. Resolved separately by
   [plan 007](../2026-07-25-007-fix-tauri-table-extraction-editability-plan.md).

Meanwhile `tui-app` avoided duplication by depending only on `sqlator-core` — and
consequently `start_connect` (`tui-app/src/app.rs:350`) calls `db.connect(&conn_id,
&conn.url)` on the raw URL. **The TUI cannot use SSH tunnels, Docker connections, or the
credential store at all.** Grepping the crate for ssh/tunnel/credential/docker returns zero
hits.

A fourth frontend must pick one of those two failure modes unless this is fixed first.

---

## Proposed Solution

A new workspace member, `sqlator-service`, exposing one `AppService` that owns what both
`AppState`s own today: `ConfigManager`, `DbManager`, the tunnel `DashMap`, `CredentialStore`,
and the schema TTL cache.

**Named `sqlator-service`, not `sqlator-app`** — the `src-tauri` package is already named
`sqlator-app` (`src-tauri/Cargo.toml:2`).

### Modules

| Module | Contents |
|---|---|
| `connections` | CRUD, the URL→`SavedConnection` builder with the db-type/default-port table (currently inlined 4x), clone, group moves |
| `connect` | `connect(id)` dispatching on `ConnectionType` across direct / SSH tunnel / remote Docker / local Docker; disconnect-with-teardown; the four `test_*` variants with ephemeral tunnels |
| `ssh` | Profile CRUD with credential side effects, `build_auth_config_for_profile`, `build_jump_hosts_for_profile`, tunnel registry create/close/list |
| `docker` | Discovery and DTO mapping, remote and local |
| `credentials` | Storage-mode switching with migration; vault create/unlock/lock/settings |
| `schema` | `extract_single_table` + regex fallback + the TTL cache |
| `portability` | The `Exported*` types and import/export resolution by name |
| `terminal_spec` | The CLI-argument builders from `terminal.rs:44-360` — frontend-agnostic; only PTY lifecycle is Tauri-bound. GTK needs these for VTE. |

### Stays frontend-specific

- **Tauri:** the `#[tauri::command]` wrappers, `State` extraction, the mpsc→`tauri::ipc::Channel` bridge (`commands.rs:334`, ~20 lines), and PTY lifecycle in `terminal.rs`.
- **Web:** axum routing, the string dispatch table, JSON arg extraction (`get`/`get_opt`, `handlers.rs:36`/`:45`), status-code mapping, the WS bridge in `ws_query.rs`, single-db policy, and the `/api/export-file` download shim (`:164`).
- **GTK:** glib main-context bridging.
- **TUI:** ratatui event loop.

### Sequencing — safety first

Test coverage across everything being moved is **zero**. The order below is chosen so that
each step is either mechanically verifiable or covered by a test written immediately before it.

1. **Complete [phase 0a](2026-07-25-000-chore-characterization-tests-before-refactor-plan.md)
   first.** It writes characterization tests for everything moved here and produces the
   divergence ledger this phase needs in order to make decisions rather than discoveries. It
   also enables `clippy::await_holding_lock` — before tunnel code is touched, not after.
2. **Resolve the divergence ledger.** Each row needs a chosen winner before the corresponding
   code moves, and **the winner is chosen per row, not per frontend.** Tauri has the more
   complete behavior for orchestration (rows 1–4), but for the three `extract_table_regex`
   rows **neither copy is correct** — web is right about the SELECT list, Tauri is right about
   the FROM clause, and both share a fourth bug neither ledger row captured. Those rows are
   resolved by [plan 007](../2026-07-25-007-fix-tauri-table-extraction-editability-plan.md);
   take its corrected implementation into `sqlator-service` rather than picking a side.
3. **Move pure helpers**, including the URL-parse + db-type + default-port table currently
   inlined four times (`commands.rs:26`, `:72`, and both web equivalents).
4. **Move credential and auth resolvers**, collapsing all three copies including
   `terminal.rs:364`.
5. **Move connect / tunnel orchestration last** — this is where the divergence decisions live.
6. **Migrate Tauri first** — it has the complete orchestration behavior, so it is the smaller
   diff — verify manually, then point web at the shared layer. This is a sequencing choice
   about which frontend moves first, *not* a rule that Tauri's implementation wins each
   divergence; see step 2. Web migration fixes divergences 1–4 as a side effect and is the
   moment to manually verify SSH and Docker connections in browser mode.
7. **Migrate the TUI**, which gains tunnel, Docker and credential support for the first time.

---

## Technical Considerations

### Design decisions this forces

These are genuine decisions, not mechanical moves. Each needs an answer before the relevant
step.

**Resolved 2026-07-26 (sqlator-9g1.3):**

- **Tunnel registry key — RESOLVED (refined 2026-07-26): `(ssh_profile_id, target_host,
  target_port)` with refcounting.** Multiple DB connections that share a profile **and** the
  same remote target share one tunnel (one local listener / session). Same profile with
  different targets gets separate tunnels — required because `TunnelHandle` is 1:1 with a
  single forward target today. Track which connection ids use each entry; create on first
  user, tear down when the last user disconnects (or an explicit close with no remaining
  users). Standalone `create_ssh_tunnel` / `close_ssh_tunnel` use the same composite key.
  `terminal.rs` (and GTK VTE later) look up the local forward port via public service API:
  connection id → profile + resolved target → tunnel. Do **not** key the shared registry by
  connection id. Rationale and the follow-up “one SSH session, many local forwards” design:
  [2026-07-26-001-design-ssh-tunnel-registry-keying.md](2026-07-26-001-design-ssh-tunnel-registry-keying.md).
- **Error typing — RESOLVED: option A.** Introduce `ServiceError` in `sqlator-service`
  wrapping `CoreError` + SSH + Docker + vault/config failures with stable `code`s
  (`VAULT_LOCKED`, `NO_CONNECTION`, …). Frontends flatten once at the adapter boundary
  (Tauri → `String`, web → `(StatusCode, String)`, GTK → dialog vs toast by code).
- **Export path policy — RESOLVED: service returns JSON bytes/string; frontend decides.**
  Backend builds the export payload only. Tauri/web/GTK each choose persistence:
  GTK **must** use a native save dialog; web may keep temp-file + HTTP download; Tauri may
  keep Downloads or also move to a dialog later. Existing “write path inside the shared
  layer” behavior is intentionally removed.
- **Web `single_db` mode — RESOLVED: connection-source abstraction in the service** (plan
  recommendation). The connection set may be a fixed synthetic singleton (web single-db,
  future GTK `--config file.json`) or the normal multi-db `ConfigManager` store. Policy
  guards like `require_multi_db` live against that abstraction, not only in web handlers.
- **Which divergent behavior wins**, per divergence. Unchanged: ledger winners from phase
  0a / plan 007. Once web calls shared `connect()`, SSH and Docker connections actually
  tunnel in web mode — correct behavior change; russh listener may run inside the server
  process.

### Runtime and thread-safety

Nothing in the stack is runtime-agnostic: sqlx is built with `runtime-tokio`, tunnels call
`tokio::spawn` and `TcpListener`, `DbManager::connect` uses `tokio::time::timeout`, local
Docker uses `tokio::process`, ClickHouse uses reqwest, Oracle uses deadpool. `sqlator-service`
will be tokio-only, and that is fine.

The good news for GTK: **everything is already `Send + Sync`.** `DbManager` is `DashMap`-based,
`TunnelHandle` holds `Arc<tokio::Mutex<_>>`, `CredentialStore` and `VaultBackend` use
`std::sync::Mutex`, and `CredentialBackend` explicitly requires `Send + Sync`
(`core/src/credentials/mod.rs:13`). `Arc<AppService>` crosses the runtime boundary without
trouble and GTK widgets never need to.

### Two blocking-call hazards to fix during extraction

Both are already technically wrong and both will visibly freeze a GTK frame:

- Every `ConfigManager` method does synchronous file I/O behind a non-`async` signature
  (`core/src/config.rs:53-66` — full read on every get, full read-modify-write on every set).
- `VaultBackend::unlock` runs Argon2id at 19 MiB / 2 iterations
  (`core/src/credentials/vault_backend.rs:215`) — hundreds of milliseconds.

Both are called from `async fn`s today without `spawn_blocking`. Make the service's config and
vault entry points `async` wrappers over `spawn_blocking`.

### Fix while in the neighbourhood

`connections.json` is read-modify-written with **no locking and no atomic rename**, unlike the
vault which does use temp-file+rename (`vault_backend.rs:261`). Tauri accesses it with no
synchronization at all; web wraps it in a `tokio::Mutex` with the comment "not internally
synchronized". The merged-entrypoints plan
(`docs/plans/2026-04-12-002-feat-merged-entrypoints-plan.md`) implies desktop and web may run
simultaneously, in which case concurrent writes lose data. Extraction is the natural moment to
make `ConfigManager` internally synchronized and atomic.

### Leave room for query history

Query history does not exist. `ConfigManager` persists only the *last* query text per
connection (`queries: HashMap<String, String>`, `core/src/config.rs:17`) plus an opaque
`tab_state` blob. There is a plan (`docs/plans/2026-04-13-003-feat-query-history-panel-plan.md`)
but no implementation. Design the service with a place for it rather than retrofitting.

---

## System-Wide Impact

- **Interaction graph:** each frontend's command/handler becomes a thin adapter — deserialize
  args, call one `AppService` method, serialize the result. The mpsc `QueryEvent` stream from
  `core` continues to flow through untouched; only its terminal bridge differs per frontend
  (Tauri `Channel`, WebSocket, ratatui `try_recv`, glib `spawn_future_local`).
- **Error propagation:** frontends stop stringifying at call sites and start flattening a
  typed error once, at the adapter boundary. The `code` field survives to the UI for the first
  time.
- **State lifecycle risks:** the tunnel registry moves ownership from two `AppState`s to one
  `AppService`. Stale-tunnel cleanup on reconnect (`commands.rs:214`, `:268`) and
  ephemeral-test-tunnel teardown on the error path (`commands.rs:740`, `:1448`) must survive
  the move exactly — losing either leaks a listener and an SSH session.
- **API surface parity:** no change to the Tauri command names or the web routes. The Svelte
  frontend should require zero changes; if it does, something moved that shouldn't have.

---

## Acceptance Criteria

- [ ] Phase 0a is complete and its divergence ledger has a chosen winner on every row
- [ ] `sqlator-service` exists as a workspace member with the eight modules above
- [ ] Every characterization test from 0a still passes after the move, or its deletion is
      justified by a ledger row
- [ ] `clippy::await_holding_lock` is enabled in CI and passes
- [ ] `src-tauri/src/commands.rs` contains only IPC glue; no tunnel, credential, Docker, import/export, or schema-cache logic remains
- [ ] `web-server/src/handlers.rs` contains only HTTP glue and single-db policy
- [ ] The third copy of credential resolution (`terminal.rs:364`) is gone
- [ ] Web `connect_database` establishes SSH tunnels and performs Docker discovery (divergence 1 fixed)
- [ ] `get_ddl` works in web mode (divergence 2 fixed)
- [ ] Web `test_connection_with_ssh` honours proxy-jump profiles (divergence 3 fixed)
- [ ] Web import preserves `local_port_binding` and `keepalive_interval` (divergence 4 fixed)
- [ ] Exactly one table-extraction fallback exists, it is the corrected implementation from
      [plan 007](../2026-07-25-007-fix-tauri-table-extraction-editability-plan.md), and neither
      frontend keeps a private copy (divergence 5 fixed)
- [ ] TUI can open an SSH-tunneled and a Docker connection
- [ ] `ConfigManager` writes are atomic and internally synchronized
- [ ] Config and vault entry points do not block the calling thread
- [ ] The Svelte frontend runs unchanged against both Tauri and web
- [ ] Manual QA matrix passed: each of 7 engines x {direct, SSH tunnel, remote Docker, local Docker} where applicable, on both Tauri and web

---

## Success Metrics

- `commands.rs` and `handlers.rs` each drop below ~400 lines
- Zero logic duplicated between them (verified by review, not by grep)
- Five known divergence bugs closed
- TUI gains three capabilities it never had

---

## Dependencies & Risks

| Dependency | Notes |
|---|---|
| **Phase 0a (characterization tests)** | Hard dependency — this phase is not safe without it |
| [Plan 007](../2026-07-25-007-fix-tauri-table-extraction-editability-plan.md) | Soft — either order works. If 007 lands first, move its single corrected function; if this lands first, move both copies as-is and let 007 fix the merged one |
| None external | Pure refactor; no new crates |
| Blocks phase 3 | GTK parity build-out consumes `AppService` |
| Does not block phase 0b or 2 | The spike uses synthetic data; the skeleton can stub the service |

### Load-bearing behavior with no test coverage

Every item below is currently uncovered and would fail silently:

| Behavior | Location | Failure mode if broken |
|---|---|---|
| `select!` + mpsc in `forward_stream` | `core/src/ssh/tunnel.rs:133` | Reintroducing `Arc<Mutex<Channel>>` while "cleaning up" recreates a documented deadlock |
| Stale-tunnel cleanup on reconnect | `commands.rs:214`, `:268` | Leaked listener + SSH session per reconnect |
| Ephemeral test tunnels closed on the error path | `commands.rs:740`, `:1448` | An early `?` return leaks the tunnel |
| 5s connect timeout | `core/src/db/mod.rs:70` | Hangs on unreachable hosts |
| `install_default_drivers()` in `DbManager::new` | `core/src/db/mod.rs` | Global side effect; `Any` pool breaks without it |
| `SshAuthConfig`'s zeroizing `Drop` | `core/src/ssh/auth.rs:90` | Deriving `Clone` carelessly, or holding auth configs long-term in the service, weakens secret hygiene |
| Vault idle-timeout semantics | `vault_backend.rs:188` | Every access touches `last_activity`; an extra probe call changes when the vault auto-locks |
| `limit.min(1000) + 1` computes `has_more` | `core/src/db/mod.rs:969` | Pagination silently wrong |
| MySQL `CAST(... AS CHAR)` for VARBINARY information_schema columns | `core/src/db/mod.rs:522` | Schema browser breaks on MySQL |
| Schema cache key uses `Debug` on an `Option` | Both copies | Fragile but currently consistent; must stay consistent |

| Risk | Mitigation |
|---|---|
| Zero test coverage over 1,100 moved lines | Phase 0a is the mitigation; mechanical reviewable moves for whatever it cannot cover; manual QA matrix |
| Web behavior change (tunnels now real) | Called out explicitly; verify in browser mode as its own QA step |
| Refactor stalls half-done, leaving three copies | Migrate Tauri fully before starting web; do not interleave |
| Secret hygiene regression | Review every `Clone`/`Debug` derive added to credential or auth types |

---

## Sources & References

### Internal References

- `core/src/lib.rs` — 18 lines; 7 public modules, no driver trait
- `core/src/db/mod.rs:28` — `DatabasePool` enum, 7 variants, match dispatch throughout
- `core/src/db/mod.rs:38` — `DbManager { pools: DashMap<String, DatabasePool> }`
- `core/src/db/mod.rs:349-361` — batch execution `UNSUPPORTED` for Mssql/Oracle/ClickHouse
- `core/src/db/mod.rs:417` — `close_pool` is a no-op for Mssql/Oracle/ClickHouse
- `core/src/models.rs:132` — `QueryEvent`; the one design that already generalizes across frontends
- `core/src/models.rs:101` — `SavedConnection::masked_url`; **DB passwords live in plaintext in the URL in `connections.json`** and are only masked on the way out
- `core/src/credentials/mod.rs:13` — `CredentialBackend: Send + Sync`
- `core/src/credentials/mod.rs:140` — key format `ssh-profile:{id}:{password|passphrase}`
- `core/src/ssh/tunnel.rs:74`, `:133`, `:234-269`, `:395`, `:411` — forwarding, jump chain, ephemeral port, teardown
- `src-tauri/src/state.rs:11` — `AppState` shape
- `src-tauri/src/lib.rs:21-82` — 56 + 4 registered commands
- `src-tauri/src/commands.rs:151`, `:178`, `:243`, `:294` — `connect_database` and its three connect helpers
- `web-server/src/state.rs:22` — `AppState`, same shape minus terminals, plus `single_db`
- `web-server/src/handlers.rs:74` — string dispatch over 52 commands
- `tui-app/src/main.rs:7-8`, `tui-app/src/app.rs:57-60`, `:181`, `:350` — runtime handle pattern to copy for GTK; raw-URL connect to fix
- `AGENTS.md` — never hold a mutex across `.await`; `clippy::await_holding_lock`
- `docs/solutions/runtime-errors/ssh-tunnel-mutex-deadlock.md`
- `docs/plans/2026-04-12-002-feat-merged-entrypoints-plan.md` — concurrent desktop+web implication
- `docs/plans/2026-04-13-003-feat-query-history-panel-plan.md` — unimplemented; leave room

### New Files

- `service/Cargo.toml`, `service/src/lib.rs` — the `sqlator-service` crate
- `service/src/{connections,connect,ssh,docker,credentials,schema,portability,terminal_spec}.rs`
- `service/src/error.rs` — typed error with preserved `code`
- `service/tests/` — the pure-function tests written in step 1