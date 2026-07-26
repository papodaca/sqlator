//! Group create / rename / color dialogs (Svelte `GroupItem` parity).

use super::colors::{group_color_css_class, GROUP_COLOR_HEXES};
use crate::application::SqlatorApplication;
use crate::window::SqlatorWindow;
use adw::prelude::*;
use gtk::glib;
use sqlator_core::models::ConnectionGroup;
use sqlator_service::SaveGroupPayload;
use std::cell::RefCell;
use std::rc::Rc;

/// Prompt for a new root-level group name (+ optional color).
pub fn present_create(window: &SqlatorWindow) {
    let dialog = adw::AlertDialog::new(Some("New Group"), Some("Choose a name for the group."));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("create", "Create");
    dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("create"));
    dialog.set_close_response("cancel");

    let entry = gtk::Entry::builder()
        .placeholder_text("Group name")
        .activates_default(true)
        .hexpand(true)
        .build();
    let (swatches, selected) = color_swatch_row(None);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.set_margin_top(8);
    content.append(&entry);
    content.append(&swatches);
    dialog.set_extra_child(Some(&content));

    dialog.connect_response(
        None,
        glib::clone!(
            #[weak]
            window,
            #[strong]
            entry,
            #[strong]
            selected,
            move |_, response| {
                if response != "create" {
                    return;
                }
                let name = entry.text().trim().to_string();
                if name.is_empty() {
                    window.show_toast("Group name cannot be empty");
                    return;
                }
                let color = selected.borrow().clone();
                create_group(&window, name, color, None);
            }
        ),
    );
    dialog.present(Some(window));
    entry.grab_focus();
}

/// Rename an existing group.
pub fn present_rename(window: &SqlatorWindow, group: ConnectionGroup) {
    let dialog = adw::AlertDialog::new(Some("Rename Group"), None);
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("save", "Save");
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("save"));
    dialog.set_close_response("cancel");

    let entry = gtk::Entry::builder()
        .text(&group.name)
        .activates_default(true)
        .hexpand(true)
        .build();
    dialog.set_extra_child(Some(&entry));

    dialog.connect_response(
        None,
        glib::clone!(
            #[weak]
            window,
            #[strong]
            entry,
            #[strong]
            group,
            move |_, response| {
                if response != "save" {
                    return;
                }
                let name = entry.text().trim().to_string();
                if name.is_empty() || name == group.name {
                    return;
                }
                let mut updated = group.clone();
                updated.name = name;
                persist_group(&window, updated);
            }
        ),
    );
    dialog.present(Some(window));
    entry.grab_focus();
    entry.select_region(0, -1);
}

/// Color picker dialog for a group.
pub fn present_color_picker(window: &SqlatorWindow, group: ConnectionGroup) {
    let dialog = adw::AlertDialog::new(Some("Group Color"), Some("Pick a color for this group."));
    dialog.add_response("cancel", "Cancel");
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let (swatches, _selected) = color_swatch_row(group.color.as_deref());
    let clear = gtk::Button::with_label("Clear color");
    clear.add_css_class("flat");

    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    content.append(&swatches);
    content.append(&clear);
    dialog.set_extra_child(Some(&content));

    // Apply when a swatch is activated.
    let mut child = swatches.first_child();
    let mut idx = 0usize;
    while let Some(widget) = child {
        let next = widget.next_sibling();
        if let Ok(btn) = widget.downcast::<gtk::Button>() {
            if let Some(hex) = GROUP_COLOR_HEXES.get(idx).copied() {
                btn.connect_clicked(glib::clone!(
                    #[weak]
                    window,
                    #[strong]
                    group,
                    #[weak]
                    dialog,
                    move |_| {
                        let mut updated = group.clone();
                        updated.color = Some(hex.to_string());
                        persist_group(&window, updated);
                        dialog.close();
                    }
                ));
            }
            idx += 1;
        }
        child = next;
    }

    clear.connect_clicked(glib::clone!(
        #[weak]
        window,
        #[strong]
        group,
        #[weak]
        dialog,
        move |_| {
            let mut updated = group.clone();
            updated.color = None;
            persist_group(&window, updated);
            dialog.close();
        }
    ));

    dialog.present(Some(window));
}

fn create_group(
    window: &SqlatorWindow,
    name: String,
    color: Option<String>,
    parent_group_id: Option<String>,
) {
    let Some(app) = window.application().and_downcast::<SqlatorApplication>() else {
        return;
    };
    let service = app.service();
    glib::spawn_future_local(glib::clone!(
        #[weak]
        window,
        async move {
            let svc = service.clone();
            let payload = SaveGroupPayload {
                name,
                color,
                parent_group_id,
            };
            let result = crate::spawn_tokio!(async move { svc.save_group(payload).await })
                .await
                .expect("join save_group");
            match result {
                Ok(_) => window.refresh_connections(),
                Err(e) => window.show_toast(&format!("Could not create group: {e}")),
            }
        }
    ));
}

fn persist_group(window: &SqlatorWindow, group: ConnectionGroup) {
    let Some(app) = window.application().and_downcast::<SqlatorApplication>() else {
        return;
    };
    let service = app.service();
    glib::spawn_future_local(glib::clone!(
        #[weak]
        window,
        async move {
            let svc = service.clone();
            let result = crate::spawn_tokio!(async move { svc.update_group(group).await })
                .await
                .expect("join update_group");
            match result {
                Ok(_) => window.refresh_connections(),
                Err(e) => window.show_toast(&format!("Could not update group: {e}")),
            }
        }
    ));
}

fn color_swatch_row(initial: Option<&str>) -> (gtk::Box, Rc<RefCell<Option<String>>>) {
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    root.set_halign(gtk::Align::Center);
    let selected = Rc::new(RefCell::new(initial.map(str::to_string)));
    for hex in GROUP_COLOR_HEXES {
        let btn = gtk::Button::new();
        btn.set_tooltip_text(Some(hex));
        btn.add_css_class("flat");
        btn.add_css_class("circular");
        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class("connection-color-dot");
        dot.add_css_class(group_color_css_class(Some(hex)));
        dot.set_size_request(18, 18);
        btn.set_child(Some(&dot));
        if initial.is_some_and(|c| c.eq_ignore_ascii_case(hex)) {
            btn.add_css_class("suggested-action");
        }
        btn.connect_clicked(glib::clone!(
            #[strong]
            selected,
            move |btn| {
                *selected.borrow_mut() = Some((*hex).to_string());
                if let Some(parent) = btn.parent() {
                    let mut child = parent.first_child();
                    while let Some(c) = child {
                        c.remove_css_class("suggested-action");
                        child = c.next_sibling();
                    }
                }
                btn.add_css_class("suggested-action");
            }
        ));
        root.append(&btn);
    }
    (root, selected)
}
