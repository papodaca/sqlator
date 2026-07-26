---
title: "feat: GTK frontend feature parity with the Svelte frontend"
type: feat
status: active
date: 2026-07-25
---

# feat: GTK frontend feature parity with the Svelte frontend

## Overview

Build the GTK frontend's feature surface on top of the phase 2 chassis, in three tiers:
a usable app, then parity with the Svelte frontend, then the things GTK does better than a
webview. Includes a set of capabilities that come nearly free in GTK and that the Svelte
frontend never got.

---

## Problem Statement / Motivation

The Svelte frontend is ~45 components across `src/lib/components/`, and "parity" is a large
and under-specified word. This plan enumerates the actual surface so scope is explicit and
tiers can be shipped independently.

It also records what *not* to port. Four pieces of the Svelte frontend are dead code, one is a
latent bug worth not reproducing, and six registered backend commands have no caller.

---

## Proposed Solution

### Tier 1 — a usable app

The minimum that makes the GTK build worth opening.

| Area | Scope |
|---|---|
| Connection list | Sidebar list with groups, colors, connect/disconnect. Source: `Sidebar.svelte` (595), `ConnectionItem.svelte` (263), `GroupItem.svelte` (440) |
| SQL editor | `sourceview::View`, `sql` language, dialect selection, `Adwaita`/`Adwaita-dark` schemes following `AdwStyleManager` |
| Query execution | Streaming `QueryEvent` → results, with the 50ms row-batch flush the Svelte frontend uses |
| Results grid | The phase 0 design: custom `ListModel`, `GtkInscription` cells, runtime columns, row selection |
| Schema tree | `GtkTreeListModel` lazy load: schemas → tables/views → columns |
| Tabs | Connection tabs and per-connection query tabs |
| Theming | Light/dark/system via `AdwStyleManager` |

**Editor setup.** `sourceview::Buffer` with `highlight_syntax`, `highlight_matching_brackets`,
`enable_undo`; `View` with `monospace`, `show_line_numbers`, `highlight_current_line`,
`auto_indent`, `smart_backspace`, `insert_spaces_instead_of_tabs`, `tab_width(2)`,
`show_right_margin`, `right_margin_position(100)`.

GtkSourceView ships one generic `sql.lang` — there is no per-dialect spec, so dialect-specific
keywords (`ILIKE`, ClickHouse syntax) are handled generically. This matches the Svelte
frontend, which silently falls back to the PostgreSQL CodeMirror dialect for `mssql`, `oracle`
and `clickhouse` anyway.

**Style scheme switching** must be done manually — GtkSourceView is not libadwaita-aware:

```rust
fn style_scheme() -> Option<sourceview::StyleScheme> {
    let name = if adw::StyleManager::default().is_dark() { "Adwaita-dark" } else { "Adwaita" };
    sourceview::StyleSchemeManager::default().scheme(name)
}
// re-apply on adw::StyleManager::default().connect_dark_notify(...)
```

Use `Adwaita`/`Adwaita-dark`, not the `builder-dark` that many snippets show — those are the
ones matching the libadwaita palette.

### Tier 2 — parity with Svelte

| Area | Scope | Svelte source |
|---|---|---|
| Connection form | Quick-URL ⇄ Advanced-fields bidirectional sync, name, color (10 swatches), group, SSL mode, SSH profile selector, Test button | `ConnectionForm.svelte` (877) |
| SSH profiles | Named reusable profiles: host, port, user, auth tabs (key file / password / agent), identity path, passphrase, advanced (local port binding, keepalive) | `SshProfileForm.svelte` (468) |
| SSH host picker | Dropdown from parsed `~/.ssh/config`, auto-fills hostname/port/user/identity | `SshHostDropdown.svelte` (303) |
| Docker wizard | 4 steps: SSH profile → container → credentials → test | `DockerConnectionWizard.svelte` (924) |
| Editable results | Change tracking, per-type cell editors, SQL preview before apply, transactional batch | `ResultGrid.svelte` (617), `GridToolbar.svelte`, `SqlPreviewModal.svelte`, `editors/` |
| Table browser | Server-side sort + filter with per-type operators, paged append | `EnhancedGrid.svelte` (572) |
| DDL viewer | Read-only source view with refresh + copy | `SchemaDdlViewer.svelte` (199) |
| Session persistence | `save_tab_state` / `get_tab_state`, debounced 500ms, reconnect on restore | `+layout.svelte` |
| Groups | Nested folders, drag-and-drop, 9-color palette, collapse state | `GroupItem.svelte` (440) |
| Import/export | Export to file with a toast + Open action; import with preview and skip-vs-rename | `ImportDialog.svelte` (395) |
| Vault | Unlock prompt gating the app when locked | `VaultUnlockPrompt.svelte` (172) |

