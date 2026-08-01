mod application;
mod connection;
mod docker;
mod editor;
mod portability;
mod preferences;
mod query_tab;
mod results;
mod runtime;
mod schema;
mod session;
mod ssh;
#[cfg(feature = "terminal")]
mod terminal;
#[cfg(test)]
mod test_support;
mod theme;
mod vault;
#[cfg(test)]
mod widget_smoke;
mod window;

use application::SqlatorApplication;
use gettextrs::{bind_textdomain_codeset, bindtextdomain, setlocale, textdomain, LocaleCategory};
use gtk::gio;
use gtk::prelude::*;
use std::env;
use std::path::Path;

fn main() {
    setup_i18n();

    // Must run before any gio::Settings construction — Settings::new aborts if
    // the schema is missing from the search path.
    if env::var_os("GSETTINGS_SCHEMA_DIR").is_none() {
        let fallback = env!("SQLATOR_SCHEMA_DIR");
        if Path::new(fallback).exists() {
            env::set_var("GSETTINGS_SCHEMA_DIR", fallback);
        }
    }

    gio::resources_register_include!("sqlator.gresource")
        .expect("Failed to register sqlator.gresource");

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let app = SqlatorApplication::new();
    app.run();
}

fn setup_i18n() {
    const GETTEXT_PACKAGE: &str = "sqlator-gtk";
    const LOCALEDIR: &str = "/usr/share/locale";

    let _ = setlocale(LocaleCategory::LcAll, "");
    let _ = bindtextdomain(GETTEXT_PACKAGE, LOCALEDIR);
    let _ = bind_textdomain_codeset(GETTEXT_PACKAGE, "UTF-8");
    let _ = textdomain(GETTEXT_PACKAGE);
}