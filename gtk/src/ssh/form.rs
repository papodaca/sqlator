//! SSH profile create/edit dialog (Svelte `SshProfileForm` + `SshHostDropdown` parity).

use crate::application::SqlatorApplication;
use adw::prelude::*;
use gtk::glib;
use sqlator_core::models::{SshAuthMethod, SshProfile};
use sqlator_service::{HostEntry, SshProfileConfig};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq)]
enum AuthTab {
    Key,
    Password,
    Agent,
}

impl AuthTab {
    fn as_wire(self) -> &'static str {
        match self {
            AuthTab::Key => "key",
            AuthTab::Password => "password",
            AuthTab::Agent => "agent",
        }
    }

    fn from_method(m: &SshAuthMethod) -> Self {
        match m {
            SshAuthMethod::Key => AuthTab::Key,
            SshAuthMethod::Password => AuthTab::Password,
            SshAuthMethod::Agent => AuthTab::Agent,
        }
    }
}

struct FormState {
    editing_id: Option<String>,
    auth: Cell<AuthTab>,
    /// Index 0 is "None" (no import). Remaining match `hosts`.
    hosts: RefCell<Vec<HostEntry>>,
}

/// Present a create dialog. `on_saved` runs on the UI thread after a successful save.
pub fn present(
    parent: &impl IsA<gtk::Widget>,
    app: &SqlatorApplication,
    on_saved: impl Fn(SshProfile) + 'static,
) {
    present_inner(parent, app, None, Rc::new(on_saved), None);
}

/// Present an edit dialog for an existing profile.
///
/// `on_deleted` runs on the UI thread after a successful delete (before the dialog closes).
pub fn present_edit(
    parent: &impl IsA<gtk::Widget>,
    app: &SqlatorApplication,
    profile: SshProfile,
    on_saved: impl Fn(SshProfile) + 'static,
    on_deleted: impl Fn() + 'static,
) {
    present_inner(
        parent,
        app,
        Some(profile),
        Rc::new(on_saved),
        Some(Rc::new(on_deleted)),
    );
}

