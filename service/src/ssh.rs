//! SSH profile helpers (auth parsing + credential resolution).
//!
//! Tunnel orchestration arrives in Phase 1d.

use crate::error::ServiceError;
use sqlator_core::credentials::CredentialStore;
use sqlator_core::models::{SshAuthMethod, SshProfile};
use sqlator_core::ssh::{SshAuthConfig, SshHostConfig};

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

/// Resolve stored credentials into an [`SshAuthConfig`] for a profile.
///
/// Collapses the three former copies in Tauri `commands.rs`, web `handlers.rs`,
/// and Tauri `terminal.rs` (`resolve_ssh_auth`).
pub fn build_auth_config_for_profile(
    profile: &SshProfile,
    credentials: &CredentialStore,
) -> Result<SshAuthConfig, ServiceError> {
    match profile.auth_method {
        SshAuthMethod::Key => {
            let key_path = profile.key_path.as_deref().unwrap_or_default();
            let passphrase = credentials.get_credential(&profile.id, "passphrase")?;
            if let Some(pp) = passphrase {
                Ok(SshAuthConfig::with_key_and_passphrase(
                    &profile.username,
                    key_path,
                    pp,
                ))
            } else {
                Ok(SshAuthConfig::with_key(&profile.username, key_path))
            }
        }
        SshAuthMethod::Password => {
            let password = credentials
                .get_credential(&profile.id, "password")?
                .unwrap_or_default();
            Ok(SshAuthConfig::with_password(&profile.username, password))
        }
        SshAuthMethod::Agent => Ok(SshAuthConfig::with_agent(&profile.username)),
    }
}

/// Build jump-host `(config, auth)` pairs for a profile's proxy-jump chain.
///
/// Password auth on stored jump hosts is rejected (same message as both frontends).
pub fn build_jump_hosts_for_profile(
    profile: &SshProfile,
) -> Result<Vec<(SshHostConfig, SshAuthConfig)>, ServiceError> {
    profile
        .proxy_jump
        .iter()
        .map(|jump| {
            let auth = match jump.auth_method {
                SshAuthMethod::Key => {
                    let key_path = jump.key_path.as_deref().unwrap_or_default();
                    SshAuthConfig::with_key(&jump.username, key_path)
                }
                SshAuthMethod::Agent => SshAuthConfig::with_agent(&jump.username),
                SshAuthMethod::Password => {
                    return Err(ServiceError::app(
                        "JUMP_PASSWORD_UNSUPPORTED",
                        format!(
                            "Jump host '{}' uses password auth, which is not supported for stored jump hosts",
                            jump.host
                        ),
                    ));
                }
            };
            // SshAuthConfig still derives Clone today (ledger row 10 / sqlator-33j).
            let config = SshHostConfig::new(&jump.host, jump.port, auth.clone());
            Ok((config, auth))
        })
        .collect()
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
