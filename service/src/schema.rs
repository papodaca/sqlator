//! Table extraction, editability verdict helpers, and schema metadata TTL cache keys.
//!
//! Exactly one regex fallback lives here — the plan-007-corrected implementation
//! (comma/`JOIN` checks apply only inside the delimited FROM region).

use sqlator_core::models::{PrimaryKeyMeta, TableMeta};
use sqlparser::ast::{SetExpr, Statement, TableFactor};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;

/// Schema metadata cache TTL used by both frontends (5 minutes).
pub const SCHEMA_CACHE_TTL_SECS: u64 = 300;

/// Production cache key for [`TableMeta`]: Debug-format the optional schema so
/// `None` and `Some("")` never collide.
pub fn schema_cache_key(
    connection_id: &str,
    schema_name: &Option<String>,
    table_name: &str,
) -> String {
    format!("{connection_id}:{schema_name:?}:{table_name}")
}

/// Outcome of trying to identify a single editable source table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableExtract {
    /// One table reference found: `(table, optional schema)`.
    Found(String, Option<String>),
    /// Parser succeeded but the query is a join, CTE, subquery, etc.
    NotSingleTable,
    /// Parser failed and the regex fallback could not determine a table.
    Undetermined,
}

/// Extract the single source table from a simple SELECT query.
pub fn extract_single_table(sql: &str) -> TableExtract {
    let stmts = match Parser::parse_sql(&GenericDialect {}, sql) {
        Ok(s) => s,
        Err(_) => {
            return match extract_table_regex(sql) {
                Some((t, s)) => TableExtract::Found(t, s),
                None => TableExtract::Undetermined,
            };
        }
    };

    let Some(stmt) = stmts.into_iter().next() else {
        return TableExtract::NotSingleTable;
    };
    let query = match stmt {
        Statement::Query(q) => q,
        _ => return TableExtract::NotSingleTable,
    };

    // Unwrap CTEs — if there's a WITH clause, mark as not-single-table
    if query.with.is_some() {
        return TableExtract::NotSingleTable;
    }

    let body = match *query.body {
        SetExpr::Select(sel) => sel,
        _ => return TableExtract::NotSingleTable,
    };

    // Must have exactly one FROM table with no joins
    if body.from.len() != 1 {
        return TableExtract::NotSingleTable;
    }
    let table_with_joins = &body.from[0];
    if !table_with_joins.joins.is_empty() {
        return TableExtract::NotSingleTable;
    }

    match &table_with_joins.relation {
        TableFactor::Table { name, .. } => {
            let idents: Vec<String> = name.0.iter().map(|i| i.value.clone()).collect();
            match idents.len() {
                1 => TableExtract::Found(idents[0].clone(), None),
                2 => TableExtract::Found(idents[1].clone(), Some(idents[0].clone())),
                _ => TableExtract::NotSingleTable,
            }
        }
        // Subquery or function — not directly editable
        _ => TableExtract::NotSingleTable,
    }
}

/// Regex/heuristic fallback when sqlparser cannot parse the statement.
///
/// Delimits the FROM region first, then rejects commas/`JOIN` only inside that
/// region — so commas in the SELECT list are fine, but `FROM t1 , t2` is not.
/// Indexes against the original string (not an uppercased copy) so non-ASCII
/// before `FROM` cannot shift offsets.
pub fn extract_table_regex(sql: &str) -> Option<(String, Option<String>)> {
    let from_idx = sql
        .as_bytes()
        .windows(6)
        .position(|w| w.eq_ignore_ascii_case(b" from "))?;
    let after_from = sql[from_idx + 6..].trim_start();
    let from_region = delimit_from_region(after_from);

    // Multi-table FROM: comma or JOIN inside the region only
    if from_region.contains(',') {
        return None;
    }
    if from_region
        .as_bytes()
        .windows(6)
        .any(|w| w.eq_ignore_ascii_case(b" join "))
    {
        return None;
    }

    let table_token: String = from_region
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != ';' && *c != '\\')
        .collect();
    if table_token.is_empty() {
        return None;
    }

    split_schema_table(&table_token)
}

