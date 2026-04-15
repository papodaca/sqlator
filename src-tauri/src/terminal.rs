use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use sqlator_core::credentials::CredentialStore;
use sqlator_core::models::{ConnectionType, SavedConnection, SshAuthMethod, SshProfile};
use sqlator_core::ssh::{AuthMethod, SshAuthConfig};
use std::io::{Read, Write};
use std::sync::Mutex;
use tauri::ipc::Channel;
use tauri::State;

use crate::state::AppState;

type CmdResult<T> = Result<T, String>;
fn map_err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub struct PtyHandle {
    pub writer: Mutex<Box<dyn Write + Send>>,
    pub master: Mutex<Box<dyn portable_pty::MasterPty + Send>>,
    pub child: Mutex<Box<dyn portable_pty::Child + Send + Sync>>,
}

/// Resolved command ready to pass to CommandBuilder.
struct CliSpec {
    binary: String,
    args: Vec<String>,
    /// Environment variables set on the spawned process.
    env: Vec<(String, String)>,
}

// ── Shell escaping ────────────────────────────────────────────────────────────

/// POSIX single-quote escape — safe for interpolation in a remote shell command.
fn sh_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

// ── CLI spec builders ─────────────────────────────────────────────────────────

/// Build CLI args for direct / SSH-tunnel connections.
/// `host` and `port` are already resolved (tunnel local endpoint if applicable).
fn direct_cli_spec(conn: &SavedConnection, host: &str, port: u16) -> CmdResult<CliSpec> {
    let parsed = url::Url::parse(&conn.url).map_err(map_err)?;
    let password = parsed.password().unwrap_or("").to_string();

    match conn.db_type.as_str() {
        "postgres" => Ok(CliSpec {
            binary: "psql".to_string(),
            args: vec![
                "-U".to_string(),
                conn.username.clone(),
                "-h".to_string(),
                host.to_string(),
                "-p".to_string(),
                port.to_string(),
                conn.database.clone(),
            ],
            env: vec![("PGPASSWORD".to_string(), password)],
        }),
        "mysql" | "mariadb" => Ok(CliSpec {
            binary: "mysql".to_string(),
            args: vec![
                format!("-u{}", conn.username),
                format!("-h{}", host),
                format!("-P{}", port),
                conn.database.clone(),
            ],
            env: vec![("MYSQL_PWD".to_string(), password)],
        }),
        "sqlite" => Ok(CliSpec {
            binary: "sqlite3".to_string(),
            args: vec![conn.database.clone()],
            env: vec![],
        }),
        "oracle" => Ok(CliSpec {
            binary: "sqlplus".to_string(),
            args: vec![format!(
                "{}/{}@{}:{}/{}",
                conn.username, password, host, port, conn.database
            )],
            env: vec![],
        }),
        "mssql" => Ok(CliSpec {
            binary: "sqlcmd".to_string(),
            args: vec![
                "-S".to_string(),
                format!("{},{}", host, port),
                "-U".to_string(),
                conn.username.clone(),
                "-P".to_string(),
                password,
                "-d".to_string(),
                conn.database.clone(),
            ],
            env: vec![],
        }),
        "clickhouse" => Ok(CliSpec {
            binary: "clickhouse-client".to_string(),
            args: vec![
                "--host".to_string(),
                host.to_string(),
                "--port".to_string(),
                port.to_string(),
                "--user".to_string(),
                conn.username.clone(),
                "--password".to_string(),
                password,
                "--database".to_string(),
                conn.database.clone(),
            ],
            env: vec![],
        }),
        other => Err(format!(
            "Unsupported database type for terminal: {}",
            other
        )),
    }
}

