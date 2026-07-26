use sqlator_service::AppService;
use std::sync::Arc;

pub struct AppState {
    pub service: Arc<AppService>,
}

impl AppState {
    pub fn new() -> Result<Self, sqlator_service::ServiceError> {
        Ok(Self {
            service: Arc::new(AppService::new()?),
        })
    }

    /// Create state pre-wired to a single database.
    /// The database pool is connected eagerly so the first page load is instant.
    pub async fn new_with_single_db(
        url: String,
        name: String,
    ) -> Result<Self, sqlator_service::ServiceError> {
        Ok(Self {
            service: Arc::new(AppService::with_single_db(url, name).await?),
        })
    }
}

/// Parse a connection URL from a config file.
/// Supports:
///   - JSON: `{"url": "...", "name": "..."}` (name optional)
///   - YAML-style line:  `url: postgres://...`
///   - Plain text: the URL itself
pub fn parse_config_file(path: &std::path::Path) -> Result<(String, String), String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read config file '{}': {}", path.display(), e))?;
    let content = content.trim();

    // Try JSON first
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(content) {
        if let Some(url) = v.get("url").and_then(|u| u.as_str()) {
            let name = v
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("Database")
                .to_string();
            return Ok((url.to_string(), name));
        }
        return Err("JSON config file must contain a 'url' field".into());
    }

    // Try YAML-style `url: <value>` line
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("url:") {
            let url = rest.trim().trim_matches('"').trim_matches('\'');
            if !url.is_empty() {
                return Ok((url.to_string(), "Database".to_string()));
            }
        }
    }

    // Also try a two-pass YAML scan for both url and name
    let mut url_found: Option<String> = None;
    let mut name_found: Option<String> = None;
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("url:") {
            url_found = Some(rest.trim().trim_matches('"').trim_matches('\'').to_string());
        }
        if let Some(rest) = line.strip_prefix("name:") {
            name_found = Some(rest.trim().trim_matches('"').trim_matches('\'').to_string());
        }
    }
    if let Some(url) = url_found {
        return Ok((url, name_found.unwrap_or_else(|| "Database".to_string())));
    }

    // Treat entire content as a raw URL
    if content.contains("://") {
        return Ok((content.to_string(), "Database".to_string()));
    }

    Err(format!(
        "Could not parse '{}' as a connection config. \
         Expected JSON {{\"url\": \"...\"}}, YAML with 'url:' field, or a bare connection URL.",
        path.display()
    ))
}
