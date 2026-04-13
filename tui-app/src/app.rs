use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::widgets::ListState;
use sqlator_core::config::ConfigManager;
use sqlator_core::db::DbManager;
use sqlator_core::error::CoreError;
use sqlator_core::models::{QueryEvent, SavedConnection, SchemaColumnInfo, SchemaInfo, TableInfo};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

use crate::ui;

const MAX_RESULT_ROWS: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    ConnectionList,
    Connecting,
    Workspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPane {
    Schema,
    Editor,
    Results,
}

#[derive(Debug, Clone)]
pub struct VisibleItem {
    pub label: String,
    pub depth: usize,
    pub kind: ItemKind,
    pub schema_name: Option<String>,
    pub table_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Schema,
    Table,
    Column,
}

struct SchemaLoadResult {
    schemas: Vec<SchemaInfo>,
    tables_by_schema: HashMap<String, Vec<TableInfo>>,
}

struct ColumnLoadResult {
    schema: Option<String>,
    table: String,
    columns: Vec<SchemaColumnInfo>,
}

pub struct App {
    rt: tokio::runtime::Handle,
    db: Arc<DbManager>,
    config: ConfigManager,

    pub mode: AppMode,
    pub should_quit: bool,

    pub connections: Vec<SavedConnection>,
    pub conn_list_state: ListState,
    pub active_connection: Option<SavedConnection>,

    connect_rx: Option<oneshot::Receiver<Result<(), CoreError>>>,
    pub connect_error: Option<String>,
    connect_spinner: u8,

    pub focus: FocusPane,

    pub schemas: Vec<SchemaInfo>,
    pub tables_by_schema: HashMap<String, Vec<TableInfo>>,
    pub columns_cache: HashMap<(Option<String>, String), Vec<SchemaColumnInfo>>,
    expanded_schemas: HashSet<String>,
    expanded_tables: HashSet<String>,
    pub visible_items: Vec<VisibleItem>,
    pub schema_list_state: ListState,
    pub schema_loading: bool,
    schema_rx: Option<mpsc::Receiver<SchemaLoadResult>>,
    column_rx: Option<mpsc::Receiver<ColumnLoadResult>>,

    pub editor: tui_textarea::TextArea<'static>,

    pub result_columns: Vec<String>,
    pub result_rows: Vec<Vec<String>>,
    pub result_status: String,
    pub result_table_state: ratatui::widgets::TableState,

    pub query_running: bool,
    query_rx: Option<mpsc::Receiver<QueryEvent>>,
}

impl App {
    pub fn new(rt: tokio::runtime::Handle) -> Self {
        let config = ConfigManager::new("sqlator").expect("Failed to init config manager");
        let db = Arc::new(DbManager::new());
        let connections = config.get_connections().unwrap_or_default();

        let mut conn_list_state = ListState::default();
        if !connections.is_empty() {
            conn_list_state.select(Some(0));
        }

        let editor = tui_textarea::TextArea::default();

        Self {
            rt,
            db,
            config,
            mode: AppMode::ConnectionList,
            should_quit: false,
            connections,
            conn_list_state,
            active_connection: None,
            connect_rx: None,
            connect_error: None,
            connect_spinner: 0,
            focus: FocusPane::Editor,
            schemas: Vec::new(),
            tables_by_schema: HashMap::new(),
            columns_cache: HashMap::new(),
            expanded_schemas: HashSet::new(),
            expanded_tables: HashSet::new(),
            visible_items: Vec::new(),
            schema_list_state: ListState::default(),
            schema_loading: false,
            schema_rx: None,
            column_rx: None,
            editor,
            result_columns: Vec::new(),
            result_rows: Vec::new(),
            result_status: String::new(),
            result_table_state: ratatui::widgets::TableState::default(),
            query_running: false,
            query_rx: None,
        }
    }

