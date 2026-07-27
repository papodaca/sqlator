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
