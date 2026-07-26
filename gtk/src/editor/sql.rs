//! Pure SQL helpers for the GTK editor (statement bounds + destructive guard).

/// Byte-offset range of one statement inside a SQL buffer (`end` exclusive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatementSpan {
    pub start: usize,
    pub end: usize,
}

/// Split `sql` into statement spans on `;` outside quotes / line+block comments.
pub fn statement_spans(sql: &str) -> Vec<StatementSpan> {
    let bytes = sql.as_bytes();
    let mut spans = Vec::new();
    let mut stmt_start = 0usize;
    let mut i = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    while i < bytes.len() {
        let c = bytes[i];
        let next = bytes.get(i + 1).copied();

        if in_line_comment {
            if c == b'\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }
        if in_block_comment {
            if c == b'*' && next == Some(b'/') {
                in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if in_single {
            if c == b'\'' {
                // SQL escaped quote: ''
                if next == Some(b'\'') {
                    i += 2;
                } else {
                    in_single = false;
                    i += 1;
                }
            } else {
                i += 1;
            }
            continue;
        }
        if in_double {
            if c == b'"' {
                if next == Some(b'"') {
                    i += 2;
                } else {
                    in_double = false;
                    i += 1;
                }
            } else {
                i += 1;
            }
            continue;
        }

        match c {
            b'-' if next == Some(b'-') => {
                in_line_comment = true;
                i += 2;
            }
            b'/' if next == Some(b'*') => {
                in_block_comment = true;
                i += 2;
            }
            b'\'' => {
                in_single = true;
                i += 1;
            }
            b'"' => {
                in_double = true;
                i += 1;
            }
            b';' => {
                let end = i; // exclude the semicolon
                if sql[stmt_start..end].trim().is_empty() {
                    // skip empty
                } else {
                    spans.push(StatementSpan {
                        start: stmt_start,
                        end,
                    });
                }
                stmt_start = i + 1;
                i += 1;
            }
            _ => i += 1,
        }
    }

    if !sql[stmt_start..].trim().is_empty() {
        spans.push(StatementSpan {
            start: stmt_start,
            end: sql.len(),
        });
    }
    spans
}

/// Statement containing `cursor` (byte offset), or the last statement if the
/// cursor sits in trailing whitespace. Returns `None` for an empty buffer.
pub fn statement_at_cursor(sql: &str, cursor: usize) -> Option<StatementSpan> {
    let cursor = cursor.min(sql.len());
    let spans = statement_spans(sql);
    if spans.is_empty() {
        return None;
    }
    for span in &spans {
        if cursor >= span.start && cursor <= span.end {
            return Some(*span);
        }
        // Cursor between statements (on the `;` or whitespace) → previous stmt.
        if cursor < span.start {
            break;
        }
    }
    // After the last span, or landed on a semicolon: use last non-empty.
    spans.last().copied()
}

/// Text of the statement under `cursor`, trimmed. Empty when none.
pub fn sql_at_cursor(sql: &str, cursor: usize) -> String {
    statement_at_cursor(sql, cursor)
        .map(|s| sql[s.start..s.end].trim().to_string())
        .unwrap_or_default()
}

/// Whether `sql` looks destructive enough to confirm before running.
///
/// Matches: `DROP`, `TRUNCATE`, or `DELETE`/`UPDATE` with no `WHERE`.
pub fn is_destructive_sql(sql: &str) -> bool {
    let spans = statement_spans(sql);
    let iter: Box<dyn Iterator<Item = &str>> = if spans.is_empty() {
        Box::new(std::iter::once(sql))
    } else {
        Box::new(spans.iter().map(|s| &sql[s.start..s.end]))
    };
    for stmt in iter {
        if statement_is_destructive(stmt) {
            return true;
        }
    }
    false
}

fn statement_is_destructive(stmt: &str) -> bool {
    let stripped = strip_leading_comments(stmt);
    let upper = stripped.to_uppercase();
    let tokens: Vec<&str> = upper.split_whitespace().collect();
    if tokens.is_empty() {
        return false;
    }
    match tokens[0] {
        "DROP" | "TRUNCATE" => true,
        "DELETE" | "UPDATE" => !upper.split_whitespace().any(|t| t == "WHERE"),
        _ => false,
    }
}

fn strip_leading_comments(sql: &str) -> &str {
    let mut s = sql.trim_start();
    loop {
        if s.starts_with("--") {
            if let Some(pos) = s.find('\n') {
                s = s[pos + 1..].trim_start();
                continue;
            }
            return "";
        }
        if s.starts_with("/*") {
            if let Some(pos) = s.find("*/") {
                s = s[pos + 2..].trim_start();
                continue;
            }
            return "";
        }
        break;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_semicolons_respecting_quotes() {
        let sql = "SELECT ';'; SELECT \"a;b\"; DELETE FROM t";
        let spans = statement_spans(sql);
        assert_eq!(spans.len(), 3);
        assert_eq!(sql[spans[0].start..spans[0].end].trim(), "SELECT ';'");
        assert_eq!(sql[spans[1].start..spans[1].end].trim(), "SELECT \"a;b\"");
        assert_eq!(sql[spans[2].start..spans[2].end].trim(), "DELETE FROM t");
    }

    #[test]
    fn statement_at_cursor_picks_middle() {
        let sql = "SELECT 1; SELECT 2; SELECT 3;";
        let mid = sql.find("SELECT 2").unwrap() + 3;
        let got = sql_at_cursor(sql, mid);
        assert_eq!(got, "SELECT 2");
    }

    #[test]
    fn destructive_detects_drop_and_whereless_delete() {
        assert!(is_destructive_sql("DROP TABLE t"));
        assert!(is_destructive_sql("TRUNCATE t"));
        assert!(is_destructive_sql("DELETE FROM t"));
        assert!(is_destructive_sql("UPDATE t SET a = 1"));
        assert!(!is_destructive_sql("DELETE FROM t WHERE id = 1"));
        assert!(!is_destructive_sql("UPDATE t SET a = 1 WHERE id = 1"));
        assert!(!is_destructive_sql("SELECT * FROM t"));
    }

    #[test]
    fn destructive_ignores_leading_comments() {
        assert!(is_destructive_sql("-- oops\nDROP TABLE t"));
        assert!(!is_destructive_sql("/* DROP */ SELECT 1"));
    }
}
