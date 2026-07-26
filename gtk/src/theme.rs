//! Light / dark / system theming via GSettings + AdwStyleManager.
//!
//! Theme is intentionally not routed through AppService — see the parity plan:
//! GTK uses its own GSetting rather than `get_theme` / `save_theme`.

use adw::prelude::*;
use gtk::{gio, glib};

pub const COLOR_SCHEME_KEY: &str = "color-scheme";

/// Map the GSetting nick to an [`adw::ColorScheme`].
pub fn color_scheme_from_setting(nick: &str) -> adw::ColorScheme {
    match nick {
        "force-light" => adw::ColorScheme::ForceLight,
        "force-dark" => adw::ColorScheme::ForceDark,
        _ => adw::ColorScheme::Default,
    }
}

/// Apply `color-scheme` from `settings` to the default style manager.
pub fn apply_from_settings(settings: &gio::Settings) {
    let nick = settings.string(COLOR_SCHEME_KEY);
    adw::StyleManager::default().set_color_scheme(color_scheme_from_setting(&nick));
}

/// Keep AdwStyleManager in sync whenever the GSetting changes (any window / process).
pub fn bind_settings(settings: &gio::Settings) {
    apply_from_settings(settings);
    settings.connect_changed(
        Some(COLOR_SCHEME_KEY),
        glib::clone!(
            #[strong]
            settings,
            move |_, _| {
                apply_from_settings(&settings);
            }
        ),
    );
}