**Port the Docker error classifier.** `DockerConnectionWizard.svelte:243-327` buckets failures
into `permission_denied`, `daemon_unreachable`, `network_isolated`, `not_found`, `stopped`,
`timeout`, `invalid_name`, `ssh`, `other`, each with a copy-pasteable remediation hint
(`sudo usermod -aG docker <user>`, `docker network connect bridge <container>`) and a
transient/permanent flag. A second classifier handles connection-test failures (TLS vs SSH vs
auth vs reachability). This is some of the best code in the frontend and belongs in
`sqlator-service` so all four frontends get it.

Note the wizard's MySQL/MariaDB default of `sslmode=disable` is deliberate — their older TLS
ciphers break rustls, and the SSH tunnel already secures the link.

**Editable results details.** After a successful SELECT, `fetch_schema_metadata` returns a
`TableMeta` (columns, types, nullability, auto-increment, generated, updatable, enum values,
primary key, `isEditable` + `editabilityReason`). Cells become editable only when the result
maps to a single editable table with a PK; otherwise show a read-only badge with the reason.
Row states: added green, deleted red + strikethrough, modified amber. Save generates
parameterized SQL client-side, previews it, then runs `execute_batch` ordered
DELETE → UPDATE → INSERT with `useTransaction: true`.

Given the phase 0 findings on `GtkEditableLabel`, consider editing through a **row-detail pane
or `AdwDialog` form** instead of in-cell editors. For a SQL client that also gives somewhere
sane to show `NULL` vs `''`, large text, and JSON.

**Dialog choices.** `AdwPreferencesDialog` with `AdwPreferencesPage`/`Group`/`SwitchRow`/
`ComboRow`/`SpinRow` for settings (libadwaita 1.8 added
`adw_preferences_group_bind_model()`, handy for per-connection defaults).
`AdwToastOverlay` per tab for "24 rows affected in 312 ms". `AdwBanner` for connection-level
state ("connected via SSH tunnel", "read-only replica") — neutral rather than accent-colored
since 1.7. `AdwSpinner` (1.6), not `GtkSpinner`.

### Tier 3 — what GTK does better

**VTE terminal instead of xterm.js.** `vte4` 0.10.0 is current, lives in GNOME's
`World/Rust/vte4-rs`, and its `v0_84` matches the installed VTE 0.84.0.

| | xterm.js (today) | vte4 |
|---|---|---|
| Runtime cost | WebView + JS engine per terminal | Native widget, GSK-rendered |
| PTY | App manages pty and pumps bytes over IPC | `VtePty` + `spawn_async` forks properly |
| Integration | Separate font/theme/selection/IME stack | GTK fonts, clipboard, IME, a11y |
| Scrollback, reflow, OSC 8, sixel | JS implementation | Same C implementation as GNOME Terminal / Ptyxis |
| Portability | Anywhere the webview goes | **Linux/BSD only** |

That last row is the trade-off; gate behind the `terminal` cargo feature.

Two rules: **reuse the existing SSH tunnel** — point `PGHOST`/`PGPORT` at the local end of the
tunnel `sqlator-core::ssh` already established rather than letting `psql` open its own SSH, so
there is one auth path. And **do not put passwords in argv** — argv is world-readable in
`/proc`. `PGPASSWORD` in the env vector is acceptable; a temporary `0600` `.pgpass` is better.
For MySQL use `MYSQL_PWD` or `--defaults-extra-file=`.

Detect the client binary at runtime and disable the action with a clear message rather than
spawning and failing. Style via `set_colors` from the libadwaita palette, re-applied on
`connect_dark_notify`.

