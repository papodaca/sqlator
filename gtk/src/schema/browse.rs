//! Table browse tab: server-side sort/filter + paged append (`query_table`).
//!
//! Pages of [`LIMIT`] rows, auto-fetched on scroll (and while the first
//! pages still fit the viewport). Safety ceiling is
//! [`sqlator_core::db::PAGED_ROW_CEILING`], same as the query tab.

use crate::application::SqlatorApplication;
use crate::results::{PagedStatus, ResultsGrid};
use crate::window::SqlatorWindow;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use sqlator_core::db::PAGED_ROW_CEILING;
use sqlator_core::models::{FilterSpec, SortSpec, TableQueryParams, TableQueryResult};
use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

const LIMIT: i64 = 500;
const FILTER_DEBOUNCE: Duration = Duration::from_millis(300);

#[derive(Debug, Clone)]
pub struct FilterEntry {
    pub operator: String,
    pub value: String,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct TableBrowseTab {
        pub root: gtk::Box,
        pub title: gtk::Label,
        pub filter_bar: gtk::Box,
        pub filter_column: gtk::DropDown,
        pub filter_op: gtk::DropDown,
        pub filter_value: gtk::Entry,
        pub filter_add: gtk::Button,
        pub filter_clear: gtk::Button,
        pub filter_chips: gtk::Box,
        pub inline_status: gtk::Label,
        pub stack: gtk::Stack,
        pub error_label: gtk::Label,
        pub grid: ResultsGrid,
        pub in_flight: Cell<bool>,
        pub service: OnceCell<Arc<sqlator_service::AppService>>,
        pub window: OnceCell<glib::WeakRef<SqlatorWindow>>,
        pub connection_id: RefCell<String>,
        pub table_name: RefCell<String>,
        pub schema: RefCell<Option<String>>,
        pub persist_id: RefCell<String>,
        pub sort: RefCell<Vec<SortSpec>>,
        pub filters: RefCell<HashMap<String, FilterEntry>>,
        pub column_names: RefCell<Vec<String>>,
        pub column_types: RefCell<Vec<String>>,
        pub offset: Cell<i64>,
        pub total_returned: Cell<usize>,
        pub has_more: Cell<bool>,
        pub has_result: Cell<bool>,
        pub generation: AtomicU64,
        pub filter_debounce: RefCell<Option<glib::SourceId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TableBrowseTab {
        const NAME: &'static str = "TableBrowseTab";
        type Type = super::TableBrowseTab;
        type ParentType = adw::Bin;

        fn new() -> Self {
            ResultsGrid::ensure_type();

            let root = gtk::Box::new(gtk::Orientation::Vertical, 0);

            let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            toolbar.set_margin_start(8);
            toolbar.set_margin_end(8);
            toolbar.set_margin_top(6);
            toolbar.set_margin_bottom(4);

            let title = gtk::Label::builder()
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .hexpand(true)
                .css_classes(["heading"])
                .build();
            toolbar.append(&title);

            let filter_bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            filter_bar.set_margin_start(8);
            filter_bar.set_margin_end(8);
            filter_bar.set_margin_bottom(4);
            filter_bar.set_visible(false);

            let filter_column = gtk::DropDown::from_strings(&[]);
            filter_column.set_hexpand(true);
            filter_column.set_tooltip_text(Some("Filter column"));

            let filter_op = gtk::DropDown::from_strings(&[
                "contains",
                "equals",
                "starts with",
                "ends with",
                ">",
                "≥",
                "<",
                "≤",
                "is null",
                "is not null",
            ]);
            filter_op.set_tooltip_text(Some("Filter operator"));

            let filter_value = gtk::Entry::new();
            filter_value.set_placeholder_text(Some("Value"));
            filter_value.set_hexpand(true);

            let filter_add = gtk::Button::with_label("Add filter");
            filter_add.add_css_class("flat");
            let filter_clear = gtk::Button::with_label("Clear filters");
            filter_clear.add_css_class("flat");
            filter_clear.set_sensitive(false);

            filter_bar.append(&filter_column);
            filter_bar.append(&filter_op);
            filter_bar.append(&filter_value);
            filter_bar.append(&filter_add);
            filter_bar.append(&filter_clear);

            let filter_chips = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            filter_chips.set_margin_start(8);
            filter_chips.set_margin_end(8);
            filter_chips.set_margin_bottom(4);
            filter_chips.set_visible(false);

            let inline_status = gtk::Label::builder()
                .xalign(0.0)
                .margin_start(8)
                .margin_end(8)
                .css_classes(["dimmed", "caption"])
                .build();
            inline_status.set_visible(false);

            let stack = gtk::Stack::new();
            stack.set_hexpand(true);
            stack.set_vexpand(true);

            let loading = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            loading.set_halign(gtk::Align::Center);
            loading.set_valign(gtk::Align::Center);
            let spinner = gtk::Spinner::new();
            spinner.set_spinning(true);
            loading.append(&spinner);
            loading.append(&gtk::Label::new(Some("Loading data…")));
            stack.add_named(&loading, Some("loading"));

            let error_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
            error_box.set_halign(gtk::Align::Center);
            error_box.set_valign(gtk::Align::Center);
            let error_label = gtk::Label::builder()
                .wrap(true)
                .justify(gtk::Justification::Center)
                .css_classes(["error"])
                .build();
            let retry_btn = gtk::Button::with_label("Retry");
            retry_btn.set_halign(gtk::Align::Center);
            error_box.append(&error_label);
            error_box.append(&retry_btn);
            stack.add_named(&error_box, Some("error"));

            let grid = ResultsGrid::new();
            grid.set_client_sorting(false);
            let results_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
            results_box.set_hexpand(true);
            results_box.set_vexpand(true);
            results_box.append(&grid);

            stack.add_named(&results_box, Some("results"));

            root.append(&toolbar);
            root.append(&filter_bar);
            root.append(&filter_chips);
            root.append(&inline_status);
            root.append(&stack);

            Self {
                root,
                title,
                filter_bar,
                filter_column,
                filter_op,
                filter_value,
                filter_add,
                filter_clear,
                filter_chips,
                inline_status,
                stack,
                error_label,
                grid,
                in_flight: Cell::new(false),
                service: OnceCell::new(),
                window: OnceCell::new(),
                connection_id: RefCell::new(String::new()),
                table_name: RefCell::new(String::new()),
                schema: RefCell::new(None),
                persist_id: RefCell::new(String::new()),
                sort: RefCell::new(Vec::new()),
                filters: RefCell::new(HashMap::new()),
                column_names: RefCell::new(Vec::new()),
                column_types: RefCell::new(Vec::new()),
                offset: Cell::new(0),
                total_returned: Cell::new(0),
                has_more: Cell::new(false),
                has_result: Cell::new(false),
                generation: AtomicU64::new(0),
                filter_debounce: RefCell::new(None),
            }
        }
    }

    impl ObjectImpl for TableBrowseTab {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().set_child(Some(&self.root));
        }

        fn dispose(&self) {
            if let Some(id) = self.filter_debounce.borrow_mut().take() {
                id.remove();
            }
        }
    }

