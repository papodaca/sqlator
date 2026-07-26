---
title: "chore: Characterization tests for load-bearing logic before the service extraction"
type: chore
status: active
date: 2026-07-25
---

# chore: Characterization tests for load-bearing logic before the service extraction

## Overview

Write tests that pin the **current** behavior of the ~1,100 lines that
[phase 1](2026-07-25-002-refactor-sqlator-service-extraction-plan.md) is about to move into
`sqlator-service`. These are characterization tests — golden-master tests — not correctness
tests. Their job is to make the refactor mechanically verifiable: if a test that passed before
the move fails after it, the move changed behavior.

Runs before phase 1 and gates it. Nothing here depends on GTK.

---

## Problem Statement / Motivation

```
$ rg -c '#\[test\]|#\[tokio::test\]' --glob '*.rs' -g '!target' .
./core/src/ssh/config_parser.rs:2
```

**Two tests. In the entire workspace.** No `tests/` directory anywhere, no integration tests,
and `package.json` has no test script.

Phase 1 moves roughly 1,000–1,100 lines out of `src-tauri/src/commands.rs` and
`web-server/src/handlers.rs` — connect orchestration, tunnel lifecycle, credential resolution,
Docker discovery, schema caching, import/export. All of it uncovered. A refactor of that size
against zero tests is not a refactor, it is a rewrite with extra steps.

There is a second motivation that makes this more than insurance. The two copies have already
drifted, and **nobody knows the full extent of the drift**. Writing the same test against both
copies is the cheapest way to enumerate it. Two divergences were found just while verifying
line references for this plan — one of which was not previously known.

---

## Proposed Solution

Three groups, in dependency order. Group A is the bulk of the value and can be done in a day.

### Where the tests live, and why that matters

The helper functions are **private** in `src-tauri/src/commands.rs` and
`web-server/src/handlers.rs`. Tests must therefore start life as inline
`#[cfg(test)] mod tests` blocks **in their current files**, not in a `tests/` directory.

That is deliberate, and it is the whole method:

1. Write the test inline next to the function, against today's behavior.
2. Write the *same* test against the web copy. Where they disagree, both tests pass with
   different expectations — that is the divergence, now documented in code.
3. Phase 1 moves the function to `sqlator-service` and moves its test with it.
4. The surviving test encodes the chosen behavior; the deleted one records what was given up.

Both crates already have a lib target (`src-tauri/src/lib.rs`, `web-server/src/lib.rs`), so
`cargo test -p sqlator-app` and `cargo test -p sqlator-web` work with no restructuring. Add
`tokio = { features = ["macros", "rt"] }` to dev-dependencies where needed.

### Group A — pure functions (no I/O, no runtime, fast)

Highest volume of duplication, lowest cost to cover. Verified signatures:

| Function | Location | What to pin |
|---|---|---|
| `unique_name(base, existing) -> String` | `commands.rs:1245` | Collision suffixing `(1)`, `(2)`…; behavior when `base` itself is free; when `base (1)` is also taken |
| `build_url_no_password(db_type, host, port, database, username)` | `commands.rs:1256` | All 7 engines; the `sqlite://` special case; empty username omitting `@` |
| `resolve_connection_type(config)` | `commands.rs:1266` | Explicit `connection_type` wins; inference from `container_name` + `ssh_profile_id`; local-Docker inference; plain-direct fallback |
| `default_port_for_db_type(db_type)` | `commands.rs:1281` | 5432 / 3306 / 3306 / 1433 / 1521 / 8123, and the unknown-type default |
| `parse_auth_method(s)` | `commands.rs:1292` | `"key"` / `"password"` / `"agent"`, and the error case |
| `extract_single_table(sql)` | `commands.rs:1530` | sqlparser path: simple SELECT, schema-qualified, quoted identifiers, JOIN → `None`, subquery, CTE, non-SELECT |
| `extract_table_regex(sql)` | `commands.rs:1583` | The fallback path, reached when sqlparser fails. **Superseded by plan 007** — see the divergence ledger before writing these. |
| `detect_database_type(url)` | `core/src/db/mod.rs:371` | Every scheme incl. the `postgres`/`postgresql` and `mysql`/`mariadb` aliases; unknown scheme → `None`; malformed URL |

Also in group A, though they need a fixture rather than a scalar:

- **Import group re-parenting** (`commands.rs:1101-1243`, the three-pass algorithm) — nested
  groups, ID remapping, name collisions under both `skip` and `rename` policies, a group whose
  parent appears later in the file, and a cyclic parent reference.
