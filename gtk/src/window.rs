use crate::application::SqlatorApplication;
use crate::connection::{status_map_from_service, ConnectionList, ConnectionStatus};
use crate::query_tab::QueryTab;
use crate::schema::{
    replace_store_with_columns, replace_store_with_error, replace_store_with_tables, SchemaDdlTab,
    SchemaTree, TableBrowseTab,
};
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib, CompositeTemplate};
use std::collections::HashMap;

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    /// One workspace page detached from `tab_view` while its connection is inactive.
    #[derive(Debug, Clone)]
    pub enum StashedPage {
        Query(QueryTab),
        TableBrowse(TableBrowseTab),
        SchemaDdl(SchemaDdlTab),
    }

    impl StashedPage {
        pub fn widget(&self) -> gtk::Widget {
            match self {
                Self::Query(t) => t.clone().upcast(),
                Self::TableBrowse(t) => t.clone().upcast(),
                Self::SchemaDdl(t) => t.clone().upcast(),
            }
        }

        pub fn set_connection_id(&self, id: Option<String>) {
            match self {
                Self::Query(t) => t.set_connection_id(id),
                Self::TableBrowse(t) => t.set_connection_id(id),
                Self::SchemaDdl(t) => t.set_connection_id(id),
            }
        }

        pub fn as_query(&self) -> Option<&QueryTab> {
            match self {
                Self::Query(t) => Some(t),
                _ => None,
            }
        }

        pub fn has_unsaved_edits(&self) -> bool {
            self.as_query().is_some_and(|t| t.has_unsaved_edits())
        }

        pub fn connection_matches(&self, id: &str) -> bool {
            match self {
                Self::Query(t) => t.connection_id().as_deref() == Some(id),
                Self::TableBrowse(t) => t.connection_id() == id,
                Self::SchemaDdl(t) => t.connection_id() == id,
            }
        }
    }

    /// One page detached from `tab_view` while its connection is inactive.
    #[derive(Debug, Clone)]
    pub struct StashedTab {
        pub title: String,
        pub page: StashedPage,
    }

    /// Full tab set for a connection that is not the active workspace.
    #[derive(Debug, Clone)]
    pub struct StashedWorkspace {
        pub tabs: Vec<StashedTab>,
        pub selected_index: u32,
    }

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/im/apodaca/SqlatorGtk/ui/window.ui")]
    pub struct SqlatorWindow {
        #[template_child]
        pub split_view: TemplateChild<adw::OverlaySplitView>,
        #[template_child]
        pub content_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub tab_view: TemplateChild<adw::TabView>,
        #[template_child]
        pub tab_bar: TemplateChild<adw::TabBar>,
        #[template_child]
        pub tab_overview: TemplateChild<adw::TabOverview>,
        #[template_child]
        pub overview_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub connection_tabs_scroll: TemplateChild<gtk::ScrolledWindow>,
        #[template_child]
        pub connection_tabs_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub connection_list: TemplateChild<gtk::ListView>,
        #[template_child]
        pub schema_list: TemplateChild<gtk::ListView>,
        #[template_child]
        pub schema_banner: TemplateChild<gtk::Label>,
        #[template_child]
        pub schema_refresh: TemplateChild<gtk::Button>,

        pub settings: OnceCell<gio::Settings>,
        pub connections: RefCell<Option<ConnectionList>>,
        pub schema_tree: RefCell<Option<SchemaTree>>,
        /// Sidebar list highlight / rebuild anchor (may track focus/selection).
        pub selected_connection_id: RefCell<Option<String>>,
        /// Connection the schema pane browses. Only changes on activate/connect,
        /// never on transient list selection (e.g. focus-follows-mouse hover).
        pub schema_connection_id: RefCell<Option<String>>,
        /// In-flight / error status overlays on top of `db.is_connected`.
        pub connection_status: RefCell<HashMap<String, ConnectionStatus>>,
        /// Display names for connection tab labels (id → name).
        pub connection_names: RefCell<HashMap<String, String>>,
        /// Open connection workspaces not currently mounted in `tab_view`.
        pub stashed_workspaces: RefCell<HashMap<String, StashedWorkspace>>,
        /// Connection whose query tabs are currently in `tab_view`.
        pub active_workspace_id: RefCell<Option<String>>,
        /// Open order for the connection tab bar.
        pub open_connection_ids: RefCell<Vec<String>>,
        /// Skip busy/last-tab guards while draining pages for workspace switch/close.
        pub force_close_pages: Cell<bool>,
    }

    use std::cell::OnceCell;

    #[glib::object_subclass]
    impl ObjectSubclass for SqlatorWindow {
        const NAME: &'static str = "SqlatorWindow";
        type Type = super::SqlatorWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[gtk::template_callbacks]
    impl SqlatorWindow {
        #[template_callback]
        fn on_tab_overview_create_tab(&self, _overview: &adw::TabOverview) -> adw::TabPage {
            // AdwTabOverview requires a page; only reachable when a workspace is active.
            self.obj().add_query_tab(None)
        }
    }

    impl ObjectImpl for SqlatorWindow {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            let settings = gio::Settings::new("im.apodaca.SqlatorGtk");
            let width = settings.get::<i32>("window-width");
            let height = settings.get::<i32>("window-height");
            let maximized = settings.get::<bool>("window-maximized");
            obj.set_default_size(width, height);
            if maximized {
                obj.maximize();
            }

            let sidebar_visible = settings.get::<bool>("sidebar-visible");
            self.split_view.set_show_sidebar(sidebar_visible);
            settings
                .bind("sidebar-visible", &*self.split_view, "show-sidebar")
                .build();

            self.settings
                .set(settings)
                .expect("settings set once in constructed");

            obj.setup_actions();
            obj.setup_tab_view();
            // Connection list + initial tab need GtkWindow:application, which is
            // not readable until after ObjectBuilder::build returns — see ::new().

            // Persist geometry on close; prompt if any tab has unsaved result edits.
            obj.connect_close_request(glib::clone!(
                #[weak]
                obj,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_| {
                    if let Some(settings) = obj.imp().settings.get() {
                        let (width, height) = obj.default_size();
                        let _ = settings.set("window-width", width);
                        let _ = settings.set("window-height", height);
                        let _ = settings.set("window-maximized", obj.is_maximized());
                    }
                    if obj.has_any_unsaved_edits() {
                        let dialog = adw::AlertDialog::new(
                            Some("Unsaved changes"),
                            Some(
                                "One or more tabs have unsaved result edits. Discard them and close Sqlator?",
                            ),
                        );
                        dialog.add_response("keep", "Keep Open");
                        dialog.add_response("discard", "Discard & Close");
                        dialog.set_response_appearance(
                            "discard",
                            adw::ResponseAppearance::Destructive,
                        );
                        dialog.set_default_response(Some("keep"));
                        dialog.set_close_response("keep");
                        dialog.connect_response(
                            None,
                            glib::clone!(
                                #[weak]
                                obj,
                                move |_, response| {
                                    if response == "discard" {
                                        obj.discard_all_edits();
                                        obj.destroy();
                                    }
                                }
                            ),
                        );
                        dialog.present(Some(&obj));
                        return glib::Propagation::Stop;
                    }
                    glib::Propagation::Proceed
                }
            ));
        }
    }

    impl WidgetImpl for SqlatorWindow {}
    impl WindowImpl for SqlatorWindow {}
    impl ApplicationWindowImpl for SqlatorWindow {}
    impl AdwApplicationWindowImpl for SqlatorWindow {}
}

