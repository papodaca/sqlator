//! Feature-gated VTE database CLI terminal.
//!
//! Reuses the active SSH tunnel local port via `AppService` and never puts
//! postgres/mysql passwords on argv (`PGPASSWORD` / `MYSQL_PWD` env instead).

use crate::application::SqlatorApplication;
use crate::window::SqlatorWindow;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::gdk;
use gtk::gio;
use gtk::{glib, Orientation};
use sqlator_core::models::ConnectionType;
use sqlator_core::ssh::{AuthMethod, SshAuthConfig};
use sqlator_service::{build_auth_config_for_profile, build_cli_for_connection, CliSpec};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use vte::prelude::*;

/// Bottom-panel host packed into `SqlatorWindow`'s `terminal_host` box.
#[derive(Debug)]
pub struct TerminalPanel {
    root: gtk::Box,
    terminal: vte::Terminal,
    status: gtk::Label,
    connection_id: RefCell<Option<String>>,
}

impl TerminalPanel {
    pub fn attach(window: &SqlatorWindow) -> Rc<Self> {
        let host = window.imp().terminal_host.get();

        let header = gtk::Box::new(Orientation::Horizontal, 6);
        header.set_margin_start(8);
        header.set_margin_end(8);
        header.set_margin_top(4);
        header.set_margin_bottom(4);

        let title = gtk::Label::new(Some("Database Terminal"));
        title.add_css_class("heading");
        title.set_xalign(0.0);
        title.set_hexpand(true);

        let status = gtk::Label::new(None);
        status.add_css_class("dim-label");
        status.set_xalign(1.0);

        let restart = gtk::Button::from_icon_name("view-refresh-symbolic");
        restart.set_tooltip_text(Some("Restart terminal"));
        restart.add_css_class("flat");

        let close = gtk::Button::from_icon_name("window-close-symbolic");
        close.set_tooltip_text(Some("Hide terminal"));
        close.add_css_class("flat");
        close.set_action_name(Some("win.toggle-terminal"));

        header.append(&title);
        header.append(&status);
        header.append(&restart);
        header.append(&close);

        let terminal = vte::Terminal::new();
        terminal.set_hexpand(true);
        terminal.set_vexpand(true);
        terminal.set_size_request(-1, 160);
        terminal.set_scroll_on_output(true);
        terminal.set_scroll_on_keystroke(true);
        apply_terminal_colors(&terminal);

        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_child(Some(&terminal));
        scrolled.set_hexpand(true);
        scrolled.set_vexpand(true);
        scrolled.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);

        let root = gtk::Box::new(Orientation::Vertical, 0);
        root.set_hexpand(true);
        root.set_vexpand(true);
        root.append(&header);
        root.append(&gtk::Separator::new(Orientation::Horizontal));
        root.append(&scrolled);

        while let Some(child) = host.first_child() {
            host.remove(&child);
        }
        host.append(&root);

        let panel = Rc::new(Self {
            root,
            terminal,
            status,
            connection_id: RefCell::new(None),
        });

        adw::StyleManager::default().connect_dark_notify(glib::clone!(
            #[weak(rename_to = term)]
            panel.terminal,
            move |_| {
                apply_terminal_colors(&term);
            }
        ));

        restart.connect_clicked(glib::clone!(
            #[weak]
            panel,
            #[weak]
            window,
            move |_| {
                panel.spawn_for_window(&window);
            }
        ));

