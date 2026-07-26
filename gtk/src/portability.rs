//! Connection import / export via GTK file dialogs + toast feedback.

use crate::application::SqlatorApplication;
use crate::window::SqlatorWindow;
use adw::prelude::*;
use gtk::{gio, glib};
use std::path::PathBuf;

/// Present a save dialog and write the export JSON payload.
pub fn export_connections(window: &SqlatorWindow) {
    let Some(app) = window.application().and_downcast::<SqlatorApplication>() else {
        return;
    };
    let service = app.service();

    let dialog = gtk::FileDialog::builder()
        .title("Export Connections")
        .accept_label("Export")
        .initial_name("sqlator-connections.json")
        .build();
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("JSON"));
    filter.add_pattern("*.json");
    filter.add_mime_type("application/json");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));
    dialog.set_default_filter(Some(&filter));

    dialog.save(
        Some(window),
        gio::Cancellable::NONE,
        glib::clone!(
            #[weak]
            window,
            move |result| {
                let Ok(file) = result else {
                    return;
                };
                let Some(path) = file.path() else {
                    window.show_toast("Could not resolve export path");
                    return;
                };
                glib::spawn_future_local(async move {
                    let svc = service.clone();
                    let json =
                        crate::spawn_tokio!(async move { svc.export_connections_json().await })
                            .await
                            .expect("join export_connections_json");
                    match json {
                        Ok(payload) => match std::fs::write(&path, payload) {
                            Ok(()) => {
                                let toast = adw::Toast::new("Connections exported");
                                toast.set_button_label(Some("Open"));
                                let path_open = path.clone();
                                toast.connect_button_clicked(move |_| {
                                    open_path(&path_open);
                                });
                                window.add_toast(toast);
                            }
                            Err(e) => {
                                window.show_toast(&format!("Export failed: {e}"));
                            }
                        },
                        Err(e) => window.show_toast(&format!("Export failed: {e}")),
                    }
                });
            }
        ),
    );
}

/// Present an open dialog, import with `"rename"` duplicate mode, then refresh.
pub fn import_connections(window: &SqlatorWindow) {
    let Some(app) = window.application().and_downcast::<SqlatorApplication>() else {
        return;
    };
    let service = app.service();

    let dialog = gtk::FileDialog::builder()
        .title("Import Connections")
        .accept_label("Import")
        .build();
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("JSON"));
    filter.add_pattern("*.json");
    filter.add_mime_type("application/json");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));
    dialog.set_default_filter(Some(&filter));

    dialog.open(
        Some(window),
        gio::Cancellable::NONE,
        glib::clone!(
            #[weak]
            window,
            move |result| {
                let Ok(file) = result else {
                    return;
                };
                let Some(path) = file.path() else {
                    window.show_toast("Could not resolve import path");
                    return;
                };
                let json = match std::fs::read_to_string(&path) {
                    Ok(s) => s,
                    Err(e) => {
                        window.show_toast(&format!("Could not read file: {e}"));
                        return;
                    }
                };
                glib::spawn_future_local(async move {
                    let svc = service.clone();
                    let result = crate::spawn_tokio!(async move {
                        svc.import_connections(json, "rename".into()).await
                    })
                    .await
                    .expect("join import_connections");
                    match result {
                        Ok(stats) => {
                            window.refresh_connections();
                            window.show_toast(&format!(
                                "Imported {} connections, {} groups, {} SSH profiles ({} skipped)",
                                stats.connections_added,
                                stats.groups_added,
                                stats.profiles_added,
                                stats.connections_skipped
                            ));
                        }
                        Err(e) => window.show_toast(&format!("Import failed: {e}")),
                    }
                });
            }
        ),
    );
}

fn open_path(path: &PathBuf) {
    let uri = match glib::filename_to_uri(path, None) {
        Ok(uri) => uri.to_string(),
        Err(_) => format!("file://{}", path.display()),
    };
    if let Err(e) = gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE) {
        tracing::warn!("open export path failed: {e}");
    }
}
