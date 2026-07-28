---
title: "chore: Testing strategy and dev tooling for the GTK frontend"
type: chore
status: active
date: 2026-07-25
---

# chore: Testing strategy and dev tooling for the GTK frontend

## Overview

A cross-cutting plan that applies from phase 2 onward: what to test in a GTK app, what not to
bother testing, how to run any of it headlessly in CI, and which debugging tools to reach for.

Written separately because it is not a phase — it is a standing practice that phases 2, 3 and 4
each contribute to.

---

## Problem Statement / Motivation

**There are two tests in the entire Rust workspace**, both in
`core/src/ssh/config_parser.rs:143,149`. There is no `tests/` directory anywhere, no
integration tests, and `package.json` has no test script, so there is no JS-side coverage
either.

That is the context in which phase 1 moves ~1,100 lines of untested logic and phases 2–3 add a
new frontend. Correctness has to come from somewhere, and GTK adds two failure modes that
ordinary Rust testing does not catch:

1. **Template mismatches are runtime errors, not compile errors.** A `.blp` referencing a
   widget id or parent type that does not match the Rust struct compiles fine and blows up on
   first construction.
2. **GObject leaks are silent.** A leaked `RowObject` per row across many query runs is a
   realistic outcome of the results-grid design and nothing will tell you.

---

## Proposed Solution

### Put the testable logic where it does not need a display

`sqlator-core` and `sqlator-service` hold connection management, SQL execution, SSH tunneling,
credentials and Docker discovery. All of it is testable with ordinary `#[tokio::test]` against
the `docker-compose.yml` services that already exist. The GTK crate should stay thin enough
that its untested surface is mostly glue.

This is the highest-leverage decision in the whole plan: **the more logic phase 1 moves out of
the frontends, the less there is that can only be tested through a display server.**

### Tier 1 — GObject model tests (write these)

Custom `ListModel`, `RowObject`, sorters, filters, and value formatting are pure logic wearing
a GObject costume. They need `gtk::init()` but no window.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn init() { let _ = gtk::init(); }

    #[test]
    #[serial]
    fn result_model_reports_row_count_and_items() {
        init();
        let model = ResultModel::new();
        model.set_result(Arc::new(fixture_result(1_000)));
        assert_eq!(model.n_items(), 1_000);
        let row = model.item(999).and_downcast::<RowObject>().unwrap();
        assert_eq!(row.value(0), Value::Int(999));
    }
}
```

Cover at minimum: row count and item retrieval, the weak-ref cache not returning stale objects,
typed sorting (integers must not sort as `1, 10, 2`), `NULL` vs empty-string formatting, and
`items_changed` emission counts during batched streaming.

### Tier 2 — widget smoke tests (high value per line)

Instantiate each composite template once and assert it constructs and its `#[template_child]`s
resolve. This catches failure mode 1 above, which is the single most common GTK4-Rust bug. A
handful of these costs almost nothing and pays repeatedly.

Add one per `.blp` as phase 3 creates them.

### Tier 3 — full interaction tests (skip)

Generally not worth it. **Do not use the `gtk-test` crate** — it is GTK3-era, depends on
`libxdo`, and has not tracked GTK4.

For anything resembling end-to-end coverage, drive the app through its `GAction`s
(`app.activate_action("win.new-tab", None)`) and assert on model state rather than synthesizing
input events. This is why phase 2 routes everything through `gio::SimpleAction` — it doubles as
the test surface.

### Running headless

Both a display and a session bus are needed:

```bash
GDK_BACKEND=x11 GTK_A11Y=none \
  xvfb-run -a dbus-run-session -- cargo test --workspace --locked -- --test-threads=1
```

- `--test-threads=1` — GTK is single-threaded and `gtk::init()` is per-thread; parallel tests
  fight. `serial_test` on individual tests is the finer-grained alternative when only some
  tests need GTK.
- `GTK_A11Y=none` — disables the AT-SPI bridge, which otherwise hangs or spews warnings without
  a session.
- `dbus-run-session` — `AdwStyleManager` and `gio::Settings`' dconf backend want a bus; without
  it you get portal timeouts.
- `GSETTINGS_SCHEMA_DIR` must be set, or anything constructing a `gio::Settings` **aborts the
  process**. Phase 2's `build.rs` already exports it; make sure the test harness picks it up.

gtk4-rs's own CI uses `xvfb-run --auto-servernum cargo test`. GTK upstream also offers a
Wayland path (`mutter --headless --wayland --no-x11 --virtual-monitor 1024x768`) and Broadway
(`gtk4-broadwayd`); Xvfb is the least fussy, Broadway is useful mainly for *looking* at the app
in a browser from a remote machine.

### Dev tooling

