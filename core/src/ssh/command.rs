use crate::ssh::auth::{AuthMethod, SshAuthConfig, SshHostConfig};
use crate::ssh::error::{SshError, SshResult};
use russh::*;
use russh::keys::*;
use std::sync::Arc;
use tracing::{debug, error};

pub struct SshCommandResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<u32>,
}

pub struct SshCommand;

impl SshCommand {
    pub async fn exec(
        ssh_config: &SshHostConfig,
        auth_config: SshAuthConfig,
        command: &str,
        timeout: std::time::Duration,
    ) -> SshResult<SshCommandResult> {
        debug!("SSH command: connecting to {}:{}", ssh_config.host, ssh_config.port);
        let mut session = Self::connect(ssh_config, auth_config).await?;
        debug!("SSH command: authenticated, executing: {}", command);

        let result = tokio::time::timeout(timeout, Self::exec_on_session(&mut session, command))
            .await
            .map_err(|_| {
                error!("SSH command timed out after {}s", timeout.as_secs());
                SshError::Other(format!("Command timed out after {}s", timeout.as_secs()))
            })?;

        let _ = session
            .disconnect(Disconnect::ByApplication, "Done", "en")
            .await;

        result
    }

    pub async fn exec_via_jump(
        target_config: &SshHostConfig,
        target_auth: SshAuthConfig,
        jump_hosts: &[(SshHostConfig, SshAuthConfig)],
        command: &str,
        timeout: std::time::Duration,
    ) -> SshResult<SshCommandResult> {
        if jump_hosts.is_empty() {
            return Self::exec(target_config, target_auth, command, timeout).await;
        }

        let (first_host, first_auth) = &jump_hosts[0];
        let mut current_session = Self::connect(first_host, first_auth.clone()).await?;

        for (jump_config, jump_auth) in jump_hosts.iter().skip(1) {
            current_session =
                Self::connect_through_jump(&current_session, jump_config, jump_auth.clone()).await?;
        }

        let mut session =
            Self::connect_through_jump(&current_session, target_config, target_auth).await?;

        let result = tokio::time::timeout(timeout, Self::exec_on_session(&mut session, command))
            .await
            .map_err(|_| {
                error!("SSH command timed out after {}s", timeout.as_secs());
                SshError::Other(format!("Command timed out after {}s", timeout.as_secs()))
            })?;

        let _ = session
            .disconnect(Disconnect::ByApplication, "Done", "en")
            .await;

        result
    }

    async fn exec_on_session(
        session: &mut client::Handle<CommandClient>,
        command: &str,
    ) -> SshResult<SshCommandResult> {
        let mut channel = session
            .channel_open_session()
            .await
            .map_err(|e| {
                error!("SSH command: failed to open session channel: {}", e);
                SshError::Other(format!("Failed to open session channel: {}", e))
            })?;

        channel
            .exec(true, command)
            .await
            .map_err(|e| {
                error!("SSH command: failed to exec: {}", e);
                SshError::Other(format!("Failed to execute command: {}", e))
            })?;

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_code: Option<u32> = None;

        loop {
            match channel.wait().await {
                Some(ChannelMsg::Data { ref data }) => {
                    stdout.extend_from_slice(data);
                }
                Some(ChannelMsg::ExtendedData { ref data, ext }) => {
                    if ext == 1 {
                        stderr.extend_from_slice(data);
                    }
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    exit_code = Some(exit_status);
                }
                Some(ChannelMsg::Eof) | None => {
                    break;
                }
                _ => {}
            }
        }

        Ok(SshCommandResult {
            stdout: String::from_utf8_lossy(&stdout).to_string(),
            stderr: String::from_utf8_lossy(&stderr).to_string(),
            exit_code,
        })
    }

    async fn connect(
        config: &SshHostConfig,
        auth_config: SshAuthConfig,
    ) -> SshResult<client::Handle<CommandClient>> {
        let ssh_config = client::Config::default();
        let config_arc = Arc::new(ssh_config);

        let mut session = client::connect(
            config_arc,
            (config.host.as_str(), config.port),
            CommandClient {},
        )
        .await
        .map_err(|e| {
            error!("SSH command: connect failed to {}:{}: {}", config.host, config.port, e);
            SshError::ConnectionFailed(e.to_string())
        })?;

        Self::authenticate(&mut session, &auth_config).await?;
        Ok(session)
    }

    async fn connect_through_jump(
        jump_session: &client::Handle<CommandClient>,
        target_config: &SshHostConfig,
        auth_config: SshAuthConfig,
    ) -> SshResult<client::Handle<CommandClient>> {
        let channel = jump_session
            .channel_open_direct_tcpip(
                target_config.host.as_str(),
                target_config.port.into(),
                "127.0.0.1",
                0,
            )
            .await
            .map_err(|e| {
                error!(
                    "SSH command: failed to open direct-tcpip to {}:{}: {}",
                    target_config.host, target_config.port, e
                );
                SshError::JumpHostFailed(e.to_string())
            })?;

        let ssh_config = client::Config::default();
        let config_arc = Arc::new(ssh_config);

        let mut session = client::connect_stream(config_arc, channel.into_stream(), CommandClient {})
            .await
            .map_err(|e| {
                error!(
                    "SSH command: connect_stream failed for {}:{}: {}",
                    target_config.host, target_config.port, e
                );
                SshError::JumpHostFailed(e.to_string())
            })?;

        Self::authenticate(&mut session, &auth_config).await?;
        Ok(session)
    }

    async fn authenticate(
        session: &mut client::Handle<CommandClient>,
        auth_config: &SshAuthConfig,
    ) -> SshResult<()> {
        let username = &auth_config.username;

        let success = match &auth_config.method {
            AuthMethod::Key => {
                let key_path = auth_config
                    .key_path
                    .as_ref()
                    .ok_or_else(|| SshError::AuthFailed("Key path not provided".into()))?;

                let key_data = std::fs::read(key_path)
                    .map_err(|e| SshError::KeyLoadFailed(e.to_string()))?;
                let key_pair = PrivateKey::from_openssh(&key_data)
                    .map_err(|e| SshError::KeyLoadFailed(e.to_string()))?;

                let key_with_hash = keys::PrivateKeyWithHashAlg::new(Arc::new(key_pair), None);

                session
                    .authenticate_publickey(username, key_with_hash)
                    .await
                    .map_err(|e| SshError::AuthFailed(e.to_string()))?
                    .success()
            }
            AuthMethod::Password => {
                let password = auth_config
                    .password
                    .as_ref()
                    .ok_or_else(|| SshError::AuthFailed("Password not provided".into()))?;

                session
                    .authenticate_password(username, password)
                    .await
                    .map_err(|e| SshError::AuthFailed(e.to_string()))?
                    .success()
            }
            AuthMethod::Agent => {
                return Err(SshError::AuthFailed(
                    "SSH agent authentication not yet implemented".into(),
                ));
            }
        };

        if !success {
            return Err(SshError::AuthFailed("Authentication rejected".into()));
        }

        Ok(())
    }
}

#[derive(Debug)]
struct CommandClient;

impl client::Handler for CommandClient {
    type Error = russh::Error;

    fn check_server_key(
        &mut self,
        _server_public_key: &ssh_key::PublicKey,
    ) -> impl std::future::Future<Output = Result<bool, Self::Error>> + Send {
        std::future::ready(Ok(true))
    }
}
