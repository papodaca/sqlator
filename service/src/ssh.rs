//! SSH profile helpers, profile CRUD, and standalone tunnel registry ops.
//!
//! Auth/jump builders live here; connect-path tunnel acquire/release is in
//! [`crate::connect`]. Registry keying: [`crate::tunnels::TunnelKey`].

use crate::error::ServiceError;
use crate::service::AppService;
use crate::tunnels::{ManagedTunnel, SshTunnelInfo, TunnelClaim, TunnelKey};
use serde::{Deserialize, Serialize};
use sqlator_core::credentials::CredentialStore;
use sqlator_core::models::{SshAuthMethod, SshJumpHost, SshProfile};
use sqlator_core::ssh::{SshAuthConfig, SshHostConfig, SshTunnel};

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

/// Frontend-facing SSH profile input (secrets optional; never returned on read).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshProfileConfig {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: String,
    pub key_path: Option<String>,
    pub password: Option<String>,
    pub key_passphrase: Option<String>,
    pub proxy_jump: Vec<SshJumpHost>,
    pub local_port_binding: Option<u16>,
    pub keepalive_interval: Option<u32>,
}

/// Standalone tunnel create request (UI “open tunnel” path).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshTunnelRequest {
    pub profile_id: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: String,
    pub password: Option<String>,
    pub key_path: Option<String>,
    pub key_passphrase: Option<String>,
    pub target_host: String,
    pub target_port: u16,
}

impl AppService {
    pub fn get_ssh_profiles(&self) -> Result<Vec<SshProfile>, ServiceError> {
        Ok(self.config.get_ssh_profiles()?)
    }

    pub fn save_ssh_profile(&self, config: SshProfileConfig) -> Result<SshProfile, ServiceError> {
        let id = uuid::Uuid::new_v4().to_string();
        let auth_method = parse_auth_method(&config.auth_method)?;
        let password = config.password.clone();
        let key_passphrase = config.key_passphrase.clone();
        let profile = SshProfile {
            id: id.clone(),
            name: config.name,
            host: config.host,
            port: config.port,
            username: config.username,
            auth_method,
            key_path: config.key_path,
            proxy_jump: config.proxy_jump,
            local_port_binding: config.local_port_binding,
            keepalive_interval: config.keepalive_interval,
        };
        self.config.save_ssh_profile(profile.clone())?;
        self.store_profile_secrets(&id, password.as_deref(), key_passphrase.as_deref())?;
        Ok(profile)
    }

    pub fn update_ssh_profile(
        &self,
        id: &str,
        config: SshProfileConfig,
    ) -> Result<SshProfile, ServiceError> {
        self.config.get_ssh_profile(id)?.ok_or_else(|| {
            ServiceError::app(
                "SSH_PROFILE_NOT_FOUND",
                format!("SSH profile '{id}' not found"),
            )
        })?;
        let auth_method = parse_auth_method(&config.auth_method)?;
        let password = config.password.clone();
        let key_passphrase = config.key_passphrase.clone();
        let profile = SshProfile {
            id: id.to_string(),
            name: config.name,
            host: config.host,
            port: config.port,
            username: config.username,
            auth_method,
            key_path: config.key_path,
            proxy_jump: config.proxy_jump,
            local_port_binding: config.local_port_binding,
            keepalive_interval: config.keepalive_interval,
        };
        self.config.update_ssh_profile(profile.clone())?;
        self.store_profile_secrets(id, password.as_deref(), key_passphrase.as_deref())?;
        Ok(profile)
    }

    pub fn delete_ssh_profile(&self, id: &str) -> Result<(), ServiceError> {
        self.config.delete_ssh_profile(id)?;
        self.credentials.delete_all_credentials(id)?;
        Ok(())
    }

    pub fn connections_using_ssh_profile(
        &self,
        profile_id: &str,
    ) -> Result<Vec<String>, ServiceError> {
        Ok(self.config.connections_using_profile(profile_id)?)
    }

    fn store_profile_secrets(
        &self,
        id: &str,
        password: Option<&str>,
        key_passphrase: Option<&str>,
    ) -> Result<(), ServiceError> {
        if let Some(pw) = password {
            if !pw.is_empty() {
                self.credentials.store_credential(id, "password", pw)?;
            }
        }
        if let Some(pp) = key_passphrase {
            if !pp.is_empty() {
                self.credentials.store_credential(id, "passphrase", pp)?;
            }
        }
        Ok(())
    }