/// Non-editable [`TableMeta`] stub with a reason string for the UI.
pub fn non_editable_meta(reason: &str) -> TableMeta {
    TableMeta {
        table_name: String::new(),
        schema: None,
        columns: vec![],
        primary_key: PrimaryKeyMeta {
            columns: vec![],
            exists: false,
        },
        is_editable: false,
        editability_reason: Some(reason.into()),
    }
}

/// Slice of `after_from` up to the next clause keyword or `;`.
fn delimit_from_region(after_from: &str) -> &str {
    const KEYWORDS: &[&str] = &[
        "where",
        "group",
        "having",
        "order",
        "limit",
        "offset",
        "fetch",
        "window",
        "union",
        "intersect",
        "except",
        "for",
        "into",
    ];

    let bytes = after_from.as_bytes();
    let mut end = after_from.len();
    if let Some(semi) = after_from.find(';') {
        end = semi;
    }

    let mut i = 0;
    while i < end {
        while i < end && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= end {
            break;
        }
        let token_start = i;
        while i < end && !bytes[i].is_ascii_whitespace() && bytes[i] != b';' {
            i += 1;
        }
        let token = &after_from[token_start..i];
        if KEYWORDS.iter().any(|kw| token.eq_ignore_ascii_case(kw)) {
            end = token_start;
            break;
        }
    }

    after_from[..end].trim()
}

fn unquote_ident(s: &str) -> String {
    s.trim_matches(|c| c == '"' || c == '`' || c == '[' || c == ']')
        .to_string()
}

