mod imp;

use crate::application::SqlatorApplication;
use crate::window::SqlatorWindow;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use sqlator_core::models::QueryEvent;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

glib::wrapper! {
    pub struct QueryTab(ObjectSubclass<imp::QueryTab>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl QueryTab {
    pub fn new(app: &SqlatorApplication, window: &SqlatorWindow) -> Self {
        let tab: Self = glib::Object::builder().build();
        tab.imp().service.set(app.service()).ok();
        tab.imp()
            .window
            .set(window.downgrade())
            .expect("window weak ref once");

        let pos = window.editor_results_position();
        tab.imp().editor_paned.set_position(pos);
        tab.imp().editor_paned.connect_notify_local(
            Some("position"),
            glib::clone!(
                #[weak]
                window,
                move |paned, _| {
                    window.set_editor_results_position(paned.position());
                }
            ),
        );

        tab.setup_actions();
        tab.setup_editor();
        tab.set_placeholder_sql();
        tab
    }

    fn setup_editor(&self) {
        use sourceview::prelude::*;

        let editor = self.imp().editor.get();
        editor.set_smart_backspace(true);

        let buffer = editor
            .buffer()
            .downcast::<sourceview::Buffer>()
            .expect("GtkSource.View provides a SourceBuffer");
        buffer.set_highlight_syntax(true);
        buffer.set_highlight_matching_brackets(true);
        buffer.set_enable_undo(true);
        match sourceview::LanguageManager::default().language("sql") {
            Some(language) => buffer.set_language(Some(&language)),
            None => {
                tracing::warn!("GtkSourceView sql language not found; editor will not highlight")
            }
        }
        Self::apply_editor_style_scheme(&buffer);

        // GtkSourceView is not libadwaita-aware — follow AdwStyleManager manually.
        adw::StyleManager::default().connect_dark_notify(glib::clone!(
            #[weak]
            buffer,
            move |_| {
                Self::apply_editor_style_scheme(&buffer);
            }
        ));

        // SourceView/TextView would otherwise insert a newline on Ctrl+Return
        // before the application accelerator can activate tab.run.
        let shortcuts = gtk::ShortcutController::new();
        shortcuts.set_scope(gtk::ShortcutScope::Local);
        shortcuts.set_propagation_phase(gtk::PropagationPhase::Capture);
        for (trigger, action) in [
            ("<Control>Return", "tab.run"),
            ("<Control>KP_Enter", "tab.run"),
            ("<Control><Shift>Return", "tab.run-all"),
            ("<Control><Shift>KP_Enter", "tab.run-all"),
            ("<Control><Alt>Return", "tab.run-selection"),
            ("<Control><Alt>KP_Enter", "tab.run-selection"),
        ] {
            if let Some(trigger) = gtk::ShortcutTrigger::parse_string(trigger) {
                shortcuts.add_shortcut(gtk::Shortcut::new(
                    Some(trigger),
                    Some(gtk::NamedAction::new(action)),
                ));
            }
        }
        editor.add_controller(shortcuts);
    }

    fn apply_editor_style_scheme(buffer: &sourceview::Buffer) {
        use sourceview::prelude::*;
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

    fn setup_actions(&self) {
        let group = gio::SimpleActionGroup::new();

        let run = gio::SimpleAction::new("run", None);
        run.connect_activate(glib::clone!(
            #[weak(rename_to = tab)]
            self,
            move |_, _| tab.run_query(QueryMode::Editor)
        ));
        group.add_action(&run);

        let run_all = gio::SimpleAction::new("run-all", None);
        run_all.connect_activate(glib::clone!(
            #[weak(rename_to = tab)]
            self,
            move |_, _| tab.run_query(QueryMode::Editor)
        ));
        group.add_action(&run_all);

        let run_selection = gio::SimpleAction::new("run-selection", None);
        run_selection.connect_activate(glib::clone!(
            #[weak(rename_to = tab)]
            self,
            move |_, _| tab.run_query(QueryMode::Selection)
        ));
        group.add_action(&run_selection);

        let cancel = gio::SimpleAction::new("cancel", None);
        cancel.set_enabled(false);
        cancel.connect_activate(glib::clone!(
            #[weak(rename_to = tab)]
            self,
            move |_, _| tab.cancel_query()
        ));
        group.add_action(&cancel);
        if self.imp().cancel_action.set(cancel).is_err() {
            panic!("cancel action once");
        }

        let format_sql = gio::SimpleAction::new("format-sql", None);
        format_sql.connect_activate(|_, _| {});
        group.add_action(&format_sql);

        let find = gio::SimpleAction::new("find", None);
        find.connect_activate(|_, _| {});
        group.add_action(&find);

        let copy_csv = gio::SimpleAction::new("copy-as-csv", None);
        copy_csv.connect_activate(glib::clone!(
            #[weak(rename_to = tab)]
            self,
            move |_, _| tab.copy_results_as_csv()
        ));
        group.add_action(&copy_csv);

        self.insert_action_group("tab", Some(&group));
    }

    fn set_placeholder_sql(&self) {
        let buffer = self.imp().editor.buffer();
        buffer.set_text(
            "-- Select a connection in the sidebar, then run with Ctrl+Return\nSELECT 1 AS n;",
        );
    }

    pub fn is_busy(&self) -> bool {
        self.imp().cancel_token.borrow().is_some()
    }

    pub fn cancel_query(&self) {
        if let Some(token) = self.imp().cancel_token.borrow_mut().take() {
            token.cancel();
        }
        self.set_busy(false);
    }

    fn set_busy(&self, busy: bool) {
        if let Some(action) = self.imp().cancel_action.get() {
            action.set_enabled(busy);
        }
        if let Some(page) = self.tab_page() {
            page.set_loading(busy);
        }
    }

    fn tab_page(&self) -> Option<adw::TabPage> {
        self.parent()
            .and_then(|p| p.downcast::<adw::TabView>().ok())
            .map(|view| view.page(self))
    }

    fn run_query(&self, mode: QueryMode) {
        if self.is_busy() {
            return;
        }

        let window = match self.imp().window.get().and_then(|w| w.upgrade()) {
            Some(w) => w,
            None => return,
        };
        let Some(connection_id) = window.selected_connection_id() else {
            self.append_message("Select a connection in the sidebar first.");
            self.imp().results_stack.set_visible_child_name("messages");
            return;
        };

        let buffer = self.imp().editor.buffer();
        let sql = match mode {
            QueryMode::Editor => {
                let (start, end) = buffer.bounds();
                buffer.text(&start, &end, false).to_string()
            }
            QueryMode::Selection => {
                if let Some((start, end)) = buffer.selection_bounds() {
                    buffer.text(&start, &end, false).to_string()
                } else {
                    let (start, end) = buffer.bounds();
                    buffer.text(&start, &end, false).to_string()
                }
            }
        };
        let sql = sql.trim().to_string();
        if sql.is_empty() {
            return;
        }

        let service = match self.imp().service.get() {
            Some(s) => Arc::clone(s),
            None => return,
        };

        let generation = self.imp().generation.fetch_add(1, Ordering::SeqCst) + 1;
        let token = CancellationToken::new();
        *self.imp().cancel_token.borrow_mut() = Some(token.clone());
        self.set_busy(true);
        self.clear_results();
        self.imp().results_stack.set_visible_child_name("results");

        let (tx, rx) = async_channel::unbounded::<QueryEvent>();

        // Client task: connect (if needed) + execute, honouring cancel.
        let conn_id = connection_id.clone();
        let sql_task = sql.clone();
        let join = crate::spawn_tokio!(async move {
            // Ensure pool is up.
            if let Err(e) = service.connect_database(&conn_id).await {
                let _ = tx
                    .send(QueryEvent::Error {
                        message: e.to_string(),
                    })
                    .await;
                return;
            }

            let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<QueryEvent>(256);
            let db = service.db_handle();
            let cancel = token.clone();
            let exec = tokio::spawn(async move {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        // Layer 3: server-side cancel when supported.
                        let _ = db.cancel_query(&conn_id).await;
                    }
                    res = db.execute_query(&conn_id, &sql_task, event_tx) => {
                        if let Err(e) = res {
                            // execute_query sends Error events itself on many paths;
                            // surface join/setup failures here.
                            tracing::debug!("execute_query returned error: {e}");
                        }
                    }
                }
            });

            while let Some(ev) = event_rx.recv().await {
                if tx.send(ev).await.is_err() {
                    break;
                }
            }
            let _ = exec.await;
        });

        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = tab)]
            self,
            async move {
                while let Ok(event) = rx.recv().await {
                    if tab.imp().generation.load(Ordering::SeqCst) != generation {
                        // Superseded run — discard late results.
                        continue;
                    }
                    tab.handle_event(event);
                }
                let _ = join.await;
                if tab.imp().generation.load(Ordering::SeqCst) == generation {
                    *tab.imp().cancel_token.borrow_mut() = None;
                    tab.set_busy(false);
                }
            }
        ));
    }

    fn handle_event(&self, event: QueryEvent) {
        match event {
            QueryEvent::Columns { names } => {
                self.append_result(&format!("columns: {}\n", names.join(" | ")));
            }
            QueryEvent::Row { values } => {
                let line = values
                    .iter()
                    .map(|v| match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(" | ");
                self.append_result(&format!("{line}\n"));
            }
            QueryEvent::Done {
                row_count,
                duration_ms,
            } => {
                self.append_result(&format!("\n{row_count} rows in {duration_ms} ms\n"));
                if let Some(page) = self.tab_page() {
                    if !page.is_selected() {
                        page.set_needs_attention(true);
                    }
                }
            }
            QueryEvent::RowsAffected { count, duration_ms } => {
                self.append_result(&format!("{count} rows affected in {duration_ms} ms\n"));
                self.imp().results_stack.set_visible_child_name("messages");
            }
            QueryEvent::Error { message } => {
                self.append_message(&message);
                self.imp().results_stack.set_visible_child_name("messages");
            }
        }
    }

    fn clear_results(&self) {
        self.imp().results_view.buffer().set_text("");
        self.imp().messages_view.buffer().set_text("");
    }

    fn append_result(&self, text: &str) {
        let buffer = self.imp().results_view.buffer();
        let mut end = buffer.end_iter();
        buffer.insert(&mut end, text);
    }

    fn append_message(&self, text: &str) {
        let buffer = self.imp().messages_view.buffer();
        let mut end = buffer.end_iter();
        buffer.insert(&mut end, text);
        buffer.insert(&mut end, "\n");
    }

    fn copy_results_as_csv(&self) {
        let buffer = self.imp().results_view.buffer();
        let (start, end) = buffer.bounds();
        let text = buffer.text(&start, &end, false);
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&text);
        }
    }
}

#[derive(Clone, Copy)]
enum QueryMode {
    Editor,
    Selection,
}
