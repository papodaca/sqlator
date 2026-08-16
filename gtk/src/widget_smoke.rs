//! Tier-2 composite-template smoke tests (plan 006).
//!
//! Rust's test harness spawns a fresh thread per `#[test]` even with
//! `--test-threads=1`. GTK treats the thread that called `adw::init` as the
//! only safe main thread, so all widget construction must share one test.

use crate::query_tab::QueryTab;
use crate::results::{PagedStatus, ResultsGrid};
use crate::test_support::init_gtk;
use crate::window::SqlatorWindow;
use glib::subclass::prelude::ObjectSubclassIsExt;
use gtk::glib;
use gtk::prelude::*;
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
        // U2: ResultsGrid paging affordances. The bottom loading row (R3)
        // exists in the tree and starts hidden; near-bottom wiring and the
        // paging-aware status helpers construct cleanly beside the unchanged
        // non-paged status path (characterization pins below keep U3 honest).
        let grid = ResultsGrid::new();

        assert!(grid.imp().loading_more_row.parent().is_some(), "loading row appended");
        assert!(
            !grid.imp().loading_more_row.is_visible(),
            "loading row starts hidden"
        );
        grid.set_loading_more(true);
        assert!(grid.imp().loading_more_row.is_visible());
        grid.set_loading_more(false);
        assert!(!grid.imp().loading_more_row.is_visible());

        assert!(
            grid.imp().refresh_overlay.parent().is_some(),
            "refresh overlay wraps the scrolled data area"
        );
        assert!(!grid.imp().refresh_scrim.is_visible(), "scrim starts hidden");
        assert!(!grid.imp().refresh_card.is_visible(), "card starts hidden");
        grid.set_refreshing(true);
        assert!(grid.imp().refresh_scrim.is_visible());
        assert!(grid.imp().refresh_card.is_visible());
        grid.set_refreshing(true);
        grid.set_refreshing(false);
        assert!(
            !grid.imp().refresh_scrim.is_visible() && !grid.imp().refresh_card.is_visible(),
            "consecutive set_refreshing(true) still hides exactly once"
        );

        grid.set_refreshing(true);
        grid.set_loading_more(true);
        assert!(grid.imp().refresh_scrim.is_visible());
        assert!(grid.imp().loading_more_row.is_visible());
        grid.set_refreshing(false);
        assert!(
            grid.imp().loading_more_row.is_visible(),
            "hiding overlay does not hide the chunk indicator"
        );
        assert!(!grid.imp().refresh_scrim.is_visible());
        grid.set_loading_more(false);
        assert!(!grid.imp().loading_more_row.is_visible());

        // Unallocated grid reports content-fits-viewport; the near-bottom
        // signal itself stays silent there (gated on upper > page_size).
        assert!(grid.content_fits_viewport());
        grid.connect_near_bottom(|| {});
        assert!(grid.imp().near_bottom_wired.get());

        // Characterization pins: the non-paged update_status/finish formats
        // are the passthrough path's contract.
        grid.begin_columns(vec!["id".to_string(), "name".to_string()]);
        assert_eq!(grid.imp().status.text(), "0 rows \u{00b7} 2 columns");
        grid.finish(42, 7);
        assert_eq!(grid.imp().status.text(), "42 rows in 7 ms \u{00b7} 2 columns");

        // Paged status copy reaches the same label with the same column
        // suffix; the R5 ceiling message is pinned verbatim.
        grid.show_paged_status(1_000, PagedStatus::More);
        assert_eq!(
            grid.imp().status.text(),
            "1000 rows \u{00b7} more rows available \u{00b7} 2 columns"
        );
        grid.show_paged_status(1_500, PagedStatus::Exhausted);
        assert_eq!(
            grid.imp().status.text(),
            "1500 rows \u{00b7} all rows loaded \u{00b7} 2 columns"
        );
        grid.show_paged_status(50_000, PagedStatus::Capped);
        assert_eq!(
            grid.imp().status.text(),
            "50000 rows \u{00b7} 50,000 row limit reached \u{00b7} 2 columns"
        );

        // Column-cap suffix branch, identical for both status paths.
        let many: Vec<String> = (0..31).map(|i| format!("c{i}")).collect();
        grid.begin_columns(many);
        assert_eq!(
            grid.imp().status.text(),
            "0 rows \u{00b7} showing 30 of 31 columns"
        );
        grid.finish(7, 3);
        assert_eq!(
            grid.imp().status.text(),
            "7 rows in 3 ms \u{00b7} showing 30 of 31 columns"
        );
        grid.show_paged_status(50_000, PagedStatus::Capped);
        assert_eq!(
            grid.imp().status.text(),
            "50000 rows \u{00b7} 50,000 row limit reached \u{00b7} showing 30 of 31 columns"
        );
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
