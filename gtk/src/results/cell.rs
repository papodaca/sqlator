//! Typed cell values for the results grid.

use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

impl CellValue {
    pub fn from_json(value: &serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(b) => Self::Bool(*b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Self::Int(i)
                } else if let Some(u) = n.as_u64() {
                    if u <= i64::MAX as u64 {
                        Self::Int(u as i64)
                    } else {
                        Self::Text(n.to_string())
                    }
                } else if let Some(f) = n.as_f64() {
                    Self::Float(f)
                } else {
                    Self::Text(n.to_string())
                }
            }
            serde_json::Value::String(s) => Self::Text(s.clone()),
            other => Self::Text(other.to_string()),
        }
    }

    pub fn display(&self) -> String {
        match self {
            Self::Null => String::new(),
            Self::Bool(b) => b.to_string(),
            Self::Int(i) => i.to_string(),
            Self::Float(f) => {
                // Prefer compact display; keep enough precision for inspection.
                let s = format!("{f}");
                if s.contains('e') || s.contains('E') {
                    format!("{f:.6}")
                } else {
                    s
                }
            }
            Self::Text(s) => s.clone(),
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Null => serde_json::Value::Null,
            Self::Bool(b) => serde_json::Value::Bool(*b),
            Self::Int(i) => serde_json::json!(i),
            Self::Float(f) => serde_json::json!(f),
            Self::Text(s) => serde_json::Value::String(s.clone()),
        }
    }

    pub fn from_json_value(value: serde_json::Value) -> Self {
        Self::from_json(&value)
    }

    pub fn cmp_typed(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Null, Self::Null) => Ordering::Equal,
            (Self::Null, _) => Ordering::Greater,
            (_, Self::Null) => Ordering::Less,
            (Self::Bool(a), Self::Bool(b)) => a.cmp(b),
            (Self::Int(a), Self::Int(b)) => a.cmp(b),
            (Self::Float(a), Self::Float(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
            (a, b) => a.tag().cmp(&b.tag()),
        }
    }

    fn tag(&self) -> u8 {
        match self {
            Self::Null => 0,
            Self::Bool(_) => 1,
            Self::Int(_) => 2,
            Self::Float(_) => 3,
            Self::Text(_) => 4,
        }
    }

    pub fn as_tsv_field(&self) -> String {
        match self {
            Self::Null => r"\N".into(),
            other => {
                let s = other.display();
                if s.contains(['\t', '\n', '\r', '"']) {
                    format!("\"{}\"", s.replace('"', "\"\""))
                } else {
                    s
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ColumnMeta {
    pub name: String,
    pub fixed_width: i32,
}

impl ColumnMeta {
    pub fn from_name(name: impl Into<String>) -> Self {
        let name = name.into();
        // Rough monospace estimate; ColumnView still lets the user resize.
        let fixed_width = ((name.len() as i32) * 9 + 24).clamp(80, 240);
        Self { name, fixed_width }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn from_json_maps_null_bool_int_float_text() {
        assert_eq!(CellValue::from_json(&json!(null)), CellValue::Null);
        assert_eq!(CellValue::from_json(&json!(true)), CellValue::Bool(true));
        assert_eq!(CellValue::from_json(&json!(42)), CellValue::Int(42));
        assert_eq!(CellValue::from_json(&json!(1.5)), CellValue::Float(1.5));
        assert_eq!(
            CellValue::from_json(&json!("hi")),
            CellValue::Text("hi".into())
        );
    }

    #[test]
    fn null_sorts_last_and_tsv_uses_postgres_null() {
        assert_eq!(
            CellValue::Null.cmp_typed(&CellValue::Int(1)),
            std::cmp::Ordering::Greater
        );
        assert_eq!(CellValue::Null.as_tsv_field(), r"\N");
        assert_eq!(CellValue::Text("a\tb".into()).as_tsv_field(), "\"a\tb\"");
    }

    #[test]
    fn null_display_is_empty_for_inscription_overlay() {
        // Display text is empty; the grid paints the "NULL" label + CSS class.
        assert_eq!(CellValue::Null.display(), "");
        assert!(CellValue::Null.is_null());
    }

    #[test]
    fn ints_sort_numerically_not_lexicographically() {
        let mut vals = [CellValue::Int(10), CellValue::Int(2), CellValue::Int(1)];
        vals.sort_by(|a, b| a.cmp_typed(b));
        assert_eq!(
            vals,
            [CellValue::Int(1), CellValue::Int(2), CellValue::Int(10),]
        );
    }
}
