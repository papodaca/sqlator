//! Vault unlock gating on startup (Argon2 runs off the UI thread via AppService).

use crate::application::SqlatorApplication;
use crate::window::SqlatorWindow;
use adw::prelude::*;
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

type ReadyCb = Rc<RefCell<Option<Box<dyn FnOnce()>>>>;

/// If storage mode is vault and the vault is locked, present a modal unlock
/// dialog. Invokes `on_ready` once the vault is unlocked, or immediately when
/// unlock is not required.
pub fn gate_startup(window: &SqlatorWindow, on_ready: impl FnOnce() + 'static) {
    let Some(app) = window.application().and_downcast::<SqlatorApplication>() else {
        on_ready();
        return;
    };
    let service = app.service();
    let on_ready: ReadyCb = Rc::new(RefCell::new(Some(Box::new(on_ready))));

    glib::spawn_future_local(glib::clone!(
        #[strong]
        window,
        #[strong]
        on_ready,
        async move {
            let svc = service.clone();
            let mode = crate::spawn_tokio!(async move { svc.get_storage_mode().await })
                .await
                .expect("join get_storage_mode");
            let Ok(mode) = mode else {
                finish(&on_ready);
                return;
            };
            if mode != "vault" {
                finish(&on_ready);
                return;
            }

            let svc = service.clone();
            let exists = crate::spawn_tokio!(async move { svc.vault_exists().await })
                .await
                .expect("join vault_exists");
            let Ok(true) = exists else {
                finish(&on_ready);
                return;
            };

            let svc = service.clone();
            let locked = crate::spawn_tokio!(async move { svc.is_vault_locked().await })
                .await
                .expect("join is_vault_locked");
            let Ok(true) = locked else {
                finish(&on_ready);
                return;
            };

            present_unlock(&window, &service, on_ready);
        }
    ));
}

fn finish(on_ready: &ReadyCb) {
    if let Some(cb) = on_ready.borrow_mut().take() {
        cb();
    }
}

fn present_unlock(
    window: &SqlatorWindow,
    service: &Arc<sqlator_service::AppService>,
    on_ready: ReadyCb,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title("Vault Locked");
    dialog.set_content_width(380);
    // Must unlock to continue — Escape / close should not dismiss.
    dialog.set_can_close(false);

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_show_end_title_buttons(false);
    header.set_show_start_title_buttons(false);
    toolbar.add_top_bar(&header);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(18);
    content.set_margin_start(18);
    content.set_margin_end(18);

    let icon = gtk::Image::from_icon_name("dialog-password-symbolic");
    icon.set_pixel_size(48);
    icon.set_halign(gtk::Align::Center);
    content.append(&icon);

    let desc = gtk::Label::builder()
        .label(
            "Your credential vault is locked. Enter your master password to access SSH profile credentials.",
        )
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dimmed"])
        .build();
    content.append(&desc);

    let password = adw::PasswordEntryRow::builder()
        .title("Master password")
        .build();
    let group = adw::PreferencesGroup::new();
    group.add(&password);
    content.append(&group);

    let error = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .css_classes(["error"])
        .build();
    content.append(&error);

    let unlock_btn = gtk::Button::with_label("Unlock Vault");
    unlock_btn.add_css_class("suggested-action");
    unlock_btn.add_css_class("pill");
    unlock_btn.set_sensitive(false);
    content.append(&unlock_btn);

    toolbar.set_content(Some(&content));
    dialog.set_child(Some(&toolbar));

    password.connect_changed(glib::clone!(
        #[weak]
        unlock_btn,
        move |row| {
            unlock_btn.set_sensitive(!row.text().trim().is_empty());
        }
    ));

    let do_unlock = glib::clone!(
        #[strong]
        service,
        #[strong]
        on_ready,
        #[weak]
        dialog,
        #[weak]
        password,
        #[weak]
        unlock_btn,
        #[weak]
        error,
        move || {
            let pwd = password.text().trim().to_string();
            if pwd.is_empty() {
                return;
            }
            unlock_btn.set_sensitive(false);
            unlock_btn.set_label("Unlocking…");
            error.set_visible(false);
            let service = service.clone();
            glib::spawn_future_local(async move {
                let svc = service.clone();
                let result = crate::spawn_tokio!(async move { svc.unlock_vault(pwd).await })
                    .await
                    .expect("join unlock_vault");
                match result {
                    Ok(()) => {
                        dialog.set_can_close(true);
                        dialog.close();
                        finish(&on_ready);
                    }
                    Err(e) => {
                        error.set_text(&e.to_string());
                        error.set_visible(true);
                        unlock_btn.set_label("Unlock Vault");
                        unlock_btn.set_sensitive(!password.text().trim().is_empty());
                        password.grab_focus();
                    }
                }
            });
        }
    );

    unlock_btn.connect_clicked(glib::clone!(
        #[strong]
        do_unlock,
        move |_| do_unlock.clone()()
    ));
    password.connect_entry_activated(glib::clone!(
        #[strong]
        do_unlock,
        move |_| do_unlock.clone()()
    ));

    dialog.present(Some(window));
    password.grab_focus();
}
