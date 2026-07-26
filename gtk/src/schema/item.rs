//! Schema browser rows — schema / table / column / placeholder (phase-0 GObject pattern).

use gio::subclass::prelude::*;
use glib::Object;
use std::cell::RefCell;

#[derive(Debug, Clone)]
pub enum SchemaKind {
    Schema {
        name: String,
        is_default: bool,
    },
    Table {
        name: String,
        schema: Option<String>,
        table_type: String,
        full_name: String,
    },
    Column {
        name: String,
        data_type: String,
        nullable: bool,
        is_primary_key: bool,
        is_foreign_key: bool,
        foreign_table: Option<String>,
        foreign_column: Option<String>,
    },
    /// Temporary child while an async expand fetch is in flight.
    Loading {
        label: String,
    },
    /// Expand fetch failed; shown as a non-expandable child.
    Error {
        message: String,
    },
}

impl SchemaKind {
    pub fn is_expandable(&self) -> bool {
        matches!(self, Self::Schema { .. } | Self::Table { .. })
    }

    pub fn is_leaf(&self) -> bool {
        !self.is_expandable()
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SchemaItem {
        pub kind: RefCell<Option<SchemaKind>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SchemaItem {
        const NAME: &'static str = "SqlatorSchemaItem";
        type Type = super::SchemaItem;
    }

    impl ObjectImpl for SchemaItem {}
}

glib::wrapper! {
    pub struct SchemaItem(ObjectSubclass<imp::SchemaItem>);
}

impl SchemaItem {
    pub fn new(kind: SchemaKind) -> Self {
        let obj: Self = Object::builder().build();
        *obj.imp().kind.borrow_mut() = Some(kind);
        obj
    }

    pub fn schema(name: impl Into<String>, is_default: bool) -> Self {
        Self::new(SchemaKind::Schema {
            name: name.into(),
            is_default,
        })
    }

    pub fn table(
        name: impl Into<String>,
        schema: Option<String>,
        table_type: impl Into<String>,
        full_name: impl Into<String>,
    ) -> Self {
        Self::new(SchemaKind::Table {
            name: name.into(),
            schema,
            table_type: table_type.into(),
            full_name: full_name.into(),
        })
    }

    pub fn column(
        name: impl Into<String>,
        data_type: impl Into<String>,
        nullable: bool,
        is_primary_key: bool,
        is_foreign_key: bool,
        foreign_table: Option<String>,
        foreign_column: Option<String>,
    ) -> Self {
        Self::new(SchemaKind::Column {
            name: name.into(),
            data_type: data_type.into(),
            nullable,
            is_primary_key,
            is_foreign_key,
            foreign_table,
            foreign_column,
        })
    }

    pub fn loading(label: impl Into<String>) -> Self {
        Self::new(SchemaKind::Loading {
            label: label.into(),
        })
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::new(SchemaKind::Error {
            message: message.into(),
        })
    }

    pub fn kind(&self) -> SchemaKind {
        self.imp()
            .kind
            .borrow()
            .clone()
            .expect("SchemaItem kind set at construction")
    }
}
