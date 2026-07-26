//! `GtkTreeListModel` schema browser with async expand loaders.

use super::item::{SchemaItem, SchemaKind};
use crate::window::SqlatorWindow;
use adw::prelude::*;
use gtk::{gio, pango};
use sqlator_core::models::{SchemaColumnInfo, SchemaInfo, TableInfo};
use std::cell::RefCell;

const ROW_DATA_KEY: &str = "sqlator-schema-row";

/// Owns the schema `ListView` model and drives lazy expands via AppService.
pub struct SchemaTree {
    root: gio::ListStore,
    connection_id: RefCell<Option<String>>,
}

impl std::fmt::Debug for SchemaTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchemaTree")
            .field("connection_id", &*self.connection_id.borrow())
            .field("n_root", &self.root.n_items())
            .finish()
    }
}

impl SchemaTree {
    pub fn attach(window: &SqlatorWindow, list_view: &gtk::ListView) -> Self {
        let root = gio::ListStore::new::<SchemaItem>();
        let window_weak = window.downgrade();

        let tree_model = gtk::TreeListModel::new(root.clone(), false, false, move |obj| {
            let window = window_weak.upgrade()?;
            let item = obj.downcast_ref::<SchemaItem>()?;
            match item.kind() {
                SchemaKind::Schema { name, .. } => {
                    let store = gio::ListStore::new::<SchemaItem>();
                    store.append(&SchemaItem::loading("Loading tables…"));
                    window.fill_schema_tables(&name, store.clone());
                    Some(store.upcast())
                }
                SchemaKind::Table { name, schema, .. } => {
                    let store = gio::ListStore::new::<SchemaItem>();
                    store.append(&SchemaItem::loading("Loading columns…"));
                    window.fill_schema_columns(schema.as_deref(), &name, store.clone());
                    Some(store.upcast())
                }
                SchemaKind::Column { .. }
                | SchemaKind::Loading { .. }
                | SchemaKind::Error { .. } => None,
            }
        });

        let selection = gtk::NoSelection::new(Some(tree_model));
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let list_item = item
                .downcast_ref::<gtk::ListItem>()
                .expect("ListItem in schema factory setup");
            let row = build_row_widget();
            list_item.set_child(Some(&row.root));
            unsafe {
                list_item.set_data(ROW_DATA_KEY, row);
            }
        });
        factory.connect_bind(|_, item| {
            let list_item = item
                .downcast_ref::<gtk::ListItem>()
                .expect("ListItem in schema factory bind");
            let Some(tree_row) = list_item.item().and_downcast::<gtk::TreeListRow>() else {
                return;
            };
            let row = unsafe {
                list_item
                    .data::<RowWidgets>(ROW_DATA_KEY)
                    .map(|p| p.as_ref().clone())
            };
            let Some(row) = row else {
                return;
            };
            row.root.set_list_row(Some(&tree_row));
            let Some(schema_item) = tree_row.item().and_downcast::<SchemaItem>() else {
                return;
            };
            bind_row(&row, &schema_item);
        });
        factory.connect_unbind(|_, item| {
            let list_item = item
                .downcast_ref::<gtk::ListItem>()
                .expect("ListItem in schema factory unbind");
            let row = unsafe {
                list_item
                    .data::<RowWidgets>(ROW_DATA_KEY)
                    .map(|p| p.as_ref().clone())
            };
            if let Some(row) = row {
                row.root.set_list_row(None::<&gtk::TreeListRow>);
            }
        });

        list_view.set_factory(Some(&factory));
        list_view.set_model(Some(&selection));

        Self {
            root,
            connection_id: RefCell::new(None),
        }
    }

    pub fn connection_id(&self) -> Option<String> {
        self.connection_id.borrow().clone()
    }

    pub fn clear(&self) {
        *self.connection_id.borrow_mut() = None;
        self.root.remove_all();
    }

    /// Mark `connection_id` as active and empty the root while a fetch is in flight.
    pub fn prepare_load(&self, connection_id: &str) {
        *self.connection_id.borrow_mut() = Some(connection_id.to_string());
        self.root.remove_all();
    }

    /// Replace the root with schemas for `connection_id`.
    pub fn set_schemas(&self, connection_id: &str, schemas: Vec<SchemaInfo>) {
        *self.connection_id.borrow_mut() = Some(connection_id.to_string());
        self.root.remove_all();
        for s in schemas {
            self.root.append(&SchemaItem::schema(s.name, s.is_default));
        }
    }
}

#[derive(Clone)]
struct RowWidgets {
    root: gtk::TreeExpander,
    icon: gtk::Label,
    name: gtk::Label,
    detail: gtk::Label,
    badges: gtk::Box,
}

