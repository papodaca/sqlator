//! Read-only DDL viewer tab (Svelte `SchemaDdlViewer` parity).

use crate::application::SqlatorApplication;
use crate::window::SqlatorWindow;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use sourceview::prelude::*;
use std::cell::{Cell, OnceCell, RefCell};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SchemaDdlTab {
        pub root: gtk::Box,
        pub title: gtk::Label,
        pub refresh_btn: gtk::Button,
        pub copy_btn: gtk::Button,
        pub stack: gtk::Stack,
        pub error_label: gtk::Label,
        pub editor: sourceview::View,
        pub service: OnceCell<Arc<sqlator_service::AppService>>,
        pub window: OnceCell<glib::WeakRef<SqlatorWindow>>,
        pub connection_id: RefCell<String>,
        pub table_name: RefCell<String>,
        pub schema: RefCell<Option<String>>,
        pub persist_id: RefCell<String>,
        pub ddl: RefCell<Option<String>>,
        pub generation: AtomicU64,
        pub loading: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SchemaDdlTab {
        const NAME: &'static str = "SchemaDdlTab";
        type Type = super::SchemaDdlTab;
        type ParentType = adw::Bin;

        fn new() -> Self {
            let root = gtk::Box::new(gtk::Orientation::Vertical, 0);

            let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            toolbar.add_css_class("toolbar");
            toolbar.set_margin_start(8);
            toolbar.set_margin_end(8);
            toolbar.set_margin_top(6);
            toolbar.set_margin_bottom(6);

            let title = gtk::Label::builder()
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .hexpand(true)
                .css_classes(["heading"])
                .build();

            let refresh_btn = gtk::Button::with_label("Refresh");
            refresh_btn.add_css_class("flat");
            let copy_btn = gtk::Button::with_label("Copy");
            copy_btn.add_css_class("flat");
            copy_btn.set_sensitive(false);

            toolbar.append(&title);
            toolbar.append(&refresh_btn);
            toolbar.append(&copy_btn);

            let stack = gtk::Stack::new();
            stack.set_hexpand(true);
            stack.set_vexpand(true);

            let loading = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            loading.set_halign(gtk::Align::Center);
            loading.set_valign(gtk::Align::Center);
            let spinner = gtk::Spinner::new();
            spinner.set_spinning(true);
            loading.append(&spinner);
            loading.append(&gtk::Label::new(Some("Loading DDL…")));
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

            let buffer = sourceview::Buffer::new(None);
            buffer.set_highlight_syntax(true);
            if let Some(language) = sourceview::LanguageManager::default().language("sql") {
                buffer.set_language(Some(&language));
            } else {
                tracing::warn!("GtkSourceView sql language not found; DDL will not highlight");
            }
            apply_editor_style_scheme(&buffer);
            adw::StyleManager::default().connect_dark_notify(glib::clone!(
                #[weak]
                buffer,
                move |_| {
                    apply_editor_style_scheme(&buffer);
                }
            ));

            let editor = sourceview::View::with_buffer(&buffer);
            editor.set_editable(false);
            editor.set_cursor_visible(false);
            editor.set_monospace(true);
            editor.set_show_line_numbers(true);
            editor.set_hexpand(true);
            editor.set_vexpand(true);

            let scrolled = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Automatic)
                .vscrollbar_policy(gtk::PolicyType::Automatic)
                .child(&editor)
                .build();
            stack.add_named(&scrolled, Some("editor"));

            root.append(&toolbar);
            root.append(&stack);

            // Retry uses the same path as Refresh.
            retry_btn.connect_clicked(glib::clone!(
                #[weak]
                refresh_btn,
                move |_| {
                    refresh_btn.emit_clicked();
                }
            ));

            Self {
                root,
                title,
                refresh_btn,
                copy_btn,
                stack,
                error_label,
                editor,
                service: OnceCell::new(),
                window: OnceCell::new(),
                connection_id: RefCell::new(String::new()),
                table_name: RefCell::new(String::new()),
                schema: RefCell::new(None),
                persist_id: RefCell::new(String::new()),
                ddl: RefCell::new(None),
                generation: AtomicU64::new(0),
                loading: Cell::new(false),
            }
        }
    }

    impl ObjectImpl for SchemaDdlTab {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().set_child(Some(&self.root));
        }
    }

    impl WidgetImpl for SchemaDdlTab {}
    impl BinImpl for SchemaDdlTab {}
}

