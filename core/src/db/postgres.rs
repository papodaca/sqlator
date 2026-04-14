use crate::error::CoreError;
use crate::models::QueryEvent;
use sqlx::postgres::PgRow;
use sqlx::{Column, PgPool, Row, TypeInfo};
use std::time::Instant;

pub async fn execute_select(
    pool: &PgPool,
    sql: &str,
    sender: tokio::sync::mpsc::Sender<QueryEvent>,
    start: Instant,
) -> Result<(), CoreError> {
    use futures::StreamExt;

    let mut stream = sqlx::query(sql).fetch(pool);
    let mut row_count: usize = 0;
    let mut columns_sent = false;
    let max_rows: usize = 1000;

    while let Some(result) = stream.next().await {
        match result {
            Ok(row) => {
                if !columns_sent {
                    let names: Vec<String> = row
                        .columns()
                        .iter()
                        .map(|c| c.name().to_string())
                        .collect();
                    let _ = sender.send(QueryEvent::Columns { names }).await;
                    columns_sent = true;
                }

                if row_count < max_rows {
                    let values: Vec<serde_json::Value> = row
                        .columns()
                        .iter()
                        .enumerate()
                        .map(|(i, _)| pg_row_to_json(&row, i))
                        .collect();
                    let _ = sender.send(QueryEvent::Row { values }).await;
                }
                row_count += 1;
            }
            Err(e) => {
                let _ = sender.send(QueryEvent::Error { message: e.to_string() }).await;
                return Ok(());
            }
        }
    }

    let duration_ms = start.elapsed().as_millis() as u64;
    let _ = sender
        .send(QueryEvent::Done {
            row_count,
            duration_ms,
        })
        .await;
    Ok(())
}

