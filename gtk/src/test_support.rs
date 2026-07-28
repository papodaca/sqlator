//! Shared helpers for GTK unit / widget smoke tests (plan 006).

use gtk::gio;
use std::env;
use std::sync::Once;

static INIT: Once = Once::new();

/// Headless-safe init: schema dir, GResource, libadwaita/GTK.
///
/// Call once per process before constructing any `gio::Settings` or template
/// widgets. Pair with `#[serial]` (or `--test-threads=1`) so GTK stays on one
/// thread.
pub fn init_gtk() {
    INIT.call_once(|| {
        if env::var_os("GSETTINGS_SCHEMA_DIR").is_none() {
            // Edition 2021: set_var is safe. INIT guarantees single-threaded setup.
            env::set_var("GSETTINGS_SCHEMA_DIR", env!("SQLATOR_SCHEMA_DIR"));
        }
        gio::resources_register_include!("sqlator.gresource")
            .expect("register sqlator.gresource for tests");
        adw::init().expect("adw::init for tests");
    });
}
