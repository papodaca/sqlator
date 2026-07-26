//! Connection create/edit dialog (Svelte `ConnectionForm` parity).

use super::colors::color_css_class;
use crate::application::SqlatorApplication;
use crate::window::SqlatorWindow;
use adw::prelude::*;
use gtk::glib;
use sqlator_core::models::{ConnectionConfig, ConnectionInfo, ConnectionType};
use sqlator_service::{default_port_for_db_type, parse_connection_url, ParsedConnectionUrl};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const COLOR_IDS: &[&str] = &[
    "red", "orange", "yellow", "green", "teal", "blue", "violet", "pink", "slate", "white",
];

const DB_TYPES: &[(&str, &str)] = &[
    ("postgres", "PostgreSQL"),
    ("mysql", "MySQL"),
    ("mariadb", "MariaDB"),
    ("sqlite", "SQLite"),
    ("mssql", "MS SQL Server"),
    ("oracle", "Oracle (experimental)"),
    ("clickhouse", "ClickHouse"),
];

const SSL_MODES: &[(&str, &str)] = &[
    ("prefer", "Prefer"),
    ("disable", "Disable"),
    ("require", "Require"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum FormTab {
    Url,
    Manual,
}

struct FormState {
    editing_id: Option<String>,
    is_docker: bool,
    connection_type: Option<ConnectionType>,
    /// Preserved for Docker edits (full Docker UI is a later bead).
    container_name: Option<String>,
    container_port: Option<u16>,
    color_id: RefCell<String>,
    tab: Cell<FormTab>,
    /// (id, display name) — index 0 is "None".
    groups: RefCell<Vec<(Option<String>, String)>>,
    /// (id, display name) — index 0 is "None".
    ssh_profiles: RefCell<Vec<(Option<String>, String)>>,
}

/// Present a create (`editing = None`) or edit dialog.
pub fn present(window: &SqlatorWindow, app: &SqlatorApplication, editing: Option<ConnectionInfo>) {
    let service = app.service();
    let dialog = adw::Dialog::new();
    dialog.set_content_width(560);
    dialog.set_title(if editing.is_some() {
        "Edit Connection"
    } else {
        "New Connection"
    });

    let toast_overlay = adw::ToastOverlay::new();
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();

    let cancel_btn = gtk::Button::with_label("Cancel");
    cancel_btn.add_css_class("flat");
    let save_btn = gtk::Button::with_label("Save");
    save_btn.add_css_class("suggested-action");
    let test_btn = gtk::Button::with_label("Test");

    header.pack_start(&cancel_btn);
    header.pack_end(&save_btn);
    header.pack_end(&test_btn);
    toolbar.add_top_bar(&header);

    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(520);
    clamp.set_tightening_threshold(400);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
    content.set_margin_top(18);
    content.set_margin_bottom(24);
    content.set_margin_start(18);
    content.set_margin_end(18);

    // ── Name ──────────────────────────────────────────────────────────────
    let general = adw::PreferencesGroup::new();
    general.set_title("General");
    let name_row = adw::EntryRow::builder().title("Name").build();
    general.add(&name_row);

    // Color swatches
    let color_row = adw::ActionRow::builder().title("Color").build();
    let color_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    color_box.set_valign(gtk::Align::Center);
    color_row.add_suffix(&color_box);
    general.add(&color_row);
    content.append(&general);

    // ── Mode tabs ─────────────────────────────────────────────────────────
    let tab_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    tab_box.add_css_class("linked");
    tab_box.set_halign(gtk::Align::Start);
    let url_tab_btn = gtk::ToggleButton::with_label("Quick (URL)");
    let manual_tab_btn = gtk::ToggleButton::with_label("Advanced (Fields)");
    manual_tab_btn.set_group(Some(&url_tab_btn));
    url_tab_btn.set_active(true);
    tab_box.append(&url_tab_btn);
    tab_box.append(&manual_tab_btn);
    content.append(&tab_box);

    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_vhomogeneous(false);

    // URL page
    let url_group = adw::PreferencesGroup::new();
    url_group.set_title("Connection URL");
    let url_row = adw::EntryRow::builder().title("URL").build();
    url_group.add(&url_row);
    let url_hint = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dimmed", "caption"])
        .margin_top(6)
        .build();
    let url_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    url_page.append(&url_group);
    url_page.append(&url_hint);
    stack.add_named(&url_page, Some("url"));

    // Manual page
    let manual_group = adw::PreferencesGroup::new();
    manual_group.set_title("Connection details");

    let db_type_row = adw::ComboRow::builder().title("Database type").build();
    let db_type_model = gtk::StringList::new(&DB_TYPES.iter().map(|(_, l)| *l).collect::<Vec<_>>());
    db_type_row.set_model(Some(&db_type_model));
    manual_group.add(&db_type_row);

    let host_row = adw::EntryRow::builder().title("Host").build();
    manual_group.add(&host_row);
    let port_row = adw::SpinRow::builder()
        .title("Port")
        .adjustment(
            &gtk::Adjustment::builder()
                .lower(0.0)
                .upper(65535.0)
                .step_increment(1.0)
                .page_increment(10.0)
                .value(5432.0)
                .build(),
        )
        .digits(0)
        .build();
    manual_group.add(&port_row);
    let database_row = adw::EntryRow::builder().title("Database").build();
    manual_group.add(&database_row);
    let username_row = adw::EntryRow::builder().title("Username").build();
    manual_group.add(&username_row);
    let password_row = adw::PasswordEntryRow::builder().title("Password").build();
    manual_group.add(&password_row);

    let manual_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    manual_page.append(&manual_group);
    stack.add_named(&manual_page, Some("manual"));
    content.append(&stack);

    // ── Options ───────────────────────────────────────────────────────────
    let options = adw::PreferencesGroup::new();
    options.set_title("Options");

    let ssl_row = adw::ComboRow::builder().title("SSL mode").build();
    let ssl_model = gtk::StringList::new(&SSL_MODES.iter().map(|(_, l)| *l).collect::<Vec<_>>());
    ssl_row.set_model(Some(&ssl_model));
    options.add(&ssl_row);

    let group_row = adw::ComboRow::builder().title("Group").build();
    options.add(&group_row);

    let ssh_row = adw::ComboRow::builder()
        .title("SSH tunnel")
        .subtitle("Optional — uses a saved SSH profile")
        .build();
    options.add(&ssh_row);
    content.append(&options);

    let status_label = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .css_classes(["caption"])
        .build();
    content.append(&status_label);

    clamp.set_child(Some(&content));
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true)
        .child(&clamp)
        .build();
    toast_overlay.set_child(Some(&scrolled));
    toolbar.set_content(Some(&toast_overlay));
    dialog.set_child(Some(&toolbar));

    let is_docker = editing
        .as_ref()
        .map(|c| {
            matches!(
                c.connection_type,
                ConnectionType::DockerContainer | ConnectionType::LocalDockerContainer
            )
        })
        .unwrap_or(false);

    let state = Rc::new(FormState {
        editing_id: editing.as_ref().map(|c| c.id.clone()),
        is_docker,
        connection_type: editing.as_ref().map(|c| c.connection_type.clone()),
        container_name: editing.as_ref().and_then(|c| c.container_name.clone()),
        container_port: editing.as_ref().and_then(|c| c.container_port),
        color_id: RefCell::new(
            editing
                .as_ref()
                .map(|c| c.color_id.clone())
                .unwrap_or_else(|| "blue".into()),
        ),
        tab: Cell::new(FormTab::Url),
        groups: RefCell::new(vec![(None, "None".into())]),
        ssh_profiles: RefCell::new(vec![(None, "None".into())]),
    });

    // Color toggle buttons
    let mut first_color_btn: Option<gtk::ToggleButton> = None;
    for id in COLOR_IDS {
        let btn = gtk::ToggleButton::new();
        btn.set_tooltip_text(Some(id));
        btn.add_css_class("connection-color-dot");
        btn.add_css_class(color_css_class(id));
        btn.set_size_request(22, 22);
        if *id == state.color_id.borrow().as_str() {
            btn.set_active(true);
        }
        if let Some(first) = first_color_btn.as_ref() {
            btn.set_group(Some(first));
        } else {
            first_color_btn = Some(btn.clone());
        }
        btn.connect_toggled(glib::clone!(
            #[strong]
            state,
            #[strong]
            id,
            move |btn| {
                if btn.is_active() {
                    *state.color_id.borrow_mut() = id.to_string();
                }
            }
        ));
        color_box.append(&btn);
    }
    drop(first_color_btn);

    // Seed fields from editing info / saved URL.
    let mut preserved_password = None;
    if let Some(info) = editing.as_ref() {
        name_row.set_text(&info.name);
        url_hint.set_text(&format!("Saved URL (password masked): {}", info.masked_url));

        // Prefer full URL from disk so edit can preserve password.
        let saved = service.find_saved_connection(&info.id).ok();
        if let Some(saved) = saved {
            preserved_password = url::Url::parse(&saved.url)
                .ok()
                .and_then(|u| u.password().map(|p| p.to_string()));
            url_row.set_text(&saved.url);
            apply_parsed_to_fields(
                &parse_connection_url(&saved.url).unwrap_or(ParsedConnectionUrl {
                    db_type: info.db_type.clone(),
                    host: info.host.clone(),
                    port: info.port,
                    database: info.database.clone(),
                    username: info.username.clone(),
                    password: None,
                }),
                &UrlFields {
                    db_type_row: &db_type_row,
                    host_row: &host_row,
                    port_row: &port_row,
                    database_row: &database_row,
                    username_row: &username_row,
                    password_row: &password_row,
                    ssl_row: &ssl_row,
                },
                false,
            );
            select_ssl_mode(&ssl_row, parse_ssl_mode(&saved.url));
        } else {
            apply_info_to_fields(
                info,
                &UrlFields {
                    db_type_row: &db_type_row,
                    host_row: &host_row,
                    port_row: &port_row,
                    database_row: &database_row,
                    username_row: &username_row,
                    password_row: &password_row,
                    ssl_row: &ssl_row,
                },
            );
            select_ssl_mode(&ssl_row, parse_ssl_mode(&info.masked_url));
        }
    }

    // Keep the password from the saved URL when the password field is left blank.
    let preserved_password = Rc::new(RefCell::new(preserved_password));

    // Hide host/port for sqlite; relabel database.
    {
        let sqlite = selected_db_type(&db_type_row) == "sqlite";
        host_row.set_visible(!sqlite);
        port_row.set_visible(!sqlite);
        database_row.set_title(if sqlite { "File path" } else { "Database" });
    }

    // Tab switching with sync.
    url_tab_btn.connect_toggled(glib::clone!(
        #[strong]
        state,
        #[strong]
        preserved_password,
        #[weak]
        stack,
        #[weak]
        url_row,
        #[weak]
        db_type_row,
        #[weak]
        host_row,
        #[weak]
        port_row,
        #[weak]
        database_row,
        #[weak]
        username_row,
        #[weak]
        password_row,
        #[weak]
        ssl_row,
        move |btn| {
            if !btn.is_active() {
                return;
            }
            // Leaving manual → push fields into URL.
            if state.tab.get() == FormTab::Manual {
                let url = build_url_from_fields(
                    &UrlFields {
                        db_type_row: &db_type_row,
                        host_row: &host_row,
                        port_row: &port_row,
                        database_row: &database_row,
                        username_row: &username_row,
                        password_row: &password_row,
                        ssl_row: &ssl_row,
                    },
                    preserved_password.borrow().as_deref(),
                );
                if !url.is_empty() {
                    url_row.set_text(&url);
                }
            }
            state.tab.set(FormTab::Url);
            stack.set_visible_child_name("url");
        }
    ));

    manual_tab_btn.connect_toggled(glib::clone!(
        #[strong]
        state,
        #[strong]
        preserved_password,
        #[weak]
        stack,
        #[weak]
        url_row,
        #[weak]
        db_type_row,
        #[weak]
        host_row,
        #[weak]
        port_row,
        #[weak]
        database_row,
        #[weak]
        username_row,
        #[weak]
        password_row,
        #[weak]
        ssl_row,
        move |btn| {
            if !btn.is_active() {
                return;
            }
            if state.tab.get() == FormTab::Url {
                let raw = url_row.text();
                if !raw.trim().is_empty() {
                    if let Ok(parsed) = parse_connection_url(raw.trim()) {
                        apply_parsed_to_fields(
                            &parsed,
                            &UrlFields {
                                db_type_row: &db_type_row,
                                host_row: &host_row,
                                port_row: &port_row,
                                database_row: &database_row,
                                username_row: &username_row,
                                password_row: &password_row,
                                ssl_row: &ssl_row,
                            },
                            true,
                        );
                        select_ssl_mode(&ssl_row, parse_ssl_mode(raw.trim()));
                        if let Some(pw) = &parsed.password {
                            *preserved_password.borrow_mut() = Some(pw.clone());
                        }
                    }
                }
            }
            state.tab.set(FormTab::Manual);
            stack.set_visible_child_name("manual");
            let sqlite = selected_db_type(&db_type_row) == "sqlite";
            host_row.set_visible(!sqlite);
            port_row.set_visible(!sqlite);
            database_row.set_title(if sqlite { "File path" } else { "Database" });
        }
    ));

    db_type_row.connect_selected_notify(glib::clone!(
        #[weak]
        port_row,
        #[weak]
        db_type_row,
        #[weak]
        host_row,
        #[weak]
        database_row,
        move |_| {
            let dt = selected_db_type(&db_type_row);
            let default = default_port_for_db_type(&dt);
            if default > 0 {
                port_row.set_value(f64::from(default));
            }
            let sqlite = dt == "sqlite";
            host_row.set_visible(!sqlite);
            port_row.set_visible(!sqlite);
            database_row.set_title(if sqlite { "File path" } else { "Database" });
        }
    ));

    // Load groups + SSH profiles, then select current values.
    let initial_group_id = editing.as_ref().and_then(|c| c.group_id.clone());
    let initial_ssh_id = editing.as_ref().and_then(|c| c.ssh_profile_id.clone());
    {
        let service_g = service.clone();
        let service_s = service.clone();
        glib::spawn_future_local(glib::clone!(
            #[strong]
            state,
            #[strong]
            initial_group_id,
            #[strong]
            initial_ssh_id,
            #[weak]
            group_row,
            #[weak]
            ssh_row,
            async move {
                let groups_res = crate::spawn_tokio!(async move { service_g.get_groups().await })
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .unwrap_or_default();
                let profiles_res = crate::spawn_tokio!(async move { service_s.get_ssh_profiles() })
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .unwrap_or_default();

                let mut groups = vec![(None, "None".into())];
                let mut glist = groups_res;
                glist.sort_by(|a, b| a.name.cmp(&b.name));
                for g in glist {
                    groups.push((Some(g.id), g.name));
                }
                let labels: Vec<&str> = groups.iter().map(|(_, n)| n.as_str()).collect();
                group_row.set_model(Some(&gtk::StringList::new(&labels)));
                if let Some(want) = &initial_group_id {
                    if let Some(idx) = groups
                        .iter()
                        .position(|(id, _)| id.as_deref() == Some(want.as_str()))
                    {
                        group_row.set_selected(idx as u32);
                    }
                }
                *state.groups.borrow_mut() = groups;

                let mut profiles = vec![(None, "None".into())];
                let mut plist = profiles_res;
                plist.sort_by(|a, b| a.name.cmp(&b.name));
                for p in plist {
                    profiles.push((Some(p.id), p.name));
                }
                let labels: Vec<&str> = profiles.iter().map(|(_, n)| n.as_str()).collect();
                ssh_row.set_model(Some(&gtk::StringList::new(&labels)));
                if let Some(want) = &initial_ssh_id {
                    if let Some(idx) = profiles
                        .iter()
                        .position(|(id, _)| id.as_deref() == Some(want.as_str()))
                    {
                        ssh_row.set_selected(idx as u32);
                    }
                }
                *state.ssh_profiles.borrow_mut() = profiles;
            }
        ));
    }

    cancel_btn.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        move |_| {
            dialog.close();
        }
    ));

    // Test
    test_btn.connect_clicked(glib::clone!(
        #[strong]
        state,
        #[strong]
        preserved_password,
        #[strong]
        service,
        #[weak]
        toast_overlay,
        #[weak]
        status_label,
        #[weak]
        url_row,
        #[weak]
        db_type_row,
        #[weak]
        host_row,
        #[weak]
        port_row,
        #[weak]
        database_row,
        #[weak]
        username_row,
        #[weak]
        password_row,
        #[weak]
        ssl_row,
        #[weak]
        ssh_row,
        #[weak]
        test_btn,
        move |_| {
            let url = effective_url(
                state.tab.get(),
                &url_row,
                &UrlFields {
                    db_type_row: &db_type_row,
                    host_row: &host_row,
                    port_row: &port_row,
                    database_row: &database_row,
                    username_row: &username_row,
                    password_row: &password_row,
                    ssl_row: &ssl_row,
                },
                preserved_password.borrow().as_deref(),
            );
            if url.is_empty() {
                status_label.set_text("Enter a connection URL or fields to test.");
                status_label.add_css_class("error");
                return;
            }
            let ssh_id = selected_optional_id(&ssh_row, &state.ssh_profiles.borrow());
            let db_type = match state.tab.get() {
                FormTab::Url => parse_connection_url(&url)
                    .map(|p| p.db_type)
                    .unwrap_or_else(|_| selected_db_type(&db_type_row)),
                FormTab::Manual => selected_db_type(&db_type_row),
            };
            let container_name = state.container_name.clone();
            let container_port = state.container_port;
            let connection_type = state.connection_type.clone();
            let is_docker = state.is_docker;

            test_btn.set_sensitive(false);
            status_label.set_text("Testing…");
            status_label.remove_css_class("error");
            status_label.remove_css_class("success");

            let svc = service.clone();
            glib::spawn_future_local(async move {
                let result = if is_docker {
                    match connection_type {
                        Some(ConnectionType::DockerContainer) => {
                            let Some(profile_id) = ssh_id else {
                                test_btn.set_sensitive(true);
                                status_label.set_text(
                                    "SSH profile is required for remote Docker connections.",
                                );
                                status_label.add_css_class("error");
                                return;
                            };
                            let Some(name) = container_name else {
                                test_btn.set_sensitive(true);
                                status_label.set_text(
                                    "Container name is missing for this Docker connection.",
                                );
                                status_label.add_css_class("error");
                                return;
                            };
                            let u = url.clone();
                            let dt = db_type.clone();
                            crate::spawn_tokio!(async move {
                                svc.test_docker_connection(
                                    &profile_id,
                                    &name,
                                    container_port,
                                    &u,
                                    &dt,
                                )
                                .await
                            })
                            .await
                            .expect("join test_docker_connection")
                        }
                        Some(ConnectionType::LocalDockerContainer) => {
                            let Some(name) = container_name else {
                                test_btn.set_sensitive(true);
                                status_label.set_text(
                                    "Container name is missing for this Docker connection.",
                                );
                                status_label.add_css_class("error");
                                return;
                            };
                            let u = url.clone();
                            let dt = db_type.clone();
                            crate::spawn_tokio!(async move {
                                svc.test_local_docker_connection(&name, container_port, &u, &dt)
                                    .await
                            })
                            .await
                            .expect("join test_local_docker_connection")
                        }
                        _ => {
                            test_btn.set_sensitive(true);
                            status_label.set_text("Unknown Docker connection type.");
                            status_label.add_css_class("error");
                            return;
                        }
                    }
                } else {
                    match ssh_id {
                        Some(profile_id) => {
                            let u = url.clone();
                            crate::spawn_tokio!(async move {
                                svc.test_connection_with_ssh(&u, &profile_id).await
                            })
                            .await
                            .expect("join test_connection_with_ssh")
                        }
                        None => {
                            let u = url.clone();
                            crate::spawn_tokio!(async move { svc.test_connection(&u).await })
                                .await
                                .expect("join test_connection")
                        }
                    }
                };
                test_btn.set_sensitive(true);
                match result {
                    Ok(msg) => {
                        status_label.set_text(&msg);
                        status_label.add_css_class("success");
                        status_label.remove_css_class("error");
                        toast_overlay.add_toast(adw::Toast::new("Connection test succeeded"));
                    }
                    Err(e) => {
                        status_label.set_text(&e.to_string());
                        status_label.add_css_class("error");
                        status_label.remove_css_class("success");
                    }
                }
            });
        }
    ));

    // Save
    save_btn.connect_clicked(glib::clone!(
        #[strong]
        state,
        #[strong]
        preserved_password,
        #[strong]
        service,
        #[weak]
        window,
        #[weak]
        dialog,
        #[weak]
        status_label,
        #[weak]
        name_row,
        #[weak]
        url_row,
        #[weak]
        db_type_row,
        #[weak]
        host_row,
        #[weak]
        port_row,
        #[weak]
        database_row,
        #[weak]
        username_row,
        #[weak]
        password_row,
        #[weak]
        ssl_row,
        #[weak]
        group_row,
        #[weak]
        ssh_row,
        #[weak]
        save_btn,
        move |_| {
            let name = name_row.text().trim().to_string();
            if name.is_empty() {
                status_label.set_text("Name is required.");
                status_label.add_css_class("error");
                return;
            }
            let url = effective_url(
                state.tab.get(),
                &url_row,
                &UrlFields {
                    db_type_row: &db_type_row,
                    host_row: &host_row,
                    port_row: &port_row,
                    database_row: &database_row,
                    username_row: &username_row,
                    password_row: &password_row,
                    ssl_row: &ssl_row,
                },
                preserved_password.borrow().as_deref(),
            );
            if url.is_empty() {
                status_label.set_text("Connection URL is required.");
                status_label.add_css_class("error");
                return;
            }

            let config = ConnectionConfig {
                name,
                color_id: state.color_id.borrow().clone(),
                url,
                ssh_profile_id: selected_optional_id(&ssh_row, &state.ssh_profiles.borrow()),
                group_id: selected_optional_id(&group_row, &state.groups.borrow()),
                connection_type: state.connection_type.clone(),
                container_name: state.container_name.clone(),
                container_port: state.container_port,
            };

            save_btn.set_sensitive(false);
            let svc = service.clone();
            let editing_id = state.editing_id.clone();
            glib::spawn_future_local(async move {
                let result = match editing_id {
                    Some(id) => {
                        crate::spawn_tokio!(async move { svc.update_connection(id, config).await })
                            .await
                            .expect("join update_connection")
                    }
                    None => crate::spawn_tokio!(async move { svc.save_connection(config).await })
                        .await
                        .expect("join save_connection"),
                };
                save_btn.set_sensitive(true);
                match result {
                    Ok(info) => {
                        window.set_selected_connection_id(Some(info.id));
                        window.refresh_connections();
                        dialog.close();
                    }
                    Err(e) => {
                        status_label.set_text(&e.to_string());
                        status_label.add_css_class("error");
                    }
                }
            });
        }
    ));

    if state.is_docker {
        url_hint.set_text(
            "Docker connections: edit credentials here; create new ones with the Docker wizard.",
        );
    }

    dialog.present(Some(window));
}

