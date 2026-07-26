//! SSH profile helpers (auth parsing; tunnel orchestration arrives in Phase 1d).

use crate::error::ServiceError;
use sqlator_core::models::SshAuthMethod;

/// Parse the wire string used by both frontends (`"key"` / `"password"` / `"agent"`).
///
/// Ledger row 8 winner: Tauri casing — `"Unknown auth method: {other}"`.
pub fn parse_auth_method(s: &str) -> Result<SshAuthMethod, ServiceError> {
    match s {
        "key" => Ok(SshAuthMethod::Key),
        "password" => Ok(SshAuthMethod::Password),
        "agent" => Ok(SshAuthMethod::Agent),
        other => Err(ServiceError::app(
            "UNKNOWN_AUTH_METHOD",
            format!("Unknown auth method: {other}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_auth_method_key_password_agent() {
        assert!(matches!(parse_auth_method("key"), Ok(SshAuthMethod::Key)));
        assert!(matches!(
            parse_auth_method("password"),
            Ok(SshAuthMethod::Password)
        ));
        assert!(matches!(
            parse_auth_method("agent"),
            Ok(SshAuthMethod::Agent)
        ));
    }

    #[test]
    fn parse_auth_method_unknown_error_string() {
        let err = parse_auth_method("token").unwrap_err();
        assert_eq!(err.code(), "UNKNOWN_AUTH_METHOD");
        assert_eq!(err.message(), "Unknown auth method: token");
    }
}
