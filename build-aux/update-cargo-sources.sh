#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/build-aux/cargo-sources.json"
GENERATOR_CMD=""

if command -v flatpak-cargo-generator >/dev/null 2>&1; then
  GENERATOR_CMD="flatpak-cargo-generator"
fi

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

cp -f "$ROOT/Cargo.lock" "$tmpdir/Cargo.lock"

if [[ -z "$GENERATOR_CMD" ]]; then
  echo "flatpak-cargo-generator not found; bootstrapping from flatpak-builder-tools..."
  git clone --depth 1 https://github.com/flatpak/flatpak-builder-tools.git "$tmpdir/flatpak-builder-tools"
  python -m venv "$tmpdir/venv"
  "$tmpdir/venv/bin/pip" install aiohttp toml tomlkit >/dev/null
  GENERATOR_CMD="$tmpdir/venv/bin/python $tmpdir/flatpak-builder-tools/cargo/flatpak-cargo-generator.py"
fi

pushd "$tmpdir" >/dev/null
eval "$GENERATOR_CMD Cargo.lock -o cargo-sources.json"
popd >/dev/null

install -m 644 "$tmpdir/cargo-sources.json" "$OUT"
echo "updated $OUT"
