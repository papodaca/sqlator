//! ColumnView results grid widget (phase-0 spike → production).

use crate::results::cell::{CellValue, ColumnMeta};
use crate::results::model::ResultModel;
use crate::results::paging_helpers::{self, NearBottomGuard};
use crate::results::PagedStatus;
use crate::results::row::RowObject;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{
    glib, ColumnView, ColumnViewCell, ColumnViewColumn, ColumnViewSorter, CustomSorter,
    GestureClick, Inscription, MultiSelection, SignalListItemFactory, SortListModel, SortType,
};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// Default mounted-column budget from the spike (no horizontal virtualization).
pub const VISIBLE_COLUMN_CAP: usize = 30;

const FLUSH_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Default, Clone)]
pub struct EditOverlay {
    pub deleted_rows: HashSet<u32>,
    pub added_rows: HashSet<u32>,
    /// `(model_index, col_idx) → display value`
    pub cell_overrides: HashMap<(u32, usize), CellValue>,
    pub modified_cells: HashSet<(u32, usize)>,
}

pub struct EditToolbarState {
    pub visible: bool,
    pub editable: bool,
    pub reason: Option<String>,
    pub change_count: usize,
    pub selected_count: u32,
}

type SimpleCb = Rc<dyn Fn()>;
type EditRowCb = Rc<dyn Fn(u32)>;
/// Server-side sort callback: `None` clears sort; `Some((column, desc))` sets it.
type ServerSortCb = Rc<dyn Fn(Option<(String, bool)>)>;

mod imp {
    use super::*;
    use adw::prelude::BinExt;

