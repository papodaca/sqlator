//! Pagination classification and wrapper construction for ad-hoc queries (U1).
//!
//! `classify_pagination` decides whether a user query can be executed inside
//! the transparent pagination wrapper (KTD-3); `wrap_for_pagination` builds the
//! CTE + dialect page clause shape from the plan (R4). Both are pure functions.

use crate::models::SortSpec;
use sqlparser::ast::Statement;
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;

/// Per-dialect page-clause family (HTD "Dialect page clauses").
///
/// `DatabaseType` has no `Any` variant (it is a URL-scheme detection result,
/// and unknown schemes fall through to `DatabasePool::Any`), so a dedicated
/// enum carries all seven pool shapes without touching the exported enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PagedDialect {
    Postgres,
    MySql,
    Sqlite,
    Any,
    Mssql,
    Oracle,
    ClickHouse,
}

/// Why a query is not wrapped (KTD-3). Tags feed `tracing::debug` on the
/// passthrough path; passthrough behavior is identical to today (R6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PassthroughReason {
    /// Nothing to execute (blank or comment-only input).
    NoStatement,
    /// More than one statement in the editor buffer.
    MultiStatement,
    /// Parsed, but not a `Statement::Query` (DML, DDL, EXPLAIN/SHOW/…).
    NonQueryStatement,
    /// MSSQL rejects CTE bodies containing their own WITH.
    MssqlTopLevelWith,
    /// sqlparser could not parse the input; safest to run it as-is.
    ParseFailure,
}

impl PassthroughReason {
    /// Stable kebab-case tag for tracing fields.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NoStatement => "no-statement",
            Self::MultiStatement => "multi-statement",
            Self::NonQueryStatement => "non-query-statement",
            Self::MssqlTopLevelWith => "mssql-top-level-with",
            Self::ParseFailure => "parse-failure",
        }
    }
}

/// Wrappable queries run through the pagination wrapper; everything else is
/// delegated to today's streaming path unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaginationClassification {
    Wrappable,
    Passthrough(PassthroughReason),
}

/// Outcome of one [`crate::db::DbManager::execute_query_paged`] call (KTD-2).
///
/// Returned alongside the normal `QueryEvent` stream — `QueryEvent` and its
/// five variants stay untouched; consumers learn page state from this struct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PagedQueryOutcome {
    /// Rows forwarded to the caller's channel on this run. Always 0 for
    /// passthrough runs (callers track passthrough rows via events, as today).
    pub row_count: usize,
    /// Whether another page exists after this one (sentinel row arrived).
    /// Always false for passthrough runs.
    pub has_more: bool,
    /// Whether the 50,000-row ceiling stopped this run (R5).
    pub capped: bool,
    /// True when the query executed through the pagination wrapper; false
    /// when it was a passthrough run. The GTK tab learns wrappability from
    /// page one through this flag (plan U3).
    pub paged: bool,
}

impl PagedQueryOutcome {
    /// Outcome for a passthrough run: no paging metadata is computed; the
    /// caller keeps driving from raw events exactly like today (R6).
    pub fn passthrough() -> Self {
        Self {
            row_count: 0,
            has_more: false,
            capped: false,
            paged: false,
        }
    }
}

/// Classify `sql` for paged execution (KTD-3).
///
/// Wrappable = exactly one parseable `Statement::Query` (plain SELECT or
/// top-level WITH) via sqlparser's GenericDialect. Everything else passes
/// through; a parse failure is always safe — worst case is today's behavior.
pub(crate) fn classify_pagination(sql: &str, dialect: PagedDialect) -> PaginationClassification {
    use PaginationClassification::{Passthrough, Wrappable};
    use PassthroughReason::*;

    let statements = match Parser::parse_sql(&GenericDialect {}, sql) {
        Ok(statements) => statements,
        Err(_) => return Passthrough(ParseFailure),
    };

    if statements.is_empty() {
        return Passthrough(NoStatement);
    }
    if statements.len() > 1 {
        return Passthrough(MultiStatement);
    }

    let Statement::Query(query) = &statements[0] else {
        return Passthrough(NonQueryStatement);
    };

    if dialect == PagedDialect::Mssql && query.with.is_some() {
        return Passthrough(MssqlTopLevelWith);
    }

    Wrappable
}

