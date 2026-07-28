#!/usr/bin/env bash
# Run sqlator-gtk tests headlessly (plan 006).
# Needs: xvfb-run, dbus-run-session, blueprint-compiler, GTK4/libadwaita stack.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

# build.rs exports SQLATOR_SCHEMA_DIR into the test binary; GSETTINGS_SCHEMA_DIR
# is also set by gtk::test_support::init_gtk when unset. GTK_A11Y=none avoids
# AT-SPI hangs without a full session.
export GDK_BACKEND="${GDK_BACKEND:-x11}"
export GTK_A11Y="${GTK_A11Y:-none}"

exec xvfb-run -a dbus-run-session -- \
  cargo test -p sqlator-gtk --locked -- --test-threads=1 "$@"
