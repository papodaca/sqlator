//! ColumnView results grid widget (phase-0 spike → production).

use crate::results::cell::{CellValue, ColumnMeta};
use crate::results::model::ResultModel;
use crate::results::row::RowObject;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{
    glib, ColumnView, ColumnViewCell, ColumnViewColumn, CustomSorter, GestureClick, Inscription,
    MultiSelection, SignalListItemFactory, SortListModel,
};
use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::time::Duration;

/// Default mounted-column budget from the spike (no horizontal virtualization).
pub const VISIBLE_COLUMN_CAP: usize = 30;

const FLUSH_INTERVAL: Duration = Duration::from_millis(50);

mod imp {
    use super::*;
    use adw::prelude::BinExt;

    #[derive(Default)]
    pub struct ResultsGrid {
        pub root: gtk::Box,
        pub status: gtk::Label,
        pub column_view: ColumnView,
        pub model: RefCell<Option<ResultModel>>,
        pub selection: RefCell<Option<MultiSelection>>,
        pub pending: RefCell<Vec<Arc<[CellValue]>>>,
        pub flush_source: RefCell<Option<glib::SourceId>>,
        pub total_columns: Cell<usize>,
        pub visible_columns: Cell<usize>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ResultsGrid {
        const NAME: &'static str = "ResultsGrid";
        type Type = super::ResultsGrid;
        type ParentType = adw::Bin;

        fn new() -> Self {
            let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
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

            root.append(&status);
            root.append(&scrolled);

            Self {
                root,
                status,
                column_view,
                model: RefCell::new(Some(model)),
                selection: RefCell::new(Some(selection)),
                pending: RefCell::new(Vec::new()),
                flush_source: RefCell::new(None),
                total_columns: Cell::new(0),
                visible_columns: Cell::new(0),
            }
        }
    }

    impl ObjectImpl for ResultsGrid {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().set_child(Some(&self.root));
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

    pub fn clear(&self) {
        self.cancel_flush();
        self.imp().pending.borrow_mut().clear();
        self.model().clear();
        self.clear_columns();
        self.imp().status.set_text("");
        self.imp().total_columns.set(0);
        self.imp().visible_columns.set(0);
    }

    /// Start a new result shape. Drops any buffered rows and rebuilds columns.
    pub fn begin_columns(&self, names: Vec<String>) {
        self.cancel_flush();
        self.imp().pending.borrow_mut().clear();

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
        if let Some(id) = self.imp().flush_source.borrow_mut().take() {
            id.remove();
        }
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

        for (col_idx, meta) in columns.iter().enumerate() {
            let factory = SignalListItemFactory::new();
            let selection_setup = selection.clone();

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

            factory.connect_bind(move |_factory, item| {
                let cell = item
                    .downcast_ref::<ColumnViewCell>()
                    .expect("ColumnView factory yields ColumnViewCell");
                let Some(row) = cell.item().and_downcast::<RowObject>() else {
                    return;
                };
                let Some(inscription) = cell.child().and_downcast::<Inscription>() else {
                    return;
                };
                let value = row.value(col_idx);
                if value.is_null() {
                    inscription.set_text(Some("NULL"));
                    inscription.add_css_class("null-cell");
                } else {
                    inscription.remove_css_class("null-cell");
                    inscription.set_text(Some(&value.display()));
                }
            });

            factory.connect_unbind(|_factory, item| {
                let cell = item
                    .downcast_ref::<ColumnViewCell>()
                    .expect("ColumnView factory yields ColumnViewCell");
                if let Some(inscription) = cell.child().and_downcast::<Inscription>() {
                    inscription.set_text(None);
                    inscription.remove_css_class("null-cell");
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

        // Keep SortListModel sorter linked after column rebuild.
        if let Some(sorter) = column_view.sorter() {
            if let Some(sort_model) = selection.model().and_downcast::<SortListModel>() {
                sort_model.set_sorter(Some(&sorter));
            }
        }
    }
}

impl Default for ResultsGrid {
    fn default() -> Self {
        Self::new()
    }
}