/// Build the pagination wrapper from the HTD anatomy:
///
/// ```text
/// WITH sqlator_page_q AS ( <user sql> )
/// SELECT * FROM sqlator_page_q [ORDER BY <validated sort>] <dialect page clause>
/// ```
///
/// A user `LIMIT`/`TOP` stays inside the body and applies first — semantics
/// never change. The sentinel fetch size is `limit + 1` (KTD-2). Sort columns
/// are whitelist-validated (`validate_column` precedent); unknown columns are
/// silently dropped and never string-interpolated. Offsets/limits are
/// integers only, so the outer clause carries no user text.
pub(crate) fn wrap_for_pagination(
    user_sql: &str,
    dialect: PagedDialect,
    sort: &[SortSpec],
    valid_columns: &[&str],
    limit: usize,
    offset: usize,
) -> String {
    // Strip trailing semicolons so the body parses inside the CTE parens.
    // (A trailing comment after the semicolon defeats this loop; the wrapper
    // then errors at runtime and surfaces through today's Error-event path.)
    let mut body = user_sql.trim();
    while let Some(stripped) = body.strip_suffix(';') {
        body = stripped.trim_end();
    }

    let order = order_clause(dialect, sort, valid_columns);
    let fetch = limit + 1;
    let page = match dialect {
        PagedDialect::Mssql | PagedDialect::Oracle => {
            format!("OFFSET {offset} ROWS FETCH NEXT {fetch} ROWS ONLY")
        }
        PagedDialect::Postgres
        | PagedDialect::MySql
        | PagedDialect::Sqlite
        | PagedDialect::Any
        | PagedDialect::ClickHouse => {
            format!("LIMIT {fetch} OFFSET {offset}")
        }
    };

    // Oracle rejects a CTE whose body is itself a WITH. Subquery wrap keeps
    // OFFSET/FETCH while remaining valid SQL.
    if dialect == PagedDialect::Oracle && body_has_top_level_with(body) {
        return format!("SELECT * FROM (\n{body}\n) sqlator_page_q{order}\n{page}");
    }

    format!("WITH sqlator_page_q AS (\n{body}\n)\nSELECT * FROM sqlator_page_q{order}\n{page}")
}

fn body_has_top_level_with(sql: &str) -> bool {
    let Ok(statements) = Parser::parse_sql(&GenericDialect {}, sql) else {
        return false;
    };
    matches!(
        statements.first(),
        Some(Statement::Query(query)) if query.with.is_some()
    )
}

/// Outer ORDER BY, reusing the module's quoting helpers (plan U1 approach).
fn order_clause(dialect: PagedDialect, sort: &[SortSpec], valid_columns: &[&str]) -> String {
    match dialect {
        PagedDialect::Postgres => super::build_order_by_pg(sort, valid_columns),
        PagedDialect::MySql | PagedDialect::ClickHouse => {
            super::build_order_by_generic(sort, valid_columns, '`')
        }
        PagedDialect::Sqlite | PagedDialect::Any => {
            super::build_order_by_generic(sort, valid_columns, '"')
        }
        PagedDialect::Oracle => {
            // OFFSET/FETCH NEXT requires ORDER BY. Unquoted seed columns are
            // stored uppercase; quoted mixed-case names close the session.
            let order = build_order_by_oracle_paged(sort, valid_columns);
            if order.is_empty() {
                " ORDER BY 1".to_string()
            } else {
                order
            }
        }
        PagedDialect::Mssql => {
            // OFFSET/FETCH NEXT requires ORDER BY (existing precedent in
            // core/src/db/mssql.rs `query_table`).
            let order = build_order_by_mssql_paged(sort, valid_columns);
            if order.is_empty() {
                " ORDER BY (SELECT NULL)".to_string()
            } else {
                order
            }
        }
    }
}

