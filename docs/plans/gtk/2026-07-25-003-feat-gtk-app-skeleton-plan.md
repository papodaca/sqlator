---
title: "feat: sqlator-gtk crate, build pipeline and application shell"
type: feat
status: completed
date: 2026-07-25
---

# feat: sqlator-gtk crate, build pipeline and application shell

## Overview

Stand up the `sqlator-gtk` workspace member: pinned crate versions, a `build.rs` that compiles
Blueprint and GResource, the tokio↔GLib bridge, the libadwaita application shell, GSettings,
and the action/accelerator map. The deliverable is an app that launches, remembers its
geometry, opens tabs, and can execute a query end-to-end against `AppService` — with no
feature depth behind any of it.

Feature parity is phase 3. This phase is the chassis.

---

## Problem Statement / Motivation

The deleted `sqlmator/` folder was an unmodified GNOME Builder template: gtk4 0.9 and
libadwaita 0.7 against installed 4.22 / 1.9, a `GtkShortcutsWindow` deprecated in GTK 4.18,
a commit-less nested `.git`, a GPL-3.0 `COPYING` against the workspace's MIT, a Flatpak
manifest pointing at `file:///home/usr1/Projects`, and no `sqlator-core` dependency. There was
nothing to migrate.

Starting fresh also lets the chassis be built around the two things that actually determine
whether this app is pleasant to work on: **how async work reaches the UI thread**, and
**how a query gets cancelled**. Both are easy to get subtly wrong and expensive to retrofit.

---

## Proposed Solution

### Crate and pinned versions

Every version below is pinned to the verified local environment. The feature graph was
resolved from crates.io metadata rather than guessed.

```toml
[package]
name = "sqlator-gtk"
version = "0.1.0"
edition = "2021"
rust-version = "1.92"

[dependencies]
sqlator-core    = { path = "../core" }
sqlator-service = { path = "../service" }

gtk        = { package = "gtk4",        version = "0.11.4", features = ["gnome_50"] }
adw        = { package = "libadwaita",  version = "0.9.2",  features = ["v1_9", "gtk_v4_22"] }
sourceview = { package = "sourceview5", version = "0.11.2", features = ["v5_18"] }
vte        = { package = "vte4",        version = "0.10.0", features = ["v0_84"], optional = true }

glib = "0.22.8"
gio  = "0.22.8"

tokio         = { version = "1", features = ["rt-multi-thread", "sync", "time", "macros"] }
tokio-util    = "0.7.19"
async-channel = "2.5"
futures-util  = "0.3"
tracing       = "0.1"

[build-dependencies]
glib-build-tools = "0.22.8"

[features]
default  = []
terminal = ["dep:vte"]
```

Notes on the feature choices:

- `gtk4`'s `gnome_50` expands to `["v4_22", "gio/v2_88", "gnome_49"]` — exactly right for GTK
  4.22.4 with glib 2.88.2. Do **not** use `v4_24`; it targets unreleased GTK.
- libadwaita needs **both** `v1_9` (API level) and `gtk_v4_22` (so adw's internal GTK gates
  line up).
- `sourceview5` has **no `v5_20` feature** — no new API landed in that release — and `v5_22`
  would require GtkSourceView 5.22. `v5_18` is the ceiling for the installed 5.20.0.
- `gtk4` 0.11.4 declares MSRV 1.92; local toolchain is 1.96.1.

Add `"sqlator-gtk"` to the workspace `members` in the root `Cargo.toml`.

### Deprecations to avoid from line one

On libadwaita 1.9 the entire pre-adaptive dialog generation is deprecated. None of these
should ever appear in this crate:

| Never use | Use instead |
|---|---|
| `gtk::Dialog`, `gtk::MessageDialog` | `adw::Dialog` / `adw::AlertDialog` |
| `adw::MessageDialog` (deprecated 1.6) | `adw::AlertDialog` |
| `adw::PreferencesWindow` | `adw::PreferencesDialog` |
| `adw::AboutWindow` | `adw::AboutDialog` |
| `gtk::FileChooserDialog` | `gtk::FileDialog` |
| `gtk::ShortcutsWindow` (deprecated GTK 4.18) | `adw::ShortcutsDialog` (libadwaita 1.8) |
| `gtk::TreeView` + `ListStore` + `CellRenderer*` | `gtk::ColumnView` + `gio::ListModel` |
| `gtk::ComboBoxText` | `gtk::DropDown` |

`adw_alert_dialog_new()` takes no parent — the parent goes to `present()` or `choose()`.

