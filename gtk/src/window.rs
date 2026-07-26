use crate::application::SqlatorApplication;
use crate::connection::{status_map_from_service, ConnectionList, ConnectionStatus};
use crate::query_tab::QueryTab;
use crate::schema::{
    replace_store_with_columns, replace_store_with_error, replace_store_with_tables, SchemaTree,
};
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib, CompositeTemplate};
use std::collections::HashMap;

mod imp {
    use super::*;
    use std::cell::RefCell;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/im/apodaca/SqlatorGtk/ui/window.ui")]
    pub struct SqlatorWindow {
        #[template_child]
        pub split_view: TemplateChild<adw::OverlaySplitView>,
        #[template_child]
        pub tab_view: TemplateChild<adw::TabView>,
        #[template_child]
        pub tab_bar: TemplateChild<adw::TabBar>,
        #[template_child]
        pub tab_overview: TemplateChild<adw::TabOverview>,
        #[template_child]
        pub overview_button: TemplateChild<gtk::Button>,
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
        pub selected_connection_id: RefCell<Option<String>>,
        /// In-flight / error status overlays on top of `db.is_connected`.
        pub connection_status: RefCell<HashMap<String, ConnectionStatus>>,
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

            // Persist geometry on close.
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
        window.add_query_tab(None);

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
                window.add_query_tab(None);
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
                let Some(tab) = page.child().downcast::<QueryTab>().ok() else {
                    return glib::Propagation::Proceed;
                };
                if tab.is_busy() {
                    let dialog = adw::AlertDialog::new(
                        Some("Query running"),
                        Some("A query is still running in this tab. Cancel it and close?"),
                    );
                    dialog.add_response("keep", "Keep Open");
                    dialog.add_response("close", "Cancel & Close");
                    dialog.set_response_appearance("close", adw::ResponseAppearance::Destructive);
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
            }
        ));
    }

    pub fn selected_connection_id(&self) -> Option<String> {
        self.imp().selected_connection_id.borrow().clone()
    }

    pub fn set_selected_connection_id(&self, id: Option<String>) {
        let changed = self.imp().selected_connection_id.borrow().as_deref() != id.as_deref();
        *self.imp().selected_connection_id.borrow_mut() = id;
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

    /// Sync schema pane to the selected connection's live pool state.
    pub fn update_schema_browser(&self) {
        let Some(id) = self.selected_connection_id() else {
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

    /// Force-refresh schemas for the selected connected connection.
    pub fn reload_schema_browser(&self) {
        let Some(id) = self.selected_connection_id() else {
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

                let current = window.selected_connection_id();
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
                        window.update_schema_browser();
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
        let page = self.imp().tab_view.append(&tab);
        page.set_title(title.unwrap_or("Query"));
        page.set_live_thumbnail(true);
        self.imp().tab_view.set_selected_page(&page);
        page
    }

    pub fn editor_results_position(&self) -> i32 {
        self.settings().get::<i32>("editor-results-position")
    }

    pub fn set_editor_results_position(&self, pos: i32) {
        let _ = self.settings().set("editor-results-position", pos);
    }
}