        panel
    }

    pub fn root(&self) -> &gtk::Box {
        &self.root
    }

    pub fn set_visible_in(&self, window: &SqlatorWindow, visible: bool) {
        let host = window.imp().terminal_host.get();
        host.set_visible(visible);
        if visible {
            let paned = window.imp().terminal_paned.get();
            // Keep a usable bottom strip when revealing.
            let total = paned.height();
            if total > 0 {
                paned.set_position((total * 2) / 3);
            }
            self.spawn_for_window(window);
            self.terminal.grab_focus();
        }
    }

    pub fn is_visible_in(&self, window: &SqlatorWindow) -> bool {
        window.imp().terminal_host.is_visible()
    }

    pub fn on_workspace_changed(&self, window: &SqlatorWindow) {
        if !self.is_visible_in(window) {
            return;
        }
        let active = window.imp().active_workspace_id.borrow().clone();
        if active != *self.connection_id.borrow() {
            self.spawn_for_window(window);
        }
    }

    pub fn spawn_for_window(&self, window: &SqlatorWindow) {
        let Some(connection_id) = window.imp().active_workspace_id.borrow().clone() else {
            self.status
                .set_text("Open a connection to use the database terminal");
            self.terminal.reset(true, true);
            *self.connection_id.borrow_mut() = None;
            return;
        };

        let app = match window.application().and_downcast::<SqlatorApplication>() {
            Some(app) => app,
            None => return,
        };
        let service = app.service();

        let conn = match service.find_saved_connection(&connection_id) {
            Ok(c) => c,
            Err(e) => {
                self.fail(&e.to_string());
                return;
            }
        };

        let tunnel_port = service.tunnel_local_port_for_connection(&connection_id);

        let spec = match &conn.connection_type {
            ConnectionType::DockerContainer => {
                let ssh_profile_id = match conn.ssh_profile_id.as_deref() {
                    Some(id) => id,
                    None => {
                        self.fail("DockerContainer connection requires an SSH profile");
                        return;
                    }
                };
                let profile = match service.config().get_ssh_profile(ssh_profile_id) {
                    Ok(Some(p)) => p,
                    Ok(None) => {
                        self.fail(&format!("SSH profile '{ssh_profile_id}' not found"));
                        return;
                    }
                    Err(e) => {
                        self.fail(&e.to_string());
                        return;
                    }
                };
                let auth = match build_auth_config_for_profile(&profile, service.credentials()) {
                    Ok(a) => a,
                    Err(e) => {
                        self.fail(&e.message());
                        return;
                    }
                };
                match build_cli_for_connection(&conn, tunnel_port, Some((&profile, &auth))) {
                    Ok(spec) => maybe_wrap_sshpass(spec, &auth),
                    Err(e) => {
                        self.fail(&e);
                        return;
                    }
                }
            }
            _ => match build_cli_for_connection(&conn, tunnel_port, None) {
                Ok(spec) => spec,
                Err(e) => {
                    self.fail(&e);
                    return;
                }
            },
        };

        let binary_path = match resolve_binary(&spec.binary) {
            Ok(p) => p,
            Err(e) => {
                self.fail(&e);
                return;
            }
        };

        self.terminal.reset(true, true);
        *self.connection_id.borrow_mut() = Some(connection_id.clone());

        let tunnel_note = match tunnel_port {
            Some(port) => format!(" via tunnel :{port}"),
            None => String::new(),
        };
        self.status.set_text(&format!(
            "{} · {}{}",
            conn.db_type,
            binary_path.display(),
            tunnel_note
        ));

        let argv_owned: Vec<String> = std::iter::once(binary_path.to_string_lossy().into_owned())
            .chain(spec.args.iter().cloned())
            .collect();
        let env_owned: Vec<String> = spec.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let argv_refs: Vec<&str> = argv_owned.iter().map(String::as_str).collect();
        let env_refs: Vec<&str> = env_owned.iter().map(String::as_str).collect();

        let term = self.terminal.clone();
        let status = self.status.clone();

        // VTE copies argv/env before returning; owned buffers only need to live
        // through this call.
        term.clone().spawn_async(
            vte::PtyFlags::DEFAULT,
            None,
            &argv_refs,
            &env_refs,
            glib::SpawnFlags::DEFAULT,
            || {},
            -1,
            None::<&gio::Cancellable>,
            move |result| match result {
                Ok(pid) => {
                    term.watch_child(pid);
                }
                Err(e) => {
                    status.set_text(&format!("Spawn failed: {e}"));
                }
            },
        );
    }

    fn fail(&self, message: &str) {
        self.status.set_text(message);
        self.terminal.reset(true, true);
        tracing::warn!("terminal: {message}");
    }
}

fn apply_terminal_colors(term: &vte::Terminal) {
    let dark = adw::StyleManager::default().is_dark();
    let (fg, bg) = if dark {
        ("#d4d4d4", "#1e1e1e")
    } else {
        ("#333333", "#ffffff")
    };
    if let (Ok(fg), Ok(bg)) = (gdk::RGBA::parse(fg), gdk::RGBA::parse(bg)) {
        term.set_color_foreground(&fg);
        term.set_color_background(&bg);
    }
}

fn resolve_binary(name: &str) -> Result<PathBuf, String> {
    which::which(name)
        .map_err(|_| format!("{name} not found on PATH. Install the appropriate client tools."))
}

fn maybe_wrap_sshpass(mut spec: CliSpec, auth: &SshAuthConfig) -> CliSpec {
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
    CliSpec {
        binary: sshpass_path.to_string_lossy().to_string(),
        args,
        env: spec.env,
    }
}
