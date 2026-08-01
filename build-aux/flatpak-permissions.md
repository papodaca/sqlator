# Flatpak Permission Justifications

This document explains each permission in `im.apodaca.SqlatorGtk.json`.

## Runtime access

- `--share=ipc`: Required by GTK for normal desktop IPC behavior.
- `--socket=fallback-x11`: X11 fallback support when Wayland is unavailable.
- `--socket=wayland`: Native Wayland support for GNOME.
- `--device=dri`: GPU acceleration for GTK rendering.
- `--share=network`: Required for direct DB connections and SSH tunnels.

## Secrets and SSH

- `--talk-name=org.freedesktop.secrets`: Enables keyring/Secret Service access
  for credential storage.
- `--filesystem=~/.ssh:ro`: Read-only access to SSH configs/keys for SSH tunnel
  and remote Docker workflows.
- `--filesystem=xdg-run/ssh-agent`: Enables SSH agent socket forwarding.

## Explicitly not requested

- No broad home directory access.
- No Docker socket mount (`/var/run/docker.sock`) in Flatpak.
- No host process escape (`flatpak-spawn --host`) by default.

Local Docker discovery is intentionally disabled in sandbox builds; users should
use remote Docker over SSH instead.