fn build_row_widget() -> RowWidgets {
    let expander = gtk::TreeExpander::new();
    expander.add_css_class("schema-row");

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.set_hexpand(true);

    let icon = gtk::Label::new(None);
    icon.add_css_class("schema-icon");
    icon.set_width_chars(1);

    let text = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    text.set_hexpand(true);
    let name = gtk::Label::builder()
        .xalign(0.0)
        .ellipsize(pango::EllipsizeMode::End)
        .hexpand(true)
        .css_classes(["schema-name"])
        .build();
    let detail = gtk::Label::builder()
        .xalign(1.0)
        .ellipsize(pango::EllipsizeMode::End)
        .css_classes(["dimmed", "caption", "schema-detail"])
        .build();
    text.append(&name);

    let badges = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    badges.add_css_class("schema-badges");

    row.append(&icon);
    row.append(&text);
    row.append(&badges);
    row.append(&detail);
    expander.set_child(Some(&row));

    RowWidgets {
        root: expander,
        icon,
        name,
        detail,
        badges,
    }
}

fn clear_children(box_: &gtk::Box) {
    while let Some(child) = box_.first_child() {
        box_.remove(&child);
    }
}

fn bind_row(row: &RowWidgets, item: &SchemaItem) {
    clear_children(&row.badges);
    row.root.remove_css_class("schema-placeholder");
    row.name.remove_css_class("dimmed");
    row.detail.set_visible(true);

    match item.kind() {
        SchemaKind::Schema { name, is_default } => {
            row.icon.set_text("▣");
            row.name.set_text(&name);
            if is_default {
                row.detail.set_text("default");
                row.detail.set_visible(true);
            } else {
                row.detail.set_text("");
                row.detail.set_visible(false);
            }
            row.root.set_tooltip_text(Some(&format!("Schema {name}")));
        }
        SchemaKind::Table {
            name,
            table_type,
            full_name,
            ..
        } => {
            let is_view = table_type.eq_ignore_ascii_case("view");
            row.icon.set_text(if is_view { "◫" } else { "⊞" });
            row.name.set_text(&name);
            row.detail.set_text(if is_view { "view" } else { "table" });
            row.root.set_tooltip_text(Some(&full_name));
        }
        SchemaKind::Column {
            name,
            data_type,
            nullable,
            is_primary_key,
            is_foreign_key,
            foreign_table,
            foreign_column,
        } => {
            row.icon.set_text(if is_primary_key {
                "🔑"
            } else if is_foreign_key {
                "🔗"
            } else {
                "○"
            });
            row.name.set_text(&name);
            let mut type_label = data_type;
            if nullable {
                type_label.push('?');
            }
            row.detail.set_text(&type_label);

            if is_primary_key {
                let pk = gtk::Label::new(Some("PK"));
                pk.add_css_class("schema-badge");
                pk.add_css_class("schema-badge-pk");
                row.badges.append(&pk);
            }
            if is_foreign_key {
                let fk = gtk::Label::new(Some("FK"));
                fk.add_css_class("schema-badge");
                fk.add_css_class("schema-badge-fk");
                row.badges.append(&fk);
            }

            let tip = if is_foreign_key {
                match (foreign_table.as_deref(), foreign_column.as_deref()) {
                    (Some(t), Some(c)) => format!("{name} → {t}.{c}"),
                    (Some(t), None) => format!("{name} → {t}"),
                    _ => format!("{name} (foreign key)"),
                }
            } else if is_primary_key {
                format!("{name} (primary key)")
            } else {
                name
            };
            row.root.set_tooltip_text(Some(&tip));
        }
        SchemaKind::Loading { label } => {
            row.icon.set_text("…");
            row.name.set_text(&label);
            row.name.add_css_class("dimmed");
            row.detail.set_visible(false);
            row.root.add_css_class("schema-placeholder");
            row.root.set_tooltip_text(None);
        }
        SchemaKind::Error { message } => {
            row.icon.set_text("!");
            row.name.set_text(&message);
            row.name.add_css_class("dimmed");
            row.detail.set_visible(false);
            row.root.add_css_class("schema-placeholder");
            row.root.set_tooltip_text(Some(&message));
        }
    }
}

/// Shared helpers used by `SqlatorWindow` async loaders.
pub fn replace_store_with_tables(store: &gio::ListStore, tables: Vec<TableInfo>) {
    store.remove_all();
    if tables.is_empty() {
        store.append(&SchemaItem::error("No tables found"));
        return;
    }
    for t in tables {
        store.append(&SchemaItem::table(
            t.name,
            t.schema,
            t.table_type,
            t.full_name,
        ));
    }
}

pub fn replace_store_with_columns(store: &gio::ListStore, columns: Vec<SchemaColumnInfo>) {
    store.remove_all();
    if columns.is_empty() {
        store.append(&SchemaItem::error("No columns found"));
        return;
    }
    for c in columns {
        store.append(&SchemaItem::column(
            c.name,
            c.data_type,
            c.nullable,
            c.is_primary_key,
            c.is_foreign_key,
            c.foreign_table,
            c.foreign_column,
        ));
    }
}

pub fn replace_store_with_error(store: &gio::ListStore, message: impl Into<String>) {
    store.remove_all();
    store.append(&SchemaItem::error(message));
}
