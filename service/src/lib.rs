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
pub use portability::{
    build_export_json, parse_export_json, ExportFile, ExportedConnection, ExportedGroup,
    ExportedJumpHost, ExportedSshProfile, ImportResult,
};
pub use schema::{
    extract_single_table, extract_table_regex, non_editable_meta, schema_cache_key, TableExtract,
    SCHEMA_CACHE_TTL_SECS,
};
pub use service::AppService;
pub use ssh::{build_auth_config_for_profile, build_jump_hosts_for_profile, parse_auth_method};
