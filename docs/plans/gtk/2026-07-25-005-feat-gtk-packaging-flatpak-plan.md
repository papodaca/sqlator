---
title: "feat: Package sqlator-gtk for GNOME (meson, Flatpak, portals, i18n)"
type: feat
status: active
date: 2026-07-25
---

# feat: Package sqlator-gtk for GNOME (meson, Flatpak, portals, i18n)

## Overview

Add the GNOME distribution layer on top of a working cargo-built app: meson, a Flatpak
manifest, desktop integration files, portal usage where the sandbox requires it, and gettext
i18n. Deliberately last, because every sandbox decision depends on knowing which features
actually shipped.

---

## Problem Statement / Motivation

Phases 2 and 3 build a cargo-only app, which is the right call while iterating — `cargo run`
works with no install step, and `build.rs` handles Blueprint, GResource and the GSettings
schema. But a GTK app that cannot be installed as a Flatpak is not really a GNOME app, and
Flathub is how GNOME users would find this.

The deleted `sqlmator/` template did carry meson and Flatpak files, but they were unmodified
boilerplate with a manifest pointing at `file:///home/usr1/Projects` — useless for a
subdirectory crate with `../core` path dependencies. This is written fresh.

The sandbox raises real problems that phases 2–3 are allowed to ignore and this phase is not.

---

## Proposed Solution

### Sandbox problems, and their answers

These are the load-bearing decisions of this phase.

| Capability | Problem under Flatpak | Approach |
|---|---|---|
| **Local Docker discovery** | `core/src/docker/local.rs:10` checks `/var/run/docker.sock` exists, then shells out to the `docker` binary. Neither is available in the sandbox. | Either hole-punch the socket (`--filesystem=/run/docker.sock`, which is effectively root-equivalent and Flathub will push back on), or use `--talk-name=org.freedesktop.Flatpak` + `flatpak-spawn --host docker`, or **detect the sandbox and disable local Docker with a clear message**, leaving remote-over-SSH Docker working. Recommend the third for the Flathub build, with the second as an opt-in. |
| **Keyring** | The `keyring` crate talks to the Secret Service over D-Bus. | Works via `--talk-name=org.freedesktop.secrets`. Verify the `keyring` crate's Secret Service backend behaves under the portal; if not, the existing encrypted vault backend is already a complete fallback and works headless. |
| **`~/.ssh` access** | Needed for `config_parser` (host dropdown) and identity files. | `--filesystem=~/.ssh:ro`. Flathub will ask why; the answer is SSH tunneling, which is a core feature. |
| **SSH agent** | `SSH_AUTH_SOCK` points outside the sandbox. | `--filesystem=xdg-run/ssh-agent` (or pass the socket path through). Test agent auth explicitly. |
| **Outbound network** | Tunnels and DB connections. | `--share=network`. Note the app also *listens* on localhost for tunnel forwards — confirm that works in-sandbox. |
| **Config location** | `dirs::config_dir()/sqlator/connections.json` becomes the per-app sandboxed path. | Decide whether the Flatpak shares config with a native install. Recommend **not** sharing by default, and offering import instead. Document it. |
| **Export destination** | Phase 1 changed export to return a JSON string. | Use `gtk::FileDialog`, which routes through the file-chooser portal. No filesystem permission needed. |
| **VTE terminal** | `spawn_async` forks a `psql` binary that must exist in the sandbox. | Bundle the client binaries in the Flatpak, or disable the terminal in the sandboxed build. Bundling `psql`/`mysql`/`sqlite3` is feasible; `sqlplus` and `sqlcmd` are not redistributable. Recommend bundling the free clients and detecting the rest. |

### Build system

Add meson alongside cargo, not replacing it — `cargo build` must keep working for development.

- `meson.build` at the crate root driving `cargo build --release` via `custom_target`, with
  `--offline` and a vendored cargo registry for reproducible/Flathub builds.
- `data/meson.build` installing the desktop file, metainfo, gschema, and icons.
- `po/` + `po/meson.build` for gettext.
- The GSettings schema gets installed to `/usr/share/glib-2.0/schemas/` by meson, at which point
  the `GSETTINGS_SCHEMA_DIR` fallback from phase 2 correctly defers to it.
- Reinstate the meson-based `.gresource`-from-installed-path option if template hot-reload
  during development proves worth it (phase 2 traded this away deliberately).

