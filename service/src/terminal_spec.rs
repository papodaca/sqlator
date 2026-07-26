//! Frontend-agnostic DB CLI argument builders (PTY lifecycle stays in Tauri/GTK).
//!
//! Passwords for postgres/mysql go via env (`PGPASSWORD` / `MYSQL_PWD`), not argv.
//! Engines whose clients lack a password env still receive credentials the way their
//! CLI requires (oracle connect string, `sqlcmd -P`, `clickhouse-client --password`).

use sqlator_core::models::{ConnectionType, SavedConnection, SshAuthMethod, SshProfile};
use sqlator_core::ssh::SshAuthConfig;

/// Resolved command ready to pass to a PTY / VTE spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliSpec {
    pub binary: String,
    pub args: Vec<String>,
    /// Environment variables set on the spawned process.
    pub env: Vec<(String, String)>,
}

/// Build the CLI for a saved connection, preferring a shared SSH-tunnel local port.
pub fn build_cli_for_connection(
    conn: &SavedConnection,
    tunnel_local_port: Option<u16>,
    ssh: Option<(&SshProfile, &SshAuthConfig)>,
) -> Result<CliSpec, String> {
    match &conn.connection_type {
        ConnectionType::DockerContainer => {
            let (profile, auth) = ssh.ok_or_else(|| {
                "DockerContainer connection requires an SSH profile and auth".to_string()
            })?;
            let container = conn
                .container_name
                .as_deref()
                .ok_or("DockerContainer connection requires a container name")?;
            ssh_docker_exec_spec(conn, profile, auth, container)
        }
        ConnectionType::LocalDockerContainer => docker_exec_cli_spec(conn),
        _ => {
            if let Some(local_port) = tunnel_local_port {
                direct_cli_spec(conn, "127.0.0.1", local_port)
            } else {
                direct_cli_spec(conn, &conn.host, conn.port)
            }
        }
    }
}

/// Build CLI args for direct / SSH-tunnel connections.
/// `host` and `port` are already resolved (tunnel local endpoint if applicable).
pub fn direct_cli_spec(conn: &SavedConnection, host: &str, port: u16) -> Result<CliSpec, String> {
    let parsed = url::Url::parse(&conn.url).map_err(|e| e.to_string())?;
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
        other => Err(format!("Unsupported database type for terminal: {other}")),
    }
}

