//! Tier-2 composite-template smoke tests (plan 006).
//!
//! Rust's test harness spawns a fresh thread per `#[test]` even with
//! `--test-threads=1`. GTK treats the thread that called `adw::init` as the
//! only safe main thread, so all widget construction must share one test.

use crate::query_tab::QueryTab;
use crate::test_support::init_gtk;
use crate::window::SqlatorWindow;
use glib::subclass::prelude::ObjectSubclassIsExt;
use gtk::glib;
use serial_test::serial;

#[test]
#[serial]
fn composite_templates_inflate_and_children_resolve() {
    init_gtk();

    {
        // Object construction runs init_template(); mismatched .blp ids panic here.
        let tab: QueryTab = glib::Object::builder().build();
        let _ = tab.imp().editor_paned.get();
        let _ = tab.imp().editor_column.get();
        let _ = tab.imp().editor.get();
        let _ = tab.imp().toast_overlay.get();
        let _ = tab.imp().results_stack.get();
        let _ = tab.imp().results_grid.get();
        let _ = tab.imp().messages_view.get();
    }

    {
        // No Application yet — avoids "windows must be added after startup".
        // constructed() still builds gio::Settings (needs GSETTINGS_SCHEMA_DIR).
        let window: SqlatorWindow = glib::Object::builder().build();
        let _ = window.imp().split_view.get();
        let _ = window.imp().content_stack.get();
        let _ = window.imp().tab_view.get();
        let _ = window.imp().tab_bar.get();
        let _ = window.imp().tab_overview.get();
        let _ = window.imp().overview_button.get();
        let _ = window.imp().connection_tabs_scroll.get();
        let _ = window.imp().connection_tabs_box.get();
        let _ = window.imp().connection_list.get();
        let _ = window.imp().schema_list.get();
        let _ = window.imp().schema_banner.get();
        let _ = window.imp().schema_refresh.get();
        let _ = window.imp().toast_overlay.get();
        let _ = window.imp().terminal_paned.get();
        let _ = window.imp().terminal_host.get();
    }
}