fn selected_db_type(row: &adw::ComboRow) -> String {
    let idx = row.selected() as usize;
    DB_TYPES
        .get(idx)
        .map(|(id, _)| (*id).to_string())
        .unwrap_or_else(|| "postgres".into())
}

fn selected_ssl_mode(row: &adw::ComboRow) -> &'static str {
    let idx = row.selected() as usize;
    SSL_MODES.get(idx).map(|(id, _)| *id).unwrap_or("prefer")
}

fn select_ssl_mode(row: &adw::ComboRow, mode: &str) {
    if let Some(idx) = SSL_MODES.iter().position(|(id, _)| *id == mode) {
        row.set_selected(idx as u32);
    }
}

fn selected_optional_id(row: &adw::ComboRow, items: &[(Option<String>, String)]) -> Option<String> {
    let idx = row.selected() as usize;
    items.get(idx).and_then(|(id, _)| id.clone())
}

fn parse_ssl_mode(raw_url: &str) -> &'static str {
    let Ok(u) = url::Url::parse(raw_url) else {
        return "prefer";
    };
    let sslmode = u
        .query_pairs()
        .find(|(k, _)| k == "sslmode")
        .map(|(_, v)| v.to_string());
    let ssl_mode = u
        .query_pairs()
        .find(|(k, _)| k == "ssl-mode")
        .map(|(_, v)| v.to_string());
    if sslmode.as_deref() == Some("disable") || ssl_mode.as_deref() == Some("disabled") {
        return "disable";
    }
    if sslmode.as_deref() == Some("require") || ssl_mode.as_deref() == Some("required") {
        return "require";
    }
    "prefer"
}

