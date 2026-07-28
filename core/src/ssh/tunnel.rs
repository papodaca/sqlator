use crate::ssh::auth::{AuthMethod, SshAuthConfig, SshHostConfig};
use crate::ssh::error::{SshError, SshResult};
use russh::keys::*;
use russh::*;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::Path;
use std::sync::Arc;
use tokio::io::{copy_bidirectional, AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

pub type SessionHandle = Arc<Mutex<client::Handle<Client>>>;

/// Authenticated SSH session. One session can own many [`LocalForward`]s.
#[derive(Clone)]
pub struct SshSession {
    pub profile_id: String,
    pub(crate) handle: SessionHandle,
}

impl SshSession {
    pub(crate) fn handle(&self) -> SessionHandle {
        Arc::clone(&self.handle)
    }
}

/// A localhost TCP listener that forwards through an [`SshSession`] to one remote target.
pub struct LocalForward {
    pub local_port: u16,
    pub target_host: String,
    pub target_port: u16,
    /// Cancels only this listener; does not disconnect the SSH session.
    pub cancel_token: CancellationToken,
}

/// Compatibility façade: one session + one forward (historical 1:1 shape).
///
/// Prefer [`SshTunnel::connect_session`] + [`SshTunnel::open_forward`] when sharing
/// a session across multiple targets.
pub struct TunnelHandle {
    pub profile_id: String,
    pub local_port: u16,
    pub target_host: String,
    pub target_port: u16,
    pub(crate) session: SessionHandle,
    pub cancel_token: CancellationToken,
}

pub struct SshTunnel;

impl SshTunnel {
    /// Authenticate an SSH session (direct or via jump hosts). No local forward yet.
    pub async fn connect_session(
        profile_id: String,
        ssh_config: &SshHostConfig,
        auth_config: &SshAuthConfig,
        jump_hosts: &[(SshHostConfig, SshAuthConfig)],
    ) -> SshResult<SshSession> {
        info!(
            "SSH session: connecting to {}:{} (jump_hosts={})",
            ssh_config.host,
            ssh_config.port,
            jump_hosts.len()
        );

        let session = if jump_hosts.is_empty() {
            debug!(
                "SSH session: direct connection to {}:{}",
                ssh_config.host, ssh_config.port
            );
            Self::connect_direct(ssh_config, auth_config).await?
        } else {
            debug!(
                "SSH session: connecting via {} jump host(s)",
                jump_hosts.len()
            );
            Self::connect_via_jump(ssh_config, auth_config, jump_hosts).await?
        };

        info!("SSH session established for profile '{}'", profile_id);

        Ok(SshSession {
            profile_id,
            handle: Arc::new(Mutex::new(session)),
        })
    }

    /// Allocate a local port for a new forward (listener not started yet).
    pub fn open_forward(target_host: String, target_port: u16) -> SshResult<LocalForward> {
        let local_port = Self::find_available_port()?;
        Ok(LocalForward {
            local_port,
            target_host,
            target_port,
            cancel_token: CancellationToken::new(),
        })
    }

    /// Start accepting connections on `forward` and open `direct-tcpip` channels
    /// through `session`.
    pub async fn start_local_forward(
        session: &SshSession,
        forward: &LocalForward,
    ) -> SshResult<()> {
        let listener =
            TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, forward.local_port))
                .await
                .map_err(|e| SshError::PortBindFailed(e.to_string()))?;

        let session = session.handle();
        let target_host = forward.target_host.clone();
        let target_port = forward.target_port;
        let cancel_token = forward.cancel_token.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        debug!("SSH forward: cancelled for {}:{}", target_host, target_port);
                        break;
                    }
                    result = listener.accept() => {
                        match result {
                            Ok((stream, addr)) => {
                                debug!(
                                    "SSH forward: accepted connection from {} -> {}:{}",
                                    addr, target_host, target_port
                                );
                                let session = session.lock().await;
                                match session
                                    .channel_open_direct_tcpip(
                                        &target_host,
                                        target_port.into(),
                                        "127.0.0.1",
                                        0,
                                    )
                                    .await
                                {
                                    Ok(channel) => {
                                        debug!("SSH forward: direct-tcpip channel opened");
                                        drop(session);
                                        tokio::spawn(Self::forward_stream(stream, channel));
                                    }
                                    Err(e) => {
                                        error!(
                                            "SSH forward: failed to open direct-tcpip to {}:{}: {}",
                                            target_host, target_port, e
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                warn!("SSH forward: failed to accept connection: {}", e);
                            }
                        }
                    }
                }
            }
        });

        info!(
            "SSH forward listening on localhost:{} -> {}:{}",
            forward.local_port, forward.target_host, forward.target_port
        );

        Ok(())
    }

    /// Cancel a single forward's listener without disconnecting the session.
    pub fn close_forward(forward: LocalForward) {
        forward.cancel_token.cancel();
        info!(
            "SSH forward closed localhost:{} -> {}:{}",
            forward.local_port, forward.target_host, forward.target_port
        );
    }

    /// Disconnect an SSH session (call only after all forwards are closed).
    pub async fn close_session(session: SshSession) -> SshResult<()> {
        let handle = session.handle.lock().await;
        handle
            .disconnect(Disconnect::ByApplication, "Connection closed", "en")
            .await
            .map_err(|e| SshError::Other(e.to_string()))?;

        info!("SSH session closed for profile: {}", session.profile_id);
        Ok(())
    }

    /// Create a dedicated session + single forward (compatibility façade).
    pub async fn create(
        profile_id: String,
        ssh_config: &SshHostConfig,
        auth_config: &SshAuthConfig,
        target_host: String,
        target_port: u16,
        jump_hosts: &[(SshHostConfig, SshAuthConfig)],
    ) -> SshResult<TunnelHandle> {
        info!(
            "SSH tunnel: connecting to {}:{} (jump_hosts={}), target={}:{}",
            ssh_config.host,
            ssh_config.port,
            jump_hosts.len(),
            target_host,
            target_port
        );

        let session =
            Self::connect_session(profile_id.clone(), ssh_config, auth_config, jump_hosts).await?;
        let forward = Self::open_forward(target_host, target_port)?;

        Ok(TunnelHandle {
            profile_id,
            local_port: forward.local_port,
            target_host: forward.target_host,
            target_port: forward.target_port,
            session: session.handle,
            cancel_token: forward.cancel_token,
        })
    }

    pub async fn start_forwarding(tunnel: &TunnelHandle) -> SshResult<()> {
        let session = SshSession {
            profile_id: tunnel.profile_id.clone(),
            handle: Arc::clone(&tunnel.session),
        };
        let forward = LocalForward {
            local_port: tunnel.local_port,
            target_host: tunnel.target_host.clone(),
            target_port: tunnel.target_port,
            cancel_token: tunnel.cancel_token.clone(),
        };
        Self::start_local_forward(&session, &forward).await
    }

    pub async fn close(tunnel: TunnelHandle) -> SshResult<()> {
        tunnel.cancel_token.cancel();

        let session = tunnel.session.lock().await;
        session
            .disconnect(Disconnect::ByApplication, "Connection closed", "en")
            .await
            .map_err(|e| SshError::Other(e.to_string()))?;

        info!("SSH tunnel closed for profile: {}", tunnel.profile_id);
        Ok(())
    }

    async fn forward_stream<S>(mut stream: S, channel: Channel<client::Msg>)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        // `into_stream` + `copy_bidirectional` keeps channel read/write polled together.
        // The previous mpsc/`channel.data()`/`wait()` split could stall the russh session
        // loop when `wait()` was not drained while `data()` awaited a window adjust.
        tokio::spawn(async move {
            let mut ssh_stream = channel.into_stream();
            match copy_bidirectional(&mut stream, &mut ssh_stream).await {
                Ok((to_ssh, from_ssh)) => {
                    debug!("SSH forward: stream closed (to_ssh={to_ssh}B, from_ssh={from_ssh}B)");
                }
                Err(e) => {
                    debug!("SSH forward: stream error: {e}");
                }
            }
        });
    }

    async fn connect_direct(
        config: &SshHostConfig,
        auth_config: &SshAuthConfig,
    ) -> SshResult<client::Handle<Client>> {
        let ssh_config = client::Config::default();
        let config_arc = Arc::new(ssh_config);

        debug!("SSH tunnel: TCP connect to {}:{}", config.host, config.port);
        let mut session =
            client::connect(config_arc, (config.host.as_str(), config.port), Client {})
                .await
                .map_err(|e| {
                    error!(
                        "SSH tunnel: TCP connect failed to {}:{}: {}",
                        config.host, config.port, e
                    );
                    SshError::ConnectionFailed(e.to_string())
                })?;

        debug!("SSH tunnel: authenticating as '{}'", auth_config.username);
        Self::authenticate(&mut session, auth_config).await?;
        debug!("SSH tunnel: authenticated");

        Ok(session)
    }

    async fn connect_via_jump(
        target_config: &SshHostConfig,
        target_auth: &SshAuthConfig,
        jump_hosts: &[(SshHostConfig, SshAuthConfig)],
    ) -> SshResult<client::Handle<Client>> {
        if jump_hosts.is_empty() {
            return Self::connect_direct(target_config, target_auth).await;
        }

        let (first_host, first_auth) = &jump_hosts[0];
        debug!(
            "SSH tunnel: connecting to first jump host {}:{}",
            first_host.host, first_host.port
        );
        let mut current_session = Self::connect_direct(first_host, first_auth).await?;

        for (i, (jump_config, jump_auth)) in jump_hosts.iter().skip(1).enumerate() {
            debug!(
                "SSH tunnel: connecting through jump host {} of {} ({}:{})",
                i + 2,
                jump_hosts.len(),
                jump_config.host,
                jump_config.port
            );
            current_session =
                Self::connect_through_jump(&current_session, jump_config, jump_auth).await?;
        }

        debug!(
            "SSH tunnel: final hop to target SSH host {}:{}",
            target_config.host, target_config.port
        );
        let final_session =
            Self::connect_through_jump(&current_session, target_config, target_auth).await?;
        Ok(final_session)
    }

    async fn connect_through_jump(
        jump_session: &client::Handle<Client>,
        target_config: &SshHostConfig,
        auth_config: &SshAuthConfig,
    ) -> SshResult<client::Handle<Client>> {
        debug!(
            "SSH tunnel: opening direct-tcpip channel to {}:{}",
            target_config.host, target_config.port
        );
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
                    "SSH tunnel: failed to open direct-tcpip channel to {}:{}: {}",
                    target_config.host, target_config.port, e
                );
                SshError::JumpHostFailed(e.to_string())
            })?;

        let ssh_config = client::Config::default();
        let config_arc = Arc::new(ssh_config);

        let mut session = client::connect_stream(config_arc, channel.into_stream(), Client {})
            .await
            .map_err(|e| {
                error!(
                    "SSH tunnel: connect_stream failed for {}:{}: {}",
                    target_config.host, target_config.port, e
                );
                SshError::JumpHostFailed(e.to_string())
            })?;

        debug!(
            "SSH tunnel: authenticating through jump as '{}'",
            auth_config.username
        );
        Self::authenticate(&mut session, auth_config).await?;
        debug!(
            "SSH tunnel: authenticated through jump to {}:{}",
            target_config.host, target_config.port
        );

        Ok(session)
    }

    async fn authenticate(
        session: &mut client::Handle<Client>,
        auth_config: &SshAuthConfig,
    ) -> SshResult<()> {
        let username = &auth_config.username;

        let success = match &auth_config.method {
            AuthMethod::Key => {
                let key_path = auth_config
                    .key_path
                    .as_ref()
                    .ok_or_else(|| SshError::AuthFailed("Key path not provided".into()))?;

                debug!(
                    "SSH tunnel: authenticating user '{}' with key '{}'",
                    username,
                    key_path.display()
                );

                let key_pair =
                    Self::load_key_pair(key_path, auth_config.key_passphrase.as_deref())?;

                let key_with_hash = keys::PrivateKeyWithHashAlg::new(Arc::new(key_pair), None);

                session
                    .authenticate_publickey(username, key_with_hash)
                    .await
                    .map_err(|e| {
                        error!("SSH tunnel: publickey auth error for '{}': {}", username, e);
                        SshError::AuthFailed(e.to_string())
                    })?
                    .success()
            }
            AuthMethod::Password => {
                debug!(
                    "SSH tunnel: authenticating user '{}' with password",
                    username
                );
                let password = auth_config
                    .password
                    .as_ref()
                    .ok_or_else(|| SshError::AuthFailed("Password not provided".into()))?;

                session
                    .authenticate_password(username, password)
                    .await
                    .map_err(|e| {
                        error!("SSH tunnel: password auth error for '{}': {}", username, e);
                        SshError::AuthFailed(e.to_string())
                    })?
                    .success()
            }
            AuthMethod::Agent => {
                return Err(SshError::AuthFailed(
                    "SSH agent authentication not yet implemented".into(),
                ));
            }
        };

        if !success {
            error!(
                "SSH tunnel: authentication rejected for user '{}'",
                username
            );
            return Err(SshError::AuthFailed("Authentication rejected".into()));
        }

        Ok(())
    }

    fn load_key_pair(path: &Path, _passphrase: Option<&str>) -> SshResult<PrivateKey> {
        let key_data = std::fs::read(path).map_err(|e| SshError::KeyLoadFailed(e.to_string()))?;

        let key_pair = PrivateKey::from_openssh(&key_data)
            .map_err(|e| SshError::KeyLoadFailed(e.to_string()))?;

        Ok(key_pair)
    }

    fn find_available_port() -> SshResult<u16> {
        use std::net::TcpListener as StdTcpListener;

        let listener = StdTcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .map_err(|e| SshError::PortBindFailed(e.to_string()))?;

        let port = listener
            .local_addr()
            .map_err(|e| SshError::PortBindFailed(e.to_string()))?
            .port();

        drop(listener);
        Ok(port)
    }
}

#[derive(Debug)]
pub(crate) struct Client;

impl client::Handler for Client {
    type Error = russh::Error;

    fn check_server_key(
        &mut self,
        _server_public_key: &ssh_key::PublicKey,
    ) -> impl std::future::Future<Output = Result<bool, Self::Error>> + Send {
        std::future::ready(Ok(true))
    }
}