glib::wrapper! {
    pub struct SqlatorWindow(ObjectSubclass<imp::SqlatorWindow>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
                    gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl SqlatorWindow {
    pub fn new(app: &SqlatorApplication) -> Self {
        let window: Self = glib::Object::builder().property("application", app).build();

        // Load CSS once per display.
        let provider = gtk::CssProvider::new();
        provider.load_from_resource("/im/apodaca/SqlatorGtk/style.css");
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        // Must run after build(): application() is None during constructed().
        window.setup_connection_list();
        window.setup_schema_tree();
        window.show_empty_workspace();

        window
    }

    fn settings(&self) -> &gio::Settings {
        self.imp().settings.get().expect("settings initialized")
    }

    fn setup_actions(&self) {
        let toggle_sidebar = gio::SimpleAction::new("toggle-sidebar", None);
        toggle_sidebar.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                let split = &window.imp().split_view;
                split.set_show_sidebar(!split.shows_sidebar());
            }
        ));
        self.add_action(&toggle_sidebar);

        let new_tab = gio::SimpleAction::new("new-tab", None);
        new_tab.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                if window.imp().active_workspace_id.borrow().is_some() {
                    window.add_query_tab(None);
                }
            }
        ));
        self.add_action(&new_tab);

        let close_tab = gio::SimpleAction::new("close-tab", None);
        close_tab.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                let tab_view = &window.imp().tab_view;
                if let Some(page) = tab_view.selected_page() {
                    tab_view.close_page(&page);
                }
            }
        ));
        self.add_action(&close_tab);

        let tab_overview = gio::SimpleAction::new("tab-overview", None);
        tab_overview.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                window.imp().tab_overview.set_open(true);
            }
        ));
        self.add_action(&tab_overview);

        // HeaderBar play/stop use action-name "tab.*". Those buttons are not
        // descendants of QueryTab, so the per-tab action group is invisible to
        // them — proxy through the selected page here.
        let tab_group = gio::SimpleActionGroup::new();
        let run = gio::SimpleAction::new("run", None);
        run.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                if let Some(tab) = window.selected_query_tab() {
                    tab.run_editor_query();
                }
            }
        ));
        tab_group.add_action(&run);

        let run_all = gio::SimpleAction::new("run-all", None);
        run_all.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                if let Some(tab) = window.selected_query_tab() {
                    tab.run_editor_query();
                }
            }
        ));
        tab_group.add_action(&run_all);

        let run_selection = gio::SimpleAction::new("run-selection", None);
        run_selection.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                if let Some(tab) = window.selected_query_tab() {
                    tab.run_selection_query();
                }
            }
        ));
        tab_group.add_action(&run_selection);

        let cancel = gio::SimpleAction::new("cancel", None);
        cancel.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                if let Some(tab) = window.selected_query_tab() {
                    tab.cancel_query();
                }
            }
        ));
        tab_group.add_action(&cancel);

        self.insert_action_group("tab", Some(&tab_group));
    }

    fn selected_query_tab(&self) -> Option<QueryTab> {
        self.imp()
            .tab_view
            .selected_page()
            .and_then(|page| page.child().downcast::<QueryTab>().ok())
    }

    fn setup_tab_view(&self) {
        let tab_view = self.imp().tab_view.clone();
        // create-tab on TabOverview is wired via #[template_callback].
        tab_view.connect_create_window(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            None,
            move |_| {
                let app = window
                    .application()
                    .and_downcast::<SqlatorApplication>()
                    .expect("SqlatorApplication");
                let new_window = SqlatorWindow::new(&app);
                new_window.present();
                Some(new_window.imp().tab_view.clone())
            }
        ));

        tab_view.connect_close_page(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |tab_view, page| {
                if window.imp().force_close_pages.get() {
                    return glib::Propagation::Proceed;
                }

                // Match Svelte: keep at least one query tab per connection workspace.
                if tab_view.n_pages() <= 1 {
                    tab_view.close_page_finish(page, false);
                    return glib::Propagation::Stop;
                }

                let child = page.child();
                if let Ok(tab) = child.clone().downcast::<QueryTab>() {
                    if tab.is_busy() {
                        let dialog = adw::AlertDialog::new(
                            Some("Query running"),
                            Some("A query is still running in this tab. Cancel it and close?"),
                        );
                        dialog.add_response("keep", "Keep Open");
                        dialog.add_response("close", "Cancel & Close");
                        dialog
                            .set_response_appearance("close", adw::ResponseAppearance::Destructive);
                        dialog.set_default_response(Some("keep"));
                        dialog.set_close_response("keep");

                        let page = page.clone();
                        let tab_view = tab_view.clone();
                        dialog.connect_response(
                            None,
                            glib::clone!(
                                #[weak]
                                tab,
                                move |_, response| {
                                    if response == "close" {
                                        tab.cancel_query();
                                        if tab.has_unsaved_edits() {
                                            tab.discard_edits();
                                        }
                                        tab_view.close_page_finish(&page, true);
                                    } else {
                                        tab_view.close_page_finish(&page, false);
                                    }
                                }
                            ),
                        );
                        dialog.present(Some(&window));
                        return glib::Propagation::Stop;
                    }
                    if tab.has_unsaved_edits() {
                        let dialog = adw::AlertDialog::new(
                            Some("Unsaved changes"),
                            Some("This tab has unsaved result edits. Discard them and close?"),
                        );
                        dialog.add_response("keep", "Keep Open");
                        dialog.add_response("discard", "Discard & Close");
                        dialog.set_response_appearance(
                            "discard",
                            adw::ResponseAppearance::Destructive,
                        );
                        dialog.set_default_response(Some("keep"));
                        dialog.set_close_response("keep");

                        let page = page.clone();
                        let tab_view = tab_view.clone();
                        dialog.connect_response(
                            None,
                            glib::clone!(
                                #[weak]
                                tab,
                                move |_, response| {
                                    if response == "discard" {
                                        tab.discard_edits();
                                        tab_view.close_page_finish(&page, true);
                                    } else {
                                        tab_view.close_page_finish(&page, false);
                                    }
                                }
                            ),
                        );
                        dialog.present(Some(&window));
                        return glib::Propagation::Stop;
                    }
                }
                glib::Propagation::Proceed
            }
        ));
    }

    fn setup_connection_list(&self) {
        let list = ConnectionList::attach(self, &self.imp().connection_list);
        *self.imp().connections.borrow_mut() = Some(list);
        self.refresh_connections();
    }

    fn setup_schema_tree(&self) {
        let tree = SchemaTree::attach(self, &self.imp().schema_list);
        *self.imp().schema_tree.borrow_mut() = Some(tree);
        self.imp().schema_refresh.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                window.reload_schema_browser();
            }
        ));
        self.update_schema_browser();
    }

    pub fn refresh_connections(&self) {
        let app = self
            .application()
            .and_downcast::<SqlatorApplication>()
            .expect("SqlatorApplication");
        let service = app.service();
        let selected = self.selected_connection_id();
        let overlay = self.imp().connection_status.borrow().clone();

        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let service_groups = service.clone();
                let service_conns = service.clone();
                let groups_res =
                    crate::spawn_tokio!(async move { service_groups.get_groups().await })
                        .await
                        .expect("join get_groups");
                let conns_res =
                    crate::spawn_tokio!(async move { service_conns.list_connections().await })
                        .await
                        .expect("join list_connections");

                let groups = match groups_res {
                    Ok(g) => g,
                    Err(e) => {
                        tracing::warn!("get_groups failed: {e}");
                        Vec::new()
                    }
                };
                let conns = match conns_res {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!("list_connections failed: {e}");
                        return;
                    }
                };

                let ids: Vec<String> = conns.iter().map(|c| c.id.clone()).collect();
                {
                    let mut names = window.imp().connection_names.borrow_mut();
                    for c in &conns {
                        names.insert(c.id.clone(), c.name.clone());
                    }
                }
                let mut status = status_map_from_service(&service, &ids);
                for (id, st) in overlay {
                    // Preserve in-flight / error until pool state supersedes.
                    match st {
                        ConnectionStatus::Connecting | ConnectionStatus::Error => {
                            if !matches!(status.get(&id), Some(ConnectionStatus::Connected)) {
                                status.insert(id, st);
                            }
                        }
                        ConnectionStatus::Connected | ConnectionStatus::Disconnected => {}
                    }
                }

                if let Some(list) = window.imp().connections.borrow().as_ref() {
                    list.rebuild(groups, conns, &status, selected.as_deref());
                }
                window.rebuild_connection_tab_bar();
            }
        ));
    }

    pub fn selected_connection_id(&self) -> Option<String> {
        self.imp().selected_connection_id.borrow().clone()
    }

    pub fn set_selected_connection_id(&self, id: Option<String>) {
        *self.imp().selected_connection_id.borrow_mut() = id;
    }

    pub fn schema_connection_id(&self) -> Option<String> {
        self.imp().schema_connection_id.borrow().clone()
    }

    /// Point the schema pane at `id` (None clears). No-op when unchanged.
    pub fn set_schema_connection_id(&self, id: Option<String>) {
        let changed = self.imp().schema_connection_id.borrow().as_deref() != id.as_deref();
        *self.imp().schema_connection_id.borrow_mut() = id;
        if changed {
            self.update_schema_browser();
        }
    }

    fn set_schema_banner(&self, message: Option<&str>) {
        let banner = &self.imp().schema_banner;
        match message {
            Some(msg) => {
                banner.set_text(msg);
                banner.set_visible(true);
            }
            None => {
                banner.set_text("");
                banner.set_visible(false);
            }
        }
    }

    fn clear_schema_browser(&self, banner: &str) {
        if let Some(tree) = self.imp().schema_tree.borrow().as_ref() {
            tree.clear();
        }
        self.set_schema_banner(Some(banner));
        self.imp().schema_refresh.set_sensitive(false);
    }

    /// Sync schema pane to the active schema connection's live pool state.
    pub fn update_schema_browser(&self) {
        let Some(id) = self.schema_connection_id() else {
            self.clear_schema_browser("Connect to browse schema");
            return;
        };

        let app = self
            .application()
            .and_downcast::<SqlatorApplication>()
            .expect("SqlatorApplication");
        let service = app.service();
        if !service.db().is_connected(&id) {
            self.clear_schema_browser("Connect to browse schema");
            return;
        }

        if let Some(tree) = self.imp().schema_tree.borrow().as_ref() {
            if tree.connection_id().as_deref() == Some(id.as_str()) {
                self.set_schema_banner(None);
                self.imp().schema_refresh.set_sensitive(true);
                return;
            }
        }

        self.reload_schema_browser();
    }

    /// Force-refresh schemas for the active schema connection.
    pub fn reload_schema_browser(&self) {
        let Some(id) = self.schema_connection_id() else {
            self.clear_schema_browser("Connect to browse schema");
            return;
        };

        let app = self
            .application()
            .and_downcast::<SqlatorApplication>()
            .expect("SqlatorApplication");
        let service = app.service();
        if !service.db().is_connected(&id) {
            self.clear_schema_browser("Connect to browse schema");
            return;
        }

        if let Some(tree) = self.imp().schema_tree.borrow().as_ref() {
            tree.prepare_load(&id);
        }
        self.set_schema_banner(Some("Loading schemas…"));
        self.imp().schema_refresh.set_sensitive(true);

        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let db = service.db_handle();
                let id_task = id.clone();
                let result = crate::spawn_tokio!(async move { db.get_schemas(&id_task).await })
                    .await
                    .expect("join get_schemas");

                let current = window.schema_connection_id();
                if current.as_deref() != Some(id.as_str()) {
                    return;
                }
                if let Some(tree) = window.imp().schema_tree.borrow().as_ref() {
                    if tree.connection_id().as_deref() != Some(id.as_str()) {
                        return;
                    }
                }

                match result {
                    Ok(schemas) => {
                        if let Some(tree) = window.imp().schema_tree.borrow().as_ref() {
                            tree.set_schemas(&id, schemas);
                        }
                        window.set_schema_banner(None);
                    }
                    Err(e) => {
                        tracing::warn!("get_schemas({id}) failed: {e}");
                        if let Some(tree) = window.imp().schema_tree.borrow().as_ref() {
                            tree.clear();
                        }
                        window.set_schema_banner(Some(&format!("Failed to load schemas: {e}")));
                    }
                }
            }
        ));
    }

    /// Lazy expand: populate `store` with tables for `schema_name`.
    pub fn fill_schema_tables(&self, schema_name: &str, store: gio::ListStore) {
        let Some(connection_id) = self
            .imp()
            .schema_tree
            .borrow()
            .as_ref()
            .and_then(|t| t.connection_id())
        else {
            replace_store_with_error(&store, "No connection");
            return;
        };

        let app = self
            .application()
            .and_downcast::<SqlatorApplication>()
            .expect("SqlatorApplication");
        let service = app.service();
        let schema_name = schema_name.to_string();

        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let db = service.db_handle();
                let conn = connection_id.clone();
                let schema = schema_name.clone();
                let result = crate::spawn_tokio!(async move {
                    db.get_tables(&conn, Some(schema.as_str())).await
                })
                .await
                .expect("join get_tables");

                let still_current = window
                    .imp()
                    .schema_tree
                    .borrow()
                    .as_ref()
                    .and_then(|t| t.connection_id())
                    .as_deref()
                    == Some(connection_id.as_str());
                if !still_current {
                    return;
                }

                match result {
                    Ok(tables) => replace_store_with_tables(&store, tables),
                    Err(e) => {
                        tracing::warn!("get_tables({connection_id}, {schema_name}) failed: {e}");
                        replace_store_with_error(&store, e.to_string());
                    }
                }
            }
        ));
    }

    /// Lazy expand: populate `store` with columns for `table_name`.
    pub fn fill_schema_columns(
        &self,
        schema: Option<&str>,
        table_name: &str,
        store: gio::ListStore,
    ) {
        let Some(connection_id) = self
            .imp()
            .schema_tree
            .borrow()
            .as_ref()
            .and_then(|t| t.connection_id())
        else {
            replace_store_with_error(&store, "No connection");
            return;
        };

        let app = self
            .application()
            .and_downcast::<SqlatorApplication>()
            .expect("SqlatorApplication");
        let service = app.service();
        let schema = schema.map(str::to_string);
        let table_name = table_name.to_string();

        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let db = service.db_handle();
                let conn = connection_id.clone();
                let table = table_name.clone();
                let schema_task = schema.clone();
                let result = crate::spawn_tokio!(async move {
                    db.get_columns(&conn, &table, schema_task.as_deref()).await
                })
                .await
                .expect("join get_columns");

                let still_current = window
                    .imp()
                    .schema_tree
                    .borrow()
                    .as_ref()
                    .and_then(|t| t.connection_id())
                    .as_deref()
                    == Some(connection_id.as_str());
                if !still_current {
                    return;
                }

                match result {
                    Ok(columns) => replace_store_with_columns(&store, columns),
                    Err(e) => {
                        tracing::warn!("get_columns({connection_id}, {table_name}) failed: {e}");
                        replace_store_with_error(&store, e.to_string());
                    }
                }
            }
        ));
    }

    pub fn connect_sidebar_connection(&self, connection_id: &str) {
        let app = self
            .application()
            .and_downcast::<SqlatorApplication>()
            .expect("SqlatorApplication");
        let service = app.service();
        let id = connection_id.to_string();

        self.set_selected_connection_id(Some(id.clone()));
        self.set_schema_connection_id(Some(id.clone()));
        self.imp()
            .connection_status
            .borrow_mut()
            .insert(id.clone(), ConnectionStatus::Connecting);
        if let Some(list) = self.imp().connections.borrow().as_ref() {
            list.set_status_for(&id, ConnectionStatus::Connecting);
        }

        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let svc = service.clone();
                let id_task = id.clone();
                let result =
                    crate::spawn_tokio!(async move { svc.connect_database(&id_task).await })
                        .await
                        .expect("join connect_database");

                match result {
                    Ok(()) => {
                        window
                            .imp()
                            .connection_status
                            .borrow_mut()
                            .insert(id.clone(), ConnectionStatus::Connected);
                        if let Some(list) = window.imp().connections.borrow().as_ref() {
                            list.set_status_for(&id, ConnectionStatus::Connected);
                        }
                        window.update_schema_browser();
                    }
                    Err(e) => {
                        tracing::warn!("connect_database({id}) failed: {e}");
                        window
                            .imp()
                            .connection_status
                            .borrow_mut()
                            .insert(id.clone(), ConnectionStatus::Error);
                        if let Some(list) = window.imp().connections.borrow().as_ref() {
                            list.set_status_for(&id, ConnectionStatus::Error);
                        }
                        window.update_schema_browser();
                        let dialog =
                            adw::AlertDialog::new(Some("Connection failed"), Some(&e.to_string()));
                        dialog.add_response("ok", "OK");
                        dialog.present(Some(&window));
                    }
                }
            }
        ));
    }

    pub fn disconnect_sidebar_connection(&self, connection_id: &str) {
        let app = self
            .application()
            .and_downcast::<SqlatorApplication>()
            .expect("SqlatorApplication");
        let service = app.service();
        let id = connection_id.to_string();

        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let svc = service.clone();
                let id_task = id.clone();
                let result =
                    crate::spawn_tokio!(async move { svc.disconnect_database(&id_task).await })
                        .await
                        .expect("join disconnect_database");

                match result {
                    Ok(()) => {
                        window.imp().connection_status.borrow_mut().remove(&id);
                        if let Some(list) = window.imp().connections.borrow().as_ref() {
                            list.set_status_for(&id, ConnectionStatus::Disconnected);
                        }
                        if window.schema_connection_id().as_deref() == Some(id.as_str()) {
                            window.update_schema_browser();
                        }
                    }
                    Err(e) => {
                        tracing::warn!("disconnect_database({id}) failed: {e}");
                        let dialog =
                            adw::AlertDialog::new(Some("Disconnect failed"), Some(&e.to_string()));
                        dialog.add_response("ok", "OK");
                        dialog.present(Some(&window));
                    }
                }
            }
        ));
    }

    pub fn clone_sidebar_connection(&self, connection_id: &str) {
        let app = self
            .application()
            .and_downcast::<SqlatorApplication>()
            .expect("SqlatorApplication");
        let service = app.service();
        let id = connection_id.to_string();

        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let svc = service.clone();
                let result = crate::spawn_tokio!(async move { svc.clone_connection(id).await })
                    .await
                    .expect("join clone_connection");
                match result {
                    Ok(info) => {
                        window.set_selected_connection_id(Some(info.id));
                        window.refresh_connections();
                    }
                    Err(e) => {
                        let dialog =
                            adw::AlertDialog::new(Some("Clone failed"), Some(&e.to_string()));
                        dialog.add_response("ok", "OK");
                        dialog.present(Some(&window));
                    }
                }
            }
        ));
    }

    pub fn delete_sidebar_connection(&self, connection_id: &str) {
        let id = connection_id.to_string();
        let dialog = adw::AlertDialog::new(
            Some("Delete connection?"),
            Some("This removes the saved connection profile. It cannot be undone."),
        );
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("delete", "Delete");
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        dialog.connect_response(
            None,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[strong]
                id,
                move |_, response| {
                    if response != "delete" {
                        return;
                    }
                    let app = window
                        .application()
                        .and_downcast::<SqlatorApplication>()
                        .expect("SqlatorApplication");
                    let service = app.service();
                    let id = id.clone();
                    glib::spawn_future_local(async move {
                        let svc = service.clone();
                        let id_task = id.clone();
                        let result =
                            crate::spawn_tokio!(
                                async move { svc.delete_connection(id_task).await }
                            )
                            .await
                            .expect("join delete_connection");
                        match result {
                            Ok(()) => {
                                if window.selected_connection_id().as_deref() == Some(id.as_str()) {
                                    window.set_selected_connection_id(None);
                                }
                                window.imp().connection_status.borrow_mut().remove(&id);
                                // Profile is gone — drop workspace UI without a second disconnect.
                                window.discard_connection_workspace(&id);
                                window.refresh_connections();
                            }
                            Err(e) => {
                                let dialog = adw::AlertDialog::new(
                                    Some("Delete failed"),
                                    Some(&e.to_string()),
                                );
                                dialog.add_response("ok", "OK");
                                dialog.present(Some(&window));
                            }
                        }
                    });
                }
            ),
        );
        dialog.present(Some(self));
    }

    pub fn toggle_group_collapsed(&self, group_id: &str, collapsed: bool) {
        let app = self
            .application()
            .and_downcast::<SqlatorApplication>()
            .expect("SqlatorApplication");
        let service = app.service();
        let id = group_id.to_string();

        // Optimistic local update via refresh after persist.
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let svc = service.clone();
                let groups = crate::spawn_tokio!(async move { svc.get_groups().await })
                    .await
                    .expect("join get_groups");
                let Ok(groups) = groups else {
                    return;
                };
                let Some(mut group) = groups.into_iter().find(|g| g.id == id) else {
                    return;
                };
                group.collapsed = collapsed;
                let svc = service.clone();
                let result = crate::spawn_tokio!(async move { svc.update_group(group).await })
                    .await
                    .expect("join update_group");
                if let Err(e) = result {
                    tracing::warn!("update_group failed: {e}");
                }
                window.refresh_connections();
            }
        ));
    }

    pub fn delete_sidebar_group(&self, group_id: &str) {
        let id = group_id.to_string();
        let dialog = adw::AlertDialog::new(
            Some("Delete group?"),
            Some("Connections in this group become ungrouped. Sub-groups are re-parented."),
        );
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("delete", "Delete");
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        dialog.connect_response(
            None,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[strong]
                id,
                move |_, response| {
                    if response != "delete" {
                        return;
                    }
                    let app = window
                        .application()
                        .and_downcast::<SqlatorApplication>()
                        .expect("SqlatorApplication");
                    let service = app.service();
                    let id = id.clone();
                    glib::spawn_future_local(async move {
                        let svc = service.clone();
                        let result = crate::spawn_tokio!(async move { svc.delete_group(id).await })
                            .await
                            .expect("join delete_group");
                        match result {
                            Ok(()) => window.refresh_connections(),
                            Err(e) => {
                                let dialog = adw::AlertDialog::new(
                                    Some("Delete group failed"),
                                    Some(&e.to_string()),
                                );
                                dialog.add_response("ok", "OK");
                                dialog.present(Some(&window));
                            }
                        }
                    });
                }
            ),
        );
        dialog.present(Some(self));
    }

    pub fn add_query_tab(&self, title: Option<&str>) -> adw::TabPage {
        let app = self
            .application()
            .and_downcast::<SqlatorApplication>()
            .expect("SqlatorApplication");
        let tab = QueryTab::new(&app, self);
        if let Some(id) = self.imp().active_workspace_id.borrow().clone() {
            tab.set_connection_id(Some(id));
        }
        let page = self.imp().tab_view.append(&tab);
        page.set_title(title.unwrap_or("Query"));
        page.set_live_thumbnail(true);
        self.imp().tab_view.set_selected_page(&page);
        page
    }

    /// Show the empty-state page (no connection workspace open).
    fn show_empty_workspace(&self) {
        self.drain_tab_view(true);
        *self.imp().active_workspace_id.borrow_mut() = None;
        self.imp().content_stack.set_visible_child_name("empty");
        self.imp().tab_bar.set_visible(false);
        self.imp().connection_tabs_scroll.set_visible(false);
    }

    /// Open a connection workspace, or focus it if already open.
    /// Returns `true` when a new workspace was created (Svelte `tabs.openConnection`).
    pub fn open_connection_workspace(&self, connection_id: &str) -> bool {
        if self.imp().active_workspace_id.borrow().as_deref() == Some(connection_id) {
            self.rebuild_connection_tab_bar();
            return false;
        }

        let already_open = self
            .imp()
            .open_connection_ids
            .borrow()
            .iter()
            .any(|id| id == connection_id);
        if already_open {
            self.switch_connection_workspace(connection_id);
            return false;
        }

        self.stash_active_workspace();
        self.imp()
            .open_connection_ids
            .borrow_mut()
            .push(connection_id.to_string());
        *self.imp().active_workspace_id.borrow_mut() = Some(connection_id.to_string());
        self.imp().content_stack.set_visible_child_name("tabs");
        self.imp().tab_bar.set_visible(true);
        self.add_query_tab(None);
        self.rebuild_connection_tab_bar();
        true
    }

    /// Switch the query-tab strip to an already-open connection workspace.
    pub fn switch_connection_workspace(&self, connection_id: &str) {
        if self.imp().active_workspace_id.borrow().as_deref() == Some(connection_id) {
            return;
        }
        let is_open = self
            .imp()
            .open_connection_ids
            .borrow()
            .iter()
            .any(|id| id == connection_id);
        if !is_open {
            return;
        }

        self.stash_active_workspace();
        let workspace = self
            .imp()
            .stashed_workspaces
            .borrow_mut()
            .remove(connection_id)
            .unwrap_or_else(|| self.fresh_workspace(connection_id));
        self.mount_workspace(connection_id, workspace);
        self.set_selected_connection_id(Some(connection_id.to_string()));
        self.set_schema_connection_id(Some(connection_id.to_string()));
        self.rebuild_connection_tab_bar();
    }

    /// Close a connection workspace and disconnect (Svelte connection tab close).
    pub fn close_connection_workspace(&self, connection_id: &str) {
        self.remove_connection_workspace(connection_id);
        self.disconnect_sidebar_connection(connection_id);
    }

    /// Drop workspace UI state without disconnecting (e.g. connection deleted).
    pub fn discard_connection_workspace(&self, connection_id: &str) {
        self.remove_connection_workspace(connection_id);
    }

    fn remove_connection_workspace(&self, connection_id: &str) {
        let was_active = self.imp().active_workspace_id.borrow().as_deref() == Some(connection_id);

        self.imp()
            .open_connection_ids
            .borrow_mut()
            .retain(|id| id != connection_id);
        self.imp()
            .stashed_workspaces
            .borrow_mut()
            .remove(connection_id);

        if was_active {
            self.drain_tab_view(true);
            *self.imp().active_workspace_id.borrow_mut() = None;

            let next_id = self.imp().open_connection_ids.borrow().last().cloned();
            if let Some(next_id) = next_id {
                let workspace = self
                    .imp()
                    .stashed_workspaces
                    .borrow_mut()
                    .remove(&next_id)
                    .unwrap_or_else(|| self.fresh_workspace(&next_id));
                self.mount_workspace(&next_id, workspace);
                self.set_selected_connection_id(Some(next_id.clone()));
                self.set_schema_connection_id(Some(next_id));
            } else {
                self.show_empty_workspace();
                if self.schema_connection_id().as_deref() == Some(connection_id) {
                    self.set_schema_connection_id(None);
                }
            }
        } else if self.schema_connection_id().as_deref() == Some(connection_id)
            && self.imp().active_workspace_id.borrow().is_none()
        {
            self.set_schema_connection_id(None);
        }

        self.rebuild_connection_tab_bar();
    }

    fn stash_active_workspace(&self) {
        let Some(id) = self.imp().active_workspace_id.borrow().clone() else {
            return;
        };
        let workspace = self.take_mounted_workspace();
        self.imp()
            .stashed_workspaces
            .borrow_mut()
            .insert(id, workspace);
        *self.imp().active_workspace_id.borrow_mut() = None;
    }

    fn take_mounted_workspace(&self) -> imp::StashedWorkspace {
        let tab_view = &self.imp().tab_view;
        let selected_index = tab_view
            .selected_page()
            .map(|p| tab_view.page_position(&p) as u32)
            .unwrap_or(0);
        let mut tabs = Vec::new();
        for i in 0..tab_view.n_pages() {
            let page = tab_view.nth_page(i);
            let title = page.title().to_string();
            let child = page.child();
            let stashed = if let Ok(tab) = child.clone().downcast::<QueryTab>() {
                imp::StashedPage::Query(tab)
            } else if let Ok(tab) = child.clone().downcast::<TableBrowseTab>() {
                imp::StashedPage::TableBrowse(tab)
            } else if let Ok(tab) = child.downcast::<SchemaDdlTab>() {
                imp::StashedPage::SchemaDdl(tab)
            } else {
                panic!("tab_view page is not a known workspace tab type");
            };
            tabs.push(imp::StashedTab {
                title,
                page: stashed,
            });
        }
        // Detach without cancelling in-flight queries — inactive workspaces keep running.
        self.drain_tab_view(false);
        imp::StashedWorkspace {
            tabs,
            selected_index,
        }
    }

    fn mount_workspace(&self, connection_id: &str, workspace: imp::StashedWorkspace) {
        let tab_view = &self.imp().tab_view;
        debug_assert_eq!(tab_view.n_pages(), 0);

        let mut selected_page = None;
        for (i, stashed) in workspace.tabs.into_iter().enumerate() {
            // Keep connection_id in sync if the tab was created before wiring existed.
            if !stashed.page.connection_matches(connection_id) {
                stashed
                    .page
                    .set_connection_id(Some(connection_id.to_string()));
            }
            let page = tab_view.append(&stashed.page.widget());
            page.set_title(&stashed.title);
            page.set_live_thumbnail(true);
            if i as u32 == workspace.selected_index {
                selected_page = Some(page);
            }
        }

        if let Some(page) = selected_page {
            tab_view.set_selected_page(&page);
        } else if tab_view.n_pages() > 0 {
            let last = tab_view.nth_page(tab_view.n_pages() - 1);
            tab_view.set_selected_page(&last);
        } else {
            *self.imp().active_workspace_id.borrow_mut() = Some(connection_id.to_string());
            self.add_query_tab(None);
            self.imp().content_stack.set_visible_child_name("tabs");
            self.imp().tab_bar.set_visible(true);
            return;
        }

        *self.imp().active_workspace_id.borrow_mut() = Some(connection_id.to_string());
        self.imp().content_stack.set_visible_child_name("tabs");
        self.imp().tab_bar.set_visible(true);
    }

    fn fresh_workspace(&self, connection_id: &str) -> imp::StashedWorkspace {
        let app = self
            .application()
            .and_downcast::<SqlatorApplication>()
            .expect("SqlatorApplication");
        let tab = QueryTab::new(&app, self);
        tab.set_connection_id(Some(connection_id.to_string()));
        imp::StashedWorkspace {
            tabs: vec![imp::StashedTab {
                title: "Query".to_string(),
                page: imp::StashedPage::Query(tab),
            }],
            selected_index: 0,
        }
    }

    /// Remove all pages from `tab_view`. When `cancel_busy`, abort in-flight queries.
    fn drain_tab_view(&self, cancel_busy: bool) {
        let tab_view = &self.imp().tab_view;
        if cancel_busy {
            for i in 0..tab_view.n_pages() {
                if let Ok(tab) = tab_view.nth_page(i).child().downcast::<QueryTab>() {
                    if tab.is_busy() {
                        tab.cancel_query();
                    }
                }
            }
        }
        self.imp().force_close_pages.set(true);
        while tab_view.n_pages() > 0 {
            let page = tab_view.nth_page(0);
            tab_view.close_page(&page);
        }
        self.imp().force_close_pages.set(false);
    }

    fn rebuild_connection_tab_bar(&self) {
        let tabs_box = &self.imp().connection_tabs_box;
        while let Some(child) = tabs_box.first_child() {
            tabs_box.remove(&child);
        }

        let ids = self.imp().open_connection_ids.borrow().clone();
        let active = self.imp().active_workspace_id.borrow().clone();
        self.imp()
            .connection_tabs_scroll
            .set_visible(!ids.is_empty());

        for id in ids {
            let name = self
                .imp()
                .connection_names
                .borrow()
                .get(&id)
                .cloned()
                .unwrap_or_else(|| id.clone());
            let is_active = active.as_deref() == Some(id.as_str());

            let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            row.add_css_class("connection-tab");
            if is_active {
                row.add_css_class("active");
            }

            let select = gtk::Button::with_label(&name);
            select.add_css_class("flat");
            select.add_css_class("connection-tab-select");
            select.set_hexpand(true);
            select.set_tooltip_text(Some(&name));
            if let Some(child) = select.child() {
                if let Ok(label) = child.downcast::<gtk::Label>() {
                    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    label.set_max_width_chars(18);
                    label.set_xalign(0.0);
                }
            }
            select.connect_clicked(glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[strong]
                id,
                move |_| {
                    window.switch_connection_workspace(&id);
                }
            ));
            row.append(&select);

            let close = gtk::Button::from_icon_name("window-close-symbolic");
            close.add_css_class("flat");
            close.add_css_class("circular");
            close.add_css_class("connection-tab-close");
            close.set_tooltip_text(Some("Disconnect & close"));
            close.connect_clicked(glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[strong]
                id,
                move |_| {
                    window.close_connection_workspace(&id);
                }
            ));
            row.append(&close);

            tabs_box.append(&row);
        }
    }

    pub fn editor_results_position(&self) -> i32 {
        self.settings().get::<i32>("editor-results-position")
    }

    pub fn set_editor_results_position(&self, pos: i32) {
        let _ = self.settings().set("editor-results-position", pos);
    }

    /// Open or focus a table-browse tab for the schema connection (Svelte `tabs.openTableBrowse`).
    pub fn open_table_browse(&self, table_name: &str, schema: Option<String>) {
        let Some(connection_id) = self.schema_connection_id() else {
            return;
        };
        if self.imp().active_workspace_id.borrow().as_deref() != Some(connection_id.as_str()) {
            self.open_connection_workspace(&connection_id);
        }

        let tab_view = &self.imp().tab_view;
        for i in 0..tab_view.n_pages() {
            let page = tab_view.nth_page(i);
            if let Ok(tab) = page.child().downcast::<TableBrowseTab>() {
                if tab.matches_table(table_name, schema.as_deref()) {
                    tab_view.set_selected_page(&page);
                    return;
                }
            }
        }

        let app = self
            .application()
            .and_downcast::<SqlatorApplication>()
            .expect("SqlatorApplication");
        let tab = TableBrowseTab::new(&app, self, connection_id, table_name, schema.clone());
        let title = match schema.as_deref() {
            Some(s) if !s.is_empty() => format!("{s}.{table_name}"),
            _ => table_name.to_string(),
        };
        let page = tab_view.append(&tab);
        page.set_title(&title);
        page.set_live_thumbnail(true);
        tab_view.set_selected_page(&page);
    }

    /// Open or focus a DDL viewer tab for the schema connection (Svelte `tabs.openSchemaDdl`).
    pub fn open_schema_ddl(&self, table_name: &str, schema: Option<String>) {
        let Some(connection_id) = self.schema_connection_id() else {
            return;
        };
        if self.imp().active_workspace_id.borrow().as_deref() != Some(connection_id.as_str()) {
            self.open_connection_workspace(&connection_id);
        }

        let tab_view = &self.imp().tab_view;
        for i in 0..tab_view.n_pages() {
            let page = tab_view.nth_page(i);
            if let Ok(tab) = page.child().downcast::<SchemaDdlTab>() {
                if tab.matches_table(table_name, schema.as_deref()) {
                    tab_view.set_selected_page(&page);
                    return;
                }
            }
        }

        let app = self
            .application()
            .and_downcast::<SqlatorApplication>()
            .expect("SqlatorApplication");
        let tab = SchemaDdlTab::new(&app, self, connection_id, table_name, schema.clone());
        let title = match schema.as_deref() {
            Some(s) if !s.is_empty() => format!("DDL: {s}.{table_name}"),
            _ => format!("DDL: {table_name}"),
        };
        let page = tab_view.append(&tab);
        page.set_title(&title);
        page.set_live_thumbnail(true);
        tab_view.set_selected_page(&page);
    }

    /// True when any mounted or stashed query tab has pending result edits.
    fn has_any_unsaved_edits(&self) -> bool {
        let tab_view = &self.imp().tab_view;
        for i in 0..tab_view.n_pages() {
            if let Ok(tab) = tab_view.nth_page(i).child().downcast::<QueryTab>() {
                if tab.has_unsaved_edits() {
                    return true;
                }
            }
        }
        self.imp()
            .stashed_workspaces
            .borrow()
            .values()
            .flat_map(|ws| ws.tabs.iter())
            .any(|stashed| stashed.page.has_unsaved_edits())
    }

    /// Discard pending result edits across mounted and stashed query tabs.
    fn discard_all_edits(&self) {
        let tab_view = &self.imp().tab_view;
        for i in 0..tab_view.n_pages() {
            if let Ok(tab) = tab_view.nth_page(i).child().downcast::<QueryTab>() {
                if tab.has_unsaved_edits() {
                    tab.discard_edits();
                }
            }
        }
        let stashed_queries: Vec<QueryTab> = self
            .imp()
            .stashed_workspaces
            .borrow()
            .values()
            .flat_map(|ws| ws.tabs.iter())
            .filter_map(|s| s.page.as_query().cloned())
            .collect();
        for tab in stashed_queries {
            if tab.has_unsaved_edits() {
                tab.discard_edits();
            }
        }
    }
}
