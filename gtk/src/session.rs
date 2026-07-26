//! Session tab-state persistence (`save_tab_state` / `get_tab_state`).
//!
//! Shape matches the Svelte frontend so both UIs share the same blob in
//! `connections.json`.

use gtk::glib;
use serde::{Deserialize, Serialize};
use sqlator_core::models::{FilterSpec, SortSpec};

pub const SAVE_DEBOUNCE_MS: u32 = 500;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PersistedTabState {
    pub active_connection_id: Option<String>,
    #[serde(default)]
    pub connection_tabs: Vec<PersistedConnectionTab>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedConnectionTab {
    pub connection_id: String,
    pub active_query_tab_id: Option<String>,
    #[serde(default)]
    pub query_tabs: Vec<PersistedQueryTab>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedQueryTab {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub sql: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table_browse: Option<PersistedTableBrowse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_ddl: Option<PersistedSchemaDdl>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedTableBrowse {
    pub table_name: String,
    #[serde(default)]
    pub schema: Option<String>,
    #[serde(default)]
    pub sort: Vec<SortSpec>,
    #[serde(default)]
    pub filters: Vec<FilterSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedSchemaDdl {
    pub table_name: String,
    #[serde(default)]
    pub schema: Option<String>,
}

/// New stable id for a restored/created query page.
pub fn new_tab_id() -> String {
    format!("tab-{}", glib::uuid_string_random())
}
