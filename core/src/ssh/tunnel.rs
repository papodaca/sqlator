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
use tracing::{debug, info};

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

        let session = if jump_hosts.is_empty() {
            Self::connect_direct(ssh_config, auth_config).await?
        } else {
            Self::connect_via_jump(ssh_config, auth_config, &jump_hosts).await?
        };

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
                        debug!("Tunnel forwarding cancelled");
                        break;
                    }
                    result = listener.accept() => {
                        match result {
                            Ok((stream, _addr)) => {
                                let session = session.lock().await;
                                match session
                                    .channel_open_direct_tcpip(&target_host, target_port.into(), "127.0.0.1", 0)
                                    .await
                                {
                                    Ok(channel) => {
                                        drop(session);
                                        tokio::spawn(Self::forward_stream(stream, channel));
                                    }
                                    Err(e) => {
                                        debug!("Failed to open direct-tcpip channel: {}", e);
                                    }
                                }
                            }
                            Err(e) => {
                                debug!("Failed to accept connection: {}", e);
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
        let channel = Arc::new(Mutex::new(channel));
        let (mut stream_reader, mut stream_writer) = tokio::io::split(stream);
        let channel_clone = channel.clone();

        let reader_handle = tokio::spawn(async move {
            let mut buf = vec![0u8; 8192];
            loop {
                match stream_reader.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        let ch = channel_clone.lock().await;
                        if ch.data(&buf[..n]).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        let writer_handle = tokio::spawn(async move {
            loop {
                let mut ch = channel.lock().await;
                let Some(msg) = ch.wait().await else {
                    break;
                };
                drop(ch);

                match msg {
                    ChannelMsg::Data { ref data } => {
                        if stream_writer.write_all(data).await.is_err() {
                            break;
                        }
                    }
                    ChannelMsg::Eof => break,
                    _ => {}
                }
            }
        });

        let _ = tokio::try_join!(reader_handle, writer_handle);
    }

    async fn connect_direct(
        config: &SshHostConfig,
        auth_config: SshAuthConfig,
    ) -> SshResult<client::Handle<Client>> {
        let ssh_config = client::Config::default();
        let config_arc = Arc::new(ssh_config);

        let mut session = client::connect(
            config_arc,
            (config.host.as_str(), config.port),
            Client {},
        )
        .await
        .map_err(|e| SshError::ConnectionFailed(e.to_string()))?;

        Self::authenticate(&mut session, &auth_config).await?;

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
        let mut current_session = Self::connect_direct(first_host, first_auth.clone()).await?;

        for (i, (jump_config, jump_auth)) in jump_hosts.iter().skip(1).enumerate() {
            debug!("Connecting to jump host {} of {}", i + 2, jump_hosts.len());
            current_session =
                Self::connect_through_jump(&current_session, jump_config, jump_auth.clone()).await?;
        }

        let final_session =
            Self::connect_through_jump(&current_session, target_config, target_auth).await?;
        Ok(final_session)
    }

    async fn connect_through_jump(
        jump_session: &client::Handle<Client>,
        target_config: &SshHostConfig,
        auth_config: SshAuthConfig,
    ) -> SshResult<client::Handle<Client>> {
        let channel = jump_session
            .channel_open_direct_tcpip(
                target_config.host.as_str(),
                target_config.port.into(),
                "127.0.0.1",
                0,
            )
            .await
            .map_err(|e| SshError::JumpHostFailed(e.to_string()))?;

        let ssh_config = client::Config::default();
        let config_arc = Arc::new(ssh_config);

        let mut session = client::connect_stream(config_arc, channel.into_stream(), Client {})
            .await
            .map_err(|e| SshError::JumpHostFailed(e.to_string()))?;

        Self::authenticate(&mut session, &auth_config).await?;

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

                let key_pair =
                    Self::load_key_pair(key_path, auth_config.key_passphrase.as_deref())?;

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
                // TODO: Implement proper SSH agent authentication
                // The russh API for agent auth requires using authenticate_future
                // which needs the agent client to implement a specific trait
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