glib::wrapper! {
    pub struct SchemaDdlTab(ObjectSubclass<imp::SchemaDdlTab>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl SchemaDdlTab {
    pub fn new(
        app: &SqlatorApplication,
        window: &SqlatorWindow,
        connection_id: impl Into<String>,
        table_name: impl Into<String>,
        schema: Option<String>,
    ) -> Self {
        let tab = Self::build(app, window, connection_id, table_name, schema, None);
        tab.fetch_ddl();
        tab
    }

    /// Restore a DDL tab from session state without fetching until connected.
    pub fn restore(
        app: &SqlatorApplication,
        window: &SqlatorWindow,
        connection_id: impl Into<String>,
        table_name: impl Into<String>,
        schema: Option<String>,
        persist_id: impl Into<String>,
    ) -> Self {
        Self::build(
            app,
            window,
            connection_id,
            table_name,
            schema,
            Some(persist_id.into()),
        )
    }

    fn build(
        app: &SqlatorApplication,
        window: &SqlatorWindow,
        connection_id: impl Into<String>,
        table_name: impl Into<String>,
        schema: Option<String>,
        persist_id: Option<String>,
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
        *tab.imp().persist_id.borrow_mut() = persist_id.unwrap_or_else(crate::session::new_tab_id);
        crate::preferences::style_editor(&tab.imp().editor);

        tab.imp().refresh_btn.connect_clicked(glib::clone!(
            #[weak]
            tab,
            move |_| tab.fetch_ddl()
        ));
        tab.imp().copy_btn.connect_clicked(glib::clone!(
            #[weak]
            tab,
            move |_| tab.copy_ddl()
        ));

        tab
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

    /// True when this tab shows DDL for the same table identity.
    pub fn matches_table(&self, table_name: &str, schema: Option<&str>) -> bool {
        self.table_name() == table_name && self.schema().as_deref() == schema
    }

    pub fn set_connection_id(&self, id: Option<String>) {
        if let Some(id) = id {
            *self.imp().connection_id.borrow_mut() = id;
        }
    }

    /// Re-fetch DDL (used after session restore reconnects).
    pub fn reload(&self) {
        self.fetch_ddl();
    }

    fn fetch_ddl(&self) {
        let Some(service) = self.imp().service.get().cloned() else {
            return;
        };
        let connection_id = self.connection_id();
        let table_name = self.table_name();
        let schema = self.schema();
        let gen = self.imp().generation.fetch_add(1, Ordering::Relaxed) + 1;

        self.imp().loading.set(true);
        self.imp().refresh_btn.set_sensitive(false);
        if self.imp().ddl.borrow().is_none() {
            self.imp().stack.set_visible_child_name("loading");
        }

        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = tab)]
            self,
            async move {
                let db = service.db_handle();
                let schema_owned = schema.clone();
                let result = crate::spawn_tokio!(async move {
                    db.get_ddl(&connection_id, &table_name, schema_owned.as_deref())
                        .await
                })
                .await
                .expect("join get_ddl");

                if tab.imp().generation.load(Ordering::Relaxed) != gen {
                    return;
                }
                tab.imp().loading.set(false);
                tab.imp().refresh_btn.set_sensitive(true);

                match result {
                    Ok(ddl) => {
                        *tab.imp().ddl.borrow_mut() = Some(ddl.clone());
                        let buffer = tab
                            .imp()
                            .editor
                            .buffer()
                            .downcast::<sourceview::Buffer>()
                            .expect("SourceBuffer");
                        buffer.set_text(&ddl);
                        tab.imp().copy_btn.set_sensitive(true);
                        tab.imp().copy_btn.set_label("Copy");
                        tab.imp().stack.set_visible_child_name("editor");
                    }
                    Err(e) => {
                        tab.imp().error_label.set_text(&e.to_string());
                        tab.imp().stack.set_visible_child_name("error");
                    }
                }
            }
        ));
    }

    fn copy_ddl(&self) {
        let Some(ddl) = self.imp().ddl.borrow().clone() else {
            return;
        };
        self.display().clipboard().set_text(&ddl);
        self.imp().copy_btn.set_label("Copied!");
        glib::timeout_add_local_once(
            std::time::Duration::from_secs(2),
            glib::clone!(
                #[weak(rename_to = tab)]
                self,
                move || {
                    if tab.imp().ddl.borrow().is_some() {
                        tab.imp().copy_btn.set_label("Copy");
                    }
                }
            ),
        );
    }
}

fn apply_editor_style_scheme(buffer: &sourceview::Buffer) {
    let name = if adw::StyleManager::default().is_dark() {
        "Adwaita-dark"
    } else {
        "Adwaita"
    };
    match sourceview::StyleSchemeManager::default().scheme(name) {
        Some(scheme) => buffer.set_style_scheme(Some(&scheme)),
        None => tracing::warn!("GtkSourceView style scheme `{name}` not found"),
    }
}