    pub fn run(
        &mut self,
        terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        while !self.should_quit {
            terminal.draw(|f| self.draw(f))?;

            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        self.handle_key(key);
                    }
                }
            }

            self.poll_async_events();
            self.connect_spinner = (self.connect_spinner + 1) % 8;
        }

        if let Some(ref conn) = self.active_connection {
            let db = self.db.clone();
            let id = conn.id.clone();
            self.rt.spawn(async move {
                db.disconnect(&id).await;
            });
        }

        Ok(())
    }

    fn draw(&self, f: &mut ratatui::Frame) {
        match self.mode {
            AppMode::ConnectionList => ui::draw_connection_list(f, self),
            AppMode::Connecting => ui::draw_connecting(f, self),
            AppMode::Workspace => ui::draw_workspace(f, self),
        }
    }

    fn poll_async_events(&mut self) {
        self.poll_connect_result();
        self.poll_schema_result();
        self.poll_column_result();
        self.poll_query_events();
    }

    fn poll_connect_result(&mut self) {
        if let Some(mut rx) = self.connect_rx.take() {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    self.mode = AppMode::Workspace;
                    self.focus = FocusPane::Editor;
                    self.start_schema_load();
                }
                Ok(Err(e)) => {
                    self.connect_error = Some(e.message.clone());
                    self.mode = AppMode::ConnectionList;
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    self.connect_rx = Some(rx);
                }
                Err(_) => {
                    self.connect_error = Some("Connection task dropped".into());
                    self.mode = AppMode::ConnectionList;
                }
            }
        }
    }

    fn poll_schema_result(&mut self) {
        if let Some(mut rx) = self.schema_rx.take() {
            match rx.try_recv() {
                Ok(result) => {
                    self.schemas = result.schemas;
                    self.tables_by_schema = result.tables_by_schema;
                    self.schema_loading = false;
                    self.rebuild_visible_items();
                    if !self.visible_items.is_empty() {
                        self.schema_list_state.select(Some(0));
                    }
                }
                Err(mpsc::error::TryRecvError::Empty) => {
                    self.schema_rx = Some(rx);
                }
                Err(_) => {
                    self.schema_loading = false;
                }
            }
        }
    }

    fn poll_column_result(&mut self) {
        if let Some(mut rx) = self.column_rx.take() {
            match rx.try_recv() {
                Ok(result) => {
                    let key = (result.schema, result.table);
                    self.columns_cache.insert(key, result.columns);
                    self.rebuild_visible_items();
                }
                Err(mpsc::error::TryRecvError::Empty) => {
                    self.column_rx = Some(rx);
                }
                Err(_) => {}
            }
        }
    }

    fn poll_query_events(&mut self) {
        let events: Vec<QueryEvent> = if let Some(ref mut rx) = self.query_rx {
            let mut events = Vec::new();
            loop {
                match rx.try_recv() {
                    Ok(event) => events.push(event),
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(_) => {
                        self.query_running = false;
                        self.query_rx = None;
                        break;
                    }
                }
            }
            events
        } else {
            return;
        };

        for event in events {
            self.handle_query_event(event);
        }
    }

    fn handle_query_event(&mut self, event: QueryEvent) {
        match event {
            QueryEvent::Columns { names } => {
                self.result_columns = names;
            }
            QueryEvent::Row { values } => {
                if self.result_rows.len() < MAX_RESULT_ROWS {
                    let row: Vec<String> = values
                        .iter()
                        .map(|v: &serde_json::Value| match v {
                            serde_json::Value::Null => "NULL".into(),
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .collect();
                    self.result_rows.push(row);
                }
            }
            QueryEvent::Done { row_count, duration_ms } => {
                self.result_status = format!("{} rows in {}ms", row_count, duration_ms);
                self.query_running = false;
                self.query_rx = None;
            }
            QueryEvent::RowsAffected { count, duration_ms } => {
                self.result_status = format!("{} rows affected in {}ms", count, duration_ms);
                self.query_running = false;
                self.query_rx = None;
            }
            QueryEvent::Error { message } => {
                self.result_status = format!("Error: {}", message);
                self.result_rows.clear();
                self.result_columns.clear();
                self.query_running = false;
                self.query_rx = None;
            }
        }
    }

    fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        match self.mode {
            AppMode::ConnectionList => self.handle_connection_list_key(key),
            AppMode::Connecting => {}
            AppMode::Workspace => self.handle_workspace_key(key),
        }
    }

    fn handle_connection_list_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.should_quit = true;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.conn_list_state.selected().unwrap_or(0);
                if i > 0 {
                    self.conn_list_state.select(Some(i - 1));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.connections.len();
                if len > 0 {
                    let i = self.conn_list_state.selected().unwrap_or(0);
                    if i < len - 1 {
                        self.conn_list_state.select(Some(i + 1));
                    }
                }
            }
            KeyCode::Enter => {
                if let Some(idx) = self.conn_list_state.selected() {
                    if let Some(conn) = self.connections.get(idx).cloned() {
                        self.start_connect(conn);
                    }
                }
            }
            _ => {}
        }
    }

    fn start_connect(&mut self, conn: SavedConnection) {
        self.mode = AppMode::Connecting;
        self.connect_error = None;
        self.active_connection = Some(conn.clone());

        let db = self.db.clone();
        let conn_id = conn.id.clone();
        let url = conn.url.clone();
        let (tx, rx) = oneshot::channel();

        self.rt.spawn(async move {
            let result = db.connect(&conn_id, &url).await;
            let _ = tx.send(result);
        });

        self.connect_rx = Some(rx);
    }

    fn start_schema_load(&mut self) {
        let Some(ref conn) = self.active_connection else {
            return;
        };

        self.schema_loading = true;
        let db = self.db.clone();
        let conn_id = conn.id.clone();
        let (tx, rx) = mpsc::channel(1);

        self.rt.spawn(async move {
            let schemas = db.get_schemas(&conn_id).await.unwrap_or_default();
            let mut tables_by_schema: HashMap<String, Vec<TableInfo>> = HashMap::new();

            for schema in &schemas {
                let tables = db.get_tables(&conn_id, Some(&schema.name)).await.unwrap_or_default();
                tables_by_schema.insert(schema.name.clone(), tables);
            }

            if schemas.is_empty() {
                let tables = db.get_tables(&conn_id, None).await.unwrap_or_default();
                tables_by_schema.insert(String::new(), tables);
            }

            let _ = tx.send(SchemaLoadResult { schemas, tables_by_schema }).await;
        });

        self.schema_rx = Some(rx);
    }

    fn rebuild_visible_items(&mut self) {
        self.visible_items.clear();

        if self.schemas.is_empty() {
            if let Some(tables) = self.tables_by_schema.get("") {
                for table in tables {
                    let icon = if table.table_type == "view" { "\u{2299}" } else { "\u{25AA}" };
                    let expanded = self.expanded_tables.contains(&table.name);
                    let exp_icon = if expanded { "\u{25BE}" } else { "\u{25B8}" };

                    self.visible_items.push(VisibleItem {
                        label: format!("{} {} {}", exp_icon, icon, table.name),
                        depth: 0,
                        kind: ItemKind::Table,
                        schema_name: None,
                        table_name: Some(table.name.clone()),
                    });

                    if expanded {
                        if let Some(cols) = self.columns_cache.get(&(None, table.name.clone())) {
                            for col in cols {
                                let pk = if col.is_primary_key { " \u{1F511}" } else { "" };
                                self.visible_items.push(VisibleItem {
                                    label: format!("  {} {}{}", col.name, col.data_type, pk),
                                    depth: 1,
                                    kind: ItemKind::Column,
                                    schema_name: None,
                                    table_name: Some(table.name.clone()),
                                });
                            }
                        }
                    }
                }
            }
            return;
        }

        for schema in &self.schemas {
            let expanded = self.expanded_schemas.contains(&schema.name);
            let exp_icon = if expanded { "\u{25BE}" } else { "\u{25B8}" };
            let default_marker = if schema.is_default { " (default)" } else { "" };

            self.visible_items.push(VisibleItem {
                label: format!("{} {}{}", exp_icon, schema.name, default_marker),
                depth: 0,
                kind: ItemKind::Schema,
                schema_name: Some(schema.name.clone()),
                table_name: None,
            });

            if expanded {
                if let Some(tables) = self.tables_by_schema.get(&schema.name) {
                    for table in tables {
                        let icon = if table.table_type == "view" { "\u{2299}" } else { "\u{25AA}" };
                        let tbl_expanded = self.expanded_tables.contains(&table.full_name);
                        let exp_icon = if tbl_expanded { "\u{25BE}" } else { "\u{25B8}" };

                        self.visible_items.push(VisibleItem {
                            label: format!("  {} {} {}", exp_icon, icon, table.name),
                            depth: 1,
                            kind: ItemKind::Table,
                            schema_name: Some(schema.name.clone()),
                            table_name: Some(table.name.clone()),
                        });

                        if tbl_expanded {
                            let key = (Some(schema.name.clone()), table.name.clone());
                            if let Some(cols) = self.columns_cache.get(&key) {
                                for col in cols {
                                    let pk = if col.is_primary_key { " \u{1F511}" } else { "" };
                                    self.visible_items.push(VisibleItem {
                                        label: format!("    {} {}{}", col.name, col.data_type, pk),
                                        depth: 2,
                                        kind: ItemKind::Column,
                                        schema_name: Some(schema.name.clone()),
                                        table_name: Some(table.name.clone()),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn handle_workspace_key(&mut self, key: crossterm::event::KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        if key.code == KeyCode::Esc {
            if self.focus == FocusPane::Editor {
                self.disconnect_and_return_to_list();
            } else {
                self.focus = FocusPane::Editor;
            }
            return;
        }

        if key.code == KeyCode::Tab && !key.modifiers.contains(KeyModifiers::SHIFT) {
            self.cycle_focus(false);
            return;
        }
        if key.code == KeyCode::BackTab
            || (key.modifiers.contains(KeyModifiers::SHIFT) && key.code == KeyCode::Tab)
        {
            self.cycle_focus(true);
            return;
        }

        match self.focus {
            FocusPane::Schema => self.handle_schema_key(key),
            FocusPane::Editor => self.handle_editor_key(key),
            FocusPane::Results => self.handle_results_key(key),
        }
    }

    fn cycle_focus(&mut self, backward: bool) {
        let order = [FocusPane::Schema, FocusPane::Editor, FocusPane::Results];
        let current = order.iter().position(|&p| p == self.focus).unwrap_or(1);
        let next = if backward {
            (current + order.len() - 1) % order.len()
        } else {
            (current + 1) % order.len()
        };
        self.focus = order[next];
    }

    fn disconnect_and_return_to_list(&mut self) {
        if let Some(ref conn) = self.active_connection {
            let db = self.db.clone();
            let id = conn.id.clone();
            self.rt.spawn(async move {
                db.disconnect(&id).await;
            });
        }
        self.active_connection = None;
        self.schemas.clear();
        self.tables_by_schema.clear();
        self.columns_cache.clear();
        self.expanded_schemas.clear();
        self.expanded_tables.clear();
        self.visible_items.clear();
        self.result_columns.clear();
        self.result_rows.clear();
        self.result_status.clear();
        self.editor = tui_textarea::TextArea::default();
        self.mode = AppMode::ConnectionList;
        self.connect_error = None;
        self.connections = self.config.get_connections().unwrap_or_default();
        if !self.connections.is_empty() {
            self.conn_list_state.select(Some(0));
        }
    }

    fn handle_schema_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.schema_list_state.selected().unwrap_or(0);
                if i > 0 {
                    self.schema_list_state.select(Some(i - 1));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.visible_items.len();
                if len > 0 {
                    let i = self.schema_list_state.selected().unwrap_or(0);
                    if i < len - 1 {
                        self.schema_list_state.select(Some(i + 1));
                    }
                }
            }
            KeyCode::Enter => {
                if let Some(idx) = self.schema_list_state.selected() {
                    if let Some(item) = self.visible_items.get(idx).cloned() {
                        match item.kind {
                            ItemKind::Schema => {
                                if let Some(ref sname) = item.schema_name {
                                    if self.expanded_schemas.contains(sname) {
                                        self.expanded_schemas.remove(sname);
                                    } else {
                                        self.expanded_schemas.insert(sname.clone());
                                    }
                                    self.rebuild_visible_items();
                                }
                            }
                            ItemKind::Table => {
                                let key = item.table_name.as_deref().unwrap_or("");
                                let full_key = match &item.schema_name {
                                    Some(s) => format!("{}.{}", s, key),
                                    None => key.to_string(),
                                };
                                if self.expanded_tables.contains(&full_key) {
                                    self.expanded_tables.remove(&full_key);
                                } else {
                                    self.expanded_tables.insert(full_key.clone());
                                    self.load_columns_for_table(
                                        item.schema_name.clone(),
                                        item.table_name.clone().unwrap_or_default(),
                                    );
                                }
                                self.rebuild_visible_items();
                            }
                            ItemKind::Column => {}
                        }
                    }
                }
            }
            KeyCode::Char('q') => {
                self.should_quit = true;
            }
            _ => {}
        }
    }

    fn load_columns_for_table(&mut self, schema: Option<String>, table: String) {
        let key = (schema.clone(), table.clone());
        if self.columns_cache.contains_key(&key) {
            return;
        }

        let db = self.db.clone();
        let Some(ref conn) = self.active_connection else {
            return;
        };
        let conn_id = conn.id.clone();
        let schema_clone = schema.clone();
        let table_clone = table.clone();

        let (tx, rx) = mpsc::channel(1);
        self.column_rx = Some(rx);

        self.rt.spawn(async move {
            let columns = db.get_columns(&conn_id, &table_clone, schema_clone.as_deref()).await.unwrap_or_default();
            let _ = tx.send(ColumnLoadResult {
                schema: schema_clone,
                table: table_clone,
                columns,
            }).await;
        });
    }

    fn handle_editor_key(&mut self, key: crossterm::event::KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('e') {
            self.execute_query();
            return;
        }

        self.editor.input(key);
    }

    fn handle_results_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.result_table_state.selected().unwrap_or(0);
                if i > 0 {
                    self.result_table_state.select(Some(i - 1));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.result_rows.len();
                if len > 0 {
                    let i = self.result_table_state.selected().unwrap_or(0);
                    if i < len - 1 {
                        self.result_table_state.select(Some(i + 1));
                    }
                }
            }
            KeyCode::Char('q') => {
                self.should_quit = true;
            }
            _ => {}
        }
    }

    fn execute_query(&mut self) {
        if self.query_running {
            return;
        }

        let sql: String = self.editor.lines().join("\n");
        if sql.trim().is_empty() {
            return;
        }

        let Some(ref conn) = self.active_connection else {
            return;
        };
        let conn_id = conn.id.clone();

        let (tx, rx) = mpsc::channel(256);
        self.query_rx = Some(rx);
        self.query_running = true;
        self.result_columns.clear();
        self.result_rows.clear();
        self.result_status = "Executing...".into();

        let db = self.db.clone();
        self.rt.spawn(async move {
            let _ = db.execute_query(&conn_id, &sql, tx).await;
        });

        self.focus = FocusPane::Results;
    }

    pub fn spinner_char(&self) -> &str {
        match self.connect_spinner {
            0 => "\u{280B}",
            1 => "\u{2819}",
            2 => "\u{2839}",
            3 => "\u{2838}",
            4 => "\u{283C}",
            5 => "\u{2834}",
            6 => "\u{2826}",
            7 => "\u{2807}",
            _ => "\u{280B}",
        }
    }
}