pub async fn get_ddl(pool: &PgPool, table_name: &str, schema: Option<&str>) -> Result<String, CoreError> {
    let schema = schema.unwrap_or("public");
    let safe_table = table_name.replace('"', "\"\"");
    let safe_schema = schema.replace('"', "\"\"");
    let qualified = format!("\"{}\".\"{}\"", safe_schema, safe_table);

    let mut parts = Vec::new();

    let col_rows = sqlx::query(
        r#"SELECT column_name, data_type, is_nullable, column_default,
                  character_maximum_length, numeric_precision, numeric_scale,
                  udt_name
           FROM information_schema.columns
           WHERE table_schema = $1 AND table_name = $2
           ORDER BY ordinal_position"#,
    )
    .bind(schema)
    .bind(table_name)
    .fetch_all(pool)
    .await
    .map_err(|e| CoreError { message: e.to_string(), code: "DDL_QUERY".into() })?;

    if col_rows.is_empty() {
        return Err(CoreError {
            message: format!("Table {}.{} not found. It may have been dropped.", schema, table_name),
            code: "TABLE_NOT_FOUND".into(),
        });
    }

    for row in &col_rows {
        let col_name: String = row.get("column_name");
        let data_type: String = row.get("data_type");
        let is_nullable: &str = row.get::<&str, _>("is_nullable");
        let default_value: Option<String> = row.try_get("column_default").ok().flatten();
        let char_max_len: Option<i32> = row.try_get("character_maximum_length").ok().flatten();
        let num_precision: Option<i32> = row.try_get("numeric_precision").ok().flatten();
        let num_scale: Option<i32> = row.try_get("numeric_scale").ok().flatten();
        let udt_name: Option<String> = row.try_get("udt_name").ok().flatten();

        let full_type = resolve_pg_type(&data_type, &udt_name, char_max_len, num_precision, num_scale);

        let mut col_def = format!("  \"{}\" {}", col_name.replace('"', "\"\""), full_type);
        if is_nullable == "NO" {
            col_def.push_str(" NOT NULL");
        }
        if let Some(def) = &default_value {
            col_def.push_str(&format!(" DEFAULT {}", def));
        }
        parts.push(col_def);
    }

    let pk_rows = sqlx::query(
        r#"SELECT tc.constraint_name, kcu.column_name
           FROM information_schema.table_constraints tc
           JOIN information_schema.key_column_usage kcu
               ON tc.constraint_name = kcu.constraint_name
               AND tc.table_schema = kcu.table_schema
           WHERE tc.constraint_type = 'PRIMARY KEY'
               AND tc.table_schema = $1 AND tc.table_name = $2
           ORDER BY tc.constraint_name, kcu.ordinal_position"#,
    )
    .bind(schema)
    .bind(table_name)
    .fetch_all(pool)
    .await
    .map_err(|e| CoreError { message: e.to_string(), code: "DDL_QUERY".into() })?;

    let mut pk_by_constraint: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    for row in &pk_rows {
        let cname: String = row.get("constraint_name");
        let col: String = row.get("column_name");
        pk_by_constraint.entry(cname).or_default().push(col);
    }
    for (cname, cols) in &pk_by_constraint {
        let quoted_cols: Vec<String> = cols.iter().map(|c| format!("\"{}\"", c.replace('"', "\"\""))).collect();
        parts.push(format!("  CONSTRAINT \"{}\" PRIMARY KEY ({})", cname.replace('"', "\"\""), quoted_cols.join(", ")));
    }

    let fk_rows = sqlx::query(
        r#"SELECT con.conname AS constraint_name,
                  att2.attname AS column_name,
                  ns2.nspname AS ref_schema,
                  cl2.relname AS ref_table,
                  att.attname AS ref_column
           FROM pg_catalog.pg_constraint con
           JOIN pg_catalog.pg_class cl ON con.conrelid = cl.oid
           JOIN pg_catalog.pg_namespace ns ON cl.relnamespace = ns.oid
           JOIN pg_catalog.pg_class cl2 ON con.confrelid = cl2.oid
           JOIN pg_catalog.pg_namespace ns2 ON cl2.relnamespace = ns2.oid
           CROSS JOIN LATERAL unnest(con.conkey, con.confkey) WITH ORDINALITY AS cols(col, ref, ord)
           JOIN pg_catalog.pg_attribute att ON att.attrelid = con.confrelid AND att.attnum = cols.ref
           JOIN pg_catalog.pg_attribute att2 ON att2.attrelid = con.conrelid AND att2.attnum = cols.col
           WHERE con.contype = 'f'
               AND ns.nspname = $1 AND cl.relname = $2
           ORDER BY con.conname, cols.ord"#,
    )
    .bind(schema)
    .bind(table_name)
    .fetch_all(pool)
    .await
    .map_err(|e| CoreError { message: e.to_string(), code: "DDL_QUERY".into() })?;

    let mut fk_by_constraint: std::collections::BTreeMap<String, Vec<(String, String, String, String)>> = std::collections::BTreeMap::new();
    for row in &fk_rows {
        let cname: String = row.get("constraint_name");
        let col: String = row.get("column_name");
        let ref_schema: String = row.get("ref_schema");
        let ref_table: String = row.get("ref_table");
        let ref_col: String = row.get("ref_column");
        fk_by_constraint.entry(cname).or_default().push((col, ref_schema, ref_table, ref_col));
    }
    for (cname, refs) in &fk_by_constraint {
        let from_cols: Vec<String> = refs.iter().map(|(c, _, _, _)| format!("\"{}\"", c.replace('"', "\"\""))).collect();
        let first = refs.first().unwrap();
        let ref_qualified = format!("\"{}\".\"{}\"", first.1.replace('"', "\"\""), first.2.replace('"', "\"\""));
        let to_cols: Vec<String> = refs.iter().map(|(_, _, _, c)| format!("\"{}\"", c.replace('"', "\"\""))).collect();
        parts.push(format!("  CONSTRAINT \"{}\" FOREIGN KEY ({}) REFERENCES {} ({})",
            cname.replace('"', "\"\""), from_cols.join(", "), ref_qualified, to_cols.join(", ")));
    }

    let uq_rows = sqlx::query(
        r#"SELECT tc.constraint_name, kcu.column_name
           FROM information_schema.table_constraints tc
           JOIN information_schema.key_column_usage kcu
               ON tc.constraint_name = kcu.constraint_name
               AND tc.table_schema = kcu.table_schema
           WHERE tc.constraint_type = 'UNIQUE'
               AND tc.table_schema = $1 AND tc.table_name = $2
           ORDER BY tc.constraint_name, kcu.ordinal_position"#,
    )
    .bind(schema)
    .bind(table_name)
    .fetch_all(pool)
    .await
    .map_err(|e| CoreError { message: e.to_string(), code: "DDL_QUERY".into() })?;

    let mut uq_by_constraint: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    for row in &uq_rows {
        let cname: String = row.get("constraint_name");
        let col: String = row.get("column_name");
        uq_by_constraint.entry(cname).or_default().push(col);
    }
    for (cname, cols) in &uq_by_constraint {
        let quoted_cols: Vec<String> = cols.iter().map(|c| format!("\"{}\"", c.replace('"', "\"\""))).collect();
        parts.push(format!("  CONSTRAINT \"{}\" UNIQUE ({})", cname.replace('"', "\"\""), quoted_cols.join(", ")));
    }

    let ck_rows = sqlx::query(
        r#"SELECT tc.constraint_name, cc.check_clause
           FROM information_schema.table_constraints tc
           JOIN information_schema.check_constraints cc
               ON tc.constraint_name = cc.constraint_name
               AND tc.constraint_schema = cc.constraint_schema
           WHERE tc.constraint_type = 'CHECK'
               AND tc.table_schema = $1 AND tc.table_name = $2
           ORDER BY tc.constraint_name"#,
    )
    .bind(schema)
    .bind(table_name)
    .fetch_all(pool)
    .await
    .map_err(|e| CoreError { message: e.to_string(), code: "DDL_QUERY".into() })?;

    for row in &ck_rows {
        let cname: String = row.get("constraint_name");
        let check_clause: String = row.get("check_clause");
        parts.push(format!("  CONSTRAINT \"{}\" CHECK ({})", cname.replace('"', "\"\""), check_clause));
    }

    Ok(format!("CREATE TABLE {} (\n{}\n);", qualified, parts.join(",\n")))
}