Styling: ship a **single `style.css`** using `@media (prefers-color-scheme: dark)` and
`@media (prefers-contrast: more)`. libadwaita 1.9 deprecated the `style-dark.css` /
`style-hc.css` convention and warns at startup. Use `.dimmed`, not the renamed-in-1.7
`.dim-label`. libadwaita's named colors are available as CSS variables
(`--accent-bg-color` and friends) for results-grid styling.

### The tokio ↔ GLib bridge

One process-wide runtime, and `spawn_future_local` awaiting the tokio `JoinHandle` directly.
A `JoinHandle` *is* a `Future`, so no oneshot channel is needed for request/response work —
polling it only touches the task's waker, not the tokio reactor.

```rust
// src/runtime.rs
pub static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("sqlator-io")
        .build()
        .expect("failed to build tokio runtime")
});

#[macro_export]
macro_rules! spawn_tokio {
    ($fut:expr) => { $crate::runtime::RUNTIME.spawn($fut) };
}
```

This is what Fractal does. There is no maintained tokio↔glib integration crate worth adding;
it is fifteen lines by hand.

**The `!Send` discipline, stated once:** everything crossing the runtime boundary is plain
owned Rust data; GObjects are created and touched only inside `spawn_future_local`. Because
`spawn_future_local` does not require `Send`, the *outer* future may freely hold widgets — only
the inner `RUNTIME.spawn(...)` argument must be `Send`. Never capture a widget in a future
handed to `RUNTIME.spawn`.

Two absolute rules: **never `Runtime::block_on` on the main thread** (freezes the UI and can
deadlock against `spawn_future_local` continuations), and use **`gio::spawn_blocking`** rather
than tokio for CPU-bound non-async work (CSV export, client-side sort of 200k rows) — it uses
GLib's own thread pool and returns an awaitable handle.

Reserve `async-channel` for genuine *streams*: `core`'s `QueryEvent` mpsc maps onto it
directly, as do progress ticks.

Note `glib::clone!` in glib 0.22 uses the **attribute form** (`#[weak]`, `#[strong]`,
`#[weak(rename_to = x)]`, `#[upgrade_or_default]`), not the pre-0.20 `@weak x =>` syntax that
most tutorials still show.

### Query cancellation needs three layers

The third is the one that gets missed. `CancellationToken` and dropping the future stop the
*client* waiting; they do not stop the *server* grinding through the query. **sqlx still has no
public query-cancellation API as of 0.9.0** — `PgConnection`'s own docs say an in-progress
stream "can only be canceled in two ways: by closing the connection, or by using another
connection to kill the server process."

1. `glib::JoinHandle::abort()` — drops the UI continuation.
2. `tokio_util::sync::CancellationToken` in a `tokio::select!` — stops the client task.
3. **Server-side cancel on a second connection**, per backend:
   - **PostgreSQL** — capture `SELECT pg_backend_pid()` at query start (or tag the session
     with a unique `application_name` and look it up in `pg_stat_activity`), then
     `SELECT pg_cancel_backend($1)` from another connection. Prefer `pg_cancel_backend`
     (cancels the statement, keeps the session) over `pg_terminate_backend`.
   - **MySQL/MariaDB** — capture `SELECT CONNECTION_ID()`, then `KILL QUERY <id>`.
   - **SQLite** — sqlx does not expose `sqlite3_interrupt`. Accept it, or cap with `LIMIT`.
   - **MSSQL (tiberius)** — cancellation is via `Attention` packets, not exposed. Closing the
     connection is the realistic lever.

**The cancel path must not compete for the last pooled connection**, or it deadlocks against
the query it is trying to cancel. Give it a dedicated connection or a reserved slot.

Also carry a monotonic generation counter per tab so a late result from a superseded run is
discarded rather than rendered.

### Build pipeline

`build.rs` does three things: Blueprint → `.ui` into `OUT_DIR`, GResource compile, and
GSettings schema compile.

```rust
// 1. blueprint-compiler batch-compile data/ui/*.blp -> OUT_DIR/ui/*.ui
// 2. glib_build_tools::compile_resources(&[OUT_DIR/ui, data/], gresource.xml, "sqlator.gresource")
// 3. glib-compile-schemas into OUT_DIR/schemas; export via cargo:rustc-env=SQLATOR_SCHEMA_DIR
```

`compile_resources` emits `cargo:rerun-if-changed` for the XML *and*, via a second
`--generate-dependencies` invocation, for every referenced file. **But** when `.ui` files are
generated into `OUT_DIR`, that dependency list points at `OUT_DIR` — so `build.rs` must emit
`rerun-if-changed` for the `.blp` sources itself.

