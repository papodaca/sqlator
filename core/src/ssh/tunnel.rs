use crate::ssh::auth::{AuthMethod, SshAuthConfig, SshHostConfig};
use crate::ssh::error::{SshError, SshResult};
use russh::*;
use russh::keys::*;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

pub type SessionHandle = Arc<Mutex<client::Handle<Client>>>;

pub struct TunnelHandle {
    pub profile_id: String,
    pub local_port: u16,
    pub target_host: String,
    pub target_port: u16,
    pub session: SessionHandle,
    pub cancel_token: CancellationToken,
}

pub struct SshTunnel;

impl SshTunnel {
    pub async fn create(
        profile_id: String,
        ssh_config: &SshHostConfig,
        auth_config: SshAuthConfig,
        target_host: String,
        target_port: u16,
        jump_hosts: Vec<(SshHostConfig, SshAuthConfig)>,
    ) -> SshResult<TunnelHandle> {
        let cancel_token = CancellationToken::new();

        info!(
            "SSH tunnel: connecting to {}:{} (jump_hosts={}), target={}:{}",
            ssh_config.host,
            ssh_config.port,
            jump_hosts.len(),
            target_host,
            target_port
        );

        let session = if jump_hosts.is_empty() {
            debug!("SSH tunnel: direct connection to {}:{}", ssh_config.host, ssh_config.port);
            Self::connect_direct(ssh_config, auth_config).await?
        } else {
            debug!(
                "SSH tunnel: connecting via {} jump host(s)",
                jump_hosts.len()
            );
            Self::connect_via_jump(ssh_config, auth_config, &jump_hosts).await?
        };

        info!("SSH tunnel: SSH session established");

        let local_port = Self::find_available_port()?;

        let tunnel_handle = TunnelHandle {
            profile_id,
            local_port,
            target_host: target_host.clone(),
            target_port,
            session: Arc::new(Mutex::new(session)),
            cancel_token,
        };

        Ok(tunnel_handle)
    }