| Tool | Use |
|---|---|
| `GTK_DEBUG=interactive` | The GTK Inspector — live widget tree, CSS editor with instant reload, property editing, a11y tree, render-node recorder, statistics. Or enable `Ctrl+Shift+D` permanently: `gsettings set org.gtk.gtk4.Settings.Debug enable-inspector-keybinding true` |
| `Ctrl+Shift+M` | libadwaita adaptive preview (1.7+), inside the inspector. Simulates phone/tablet form factors. Use on every layout change. |
| `GTK_DEBUG=builder` | Template/`.ui` diagnostics. libadwaita 1.9 supports it across all its widgets. Run with it on at least once after writing any new template. |
| `GTK_DEBUG=actions` | Logs action activation and enablement — the fastest way to find out why an accelerator "does nothing". |
| `GOBJECT_DEBUG=instance-count` + inspector statistics | Finds GObject leaks. Directly relevant to failure mode 2. |
| `GSK_RENDERER=cairo\|ngl\|vulkan`, `GTK_DEBUG=no-cache` | Diagnosing grid rendering oddities |
| **Sysprof** | The GNOME profiler. GTK emits frame marks so you can see where the main loop is blocked. **Use this before optimizing the grid** — it is how the ColumnView `GtkShortcutController` cost was identified upstream. |
| `RUST_LOG` | e.g. `RUST_LOG=sqlator_gtk=debug,sqlator_core::ssh=debug`, per `AGENTS.md` |

### Lints

- **`clippy::await_holding_lock`** — already recommended by `AGENTS.md`; phase 1 enables it in
  CI *before* touching tunnel code.
- **`clippy::rc_buffer`** — relevant given the `Arc<[Value]>` row representation.
- **`unused_must_use` denied** — a great many gtk-rs calls return a `Result` that is tempting to
  ignore.

### Template iteration

There is no hot-reload for GResource-embedded templates; a `.ui` change triggers a Rust relink.
If layout iteration becomes painful, add a `#[cfg(debug_assertions)]` path that loads `.ui`
from disk via `gtk::Builder::from_file`, guarded by an env var. (A meson-based workflow gets
this free by loading the `.gresource` from an installed path; phase 2 trades it away for cargo
simplicity.)

---

## Technical Considerations

- Tier-1 and tier-2 tests live in the `sqlator-gtk` crate; service and core tests live in their
  own crates and need no display, so CI should run them as two separate jobs — the fast one
  should not wait on Xvfb.
- The phase 1 pure-function tests (`extract_single_table`, `unique_name`,
  `build_url_no_password`, `resolve_connection_type`, `detect_database_type`, import
  re-parenting, export round-trip) are the first real tests this workspace will have. They
  belong in `sqlator-service` and run in the fast job.
- Integration tests against `docker-compose.yml` need the services up; gate them behind a
  feature or an env var so a bare `cargo test` still passes.

---

## Acceptance Criteria

- [x] `clippy::await_holding_lock`, `clippy::rc_buffer`, and denied `unused_must_use` enforced in CI
- [x] A fast CI job runs `core` + `service` tests with no display server
- [x] A second CI job runs GTK tests under `xvfb-run` + `dbus-run-session` with `--test-threads=1`
- [x] `GSETTINGS_SCHEMA_DIR` is set in the test environment; no test aborts the process
- [x] Tier-1 model tests cover row count, item retrieval, cache staleness, typed sorting, `NULL` formatting, and `items_changed` batching
- [x] One tier-2 smoke test exists per composite template, added alongside each new `.blp`
- [x] A documented `GOBJECT_DEBUG=instance-count` procedure for checking `RowObject` leaks, run before each release
- [x] Sysprof used to validate grid performance before any grid optimization is merged (procedure documented in `gtk/README.md`; run before merging grid optimizations)

---

## Success Metrics

- Workspace test count goes from 2 to something that would actually catch a regression
- No template mismatch ever reaches a manual test session
- Grid performance claims are backed by Sysprof traces, not impressions

---

## Dependencies & Risks

| Dependency | Notes |
|---|---|
| `serial_test` | For per-test GTK serialization |
| Xvfb, `dbus-run-session` | CI images must have both |
| `docker-compose.yml` | Already exists; used by service/core integration tests |
| Sysprof | Local profiling only, not CI |

| Risk | Mitigation |
|---|---|
| GTK tests are flaky in CI and get disabled | Keep them few and fast; tier 3 deliberately skipped |
| Testing effort displaces feature work | Tier 2 is one assertion per template; tier 1 targets pure logic only |
| `gtk::init()` in parallel tests | `--test-threads=1` plus `serial_test` |

---

## Sources & References

### Internal References

- The entire existing test suite: `core/src/ssh/config_parser.rs:143,149`
- `AGENTS.md` — `clippy::await_holding_lock`, `RUST_LOG` conventions
- `docs/solutions/runtime-errors/ssh-tunnel-mutex-deadlock.md` — why the lint matters
- `docker-compose.yml` — integration test targets

### External References

- gtk4-rs CI (`xvfb-run --auto-servernum`): https://github.com/gtk-rs/gtk4-rs
- GTK Inspector: https://developer.gnome.org/documentation/tools/inspector.html
- Sysprof: https://apps.gnome.org/Sysprof/
- libadwaita adaptive preview (1.7): https://nyaa.place/blog/libadwaita-1-7/

### New Files

- `gtk/src/**/tests.rs` — tier 1 and tier 2 tests, colocated
- `service/tests/` — phase 1 pure-function tests
- CI workflow additions for the two-job split
