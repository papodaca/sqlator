//! Per-tab editable-results change tracking (Svelte `edit.svelte.ts` parity).
//!
//! Owned by each [`crate::query_tab::QueryTab`] — never a process singleton.

use sqlator_core::models::{SqlBatch, TableMeta};
use std::collections::{HashMap, HashSet};

use super::sql_gen;

/// Primary-key value: single column or composite.
#[derive(Debug, Clone, PartialEq)]
pub enum PkValue {
    Single(serde_json::Value),
    Composite(Vec<serde_json::Value>),
}

impl PkValue {
    pub fn to_key(&self) -> String {
        match self {
            Self::Single(v) => serde_json::to_string(v).unwrap_or_else(|_| "null".into()),
            Self::Composite(vs) => serde_json::to_string(vs).unwrap_or_else(|_| "[]".into()),
        }
    }

    pub fn from_key(key: &str) -> Result<Self, serde_json::Error> {
        let value: serde_json::Value = serde_json::from_str(key)?;
        Ok(match value {
            serde_json::Value::Array(items) => Self::Composite(items),
            other => Self::Single(other),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CellChange {
    pub old_value: serde_json::Value,
    pub new_value: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct AddedRow {
    pub temp_id: String,
    pub data: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct ModifiedRow {
    pub primary_key: PkValue,
    pub changes: HashMap<String, CellChange>,
}

#[derive(Debug, Clone, Default)]
pub struct ChangeSet {
    pub added: HashMap<String, AddedRow>,
    pub modified: HashMap<String, ModifiedRow>,
    pub deleted: HashSet<String>,
}

#[derive(Debug, Default)]
pub struct EditState {
    change_set: ChangeSet,
    table_meta: Option<TableMeta>,
    connection_id: Option<String>,
    db_type: String,
    last_sql: String,
    temp_id_counter: u64,
    /// Model row index → temp id for pending inserts displayed in the grid.
    added_model_rows: HashMap<u32, String>,
}

impl EditState {
    pub fn reset(&mut self, connection_id: &str, db_type: &str, sql: &str) {
        self.change_set = ChangeSet::default();
        self.table_meta = None;
        self.connection_id = Some(connection_id.to_string());
        self.db_type = db_type.to_string();
        self.last_sql = sql.to_string();
        self.temp_id_counter = 0;
        self.added_model_rows.clear();
    }

    pub fn clear_all(&mut self) {
        self.change_set = ChangeSet::default();
        self.table_meta = None;
        self.added_model_rows.clear();
        self.temp_id_counter = 0;
    }

    pub fn set_table_meta(&mut self, meta: Option<TableMeta>) {
        self.table_meta = meta;
    }

    pub fn set_db_type(&mut self, db_type: &str) {
        self.db_type = db_type.to_string();
    }

    pub fn table_meta(&self) -> Option<&TableMeta> {
        self.table_meta.as_ref()
    }

    pub fn connection_id(&self) -> Option<&str> {
        self.connection_id.as_deref()
    }

    pub fn db_type(&self) -> &str {
        &self.db_type
    }

    pub fn last_sql(&self) -> &str {
        &self.last_sql
    }

    pub fn change_set(&self) -> &ChangeSet {
        &self.change_set
    }

    pub fn is_editable(&self) -> bool {
        self.table_meta
            .as_ref()
            .map(|m| m.is_editable)
            .unwrap_or(false)
    }

    pub fn editability_reason(&self) -> Option<&str> {
        self.table_meta.as_ref().and_then(|m| {
            if m.is_editable {
                None
            } else {
                m.editability_reason.as_deref().or(Some("Not editable"))
            }
        })
    }

    pub fn has_changes(&self) -> bool {
        !self.change_set.added.is_empty()
            || !self.change_set.modified.is_empty()
            || !self.change_set.deleted.is_empty()
    }

    pub fn change_count(&self) -> usize {
        self.change_set.added.len() + self.change_set.modified.len() + self.change_set.deleted.len()
    }

    pub fn has_table_meta(&self) -> bool {
        self.table_meta.is_some()
    }

    fn extract_pk(
        row: &HashMap<String, serde_json::Value>,
        pk_columns: &[String],
    ) -> Option<PkValue> {
        if pk_columns.len() == 1 {
            let col = &pk_columns[0];
            return Some(PkValue::Single(
                row.get(col).cloned().unwrap_or(serde_json::Value::Null),
            ));
        }
        let values: Vec<_> = pk_columns
            .iter()
            .map(|c| row.get(c).cloned().unwrap_or(serde_json::Value::Null))
            .collect();
        Some(PkValue::Composite(values))
    }

    pub fn modify_cell(
        &mut self,
        row: &HashMap<String, serde_json::Value>,
        column_name: &str,
        new_value: serde_json::Value,
    ) {
        let Some(meta) = self.table_meta.as_ref() else {
            return;
        };
        if !meta.primary_key.exists {
            return;
        }
        let Some(pk_value) = Self::extract_pk(row, &meta.primary_key.columns) else {
            return;
        };
        let pk_key = pk_value.to_key();
        let old_value = row
            .get(column_name)
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        let mut modified_row = self
            .change_set
            .modified
            .remove(&pk_key)
            .unwrap_or(ModifiedRow {
                primary_key: pk_value,
                changes: HashMap::new(),
            });

        if new_value == old_value {
            modified_row.changes.remove(column_name);
            if modified_row.changes.is_empty() {
                return;
            }
        } else {
            modified_row.changes.insert(
                column_name.to_string(),
                CellChange {
                    old_value,
                    new_value,
                },
            );
        }

        self.change_set.modified.insert(pk_key, modified_row);
    }

    pub fn add_row(&mut self) -> String {
        self.temp_id_counter += 1;
        let temp_id = format!("temp_{}", self.temp_id_counter);
        self.change_set.added.insert(
            temp_id.clone(),
            AddedRow {
                temp_id: temp_id.clone(),
                data: HashMap::new(),
            },
        );
        temp_id
    }

    pub fn register_added_model_row(&mut self, model_index: u32, temp_id: String) {
        self.added_model_rows.insert(model_index, temp_id);
    }

    pub fn added_temp_id_for_model_row(&self, model_index: u32) -> Option<&str> {
        self.added_model_rows.get(&model_index).map(|s| s.as_str())
    }

    pub fn take_added_model_rows(&mut self) -> HashMap<u32, String> {
        std::mem::take(&mut self.added_model_rows)
    }

    pub fn modify_added_cell(
        &mut self,
        temp_id: &str,
        column_name: &str,
        new_value: serde_json::Value,
    ) {
        let Some(row) = self.change_set.added.get_mut(temp_id) else {
            return;
        };
        row.data.insert(column_name.to_string(), new_value);
    }

    pub fn delete_row(&mut self, row: &HashMap<String, serde_json::Value>) {
        let Some(meta) = self.table_meta.as_ref() else {
            return;
        };
        if !meta.primary_key.exists {
            return;
        }
        let Some(pk_value) = Self::extract_pk(row, &meta.primary_key.columns) else {
            return;
        };
        let pk_key = pk_value.to_key();
        self.change_set.modified.remove(&pk_key);
        self.change_set.deleted.insert(pk_key);
    }

    pub fn delete_added_row(&mut self, temp_id: &str) {
        self.change_set.added.remove(temp_id);
        self.added_model_rows.retain(|_, id| id != temp_id);
    }

    /// After a model row is removed, shift pending-insert index keys down.
    pub fn shift_added_indices_after_remove(&mut self, removed_index: u32) {
        let old = std::mem::take(&mut self.added_model_rows);
        self.added_model_rows = old
            .into_iter()
            .filter_map(|(idx, id)| {
                if idx == removed_index {
                    None
                } else if idx > removed_index {
                    Some((idx - 1, id))
                } else {
                    Some((idx, id))
                }
            })
            .collect();
    }

    pub fn undo_delete_row(&mut self, row_key: &str) {
        self.change_set.deleted.remove(row_key);
    }

    pub fn discard_all_changes(&mut self) {
        self.change_set = ChangeSet::default();
        self.added_model_rows.clear();
    }

    pub fn is_cell_modified(
        &self,
        row: &HashMap<String, serde_json::Value>,
        column_name: &str,
    ) -> bool {
        let Some(meta) = self.table_meta.as_ref() else {
            return false;
        };
        if !meta.primary_key.exists {
            return false;
        }
        let Some(pk_value) = Self::extract_pk(row, &meta.primary_key.columns) else {
            return false;
        };
        self.change_set
            .modified
            .get(&pk_value.to_key())
            .is_some_and(|m| m.changes.contains_key(column_name))
    }

    pub fn is_row_deleted(&self, row: &HashMap<String, serde_json::Value>) -> bool {
        let Some(meta) = self.table_meta.as_ref() else {
            return false;
        };
        if !meta.primary_key.exists {
            return false;
        }
        let Some(pk_value) = Self::extract_pk(row, &meta.primary_key.columns) else {
            return false;
        };
        self.change_set.deleted.contains(&pk_value.to_key())
    }

    pub fn get_cell_display_value(
        &self,
        row: &HashMap<String, serde_json::Value>,
        column_name: &str,
    ) -> serde_json::Value {
        if let Some(meta) = self.table_meta.as_ref() {
            if meta.primary_key.exists {
                if let Some(pk_value) = Self::extract_pk(row, &meta.primary_key.columns) {
                    if let Some(change) = self
                        .change_set
                        .modified
                        .get(&pk_value.to_key())
                        .and_then(|m| m.changes.get(column_name))
                    {
                        return change.new_value.clone();
                    }
                }
            }
        }
        row.get(column_name)
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    }

    pub fn generate_batch(&self) -> Option<SqlBatch> {
        let meta = self.table_meta.as_ref()?;
        if !self.has_changes() {
            return None;
        }
        Some(sql_gen::generate_batch(
            &self.change_set,
            meta,
            &self.db_type,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_added_indices_after_remove_renumbers() {
        let mut state = EditState::default();
        state.register_added_model_row(1, "t1".into());
        state.register_added_model_row(3, "t3".into());
        state.register_added_model_row(4, "t4".into());
        state.shift_added_indices_after_remove(3);
        assert_eq!(state.added_temp_id_for_model_row(1), Some("t1"));
        assert_eq!(state.added_temp_id_for_model_row(3), Some("t4"));
        assert!(state.added_temp_id_for_model_row(4).is_none());
    }
}
