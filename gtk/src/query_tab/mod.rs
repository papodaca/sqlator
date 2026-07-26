mod imp;

use crate::application::SqlatorApplication;
use crate::results::{
    present_row_editor, present_sql_preview, CellValue, EditOverlay, EditToolbarState,
    RowEditorMode,
};
use crate::window::SqlatorWindow;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use sqlator_core::models::QueryEvent;
use std::collections::HashMap;
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
        tab.setup_edit_handlers();
        tab.set_placeholder_sql();
        tab
    }

    pub fn connection_id(&self) -> Option<String> {
        self.imp().connection_id.borrow().clone()
    }

    pub fn set_connection_id(&self, id: Option<String>) {
        *self.imp().connection_id.borrow_mut() = id;
    }

    pub fn has_unsaved_edits(&self) -> bool {
        self.imp().edit_state.borrow().has_changes()
    }

    pub fn discard_edits(&self) {
        // Remove pending added rows from the model before clearing state.
        let added = self.imp().edit_state.borrow_mut().take_added_model_rows();
        let mut idxs: Vec<u32> = added.into_keys().collect();
        idxs.sort_unstable_by(|a, b| b.cmp(a));
        for idx in idxs {
            self.imp().results_grid.remove_model_row(idx);
        }
        self.imp().edit_state.borrow_mut().discard_all_changes();
        self.refresh_edit_ui();
    }

    /// Drop a pending insert from both edit state and the results model, then
    /// renumber remaining added-row index keys.
    fn remove_added_row_at(&self, model_index: u32, temp_id: &str) {
        self.imp().edit_state.borrow_mut().delete_added_row(temp_id);
        self.imp().results_grid.remove_model_row(model_index);
        self.imp()
            .edit_state
            .borrow_mut()
            .shift_added_indices_after_remove(model_index);
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
            ("<Control>s", "tab.save-edits"),
            ("<Control>n", "tab.add-row"),
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

        let save_edits = gio::SimpleAction::new("save-edits", None);
        save_edits.connect_activate(glib::clone!(
            #[weak(rename_to = tab)]
            self,
            move |_, _| tab.begin_save_edits()
        ));
        group.add_action(&save_edits);

        let add_row = gio::SimpleAction::new("add-row", None);
        add_row.connect_activate(glib::clone!(
            #[weak(rename_to = tab)]
            self,
            move |_, _| tab.add_row()
        ));
        group.add_action(&add_row);

        self.insert_action_group("tab", Some(&group));
    }

    fn setup_edit_handlers(&self) {
        let grid = self.imp().results_grid.get();
        grid.connect_edit_handlers(
            glib::clone!(
                #[weak(rename_to = tab)]
                self,
                move || tab.add_row()
            ),
            glib::clone!(
                #[weak(rename_to = tab)]
                self,
                move || tab.begin_save_edits()
            ),
            glib::clone!(
                #[weak(rename_to = tab)]
                self,
                move || tab.discard_edits()
            ),
            glib::clone!(
                #[weak(rename_to = tab)]
                self,
                move || tab.delete_selected_rows()
            ),
            glib::clone!(
                #[weak(rename_to = tab)]
                self,
                move |model_index| tab.edit_row_at(model_index)
            ),
        );
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

    /// Run the editor contents (same as `tab.run` / toolbar play).
    pub fn run_editor_query(&self) {
        self.run_query(QueryMode::Editor);
    }

    /// Run the current selection, or the whole editor if nothing is selected.
    pub fn run_selection_query(&self) {
        self.run_query(QueryMode::Selection);
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

    fn resolve_db_type(&self, connection_id: &str) -> String {
        let Some(service) = self.imp().service.get() else {
            return "postgres".into();
        };
        service
            .find_saved_connection(connection_id)
            .map(|c| c.db_type)
            .unwrap_or_else(|_| "postgres".into())
    }

    fn run_query(&self, mode: QueryMode) {
        if self.is_busy() {
            return;
        }

        let Some(connection_id) = self.connection_id() else {
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

        self.run_sql(connection_id, sql);
    }

    fn run_sql(&self, connection_id: String, sql: String) {
        let service = match self.imp().service.get() {
            Some(s) => Arc::clone(s),
            None => return,
        };

        let db_type = self.resolve_db_type(&connection_id);
        self.imp()
            .edit_state
            .borrow_mut()
            .reset(&connection_id, &db_type, &sql);
        *self.imp().last_select_sql.borrow_mut() = Some(sql.clone());

        let generation = self.imp().generation.fetch_add(1, Ordering::SeqCst) + 1;
        let token = CancellationToken::new();
        *self.imp().cancel_token.borrow_mut() = Some(token.clone());
        self.set_busy(true);
        self.clear_results();
        self.imp().results_stack.set_visible_child_name("results");

        let (tx, rx) = async_channel::unbounded::<QueryEvent>();

        let conn_id = connection_id.clone();
        let sql_task = sql.clone();
        let join = crate::spawn_tokio!(async move {
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
                        let _ = db.cancel_query(&conn_id).await;
                    }
                    res = db.execute_query(&conn_id, &sql_task, event_tx) => {
                        if let Err(e) = res {
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
        let grid = self.imp().results_grid.get();
        match event {
            QueryEvent::Columns { names } => {
                grid.begin_columns(names);
                self.imp().results_stack.set_visible_child_name("results");
            }
            QueryEvent::Row { values } => {
                grid.push_row(values);
            }
            QueryEvent::Done {
                row_count,
                duration_ms,
            } => {
                grid.finish(row_count, duration_ms);
                self.imp().results_stack.set_visible_child_name("results");
                if let Some(page) = self.tab_page() {
                    if !page.is_selected() {
                        page.set_needs_attention(true);
                    }
                }
                self.fetch_edit_metadata_after_select();
            }
            QueryEvent::RowsAffected { count, duration_ms } => {
                let msg = format!("{count} rows affected in {duration_ms} ms");
                grid.show_message_line(&msg);
                self.append_message(&msg);
                self.imp().results_stack.set_visible_child_name("messages");
                self.imp().edit_state.borrow_mut().clear_all();
                self.refresh_edit_ui();
            }
            QueryEvent::Error { message } => {
                self.append_message(&message);
                self.imp().results_stack.set_visible_child_name("messages");
                self.imp().edit_state.borrow_mut().clear_all();
                self.refresh_edit_ui();
            }
        }
    }

    fn fetch_edit_metadata_after_select(&self) {
        let Some(connection_id) = self.connection_id() else {
            return;
        };
        let Some(sql) = self.imp().last_select_sql.borrow().clone() else {
            return;
        };
        let Some(service) = self.imp().service.get().map(Arc::clone) else {
            return;
        };
        let generation = self.imp().generation.load(Ordering::SeqCst);

        let join = crate::spawn_tokio!(async move {
            service
                .fetch_schema_metadata_for_sql(&connection_id, &sql)
                .await
        });

        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = tab)]
            self,
            async move {
                let result = join.await;
                if tab.imp().generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                match result {
                    Ok(Ok(meta)) => {
                        tab.imp().edit_state.borrow_mut().set_table_meta(meta);
                        tab.refresh_edit_ui();
                    }
                    Ok(Err(e)) => {
                        tracing::debug!("fetch_schema_metadata_for_sql failed: {e}");
                        tab.imp().edit_state.borrow_mut().set_table_meta(None);
                        tab.refresh_edit_ui();
                    }
                    Err(e) => {
                        tracing::debug!("fetch_schema_metadata join failed: {e}");
                    }
                }
            }
        ));
    }

    fn refresh_edit_ui(&self) {
        let grid = self.imp().results_grid.get();
        let selected = grid.selected_model_indices().len() as u32;
        let state = self.imp().edit_state.borrow();
        grid.set_edit_toolbar(&EditToolbarState {
            visible: state.has_table_meta(),
            editable: state.is_editable(),
            reason: state.editability_reason().map(str::to_string),
            change_count: state.change_count(),
            selected_count: selected,
        });

        let names = grid.column_names();
        let mut overlay = EditOverlay::default();
        let mut idx = 0u32;
        while let Some(row_map) = grid.row_map(idx) {
            if state.is_row_deleted(&row_map) {
                overlay.deleted_rows.insert(idx);
            }
            if let Some(temp) = state.added_temp_id_for_model_row(idx) {
                overlay.added_rows.insert(idx);
                if let Some(added) = state.change_set().added.get(temp) {
                    for (col_name, value) in &added.data {
                        if let Some(col_idx) = names.iter().position(|n| n == col_name) {
                            overlay
                                .cell_overrides
                                .insert((idx, col_idx), CellValue::from_json(value));
                            overlay.modified_cells.insert((idx, col_idx));
                        }
                    }
                }
            } else {
                for (col_idx, name) in names.iter().enumerate() {
                    if state.is_cell_modified(&row_map, name) {
                        let display = state.get_cell_display_value(&row_map, name);
                        overlay
                            .cell_overrides
                            .insert((idx, col_idx), CellValue::from_json(&display));
                        overlay.modified_cells.insert((idx, col_idx));
                    }
                }
            }
            idx += 1;
            if idx > 500_000 {
                break;
            }
        }
        drop(state);
        grid.set_overlay(overlay);
    }

    fn add_row(&self) {
        if !self.imp().edit_state.borrow().is_editable() {
            return;
        }
        let grid = self.imp().results_grid.get();
        let temp_id = self.imp().edit_state.borrow_mut().add_row();
        let model_index = grid.append_empty_row();
        self.imp()
            .edit_state
            .borrow_mut()
            .register_added_model_row(model_index, temp_id.clone());

        let Some(meta) = self.imp().edit_state.borrow().table_meta().cloned() else {
            return;
        };
        let names = grid.column_names();
        let current = HashMap::new();
        present_row_editor(
            self,
            &meta,
            &names,
            &current,
            RowEditorMode::Added,
            glib::clone!(
                #[weak(rename_to = tab)]
                self,
                move |result| {
                    let Some(result) = result else {
                        tab.remove_added_row_at(model_index, &temp_id);
                        tab.refresh_edit_ui();
                        return;
                    };
                    if result.delete {
                        tab.remove_added_row_at(model_index, &temp_id);
                        tab.refresh_edit_ui();
                        return;
                    }
                    {
                        let mut state = tab.imp().edit_state.borrow_mut();
                        for (col, value) in &result.values {
                            state.modify_added_cell(&temp_id, col, value.clone());
                        }
                    }
                    tab.imp()
                        .results_grid
                        .update_row_from_map(model_index, &result.values);
                    tab.refresh_edit_ui();
                }
            ),
        );
    }

    fn edit_row_at(&self, model_index: u32) {
        if !self.imp().edit_state.borrow().is_editable() {
            return;
        }
        let grid = self.imp().results_grid.get();
        let Some(meta) = self.imp().edit_state.borrow().table_meta().cloned() else {
            return;
        };
        let names = grid.column_names();

        if let Some(temp_id) = self
            .imp()
            .edit_state
            .borrow()
            .added_temp_id_for_model_row(model_index)
            .map(str::to_string)
        {
            let current = self
                .imp()
                .edit_state
                .borrow()
                .change_set()
                .added
                .get(&temp_id)
                .map(|r| r.data.clone())
                .unwrap_or_default();
            present_row_editor(
                self,
                &meta,
                &names,
                &current,
                RowEditorMode::Added,
                glib::clone!(
                    #[weak(rename_to = tab)]
                    self,
                    move |result| {
                        let Some(result) = result else {
                            return;
                        };
                        if result.delete {
                            tab.remove_added_row_at(model_index, &temp_id);
                            tab.refresh_edit_ui();
                            return;
                        }
                        {
                            let mut state = tab.imp().edit_state.borrow_mut();
                            for (col, value) in &result.values {
                                state.modify_added_cell(&temp_id, col, value.clone());
                            }
                        }
                        tab.imp()
                            .results_grid
                            .update_row_from_map(model_index, &result.values);
                        tab.refresh_edit_ui();
                    }
                ),
            );
            return;
        }

        let Some(original) = grid.row_map(model_index) else {
            return;
        };
        let mut current = original.clone();
        {
            let state = self.imp().edit_state.borrow();
            for name in &names {
                current.insert(name.clone(), state.get_cell_display_value(&original, name));
            }
        }

        let names_for_cb = names.clone();
        present_row_editor(
            self,
            &meta,
            &names,
            &current,
            RowEditorMode::Existing,
            glib::clone!(
                #[weak(rename_to = tab)]
                self,
                move |result| {
                    let Some(result) = result else {
                        return;
                    };
                    if result.delete {
                        tab.imp().edit_state.borrow_mut().delete_row(&original);
                        tab.refresh_edit_ui();
                        return;
                    }
                    {
                        let mut state = tab.imp().edit_state.borrow_mut();
                        for (col, value) in &result.values {
                            if names_for_cb.iter().any(|n| n == col) {
                                state.modify_cell(&original, col, value.clone());
                            }
                        }
                    }
                    tab.refresh_edit_ui();
                }
            ),
        );
    }

    fn delete_selected_rows(&self) {
        if !self.imp().edit_state.borrow().is_editable() {
            return;
        }
        let grid = self.imp().results_grid.get();
        let indices = grid.selected_model_indices();
        let mut remove_added: Vec<(u32, String)> = Vec::new();
        for idx in indices {
            if let Some(temp) = self
                .imp()
                .edit_state
                .borrow()
                .added_temp_id_for_model_row(idx)
                .map(str::to_string)
            {
                remove_added.push((idx, temp));
            } else if let Some(row) = grid.row_map(idx) {
                self.imp().edit_state.borrow_mut().delete_row(&row);
            }
        }
        remove_added.sort_by_key(|b| std::cmp::Reverse(b.0));
        for (idx, temp) in remove_added {
            self.remove_added_row_at(idx, &temp);
        }
        self.refresh_edit_ui();
    }

    fn begin_save_edits(&self) {
        let batch = self.imp().edit_state.borrow().generate_batch();
        let Some(batch) = batch else {
            return;
        };
        present_sql_preview(
            self,
            &batch,
            glib::clone!(
                #[weak(rename_to = tab)]
                self,
                #[strong]
                batch,
                move || tab.execute_edits_batch(batch.clone())
            ),
        );
    }

    fn execute_edits_batch(&self, batch: sqlator_core::models::SqlBatch) {
        let Some(connection_id) = self
            .imp()
            .edit_state
            .borrow()
            .connection_id()
            .map(str::to_string)
        else {
            return;
        };
        let Some(service) = self.imp().service.get().map(Arc::clone) else {
            return;
        };

        let join =
            crate::spawn_tokio!(
                async move { service.db().execute_batch(&connection_id, &batch).await }
            );

        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = tab)]
            self,
            async move {
                match join.await {
                    Ok(Ok(result)) => {
                        if result.success {
                            tab.imp().edit_state.borrow_mut().discard_all_changes();
                            tab.imp().toast_overlay.add_toast(adw::Toast::new(&format!(
                                "Applied {} statement{}",
                                result.executed_count,
                                if result.executed_count == 1 { "" } else { "s" }
                            )));
                            if let (Some(conn), Some(sql)) = (
                                tab.connection_id(),
                                tab.imp().last_select_sql.borrow().clone(),
                            ) {
                                tab.run_sql(conn, sql);
                            } else {
                                tab.refresh_edit_ui();
                            }
                        } else {
                            let msg = result
                                .error
                                .map(|e| e.message)
                                .unwrap_or_else(|| "Batch failed".into());
                            tab.imp()
                                .toast_overlay
                                .add_toast(adw::Toast::new(&format!("Save failed: {msg}")));
                        }
                    }
                    Ok(Err(e)) => {
                        tab.imp()
                            .toast_overlay
                            .add_toast(adw::Toast::new(&format!("Save failed: {e}")));
                    }
                    Err(e) => {
                        tab.imp()
                            .toast_overlay
                            .add_toast(adw::Toast::new(&format!("Save failed: {e}")));
                    }
                }
            }
        ));
    }

    fn clear_results(&self) {
        self.imp().results_grid.clear();
        self.imp().messages_view.buffer().set_text("");
    }

    fn append_message(&self, text: &str) {
        let buffer = self.imp().messages_view.buffer();
        let mut end = buffer.end_iter();
        buffer.insert(&mut end, text);
        buffer.insert(&mut end, "\n");
    }

    fn copy_results_as_csv(&self) {
        let text = self.imp().results_grid.copy_selection_tsv();
        if text.is_empty() {
            return;
        }
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