    impl WidgetImpl for TableBrowseTab {}
    impl BinImpl for TableBrowseTab {}
}

glib::wrapper! {
    pub struct TableBrowseTab(ObjectSubclass<imp::TableBrowseTab>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl TableBrowseTab {
    pub fn new(
        app: &SqlatorApplication,
        window: &SqlatorWindow,
        connection_id: impl Into<String>,
        table_name: impl Into<String>,
        schema: Option<String>,
    ) -> Self {
        let tab: Self = glib::Object::builder().build();
        tab.imp().service.set(app.service()).ok();
        tab.imp()
            .window
            .set(window.downgrade())
            .expect("window weak ref once");

        let connection_id = connection_id.into();
        let table_name = table_name.into();
        let title = match schema.as_deref() {
            Some(s) if !s.is_empty() => format!("{s}.{table_name}"),
            _ => table_name.clone(),
        };
        tab.imp().title.set_text(&title);
        *tab.imp().connection_id.borrow_mut() = connection_id;
        *tab.imp().table_name.borrow_mut() = table_name;
        *tab.imp().schema.borrow_mut() = schema;
        *tab.imp().persist_id.borrow_mut() = crate::session::new_tab_id();

        tab.wire_controls();
        tab.fetch(0, false);
        tab
    }

    /// Restore a browse tab from session state (sort/filters applied before first fetch).
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        app: &SqlatorApplication,
        window: &SqlatorWindow,
        connection_id: impl Into<String>,
        table_name: impl Into<String>,
        schema: Option<String>,
        persist_id: impl Into<String>,
        sort: Vec<SortSpec>,
        filters: Vec<FilterSpec>,
    ) -> Self {
        let tab: Self = glib::Object::builder().build();
        tab.imp().service.set(app.service()).ok();
        tab.imp()
            .window
            .set(window.downgrade())
            .expect("window weak ref once");

        let connection_id = connection_id.into();
        let table_name = table_name.into();
        let title = match schema.as_deref() {
            Some(s) if !s.is_empty() => format!("{s}.{table_name}"),
            _ => table_name.clone(),
        };
        tab.imp().title.set_text(&title);
        *tab.imp().connection_id.borrow_mut() = connection_id;
        *tab.imp().table_name.borrow_mut() = table_name;
        *tab.imp().schema.borrow_mut() = schema;
        *tab.imp().persist_id.borrow_mut() = persist_id.into();
        *tab.imp().sort.borrow_mut() = sort;
        {
            let mut map = tab.imp().filters.borrow_mut();
            map.clear();
            for f in filters {
                map.insert(
                    f.column,
                    FilterEntry {
                        operator: f.operator,
                        value: f
                            .value
                            .map(|v| match v {
                                serde_json::Value::String(s) => s,
                                other => other.to_string(),
                            })
                            .unwrap_or_default(),
                    },
                );
            }
        }

        tab.wire_controls();
        // Defer fetch until reconnect — session restore may run before connect.
        tab
    }

    /// Re-fetch from offset 0 (after restore reconnect or preference change).
    pub fn reload(&self) {
        self.fetch(0, false);
    }

    fn wire_controls(&self) {
        let tab = self;
        tab.imp().grid.connect_server_sort(glib::clone!(
            #[weak]
            tab,
            move |spec| {
                tab.on_server_sort(spec);
            }
        ));

        // Scrolling near the bottom loads the next page (R2); the grid only
        // signals proximity — the gating state check lives in maybe_load_more.
        tab.imp().grid.connect_near_bottom(glib::clone!(
            #[weak]
            tab,
            move || tab.maybe_load_more()
        ));

        tab.imp().filter_add.connect_clicked(glib::clone!(
            #[weak]
            tab,
            move |_| tab.add_filter_from_bar()
        ));
        tab.imp().filter_clear.connect_clicked(glib::clone!(
            #[weak]
            tab,
            move |_| tab.clear_filters()
        ));
        tab.imp().filter_value.connect_activate(glib::clone!(
            #[weak]
            tab,
            move |_| tab.add_filter_from_bar()
        ));

        // Retry button is the second child of the error page.
        if let Some(error_page) = tab.imp().stack.child_by_name("error") {
            if let Some(retry) = error_page
                .downcast_ref::<gtk::Box>()
                .and_then(|b| b.last_child())
                .and_then(|w| w.downcast::<gtk::Button>().ok())
            {
                retry.connect_clicked(glib::clone!(
                    #[weak]
                    tab,
                    move |_| {
                        tab.fetch(0, false);
                    }
                ));
            }
        }
    }

    pub fn connection_id(&self) -> String {
        self.imp().connection_id.borrow().clone()
    }

    pub fn table_name(&self) -> String {
        self.imp().table_name.borrow().clone()
    }

    pub fn schema(&self) -> Option<String> {
        self.imp().schema.borrow().clone()
    }

    pub fn persist_id(&self) -> String {
        self.imp().persist_id.borrow().clone()
    }

    pub fn set_persist_id(&self, id: impl Into<String>) {
        *self.imp().persist_id.borrow_mut() = id.into();
    }

    pub fn sort_specs(&self) -> Vec<SortSpec> {
        self.imp().sort.borrow().clone()
    }

    pub fn filter_specs(&self) -> Vec<FilterSpec> {
        self.current_filter_specs()
    }

    pub fn matches_table(&self, table_name: &str, schema: Option<&str>) -> bool {
        self.table_name() == table_name && self.schema().as_deref() == schema
    }

    pub fn set_connection_id(&self, id: Option<String>) {
        if let Some(id) = id {
            *self.imp().connection_id.borrow_mut() = id;
        }
    }

    fn schedule_session_save(&self) {
        if let Some(window) = self.imp().window.get().and_then(|w| w.upgrade()) {
            window.schedule_session_save();
        }
    }

    fn on_server_sort(&self, spec: Option<(String, bool)>) {
        {
            let mut sort = self.imp().sort.borrow_mut();
            sort.clear();
            if let Some((column, desc)) = spec {
                if !column.is_empty() {
                    sort.push(SortSpec { column, desc });
                }
            }
        }
        self.schedule_session_save();
        self.fetch(0, false);
    }

    fn operator_from_dropdown(index: u32) -> &'static str {
        match index {
            0 => "contains",
            1 => "equals",
            2 => "startsWith",
            3 => "endsWith",
            4 => "gt",
            5 => "gte",
            6 => "lt",
            7 => "lte",
            8 => "isNull",
            9 => "isNotNull",
            _ => "contains",
        }
    }

    fn operator_label(op: &str) -> &str {
        match op {
            "contains" => "contains",
            "equals" => "equals",
            "startsWith" => "starts with",
            "endsWith" => "ends with",
            "gt" => ">",
            "gte" => "≥",
            "lt" => "<",
            "lte" => "≤",
            "isNull" => "is null",
            "isNotNull" => "is not null",
            other => other,
        }
    }

    fn add_filter_from_bar(&self) {
        let cols = self.imp().column_names.borrow();
        let idx = self.imp().filter_column.selected() as usize;
        let Some(column) = cols.get(idx).cloned() else {
            return;
        };
        drop(cols);

        let op = Self::operator_from_dropdown(self.imp().filter_op.selected()).to_string();
        let value = self.imp().filter_value.text().to_string();
        if op != "isNull" && op != "isNotNull" && value.trim().is_empty() {
            return;
        }

        self.imp().filters.borrow_mut().insert(
            column,
            FilterEntry {
                operator: op,
                value,
            },
        );
        self.imp().filter_value.set_text("");
        self.rebuild_filter_chips();
        self.schedule_session_save();
        self.schedule_filter_fetch();
    }

    fn clear_filters(&self) {
        self.imp().filters.borrow_mut().clear();
        self.rebuild_filter_chips();
        self.schedule_session_save();
        self.fetch(0, false);
    }

    fn remove_filter(&self, column: &str) {
        self.imp().filters.borrow_mut().remove(column);
        self.rebuild_filter_chips();
        self.schedule_session_save();
        self.schedule_filter_fetch();
    }

    fn schedule_filter_fetch(&self) {
        if let Some(id) = self.imp().filter_debounce.borrow_mut().take() {
            id.remove();
        }
        let tab = self.downgrade();
        let id = glib::timeout_add_local_once(FILTER_DEBOUNCE, move || {
            if let Some(tab) = tab.upgrade() {
                *tab.imp().filter_debounce.borrow_mut() = None;
                tab.fetch(0, false);
            }
        });
        *self.imp().filter_debounce.borrow_mut() = Some(id);
    }

    fn rebuild_filter_chips(&self) {
        let chips = &self.imp().filter_chips;
        while let Some(child) = chips.first_child() {
            chips.remove(&child);
        }
        let filters: Vec<(String, FilterEntry)> = self
            .imp()
            .filters
            .borrow()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let has = !filters.is_empty();
        chips.set_visible(has);
        self.imp().filter_clear.set_sensitive(has);
        for (column, entry) in filters {
            let label = if entry.operator == "isNull" || entry.operator == "isNotNull" {
                format!("{column} {}", Self::operator_label(&entry.operator))
            } else {
                format!(
                    "{column} {} {}",
                    Self::operator_label(&entry.operator),
                    entry.value
                )
            };
            let chip = gtk::Box::new(gtk::Orientation::Horizontal, 2);
            chip.add_css_class("card");
            chip.set_margin_end(2);
            let text = gtk::Label::new(Some(&label));
            text.set_margin_start(6);
            text.set_margin_end(2);
            text.set_margin_top(2);
            text.set_margin_bottom(2);
            let remove = gtk::Button::from_icon_name("window-close-symbolic");
            remove.add_css_class("flat");
            remove.add_css_class("circular");
            remove.set_tooltip_text(Some("Remove filter"));
            remove.connect_clicked(glib::clone!(
                #[weak(rename_to = tab)]
                self,
                #[strong]
                column,
                move |_| tab.remove_filter(&column)
            ));
            chip.append(&text);
            chip.append(&remove);
            chips.append(&chip);
        }
    }

    fn current_filter_specs(&self) -> Vec<FilterSpec> {
        self.imp()
            .filters
            .borrow()
            .iter()
            .filter_map(|(column, entry)| {
                let nullary = entry.operator == "isNull" || entry.operator == "isNotNull";
                if !nullary && entry.value.trim().is_empty() {
                    return None;
                }
                let value = if nullary {
                    None
                } else {
                    Some(parse_filter_value(&entry.value))
                };
                Some(FilterSpec {
                    column: column.clone(),
                    operator: entry.operator.clone(),
                    value,
                })
            })
            .collect()
    }

    fn load_more(&self) {
        let next = self.imp().offset.get() + self.imp().total_returned.get() as i64;
        if next as usize >= PAGED_ROW_CEILING {
            return;
        }
        self.fetch(next, true);
    }

    /// Scroll-driven next page (R2): fire `load_more` from the grid's
    /// near-bottom signal, gated on a loaded result, more rows under the
    /// ceiling, and no fetch in flight — the state check replaces the gate
    /// the removed button's disabled state provided.
    fn maybe_load_more(&self) {
        let next = self.imp().offset.get() + self.imp().total_returned.get() as i64;
        if should_fetch_more(
            self.imp().has_result.get(),
            self.imp().has_more.get(),
            self.imp().in_flight.get(),
            next,
            PAGED_ROW_CEILING,
        ) {
            self.load_more();
        }
    }

    /// Keep fetching while the first pages still fit the viewport (U3).
    fn maybe_fill_viewport(&self) {
        let tab = self.downgrade();
        glib::idle_add_local_once(move || {
            let Some(tab) = tab.upgrade() else {
                return;
            };
            if tab.imp().grid.content_fits_viewport() {
                tab.maybe_load_more();
            }
        });
    }

    fn fetch(&self, offset: i64, append: bool) {
        let Some(service) = self.imp().service.get().cloned() else {
            return;
        };
        let connection_id = self.connection_id();
        let table_name = self.table_name();
        let schema = self.schema();
        let sort = self.imp().sort.borrow().clone();
        let filters = self.current_filter_specs();
        let gen = self.imp().generation.fetch_add(1, Ordering::Relaxed) + 1;

        self.imp().offset.set(offset);
        self.imp().in_flight.set(true);
        if append {
            self.imp().grid.set_loading_more(true);
        } else if !self.imp().has_result.get() {
            self.imp().stack.set_visible_child_name("loading");
        } else {
            self.imp().inline_status.set_text("Updating…");
            self.imp().inline_status.set_visible(true);
        }

        let params = TableQueryParams {
            connection_id: connection_id.clone(),
            table_name,
            schema,
            sort,
            filters,
            limit: LIMIT,
            offset,
        };

        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = tab)]
            self,
            async move {
                let db = service.db_handle();
                let result =
                    crate::spawn_tokio!(
                        async move { db.query_table(&connection_id, &params).await }
                    )
                    .await
                    .expect("join query_table");

                if tab.imp().generation.load(Ordering::Relaxed) != gen {
                    return;
                }

                tab.imp().in_flight.set(false);
                tab.imp().grid.set_loading_more(false);
                tab.imp().inline_status.set_visible(false);
                match result {
                    Ok(data) => tab.apply_result(data, append),
                    Err(e) => {
                        tab.imp().error_label.set_text(&e.to_string());
                        tab.imp().stack.set_visible_child_name("error");
                    }
                }
            }
        ));
    }

    fn apply_result(&self, data: TableQueryResult, append: bool) {
        let grid = &self.imp().grid;
        if !append {
            grid.begin_columns(data.columns.clone());
            *self.imp().column_names.borrow_mut() = data.columns.clone();
            *self.imp().column_types.borrow_mut() = data.column_types.clone();
            self.imp().total_returned.set(0);
            self.update_filter_column_model(&data.columns);
            self.imp().filter_bar.set_visible(!data.columns.is_empty());
        }

        for row in &data.rows {
            let values = project_object_row(row, &data.columns);
            grid.push_row(values);
        }

        let returned = if append {
            self.imp().total_returned.get() + data.total_returned
        } else {
            data.total_returned
        };
        // Prefer counting projected rows if the driver under-reports.
        let returned = returned.max(if append {
            self.imp().total_returned.get() + data.rows.len()
        } else {
            data.rows.len()
        });
        let capped = returned >= PAGED_ROW_CEILING;
        self.imp()
            .total_returned
            .set(returned.min(PAGED_ROW_CEILING));
        self.imp()
            .has_more
            .set(data.has_more && !capped);
        self.imp().has_result.set(true);

        let loaded = self.imp().total_returned.get();
        let status = if capped {
            PagedStatus::Capped
        } else if self.imp().has_more.get() {
            PagedStatus::More
        } else {
            PagedStatus::Exhausted
        };
        grid.show_paged_status(loaded, status);

        self.imp().stack.set_visible_child_name("results");
        self.maybe_fill_viewport();
    }

    fn update_filter_column_model(&self, columns: &[String]) {
        let strings: Vec<&str> = columns.iter().map(|s| s.as_str()).collect();
        let model = gtk::StringList::new(&strings);
        self.imp().filter_column.set_model(Some(&model));
        if !columns.is_empty() {
            self.imp().filter_column.set_selected(0);
        }
    }
}