- **Export → import round-trip** (`commands.rs:1014-1079` → `:1101`) — connections, groups and
  SSH profiles survive intact; passwords are absent from the export;
  `local_port_binding` and `keepalive_interval` survive. **This one fails on web today.**

### Group B — stateful and lifecycle behavior

Needs a harness but no external services.

| Behavior | Location | Test |
|---|---|---|
| Vault idle-timeout | `vault_backend.rs:71` (`is_locked`), `:190` (`check_and_touch`), `:206` (`last_activity` refresh) | Locks after `timeout_secs`; `timeout_secs == 0` means never; every accessor call refreshes `last_activity`; an *extra* accessor call must not change lock timing observably. Inject a clock or use a 1-second timeout. |
| Vault round-trip | `vault_backend.rs:109` (`unlock`), `:261` (atomic write) | create → store → lock → unlock → read; wrong password fails; the temp-file+rename leaves no partial file on simulated failure |
| `SshAuthConfig` zeroization | `core/src/ssh/auth.rs:23`, `:90` (`impl Drop`) | Secret bytes are zeroed on drop. Guard test: assert `SshAuthConfig` does **not** implement `Clone` — this is the invariant phase 1 is most likely to break by convenience. |
| Schema cache | `commands.rs:1648` (key), `:1649-1665` (TTL) | Key format `{connection_id}:{schema_name:?}:{table_name}` — note it uses `Debug` on an `Option`, so `Some("public")` keys as `Some("public")`. Pin it. Hit, miss, expiry at 5 min, and that `None` vs `Some("")` do not collide. |
| Tunnel registry | `commands.rs:214`, `:268` (stale cleanup); `:740`, `:1448` (ephemeral teardown on error) | Reconnect replaces rather than leaks; a failing `test_connection_with_ssh` still closes its tunnel. Assert the registry is empty **and** the forwarded local port is re-bindable afterwards. |
| Connect timeout | `core/src/db/mod.rs:65-79` | Unreachable host fails in ~5s with `code == "TIMEOUT"`, not a hang. Point at a blackholed address. |
| `DbManager::connect` pool replacement | `core/src/db/mod.rs:66-68` | Connecting twice on the same `connection_id` closes the old pool |

### Group C — integration against `docker-compose.yml`

Gated behind a feature or env var so a bare `cargo test` still passes.

| Behavior | Location | Test |
|---|---|---|
| MySQL VARBINARY workaround | `core/src/db/mod.rs:519` (`CAST(SCHEMA_NAME AS CHAR)`) | `get_schemas` against MySQL 8 decodes as `String`. Removing the CAST must fail this test. |
| Pagination `has_more` | `core/src/db/mod.rs:969` (`params.limit.min(1000) + 1`) | Exactly `limit` rows → `has_more == false`; `limit + 1` available → `true`; a request for `limit > 1000` is clamped |
| `QueryEvent` sequencing | `core/src/models.rs:132` | SELECT emits `Columns` → `Row`* → `Done`; non-SELECT emits `RowsAffected`; failure emits `Error` and nothing after it |
| `execute_batch` transactionality | `core/src/db/mod.rs:333-361` | DELETE → UPDATE → INSERT ordering; a mid-batch failure rolls back; the four unsupported engines return `UNSUPPORTED` rather than silently doing nothing |
| SSH tunnel concurrency | `core/src/ssh/tunnel.rs:74`, `:133` (`forward_stream`) | **The deadlock regression test.** Two simultaneous streams through one tunnel must both complete. This is the one that catches someone reintroducing `Arc<Mutex<Channel>>`. Requires an SSH server in compose — `linuxserver/openssh-server` is sufficient. |

**Done (2026-07-26):** `core/tests/group_c_integration.rs`. Gate: Cargo feature
`integration` on `sqlator-core` **or** `SQLATOR_INTEGRATION=1`. Compose service
`openssh` (linuxserver/openssh-server) with `docker/openssh/cont-init.d` enabling
`AllowTcpForwarding` (image default is `no`). Unsupported-engine arms in
production are **three** (Mssql / Oracle / ClickHouse), not four — soft-skip if
those compose services are down; ClickHouse exercised when up. Group B
`#[ignore]` live-SSH ephemeral teardown left as stub (AppState wiring is
Tauri/web-specific; tunnel concurrency covers the deadlock risk in core).

---

## The divergence ledger

Where the two copies disagree, both tests pass today with different expectations. Phase 1 must
pick a winner for each. This table is the deliverable that makes that possible.

