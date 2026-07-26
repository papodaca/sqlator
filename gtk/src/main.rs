mod application;
mod query_tab;
mod results;
mod runtime;
mod window;

use application::SqlatorApplication;
use gtk::gio;
use gtk::prelude::*;
use std::env;

fn main() {
    // Must run before any gio::Settings construction — Settings::new aborts if
    // the schema is missing from the search path.
    if env::var_os("GSETTINGS_SCHEMA_DIR").is_none() {
        env::set_var("GSETTINGS_SCHEMA_DIR", env!("SQLATOR_SCHEMA_DIR"));
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