    #[derive(Default)]
    pub struct ResultsGrid {
        pub root: gtk::Box,
        pub action_bar: gtk::ActionBar,
        pub readonly_badge: gtk::Label,
        pub add_btn: gtk::Button,
        pub delete_btn: gtk::Button,
        pub change_badge: gtk::Label,
        pub discard_btn: gtk::Button,
        pub save_btn: gtk::Button,
        pub status: gtk::Label,
        pub scrolled: gtk::ScrolledWindow,
        /// Bottom loading row appended after the scrolled area (R3); hidden by default.
        pub loading_more_row: gtk::Box,
        pub column_view: ColumnView,
        pub model: RefCell<Option<ResultModel>>,
        pub selection: RefCell<Option<MultiSelection>>,
        pub pending: RefCell<Vec<Arc<[CellValue]>>>,
        pub flush_source: RefCell<Option<glib::SourceId>>,
        pub total_columns: Cell<usize>,
        pub visible_columns: Cell<usize>,
        pub overlay: RefCell<EditOverlay>,
        pub on_add: RefCell<Option<SimpleCb>>,
        pub on_save: RefCell<Option<SimpleCb>>,
        pub on_discard: RefCell<Option<SimpleCb>>,
        pub on_delete_selected: RefCell<Option<SimpleCb>>,
        pub on_edit_row: RefCell<Option<EditRowCb>>,
        /// When false, column-header clicks drive server-side sort (no client reorder).
        pub client_sorting: Cell<bool>,
        pub on_server_sort: RefCell<Option<ServerSortCb>>,
        pub server_sort_wired: Cell<bool>,
        pub on_near_bottom: RefCell<Option<SimpleCb>>,
        pub near_bottom_guard: RefCell<NearBottomGuard>,
        pub near_bottom_wired: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ResultsGrid {
        const NAME: &'static str = "ResultsGrid";
        type Type = super::ResultsGrid;
        type ParentType = adw::Bin;

        fn new() -> Self {
            let root = gtk::Box::new(gtk::Orientation::Vertical, 0);

            let action_bar = gtk::ActionBar::new();
            action_bar.set_revealed(false);

            let readonly_badge = gtk::Label::new(Some("Read-only"));
            readonly_badge.add_css_class("readonly-badge");
            readonly_badge.set_visible(false);

            let add_btn = gtk::Button::with_label("Add Row");
            add_btn.add_css_class("flat");
            add_btn.set_visible(false);

            let delete_btn = gtk::Button::with_label("Delete");
            delete_btn.add_css_class("flat");
            delete_btn.add_css_class("destructive-action");
            delete_btn.set_visible(false);

            let change_badge = gtk::Label::new(None);
            change_badge.add_css_class("change-badge");
            change_badge.set_visible(false);

            let discard_btn = gtk::Button::with_label("Discard All");
            discard_btn.add_css_class("flat");
            discard_btn.set_visible(false);

            let save_btn = gtk::Button::with_label("Save Changes");
            save_btn.add_css_class("suggested-action");
            save_btn.set_visible(false);

            action_bar.pack_start(&readonly_badge);
            action_bar.pack_start(&add_btn);
            action_bar.pack_start(&delete_btn);
            action_bar.pack_end(&save_btn);
            action_bar.pack_end(&discard_btn);
            action_bar.pack_end(&change_badge);

            let status = gtk::Label::new(None);
            status.set_xalign(0.0);
            status.add_css_class("dimmed");
            status.set_margin_start(8);
            status.set_margin_end(8);
            status.set_margin_top(4);
            status.set_margin_bottom(4);
            status.set_ellipsize(gtk::pango::EllipsizeMode::End);

            let model = ResultModel::new();
            let sort_model = SortListModel::new(Some(model.clone()), Option::<gtk::Sorter>::None);
            sort_model.set_incremental(true);
            let selection = MultiSelection::new(Some(sort_model.clone()));

            let column_view = ColumnView::new(Some(selection.clone()));
            column_view.set_reorderable(false);
            column_view.set_show_row_separators(true);
            column_view.set_show_column_separators(true);
            column_view.set_enable_rubberband(true);
            column_view.set_hexpand(true);
            column_view.set_vexpand(true);
            sort_model.set_sorter(column_view.sorter().as_ref());

            let scrolled = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Automatic)
                .vscrollbar_policy(gtk::PolicyType::Automatic)
                .hexpand(true)
                .vexpand(true)
                .child(&column_view)
                .build();

            // Bottom paging indicator (R3), hidden until `set_loading_more`.
            // Mirrors the spinner + caption precedent in schema/browse.rs.
            let loading_more_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            loading_more_row.set_halign(gtk::Align::Center);
            loading_more_row.set_margin_top(4);
            loading_more_row.set_margin_bottom(4);
            let loading_spinner = gtk::Spinner::new();
            loading_spinner.set_spinning(true);
            let loading_caption = gtk::Label::new(Some("Loading more rows…"));
            loading_caption.add_css_class("dimmed");
            loading_more_row.append(&loading_spinner);
            loading_more_row.append(&loading_caption);
            loading_more_row.set_visible(false);

            root.append(&action_bar);
            root.append(&status);
            root.append(&scrolled);
            root.append(&loading_more_row);

            Self {
                root,
                action_bar,
                readonly_badge,
                add_btn,
                delete_btn,
                change_badge,
                discard_btn,
                save_btn,
                status,
                scrolled,
                loading_more_row,
                column_view,
                model: RefCell::new(Some(model)),
                selection: RefCell::new(Some(selection)),
                pending: RefCell::new(Vec::new()),
                flush_source: RefCell::new(None),
                total_columns: Cell::new(0),
                visible_columns: Cell::new(0),
                overlay: RefCell::new(EditOverlay::default()),
                on_add: RefCell::new(None),
                on_save: RefCell::new(None),
                on_discard: RefCell::new(None),
                on_delete_selected: RefCell::new(None),
                on_edit_row: RefCell::new(None),
                client_sorting: Cell::new(true),
                on_server_sort: RefCell::new(None),
                server_sort_wired: Cell::new(false),
                on_near_bottom: RefCell::new(None),
                near_bottom_guard: RefCell::new(NearBottomGuard::default()),
                near_bottom_wired: Cell::new(false),
            }
        }
    }

    impl ObjectImpl for ResultsGrid {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().set_child(Some(&self.root));
            self.obj().wire_toolbar_buttons();
            self.obj().wire_activation();
        }

        fn dispose(&self) {
            if let Some(id) = self.flush_source.borrow_mut().take() {
                id.remove();
            }
        }
    }

    impl WidgetImpl for ResultsGrid {}
    impl BinImpl for ResultsGrid {}
}