fn present_inner(
    parent: &impl IsA<gtk::Widget>,
    app: &SqlatorApplication,
    editing: Option<SshProfile>,
    on_saved: Rc<dyn Fn(SshProfile)>,
    on_deleted: Option<Rc<dyn Fn()>>,
) {
    let service = app.service();
    let dialog = adw::Dialog::new();
    dialog.set_content_width(520);
    dialog.set_title(if editing.is_some() {
        "Edit SSH Profile"
    } else {
        "New SSH Profile"
    });

    let toast_overlay = adw::ToastOverlay::new();
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();

    let cancel_btn = gtk::Button::with_label("Cancel");
    cancel_btn.add_css_class("flat");
    let save_btn = gtk::Button::with_label("Save");
    save_btn.add_css_class("suggested-action");

    header.pack_start(&cancel_btn);
    header.pack_end(&save_btn);

    if let (Some(profile), Some(on_deleted)) = (editing.as_ref(), on_deleted.clone()) {
        let delete_btn = gtk::Button::with_label("Delete");
        delete_btn.add_css_class("destructive-action");
        header.pack_start(&delete_btn);
        wire_delete(
            &delete_btn,
            &dialog,
            &toast_overlay,
            service.clone(),
            profile.id.clone(),
            on_deleted,
        );
    }

    toolbar.add_top_bar(&header);

    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(480);
    clamp.set_tightening_threshold(360);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
    content.set_margin_top(18);
    content.set_margin_bottom(24);
    content.set_margin_start(18);
    content.set_margin_end(18);

    // ── Import from ~/.ssh/config ─────────────────────────────────────────
    let import_group = adw::PreferencesGroup::new();
    import_group.set_title("Import");
    import_group.set_description(Some("Auto-fill from hosts in ~/.ssh/config"));
    let import_row = adw::ComboRow::builder()
        .title("SSH config host")
        .subtitle("Optional")
        .build();
    import_group.add(&import_row);
    content.append(&import_group);

    // ── General ───────────────────────────────────────────────────────────
    let general = adw::PreferencesGroup::new();
    general.set_title("General");
    let name_row = adw::EntryRow::builder().title("Profile name").build();
    general.add(&name_row);
    let host_row = adw::EntryRow::builder().title("SSH host").build();
    general.add(&host_row);
    let port_row = adw::SpinRow::builder()
        .title("Port")
        .adjustment(
            &gtk::Adjustment::builder()
                .lower(1.0)
                .upper(65535.0)
                .step_increment(1.0)
                .page_increment(10.0)
                .value(22.0)
                .build(),
        )
        .digits(0)
        .build();
    general.add(&port_row);
    let username_row = adw::EntryRow::builder().title("Username").build();
    general.add(&username_row);
    content.append(&general);

    // ── Auth tabs ─────────────────────────────────────────────────────────
    let auth_group = adw::PreferencesGroup::new();
    auth_group.set_title("Authentication");

    let auth_tab_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    auth_tab_box.add_css_class("linked");
    auth_tab_box.set_halign(gtk::Align::Start);
    auth_tab_box.set_margin_bottom(6);
    let key_tab = gtk::ToggleButton::with_label("Key file");
    let password_tab = gtk::ToggleButton::with_label("Password");
    let agent_tab = gtk::ToggleButton::with_label("SSH agent");
    password_tab.set_group(Some(&key_tab));
    agent_tab.set_group(Some(&key_tab));
    key_tab.set_active(true);
    auth_tab_box.append(&key_tab);
    auth_tab_box.append(&password_tab);
    auth_tab_box.append(&agent_tab);

    let auth_stack = gtk::Stack::new();
    auth_stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    auth_stack.set_vhomogeneous(false);

    let key_page = adw::PreferencesGroup::new();
    let key_path_row = adw::EntryRow::builder().title("Identity file path").build();
    key_page.add(&key_path_row);
    let passphrase_row = adw::PasswordEntryRow::builder()
        .title("Key passphrase")
        .build();
    if editing.is_some() {
        passphrase_row.set_title("Key passphrase (leave blank to keep)");
    }
    key_page.add(&passphrase_row);
    auth_stack.add_named(&key_page, Some("key"));

    let password_page = adw::PreferencesGroup::new();
    let password_row = adw::PasswordEntryRow::builder().title("Password").build();
    if editing.is_some() {
        password_row.set_title("Password (leave blank to keep)");
    }
    password_page.add(&password_row);
    auth_stack.add_named(&password_page, Some("password"));

    let agent_page = gtk::Label::builder()
        .label("Credentials will be read from your running SSH agent (SSH_AUTH_SOCK).")
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dimmed", "caption"])
        .margin_top(6)
        .margin_bottom(6)
        .build();
    auth_stack.add_named(&agent_page, Some("agent"));

    let auth_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    auth_box.append(&auth_tab_box);
    auth_box.append(&auth_stack);
    // PreferencesGroup wants ActionRow children; wrap via ActionRow-less box under content.
    content.append(&auth_group);
    // Place tabs under the group title by appending after the empty group.
    content.append(&auth_box);

    // ── Advanced ──────────────────────────────────────────────────────────
    let advanced = adw::PreferencesGroup::new();
    advanced.set_title("Advanced");
    let local_bind_row = adw::SpinRow::builder()
        .title("Local port binding")
        .subtitle("0 = auto")
        .adjustment(
            &gtk::Adjustment::builder()
                .lower(0.0)
                .upper(65535.0)
                .step_increment(1.0)
                .page_increment(10.0)
                .value(0.0)
                .build(),
        )
        .digits(0)
        .build();
    advanced.add(&local_bind_row);
    let keepalive_row = adw::SpinRow::builder()
        .title("Keep-alive interval")
        .subtitle("Seconds; 0 = disabled")
        .adjustment(
            &gtk::Adjustment::builder()
                .lower(0.0)
                .upper(86400.0)
                .step_increment(1.0)
                .page_increment(10.0)
                .value(0.0)
                .build(),
        )
        .digits(0)
        .build();
    advanced.add(&keepalive_row);
    content.append(&advanced);

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

    let state = Rc::new(FormState {
        editing_id: editing.as_ref().map(|p| p.id.clone()),
        auth: Cell::new(AuthTab::Key),
        hosts: RefCell::new(Vec::new()),
    });

    if let Some(profile) = editing.as_ref() {
        name_row.set_text(&profile.name);
        host_row.set_text(&profile.host);
        port_row.set_value(f64::from(profile.port));
        username_row.set_text(&profile.username);
        if let Some(path) = &profile.key_path {
            key_path_row.set_text(path);
        }
        if let Some(bind) = profile.local_port_binding {
            local_bind_row.set_value(f64::from(bind));
        }
        if let Some(ka) = profile.keepalive_interval {
            keepalive_row.set_value(f64::from(ka));
        }
        let tab = AuthTab::from_method(&profile.auth_method);
        state.auth.set(tab);
        match tab {
            AuthTab::Key => key_tab.set_active(true),
            AuthTab::Password => password_tab.set_active(true),
            AuthTab::Agent => agent_tab.set_active(true),
        }
        auth_stack.set_visible_child_name(tab.as_wire());
    }

    // Auth tab switching
    key_tab.connect_toggled(glib::clone!(
        #[strong]
        state,
        #[weak]
        auth_stack,
        move |btn| {
            if btn.is_active() {
                state.auth.set(AuthTab::Key);
                auth_stack.set_visible_child_name("key");
            }
        }
    ));
    password_tab.connect_toggled(glib::clone!(
        #[strong]
        state,
        #[weak]
        auth_stack,
        move |btn| {
            if btn.is_active() {
                state.auth.set(AuthTab::Password);
                auth_stack.set_visible_child_name("password");
            }
        }
    ));
    agent_tab.connect_toggled(glib::clone!(
        #[strong]
        state,
        #[weak]
        auth_stack,
        move |btn| {
            if btn.is_active() {
                state.auth.set(AuthTab::Agent);
                auth_stack.set_visible_child_name("agent");
            }
        }
    ));

    // Validation: require name, host, username.
    let update_validity = {
        let name_row = name_row.clone();
        let host_row = host_row.clone();
        let username_row = username_row.clone();
        let save_btn = save_btn.clone();
        Rc::new(move || {
            let ok = !name_row.text().trim().is_empty()
                && !host_row.text().trim().is_empty()
                && !username_row.text().trim().is_empty();
            save_btn.set_sensitive(ok);
        })
    };
    update_validity();
    for row in [&name_row, &host_row, &username_row] {
        row.connect_changed(glib::clone!(
            #[strong]
            update_validity,
            move |_| update_validity()
        ));
    }

    // Load ~/.ssh/config hosts into the import picker.
    {
        let svc = service.clone();
        glib::spawn_future_local(glib::clone!(
            #[strong]
            state,
            #[weak]
            import_row,
            async move {
                let hosts = crate::spawn_tokio!(async move { svc.list_ssh_hosts() })
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .unwrap_or_default();
                let mut labels = vec!["— None —".to_string()];
                for h in &hosts {
                    let mut label = h.alias.clone();
                    let mut meta = format!("{}:{}", h.hostname, h.port);
                    if let Some(user) = &h.user {
                        meta = format!("{user}@{meta}");
                    }
                    if let Some(jump) = &h.proxy_jump {
                        meta = format!("{meta} · via {jump}");
                    }
                    label = format!("{label} ({meta})");
                    labels.push(label);
                }
                let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
                import_row.set_model(Some(&gtk::StringList::new(&refs)));
                import_row.set_selected(0);
                *state.hosts.borrow_mut() = hosts;
            }
        ));
    }

    import_row.connect_selected_notify(glib::clone!(
        #[strong]
        state,
        #[strong]
        update_validity,
        #[weak]
        name_row,
        #[weak]
        host_row,
        #[weak]
        port_row,
        #[weak]
        username_row,
        #[weak]
        key_path_row,
        #[weak]
        key_tab,
        #[weak]
        auth_stack,
        move |row| {
            let idx = row.selected() as usize;
            if idx == 0 {
                return;
            }
            let hosts = state.hosts.borrow();
            let Some(entry) = hosts.get(idx - 1) else {
                return;
            };
            host_row.set_text(&entry.hostname);
            port_row.set_value(f64::from(entry.port));
            if let Some(user) = &entry.user {
                username_row.set_text(user);
            }
            if let Some(identity) = &entry.identity_file {
                key_path_row.set_text(identity);
                key_tab.set_active(true);
                state.auth.set(AuthTab::Key);
                auth_stack.set_visible_child_name("key");
            }
            if name_row.text().trim().is_empty() {
                name_row.set_text(&entry.alias);
            }
            update_validity();
        }
    ));

    cancel_btn.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        move |_| {
            dialog.close();
        }
    ));

    save_btn.connect_clicked(glib::clone!(
        #[strong]
        state,
        #[strong]
        service,
        #[strong]
        on_saved,
        #[weak]
        dialog,
        #[weak]
        toast_overlay,
        #[weak]
        name_row,
        #[weak]
        host_row,
        #[weak]
        port_row,
        #[weak]
        username_row,
        #[weak]
        key_path_row,
        #[weak]
        passphrase_row,
        #[weak]
        password_row,
        #[weak]
        local_bind_row,
        #[weak]
        keepalive_row,
        #[weak]
        save_btn,
        #[weak]
        status_label,
        move |_| {
            let name = name_row.text().trim().to_string();
            let host = host_row.text().trim().to_string();
            let username = username_row.text().trim().to_string();
            if name.is_empty() || host.is_empty() || username.is_empty() {
                status_label.set_text("Name, host, and username are required.");
                status_label.add_css_class("error");
                return;
            }

            let auth = state.auth.get();
            let port = port_row.value() as u16;
            let key_path = {
                let t = key_path_row.text().trim().to_string();
                if auth == AuthTab::Key && !t.is_empty() {
                    Some(t)
                } else {
                    None
                }
            };
            let password = {
                let t = password_row.text();
                if auth == AuthTab::Password && !t.is_empty() {
                    Some(t.to_string())
                } else {
                    None
                }
            };
            let key_passphrase = {
                let t = passphrase_row.text();
                if auth == AuthTab::Key && !t.is_empty() {
                    Some(t.to_string())
                } else {
                    None
                }
            };
            let local_bind = {
                let v = local_bind_row.value() as u16;
                if v == 0 {
                    None
                } else {
                    Some(v)
                }
            };
            let keepalive = {
                let v = keepalive_row.value() as u32;
                if v == 0 {
                    None
                } else {
                    Some(v)
                }
            };

            let config = SshProfileConfig {
                name,
                host,
                port,
                username,
                auth_method: auth.as_wire().to_string(),
                key_path,
                password,
                key_passphrase,
                proxy_jump: Vec::new(),
                local_port_binding: local_bind,
                keepalive_interval: keepalive,
            };
            let editing_id = state.editing_id.clone();
            let svc = service.clone();
            let on_saved = on_saved.clone();

            save_btn.set_sensitive(false);
            status_label.set_text("Saving…");
            status_label.remove_css_class("error");

            glib::spawn_future_local(async move {
                let result = crate::spawn_tokio!(async move {
                    match editing_id {
                        Some(id) => svc.update_ssh_profile(&id, config),
                        None => svc.save_ssh_profile(config),
                    }
                })
                .await
                .expect("join save_ssh_profile");

                save_btn.set_sensitive(true);
                match result {
                    Ok(profile) => {
                        on_saved(profile);
                        dialog.close();
                    }
                    Err(e) => {
                        status_label.set_text(&e.message());
                        status_label.add_css_class("error");
                        toast_overlay.add_toast(adw::Toast::new(&e.message()));
                    }
                }
            });
        }
    ));

    dialog.present(Some(parent));
}