### Desktop integration

- `data/im.apodaca.SqlatorGtk.desktop.in` — with `MimeType=application/sql;` so `.sql` files can
  open in the app (pairs with the unimplemented
  `docs/plans/2026-04-14-005-feat-open-save-sql-files-plan.md`).
- `data/im.apodaca.SqlatorGtk.metainfo.xml.in` — AppStream. Must validate with
  `appstreamcli validate` and needs real screenshots, a summary, a description, release notes,
  and correct `<content_rating>` for Flathub.
- Icons: a real scalable app icon plus a symbolic variant, following the GNOME HIG. The deleted
  template's placeholder icons were not kept — these need to be designed.
- `data/im.apodaca.SqlatorGtk.service.in` — only if D-Bus activation is wanted; probably not.

### Flatpak manifest

`im.apodaca.SqlatorGtk.json` against `org.gnome.Platform` // `49` (or current) with
`org.freedesktop.Sdk.Extension.rust-stable`. Sources must be git or archive URLs, never
`file:///home/...`. Because the crate lives in a subdirectory of a workspace with `../core`
path dependencies, the manifest builds the **repository root** and points meson/cargo at the
`gtk/` member.

Vendor dependencies with `flatpak-cargo-generator.py` to produce `cargo-sources.json` for
offline builds.

Starting permission set, to be justified line by line for Flathub:

```
--share=ipc --socket=fallback-x11 --socket=wayland --device=dri
--share=network
--talk-name=org.freedesktop.secrets
--filesystem=~/.ssh:ro
--filesystem=xdg-run/ssh-agent
```

### i18n

The app is currently English-only with no i18n anywhere in the Svelte frontend either. Set up
gettext properly here rather than retrofitting: `gettext-rs`, a `po/POTFILES.in` covering both
`.blp` and `.rs` sources (blueprint-compiler emits translatable strings correctly), and
`xgettext` wired into meson. No translations need to ship initially — the point is that the
strings are extractable.

---

## Technical Considerations

- **`org.gnome.Platform` version must match the GTK the app is built against.** The runtime
  ships GTK/libadwaita; the pinned crate features (`gnome_50`, `v1_9`, `gtk_v4_22`) must not
  exceed what the chosen runtime provides. Verify before choosing the runtime version, and
  re-verify on every runtime bump.
- **`blueprint-compiler` is in the GNOME SDK**, so the build-time dependency is satisfied inside
  Flatpak without extra work.
- **`sqlator-gtk` is a workspace member**, so a Flatpak build compiles the whole workspace
  unless scoped with `-p sqlator-gtk`. Do that, or Tauri and web get dragged in.
- **App ID consistency**: `im.apodaca.SqlatorGtk` must match across the binary's `APP_ID`, the
  GResource prefix (`/im/apodaca/SqlatorGtk/`), the gschema id and path, the desktop file name,
  the metainfo id, the icon file names, and the Flatpak manifest id. Any mismatch produces a
  silent failure at a different layer each time.
- **The merged-entrypoints plan** (`docs/plans/2026-04-12-002-feat-merged-entrypoints-plan.md`)
  folds TUI and web into a single `sqlator` binary via clap subcommands. Decide whether the GTK
  build is a fifth subcommand of that binary or a separate one. Recommend **separate** — a
  Flatpak shipping a web server and a TUI inside a sandboxed GUI app is confusing and expands
  the permission ask.
- **`connections.json` concurrency** matters more once a Flatpak and a native build can both
  run: phase 1's atomic-write fix is a prerequisite for this phase, not a nicety.

---

## System-Wide Impact

- **Interaction graph:** no runtime change. Portals interpose on file selection and secrets.
- **Error propagation:** sandbox-specific failures (no Docker socket, no agent socket) must
  produce the same classified, remediation-carrying errors as their unsandboxed counterparts.
  The Docker classifier from phase 3 needs a `sandboxed` bucket.
- **State lifecycle risks:** config path divergence between Flatpak and native installs is a
  real user-facing trap. Document, and offer import.
- **API surface parity:** none.

---

## Acceptance Criteria