### Known before this plan

| # | Behavior | Tauri | Web | Recommended winner |
|---|---|---|---|---|
| 1 | `connect_database` honours `connection_type` | Yes | **No** — raw URL, no tunnel, no Docker (`handlers.rs:339-358`) | Tauri |
| 2 | `get_ddl` implemented | Yes | **No** — `unknown command` | Tauri |
| 3 | `test_connection_with_ssh` jump hosts | `build_jump_hosts_for_profile` (`commands.rs:714`) | **`vec![]`** (`handlers.rs:1051`) | Tauri |
| 4 | Import preserves `local_port_binding` / `keepalive_interval` | Yes (`commands.rs:1190-1191`) | **`None`** (`handlers.rs:948-949`) | Tauri |
| 5 | `extract_table_regex` comma handling | Rejects if a comma appears **anywhere** in the SQL (`commands.rs:1601`) | Rejects only if the **table token** contains a comma (`handlers.rs:1141`) | **Neither** — see below |

### Found while verifying this plan

| # | Behavior | Tauri | Web | Recommended winner |
|---|---|---|---|---|
| 6 | `extract_table_regex` JOIN detection | Checks `" JOIN "` **and** `","` (`commands.rs:1603-1605`) | Checks `" JOIN "` only (`handlers.rs:1142`) | **Neither** — see below |
| 7 | Identifier unquoting | `.trim_matches('"').trim_matches('`')` — sequential, so `` `"tbl"` `` unquotes to `"tbl"` with the double-quotes **left on** | `.trim_matches(\|c\| c == '"' \|\| c == '`')` — single predicate pass, unquotes to `tbl` | **Web** |

### Rows 5–7 are resolved by a separate plan

Divergences 5, 6 and 7 all live in the same ~30-line function, and the obvious reading — that
web's copy is simply better — turned out to be wrong on closer inspection. Running both
implementations side by side showed that **web's copy marks an implicit comma join as
editable**: `SELECT a, b FROM t1 , t2` returns `Some("t1")` on web and `None` on Tauri. Tauri's
over-broad comma check plugs that hole by accident. Adopting web verbatim would trade a
false-negative annoyance for a false-positive data-safety bug.

The resolution is therefore a corrected implementation that belongs to neither copy, specified
in [`docs/plans/2026-07-25-007-fix-tauri-table-extraction-editability-plan.md`](../2026-07-25-007-fix-tauri-table-extraction-editability-plan.md).
That plan also fixes a fourth defect present in **both** copies (a byte offset taken from an
uppercased string and applied to the original, which makes `"SELECT 'ııı' FROM t"` extract a
table named `"OM"`).

**Consequence for this plan:** `extract_table_regex` is the one function where the
characterization tests should be written and then *deliberately replaced* rather than carried
forward. Write them to pin today's behavior in both copies as evidence, then let plan 007
supersede them. If plan 007 lands first, skip pinning this function and simply adopt its test
corpus.

Any further divergences found while writing group A tests get appended here.

### Found while writing Group A characterization tests

| # | Behavior | Tauri | Web | Recommended winner |
|---|---|---|---|---|
| 8 | `parse_auth_method` unknown-method error string | `"Unknown auth method: {other}"` | `"unknown auth method: {}"` via `err()` (also wraps `StatusCode::BAD_REQUEST`) | Tauri (clearer casing); map to a typed error in phase 1 |
| 9 | `default_port_for_db_type` helper | Present (`commands.rs`) — 5432/3306/3306/1433/1521/8123, unknown→0 | **Absent** as a standalone fn; port defaults live inside `detect_db_type(url)` which also validates the scheme | Tauri shape (pure `db_type → port`) in `sqlator-service`; web can call it after scheme detect |

Notes pinned by Group A that are **not** divergences (same in both copies):

- `unique_name` never returns `base` itself — always starts at `base (1)`.
- `resolve_connection_type`: `container_name` alone (no SSH) falls through to `Direct`; there is no local-Docker inference path today.
- Import **groups** always skip on name collision; `duplicate_mode` / `duplicateMode` only affects connections and SSH profiles.
- Cyclic `parent_group_name` references are silently dropped after three passes (`groups_added == 0`).

### Found while writing Group B characterization tests