fn resolve_pg_type(
    data_type: &str,
    udt_name: &Option<String>,
    char_max_len: Option<i32>,
    num_precision: Option<i32>,
    num_scale: Option<i32>,
) -> String {
    let lower = data_type.to_lowercase();
    match lower.as_str() {
        "character varying" | "varchar" => {
            if let Some(len) = char_max_len {
                format!("character varying({})", len)
            } else {
                "character varying".to_string()
            }
        }
        "character" | "char" | "bpchar" => {
            if let Some(len) = char_max_len {
                format!("character({})", len)
            } else {
                "character".to_string()
            }
        }
        "numeric" | "decimal" => match (num_precision, num_scale) {
            (Some(p), Some(s)) => format!("numeric({}, {})", p, s),
            (Some(p), None) => format!("numeric({})", p),
            _ => "numeric".to_string(),
        },
        "user-defined" => {
            if let Some(udt) = udt_name {
                format!("\"{}\"", udt.replace('"', "\"\""))
            } else {
                "unknown".to_string()
            }
        }
        "array" => {
            if let Some(udt) = udt_name {
                let stripped = udt.trim_start_matches('_');
                format!("{}[]", stripped)
            } else {
                "unknown[]".to_string()
            }
        }
        _ => lower,
    }
}

pub(super) fn pg_row_to_json(row: &PgRow, index: usize) -> serde_json::Value {
    if let Ok(v) = row.try_get::<Option<sqlx::types::Json<serde_json::Value>>, _>(index) {
        return v.map(|j| j.0).unwrap_or(serde_json::Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<Vec<String>>, _>(index) {
        return serde_json::json!(v);
    }
    if let Ok(v) = row.try_get::<Option<Vec<i32>>, _>(index) {
        return serde_json::json!(v);
    }
    if let Ok(v) = row.try_get::<Option<Vec<i64>>, _>(index) {
        return serde_json::json!(v);
    }
    if let Ok(v) = row.try_get::<Option<Vec<f64>>, _>(index) {
        return serde_json::json!(v);
    }
    if let Ok(v) = row.try_get::<Option<Vec<bool>>, _>(index) {
        return serde_json::json!(v);
    }
    if let Ok(v) = row.try_get::<Option<uuid::Uuid>, _>(index) {
        return match v {
            Some(u) => serde_json::json!(u.to_string()),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(index) {
        return match v {
            Some(dt) => serde_json::json!(dt.to_rfc3339()),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<chrono::NaiveDateTime>, _>(index) {
        return match v {
            Some(dt) => serde_json::json!(dt.to_string()),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<chrono::NaiveDate>, _>(index) {
        return match v {
            Some(d) => serde_json::json!(d.to_string()),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<chrono::NaiveTime>, _>(index) {
        return match v {
            Some(t) => serde_json::json!(t.to_string()),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<String>, _>(index) {
        return match v {
            Some(s) => serde_json::Value::String(s),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<i64>, _>(index) {
        return match v {
            Some(n) => serde_json::json!(n),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<i32>, _>(index) {
        return match v {
            Some(n) => serde_json::json!(n),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<i16>, _>(index) {
        return match v {
            Some(n) => serde_json::json!(n),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<f64>, _>(index) {
        return match v {
            Some(n) => serde_json::json!(n),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<f32>, _>(index) {
        return match v {
            Some(n) => serde_json::json!(n),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<bool>, _>(index) {
        return match v {
            Some(b) => serde_json::json!(b),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<Vec<u8>>, _>(index) {
        return match v {
            Some(bytes) => serde_json::json!(format!("<binary: {} bytes>", bytes.len())),
            None => serde_json::Value::Null,
        };
    }

    let type_name = row.column(index).type_info().name().to_string();
    serde_json::Value::String(format!("<{}>", type_name))
}
