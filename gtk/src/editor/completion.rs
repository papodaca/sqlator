//! Schema-aware GtkSourceView completion (in-memory prefix match only).

use gio::subclass::prelude::*;
use gtk::prelude::*;
use gtk::{gio, glib};
use sourceview::prelude::*;
use sourceview::subclass::prelude::*;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};

/// In-memory schema for completion — never query the catalog from `populate_future`.
#[derive(Debug, Clone, Default)]
pub struct SchemaSnapshot {
    /// table key (short and/or `schema.table`) → column names
    pub tables: BTreeMap<String, Vec<String>>,
}

impl SchemaSnapshot {
    pub fn from_tables_and_columns(
        tables: &[(String, Option<String>, String)],
        columns: &BTreeMap<String, Vec<String>>,
    ) -> Self {
        // tables entries: (name, schema, full_name)
        let mut map = BTreeMap::new();
        for (name, schema, full_name) in tables {
            let cols = columns
                .get(name)
                .or_else(|| columns.get(full_name))
                .cloned()
                .unwrap_or_default();
            map.insert(name.clone(), cols.clone());
            if !full_name.is_empty() && full_name != name {
                map.insert(full_name.clone(), cols.clone());
            }
            if let Some(schema) = schema {
                let qualified = format!("{schema}.{name}");
                map.insert(qualified, cols);
            }
        }
        Self { tables: map }
    }

    fn match_prefix(&self, word: &str, after_dot: Option<&str>) -> Vec<(String, String)> {
        let word_lower = word.to_ascii_lowercase();
        let mut out = Vec::new();
        if let Some(table) = after_dot {
            // Column completions for `table.` / `schema.table.`
            let key_candidates = [table.to_string(), table.to_ascii_lowercase()];
            for key in &key_candidates {
                if let Some(cols) = self.tables.get(key) {
                    for col in cols {
                        if word.is_empty() || col.to_ascii_lowercase().starts_with(&word_lower) {
                            out.push((col.clone(), format!("column · {key}")));
                        }
                    }
                    break;
                }
                // Case-insensitive table key lookup
                if let Some((_, cols)) = self
                    .tables
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(key))
                {
                    for col in cols {
                        if word.is_empty() || col.to_ascii_lowercase().starts_with(&word_lower) {
                            out.push((col.clone(), format!("column · {key}")));
                        }
                    }
                    break;
                }
            }
        } else {
            for (table, cols) in &self.tables {
                if word.is_empty() || table.to_ascii_lowercase().starts_with(&word_lower) {
                    let detail = if cols.is_empty() {
                        "table".to_string()
                    } else {
                        format!("table · {} cols", cols.len())
                    };
                    out.push((table.clone(), detail));
                }
            }
        }
        out.sort_by_key(|a| a.0.to_ascii_lowercase());
        out.truncate(50);
        out
    }
}

mod proposal {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct SchemaProposal {
        pub typed_text: RefCell<String>,
        pub detail: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SchemaProposal {
        const NAME: &'static str = "SqlatorSchemaProposal";
        type Type = super::SchemaProposal;
        type Interfaces = (sourceview::CompletionProposal,);
    }

    impl ObjectImpl for SchemaProposal {
        fn properties() -> &'static [glib::ParamSpec] {
            use std::sync::OnceLock;
            static PROPS: OnceLock<Vec<glib::ParamSpec>> = OnceLock::new();
            PROPS.get_or_init(|| {
                vec![glib::ParamSpecString::builder("typed-text")
                    .nick("Typed Text")
                    .blurb("Text inserted when the proposal is activated")
                    .readwrite()
                    .build()]
            })
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "typed-text" => {
                    *self.typed_text.borrow_mut() = value.get::<String>().unwrap_or_default();
                }
                _ => unimplemented!("{}", pspec.name()),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "typed-text" => self.typed_text.borrow().to_value(),
                _ => unimplemented!("{}", pspec.name()),
            }
        }
    }

    impl sourceview::subclass::completion_proposal::CompletionProposalImpl for SchemaProposal {}
}

glib::wrapper! {
    pub struct SchemaProposal(ObjectSubclass<proposal::SchemaProposal>)
        @implements sourceview::CompletionProposal;
}

impl SchemaProposal {
    pub fn new(typed_text: &str, detail: &str) -> Self {
        let obj: Self = glib::Object::builder()
            .property("typed-text", typed_text)
            .build();
        *obj.imp().detail.borrow_mut() = detail.to_string();
        obj
    }

    pub fn detail(&self) -> String {
        self.imp().detail.borrow().clone()
    }
}

mod provider {
    use super::*;

    #[derive(Default)]
    pub struct SchemaCompletionProvider {
        pub snapshot: RwLock<Arc<SchemaSnapshot>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SchemaCompletionProvider {
        const NAME: &'static str = "SqlatorSchemaCompletionProvider";
        type Type = super::SchemaCompletionProvider;
        type Interfaces = (sourceview::CompletionProvider,);
    }

    impl ObjectImpl for SchemaCompletionProvider {}

    impl CompletionProviderImpl for SchemaCompletionProvider {
        fn title(&self) -> Option<glib::GString> {
            Some(glib::GString::from("Schema"))
        }

        fn priority(&self, _context: &sourceview::CompletionContext) -> i32 {
            10
        }

        fn is_trigger(&self, _iter: &gtk::TextIter, c: char) -> bool {
            c == '.'
        }