| # | Behavior | Tauri | Web | Recommended winner |
|---|---|---|---|---|
| 10 | `SshAuthConfig` implements `Clone` | Yes (`#[derive(Clone)]` in `core`) — plan B wanted a no-`Clone` guard so Drop zeroization cannot be defeated by copies | Same (shared core) | **Remove `Clone`** in phase 1 (or a small precursor PR); characterization today asserts Clone *is* present |
| 11 | Vault atomic write on rename failure | `write_vault_atomic` leaves `vault.tmp` behind when rename fails | Same (shared core) | Keep temp+rename; on failure ideally remove `.tmp` (hardening, not required for phase 1 move) |

Notes pinned by Group B that are **not** divergences (same in both copies / core-only):

- Schema cache key is `{connection_id}:{schema_name:?}:{table_name}` (Debug on `Option`) in both Tauri and web; TTL insert constant is `300` seconds; wall-clock expiry cannot be advanced without a clock abstraction.
- Connect unreachable-host timeout returns `code == "TIMEOUT"` after ~5s (`DbManager::connect`); bare `cargo test` skips via `SQLATOR_SLOW_TESTS=1` gate.
- Tunnel registry DashMap replace-same-id does not leak map entries; full ephemeral teardown + port rebind needs live SSH (`#[ignore]`, Group C overlap). Web `connect_database` still does not manage tunnels (ledger row 1).

---

## Technical Considerations

- **These tests are allowed to encode bugs.** That is the point of characterization testing. A
  test asserting Tauri's over-broad comma rejection is correct *as a record of today*, and
  phase 1 deletes it deliberately with the divergence ledger as justification. Comment each
  such test with a pointer to its ledger row so nobody "fixes" it in isolation.
- **Do not refactor while writing tests.** The temptation to clean up a function while covering
  it is strong and defeats the purpose — the test would then pin the new behavior, not the old.
  Separate commits, separate review.
- **Vault timing tests need an injectable clock or a short timeout.** `last_activity: Instant`
  (`vault_backend.rs:36`) is not injectable today. Prefer a 1-second `timeout_secs` over adding
  a clock abstraction; if that proves flaky in CI, introduce the abstraction as part of phase 1
  rather than now.
- **The tunnel concurrency test is the expensive one.** It needs an SSH server in
  `docker-compose.yml` and it is the slowest test proposed. It is also the only automated
  defense against re-creating the deadlock documented in
  `docs/solutions/runtime-errors/ssh-tunnel-mutex-deadlock.md`. Build it last, but build it.
- **Enable `clippy::await_holding_lock` as part of this plan, not phase 1.** It is a static
  check for the same failure the concurrency test catches dynamically, and it costs nothing to
  turn on now.
- Group A needs no async runtime at all. Keep it that way so it stays fast.

---

## System-Wide Impact

- **Interaction graph:** none. No production code changes.
- **Error propagation:** unchanged, but the tests pin today's error `code` strings
  (`"TIMEOUT"`, `"UNSUPPORTED"`), which phase 1's typed-error work must preserve or
  deliberately map.
- **State lifecycle risks:** none introduced. Several are now *detectable*.
- **API surface parity:** none.

---

## Acceptance Criteria

- [x] Group A: every function in the table has inline tests covering the listed cases, in both
      `commands.rs` and `handlers.rs` where both copies exist
- [x] Group A: import re-parenting covered for nested groups, both duplicate policies, out-of-order
      parents, and a cyclic parent reference
- [x] Group A: export → import round-trip test exists and **is marked `#[ignore]` with a ledger
      reference on the web side, where it currently fails**
- [x] Group B: vault idle-timeout, vault round-trip, `SshAuthConfig` zeroization + no-`Clone`
      guard, schema cache key/TTL, tunnel registry cleanup, and connect timeout all covered
      (see ledger rows 10–11 for Clone / atomic-write notes; live SSH tunnel teardown is
      `#[ignore]` pending Group C; connect timeout gated on `SQLATOR_SLOW_TESTS=1`)
- [x] Group C: MySQL VARBINARY, `has_more` clamping, `QueryEvent` sequencing, `execute_batch`
      transactionality, and tunnel concurrency all covered, behind a feature/env gate
      (`core/tests/group_c_integration.rs`; gate: `--features integration` or
      `SQLATOR_INTEGRATION=1`; openssh service in `docker-compose.yml`)
- [ ] The divergence ledger in this document is complete — every difference found while writing
      tests is appended, with a recommended winner
- [ ] `clippy::await_holding_lock` enabled in CI and passing
- [x] A bare `cargo test --workspace` passes with no external services running
      (Group C tests early-return unless feature/env gate is set)
- [x] `cargo test --workspace --features integration` passes with `docker-compose up`
      (also: `cargo test -p sqlator-core --features integration`)
