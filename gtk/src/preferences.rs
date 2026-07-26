//! `AdwPreferencesDialog` bound to GSettings (`editor-font`, `max-rows`,
//! `confirm-destructive`).

use adw::prelude::*;
use gtk::{gdk, gio, glib, pango};

pub const EDITOR_FONT_KEY: &str = "editor-font";
pub const MAX_ROWS_KEY: &str = "max-rows";
pub const CONFIRM_DESTRUCTIVE_KEY: &str = "confirm-destructive";

const EDITOR_CSS_CLASS: &str = "sqlator-editor";

/// Present the preferences dialog parented to `parent`.
pub fn present(parent: &impl IsA<gtk::Widget>, settings: &gio::Settings) {
    let dialog = adw::PreferencesDialog::new();
    dialog.set_title("Preferences");
    dialog.set_search_enabled(true);

    let page = adw::PreferencesPage::new();
    page.set_title("General");
    page.set_icon_name(Some("preferences-system-symbolic"));

    // ── Editor ────────────────────────────────────────────────────────────
    let editor_group = adw::PreferencesGroup::new();
    editor_group.set_title("Editor");

    let font_row = adw::ActionRow::builder()
        .title("Editor font")
        .subtitle("Applied to SQL editors and the DDL viewer")
        .build();
    let font_dialog = gtk::FontDialog::builder()
        .title("Editor font")
        .modal(true)
        .build();
    let font_btn = gtk::FontDialogButton::new(Some(font_dialog));
    let desc = pango::FontDescription::from_string(&settings.string(EDITOR_FONT_KEY));
    font_btn.set_font_desc(&desc);
    font_btn.set_valign(gtk::Align::Center);
    font_btn.connect_notify_local(
        Some("font-desc"),
        glib::clone!(
            #[strong]
            settings,
            move |btn, _| {
                if let Some(desc) = btn.font_desc() {
                    let _ = settings.set_string(EDITOR_FONT_KEY, &desc.to_str());
                }
            }
        ),
    );
    font_row.add_suffix(&font_btn);
    font_row.set_activatable_widget(Some(&font_btn));
    editor_group.add(&font_row);

    // ── Results ───────────────────────────────────────────────────────────
    let results_group = adw::PreferencesGroup::new();
    results_group.set_title("Results");

    let max_adj = gtk::Adjustment::new(
        f64::from(settings.get::<i32>(MAX_ROWS_KEY)),
        50.0,
        100_000.0,
        50.0,
        500.0,
        0.0,
    );
    let max_rows = adw::SpinRow::builder()
        .title("Maximum rows")
        .subtitle("Cap for table browse and result fetches")
        .adjustment(&max_adj)
        .digits(0)
        .build();
    max_rows.connect_changed(glib::clone!(
        #[strong]
        settings,
        move |row| {
            let v = row.value().round() as i32;
            if settings.get::<i32>(MAX_ROWS_KEY) != v {
                let _ = settings.set(MAX_ROWS_KEY, v);
            }
        }
    ));
    results_group.add(&max_rows);

    let confirm = adw::SwitchRow::builder()
        .title("Confirm destructive statements")
        .subtitle("Ask before running DELETE/UPDATE without WHERE, DROP, or TRUNCATE")
        .build();
    settings
        .bind(CONFIRM_DESTRUCTIVE_KEY, &confirm, "active")
        .build();
    results_group.add(&confirm);

    page.add(&editor_group);
    page.add(&results_group);
    dialog.add(&page);
    dialog.present(Some(parent));
}

/// Keep a display-wide CSS provider in sync with `editor-font`.
pub fn bind_editor_font(settings: &gio::Settings) {
    let provider = gtk::CssProvider::new();
    apply_editor_font_css(&provider, &settings.string(EDITOR_FONT_KEY));

    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
    }

    settings.connect_changed(
        Some(EDITOR_FONT_KEY),
        glib::clone!(
            #[strong]
            settings,
            #[strong]
            provider,
            move |_, _| {
                apply_editor_font_css(&provider, &settings.string(EDITOR_FONT_KEY));
            }
        ),
    );
}

fn apply_editor_font_css(provider: &gtk::CssProvider, font: &str) {
    let desc = pango::FontDescription::from_string(font);
    let family = desc
        .family()
        .map(|f| f.as_str().to_string())
        .unwrap_or_else(|| "Monospace".to_string());
    let size_pt = if desc.size() > 0 {
        let scaled = desc.size() as f64 / f64::from(pango::SCALE);
        if desc.is_size_absolute() {
            // Absolute size is in device units; approximate as px ≈ pt for CSS.
            scaled * 0.75
        } else {
            scaled
        }
    } else {
        11.0
    };
    let escaped_family = family.replace('\\', "\\\\").replace('"', "\\\"");
    let css = format!(
        ".{EDITOR_CSS_CLASS} {{ font-family: \"{escaped_family}\"; font-size: {size_pt}pt; }}\n"
    );
    provider.load_from_string(&css);
}

/// Mark a SourceView / TextView so the editor-font CSS applies.
pub fn style_editor(view: &impl IsA<gtk::Widget>) {
    view.add_css_class(EDITOR_CSS_CLASS);
}

/// Read the configured max-rows cap (falls back to 1000).
pub fn max_rows(settings: &gio::Settings) -> usize {
    settings.get::<i32>(MAX_ROWS_KEY).max(1) as usize
}

/// Whether destructive SQL should prompt (default true).
pub fn confirm_destructive(settings: &gio::Settings) -> bool {
    settings.get::<bool>(CONFIRM_DESTRUCTIVE_KEY)
}
