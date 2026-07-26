//! Client-side DELETE → UPDATE → INSERT batch generation (Svelte `sql-generator.ts` parity).

use sqlator_core::models::{ParameterizedStatement, PrimaryKeyMeta, SqlBatch, TableMeta};

use super::edit_state::{AddedRow, ChangeSet, ModifiedRow, PkValue};

fn quote_pg(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn quote_mysql(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

fn pk_to_array(pk: &PkValue) -> Vec<serde_json::Value> {
    match pk {
        PkValue::Single(v) => vec![v.clone()],
        PkValue::Composite(vs) => vs.clone(),
    }
}

fn pg_where_clause(pk: &PrimaryKeyMeta, pk_value: &PkValue, start_idx: usize) -> String {
    let _ = pk_to_array(pk_value); // length must match pk.columns (caller responsibility)
    pk.columns
        .iter()
        .enumerate()
        .map(|(i, col)| format!("{} = ${}", quote_pg(col), start_idx + i))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn pg_insert(table: &TableMeta, temp_id: &str, row: &AddedRow) -> ParameterizedStatement {
    let updatable_cols: Vec<&String> = row
        .data
        .keys()
        .filter(|col| {
            table
                .columns
                .iter()
                .find(|c| c.name == **col)
                .map(|c| c.is_updatable)
                .unwrap_or(true)
        })
        .collect();
    // Stable order for deterministic SQL / tests.
    let mut updatable_cols: Vec<&String> = updatable_cols.into_iter().collect();
    updatable_cols.sort();

    let params: Vec<serde_json::Value> = updatable_cols
        .iter()
        .map(|col| {
            row.data
                .get(*col)
                .cloned()
                .unwrap_or(serde_json::Value::Null)
        })
        .collect();
    let col_list = updatable_cols
        .iter()
        .map(|c| quote_pg(c))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = (1..=params.len())
        .map(|i| format!("${i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let returning = table
        .primary_key
        .columns
        .iter()
        .map(|c| quote_pg(c))
        .collect::<Vec<_>>()
        .join(", ");

    ParameterizedStatement {
        sql: format!(
            "INSERT INTO {} ({col_list}) VALUES ({placeholders}) RETURNING {returning}",
            quote_pg(&table.table_name)
        ),
        params,
        temp_id: Some(temp_id.to_string()),
    }
}

fn pg_update(table: &TableMeta, modified: &ModifiedRow) -> ParameterizedStatement {
    let mut set_clauses = Vec::new();
    let mut params = Vec::new();
    let mut idx = 1usize;

    let mut changes: Vec<_> = modified.changes.iter().collect();
    changes.sort_by(|a, b| a.0.cmp(b.0));
    for (col, change) in changes {
        set_clauses.push(format!("{} = ${idx}", quote_pg(col)));
        params.push(change.new_value.clone());
        idx += 1;
    }

    let where_clause = pg_where_clause(&table.primary_key, &modified.primary_key, idx);
    params.extend(pk_to_array(&modified.primary_key));

    ParameterizedStatement {
        sql: format!(
            "UPDATE {} SET {} WHERE {where_clause}",
            quote_pg(&table.table_name),
            set_clauses.join(", ")
        ),
        params,
        temp_id: None,
    }
}

fn pg_delete(table: &TableMeta, pk_value: &PkValue) -> ParameterizedStatement {
    let where_clause = pg_where_clause(&table.primary_key, pk_value, 1);
    ParameterizedStatement {
        sql: format!(
            "DELETE FROM {} WHERE {where_clause}",
            quote_pg(&table.table_name)
        ),
        params: pk_to_array(pk_value),
        temp_id: None,
    }
}

fn my_where_clause(pk: &PrimaryKeyMeta, quote: fn(&str) -> String) -> String {
    pk.columns
        .iter()
        .map(|col| format!("{} = ?", quote(col)))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn my_insert(
    table: &TableMeta,
    temp_id: &str,
    row: &AddedRow,
    quote: fn(&str) -> String,
) -> ParameterizedStatement {
    let mut updatable_cols: Vec<&String> = row
        .data
        .keys()
        .filter(|col| {
            table
                .columns
                .iter()
                .find(|c| c.name == **col)
                .map(|c| c.is_updatable)
                .unwrap_or(true)
        })
        .collect();
    updatable_cols.sort();

    let params: Vec<serde_json::Value> = updatable_cols
        .iter()
        .map(|col| {
            row.data
                .get(*col)
                .cloned()
                .unwrap_or(serde_json::Value::Null)
        })
        .collect();
    let col_list = updatable_cols
        .iter()
        .map(|c| quote(c))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = params.iter().map(|_| "?").collect::<Vec<_>>().join(", ");

    ParameterizedStatement {
        sql: format!(
            "INSERT INTO {} ({col_list}) VALUES ({placeholders})",
            quote(&table.table_name)
        ),
        params,
        temp_id: Some(temp_id.to_string()),
    }
}

fn my_update(
    table: &TableMeta,
    modified: &ModifiedRow,
    quote: fn(&str) -> String,
) -> ParameterizedStatement {
    let mut set_clauses = Vec::new();
    let mut params = Vec::new();

    let mut changes: Vec<_> = modified.changes.iter().collect();
    changes.sort_by(|a, b| a.0.cmp(b.0));
    for (col, change) in changes {
        set_clauses.push(format!("{} = ?", quote(col)));
        params.push(change.new_value.clone());
    }

    let where_clause = my_where_clause(&table.primary_key, quote);
    params.extend(pk_to_array(&modified.primary_key));

    ParameterizedStatement {
        sql: format!(
            "UPDATE {} SET {} WHERE {where_clause}",
            quote(&table.table_name),
            set_clauses.join(", ")
        ),
        params,
        temp_id: None,
    }
}

fn my_delete(
    table: &TableMeta,
    pk_value: &PkValue,
    quote: fn(&str) -> String,
) -> ParameterizedStatement {
    let where_clause = my_where_clause(&table.primary_key, quote);
    ParameterizedStatement {
        sql: format!(
            "DELETE FROM {} WHERE {where_clause}",
            quote(&table.table_name)
        ),
        params: pk_to_array(pk_value),
        temp_id: None,
    }
}

/// Build a transactional batch ordered DELETE → UPDATE → INSERT.
pub fn generate_batch(change_set: &ChangeSet, table_meta: &TableMeta, db_type: &str) -> SqlBatch {
    let mut statements = Vec::new();
    let is_postgres = db_type == "postgres";
    let quote: fn(&str) -> String = if db_type == "mysql" || db_type == "mariadb" {
        quote_mysql
    } else {
        quote_pg
    };

    let mut deleted: Vec<_> = change_set.deleted.iter().collect();
    deleted.sort();
    for pk_key in deleted {
        let Ok(pk_value) = PkValue::from_key(pk_key) else {
            continue;
        };
        statements.push(if is_postgres {
            pg_delete(table_meta, &pk_value)
        } else {
            my_delete(table_meta, &pk_value, quote)
        });
    }

    let mut modified: Vec<_> = change_set.modified.iter().collect();
    modified.sort_by(|a, b| a.0.cmp(b.0));
    for (_, modified_row) in modified {
        statements.push(if is_postgres {
            pg_update(table_meta, modified_row)
        } else {
            my_update(table_meta, modified_row, quote)
        });
    }

    // Preserve insertion order via BTreeMap-friendly temp ids (`temp_1`, …).
    let mut added: Vec<_> = change_set.added.iter().collect();
    added.sort_by(|a, b| a.0.cmp(b.0));
    for (temp_id, added_row) in added {
        statements.push(if is_postgres {
            pg_insert(table_meta, temp_id, added_row)
        } else {
            my_insert(table_meta, temp_id, added_row, quote)
        });
    }

    SqlBatch {
        statements,
        use_transaction: true,
    }
}

/// Human-readable preview with parameter annotations.
pub fn format_batch_for_preview(batch: &SqlBatch) -> String {
    batch
        .statements
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let params = s
                .params
                .iter()
                .map(|p| {
                    if p.is_null() {
                        "NULL".to_string()
                    } else {
                        p.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("-- Statement {}\n{}\n-- Params: [{params}]", i + 1, s.sql)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::results::edit_state::EditState;
    use serde_json::json;
    use sqlator_core::models::{ColumnMeta, PrimaryKeyMeta};
    use std::collections::HashMap;

    fn sample_meta() -> TableMeta {
        TableMeta {
            table_name: "users".into(),
            schema: Some("public".into()),
            columns: vec![
                ColumnMeta {
                    name: "id".into(),
                    column_type: "integer".into(),
                    nullable: false,
                    is_auto_increment: true,
                    is_generated: false,
                    is_updatable: false,
                    default_value: None,
                },
                ColumnMeta {
                    name: "name".into(),
                    column_type: "text".into(),
                    nullable: true,
                    is_auto_increment: false,
                    is_generated: false,
                    is_updatable: true,
                    default_value: None,
                },
                ColumnMeta {
                    name: "age".into(),
                    column_type: "integer".into(),
                    nullable: true,
                    is_auto_increment: false,
                    is_generated: false,
                    is_updatable: true,
                    default_value: None,
                },
            ],
            primary_key: PrimaryKeyMeta {
                columns: vec!["id".into()],
                exists: true,
            },
            is_editable: true,
            editability_reason: None,
        }
    }

    #[test]
    fn order_is_delete_update_insert_postgres() {
        let meta = sample_meta();
        let mut state = EditState::default();
        state.set_table_meta(Some(meta.clone()));
        state.set_db_type("postgres");

        let mut row = HashMap::new();
        row.insert("id".into(), json!(1));
        row.insert("name".into(), json!("alice"));
        state.delete_row(&row);

        let mut row2 = HashMap::new();
        row2.insert("id".into(), json!(2));
        row2.insert("name".into(), json!("bob"));
        state.modify_cell(&row2, "name", json!("bobby"));

        let temp = state.add_row();
        state.modify_added_cell(&temp, "name", json!("carol"));

        let batch = state.generate_batch().expect("batch");
        assert_eq!(batch.statements.len(), 3);
        assert!(batch.statements[0].sql.starts_with("DELETE FROM"));
        assert!(batch.statements[1].sql.starts_with("UPDATE "));
        assert!(batch.statements[2].sql.starts_with("INSERT INTO"));
        assert!(batch.statements[2].sql.contains("RETURNING"));
        assert_eq!(batch.statements[2].temp_id.as_deref(), Some(temp.as_str()));
        assert!(batch.use_transaction);
    }

    #[test]
    fn mysql_uses_question_placeholders() {
        let meta = sample_meta();
        let mut cs = ChangeSet::default();
        let pk = PkValue::Single(json!(9));
        cs.deleted.insert(pk.to_key());

        let batch = generate_batch(&cs, &meta, "mysql");
        assert_eq!(batch.statements.len(), 1);
        assert!(batch.statements[0].sql.contains("`users`"));
        assert!(batch.statements[0].sql.contains("= ?"));
        assert!(!batch.statements[0].sql.contains("$1"));
    }

    #[test]
    fn preview_annotates_params() {
        let meta = sample_meta();
        let mut cs = ChangeSet::default();
        let pk = PkValue::Single(json!(1));
        cs.deleted.insert(pk.to_key());
        let batch = generate_batch(&cs, &meta, "postgres");
        let preview = format_batch_for_preview(&batch);
        assert!(preview.contains("-- Statement 1"));
        assert!(preview.contains("-- Params: [1]"));
    }

    #[test]
    fn composite_pk_where_clause() {
        let mut meta = sample_meta();
        meta.primary_key.columns = vec!["a".into(), "b".into()];
        let mut cs = ChangeSet::default();
        let pk = PkValue::Composite(vec![json!(1), json!("x")]);
        cs.deleted.insert(pk.to_key());
        let batch = generate_batch(&cs, &meta, "postgres");
        assert!(batch.statements[0].sql.contains("\"a\" = $1"));
        assert!(batch.statements[0].sql.contains("\"b\" = $2"));
        assert_eq!(batch.statements[0].params, vec![json!(1), json!("x")]);
    }
}