/// Build a `docker exec -it <container> <cli> <args>` spec for local Docker.
/// Passwords are passed via docker exec `-e KEY=VALUE` to avoid host process list exposure.
fn docker_exec_cli_spec(conn: &SavedConnection) -> CmdResult<CliSpec> {
    let container = conn
        .container_name
        .as_deref()
        .ok_or("LocalDockerContainer connection is missing a container name")?;

    let parsed = url::Url::parse(&conn.url).map_err(map_err)?;
    let password = parsed.password().unwrap_or("").to_string();

    let (cli_binary, mut cli_args, env_kv): (&str, Vec<String>, Option<(String, String)>) =
        match conn.db_type.as_str() {
            "postgres" => (
                "psql",
                vec!["-U".to_string(), conn.username.clone(), conn.database.clone()],
                Some(("PGPASSWORD".to_string(), password)),
            ),
            "mysql" | "mariadb" => (
                "mysql",
                vec![format!("-u{}", conn.username), conn.database.clone()],
                Some(("MYSQL_PWD".to_string(), password)),
            ),
            "sqlite" => ("sqlite3", vec![conn.database.clone()], None),
            "oracle" => (
                "sqlplus",
                vec![format!("{}/{}@/{}", conn.username, password, conn.database)],
                None,
            ),
            "mssql" => (
                "sqlcmd",
                vec![
                    "-U".to_string(),
                    conn.username.clone(),
                    "-P".to_string(),
                    password,
                    "-d".to_string(),
                    conn.database.clone(),
                ],
                None,
            ),
            "clickhouse" => (
                "clickhouse-client",
                vec![
                    "--user".to_string(),
                    conn.username.clone(),
                    "--password".to_string(),
                    password,
                    "--database".to_string(),
                    conn.database.clone(),
                ],
                None,
            ),
            other => {
                return Err(format!(
                    "Unsupported database type for terminal: {}",
                    other
                ))
            }
        };

    // Assemble: docker exec [-e KEY=VALUE] -it <container> <cli> <args...>
    let mut docker_args = vec!["exec".to_string()];
    if let Some((key, val)) = env_kv {
        docker_args.push("-e".to_string());
        docker_args.push(format!("{}={}", key, val));
    }
    docker_args.push("-it".to_string());
    docker_args.push(container.to_string());
    docker_args.push(cli_binary.to_string());
    docker_args.append(&mut cli_args);

    Ok(CliSpec {
        binary: "docker".to_string(),
        args: docker_args,
        env: vec![],
    })
}

/// Build the remote shell command string for `docker exec` inside an SSH session.
/// The returned string is passed verbatim to the remote shell, so all arguments
/// are single-quote-escaped.
fn remote_docker_exec_cmd(conn: &SavedConnection, container: &str) -> CmdResult<String> {
    let parsed = url::Url::parse(&conn.url).map_err(map_err)?;
    let password = parsed.password().unwrap_or("").to_string();

    let (cli, mut cli_parts, env_prefix): (&str, Vec<String>, Option<String>) =
        match conn.db_type.as_str() {
            "postgres" => (
                "psql",
                vec![
                    "-U".to_string(),
                    sh_escape(&conn.username),
                    sh_escape(&conn.database),
                ],
                Some(format!("PGPASSWORD={}", sh_escape(&password))),
            ),
            "mysql" | "mariadb" => (
                "mysql",
                vec![
                    format!("-u{}", sh_escape(&conn.username)),
                    sh_escape(&conn.database),
                ],
                Some(format!("MYSQL_PWD={}", sh_escape(&password))),
            ),
            "sqlite" => ("sqlite3", vec![sh_escape(&conn.database)], None),
            "oracle" => (
                "sqlplus",
                vec![sh_escape(&format!(
                    "{}/{}@/{}",
                    conn.username, password, conn.database
                ))],
                None,
            ),
            "mssql" => (
                "sqlcmd",
                vec![
                    "-U".to_string(),
                    sh_escape(&conn.username),
                    "-P".to_string(),
                    sh_escape(&password),
                    "-d".to_string(),
                    sh_escape(&conn.database),
                ],
                None,
            ),
            "clickhouse" => (
                "clickhouse-client",
                vec![
                    "--user".to_string(),
                    sh_escape(&conn.username),
                    "--password".to_string(),
                    sh_escape(&password),
                    "--database".to_string(),
                    sh_escape(&conn.database),
                ],
                None,
            ),
            other => {
                return Err(format!(
                    "Unsupported database type for terminal: {}",
                    other
                ))
            }
        };

    // docker exec [-e KEY=VALUE] -it <container> <cli> <args>
    let mut parts = vec!["docker".to_string(), "exec".to_string()];
    if let Some(env) = env_prefix {
        parts.push("-e".to_string());
        parts.push(env);
    }
    parts.push("-it".to_string());
    parts.push(sh_escape(container));
    parts.push(cli.to_string());
    parts.append(&mut cli_parts);

    Ok(parts.join(" "))
}