Worth considering as an alternative or complement: much of what people use `psql` for in a GUI
(`\dt`, `\d table`, `\timing`) can be offered natively via `sqlator-service` — no PTY, no
external binary, identical on every platform. Embed VTE as the escape hatch, not the primary
interface.

### Free wins the Svelte frontend never got

These are cheap in GTK and are genuine product improvements, not parity.

- **Schema-aware autocompletion.** Svelte calls `sql({ dialect })` without the `schema` option,
  so completions are dialect keywords only — despite `schemaStore` already holding full table
  and column metadata. Implement a `CompletionProvider` subclass overriding `populate_future`,
  `display`, `activate`, and `is_trigger` (fire on `.`).

  **`populate_future` must be fast — never run a catalog query inside it.** Load the schema
  once on connect into an `Arc<SchemaSnapshot>`, hand it to the provider on the main thread,
  and have `populate_future` do nothing but an in-memory prefix match. Add
  `CompletionWords` and `CompletionSnippets` alongside it.
- **Snippets** via `SnippetManager` + `CompletionSnippets`: `sel`, `ins`, `upd`, `cte`, `expl`
  shipped in the GResource. Snippets can also be generated dynamically — e.g. a `SELECT`
  listing every column of the table under the cursor.
- **Vim mode** via `sourceview::VimIMContext` — essentially one line, and DB power users
  reliably want it.
- **Find/replace** via `SearchSettings` + `SearchContext` in a `GtkSearchBar` on `Ctrl+F`, with
  `occurrences_count()` driving a "3 of 17" label and `replace_all` as a single call.
- **Destructive-statement guard**: detect `DELETE`/`UPDATE` without `WHERE`, `DROP`, `TRUNCATE`
  and confirm via `AdwAlertDialog` with `ResponseAppearance::Destructive`, gated on the
  `confirm-destructive` GSetting. Use `choose_future()` to `.await` cleanly inside
  `spawn_future_local`.
- **Inline diagnostics** via `Annotation`/`AnnotationProvider` (new in `v5_18`) — a natural home
  for "syntax error at position 42" returned by the server.
- **Minimap** via `sourceview5::Map`.
- **Run statement under cursor / run selection.** The Svelte frontend has neither — it always
  sends the whole buffer as one string. The accelerators are already reserved in phase 2.

### Do not port

| Item | Why |
|---|---|
| `StorageSettings.svelte` (436 lines) | Complete credential-storage UI, **never imported anywhere**. Build the equivalent properly in `AdwPreferencesDialog` instead of transliterating dead code. |
| `services/validator.ts` | Never called |
| `stores/query.svelte.ts` (103 lines) | Superseded by `tabs.executeQuery`; duplicates its streaming logic |
| `editors/TextAreaEditor` | Imported by `ResultGrid`, never rendered |
| Global singleton edit store | `stores/edit.svelte.ts` is scoped to whichever query tab last executed, not per-tab. A latent bug. Make it per-tab in GTK. |

Six registered backend commands have no frontend caller and need no GTK equivalent unless
wanted deliberately: `get_query`, `save_query`, `create_ssh_tunnel`, `close_ssh_tunnel`,
`get_active_tunnels`, `parse_connection_url`.

### Operation surface