`--sourcedir` order matters: `OUT_DIR/ui` first so generated `.ui` wins, then `data/` for CSS,
icons, and any hand-written `.ui`.

**Use Blueprint via `build.rs`, not the `gtk4/blueprint` cargo feature.** That feature only
supports `#[template(file = ...)]` and `string = ...`, explicitly **not** `resource = ...`.
Losing `resource=` would mean templates are outside the GResource bundle: no
`preprocess="xml-stripblanks"`, no shared bundle for icons and CSS, and a second code path for
anything `GtkBuilder` loads at runtime. Fail the build with a clear message if
`blueprint-compiler` is absent. Hand-written `.ui` files can be mixed in freely — files are
picked up from whichever `--sourcedir` provides them.

Avoid the `gtk-blueprint` crate (`include_blp!`) — 2022-era, unmaintained.

### GSettings — and the footgun

**`gio::Settings::new()` aborts the process** (it does not return an error) if the schema is
not installed, and a cargo-only build has no `make install` step to put one in
`/usr/share/glib-2.0/schemas/`. Hence step 3 of `build.rs`: compile the schema into
`OUT_DIR/schemas`, export the path, and set `GSETTINGS_SCHEMA_DIR` at the very top of `main()`
if unset — **before any `Settings` is constructed**. This makes `cargo run` work out of the
box while letting a packaged install override it.

Scope: **GSettings for UI/window state; connection profiles stay in `sqlator-core`'s config +
keyring.** Window geometry, sidebar visibility, paned position, font, theme, "confirm
destructive statements", and max-rows are exactly what GSettings is for — `Settings::bind()`
gives two-way property binding with no code. Connection profiles are structured and
secret-adjacent and already handled.

Initial schema (`im.apodaca.SqlatorGtk`): `window-width`, `window-height`, `window-maximized`,
`sidebar-visible`, `sidebar-width`, `editor-results-position`, `editor-font`,
`confirm-destructive`, `max-rows`.

GTK does not save window geometry for you; save it in `connect_close_request`.

Ignore GTK 4.22's session save/restore APIs (removed in `gtk4` 0.11.0, re-added in 0.11.2 with
`RestoreReason`) — they target compositor-driven session restore, sit in the `v4_24` window,
and are not a substitute.

### Application shell

```
AdwApplicationWindow
+- AdwBreakpoint (max-width: 800sp)
|    setter split_view.collapsed    = true
|    setter tab_bar.visible         = false
|    setter overview_button.visible = true
+- AdwOverlaySplitView
   +- sidebar: AdwToolbarView
   |    +- top: AdwHeaderBar (new connection, search toggle)
   |    +- content: GtkScrolledWindow
   |         +- GtkListView (.navigation-sidebar) over GtkTreeListModel
   +- content: AdwToolbarView (top-bar-style: raised)
        +- top: AdwHeaderBar (sidebar toggle, run, menu)
        +- top: AdwTabBar
        +- content: AdwTabView
             +- per tab: GtkPaned (vertical)
                  +- start: sourceview::View in GtkScrolledWindow
                  +- end:   AdwToastOverlay
                             +- GtkStack { results | messages | plan }
```

**`AdwOverlaySplitView`, not `AdwNavigationSplitView`.** They are identical when expanded and
differ when collapsed: `NavigationSplitView` makes the sidebar a *root page* and the content a
*subpage* ("choose a thing, then look at it" — right for a mail client), while
`OverlaySplitView` keeps content primary and overlays the sidebar ("a workspace with a panel I
toggle"). For a SQL client the editor is the workspace. `NavigationSplitView` would also fight
structurally, since it requires both children to be `AdwNavigationPage`s, which is awkward for
a tab container.

If a third pane is ever wanted (a right-hand object inspector), nest a second
`AdwOverlaySplitView` with `sidebar-position: end` inside the content.

**Sidebar: not `AdwSidebar`.** It is new and nice but flat-with-sections by design — Alice's
own post notes it "doesn't aim to support every single use case (sidebars can get very
complex, see e.g. GNOME Builder)". A database tree is hierarchical, so
`GtkListView` + `GtkTreeListModel` + `GtkTreeExpander` with `.navigation-sidebar`.
`GtkTreeListModel`'s `create_func` fires only on expand, which gives lazy catalog loading for
free. `AdwSidebar` remains a good fit for the *connections* list specifically, if connections
and schema are split into two levels — it brings search filtering, context menus and suffix
widgets for free.

