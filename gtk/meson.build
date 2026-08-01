project(
  'sqlator-gtk',
  'c',
  version: '0.1.0',
  meson_version: '>= 1.3.0',
)

i18n = import('i18n')

app_id = 'im.apodaca.SqlatorGtk'
gettext_package = 'sqlator-gtk'

cargo = find_program('cargo', required: true)

# Flatpak builds vendor crates under $CARGO_HOME/vendor and set CARGO_HOME.
# Native meson installs use a normal online/locked cargo build.
cargo_build = custom_target(
  'cargo-build-sqlator-gtk',
  output: 'sqlator-gtk',
  command: [
    'sh',
    '-c',
    '''set -eu
manifest="@1@/Cargo.toml"
out="@2@"
workspace="$(dirname "@1@")"
if [ -n "${CARGO_HOME:-}" ] && [ -d "$CARGO_HOME/vendor" ]; then
  mkdir -p "$CARGO_HOME"
  printf '%s\n' \
    '[source.crates-io]' \
    'replace-with = "vendored-sources"' \
    '[source.vendored-sources]' \
    "directory = \"$CARGO_HOME/vendor\"" \
    > "$CARGO_HOME/config.toml"
  "@0@" build --release --locked --offline -p sqlator-gtk --manifest-path "$manifest"
else
  "@0@" build --release --locked -p sqlator-gtk --manifest-path "$manifest"
fi
cp -f "$workspace/target/release/sqlator-gtk" "$out"
'''
      .format(cargo.full_path(), meson.project_source_root(), '@OUTPUT@'),
  ],
  build_by_default: true,
  install: true,
  install_dir: get_option('bindir'),
)

subdir('data')
subdir('po')