        fn display(
            &self,
            _context: &sourceview::CompletionContext,
            proposal: &sourceview::CompletionProposal,
            cell: &sourceview::CompletionCell,
        ) {
            let Ok(p) = proposal.clone().downcast::<super::SchemaProposal>() else {
                return;
            };
            match cell.column() {
                sourceview::CompletionColumn::TypedText => {
                    cell.set_text(p.typed_text().as_deref());
                }
                sourceview::CompletionColumn::Comment => {
                    cell.set_text(Some(&p.detail()));
                }
                sourceview::CompletionColumn::Icon => {
                    cell.set_icon_name("x-office-spreadsheet-symbolic");
                }
                _ => {}
            }
        }

        fn activate(
            &self,
            context: &sourceview::CompletionContext,
            proposal: &sourceview::CompletionProposal,
        ) {
            let Some(text) = proposal.typed_text() else {
                return;
            };
            let Some(buffer) = context.buffer() else {
                return;
            };
            let Some((mut start, mut end)) = context.bounds() else {
                return;
            };
            buffer.begin_user_action();
            buffer.delete(&mut start, &mut end);
            buffer.insert(&mut start, &text);
            buffer.end_user_action();
        }

        fn populate_future(
            &self,
            context: &sourceview::CompletionContext,
        ) -> Pin<Box<dyn Future<Output = Result<gio::ListModel, glib::Error>>>> {
            let word = context.word().to_string();
            let snapshot = self
                .snapshot
                .read()
                .map(|g| Arc::clone(&g))
                .unwrap_or_default();

            // Detect `table.` / `schema.table.` qualifier immediately before the word.
            let after_dot = {
                let mut qualifier = None;
                if let Some((start, _)) = context.bounds() {
                    let mut iter = start;
                    if iter.backward_char() && iter.char() == '.' {
                        let end = iter;
                        let mut begin = iter;
                        // Walk back over identifier / dots for schema.table
                        while begin.backward_char() {
                            let ch = begin.char();
                            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' {
                                continue;
                            }
                            begin.forward_char();
                            break;
                        }
                        let q = begin.text(&end).to_string();
                        if !q.is_empty() {
                            qualifier = Some(q);
                        }
                    }
                }
                qualifier
            };

            let matches = snapshot.match_prefix(&word, after_dot.as_deref());
            Box::pin(async move {
                let store = gio::ListStore::new::<super::SchemaProposal>();
                for (text, detail) in matches {
                    store.append(&super::SchemaProposal::new(&text, &detail));
                }
                Ok(store.upcast())
            })
        }
    }
}

glib::wrapper! {
    pub struct SchemaCompletionProvider(ObjectSubclass<provider::SchemaCompletionProvider>)
        @implements sourceview::CompletionProvider;
}

impl SchemaCompletionProvider {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn set_snapshot(&self, snapshot: Arc<SchemaSnapshot>) {
        if let Ok(mut guard) = self.imp().snapshot.write() {
            *guard = snapshot;
        }
    }

    pub fn snapshot(&self) -> Arc<SchemaSnapshot> {
        self.imp()
            .snapshot
            .read()
            .map(|g| Arc::clone(&g))
            .unwrap_or_default()
    }
}

impl Default for SchemaCompletionProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Attach schema + words + snippets providers to a SourceView.
pub fn attach_providers(view: &sourceview::View, schema_provider: &SchemaCompletionProvider) {
    let completion = view.completion();
    completion.add_provider(schema_provider);

    let words = sourceview::CompletionWords::new(Some("Words"));
    words.register(&view.buffer());
    completion.add_provider(&words);

    let snippets = sourceview::CompletionSnippets::new();
    completion.add_provider(&snippets);
}

/// Prefetch tables/columns into a [`SchemaSnapshot`] via `AppService`.
pub async fn load_snapshot(
    service: Arc<sqlator_service::AppService>,
    connection_id: String,
) -> SchemaSnapshot {
    let db = service.db_handle();
    let schemas = match db.get_schemas(&connection_id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!("completion get_schemas failed: {e}");
            return SchemaSnapshot::default();
        }
    };

    let preferred = schemas
        .iter()
        .find(|s| s.is_default)
        .or_else(|| schemas.first())
        .map(|s| s.name.clone());

    let tables = match db.get_tables(&connection_id, preferred.as_deref()).await {
        Ok(t) => t,
        Err(e) => {
            tracing::debug!("completion get_tables failed: {e}");
            return SchemaSnapshot::default();
        }
    };

    let table_meta: Vec<(String, Option<String>, String)> = tables
        .iter()
        .map(|t| (t.name.clone(), t.schema.clone(), t.full_name.clone()))
        .collect();

    let mut columns: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // Cap concurrency-ish by chunking.
    for chunk in tables.chunks(10) {
        for table in chunk {
            match db
                .get_columns(&connection_id, &table.name, table.schema.as_deref())
                .await
            {
                Ok(cols) => {
                    let names: Vec<String> = cols.into_iter().map(|c| c.name).collect();
                    columns.insert(table.name.clone(), names.clone());
                    columns.insert(table.full_name.clone(), names);
                }
                Err(e) => {
                    tracing::debug!(
                        "completion get_columns({}.{}) failed: {e}",
                        table.schema.as_deref().unwrap_or(""),
                        table.name
                    );
                }
            }
        }
    }

    SchemaSnapshot::from_tables_and_columns(&table_meta, &columns)
}