fn build_ssl_param(db_type: &str, mode: &str) -> String {
    if mode == "prefer" {
        return String::new();
    }
    match db_type {
        "postgres" => {
            if mode == "disable" {
                "?sslmode=disable".into()
            } else {
                "?sslmode=require".into()
            }
        }
        "mysql" | "mariadb" => {
            if mode == "disable" {
                "?ssl-mode=disabled".into()
            } else {
                "?ssl-mode=required".into()
            }
        }
        _ => String::new(),
    }
}

fn strip_ssl_params(raw_url: &str) -> String {
    let Ok(mut u) = url::Url::parse(raw_url) else {
        return raw_url.to_string();
    };
    let pairs: Vec<(String, String)> = u
        .query_pairs()
        .filter(|(k, _)| k != "sslmode" && k != "ssl-mode" && k != "ssl")
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    if pairs.is_empty() {
        u.set_query(None);
    } else {
        u.query_pairs_mut().clear().extend_pairs(pairs);
    }
    u.to_string()
}

/// Manual + SSL widgets shared by URL sync helpers (avoids too-many-arguments).
struct UrlFields<'a> {
    db_type_row: &'a adw::ComboRow,
    host_row: &'a adw::EntryRow,
    port_row: &'a adw::SpinRow,
    database_row: &'a adw::EntryRow,
    username_row: &'a adw::EntryRow,
    password_row: &'a adw::PasswordEntryRow,
    ssl_row: &'a adw::ComboRow,
}

