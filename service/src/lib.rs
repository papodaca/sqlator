//! Shared application layer for SQLator frontends.
//!
//! Sits between `sqlator-core` and Tauri / web / TUI / GTK adapters. See
//! `docs/plans/gtk/2026-07-25-002-refactor-sqlator-service-extraction-plan.md`.

pub mod connect;
pub mod connection_source;
pub mod connections;
pub mod credentials;
pub mod docker;
pub mod error;
pub mod portability;
pub mod schema;
pub mod service;
pub mod ssh;
pub mod terminal_spec;

pub use connection_source::ConnectionSource;
pub use connections::{
    build_saved_connection, build_url_no_password, db_type_from_url, default_port_for_db_type,
    parse_connection_url, resolve_connection_type, unique_name, ParsedConnectionUrl,
};
pub use error::ServiceError;
pub use service::AppService;
pub use ssh::parse_auth_method;