/// Build a `docker exec -it <container> <cli> <args>` spec for local Docker.
/// Passwords are passed via docker exec `-e KEY=VALUE` when the client supports env.
pub fn docker_exec_cli_spec(conn: &SavedConnection) -> Result<CliSpec, String> {
    let container = conn
        .container_name
        .as_deref()
        .ok_or("LocalDockerContainer connection is missing a container name")?;

    let parsed = url::Url::parse(&conn.url).map_err(|e| e.to_string())?;
    let password = parsed.password().unwrap_or("").to_string();

    let (cli_binary, mut cli_args, env_kv): (&str, Vec<String>, Option<(String, String)>) =
        match conn.db_type.as_str() {
            "postgres" => (
                "psql",
                vec![
                    "-U".to_string(),
                    conn.username.clone(),
                    conn.database.clone(),
                ],
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
            other => return Err(format!("Unsupported database type for terminal: {other}")),
        };

    let mut docker_args = vec!["exec".to_string()];
    if let Some((key, val)) = env_kv {
        docker_args.push("-e".to_string());
        docker_args.push(format!("{key}={val}"));
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

/// POSIX single-quote escape — safe for interpolation in a remote shell command.
pub fn sh_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Build the remote shell command string for `docker exec` inside an SSH session.
pub fn remote_docker_exec_cmd(conn: &SavedConnection, container: &str) -> Result<String, String> {
    let parsed = url::Url::parse(&conn.url).map_err(|e| e.to_string())?;
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
            other => return Err(format!("Unsupported database type for terminal: {other}")),
        };

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

/// Build `ssh [-J jump,...] -t user@host "docker exec ..."` for remote Docker.
pub fn ssh_docker_exec_spec(
    conn: &SavedConnection,
    profile: &SshProfile,
    auth: &SshAuthConfig,
    container: &str,
) -> Result<CliSpec, String> {
    let remote_cmd = remote_docker_exec_cmd(conn, container)?;

    let mut ssh_args: Vec<String> = vec![
        "-t".to_string(),
        "-p".to_string(),
        profile.port.to_string(),
        "-o".to_string(),
        "StrictHostKeyChecking=accept-new".to_string(),
        "-o".to_string(),
        "BatchMode=no".to_string(),
    ];

    if let Some(key) = &auth.key_path {
        ssh_args.push("-i".to_string());
        ssh_args.push(key.to_string_lossy().to_string());
    }

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

    ssh_args.push(format!("{}@{}", profile.username, profile.host));
    ssh_args.push(remote_cmd);

    // Password auth: frontends may wrap with `sshpass` when available; otherwise
    // SSH prompts through the PTY (`BatchMode=no` above).
    Ok(CliSpec {
        binary: "ssh".to_string(),
        args: ssh_args,
        env: vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlator_core::models::ConnectionType;

    fn sample_conn(db_type: &str) -> SavedConnection {
        SavedConnection {
            id: "c1".into(),
            name: "test".into(),
            color_id: "blue".into(),
            db_type: db_type.into(),
            host: "db.example".into(),
            port: 5432,
            database: "app".into(),
            username: "alice".into(),
            url: format!("{db_type}://alice:s3cret@db.example:5432/app"),
            ssh_profile_id: None,
            group_id: None,
            connection_type: ConnectionType::Direct,
            container_name: None,
            container_port: None,
        }
    }

    #[test]
    fn postgres_password_goes_in_env_not_argv() {
        let spec = direct_cli_spec(&sample_conn("postgres"), "127.0.0.1", 15432).unwrap();
        assert_eq!(spec.binary, "psql");
        assert!(spec.args.iter().all(|a| !a.contains("s3cret")));
        assert_eq!(spec.env, vec![("PGPASSWORD".into(), "s3cret".into())]);
        assert!(spec.args.contains(&"127.0.0.1".into()));
        assert!(spec.args.contains(&"15432".into()));
    }

    #[test]
    fn mysql_password_goes_in_env_not_argv() {
        let mut conn = sample_conn("mysql");
        conn.port = 3306;
        let spec = direct_cli_spec(&conn, "127.0.0.1", 13306).unwrap();
        assert_eq!(spec.binary, "mysql");
        assert!(spec.args.iter().all(|a| !a.contains("s3cret")));
        assert_eq!(spec.env, vec![("MYSQL_PWD".into(), "s3cret".into())]);
    }

    #[test]
    fn sqlite_uses_database_path_only() {
        let mut conn = sample_conn("sqlite");
        conn.database = "/tmp/x.db".into();
        conn.url = "sqlite:///tmp/x.db".into();
        let spec = direct_cli_spec(&conn, "", 0).unwrap();
        assert_eq!(spec.binary, "sqlite3");
        assert_eq!(spec.args, vec!["/tmp/x.db".to_string()]);
        assert!(spec.env.is_empty());
    }

    #[test]
    fn local_docker_passes_password_via_exec_env() {
        let mut conn = sample_conn("postgres");
        conn.connection_type = ConnectionType::LocalDockerContainer;
        conn.container_name = Some("pg".into());
        let spec = docker_exec_cli_spec(&conn).unwrap();
        assert_eq!(spec.binary, "docker");
        assert!(spec.args.iter().any(|a| a == "PGPASSWORD=s3cret"));
        assert!(spec.args.iter().all(|a| a != "s3cret"));
        assert!(spec.env.is_empty());
    }

    #[test]
    fn sh_escape_quotes_and_embeds() {
        assert_eq!(sh_escape("plain"), "'plain'");
        assert_eq!(sh_escape("a'b"), "'a'\\''b'");
    }

    #[test]
    fn build_cli_prefers_tunnel_port() {
        let conn = sample_conn("postgres");
        let spec = build_cli_for_connection(&conn, Some(9999), None).unwrap();
        assert!(spec.args.contains(&"9999".into()));
        assert!(spec.args.contains(&"127.0.0.1".into()));
    }
}
