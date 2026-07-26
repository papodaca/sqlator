//! Lazy schema browser: schemas → tables/views → columns.

mod item;
mod tree;

pub use tree::{
    replace_store_with_columns, replace_store_with_error, replace_store_with_tables, SchemaTree,
};