/// Gate for the scroll-driven next-page trigger (R2): fire only when a
/// result is loaded, more rows exist, no fetch is in flight, and the next
/// offset sits below [`PAGED_ROW_CEILING`] (mirrors `load_more`'s check).
fn should_fetch_more(
    has_result: bool,
    has_more: bool,
    in_flight: bool,
    next_offset: i64,
    max_rows: usize,
) -> bool {
    has_result && has_more && !in_flight && (next_offset as usize) < max_rows
}

fn parse_filter_value(v: &str) -> serde_json::Value {
    let trimmed = v.trim();
    if let Ok(n) = trimmed.parse::<i64>() {
        return serde_json::json!(n);
    }
    if let Ok(n) = trimmed.parse::<f64>() {
        return serde_json::json!(n);
    }
    serde_json::Value::String(v.to_string())
}

fn project_object_row(row: &serde_json::Value, columns: &[String]) -> Vec<serde_json::Value> {
    let obj = row.as_object();
    columns
        .iter()
        .map(|col| {
            obj.and_then(|m| m.get(col))
                .cloned()
                .unwrap_or(serde_json::Value::Null)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::should_fetch_more;
    use sqlator_core::db::PAGED_ROW_CEILING;

    #[test]
    fn ceiling_or_exhausted_result_blocks_trigger() {
        assert!(!should_fetch_more(true, false, false, 49_500, PAGED_ROW_CEILING));
        assert!(!should_fetch_more(
            true,
            true,
            false,
            PAGED_ROW_CEILING as i64,
            PAGED_ROW_CEILING
        ));
        assert!(!should_fetch_more(
            true,
            true,
            false,
            (PAGED_ROW_CEILING + 500) as i64,
            PAGED_ROW_CEILING
        ));
    }

    #[test]
    fn stays_silent_until_first_result_applies() {
        // After refresh()/restore reset (offset back to 0, has_result false
        // until the first apply), the trigger is not armed.
        assert!(!should_fetch_more(false, false, false, 0, PAGED_ROW_CEILING));
        assert!(!should_fetch_more(false, true, false, 0, PAGED_ROW_CEILING));
    }

    #[test]
    fn armed_after_apply_with_more_rows() {
        // After the first apply post-refresh, and after a filter/sort
        // re-fetch (offset reset to 0) landing a fresh result with more
        // rows under the ceiling, the trigger re-arms.
        assert!(should_fetch_more(true, true, false, 500, PAGED_ROW_CEILING));
    }

    #[test]
    fn in_flight_fetch_blocks_trigger() {
        // A near-bottom signal while a fetch is in flight must not start
        // another — exactly one fetch in flight, enforced by state now that
        // the button's disabled state is gone.
        assert!(!should_fetch_more(true, true, true, 500, PAGED_ROW_CEILING));
    }
}