Breakpoints use `sp`, not `px`, so they respect text scaling. Since libadwaita 1.6,
`AdwApplicationWindow` has a default 360x200 minimum, so breakpoints work without setting one.

### Tabs

`AdwTabView` + `AdwTabBar`, with `AdwTabOverview` + `AdwTabButton` for narrow widths. The
`AdwTabBar` goes in as a **second top bar** of the `AdwToolbarView` with
`top-bar-style: raised` — the docs call out `AdwTabView` as precisely the case where `raised`
is correct, since pages can have different backgrounds.

`TabPage` carries the per-tab state worth using: `title`, `tooltip`, `icon`, `loading` (wire to
the running flag — gives a spinner for free), `needs-attention` (flash when a background query
finishes on an unfocused tab), and `indicator-icon` (unsaved changes).

Intercept `close-page` for tabs with unsaved SQL or a running query: return
`Propagation::Stop`, show an `AdwAlertDialog`, then call `close_page_finish()`. That is the
correct async close protocol.

`AdwTabView` supplies `Ctrl+PageUp/Down`, `Ctrl+Tab` and `Alt+1..9` via its `shortcuts`
property.

### Actions and accelerators

Everything through `gio::SimpleAction` — menus, `.ui` `action-name`, and `AdwShortcutsDialog`
all speak it. Drive sensitivity via `SimpleAction::set_enabled`, never by poking buttons.

| Action | Accel |
|---|---|
| `app.quit` | `<Ctrl>q` |
| `app.preferences` | `<Ctrl>comma` |
| `app.new-connection` | `<Ctrl><Shift>n` |
| `win.toggle-sidebar` | `F9`, `<Ctrl>b` |
| `win.new-tab` | `<Ctrl>t` |
| `win.close-tab` | `<Ctrl>w` |
| `win.tab-overview` | `<Ctrl><Shift>o` |
| `tab.run` | `<Ctrl>Return` |
| `tab.run-all` | `<Ctrl><Shift>Return` |
| `tab.run-selection` | `<Ctrl><Alt>Return` |
| `tab.cancel` | `Escape` (enabled only while running) |
| `tab.format-sql` | `<Ctrl><Shift>f` |
| `tab.find` | `<Ctrl>f` |
| `tab.copy-as-csv` | `<Ctrl><Shift>c` |

Scope per-tab actions with `insert_action_group("tab", Some(&group))` on the tab widget so
`Ctrl+Return` reaches the focused tab.

---

## Technical Considerations

- **App ID `im.apodaca.SqlatorGtk`**, resource prefix `/im/apodaca/SqlatorGtk/`, gschema id
  matching. Chosen to avoid colliding with the Tauri build's `im.apodaca.sqlator`.
- **MIT**, matching the workspace. Dynamically linking LGPL GTK / libadwaita / VTE from an MIT
  application is fine.
- The GNOME Builder template's `config.rs.in` / meson `configure_file` pattern is not used;
  constants live in Rust and `build.rs`.
- No hot-reload for GResource-embedded templates — a `.ui` change triggers a Rust relink. If
  layout iteration becomes painful, add a `#[cfg(debug_assertions)]` + env-var path that loads
  `.ui` from disk via `gtk::Builder::from_file`.
- `relm4` 0.11.0 is a live, maintained option for an Elm-style architecture. Not recommended
  here: with custom list models and a custom completion provider this app drops to raw gtk-rs
  constantly, and the abstraction would mostly be in the way.

---

## System-Wide Impact

- **Interaction graph:** GTK signal → `spawn_future_local` → `RUNTIME.spawn(AppService::…)` →
  owned data returned → main-thread continuation updates GObjects. `QueryEvent` streams arrive
  over `async-channel` consumed by `spawn_future_local`.
- **Error propagation:** `sqlator-service`'s typed error is matched on the main thread. `code`
  drives the presentation choice — `AdwToast` for transient, `AdwAlertDialog` for actionable,
  `AdwBanner` for connection-level state.
- **State lifecycle risks:** the per-tab generation counter is load-bearing; without it a
  superseded query's late result overwrites a newer one. Tab close during a running query must
  cancel all three layers.
- **API surface parity:** none — this phase consumes `AppService` and adds no new service API.

---

## Acceptance Criteria

