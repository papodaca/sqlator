#!/usr/bin/env bash
# Install sqlator-gtk desktop integration files into an XDG prefix (no meson).
#
# Default PREFIX=$HOME/.local — puts the app in the user application menu after
# `cargo build -p sqlator-gtk` (or pass --bin to install the binary too).
#
# Usage:
#   ./gtk/scripts/install-xdg-data.sh
#   ./gtk/scripts/install-xdg-data.sh --prefix /usr
#   ./gtk/scripts/install-xdg-data.sh --bin target/debug/sqlator-gtk
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DATA="$ROOT/gtk/data"
PREFIX="${PREFIX:-$HOME/.local}"
BIN_SRC=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --prefix)
      PREFIX="$2"
      shift 2
      ;;
    --bin)
      BIN_SRC="$2"
      shift 2
      ;;
    -h|--help)
      sed -n '2,12p' "$0"
      exit 0
      ;;
    *)
      echo "unknown arg: $1" >&2
      exit 1
      ;;
  esac
done

APP_ID="im.apodaca.SqlatorGtk"
SHARE="$PREFIX/share"
BIN_DIR="$PREFIX/bin"

install -d \
  "$SHARE/applications" \
  "$SHARE/metainfo" \
  "$SHARE/glib-2.0/schemas" \
  "$SHARE/icons/hicolor/scalable/apps" \
  "$SHARE/icons/hicolor/symbolic/apps"

install -m 644 "$DATA/${APP_ID}.desktop" "$SHARE/applications/${APP_ID}.desktop"
install -m 644 "$DATA/${APP_ID}.metainfo.xml" "$SHARE/metainfo/${APP_ID}.metainfo.xml"
install -m 644 "$DATA/${APP_ID}.gschema.xml" "$SHARE/glib-2.0/schemas/${APP_ID}.gschema.xml"
install -m 644 \
  "$DATA/icons/hicolor/scalable/apps/${APP_ID}.svg" \
  "$SHARE/icons/hicolor/scalable/apps/${APP_ID}.svg"
install -m 644 \
  "$DATA/icons/hicolor/symbolic/apps/${APP_ID}-symbolic.svg" \
  "$SHARE/icons/hicolor/symbolic/apps/${APP_ID}-symbolic.svg"

# If Exec= points at PATH, optionally install the cargo binary next to it.
if [[ -n "$BIN_SRC" ]]; then
  if [[ ! -f "$BIN_SRC" ]]; then
    echo "binary not found: $BIN_SRC" >&2
    exit 1
  fi
  install -d "$BIN_DIR"
  install -m 755 "$BIN_SRC" "$BIN_DIR/sqlator-gtk"
  echo "installed binary → $BIN_DIR/sqlator-gtk"
fi

if command -v glib-compile-schemas >/dev/null 2>&1; then
  glib-compile-schemas "$SHARE/glib-2.0/schemas"
fi
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$SHARE/applications" 2>/dev/null || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$SHARE/icons/hicolor" 2>/dev/null || true
fi

echo "installed desktop integration under $PREFIX"
echo "  desktop : $SHARE/applications/${APP_ID}.desktop"
echo "  metainfo: $SHARE/metainfo/${APP_ID}.metainfo.xml"
echo "  icons   : $SHARE/icons/hicolor/{scalable,symbolic}/apps/"
echo "  gschema : $SHARE/glib-2.0/schemas/${APP_ID}.gschema.xml"