fn build_url_from_fields(fields: &UrlFields<'_>, preserved_password: Option<&str>) -> String {
    let db_type = selected_db_type(fields.db_type_row);
    let host = fields.host_row.text();
    if host.is_empty() && db_type != "sqlite" {
        return String::new();
    }
    let database = fields.database_row.text();
    let username = fields.username_row.text();
    let password = fields.password_row.text();
    let password = if password.is_empty() {
        preserved_password.unwrap_or("")
    } else {
        password.as_str()
    };
    let port = fields.port_row.value() as u16;
    let ssl = selected_ssl_mode(fields.ssl_row);

    if db_type == "sqlite" {
        return format!("sqlite://{database}");
    }

    let user_part = if username.is_empty() {
        String::new()
    } else if password.is_empty() {
        format!("{}@", encode_userinfo(&username))
    } else {
        format!(
            "{}:{}@",
            encode_userinfo(&username),
            encode_userinfo(password)
        )
    };
    format!(
        "{db_type}://{user_part}{host}:{port}/{database}{}",
        build_ssl_param(&db_type, ssl)
    )
}

fn encode_userinfo(s: &str) -> String {
    // Minimal encoding for userinfo — mirrors JS encodeURIComponent for common cases.
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn effective_url(
    tab: FormTab,
    url_row: &adw::EntryRow,
    fields: &UrlFields<'_>,
    preserved_password: Option<&str>,
) -> String {
    match tab {
        FormTab::Url => {
            let raw = url_row.text().trim().to_string();
            if raw.is_empty() {
                return String::new();
            }
            let db_type = parse_connection_url(&raw)
                .map(|p| p.db_type)
                .unwrap_or_else(|_| selected_db_type(fields.db_type_row));
            let ssl = selected_ssl_mode(fields.ssl_row);
            format!(
                "{}{}",
                strip_ssl_params(&raw),
                build_ssl_param(&db_type, ssl)
            )
        }
        FormTab::Manual => build_url_from_fields(fields, preserved_password),
    }
}

fn apply_parsed_to_fields(
    parsed: &ParsedConnectionUrl,
    fields: &UrlFields<'_>,
    set_password: bool,
) {
    if let Some(idx) = DB_TYPES.iter().position(|(id, _)| *id == parsed.db_type) {
        fields.db_type_row.set_selected(idx as u32);
    }
    fields.host_row.set_text(&parsed.host);
    fields.port_row.set_value(f64::from(parsed.port));
    fields.database_row.set_text(&parsed.database);
    fields.username_row.set_text(&parsed.username);
    if set_password {
        fields
            .password_row
            .set_text(parsed.password.as_deref().unwrap_or(""));
    } else {
        fields.password_row.set_text("");
    }
}

fn apply_info_to_fields(info: &ConnectionInfo, fields: &UrlFields<'_>) {
    if let Some(idx) = DB_TYPES.iter().position(|(id, _)| *id == info.db_type) {
        fields.db_type_row.set_selected(idx as u32);
    }
    fields.host_row.set_text(&info.host);
    fields.port_row.set_value(f64::from(info.port));
    fields.database_row.set_text(&info.database);
    fields.username_row.set_text(&info.username);
}

/// Open edit dialog for a sidebar connection id (loads ConnectionInfo list).
pub fn present_edit(window: &SqlatorWindow, app: &SqlatorApplication, connection_id: &str) {
    let service = app.service();
    let id = connection_id.to_string();
    glib::spawn_future_local(glib::clone!(
        #[weak]
        window,
        #[weak]
        app,
        async move {
            let svc = service.clone();
            let list = crate::spawn_tokio!(async move { svc.list_connections().await })
                .await
                .expect("join list_connections");
            match list {
                Ok(conns) => {
                    if let Some(info) = conns.into_iter().find(|c| c.id == id) {
                        present(&window, &app, Some(info));
                    } else {
                        let dialog = adw::AlertDialog::new(
                            Some("Connection not found"),
                            Some("It may have been deleted."),
                        );
                        dialog.add_response("ok", "OK");
                        dialog.present(Some(window.upcast_ref::<gtk::Widget>()));
                    }
                }
                Err(e) => {
                    let dialog = adw::AlertDialog::new(
                        Some("Failed to load connection"),
                        Some(&e.to_string()),
                    );
                    dialog.add_response("ok", "OK");
                    dialog.present(Some(window.upcast_ref::<gtk::Widget>()));
                }
            }
        }
    ));
}
