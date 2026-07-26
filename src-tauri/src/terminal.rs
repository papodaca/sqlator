use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use sqlator_core::models::ConnectionType;
use sqlator_service::build_cli_for_connection;
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

fn resolve_binary(name: &str) -> CmdResult<std::path::PathBuf> {
    which::which(name)
        .map_err(|_| format!("{name} not found on PATH. Install the appropriate client tools."))
}

/// Optionally wrap an ssh password-auth spec with `sshpass` when available.
fn maybe_wrap_sshpass(
    mut spec: sqlator_service::CliSpec,
    auth: &sqlator_core::ssh::SshAuthConfig,
) -> sqlator_service::CliSpec {
    use sqlator_core::ssh::AuthMethod;
    if spec.binary != "ssh" || !matches!(auth.method, AuthMethod::Password) {
        return spec;
    }
    let Some(password) = auth.password.as_ref() else {
        return spec;
    };
    let Ok(sshpass_path) = which::which("sshpass") else {
        return spec;
    };
    let mut args = vec!["-p".to_string(), password.clone(), "ssh".to_string()];
    args.append(&mut spec.args);
    sqlator_service::CliSpec {
        binary: sshpass_path.to_string_lossy().to_string(),
        args,
        env: spec.env,
    }
}

#[tauri::command]
pub async fn spawn_db_terminal(
    state: State<'_, AppState>,
    connection_id: String,
    cols: u16,
    rows: u16,
    on_data: Channel<String>,
) -> CmdResult<String> {
    let connections = state.service.config().get_connections().map_err(map_err)?;
    let conn = connections
        .iter()
        .find(|c| c.id == connection_id)
        .ok_or_else(|| format!("Connection '{connection_id}' not found"))?
        .clone();

    let tunnel_port = state
        .service
        .tunnel_local_port_for_connection(&connection_id);

    let (spec, auth_for_wrap) = match &conn.connection_type {
        ConnectionType::DockerContainer => {
            let ssh_profile_id = conn
                .ssh_profile_id
                .as_ref()
                .ok_or("DockerContainer connection requires an SSH profile")?;
            let profile = state
                .service
                .config()
                .get_ssh_profile(ssh_profile_id)
                .map_err(map_err)?
                .ok_or_else(|| format!("SSH profile '{ssh_profile_id}' not found"))?;
            let auth = sqlator_service::build_auth_config_for_profile(
                &profile,
                state.service.credentials(),
            )
            .map_err(|e| e.message())?;
            let spec = build_cli_for_connection(&conn, tunnel_port, Some((&profile, &auth)))?;
            (maybe_wrap_sshpass(spec, &auth), None)
        }
        _ => {
            let spec = build_cli_for_connection(&conn, tunnel_port, None)?;
            (spec, None::<()>)
        }
    };
    let _ = auth_for_wrap;

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
    drop(pair.slave);

    let writer = pair.master.take_writer().map_err(map_err)?;
    let mut reader = pair.master.try_clone_reader().map_err(map_err)?;
    let master = pair.master;

    let terminal_id = uuid::Uuid::new_v4().to_string();

    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => {
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
        .ok_or_else(|| format!("Terminal '{terminal_id}' not found"))?;

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
        .ok_or_else(|| format!("Terminal '{terminal_id}' not found"))?;

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
pub async fn close_terminal(state: State<'_, AppState>, terminal_id: String) -> CmdResult<()> {
    if let Some((_, handle)) = state.terminals.remove(&terminal_id) {
        let mut child = handle.child.lock().map_err(map_err)?;
        child.kill().map_err(map_err)?;
    }
    Ok(())
}