    pub async fn start_forwarding(tunnel: &TunnelHandle) -> SshResult<()> {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, tunnel.local_port))
            .await
            .map_err(|e| SshError::PortBindFailed(e.to_string()))?;

        let session = tunnel.session.clone();
        let target_host = tunnel.target_host.clone();
        let target_port = tunnel.target_port;
        let cancel_token = tunnel.cancel_token.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        debug!("SSH tunnel: forwarding cancelled");
                        break;
                    }
                    result = listener.accept() => {
                        match result {
                            Ok((stream, addr)) => {
                                debug!(
                                    "SSH tunnel: accepted connection from {} -> opening channel to {}:{}",
                                    addr, target_host, target_port
                                );
                                let session = session.lock().await;
                                match session
                                    .channel_open_direct_tcpip(&target_host, target_port.into(), "127.0.0.1", 0)
                                    .await
                                {
                                    Ok(channel) => {
                                        debug!("SSH tunnel: direct-tcpip channel opened, forwarding stream");
                                        drop(session);
                                        tokio::spawn(Self::forward_stream(stream, channel));
                                    }
                                    Err(e) => {
                                        error!(
                                            "SSH tunnel: failed to open direct-tcpip channel to {}:{}: {}",
                                            target_host, target_port, e
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                warn!("SSH tunnel: failed to accept connection: {}", e);
                            }
                        }
                    }
                }
            }
        });

        info!(
            "SSH tunnel listening on localhost:{} -> {}:{}",
            tunnel.local_port, tunnel.target_host, tunnel.target_port
        );

        Ok(())
    }

    async fn forward_stream<S>(stream: S, channel: Channel<client::Msg>)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (mut stream_reader, mut stream_writer) = tokio::io::split(stream);
        let (to_ssh_tx, mut to_ssh_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(16);
        let (from_ssh_tx, mut from_ssh_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(16);

        // Single task owns the SSH channel and handles both directions with select!,
        // avoiding the deadlock that occurs when holding a mutex across ch.wait().
        tokio::spawn(async move {
            let mut channel = channel;
            loop {
                tokio::select! {
                    msg = to_ssh_rx.recv() => {
                        match msg {
                            Some(data) => {
                                if channel.data(data.as_ref()).await.is_err() {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                    msg = channel.wait() => {
                        match msg {
                            Some(ChannelMsg::Data { ref data }) => {
                                if from_ssh_tx.send(data.to_vec()).await.is_err() {
                                    break;
                                }
                            }
                            Some(ChannelMsg::Eof) | None => break,
                            _ => {}
                        }
                    }
                }
            }
        });

        // Read from local TCP stream, forward to SSH channel task.
        tokio::spawn(async move {
            let mut buf = vec![0u8; 8192];
            loop {
                match stream_reader.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if to_ssh_tx.send(buf[..n].to_vec()).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });

        // Write data arriving from SSH channel to local TCP stream.
        tokio::spawn(async move {
            loop {
                match from_ssh_rx.recv().await {
                    Some(data) => {
                        if stream_writer.write_all(&data).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
        });
    }

    async fn connect_direct(
        config: &SshHostConfig,
        auth_config: SshAuthConfig,
    ) -> SshResult<client::Handle<Client>> {
        let ssh_config = client::Config::default();
        let config_arc = Arc::new(ssh_config);

        debug!(
            "SSH tunnel: TCP connect to {}:{}",
            config.host, config.port
        );
        let mut session = client::connect(
            config_arc,
            (config.host.as_str(), config.port),
            Client {},
        )
        .await
        .map_err(|e| {
            error!(
                "SSH tunnel: TCP connect failed to {}:{}: {}",
                config.host, config.port, e
            );
            SshError::ConnectionFailed(e.to_string())
        })?;

        debug!("SSH tunnel: authenticating as '{}'", auth_config.username);
        Self::authenticate(&mut session, &auth_config).await?;
        debug!("SSH tunnel: authenticated");

        Ok(session)
    }

    async fn connect_via_jump(
        target_config: &SshHostConfig,
        target_auth: SshAuthConfig,
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
        let mut current_session = Self::connect_direct(first_host, first_auth.clone()).await?;

        for (i, (jump_config, jump_auth)) in jump_hosts.iter().skip(1).enumerate() {
            debug!(
                "SSH tunnel: connecting through jump host {} of {} ({}:{})",
                i + 2,
                jump_hosts.len(),
                jump_config.host,
                jump_config.port
            );
            current_session =
                Self::connect_through_jump(&current_session, jump_config, jump_auth.clone()).await?;
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
        auth_config: SshAuthConfig,
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
        Self::authenticate(&mut session, &auth_config).await?;
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
                debug!("SSH tunnel: authenticating user '{}' with password", username);
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
            error!("SSH tunnel: authentication rejected for user '{}'", username);
            return Err(SshError::AuthFailed("Authentication rejected".into()));
        }

        Ok(())
    }

    fn load_key_pair(path: &Path, _passphrase: Option<&str>) -> SshResult<PrivateKey> {
        let key_data = std::fs::read(path).map_err(|e| SshError::KeyLoadFailed(e.to_string()))?;

        let key_pair =
            PrivateKey::from_openssh(&key_data).map_err(|e| SshError::KeyLoadFailed(e.to_string()))?;

        Ok(key_pair)
    }

    fn find_available_port() -> SshResult<u16> {
        use std::net::TcpListener as StdTcpListener;

        let listener =
            StdTcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
                .map_err(|e| SshError::PortBindFailed(e.to_string()))?;

        let port = listener
            .local_addr()
            .map_err(|e| SshError::PortBindFailed(e.to_string()))?
            .port();

        drop(listener);
        Ok(port)
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
}

#[derive(Debug)]
struct Client;

impl client::Handler for Client {
    type Error = russh::Error;

    fn check_server_key(
        &mut self,
        _server_public_key: &ssh_key::PublicKey,
    ) -> impl std::future::Future<Output = Result<bool, Self::Error>> + Send {
        std::future::ready(Ok(true))
    }
}