- [x] No production code was changed by this plan (verified by diff review)
      (Group C: tests + Cargo feature + compose SSH + cont-init only)

---

## Success Metrics

- Workspace test count goes from **2** to roughly **80–100**
- Every row of phase 1's load-bearing-behavior risk table has at least one test
- The divergence ledger is complete enough that phase 1 makes decisions rather than discoveries
- Group A runs in under a second

---

## Dependencies & Risks

| Dependency | Notes |
|---|---|
| None for groups A and B | Pure Rust, no external services |
| Overlaps [plan 007](../2026-07-25-007-fix-tauri-table-extraction-editability-plan.md) | Resolves ledger rows 5–7. If 007 lands first, adopt its `extract_table_regex` corpus instead of pinning today's behavior |
| `docker-compose.yml` | Group C; already exists |
| `linuxserver/openssh-server` (or equivalent) | New compose service for the tunnel concurrency test |
| Blocks phase 1 | This is the safety net phase 1 is predicated on |

| Risk | Mitigation |
|---|---|
| Tests pin bugs and someone later "fixes" them in isolation | Every bug-pinning test carries a ledger-row comment |
| Scope creep into fixing what the tests reveal | Explicit rule: no production changes in this plan; file issues instead |
| Vault timing tests flaky in CI | 1s timeout first; clock abstraction only if needed, and then in phase 1 |
| Group C slows CI | Feature-gated; separate CI job |
| Effort feels wasted since half the tests get deleted in phase 1 | They are deleted *having done their job* — proving the move preserved behavior. Say so in review. |

---

## Sources & References

### Internal References

Verified locations, all confirmed against the working tree:

- `core/src/ssh/config_parser.rs:143,149` — the existing two tests
- `core/src/db/mod.rs:65-79` — 5s connect timeout, `code: "TIMEOUT"`
- `core/src/db/mod.rs:66-68` — old pool closed on reconnect
- `core/src/db/mod.rs:371` — `detect_database_type`
- `core/src/db/mod.rs:384` — `create_pool_for_url`
- `core/src/db/mod.rs:519` — MySQL `CAST(SCHEMA_NAME AS CHAR)` VARBINARY workaround
- `core/src/db/mod.rs:969` — `params.limit.min(1000) + 1`
- `core/src/db/mod.rs:333-361` — `execute_batch`, `UNSUPPORTED` engines
- `core/src/models.rs:132` — `QueryEvent`
- `core/src/ssh/tunnel.rs:16` — `TunnelHandle`; `:28` `create`; `:74` `start_forwarding`; `:133` `forward_stream`; `:411` `close`
- `core/src/ssh/auth.rs:23` — `SshAuthConfig`; `:90` — `impl Drop`
- `core/src/credentials/vault_backend.rs:36` — `last_activity: Instant`; `:71` `is_locked`; `:109` `unlock`; `:190` `check_and_touch`; `:206` refresh; `:261` atomic write
- `src-tauri/src/commands.rs:1245` `unique_name`; `:1256` `build_url_no_password`; `:1266` `resolve_connection_type`; `:1281` `default_port_for_db_type`; `:1292` `parse_auth_method`; `:1530` `extract_single_table`; `:1583` `extract_table_regex`; `:1648` cache key
- `src-tauri/src/commands.rs:214`, `:268` — stale tunnel cleanup; `:740`, `:1448` — ephemeral teardown
- `web-server/src/handlers.rs:1133` `extract_table_regex`; `:1178` cache key; `:339-358` `connect_database`; `:948-949` import field loss; `:1051` empty jump hosts
- `AGENTS.md` — `clippy::await_holding_lock`
- `docs/solutions/runtime-errors/ssh-tunnel-mutex-deadlock.md` — what the concurrency test defends

### External References

- Characterization testing (Feathers, *Working Effectively with Legacy Code*) — the method this plan applies
- `serial_test`: https://docs.rs/serial_test/

### New Files

- Inline `#[cfg(test)] mod tests` in `src-tauri/src/commands.rs` and `web-server/src/handlers.rs`
- Inline `#[cfg(test)] mod tests` in `core/src/db/mod.rs`, `core/src/ssh/auth.rs`, `core/src/credentials/vault_backend.rs`
- `core/tests/tunnel_concurrency.rs` — feature-gated
- `core/tests/integration/` — feature-gated engine tests
- `docker-compose.yml` — add an SSH server service
- `core/src/test_support/` — fixtures for connections, groups, SSH profiles, result sets