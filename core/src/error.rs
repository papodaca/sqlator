use serde::Serialize;
use std::fmt;

#[derive(Debug, Serialize, Clone)]
pub struct CoreError {
    pub message: String,
    pub code: String,
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for CoreError {}

impl From<sqlx::Error> for CoreError {
    fn from(e: sqlx::Error) -> Self {
        let code = match &e {
            sqlx::Error::Database(_) => "DATABASE_ERROR",
            sqlx::Error::Io(_) => "CONNECTION_FAILED",
            sqlx::Error::PoolTimedOut => "TIMEOUT",
            _ => "UNKNOWN",
        };
        CoreError {
            message: e.to_string(),
            code: code.to_string(),
        }
    }
}

impl From<url::ParseError> for CoreError {
    fn from(e: url::ParseError) -> Self {
        CoreError {
            message: format!("Invalid URL: {}", e),
            code: "INVALID_URL".to_string(),
        }
    }
}

impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        CoreError {
            message: e.to_string(),
            code: "IO_ERROR".to_string(),
        }
    }
}

impl From<serde_json::Error> for CoreError {
    fn from(e: serde_json::Error) -> Self {
        CoreError {
            message: e.to_string(),
            code: "SERIALIZATION_ERROR".to_string(),
        }
    }
}

impl From<tiberius::error::Error> for CoreError {
    fn from(e: tiberius::error::Error) -> Self {
        CoreError {
            message: e.to_string(),
            code: "DATABASE_ERROR".to_string(),
        }
    }
}

impl From<oracle_rs::Error> for CoreError {
    fn from(e: oracle_rs::Error) -> Self {
        CoreError {
            message: e.to_string(),
            code: "DATABASE_ERROR".to_string(),
        }
    }
}
