//! SSH profile helpers, profile CRUD, and standalone tunnel registry ops.
//!
//! Auth/jump builders live here; connect-path tunnel acquire/release is in
//! [`crate::connect`]. Sessions are shared per profile; forwards use
//! [`crate::tunnels::TunnelKey`].

use crate::error::ServiceError;
use crate::service::AppService;
use crate::tunnels::{SshTunnelInfo, TunnelClaim, TunnelKey};
use serde::{Deserialize, Serialize};
use sqlator_core::credentials::CredentialStore;
use sqlator_core::models::{SshAuthMethod, SshJumpHost, SshProfile};
use sqlator_core::ssh::{config_parser, SshAuthConfig, SshHostConfig, SshTunnel};

pub use sqlator_core::ssh::HostEntry;

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
            // Metadata only — secrets stay in `auth` (no Clone on SshAuthConfig).
            let config = SshHostConfig::new(&jump.host, jump.port, &auth);
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
    /// Parse `~/.ssh/config` host aliases for the UI host picker.
    pub fn list_ssh_hosts(&self) -> Result<Vec<HostEntry>, ServiceError> {
        Ok(config_parser::load_ssh_config()?)
    }

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
    ///
    /// Reuses the shared SSH session for the profile when one is already live.
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
            return Ok(SshTunnelInfo::from_forward(
                &request.profile_id,
                &entry.forward,
            ));
        }

        // Prefer the saved profile path (jumps + stored secrets) when present;
        // otherwise fall back to the request's inline host/auth (UI standalone).
        let local_port = if self.config.get_ssh_profile(&request.profile_id)?.is_some() {
            self.ensure_forward(&key, TunnelClaim::Standalone).await?
        } else {
            self.ensure_forward_from_request(&request, &key).await?
        };

        Ok(SshTunnelInfo {
            profile_id: request.profile_id,
            local_port,
            target_host: request.target_host,
            target_port: request.target_port,
        })
    }

    /// Standalone create when the profile is not in config (inline request auth).
    async fn ensure_forward_from_request(
        &self,
        request: &SshTunnelRequest,
        key: &TunnelKey,
    ) -> Result<u16, ServiceError> {
        if let Some(mut entry) = self.tunnels.get_mut(key) {
            entry.add_claim(TunnelClaim::Standalone);
            return Ok(entry.forward.local_port);
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

        let ssh_config = SshHostConfig::new(&request.host, request.port, &auth_config);

        // Session: reuse if present, else create without jump hosts (legacy UI path).
        let session_view = if let Some(existing) = self.sessions.get(&request.profile_id) {
            existing.session.clone()
        } else {
            let new_session = SshTunnel::connect_session(
                request.profile_id.clone(),
                &ssh_config,
                &auth_config,
                &[],
            )
            .await?;
            match self.sessions.entry(request.profile_id.clone()) {
                dashmap::mapref::entry::Entry::Occupied(occ) => {
                    let view = occ.get().session.clone();
                    drop(occ);
                    SshTunnel::close_session(new_session).await.ok();
                    view
                }
                dashmap::mapref::entry::Entry::Vacant(vacant) => {
                    let view = new_session.clone();
                    vacant.insert(crate::tunnels::ManagedSession::new(new_session));
                    view
                }
            }
        };

        let forward = SshTunnel::open_forward(request.target_host.clone(), request.target_port)?;
        SshTunnel::start_local_forward(&session_view, &forward).await?;

        let local_port = match self.tunnels.entry(key.clone()) {
            dashmap::mapref::entry::Entry::Occupied(mut occ) => {
                occ.get_mut().add_claim(TunnelClaim::Standalone);
                let port = occ.get().forward.local_port;
                drop(occ);
                SshTunnel::close_forward(forward);
                port
            }
            dashmap::mapref::entry::Entry::Vacant(vacant) => {
                let port = forward.local_port;
                let mut managed = crate::tunnels::ManagedForward::new(forward);
                managed.add_claim(TunnelClaim::Standalone);
                vacant.insert(managed);
                if let Some(mut session) = self.sessions.get_mut(&key.profile_id) {
                    session.add_forward(key.clone());
                }
                port
            }
        };

        Ok(local_port)
    }

    /// Drop the standalone claim for every tunnel under `profile_id`.
    ///
    /// Entries that still have connection users stay open. Idle entries close;
    /// the shared session tears down when its last forward closes.
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
            self.release_forward_claim(&key, TunnelClaim::Standalone)
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
        self.release_forward_claim(&key, TunnelClaim::Standalone)
            .await
    }

    pub fn get_active_tunnels(&self) -> Vec<SshTunnelInfo> {
        self.tunnels
            .iter()
            .map(|entry| SshTunnelInfo::from_forward(&entry.key().profile_id, &entry.forward))
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