The GTK app needs `AppService` equivalents of these, grouped as the Svelte adapter groups them.
(These are today's Tauri command names; after phase 1 they are `AppService` methods.)

- **Connections** — `get_connections`, `save_connection`, `update_connection`,
  `clone_connection`, `delete_connection`, `test_connection`, `test_connection_with_ssh`,
  `connect_database`, `disconnect_database`
- **Query** — `execute_query` (streaming), `execute_batch`, `fetch_schema_metadata`
- **Schema** — `get_schemas`, `get_tables`, `get_columns`, `query_table`, `get_ddl`
- **Groups** — `get_groups`, `save_group`, `update_group`, `delete_group`,
  `move_connection_to_group`
- **SSH** — `list_ssh_hosts`, `get_ssh_profiles`, `save_ssh_profile`, `update_ssh_profile`,
  `delete_ssh_profile`, `connections_using_ssh_profile`
- **Credentials** — `check_keyring_available`, `get_storage_mode`, `set_storage_mode`,
  `vault_exists`, `is_vault_locked`, `create_vault`, `unlock_vault`, `lock_vault`,
  `get_vault_settings`, `save_vault_settings`
- **Docker** — `discover_container`, `list_running_containers`, `test_docker_connection`,
  `discover_local_container`, `list_local_containers`, `test_local_docker_connection`
- **Persistence** — `get_tab_state`, `save_tab_state`, `get_theme`, `save_theme`,
  `export_connections`, `import_connections`
- **Terminal** (tier 3, `terminal_spec` from phase 1) — CLI argv/env construction

Theme is the one to *not* route through the service: GTK should use its GSetting +
`AdwStyleManager`, not `get_theme`/`save_theme`.

---

## Technical Considerations

- **Batch `items_changed`.** The Svelte frontend buffers streamed rows and flushes on a 50ms
  interval to avoid per-row reactivity churn. Do the same — each `items_changed` emission makes
  the view re-examine its window, so emit per few thousand rows, not per row.
- **Row cap.** Both frontends cap display at 1000 rows (`core/src/db/mod.rs:969` computes
  `has_more` via `limit.min(1000) + 1`). The GTK `max-rows` GSetting defaults to 50,000; keep
  the service-side cap and the display cap distinct and both visible in the UI.
- **Sorting honesty.** Client-side sort is correct only when the full result is in memory. When
  the result is truncated or paged, sorting the page is a lie — intercept the header click,
  re-issue with `ORDER BY`, and do not attach a `GtkSorter`. Make the distinction visible
  ("showing first 50,000 rows — sorting applies to the server"). Same for filtering:
  `FilterListModel` + `set_incremental(true)` for in-memory quick-find, re-query with `WHERE`
  for anything authoritative.
- **Editability gating.** Only enable editing when the row can be identified (primary key or
  `ctid`/`ROWID` in the result); otherwise a safe `UPDATE` cannot be generated.
- **Engines.** 7 supported: `postgres`, `mysql`, `mariadb`, `sqlite`, `mssql`,
  `oracle` (experimental), `clickhouse`. Default ports 5432 / 3306 / 3306 / — / 1433 / 1521 /
  8123. SQLite hides host/port/user/password and relabels Database as "File path".
- **Proxy jump** is modelled (`SshProfile.proxy_jump: SshJumpHost[]`) and backend-supported, but
  `SshProfileForm.svelte` hardcodes `proxy_jump: []`. GTK can expose it — it is UI-only work.
- **DB passwords are stored in plaintext** inside the connection URL in `connections.json`
  (`core/src/models.rs:101` masks only on the way out). Only SSH secrets go to keyring/vault.
  Do not let the GTK UI imply otherwise; consider filing this as a separate hardening issue.

---

## System-Wide Impact

- **Interaction graph:** each UI action maps to one `AppService` call via `spawn_future_local`.
  The schema tree's `GtkTreeListModel::create_func` triggers lazy `get_columns` calls; results
  populate a fresh `gio::ListStore` per expanded node.
- **Error propagation:** the typed error's `code` selects presentation — `AdwToast` for
  transient, `AdwAlertDialog` for actionable, `AdwBanner` for connection state, inline for
  form validation. The Docker classifier drives remediation hints.
- **State lifecycle risks:** per-tab edit state (fixing the Svelte singleton bug); tab close
  with pending unsaved grid changes must prompt; schema cache invalidation on schema switch and
  refresh.
- **API surface parity:** if the Docker error classifier moves into `sqlator-service` as
  recommended, Tauri and web gain it too — a small behavior improvement to shipped paths.

---

## Acceptance Criteria

### Tier 1
- [ ] Connect to, and run a query against, each of the 7 supported engines
- [ ] Schema tree lazily loads schemas → tables/views → columns, with PK/FK indicators
- [ ] Results render 50k rows without UI stall; `NULL` is visually distinct
- [ ] Editor highlights SQL and follows light/dark
- [ ] Connection and query tabs open, close and switch

### Tier 2
- [ ] Connection form round-trips quick-URL ⇄ fields without data loss, for all 7 engines
- [ ] SSH profile CRUD works, including agent auth and the `~/.ssh/config` host picker
- [ ] A tunneled connection opens, and the tunnel is torn down on disconnect
- [ ] Docker wizard completes for both local and remote containers, with classified errors and remediation hints
- [ ] Grid edits generate correct parameterized SQL, preview it, and apply transactionally
- [ ] Table browser sorts and filters server-side with per-type operators
- [ ] Session restores open tabs and reconnects on launch
- [ ] Import/export round-trips connections, groups and SSH profiles without loss
- [ ] Vault lock/unlock gates the app and does not block the UI thread during Argon2id

### Tier 3
- [ ] VTE terminal launches the right CLI per engine, reusing the existing tunnel, with no password in argv
- [ ] Schema-aware completion suggests tables and columns, and qualifies on `.`
- [ ] Snippets, vim mode, find/replace, and the destructive-statement guard all work
- [ ] Run-selection and run-statement-under-cursor behave correctly with multiple statements

---

## Success Metrics

- Feature parity checklist complete for tiers 1 and 2
- Schema-aware completion responds within one frame (in-memory only)
- No UI stall over 16ms during any tier-2 operation

---

## Dependencies & Risks

| Dependency | Notes |
|---|---|
| Phase 1 (`sqlator-service`) | Hard dependency — this phase is almost entirely service consumption |
| Phase 2 (skeleton) | Hard dependency |
| Phase 0 findings | Determine whether in-cell editing or a row-detail pane is used |
| `vte4` 0.10.0 | Tier 3 only, feature-gated, Linux/BSD only |
| Native CLI binaries | User-installed; detect and disable with a clear message |

| Risk | Mitigation |
|---|---|
| Scope — ~45 Svelte components | Strict tiering; tier 1 ships independently |
| In-cell editing feels wrong on GTK | Row-detail pane fallback, decided by phase 0 |
| Sorting a truncated result silently misleads | Explicit UI distinction; acceptance criterion |
| Docker paths break under Flatpak | Deferred to phase 4, which owns the sandbox story |

---

## Sources & References

### Internal References

- Full Svelte component set: `src/lib/components/` (45 files)
- Adapter contract and operation names: `src/lib/api/adapter.ts`, `tauri-adapter.ts`, `web-adapter.ts`
- Streaming protocol: `columns` / `row` / `done` / `rowsAffected` / `error`; 50ms flush in `src/lib/stores/tabs.svelte.ts` (520)
- Docker error classifier to port: `src/lib/components/DockerConnectionWizard.svelte:243-327`
- Editability metadata: `core/src/db/mod.rs:178` (`fetch_schema_metadata`)
- Row cap: `core/src/db/mod.rs:969`
- Plaintext DB password caveat: `core/src/models.rs:101`
- Dead code not to port: `StorageSettings.svelte`, `services/validator.ts`, `stores/query.svelte.ts`, `editors/TextAreaEditor.svelte`
- Per-tab edit state bug: `src/lib/stores/edit.svelte.ts` (218)
- Related unimplemented plans worth aligning with: `docs/plans/2026-04-14-002-feat-schema-aware-sql-autocomplete-plan.md`, `2026-04-14-001-feat-export-results-plan.md`, `2026-04-13-003-feat-query-history-panel-plan.md`

### External References

- GtkSourceView 5 completion providers: https://gnome.pages.gitlab.gnome.org/gtksourceview/
- Fractal `src/utils/sourceview.rs` — the canonical Adwaita style-scheme switcher
- `vte4-rs`: https://gitlab.gnome.org/World/Rust/vte4-rs
- Mission Center (ColumnView + blueprint + per-row context menus): https://gitlab.com/mission-center-devs/mission-center

### New Files

- `gtk/src/connection/{form,list,group}.rs`
- `gtk/src/ssh/{profile_form,host_picker}.rs`
- `gtk/src/docker/wizard.rs`
- `gtk/src/results/{model,grid,editors}.rs`
- `gtk/src/schema/{tree,ddl}.rs`
- `gtk/src/editor/{completion,snippets,search}.rs`
- `gtk/src/terminal.rs` (feature-gated)
- `gtk/src/preferences.rs`
- `gtk/data/ui/*.blp` for each of the above
- `gtk/data/snippets/sql.snippets`
