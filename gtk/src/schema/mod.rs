//! Lazy schema browser: schemas → tables/views → columns.
//!
//! Also hosts schema-opened tabs: table browse + DDL viewer.

mod browse;
mod ddl;
mod item;
mod tree;

pub use browse::TableBrowseTab;
pub use ddl::SchemaDdlTab;
pub use tree::{
    replace_store_with_columns, replace_store_with_error, replace_store_with_tables, SchemaTree,
};
