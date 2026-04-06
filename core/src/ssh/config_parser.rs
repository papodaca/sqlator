use crate::ssh::error::{SshError, SshResult};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::warn;

/// A single host entry parsed from ~/.ssh/config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostEntry {
    /// The alias from the `Host` line (e.g. "bastion")
    pub alias: String,
    /// Resolved `HostName` value (falls back to alias when absent)
    pub hostname: String,
    pub port: u16,
    pub user: Option<String>,
    pub identity_file: Option<String>,
    pub proxy_jump: Option<String>,
}

/// Parse `~/.ssh/config` and return all concrete host entries.
/// Wildcard patterns (`*`, `?`) are skipped.
/// Invalid / unresolvable entries are logged and skipped.
pub fn load_ssh_config() -> SshResult<Vec<HostEntry>> {
    let config_path = ssh_config_path()?;

    if !config_path.exists() {
        return Ok(vec![]);
    }

    let raw =
        std::fs::read_to_string(&config_path).map_err(|e| SshError::ConfigError(e.to_string()))?;

    let aliases = extract_host_aliases(&raw);
    let mut entries = Vec::new();

    for alias in aliases {
        match russh_config::parse(&raw, &alias) {
            Ok(cfg) => {
                let hostname = cfg
                    .host_config
                    .hostname
                    .as_deref()
                    .unwrap_or(&alias)
                    .to_string();

                let port = cfg.host_config.port.unwrap_or(22);

                let identity_file = cfg
                    .host_config
                    .identity_file
                    .as_deref()
                    .and_then(|v| v.first())
                    .map(|p| p.to_string_lossy().to_string());

                let proxy_jump = cfg.host_config.proxy_jump.clone();

                entries.push(HostEntry {
                    alias,
                    hostname,
                    port,
                    user: cfg.host_config.user.clone(),
                    identity_file,
                    proxy_jump,
                });
            }
            Err(e) => {
                warn!("Skipping SSH config host '{}': {}", alias, e);
            }
        }
    }

    // Stable alphabetical order
    entries.sort_by(|a, b| a.alias.cmp(&b.alias));

    Ok(entries)
}

/// Extract non-wildcard host aliases from raw ssh_config text.
/// Handles `Host alias1 alias2` multi-value lines; skips `*` / `?` patterns.
fn extract_host_aliases(raw: &str) -> Vec<String> {
    let mut aliases = Vec::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // Split on first whitespace
        let mut parts = trimmed.splitn(2, char::is_whitespace);
        let key = parts.next().unwrap_or("").to_lowercase();
        if key != "host" {
            continue;
        }
        let values = parts.next().unwrap_or("");
        for token in values.split_whitespace() {
            // Skip negation prefixes and wildcards
            let token = token.trim_start_matches('!');
            if token.contains('*') || token.contains('?') {
                continue;
            }
            if !token.is_empty() {
                aliases.push(token.to_string());
            }
        }
    }

    // Deduplicate while preserving first-seen order
    let mut seen = std::collections::HashSet::new();
    aliases.retain(|a| seen.insert(a.clone()));

    aliases
}

fn ssh_config_path() -> SshResult<PathBuf> {
    dirs::home_dir()
        .map(|h| h.join(".ssh").join("config"))
        .ok_or_else(|| SshError::ConfigError("Could not determine home directory".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
# Work machines
Host bastion
    HostName bastion.example.com
    User deploy
    Port 2222
    IdentityFile ~/.ssh/id_ed25519

Host dev-db
    HostName 10.0.1.5
    User admin
    ProxyJump bastion

Host *.internal
    User ubuntu

Host *
    ServerAliveInterval 60
"#;

    #[test]
    fn extracts_concrete_aliases() {
        let aliases = extract_host_aliases(SAMPLE);
        assert_eq!(aliases, vec!["bastion", "dev-db"]);
    }

    #[test]
    fn parse_returns_host_entries() {
        let entries = {
            let mut out = Vec::new();
            for alias in extract_host_aliases(SAMPLE) {
                if let Ok(cfg) = russh_config::parse(SAMPLE, &alias) {
                    let hostname = cfg
                        .host_config
                        .hostname
                        .as_deref()
                        .unwrap_or(&alias)
                        .to_string();
                    let port = cfg.host_config.port.unwrap_or(22);
                    out.push(HostEntry {
                        alias,
                        hostname,
                        port,
                        user: cfg.host_config.user.clone(),
                        identity_file: cfg
                            .host_config
                            .identity_file
                            .as_deref()
                            .and_then(|v| v.first())
                            .map(|p| p.to_string_lossy().to_string()),
                        proxy_jump: cfg.host_config.proxy_jump.clone(),
                    });
                }
            }
            out
        };

        assert_eq!(entries.len(), 2);

        let bastion = entries.iter().find(|e| e.alias == "bastion").unwrap();
        assert_eq!(bastion.hostname, "bastion.example.com");
        assert_eq!(bastion.port, 2222);
        assert_eq!(bastion.user.as_deref(), Some("deploy"));

        let dev_db = entries.iter().find(|e| e.alias == "dev-db").unwrap();
        assert_eq!(dev_db.hostname, "10.0.1.5");
        assert_eq!(dev_db.proxy_jump.as_deref(), Some("bastion"));
    }
}