glib::wrapper! {
    pub struct ResultsGrid(ObjectSubclass<imp::ResultsGrid>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ResultsGrid {
    pub fn new() -> Self {
        glib::Object::builder().build()
    }

    fn model(&self) -> ResultModel {
        self.imp()
            .model
            .borrow()
            .clone()
            .expect("ResultModel set in subclass::new")
    }

    fn selection(&self) -> MultiSelection {
        self.imp()
            .selection
            .borrow()
            .clone()
            .expect("MultiSelection set in subclass::new")
    }

    fn wire_toolbar_buttons(&self) {
        self.imp().add_btn.connect_clicked(glib::clone!(
            #[weak(rename_to = grid)]
            self,
            move |_| {
                if let Some(cb) = grid.imp().on_add.borrow().as_ref() {
                    cb();
                }
            }
        ));
        self.imp().save_btn.connect_clicked(glib::clone!(
            #[weak(rename_to = grid)]
            self,
            move |_| {
                if let Some(cb) = grid.imp().on_save.borrow().as_ref() {
                    cb();
                }
            }
        ));
        self.imp().discard_btn.connect_clicked(glib::clone!(
            #[weak(rename_to = grid)]
            self,
            move |_| {
                if let Some(cb) = grid.imp().on_discard.borrow().as_ref() {
                    cb();
                }
            }
        ));
        self.imp().delete_btn.connect_clicked(glib::clone!(
            #[weak(rename_to = grid)]
            self,
            move |_| {
                if let Some(cb) = grid.imp().on_delete_selected.borrow().as_ref() {
                    cb();
                }
            }
        ));
        self.selection().connect_selection_changed(glib::clone!(
            #[weak(rename_to = grid)]
            self,
            move |_, _, _| {
                let n = grid.selected_model_indices().len() as u32;
                grid.imp().delete_btn.set_visible(
                    grid.imp().action_bar.is_revealed() && grid.imp().add_btn.is_visible() && n > 0,
                );
            }
        ));
    }

    fn wire_activation(&self) {
        self.imp().column_view.connect_activate(glib::clone!(
            #[weak(rename_to = grid)]
            self,
            move |_, position| {
                let selection = grid.selection();
                let Some(row) = selection.item(position).and_downcast::<RowObject>() else {
                    return;
                };
                if let Some(cb) = grid.imp().on_edit_row.borrow().as_ref() {
                    cb(row.index());
                }
            }
        ));
    }

    pub fn connect_edit_handlers(
        &self,
        on_add: impl Fn() + 'static,
        on_save: impl Fn() + 'static,
        on_discard: impl Fn() + 'static,
        on_delete_selected: impl Fn() + 'static,
        on_edit_row: impl Fn(u32) + 'static,
    ) {
        *self.imp().on_add.borrow_mut() = Some(Rc::new(on_add));
        *self.imp().on_save.borrow_mut() = Some(Rc::new(on_save));
        *self.imp().on_discard.borrow_mut() = Some(Rc::new(on_discard));
        *self.imp().on_delete_selected.borrow_mut() = Some(Rc::new(on_delete_selected));
        *self.imp().on_edit_row.borrow_mut() = Some(Rc::new(on_edit_row));
    }

    /// When `enabled` is false, column-header clicks emit [`Self::connect_server_sort`]
    /// instead of reordering the local model (table-browse parity with EnhancedGrid).
    pub fn set_client_sorting(&self, enabled: bool) {
        self.imp().client_sorting.set(enabled);
        self.sync_sort_model_link();
        if !enabled {
            self.ensure_server_sort_wired();
        }
    }

    pub fn connect_server_sort(&self, on_sort: impl Fn(Option<(String, bool)>) + 'static) {
        *self.imp().on_server_sort.borrow_mut() = Some(Rc::new(on_sort));
        self.ensure_server_sort_wired();
    }

    /// Register a callback fired when the user scrolls near the bottom of the
    /// loaded rows (KTD-6): the viewport's bottom edge coming within roughly
    /// one viewport height of the content end.
    ///
    /// Re-arm semantics: the callback is one-shot per crossing — parked at the
    /// bottom it fires at most once, re-arming only when the content grows (a
    /// page append changes the vadjustment `upper`) or the view scrolls out of
    /// the trigger zone. `clear()`/`begin_columns()` re-arm unconditionally, so
    /// a cleared-and-refilled grid triggers again. The signal is gated on
    /// `upper > page_size`: when all loaded rows fit the viewport it never
    /// fires — use [`Self::content_fits_viewport`] for the viewport-fill
    /// concern instead. The grid is a dumb signal source; whether a fetch
    /// actually happens is the owning tab's decision.
    pub fn connect_near_bottom(&self, on_near_bottom: impl Fn() + 'static) {
        *self.imp().on_near_bottom.borrow_mut() = Some(Rc::new(on_near_bottom));
        self.ensure_near_bottom_wired();
    }

    fn ensure_near_bottom_wired(&self) {
        if self.imp().near_bottom_wired.get() {
            return;
        }
        self.imp().near_bottom_wired.set(true);
        // notify::value covers wheel, trackpad, keyboard, and scrollbar drags —
        // everything that moves the viewport (edge-overshot would miss keys).
        self.imp()
            .scrolled
            .vadjustment()
            .connect_value_notify(glib::clone!(
                #[weak(rename_to = grid)]
                self,
                move |adj| {
                    let fire = grid
                        .imp()
                        .near_bottom_guard
                        .borrow_mut()
                        .evaluate(adj.value(), adj.page_size(), adj.upper());
                    if !fire {
                        return;
                    }
                    if let Some(cb) = grid.imp().on_near_bottom.borrow().as_ref() {
                        cb();
                    }
                }
            ));
    }

    /// Show/hide the bottom loading indicator appended after the scrolled
    /// area (R3) while a chunk fetch is in flight. Hidden by default.
    pub fn set_loading_more(&self, active: bool) {
        self.imp().loading_more_row.set_visible(active);
    }

    /// True while every loaded row fits inside the viewport
    /// (`upper <= page_size`). The near-bottom signal intentionally stays
    /// silent in that state, so the owning tab uses this to keep fetching
    /// chunks until the first fill overflows the viewport (U3 viewport-fill).
    pub fn content_fits_viewport(&self) -> bool {
        let adj = self.imp().scrolled.vadjustment();
        adj.upper() <= adj.page_size()
    }

    /// Paging-aware status line for the paged path: the loaded count plus the
    /// [`PagedStatus`] copy, with the same column-count suffix the non-paged
    /// `update_status`/`finish` produce. `finish`/`show_message_line` stay in
    /// charge of the non-paged path — this is purely additive.
    pub fn show_paged_status(&self, loaded: usize, state: PagedStatus) {
        self.flush_pending();
        self.cancel_flush();
        let total_cols = self.imp().total_columns.get();
        let visible_cols = self.imp().visible_columns.get();
        let mut text = paging_helpers::paged_status_text(loaded, state);
        if total_cols > visible_cols {
            text.push_str(&format!(
                " · showing {visible_cols} of {total_cols} columns"
            ));
        } else if total_cols > 0 {
            text.push_str(&format!(" · {total_cols} columns"));
        }
        self.imp().status.set_text(&text);
    }

    /// Re-arm the near-bottom latch and hide the paging indicator; called on
    /// every new result shape so a fresh run behaves like a fresh grid.
    fn reset_paging_affordances(&self) {
        self.imp().near_bottom_guard.borrow_mut().reset();
        self.set_loading_more(false);
    }

    fn ensure_server_sort_wired(&self) {
        if self.imp().server_sort_wired.get() {
            return;
        }
        self.imp().server_sort_wired.set(true);
        let Some(sorter) = self
            .imp()
            .column_view
            .sorter()
            .and_downcast::<ColumnViewSorter>()
        else {
            tracing::warn!("ColumnView sorter is not a ColumnViewSorter; server sort disabled");
            return;
        };

        let emit = glib::clone!(
            #[weak(rename_to = grid)]
            self,
            move || {
                if grid.imp().client_sorting.get() {
                    return;
                }
                let Some(cb) = grid.imp().on_server_sort.borrow().clone() else {
                    return;
                };
                let Some(sorter) = grid
                    .imp()
                    .column_view
                    .sorter()
                    .and_downcast::<ColumnViewSorter>()
                else {
                    return;
                };
                let spec = sorter.primary_sort_column().map(|col| {
                    let name = col.title().map(|t| t.to_string()).unwrap_or_default();
                    let desc = sorter.primary_sort_order() == SortType::Descending;
                    (name, desc)
                });
                cb(spec);
            }
        );

        sorter.connect_primary_sort_column_notify(glib::clone!(
            #[strong]
            emit,
            move |_| emit()
        ));
        sorter.connect_primary_sort_order_notify(move |_| emit());
    }

    fn sync_sort_model_link(&self) {
        let selection = self.selection();
        let Some(sort_model) = selection.model().and_downcast::<SortListModel>() else {
            return;
        };
        if self.imp().client_sorting.get() {
            if let Some(sorter) = self.imp().column_view.sorter() {
                sort_model.set_sorter(Some(&sorter));
            }
        } else {
            sort_model.set_sorter(gtk::Sorter::NONE);
        }
    }

    pub fn set_edit_toolbar(&self, state: &EditToolbarState) {
        self.imp().action_bar.set_revealed(state.visible);
        if !state.visible {
            return;
        }
        if state.editable {
            self.imp().readonly_badge.set_visible(false);
            self.imp().add_btn.set_visible(true);
            let n = state.selected_count;
            self.imp().delete_btn.set_visible(n > 0);
        } else {
            self.imp().readonly_badge.set_visible(true);
            if let Some(reason) = &state.reason {
                self.imp().readonly_badge.set_tooltip_text(Some(reason));
            } else {
                self.imp().readonly_badge.set_tooltip_text(None);
            }
            self.imp().add_btn.set_visible(false);
            self.imp().delete_btn.set_visible(false);
        }
        let has = state.change_count > 0;
        self.imp().change_badge.set_visible(has);
        if has {
            let n = state.change_count;
            self.imp()
                .change_badge
                .set_text(&format!("{n} change{}", if n == 1 { "" } else { "s" }));
        }
        self.imp().discard_btn.set_visible(has);
        self.imp().save_btn.set_visible(has && state.editable);
    }

    pub fn set_overlay(&self, overlay: EditOverlay) {
        *self.imp().overlay.borrow_mut() = overlay;
        // Force ColumnView to rebind visible cells.
        let n = self.model().n_items();
        if n > 0 {
            self.model().items_changed(0, n, n);
        }
    }

    pub fn clear_overlay(&self) {
        self.set_overlay(EditOverlay::default());
    }

    pub fn column_names(&self) -> Vec<String> {
        self.model().columns().into_iter().map(|c| c.name).collect()
    }

    pub fn selected_model_indices(&self) -> Vec<u32> {
        self.flush_pending();
        let selection = self.selection();
        let mut out = Vec::new();
        let n = selection.n_items();
        for i in 0..n {
            if selection.is_selected(i) {
                if let Some(row) = selection.item(i).and_downcast::<RowObject>() {
                    out.push(row.index());
                }
            }
        }
        out
    }

    pub fn row_map(&self, model_index: u32) -> Option<HashMap<String, serde_json::Value>> {
        let values = self.model().row_values(model_index)?;
        let names = self.column_names();
        let mut map = HashMap::new();
        for (i, name) in names.into_iter().enumerate() {
            let cell = values.get(i).cloned().unwrap_or(CellValue::Null);
            map.insert(name, cell.to_json());
        }
        Some(map)
    }

    pub fn append_empty_row(&self) -> u32 {
        let n_cols = self.imp().visible_columns.get();
        let cells = vec![CellValue::Null; n_cols];
        self.model().append_row(Arc::<[CellValue]>::from(cells))
    }

    pub fn update_row_from_map(&self, model_index: u32, map: &HashMap<String, serde_json::Value>) {
        let names = self.column_names();
        let mut cells = Vec::with_capacity(names.len());
        for name in &names {
            let v = map.get(name).cloned().unwrap_or(serde_json::Value::Null);
            cells.push(CellValue::from_json(&v));
        }
        self.model()
            .set_row_values(model_index, Arc::<[CellValue]>::from(cells));
    }

    pub fn remove_model_row(&self, model_index: u32) {
        self.model().remove_row(model_index);
    }

    pub fn clear(&self) {
        self.cancel_flush();
        self.imp().pending.borrow_mut().clear();
        self.model().clear();
        self.clear_columns();
        self.imp().status.set_text("");
        self.imp().total_columns.set(0);
        self.imp().visible_columns.set(0);
        self.reset_paging_affordances();
        self.clear_overlay();
        self.set_edit_toolbar(&EditToolbarState {
            visible: false,
            editable: false,
            reason: None,
            change_count: 0,
            selected_count: 0,
        });
    }

    /// Start a new result shape. Drops any buffered rows and rebuilds columns.
    pub fn begin_columns(&self, names: Vec<String>) {
        self.cancel_flush();
        self.imp().pending.borrow_mut().clear();
        self.clear_overlay();
        self.reset_paging_affordances();

        let total = names.len();
        let visible = names
            .into_iter()
            .take(VISIBLE_COLUMN_CAP)
            .collect::<Vec<_>>();
        let visible_n = visible.len();
        let metas: Vec<ColumnMeta> = visible.into_iter().map(ColumnMeta::from_name).collect();

        self.imp().total_columns.set(total);
        self.imp().visible_columns.set(visible_n);
        self.model().set_columns(metas);
        self.rebuild_columns();
        self.update_status();
    }

    /// Buffer a streamed row; flushed on a 50ms cadence (Svelte parity).
    pub fn push_row(&self, values: Vec<serde_json::Value>) {
        let visible_n = self.imp().visible_columns.get();
        let cells: Vec<CellValue> = values
            .iter()
            .take(visible_n)
            .map(CellValue::from_json)
            .collect();
        // Pad short rows so column binds never panic on missing cells.
        let mut cells = cells;
        while cells.len() < visible_n {
            cells.push(CellValue::Null);
        }
        self.imp()
            .pending
            .borrow_mut()
            .push(Arc::<[CellValue]>::from(cells));
        self.ensure_flush_scheduled();
    }

    pub fn finish(&self, row_count: usize, duration_ms: u64) {
        self.flush_pending();
        self.cancel_flush();
        let total_cols = self.imp().total_columns.get();
        let visible_cols = self.imp().visible_columns.get();
        let mut text = format!("{row_count} rows in {duration_ms} ms");
        if total_cols > visible_cols {
            text.push_str(&format!(
                " · showing {visible_cols} of {total_cols} columns"
            ));
        } else if total_cols > 0 {
            text.push_str(&format!(" · {total_cols} columns"));
        }
        self.imp().status.set_text(&text);
    }

    pub fn show_message_line(&self, text: &str) {
        self.flush_pending();
        self.cancel_flush();
        self.imp().status.set_text(text);
    }

    pub fn copy_selection_tsv(&self) -> String {
        self.flush_pending();
        let selection = self.selection();
        let n_cols = self.imp().visible_columns.get();
        let mut lines = Vec::new();
        let n = selection.n_items();
        for i in 0..n {
            if !selection.is_selected(i) {
                continue;
            }
            if let Some(row) = selection.item(i).and_downcast::<RowObject>() {
                let fields: Vec<String> =
                    (0..n_cols).map(|c| row.value(c).as_tsv_field()).collect();
                lines.push(fields.join("\t"));
            }
        }
        lines.join("\n")
    }

    fn ensure_flush_scheduled(&self) {
        if self.imp().flush_source.borrow().is_some() {
            return;
        }
        let grid = self.downgrade();
        let id = glib::timeout_add_local_once(FLUSH_INTERVAL, move || {
            if let Some(grid) = grid.upgrade() {
                *grid.imp().flush_source.borrow_mut() = None;
                grid.flush_pending();
                // If more rows arrived during flush, reschedule.
                if !grid.imp().pending.borrow().is_empty() {
                    grid.ensure_flush_scheduled();
                }
            }
        });
        *self.imp().flush_source.borrow_mut() = Some(id);
    }

    fn cancel_flush(&self) {
        if let Some(id) = self.flush_source_take() {
            id.remove();
        }
    }

    fn flush_source_take(&self) -> Option<glib::SourceId> {
        self.imp().flush_source.borrow_mut().take()
    }

    fn flush_pending(&self) {
        let rows = std::mem::take(&mut *self.imp().pending.borrow_mut());
        if !rows.is_empty() {
            self.model().append_rows(rows);
            self.update_status();
        }
    }

    fn update_status(&self) {
        let loaded = self.model().row_count();
        let total_cols = self.imp().total_columns.get();
        let visible_cols = self.imp().visible_columns.get();
        let mut parts = vec![format!("{loaded} rows")];
        if total_cols > visible_cols {
            parts.push(format!("showing {visible_cols} of {total_cols} columns"));
        } else if total_cols > 0 {
            parts.push(format!("{total_cols} columns"));
        }
        self.imp().status.set_text(&parts.join(" · "));
    }

    fn clear_columns(&self) {
        let column_view = &self.imp().column_view;
        while let Some(col) = column_view.columns().item(0) {
            if let Some(c) = col.downcast_ref::<ColumnViewColumn>() {
                column_view.remove_column(c);
            } else {
                break;
            }
        }
    }

    fn rebuild_columns(&self) {
        self.clear_columns();
        let model = self.model();
        let selection = self.selection();
        let column_view = &self.imp().column_view;
        let columns = model.columns();
        let grid = self.downgrade();

        for (col_idx, meta) in columns.iter().enumerate() {
            let factory = SignalListItemFactory::new();
            let selection_setup = selection.clone();
            let grid_setup = grid.clone();

            factory.connect_setup(move |_factory, item| {
                let cell = item
                    .downcast_ref::<ColumnViewCell>()
                    .expect("ColumnView factory yields ColumnViewCell");
                let inscription = Inscription::new(None);
                inscription.set_xalign(0.0);
                inscription.set_min_chars(1);
                inscription.set_hexpand(true);

                let gesture = GestureClick::new();
                gesture.set_button(3);
                {
                    let selection = selection_setup.clone();
                    let cell_weak = cell.downgrade();
                    gesture.connect_pressed(move |_g, _n, _x, _y| {
                        let Some(cell) = cell_weak.upgrade() else {
                            return;
                        };
                        let position = cell.position();
                        if !selection.is_selected(position) {
                            selection.select_item(position, true);
                        }
                    });
                }
                inscription.add_controller(gesture);
                cell.set_child(Some(&inscription));
            });

            factory.connect_bind(glib::clone!(
                #[strong]
                grid_setup,
                move |_factory, item| {
                    let cell = item
                        .downcast_ref::<ColumnViewCell>()
                        .expect("ColumnView factory yields ColumnViewCell");
                    let Some(row) = cell.item().and_downcast::<RowObject>() else {
                        return;
                    };
                    let Some(inscription) = cell.child().and_downcast::<Inscription>() else {
                        return;
                    };
                    let model_idx = row.index();
                    let overlay = grid_setup
                        .upgrade()
                        .map(|g| g.imp().overlay.borrow().clone())
                        .unwrap_or_default();

                    let value = overlay
                        .cell_overrides
                        .get(&(model_idx, col_idx))
                        .cloned()
                        .unwrap_or_else(|| row.value(col_idx));

                    inscription.remove_css_class("null-cell");
                    inscription.remove_css_class("modified-cell");
                    inscription.remove_css_class("deleted-row");
                    inscription.remove_css_class("added-row");

                    if overlay.deleted_rows.contains(&model_idx) {
                        inscription.add_css_class("deleted-row");
                    } else if overlay.added_rows.contains(&model_idx) {
                        inscription.add_css_class("added-row");
                    }
                    if overlay.modified_cells.contains(&(model_idx, col_idx)) {
                        inscription.add_css_class("modified-cell");
                    }

                    if value.is_null() {
                        inscription.set_text(Some("NULL"));
                        inscription.add_css_class("null-cell");
                    } else {
                        inscription.set_text(Some(&value.display()));
                    }
                }
            ));

            factory.connect_unbind(|_factory, item| {
                let cell = item
                    .downcast_ref::<ColumnViewCell>()
                    .expect("ColumnView factory yields ColumnViewCell");
                if let Some(inscription) = cell.child().and_downcast::<Inscription>() {
                    inscription.set_text(None);
                    inscription.remove_css_class("null-cell");
                    inscription.remove_css_class("modified-cell");
                    inscription.remove_css_class("deleted-row");
                    inscription.remove_css_class("added-row");
                }
            });

            let column = ColumnViewColumn::new(Some(&meta.name), Some(factory));
            column.set_resizable(true);
            column.set_expand(false);
            column.set_fixed_width(meta.fixed_width);

            let sorter = CustomSorter::new(move |a, b| {
                let a = a.downcast_ref::<RowObject>().expect("RowObject");
                let b = b.downcast_ref::<RowObject>().expect("RowObject");
                a.value(col_idx).cmp_typed(&b.value(col_idx)).into()
            });
            column.set_sorter(Some(&sorter));
            column_view.append_column(&column);
        }

        // Re-apply client vs server sort linking after column rebuild.
        self.sync_sort_model_link();
    }
}

impl Default for ResultsGrid {
    fn default() -> Self {
        Self::new()
    }
}