    /// Create (or claim) a standalone tunnel for `profile_id` + target.
    pub async fn create_ssh_tunnel(
        &self,
        request: SshTunnelRequest,
    ) -> Result<SshTunnelInfo, ServiceError> {
        let key = TunnelKey::new(
            &request.profile_id,
            &request.target_host,
            request.target_port,
        );

        if let Some(mut entry) = self.tunnels.get_mut(&key) {
            entry.add_claim(TunnelClaim::Standalone);
            return Ok(SshTunnelInfo::from(&entry.handle));
        }

        let auth_config = match request.auth_method.as_str() {
            "key" => {
                let key_path = request.key_path.clone().unwrap_or_default();
                if let Some(passphrase) = &request.key_passphrase {
                    SshAuthConfig::with_key_and_passphrase(&request.username, key_path, passphrase)
                } else {
                    SshAuthConfig::with_key(&request.username, key_path)
                }
            }
            "password" => {
                let password = request.password.clone().unwrap_or_default();
                SshAuthConfig::with_password(&request.username, password)
            }
            "agent" => SshAuthConfig::with_agent(&request.username),
            other => {
                return Err(ServiceError::app(
                    "UNSUPPORTED_AUTH_METHOD",
                    format!("Unsupported auth method: {other}"),
                ));
            }
        };

        let ssh_config = SshHostConfig::new(&request.host, request.port, auth_config.clone());
        // Standalone create historically passed no jump hosts (UI path).
        let tunnel = SshTunnel::create(
            request.profile_id.clone(),
            &ssh_config,
            auth_config,
            request.target_host.clone(),
            request.target_port,
            vec![],
        )
        .await?;
        SshTunnel::start_forwarding(&tunnel).await?;

        // Same race as connect-path acquire: prefer an existing entry if one appeared.
        match self.tunnels.entry(key) {
            dashmap::mapref::entry::Entry::Occupied(mut occ) => {
                occ.get_mut().add_claim(TunnelClaim::Standalone);
                let info = SshTunnelInfo::from(&occ.get().handle);
                drop(occ);
                SshTunnel::close(tunnel).await.ok();
                Ok(info)
            }
            dashmap::mapref::entry::Entry::Vacant(vacant) => {
                let info = SshTunnelInfo::from(&tunnel);
                let mut managed = ManagedTunnel::new(tunnel);
                managed.add_claim(TunnelClaim::Standalone);
                vacant.insert(managed);
                Ok(info)
            }
        }
    }

    /// Drop the standalone claim for every tunnel under `profile_id`.
    ///
    /// Entries that still have connection users stay open. Idle entries close.
    /// For a precise single-target close, prefer [`Self::close_ssh_tunnel_for_target`].
    pub async fn close_ssh_tunnel(&self, profile_id: &str) -> Result<(), ServiceError> {
        let keys: Vec<TunnelKey> = self
            .tunnels
            .iter()
            .filter(|e| e.key().profile_id == profile_id)
            .map(|e| e.key().clone())
            .collect();

        if keys.is_empty() {
            return Err(ServiceError::app(
                "SSH_TUNNEL_NOT_FOUND",
                format!("Tunnel '{profile_id}' not found"),
            ));
        }

        for key in keys {
            self.close_tunnel_claim(&key, TunnelClaim::Standalone)
                .await?;
        }
        Ok(())
    }

    pub async fn close_ssh_tunnel_for_target(
        &self,
        profile_id: &str,
        target_host: &str,
        target_port: u16,
    ) -> Result<(), ServiceError> {
        let key = TunnelKey::new(profile_id, target_host, target_port);
        if !self.tunnels.contains_key(&key) {
            return Err(ServiceError::app(
                "SSH_TUNNEL_NOT_FOUND",
                format!("Tunnel '{profile_id}' -> {target_host}:{target_port} not found"),
            ));
        }
        self.close_tunnel_claim(&key, TunnelClaim::Standalone).await
    }

    async fn close_tunnel_claim(
        &self,
        key: &TunnelKey,
        claim: TunnelClaim,
    ) -> Result<(), ServiceError> {
        let should_close = {
            let mut entry = match self.tunnels.get_mut(key) {
                Some(e) => e,
                None => return Ok(()),
            };
            entry.remove_claim(&claim)
        };
        if should_close {
            if let Some((_, managed)) = self.tunnels.remove(key) {
                SshTunnel::close(managed.handle).await.ok();
            }
        }
        Ok(())
    }

    pub fn get_active_tunnels(&self) -> Vec<SshTunnelInfo> {
        self.tunnels
            .iter()
            .map(|entry| SshTunnelInfo::from(&entry.handle))
            .collect()
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