fn wire_delete(
    delete_btn: &gtk::Button,
    dialog: &adw::Dialog,
    toast_overlay: &adw::ToastOverlay,
    service: Arc<sqlator_service::AppService>,
    profile_id: String,
    on_deleted: Rc<dyn Fn()>,
) {
    delete_btn.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        #[weak]
        toast_overlay,
        #[strong]
        service,
        #[strong]
        profile_id,
        #[strong]
        on_deleted,
        move |_| {
            let svc = service.clone();
            let pid = profile_id.clone();
            glib::spawn_future_local(glib::clone!(
                #[weak]
                dialog,
                #[weak]
                toast_overlay,
                #[strong]
                service,
                #[strong]
                profile_id,
                #[strong]
                on_deleted,
                async move {
                    let users = crate::spawn_tokio!(async move {
                        svc.connections_using_ssh_profile(&pid)
                    })
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .unwrap_or_default();

                    let body = if users.is_empty() {
                        "Delete this SSH profile? This cannot be undone.".to_string()
                    } else {
                        format!(
                            "This profile is used by {} connection(s):\n{}\n\nDelete it anyway? Connections will lose their tunnel.",
                            users.len(),
                            users.join(", ")
                        )
                    };

                    let confirm = adw::AlertDialog::new(Some("Delete SSH profile?"), Some(&body));
                    confirm.add_response("cancel", "Cancel");
                    confirm.add_response("delete", "Delete");
                    confirm.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
                    confirm.set_default_response(Some("cancel"));
                    confirm.set_close_response("cancel");

                    confirm.connect_response(
                        None,
                        glib::clone!(
                            #[weak]
                            dialog,
                            #[weak]
                            toast_overlay,
                            #[strong]
                            service,
                            #[strong]
                            profile_id,
                            #[strong]
                            on_deleted,
                            move |_, response| {
                                if response != "delete" {
                                    return;
                                }
                                let svc = service.clone();
                                let pid = profile_id.clone();
                                let on_deleted = on_deleted.clone();
                                glib::spawn_future_local(async move {
                                    let result = crate::spawn_tokio!(async move {
                                        svc.delete_ssh_profile(&pid)
                                    })
                                    .await
                                    .expect("join delete_ssh_profile");
                                    match result {
                                        Ok(()) => {
                                            on_deleted();
                                            dialog.close();
                                        }
                                        Err(e) => {
                                            toast_overlay.add_toast(adw::Toast::new(&e.message()));
                                        }
                                    }
                                });
                            }
                        ),
                    );
                    confirm.present(Some(&dialog));
                }
            ));
        }
    ));
}