- [ ] `cargo build` and `cargo run` still work unchanged, with no meson involvement
- [ ] `meson setup && ninja && ninja install` produces a working installed app
- [ ] `flatpak-builder` builds from a clean checkout with no local path sources
- [ ] The Flatpak launches, connects to a database, and opens an SSH-tunneled connection
- [ ] SSH agent auth works inside the sandbox
- [ ] Keyring works inside the sandbox, or falls back cleanly to the vault with a clear message
- [ ] Local Docker discovery either works or is disabled with an actionable message — never a silent failure
- [ ] Remote (SSH) Docker discovery works inside the sandbox
- [ ] Export uses the file-chooser portal and needs no filesystem permission
- [ ] `appstreamcli validate` passes on the metainfo
- [ ] `desktop-file-validate` passes
- [ ] The installed gschema takes precedence over the `build.rs` `OUT_DIR` fallback
- [ ] `xgettext` extracts strings from both `.blp` and `.rs` sources
- [ ] Icon renders correctly at all sizes, symbolic variant included
- [ ] Every Flatpak permission has a written justification

---

## Success Metrics

- A Flatpak bundle installable from a local repo that passes the full tier-1 and tier-2
  acceptance criteria from phase 3
- Flathub-submittable: no unjustified permissions, valid metainfo, real screenshots

---

## Dependencies & Risks

| Dependency | Notes |
|---|---|
| Phases 2 and 3 complete | Sandbox decisions depend on which features shipped |
| Phase 1 atomic config writes | Prerequisite once two builds can run concurrently |
| `org.gnome.Platform` runtime | Must provide GTK ≥ 4.22 and libadwaita ≥ 1.9 for the pinned features |
| `flatpak-cargo-generator.py` | For offline vendored builds |
| Icon design | Not yet started; the template placeholders were discarded |

| Risk | Mitigation |
|---|---|
| Flathub rejects the `~/.ssh` or Docker socket permission | Have the fallback ready: vault instead of keyring, SSH-only Docker instead of local socket |
| Runtime GTK older than the pinned crate features | Verify before choosing the runtime; treat as a gate |
| Workspace build drags in Tauri/web | Scope with `-p sqlator-gtk` |
| App ID mismatch across seven files | Single checklist item in acceptance criteria; grep as part of review |
| Bundled DB clients bloat the Flatpak | Bundle only the free clients; detect the rest at runtime |

---

## Sources & References

### Internal References

- Local Docker socket check and subprocess spawn: `core/src/docker/local.rs:10`
- Remote Docker over SSH (works in sandbox): `core/src/docker/inspector.rs:86`, `:124`
- Keyring service name `"sqlator"`: `core/src/credentials/keyring_backend.rs:4`
- Vault fallback (headless-capable): `core/src/credentials/vault_backend.rs`
- SSH config parsing: `core/src/ssh/config_parser.rs`
- Config path: `core/src/config.rs` — `dirs::config_dir()/sqlator/connections.json`
- Entrypoint strategy to reconcile: `docs/plans/2026-04-12-002-feat-merged-entrypoints-plan.md`
- SQL file MIME association: `docs/plans/2026-04-14-005-feat-open-save-sql-files-plan.md`

### External References

- Flathub submission requirements: https://docs.flathub.org/docs/for-app-authors/submission
- Flatpak sandbox permissions: https://docs.flatpak.org/en/latest/sandbox-permissions.html
- AppStream metainfo spec: https://www.freedesktop.org/software/appstream/docs/
- GNOME HIG app icons: https://developer.gnome.org/hig/guidelines/app-icons.html
- `flatpak-cargo-generator.py`: https://github.com/flatpak/flatpak-builder-tools/tree/master/cargo

### New Files

- `gtk/meson.build`, `gtk/src/meson.build`, `gtk/data/meson.build`, `gtk/data/icons/meson.build`
- `gtk/po/` — `POTFILES.in`, `LINGUAS`, `meson.build`
- `gtk/data/im.apodaca.SqlatorGtk.desktop.in`
- `gtk/data/im.apodaca.SqlatorGtk.metainfo.xml.in`
- `gtk/data/icons/hicolor/scalable/apps/im.apodaca.SqlatorGtk.svg`
- `gtk/data/icons/hicolor/symbolic/apps/im.apodaca.SqlatorGtk-symbolic.svg`
- `build-aux/im.apodaca.SqlatorGtk.json` — Flatpak manifest
- `build-aux/cargo-sources.json` — generated
