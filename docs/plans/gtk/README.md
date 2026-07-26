# GTK4 / libadwaita Native Frontend

A five-phase initiative adding a native GNOME frontend to SQLator, and extracting the
shared service layer it needs.

## Why this is more than "add a frontend"

SQLator has four frontends' worth of ambition and one frontend's worth of shared code.
`sqlator-core` provides drivers, pools, tunnels, credentials and Docker access — but the
*application* logic that sits above it (connect orchestration, tunnel lifecycle, credential
resolution, container discovery, schema caching, import/export) lives duplicated in
`src-tauri/src/commands.rs` and `web-server/src/handlers.rs`.

Roughly **1,000–1,100 of the 1,727 lines in `commands.rs` have a near-verbatim twin in
`handlers.rs`**, and the two copies have already drifted into user-visible bugs:

| Divergence | Consequence |
|---|---|
| Web's `connect_database` ignores `connection_type` entirely | SSH-tunneled and Docker connections silently fail or connect to the wrong host in browser mode |
| `get_ddl` unimplemented in web, but the shared Svelte UI calls it | The DDL viewer is broken in web mode |
| Web's `test_connection_with_ssh` hardcodes `vec![]` jump hosts | Proxy-jump profiles connect wrong |
| Tauri's regex fallback rejects SQL containing a comma; web's doesn't | Same query, different editability verdict — and *neither* copy is correct, see below |
| Web's import drops `local_port_binding` / `keepalive_interval` | Round-trip loses SSH profile settings |

The fourth row is the odd one out. On the other three, Tauri is simply right and web is simply
behind, so de-duplication fixes them by adopting Tauri's version. On table extraction both
copies are wrong in different directions — plus they share a fourth bug neither divergence
captured — so there is no version to adopt.
[Plan 007](../2026-07-25-007-fix-tauri-table-extraction-editability-plan.md) works out the
correct behavior separately; phase 1 moves *that* into the shared crate rather than picking a
side. It lives outside this folder because it is worth fixing whether or not the GTK frontend
ever ships.

`tui-app` took the opposite approach — it depends only on `sqlator-core` and duplicates
nothing — and the price is that **the TUI cannot use SSH tunnels, Docker connections, or the
credential store at all.**

Adding a fourth frontend without a shared service layer means picking one of those two
failure modes. So the extraction comes first, and the GTK app becomes its first consumer.

## Target architecture

```
sqlator-core      (unchanged)  drivers, pools, tunnels, credentials, docker, config I/O
      ^
sqlator-service   (NEW)        AppService: connect orchestration, tunnel registry,
                               credential resolution, docker discovery, schema cache,
                               import/export, typed errors
      ^
   +--+--------+--------------+-------------+
src-tauri   web-server     tui-app       sqlator-gtk (NEW)
 IPC glue   HTTP/WS glue   ratatui        GTK4/libadwaita
```

## Phases

| # | Plan | Type | Purpose |
|---|---|---|---|
| 0a | [`...-000-chore-characterization-tests-before-refactor-plan.md`](2026-07-25-000-chore-characterization-tests-before-refactor-plan.md) | chore | Pin the behavior of the untested load-bearing logic phase 1 is about to move |
| 0b | [`...-001-spike-columnview-results-grid-plan.md`](2026-07-25-001-spike-columnview-results-grid-plan.md) | spike | Prove `GtkColumnView` can be a database results grid before committing to the rest |
| 1 | [`...-002-refactor-sqlator-service-extraction-plan.md`](2026-07-25-002-refactor-sqlator-service-extraction-plan.md) | refactor | Extract `sqlator-service`; de-duplicate Tauri and web; close the five divergences above |
| 2 | [`...-003-feat-gtk-app-skeleton-plan.md`](2026-07-25-003-feat-gtk-app-skeleton-plan.md) | feat | `sqlator-gtk` crate, build pipeline, tokio/GLib bridge, application shell |
| 3 | [`...-004-feat-gtk-parity-buildout-plan.md`](2026-07-25-004-feat-gtk-parity-buildout-plan.md) | feat | Feature parity with the Svelte frontend, in three tiers |
| 4 | [`...-005-feat-gtk-packaging-flatpak-plan.md`](2026-07-25-005-feat-gtk-packaging-flatpak-plan.md) | feat | meson, Flatpak, portals, desktop integration, i18n |
| — | [`...-006-chore-gtk-testing-and-tooling-plan.md`](2026-07-25-006-chore-gtk-testing-and-tooling-plan.md) | chore | Cross-cutting: test tiers, headless CI, debugging tools, lints |

One related plan lives outside this folder:
[`2026-07-25-007-fix-tauri-table-extraction-editability-plan.md`](../2026-07-25-007-fix-tauri-table-extraction-editability-plan.md),
which fixes the table-extraction divergence. It is not a phase and does not gate anything —
either order works with phase 1 — but landing it first makes phase 1 a smaller move.

The two phase-0 plans are independent of each other and both gate what follows: **0a gates
phase 1** (do not refactor untested code) and **0b gates phase 2** (do not build a GTK app
whose central widget may not work). They can run in parallel. Phases 1 and 2 can then also run
in parallel; phase 3 depends on both. The testing plan (006) is not a phase — phases 2, 3 and 4
each contribute to it.

```
0a tests ──> 1 service ──┐
                         ├──> 3 parity ──> 4 packaging
0b spike ──> 2 skeleton ─┘
```

## Decisions taken

- **`sqlator-service`, not `sqlator-app`** — the `src-tauri` package is *already* named
  `sqlator-app`.
- **New crate `sqlator-gtk`, in the `gtk/` directory**, added to `workspace.members`. The
  `sqlmator/` folder (an unmodified GNOME Builder template
  targeting gtk4 0.9 / libadwaita 0.7, carrying its own commit-less `.git` and a GPL-3.0
  `COPYING`) has been deleted. Nothing was salvaged.
- **MIT**, matching the rest of the workspace. Dynamically linking LGPL GTK, libadwaita and
  VTE from an MIT-licensed application is fine.
- **App ID `im.apodaca.SqlatorGtk`**, to avoid colliding with the Tauri build's
  `im.apodaca.sqlator`.
- **Cargo-only build through phases 2–3.** meson and Flatpak arrive in phase 4, written fresh.
- **Tunnel registry key (Phase 1d interim):** `(ssh_profile_id, target_host, target_port)` with
  refcounting — not profile id alone. `TunnelHandle` is one local forward to one target; same
  bastion + different DB hosts must not share a single listener. Follow-up (shared session,
  multiple forwards): [`2026-07-26-001-design-ssh-tunnel-registry-keying.md`](2026-07-26-001-design-ssh-tunnel-registry-keying.md)
  / `sqlator-qa2`.

## Verified environment

Checked on the development machine, and every crate version in these plans is pinned to match:

| Component | Version |
|---|---|
| GTK | 4.22.4 |
| libadwaita | 1.9.2 |
| GtkSourceView | 5.20.0 |
| VTE (`vte-2.91-gtk4`) | 0.84.0 |
| glib | 2.88.2 |
| blueprint-compiler | 0.22.2 |
| rustc / cargo | 1.96.1 |

## Standing risk

**There are two tests in the entire Rust workspace**, both in
`core/src/ssh/config_parser.rs:143,149`. Everything phase 1 moves is untested. That single fact
is why phase 0a exists and why it gates phase 1 rather than running alongside it.