/// Quote Oracle sort identifiers uppercase so they match unquoted columns.
fn build_order_by_oracle_paged(sort: &[SortSpec], valid: &[&str]) -> String {
    let parts: Vec<String> = sort
        .iter()
        .filter(|s| super::validate_column(&s.column, valid))
        .map(|s| {
            let ident = s.column.to_ascii_uppercase().replace('"', "\"\"");
            format!("\"{}\" {}", ident, if s.desc { "DESC" } else { "ASC" })
        })
        .collect();

    if parts.is_empty() {
        String::new()
    } else {
        format!(" ORDER BY {}", parts.join(", "))
    }
}

/// Mirrors `build_order_by_mssql` (core/src/db/mssql.rs). Kept local because
/// the driver module's copy is private and driver files stay untouched (U1).
fn build_order_by_mssql_paged(sort: &[SortSpec], valid: &[&str]) -> String {
    let parts: Vec<String> = sort
        .iter()
        .filter(|s| super::validate_column(&s.column, valid))
        .map(|s| {
            let col = format!("[{}]", s.column.replace(']', "]]"));
            format!("{} {}", col, if s.desc { "DESC" } else { "ASC" })
        })
        .collect();

    if parts.is_empty() {
        String::new()
    } else {
        format!(" ORDER BY {}", parts.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SortSpec;

    fn sort(column: &str, desc: bool) -> SortSpec {
        SortSpec {
            column: column.into(),
            desc,
        }
    }

    const NON_MSSQL_DIALECTS: [PagedDialect; 6] = [
        PagedDialect::Postgres,
        PagedDialect::MySql,
        PagedDialect::Sqlite,
        PagedDialect::Any,
        PagedDialect::Oracle,
        PagedDialect::ClickHouse,
    ];

    // ── Classification: wrappable ─────────────────────────────────────────────

    #[test]
    fn plain_select_is_wrappable_on_all_dialects() {
        for dialect in NON_MSSQL_DIALECTS
            .into_iter()
            .chain(std::iter::once(PagedDialect::Mssql))
        {
            assert_eq!(
                classify_pagination("SELECT id, name FROM users", dialect),
                PaginationClassification::Wrappable,
                "plain SELECT must be wrappable for {dialect:?}"
            );
        }
    }

    #[test]
    fn top_level_with_is_wrappable_except_mssql() {
        let sql = "WITH recent AS (SELECT id FROM t ORDER BY id DESC) SELECT * FROM recent";
        for dialect in NON_MSSQL_DIALECTS {
            assert_eq!(
                classify_pagination(sql, dialect),
                PaginationClassification::Wrappable,
                "top-level WITH must be wrappable for {dialect:?}"
            );
        }
    }

    #[test]
    fn top_level_with_passes_through_on_mssql() {
        // MSSQL rejects CTE bodies containing their own WITH (KTD-3).
        let sql = "WITH recent AS (SELECT id FROM t) SELECT * FROM recent";
        assert_eq!(
            classify_pagination(sql, PagedDialect::Mssql),
            PaginationClassification::Passthrough(PassthroughReason::MssqlTopLevelWith)
        );
    }

    #[test]
    fn select_with_inner_limit_is_wrappable() {
        assert_eq!(
            classify_pagination("SELECT id FROM t LIMIT 10", PagedDialect::Postgres),
            PaginationClassification::Wrappable
        );
    }

    #[test]
    fn leading_comments_and_whitespace_are_wrappable() {
        let sql = "  -- report for q3\n  /* multi\n line */\n SELECT id FROM t";
        assert_eq!(
            classify_pagination(sql, PagedDialect::Sqlite),
            PaginationClassification::Wrappable
        );
    }

    #[test]
    fn trailing_semicolon_is_wrappable() {
        assert_eq!(
            classify_pagination("SELECT id FROM t;", PagedDialect::Sqlite),
            PaginationClassification::Wrappable
        );
    }

    // ── Classification: passthrough ───────────────────────────────────────────

    #[test]
    fn dml_passes_through() {
        for sql in [
            "INSERT INTO t (a) VALUES (1)",
            "UPDATE t SET a = 1",
            "DELETE FROM t WHERE a = 1",
        ] {
            assert_eq!(
                classify_pagination(sql, PagedDialect::Postgres),
                PaginationClassification::Passthrough(PassthroughReason::NonQueryStatement),
                "{sql:?} must pass through"
            );
        }
    }

    #[test]
    fn ddl_passes_through() {
        assert_eq!(
            classify_pagination("CREATE TABLE t (a INT)", PagedDialect::Postgres),
            PaginationClassification::Passthrough(PassthroughReason::NonQueryStatement)
        );
    }

    #[test]
    fn explain_passes_through() {
        assert_eq!(
            classify_pagination("EXPLAIN SELECT 1 FROM t", PagedDialect::Postgres),
            PaginationClassification::Passthrough(PassthroughReason::NonQueryStatement)
        );
    }

    #[test]
    fn show_passes_through() {
        assert_eq!(
            classify_pagination("SHOW TABLES", PagedDialect::MySql),
            PaginationClassification::Passthrough(PassthroughReason::NonQueryStatement)
        );
    }

    #[test]
    fn describe_passes_through() {
        // sqlparser may or may not parse DESCRIBE per dialect; either way it
        // must not be treated as a wrappable Query.
        assert!(matches!(
            classify_pagination("DESCRIBE t", PagedDialect::MySql),
            PaginationClassification::Passthrough(_)
        ));
    }

    #[test]
    fn two_statements_pass_through() {
        assert_eq!(
            classify_pagination("SELECT 1; SELECT 2", PagedDialect::Sqlite),
            PaginationClassification::Passthrough(PassthroughReason::MultiStatement)
        );
    }

    #[test]
    fn unparsable_sql_passes_through() {
        assert_eq!(
            classify_pagination("SELEC 1 FRM nowhere", PagedDialect::Sqlite),
            PaginationClassification::Passthrough(PassthroughReason::ParseFailure)
        );
    }

    #[test]
    fn empty_sql_passes_through() {
        assert_eq!(
            classify_pagination("   ", PagedDialect::Sqlite),
            PaginationClassification::Passthrough(PassthroughReason::NoStatement)
        );
    }

    // ── Wrapper construction ──────────────────────────────────────────────────

    #[test]
    fn wrap_limit_offset_dialects() {
        for dialect in [
            PagedDialect::Postgres,
            PagedDialect::MySql,
            PagedDialect::Sqlite,
            PagedDialect::Any,
            PagedDialect::ClickHouse,
        ] {
            let wrapped = wrap_for_pagination("SELECT * FROM t", dialect, &[], &[], 500, 100);
            let expected = "WITH sqlator_page_q AS (\nSELECT * FROM t\n)\nSELECT * FROM sqlator_page_q\nLIMIT 501 OFFSET 100";
            assert_eq!(wrapped, expected, "wrapper shape wrong for {dialect:?}");
        }
    }

    #[test]
    fn wrap_mssql_no_sort_emits_select_null_fallback() {
        let wrapped =
            wrap_for_pagination("SELECT * FROM t", PagedDialect::Mssql, &[], &[], 500, 100);
        let expected = "WITH sqlator_page_q AS (\nSELECT * FROM t\n)\nSELECT * FROM sqlator_page_q ORDER BY (SELECT NULL)\nOFFSET 100 ROWS FETCH NEXT 501 ROWS ONLY";
        assert_eq!(wrapped, expected);
    }

    #[test]
    fn wrap_oracle_offset_fetch_requires_order_by() {
        let wrapped =
            wrap_for_pagination("SELECT * FROM t", PagedDialect::Oracle, &[], &[], 500, 100);
        let expected = "WITH sqlator_page_q AS (\nSELECT * FROM t\n)\nSELECT * FROM sqlator_page_q ORDER BY 1\nOFFSET 100 ROWS FETCH NEXT 501 ROWS ONLY";
        assert_eq!(wrapped, expected);
    }

    #[test]
    fn wrap_oracle_top_level_with_uses_subquery_not_nested_cte() {
        let sql = "WITH q AS (SELECT id FROM t) SELECT id FROM q";
        let wrapped = wrap_for_pagination(sql, PagedDialect::Oracle, &[], &[], 500, 0);
        let expected = "SELECT * FROM (\nWITH q AS (SELECT id FROM t) SELECT id FROM q\n) sqlator_page_q ORDER BY 1\nOFFSET 0 ROWS FETCH NEXT 501 ROWS ONLY";
        assert_eq!(wrapped, expected);
        assert!(!wrapped.contains("WITH sqlator_page_q"), "{wrapped}");
    }

    #[test]
    fn wrap_postgres_sort_uses_nulls_last() {
        let sort = [sort("n", false)];
        let wrapped = wrap_for_pagination(
            "SELECT n FROM t",
            PagedDialect::Postgres,
            &sort,
            &["n"],
            500,
            0,
        );
        assert!(
            wrapped.contains(" ORDER BY \"n\" ASC NULLS LAST\nLIMIT 501 OFFSET 0"),
            "pg sort clause wrong: {wrapped}"
        );
    }

    #[test]
    fn wrap_mysql_quotes_with_backticks() {
        let sort = [sort("n", true)];
        let wrapped = wrap_for_pagination(
            "SELECT n FROM t",
            PagedDialect::MySql,
            &sort,
            &["n"],
            500,
            750,
        );
        assert!(
            wrapped.contains(" ORDER BY `n` DESC\nLIMIT 501 OFFSET 750"),
            "mysql sort clause wrong: {wrapped}"
        );
    }

    #[test]
    fn wrap_two_column_sort_renders_stable_order() {
        let sort = [sort("a", false), sort("b", true)];
        let wrapped = wrap_for_pagination(
            "SELECT a, b FROM t",
            PagedDialect::Sqlite,
            &sort,
            &["a", "b"],
            100,
            0,
        );
        assert!(
            wrapped.contains(" ORDER BY \"a\" ASC, \"b\" DESC\nLIMIT 101 OFFSET 0"),
            "two-column sort must render ASC then DESC in spec order: {wrapped}"
        );
    }

    #[test]
    fn wrap_oracle_honors_user_sort() {
        let sort = [sort("id", true)];
        let wrapped = wrap_for_pagination(
            "SELECT id FROM t",
            PagedDialect::Oracle,
            &sort,
            &["id"],
            10,
            20,
        );
        assert!(
            wrapped.contains(" ORDER BY \"ID\" DESC\nOFFSET 20 ROWS FETCH NEXT 11 ROWS ONLY"),
            "oracle must quote unquoted columns uppercase: {wrapped}"
        );
        assert!(!wrapped.contains("ORDER BY 1"), "{wrapped}");
        assert!(!wrapped.contains("\"id\""), "{wrapped}");
    }

    #[test]
    fn wrap_oracle_unknown_sort_column_falls_back_to_ordinal() {
        let sort = [sort("evil_col", false)];
        let wrapped = wrap_for_pagination(
            "SELECT a FROM t",
            PagedDialect::Oracle,
            &sort,
            &["a"],
            500,
            0,
        );
        assert!(!wrapped.contains("evil_col"), "{wrapped}");
        assert!(
            wrapped.contains("ORDER BY 1"),
            "oracle still needs an ORDER BY for OFFSET/FETCH: {wrapped}"
        );
    }

    #[test]
    fn wrap_mssql_honors_user_sort() {
        let sort = [sort("id", true)];
        let wrapped = wrap_for_pagination(
            "SELECT id FROM t",
            PagedDialect::Mssql,
            &sort,
            &["id"],
            10,
            20,
        );
        assert!(
            wrapped.contains(" ORDER BY [id] DESC\nOFFSET 20 ROWS FETCH NEXT 11 ROWS ONLY"),
            "mssql must honor user sort instead of fallback: {wrapped}"
        );
        assert!(!wrapped.contains("(SELECT NULL)"), "{wrapped}");
    }

    #[test]
    fn wrap_unknown_sort_column_is_silently_dropped() {
        let sort = [sort("evil_col", true)];
        let wrapped = wrap_for_pagination(
            "SELECT a FROM t",
            PagedDialect::Postgres,
            &sort,
            &["a"],
            500,
            0,
        );
        assert!(
            !wrapped.contains("evil_col"),
            "unknown sort column must never enter the SQL: {wrapped}"
        );
        assert!(
            !wrapped.contains("ORDER BY"),
            "only-unknown sort must leave the wrapper unsorted: {wrapped}"
        );
    }

    #[test]
    fn wrap_mssql_unknown_sort_column_falls_back_to_select_null() {
        let sort = [sort("evil_col", false)];
        let wrapped = wrap_for_pagination(
            "SELECT a FROM t",
            PagedDialect::Mssql,
            &sort,
            &["a"],
            500,
            0,
        );
        assert!(!wrapped.contains("evil_col"), "{wrapped}");
        assert!(
            wrapped.contains("ORDER BY (SELECT NULL)"),
            "mssql still needs an ORDER BY for OFFSET/FETCH: {wrapped}"
        );
    }

    #[test]
    fn wrap_strips_trailing_semicolon_from_body() {
        let wrapped =
            wrap_for_pagination("SELECT id FROM t;", PagedDialect::Sqlite, &[], &[], 500, 0);
        assert!(
            !wrapped.contains(';'),
            "semicolons must not leak into the CTE body: {wrapped}"
        );
        assert!(wrapped.contains("AS (\nSELECT id FROM t\n)"), "{wrapped}");
    }

    #[test]
    fn wrap_preserves_inner_limit_in_body() {
        let wrapped = wrap_for_pagination(
            "SELECT id FROM t LIMIT 10",
            PagedDialect::Sqlite,
            &[],
            &[],
            500,
            0,
        );
        assert!(
            wrapped.contains("AS (\nSELECT id FROM t LIMIT 10\n)"),
            "user LIMIT stays inside the body (semantics unchanged): {wrapped}"
        );
        assert!(
            wrapped.contains("LIMIT 501 OFFSET 0"),
            "sentinel limit goes on the wrapper: {wrapped}"
        );
    }

    #[test]
    fn wrap_escapes_quotes_in_valid_column_names() {
        let sort = [sort("we\"ird", true)];
        let wrapped = wrap_for_pagination(
            "SELECT \"we\"\"ird\" FROM t",
            PagedDialect::Postgres,
            &sort,
            &["we\"ird"],
            5,
            0,
        );
        assert!(
            wrapped.contains(" ORDER BY \"we\"\"ird\" DESC NULLS LAST"),
            "quote characters must be escaped, not interpolated raw: {wrapped}"
        );
    }

    #[test]
    fn classification_reasons_have_stable_tags_for_tracing() {
        // Reason tags feed tracing::debug on the passthrough path.
        assert_eq!(PassthroughReason::ParseFailure.as_str(), "parse-failure");
        assert_eq!(
            PassthroughReason::MultiStatement.as_str(),
            "multi-statement"
        );
        assert_eq!(
            PassthroughReason::NonQueryStatement.as_str(),
            "non-query-statement"
        );
        assert_eq!(
            PassthroughReason::MssqlTopLevelWith.as_str(),
            "mssql-top-level-with"
        );
        assert_eq!(PassthroughReason::NoStatement.as_str(), "no-statement");
    }
}