/// Split `schema.table` on the first dot not inside quotes/brackets.
fn split_schema_table(token: &str) -> Option<(String, Option<String>)> {
    let mut in_quotes: Option<char> = None;
    let mut dot_pos = None;
    for (i, c) in token.char_indices() {
        match (in_quotes, c) {
            (None, '"' | '`') => in_quotes = Some(c),
            (None, '[') => in_quotes = Some(']'),
            (Some(q), c) if c == q => in_quotes = None,
            (None, '.') => {
                dot_pos = Some(i);
                break;
            }
            _ => {}
        }
    }

    match dot_pos {
        Some(i) => Some((
            unquote_ident(&token[i + 1..]),
            Some(unquote_ident(&token[..i])),
        )),
        None => Some((unquote_ident(token), None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regex_simple_table() {
        assert_eq!(
            extract_table_regex("SELECT a FROM t"),
            Some(("t".into(), None))
        );
    }

    #[test]
    fn regex_comma_in_select_list() {
        assert_eq!(
            extract_table_regex("SELECT a, b FROM t"),
            Some(("t".into(), None))
        );
    }

    #[test]
    fn regex_comma_in_function_call() {
        assert_eq!(
            extract_table_regex("SELECT f(a, b) FROM t"),
            Some(("t".into(), None))
        );
    }

    #[test]
    fn regex_comma_in_in_list() {
        assert_eq!(
            extract_table_regex("SELECT a FROM t WHERE x IN (1,2)"),
            Some(("t".into(), None))
        );
    }

    #[test]
    fn regex_implicit_comma_join_rejected() {
        assert_eq!(extract_table_regex("SELECT a, b FROM t1, t2"), None);
        assert_eq!(extract_table_regex("SELECT a, b FROM t1 , t2"), None);
    }

    #[test]
    fn regex_join_rejected() {
        assert_eq!(
            extract_table_regex("SELECT a FROM t1 JOIN t2 ON t1.id = t2.id"),
            None
        );
    }

    #[test]
    fn regex_mixed_quoting() {
        assert_eq!(
            extract_table_regex("SELECT a FROM `\"t\"`"),
            Some(("t".into(), None))
        );
    }

    #[test]
    fn regex_schema_qualified() {
        assert_eq!(
            extract_table_regex("SELECT a FROM s.t"),
            Some(("t".into(), Some("s".into())))
        );
    }

    #[test]
    fn regex_mssql_brackets() {
        assert_eq!(
            extract_table_regex("SELECT a FROM [dbo].[t]"),
            Some(("t".into(), Some("dbo".into())))
        );
    }

    #[test]
    fn regex_non_ascii_before_from_does_not_shift_offset() {
        // Dotless-i uppercases to a different byte length; must still find `t`.
        assert_eq!(
            extract_table_regex("SELECT 'ııı' FROM t"),
            Some(("t".into(), None))
        );
    }

    #[test]
    fn regex_quoted_ident_containing_dot() {
        assert_eq!(
            extract_table_regex(r#"SELECT a FROM "my.schema".t"#),
            Some(("t".into(), Some("my.schema".into())))
        );
    }

    #[test]
    fn regex_mysql_g_terminator() {
        // Confirmed sqlparser failure; fallback must still extract the table.
        assert_eq!(
            extract_table_regex("SELECT a FROM t\\G"),
            Some(("t".into(), None))
        );
        match extract_single_table("SELECT a FROM t\\G") {
            TableExtract::Found(t, s) => {
                assert_eq!(t, "t");
                assert_eq!(s, None);
            }
            other => panic!("expected Found, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn undetermined_reason_for_fallback_failure() {
        match extract_single_table("SELECT a FROM t1 , t2\\G") {
            TableExtract::Undetermined | TableExtract::NotSingleTable => {}
            TableExtract::Found(t, _) => panic!("expected non-editable, got {t}"),
        }
    }

    #[test]
    fn extract_single_table_simple_select() {
        match extract_single_table("SELECT a FROM t") {
            TableExtract::Found(t, s) => {
                assert_eq!(t, "t");
                assert_eq!(s, None);
            }
            other => panic!(
                "expected Found, got discriminant {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }

    #[test]
    fn extract_single_table_schema_qualified() {
        match extract_single_table("SELECT a FROM public.users") {
            TableExtract::Found(t, s) => {
                assert_eq!(t, "users");
                assert_eq!(s.as_deref(), Some("public"));
            }
            other => panic!(
                "expected Found, got discriminant {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }

    #[test]
    fn extract_single_table_join_is_not_single() {
        assert!(matches!(
            extract_single_table("SELECT a FROM t1 JOIN t2 ON t1.id = t2.id"),
            TableExtract::NotSingleTable
        ));
    }

    #[test]
    fn extract_single_table_subquery_is_not_single() {
        assert!(matches!(
            extract_single_table("SELECT a FROM (SELECT 1 AS a) AS sub"),
            TableExtract::NotSingleTable
        ));
    }

    #[test]
    fn extract_single_table_cte_is_not_single() {
        assert!(matches!(
            extract_single_table("WITH c AS (SELECT 1 AS a) SELECT a FROM c"),
            TableExtract::NotSingleTable
        ));
    }

    #[test]
    fn extract_single_table_non_select_is_not_single() {
        assert!(matches!(
            extract_single_table("INSERT INTO t (a) VALUES (1)"),
            TableExtract::NotSingleTable
        ));
        assert!(matches!(
            extract_single_table("UPDATE t SET a = 1"),
            TableExtract::NotSingleTable
        ));
    }

    #[test]
    fn schema_cache_key_uses_debug_on_option() {
        assert_eq!(
            schema_cache_key("c1", &Some("public".into()), "users"),
            r#"c1:Some("public"):users"#
        );
        assert_eq!(schema_cache_key("c1", &None, "users"), "c1:None:users");
        assert_eq!(
            schema_cache_key("c1", &Some("".into()), "users"),
            r#"c1:Some(""):users"#
        );
        assert_ne!(
            schema_cache_key("c1", &None, "t"),
            schema_cache_key("c1", &Some("".into()), "t")
        );
    }

    #[test]
    fn schema_cache_ttl_constant_is_300_seconds() {
        assert_eq!(SCHEMA_CACHE_TTL_SECS, 300);
    }
}
