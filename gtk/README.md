# sqlator-gtk

Native GTK4 / libadwaita frontend for SQLator (`im.apodaca.SqlatorGtk`).

## Develop

```bash
# From the workspace root
cargo run -p sqlator-gtk
# Optional VTE database terminal
cargo run -p sqlator-gtk --features terminal
```

`build.rs` compiles Blueprint UI, the GResource bundle, and a private GSettings
schema under `OUT_DIR` so `cargo run` works without installing anything.
`GSETTINGS_SCHEMA_DIR` is set automatically when unset.

### Tests

Pure-logic (tier 1) and widget smoke (tier 2) tests live next to the code they
cover. Full interaction tests are intentionally skipped (see plan 006).

```bash
# Headless (Xvfb + session bus) — matches CI
./gtk/scripts/run-tests-headless.sh

# Or manually:
GDK_BACKEND=x11 GTK_A11Y=none \
  xvfb-run -a dbus-run-session -- \
  cargo test -p sqlator-gtk --locked -- --test-threads=1
```

CI runs the GTK job in an **Arch Linux** container. The crate enables gtk4
`gnome_50` / libadwaita `v1_9` (GLib ≥ 2.88, GTK ≥ 4.22), which is newer than
Ubuntu 24.04's packages.

`GSETTINGS_SCHEMA_DIR` is exported by `build.rs` into the test binary and also
set by `test_support::init_gtk` when unset — constructing `gio::Settings`
without it **aborts the process**.

### Debugging entry points

| Tool / env | Use |
|---|---|
| `GTK_DEBUG=interactive` | GTK Inspector (widget tree, CSS, a11y, render nodes). Or permanently: `gsettings set org.gtk.gtk4.Settings.Debug enable-inspector-keybinding true` then `Ctrl+Shift+D` |
| `Ctrl+Shift+M` | libadwaita adaptive preview (phone/tablet) inside the inspector |
| `GTK_DEBUG=builder` | Template / `.ui` diagnostics after editing Blueprint |
| `GTK_DEBUG=actions` | Action activation / enablement (why an accelerator "does nothing") |
| `GSK_RENDERER=cairo\|ngl\|vulkan`, `GTK_DEBUG=no-cache` | Grid / renderer oddities |
| `RUST_LOG=sqlator_gtk=debug,sqlator_core::ssh=debug` | Tracing (see `AGENTS.md`) |

**GObject leak check (run before each release):**

```bash
GOBJECT_DEBUG=instance-count GTK_DEBUG=interactive cargo run -p sqlator-gtk
# In the inspector: Statistics — watch SqlatorRowObject / SqlatorResultModel
# counts across repeated query runs. Counts should fall after clearing results.
```

**Sysprof (before any results-grid optimization lands):** record a session while
scrolling / streaming a large result set; GTK frame marks show main-loop
blocks. Prefer evidence from Sysprof over impressions.

## Desktop integration (no meson / Flatpak)

Install the `.desktop` entry, AppStream metainfo, icons, and GSettings schema
into an XDG prefix (default `~/.local`):

```bash
cargo build -p sqlator-gtk
./gtk/scripts/install-xdg-data.sh --bin target/debug/sqlator-gtk
```

Validate the metadata:

```bash
desktop-file-validate gtk/data/im.apodaca.SqlatorGtk.desktop
appstreamcli validate gtk/data/im.apodaca.SqlatorGtk.metainfo.xml
```

Meson and Flatpak packaging are deferred (`sqlator-9g1.11`).

## App ID checklist

Keep these in sync as `im.apodaca.SqlatorGtk`:

- `application-id` / `resource-base-path` in `src/application.rs`
- GResource prefix `/im/apodaca/SqlatorGtk/`
- GSettings schema id + path
- Desktop file / metainfo / icon basenames