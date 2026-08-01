desktop_conf = configuration_data()
desktop_conf.set('APP_ID', app_id)

desktop_file = configure_file(
  input: 'im.apodaca.SqlatorGtk.desktop.in',
  output: app_id + '.desktop',
  configuration: desktop_conf,
)

metainfo_conf = configuration_data()
metainfo_conf.set('APP_ID', app_id)

metainfo_file = configure_file(
  input: 'im.apodaca.SqlatorGtk.metainfo.xml.in',
  output: app_id + '.metainfo.xml',
  configuration: metainfo_conf,
)

install_data(
  desktop_file,
  install_dir: join_paths(get_option('datadir'), 'applications'),
)

install_data(
  metainfo_file,
  install_dir: join_paths(get_option('datadir'), 'metainfo'),
)

install_data(
  app_id + '.gschema.xml',
  install_dir: join_paths(get_option('datadir'), 'glib-2.0', 'schemas'),
)

glib_compile_schemas = find_program('glib-compile-schemas', required: true)
meson.add_install_script(
  'sh',
  '-c',
  '"@0@" "$DESTDIR$MESON_INSTALL_PREFIX/@1@/glib-2.0/schemas"'
    .format(glib_compile_schemas.full_path(), get_option('datadir')),
)

install_data(
  'style.css',
  install_dir: join_paths(get_option('datadir'), 'sqlator-gtk'),
)

subdir('icons')