- [ ] `sqlator-gtk` builds via plain `cargo build` with no meson and no install step
- [ ] `cargo run` launches without a GSettings abort on a machine with no schema installed
- [ ] `build.rs` recompiles on `.blp`, `.css` and `.gschema.xml` changes
- [ ] Window geometry, maximized state and sidebar visibility persist across restarts
- [ ] Sidebar toggles with `F9`; the breakpoint collapses it below 800sp and shows the tab-overview button
- [ ] Tabs open, close, reorder, and show a spinner via `TabPage:loading` while a query runs
- [ ] Closing a tab with a running query prompts via `AdwAlertDialog` and honours the async close protocol
- [ ] A query executes end-to-end against `AppService` and streams `QueryEvent`s to a placeholder view
- [ ] `Escape` cancels a running query at all three layers, verified server-side (`pg_stat_activity` shows the backend cancelled)
- [ ] A superseded query's late result is discarded, not rendered
- [ ] Light/dark follows `AdwStyleManager` with a single `style.css`
- [ ] Zero deprecation warnings at startup; `GTK_DEBUG=builder` reports no template diagnostics
- [ ] No GTK object is captured into a `RUNTIME.spawn` future (verified by review)

---

## Success Metrics

- Cold `cargo run` to visible window under 500ms
- Query cancel takes effect server-side within ~200ms
- The UI never blocks: no frame longer than 16ms during connect, vault unlock, or config write

---

## Dependencies & Risks

| Dependency | Notes |
|---|---|
| [Phase 1 (`sqlator-service`)](2026-07-25-002-refactor-sqlator-service-extraction-plan.md) | Can be stubbed initially to unblock parallel work |
| [Phase 0b (ColumnView spike)](2026-07-25-001-spike-columnview-results-grid-plan.md) | Should pass before investing here |
| `blueprint-compiler` 0.22.2 | Build-time dependency; present locally and in the GNOME SDK; `build.rs` must fail clearly without it |

| Risk | Mitigation |
|---|---|
| `gio::Settings` aborting the process | `build.rs` + `GSETTINGS_SCHEMA_DIR` before any `Settings` construction; covered by an acceptance criterion |
| Blueprint line numbers not matching `.blp` on template errors | Accepted cost; `GTK_DEBUG=builder` and template smoke tests mitigate |
| Accidentally holding a widget across the runtime boundary | Stated discipline + review; the compiler catches most of it via `Send` |
| Cancel path deadlocking on a size-1 pool | Dedicated connection or reserved slot; acceptance criterion verifies server-side |

---

## Sources & References

### Internal References

- Runtime-handle pattern to adapt: `tui-app/src/main.rs:7-8`, `tui-app/src/app.rs:57-60`, `:181`
- `QueryEvent` stream: `core/src/models.rs:132`
- Blocking hazards to route through `spawn_blocking`: `core/src/config.rs:53-66`, `core/src/credentials/vault_backend.rs:215`
- Keyboard shortcut set to match: `src/routes/+layout.svelte` global handler

### External References

- gtk-rs book, Main Event Loop: https://gtk-rs.org/gtk4-rs/stable/latest/book/main_event_loop.html
- `glib::clone!` attribute syntax: https://docs.rs/glib/latest/glib/macro.clone.html
- Migrating to Adaptive Dialogs: https://gnome.pages.gitlab.gnome.org/libadwaita/doc/1-latest/migrating-to-adaptive-dialogs.html
- Adaptive Layouts: https://gnome.pages.gitlab.gnome.org/libadwaita/doc/1.5/adaptive-layouts.html
- libadwaita release notes 1.6 / 1.7 / 1.8 / 1.9: https://nyaa.place/blog/libadwaita-1-9/
- gtk4-rs 0.11.0 release notes: https://github.com/gtk-rs/gtk4-rs/releases/tag/0.11.0
- `CompositeTemplate` (blueprint feature does not support `resource=`): https://docs.rs/gtk4-macros/latest/gtk4_macros/derive.CompositeTemplate.html
- sqlx Postgres background-IO PR (cancellation still unshipped): https://github.com/launchbadge/sqlx/pull/3891
- Fractal (tokio bridge, `spawn_tokio!`): https://gitlab.gnome.org/World/fractal

### New Files

- `gtk/Cargo.toml`, `gtk/build.rs`
- `gtk/src/main.rs`, `gtk/src/application.rs`, `gtk/src/window.rs`, `gtk/src/runtime.rs`
- `gtk/src/query_tab/{mod.rs,imp.rs}`
- `gtk/data/ui/*.blp`
- `gtk/data/style.css`
- `gtk/data/sqlator.gresource.xml`
- `gtk/data/im.apodaca.SqlatorGtk.gschema.xml`