/// Build `ssh [-J jump,...] -t user@host "docker exec ..."` spec for remote Docker.
///
/// Auth strategy:
/// - Key:    passes `-i <key_path>`; SSH prompts for passphrase through the PTY.
/// - Agent:  SSH picks up keys from `SSH_AUTH_SOCK` automatically.
/// - Password: uses `sshpass -p <pw>` if available; otherwise SSH prompts through the PTY.
fn ssh_docker_exec_spec(
    conn: &SavedConnection,
    profile: &SshProfile,
    auth: &SshAuthConfig,
    container: &str,
) -> CmdResult<CliSpec> {
    let remote_cmd = remote_docker_exec_cmd(conn, container)?;

    let mut ssh_args: Vec<String> = vec![
        "-t".to_string(), // force PTY on remote
        "-p".to_string(),
        profile.port.to_string(),
        "-o".to_string(),
        "StrictHostKeyChecking=accept-new".to_string(),
        "-o".to_string(),
        "BatchMode=no".to_string(),
    ];

    // Identity file for key auth on the main hop
    if let Some(key) = &auth.key_path {
        ssh_args.push("-i".to_string());
        ssh_args.push(key.to_string_lossy().to_string());
    }

    // Jump hosts: -J user@host:port,user@host2:port2
    // Identity files for key-auth jump hosts are also added via -i so SSH can
    // try them across all hops.
    if !profile.proxy_jump.is_empty() {
        let jump_chain: Vec<String> = profile
            .proxy_jump
            .iter()
            .map(|j| format!("{}@{}:{}", j.username, j.host, j.port))
            .collect();
        ssh_args.push("-J".to_string());
        ssh_args.push(jump_chain.join(","));

        for jump in &profile.proxy_jump {
            if matches!(jump.auth_method, SshAuthMethod::Key) {
                if let Some(key) = &jump.key_path {
                    ssh_args.push("-i".to_string());
                    ssh_args.push(key.clone());
                }
            }
        }
    }

    // Target
    ssh_args.push(format!("{}@{}", profile.username, profile.host));
    ssh_args.push(remote_cmd);

    // Password auth: wrap with sshpass if available, otherwise let SSH prompt
    if matches!(auth.method, AuthMethod::Password) {
        if let Some(password) = &auth.password {
            if let Ok(sshpass_path) = which::which("sshpass") {
                let mut sshpass_args =
                    vec!["-p".to_string(), password.clone(), "ssh".to_string()];
                sshpass_args.extend(ssh_args);
                return Ok(CliSpec {
                    binary: sshpass_path.to_string_lossy().to_string(),
                    args: sshpass_args,
                    env: vec![],
                });
            }
            // sshpass not available — SSH will prompt in the PTY
        }
    }

    Ok(CliSpec {
        binary: "ssh".to_string(),
        args: ssh_args,
        env: vec![],
    })
}

// ── Auth helpers ──────────────────────────────────────────────────────────────

fn resolve_ssh_auth(profile: &SshProfile, credentials: &CredentialStore) -> CmdResult<SshAuthConfig> {
    match profile.auth_method {
        SshAuthMethod::Key => {
            let key_path = profile.key_path.as_deref().unwrap_or_default();
            let passphrase = credentials
                .get_credential(&profile.id, "passphrase")
                .map_err(map_err)?;
            Ok(if let Some(pp) = passphrase {
                SshAuthConfig::with_key_and_passphrase(&profile.username, key_path, pp)
            } else {
                SshAuthConfig::with_key(&profile.username, key_path)
            })
        }
        SshAuthMethod::Password => {
            let password = credentials
                .get_credential(&profile.id, "password")
                .map_err(map_err)?
                .unwrap_or_default();
            Ok(SshAuthConfig::with_password(&profile.username, password))
        }
        SshAuthMethod::Agent => Ok(SshAuthConfig::with_agent(&profile.username)),
    }
}

// ── Binary resolution ─────────────────────────────────────────────────────────

fn resolve_binary(name: &str) -> CmdResult<std::path::PathBuf> {
    which::which(name).map_err(|_| {
        format!(
            "{} not found on PATH. Install the appropriate client tools.",
            name
        )
    })
}

