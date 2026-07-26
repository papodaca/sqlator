//! Ship SQL snippets via GResource and register them with SnippetManager.

use gtk::{gio, glib};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Once;

static INIT: Once = Once::new();

const RESOURCE_PATH: &str = "/im/apodaca/SqlatorGtk/snippets/sql.snippets";

/// Extract bundled snippets and prepend their directory to the default manager.
pub fn ensure_registered() {
    INIT.call_once(|| {
        if let Err(e) = register_inner() {
            tracing::warn!("failed to register SQL snippets: {e}");
        }
    });
}

fn register_inner() -> Result<(), String> {
    let dir = snippets_cache_dir()?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = dir.join("sql.snippets");

    let bytes = gio::resources_lookup_data(RESOURCE_PATH, gio::ResourceLookupFlags::NONE)
        .map_err(|e| format!("lookup {RESOURCE_PATH}: {e}"))?;
    let mut file = fs::File::create(&dest).map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;

    let manager = sourceview::SnippetManager::default();
    let mut paths: Vec<String> = manager
        .search_path()
        .into_iter()
        .map(|g| g.to_string())
        .collect();
    let dir_str = dir.to_string_lossy().to_string();
    if !paths.iter().any(|p| p == &dir_str) {
        paths.insert(0, dir_str);
    }
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    manager.set_search_path(&refs);
    Ok(())
}

fn snippets_cache_dir() -> Result<PathBuf, String> {
    Ok(glib::user_cache_dir().join("sqlator-gtk").join("snippets"))
}