// ── Tauri commands ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn spawn_db_terminal(
    state: State<'_, AppState>,
    connection_id: String,
    cols: u16,
    rows: u16,
    on_data: Channel<String>,
) -> CmdResult<String> {
    let connections = state.config.get_connections().map_err(map_err)?;
    let conn = connections
        .iter()
        .find(|c| c.id == connection_id)
        .ok_or_else(|| format!("Connection '{}' not found", connection_id))?
        .clone();

    let spec = match &conn.connection_type {
        // Remote Docker via SSH: spawn ssh -t ... "docker exec ..."
        ConnectionType::DockerContainer => {
            let ssh_profile_id = conn
                .ssh_profile_id
                .as_ref()
                .ok_or("DockerContainer connection requires an SSH profile")?;
            let profile = state
                .config
                .get_ssh_profile(ssh_profile_id)
                .map_err(map_err)?
                .ok_or_else(|| format!("SSH profile '{}' not found", ssh_profile_id))?;
            let container = conn
                .container_name
                .as_deref()
                .ok_or("DockerContainer connection requires a container name")?;
            let auth = resolve_ssh_auth(&profile, &state.credentials)?;
            ssh_docker_exec_spec(&conn, &profile, &auth, container)?
        }
        // Local Docker: docker exec -it <container> <cli>
        ConnectionType::LocalDockerContainer => docker_exec_cli_spec(&conn)?,
        // SSH tunnel: the tunnel is already forwarded to localhost:local_port
        _ if state.tunnels.contains_key(&connection_id) => {
            let local_port = state
                .tunnels
                .get(&connection_id)
                .map(|t| t.local_port)
                .unwrap();
            direct_cli_spec(&conn, "127.0.0.1", local_port)?
        }
        // Direct connection
        _ => direct_cli_spec(&conn, &conn.host.clone(), conn.port)?,
    };

    let binary_path = resolve_binary(&spec.binary)?;

    let pty_system = NativePtySystem::default();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(map_err)?;

    let mut cmd = CommandBuilder::new(binary_path);
    for arg in &spec.args {
        cmd.arg(arg);
    }
    for (key, val) in &spec.env {
        cmd.env(key, val);
    }

    let child = pair.slave.spawn_command(cmd).map_err(map_err)?;
    // Close slave in parent so master EOF fires when child exits
    drop(pair.slave);

    let writer = pair.master.take_writer().map_err(map_err)?;
    let mut reader = pair.master.try_clone_reader().map_err(map_err)?;
    let master = pair.master;

    let terminal_id = uuid::Uuid::new_v4().to_string();

    // Relay PTY output to the frontend via Tauri channel
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => {
                    // Null byte sentinel: frontend shows "session ended" message
                    let _ = on_data.send("\x00".to_string());
                    break;
                }
                Ok(n) => {
                    let encoded = BASE64.encode(&buf[..n]);
                    if on_data.send(encoded).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let handle = PtyHandle {
        writer: Mutex::new(writer),
        master: Mutex::new(master),
        child: Mutex::new(child),
    };

    state.terminals.insert(terminal_id.clone(), handle);
    Ok(terminal_id)
}

#[tauri::command]
pub async fn send_terminal_input(
    state: State<'_, AppState>,
    terminal_id: String,
    data: String,
) -> CmdResult<()> {
    let handle = state
        .terminals
        .get(&terminal_id)
        .ok_or_else(|| format!("Terminal '{}' not found", terminal_id))?;

    let mut writer = handle.writer.lock().map_err(map_err)?;
    writer.write_all(data.as_bytes()).map_err(map_err)?;
    writer.flush().map_err(map_err)
}

#[tauri::command]
pub async fn resize_terminal(
    state: State<'_, AppState>,
    terminal_id: String,
    cols: u16,
    rows: u16,
) -> CmdResult<()> {
    let handle = state
        .terminals
        .get(&terminal_id)
        .ok_or_else(|| format!("Terminal '{}' not found", terminal_id))?;

    let master = handle.master.lock().map_err(map_err)?;
    master
        .resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(map_err)
}

#[tauri::command]
pub async fn close_terminal(
    state: State<'_, AppState>,
    terminal_id: String,
) -> CmdResult<()> {
    if let Some((_, handle)) = state.terminals.remove(&terminal_id) {
        let mut child = handle.child.lock().map_err(map_err)?;
        child.kill().map_err(map_err)?;
    }
    Ok(())
}
