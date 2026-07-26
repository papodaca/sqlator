//! Builds and drives the sidebar `ListView` of groups + connections.

use super::colors::{color_css_class, group_color_css_class};
use super::item::{ConnectionStatus, SidebarItem, SidebarKind};
use crate::window::SqlatorWindow;
use adw::prelude::*;
use gtk::{gdk, gio, glib, pango};
use sqlator_core::models::{ConnectionGroup, ConnectionInfo};
use std::collections::HashMap;
use std::sync::Arc;

const ROW_DATA_KEY: &str = "sqlator-row";
const ITEM_DATA_KEY: &str = "sqlator-sidebar-item";

/// Owns the sidebar model and wires selection / activation / context menus.
pub struct ConnectionList {
    store: gio::ListStore,
    selection: gtk::SingleSelection,
}

impl std::fmt::Debug for ConnectionList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionList")
            .field("n_items", &self.store.n_items())
            .finish()
    }
}

impl ConnectionList {
    pub fn attach(window: &SqlatorWindow, list_view: &gtk::ListView) -> Self {
        let store = gio::ListStore::new::<SidebarItem>();
        let selection = gtk::SingleSelection::new(Some(store.clone()));
        selection.set_autoselect(false);
        selection.set_can_unselect(true);

        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(glib::clone!(
            #[weak]
            window,
            move |_, item| {
                let list_item = item
                    .downcast_ref::<gtk::ListItem>()
                    .expect("ListItem in factory setup");
                let row = build_row_widget();
                install_context_menu(&window, &row.root);
                list_item.set_child(Some(&row.root));
                // Safety: RowWidgets is 'static; removed automatically when ListItem drops.
                unsafe {
                    list_item.set_data(ROW_DATA_KEY, row);
                }
            }
        ));
        factory.connect_bind(|_, item| {
            let list_item = item
                .downcast_ref::<gtk::ListItem>()
                .expect("ListItem in factory bind");
            let Some(sidebar_item) = list_item.item().and_downcast::<SidebarItem>() else {
                return;
            };
            let row = unsafe {
                list_item
                    .data::<RowWidgets>(ROW_DATA_KEY)
                    .map(|p| p.as_ref().clone())
            };
            let Some(row) = row else {
                return;
            };
            bind_row(&row, &sidebar_item);
            unsafe {
                row.root.set_data(ITEM_DATA_KEY, sidebar_item);
            }
        });
        factory.connect_unbind(|_, item| {
            let list_item = item
                .downcast_ref::<gtk::ListItem>()
                .expect("ListItem in factory unbind");
            if let Some(child) = list_item.child() {
                unsafe {
                    child.steal_data::<SidebarItem>(ITEM_DATA_KEY);
                }
            }
        });

        list_view.set_factory(Some(&factory));
        list_view.set_model(Some(&selection));
        list_view.set_single_click_activate(true);

        // Highlight only — ListView selection can change on focus/hover without
        // activate; schema must not follow that (see set_schema_connection_id).
        selection.connect_selection_changed(glib::clone!(
            #[weak]
            window,
            move |selection, _, _| {
                if let Some(item) = selection.selected_item().and_downcast::<SidebarItem>() {
                    if let Some(id) = item.connection_id() {
                        window.set_selected_connection_id(Some(id));
                    }
                }
            }
        ));

        list_view.connect_activate(glib::clone!(
            #[weak]
            window,
            move |list_view, position| {
                let Some(model) = list_view.model() else {
                    return;
                };
                let Some(item) = model.item(position).and_downcast::<SidebarItem>() else {
                    return;
                };
                match item.kind() {
                    SidebarKind::Group { id, collapsed, .. } => {
                        window.toggle_group_collapsed(&id, !collapsed);
                    }
                    SidebarKind::Connection { id, status, .. } => {
                        // Selection may follow focus/hover; schema / workspace follow activate.
                        window.set_selected_connection_id(Some(id.clone()));
                        window.set_schema_connection_id(Some(id.clone()));
                        let newly_opened = window.open_connection_workspace(&id);
                        // Match Svelte ConnectionItem: connect only when first opening.
                        if newly_opened
                            && !matches!(
                                status,
                                ConnectionStatus::Connected | ConnectionStatus::Connecting
                            )
                        {
                            window.connect_sidebar_connection(&id);
                        }
                    }
                }
            }
        ));

        Self { store, selection }
    }

    pub fn store(&self) -> gio::ListStore {
        self.store.clone()
    }

    pub fn selection(&self) -> gtk::SingleSelection {
        self.selection.clone()
    }

    pub fn selected_item(&self) -> Option<SidebarItem> {
        self.selection.selected_item().and_downcast::<SidebarItem>()
    }

    /// Rebuild the flat list from groups + connections, preserving selection.
    pub fn rebuild(
        &self,
        groups: Vec<ConnectionGroup>,
        connections: Vec<ConnectionInfo>,
        connected: &HashMap<String, ConnectionStatus>,
        selected_id: Option<&str>,
    ) {
        let previous = selected_id
            .map(str::to_string)
            .or_else(|| self.selected_item().and_then(|i| i.connection_id()));

        let items = flatten_sidebar(groups, connections, connected);
        self.store.remove_all();
        let mut select_pos: Option<u32> = None;
        for (idx, kind) in items.into_iter().enumerate() {
            if let (Some(want), SidebarKind::Connection { id, .. }) = (previous.as_deref(), &kind) {
                if id == want {
                    select_pos = Some(idx as u32);
                }
            }
            self.store.append(&SidebarItem::new(kind));
        }

        if let Some(pos) = select_pos {
            self.selection.set_selected(pos);
        } else {
            self.selection.set_selected(gtk::INVALID_LIST_POSITION);
        }
    }

    pub fn set_status_for(&self, connection_id: &str, status: ConnectionStatus) {
        for i in 0..self.store.n_items() {
            let Some(item) = self.store.item(i).and_downcast::<SidebarItem>() else {
                continue;
            };
            if item.connection_id().as_deref() != Some(connection_id) {
                continue;
            }
            let mut kind = item.kind();
            if let SidebarKind::Connection { status: slot, .. } = &mut kind {
                *slot = status;
            }
            self.store.splice(i, 1, &[SidebarItem::new(kind)]);
            break;
        }
    }
}

#[derive(Clone)]
struct RowWidgets {
    root: gtk::Box,
    indent: gtk::Box,
    expander: gtk::Image,
    swatch: gtk::Box,
    name: gtk::Label,
    detail: gtk::Label,
    status: gtk::Label,
}

fn build_row_widget() -> RowWidgets {
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    root.set_margin_start(4);
    root.set_margin_end(8);
    root.set_margin_top(4);
    root.set_margin_bottom(4);
    root.add_css_class("connection-row");

    let indent = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    indent.set_hexpand(false);

    let expander = gtk::Image::from_icon_name("pan-end-symbolic");
    expander.set_pixel_size(12);
    expander.set_visible(false);

    let swatch = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    swatch.add_css_class("connection-color-dot");
    swatch.set_valign(gtk::Align::Center);
    swatch.set_halign(gtk::Align::Center);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_hexpand(true);
    let name = gtk::Label::builder()
        .xalign(0.0)
        .ellipsize(pango::EllipsizeMode::End)
        .css_classes(["connection-name"])
        .build();
    let detail = gtk::Label::builder()
        .xalign(0.0)
        .ellipsize(pango::EllipsizeMode::End)
        .css_classes(["dimmed", "caption", "connection-detail"])
        .build();
    text.append(&name);
    text.append(&detail);

    let status = gtk::Label::new(None);
    status.set_width_chars(1);
    status.add_css_class("connection-status");
    status.set_valign(gtk::Align::Center);

    root.append(&indent);
    root.append(&expander);
    root.append(&swatch);
    root.append(&text);
    root.append(&status);

    RowWidgets {
        root,
        indent,
        expander,
        swatch,
        name,
        detail,
        status,
    }
}

fn clear_color_classes(swatch: &gtk::Box) {
    for class in [
        "connection-color-red",
        "connection-color-orange",
        "connection-color-yellow",
        "connection-color-green",
        "connection-color-teal",
        "connection-color-blue",
        "connection-color-violet",
        "connection-color-pink",
        "connection-color-slate",
        "connection-color-white",
    ] {
        swatch.remove_css_class(class);
    }
}

fn bind_row(row: &RowWidgets, item: &SidebarItem) {
    clear_color_classes(&row.swatch);
    for class in [
        "status-connected",
        "status-connecting",
        "status-error",
        "status-disconnected",
    ] {
        row.status.remove_css_class(class);
    }

    match item.kind() {
        SidebarKind::Group {
            name,
            color,
            collapsed,
            depth,
            ..
        } => {
            row.indent.set_margin_start((depth * 12) as i32);
            row.expander.set_visible(true);
            row.expander.set_icon_name(Some(if collapsed {
                "pan-end-symbolic"
            } else {
                "pan-down-symbolic"
            }));
            row.swatch
                .add_css_class(group_color_css_class(color.as_deref()));
            row.name.set_text(&name);
            row.name.add_css_class("heading");
            row.detail.set_visible(false);
            row.status.set_text("");
            row.status.set_tooltip_text(None);
            row.root.add_css_class("connection-group-row");
            row.root.remove_css_class("connection-conn-row");
        }
        SidebarKind::Connection {
            name,
            color_id,
            db_type,
            host,
            depth,
            status,
            ..
        } => {
            row.indent.set_margin_start((depth * 12) as i32);
            row.expander.set_visible(false);
            row.swatch.add_css_class(color_css_class(&color_id));
            row.name.set_text(&name);
            row.name.remove_css_class("heading");
            row.detail.set_visible(true);
            let host_label = if host.is_empty() {
                "—"
            } else {
                host.as_str()
            };
            row.detail.set_text(&format!("{db_type} · {host_label}"));
            match status {
                ConnectionStatus::Connected => {
                    row.status.set_text("●");
                    row.status.add_css_class("status-connected");
                    row.status.set_tooltip_text(Some("Connected"));
                }
                ConnectionStatus::Connecting => {
                    row.status.set_text("◌");
                    row.status.add_css_class("status-connecting");
                    row.status.set_tooltip_text(Some("Connecting…"));
                }
                ConnectionStatus::Error => {
                    row.status.set_text("!");
                    row.status.add_css_class("status-error");
                    row.status.set_tooltip_text(Some("Connection error"));
                }
                ConnectionStatus::Disconnected => {
                    row.status.set_text("");
                    row.status.add_css_class("status-disconnected");
                    row.status.set_tooltip_text(None);
                }
            }
            row.root.add_css_class("connection-conn-row");
            row.root.remove_css_class("connection-group-row");
        }
    }
}

fn install_context_menu(window: &SqlatorWindow, root: &gtk::Box) {
    let gesture = gtk::GestureClick::new();
    gesture.set_button(gdk::BUTTON_SECONDARY);
    gesture.connect_released(glib::clone!(
        #[weak]
        window,
        #[weak]
        root,
        move |gesture, _, x, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            let Some(item_ptr) = (unsafe { root.data::<SidebarItem>(ITEM_DATA_KEY) }) else {
                return;
            };
            let item = unsafe { item_ptr.as_ref() }.clone();
            let popover = build_context_popover(&window, &item);
            popover.set_parent(&root);
            popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover.connect_closed(|p| {
                p.unparent();
            });
            popover.popup();
        }
    ));
    root.add_controller(gesture);
}

fn build_context_popover(window: &SqlatorWindow, item: &SidebarItem) -> gtk::Popover {
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 0);
    box_.add_css_class("connection-menu");

    match item.kind() {
        SidebarKind::Connection { id, status, .. } => {
            if status != ConnectionStatus::Connected {
                let connect_btn = menu_button("Connect");
                connect_btn.connect_clicked(glib::clone!(
                    #[weak]
                    window,
                    #[strong]
                    id,
                    move |btn| {
                        close_popover(btn);
                        window.set_selected_connection_id(Some(id.clone()));
                        window.set_schema_connection_id(Some(id.clone()));
                        window.open_connection_workspace(&id);
                        window.connect_sidebar_connection(&id);
                    }
                ));
                box_.append(&connect_btn);
            }
            if matches!(
                status,
                ConnectionStatus::Connected | ConnectionStatus::Connecting
            ) {
                let disconnect_btn = menu_button("Disconnect");
                disconnect_btn.connect_clicked(glib::clone!(
                    #[weak]
                    window,
                    #[strong]
                    id,
                    move |btn| {
                        close_popover(btn);
                        window.disconnect_sidebar_connection(&id);
                    }
                ));
                box_.append(&disconnect_btn);
            }

            let clone_btn = menu_button("Clone");
            clone_btn.connect_clicked(glib::clone!(
                #[weak]
                window,
                #[strong]
                id,
                move |btn| {
                    close_popover(btn);
                    window.clone_sidebar_connection(&id);
                }
            ));
            box_.append(&clone_btn);

            let delete_btn = menu_button("Delete");
            delete_btn.add_css_class("destructive-action");
            delete_btn.connect_clicked(glib::clone!(
                #[weak]
                window,
                #[strong]
                id,
                move |btn| {
                    close_popover(btn);
                    window.delete_sidebar_connection(&id);
                }
            ));
            box_.append(&delete_btn);
        }
        SidebarKind::Group {
            id,
            collapsed,
            name,
            ..
        } => {
            let toggle_label = if collapsed { "Expand" } else { "Collapse" };
            let toggle_btn = menu_button(toggle_label);
            toggle_btn.connect_clicked(glib::clone!(
                #[weak]
                window,
                #[strong]
                id,
                move |btn| {
                    close_popover(btn);
                    window.toggle_group_collapsed(&id, !collapsed);
                }
            ));
            box_.append(&toggle_btn);

            let delete_btn = menu_button(&format!("Delete “{name}”"));
            delete_btn.add_css_class("destructive-action");
            delete_btn.connect_clicked(glib::clone!(
                #[weak]
                window,
                #[strong]
                id,
                move |btn| {
                    close_popover(btn);
                    window.delete_sidebar_group(&id);
                }
            ));
            box_.append(&delete_btn);
        }
    }

    let popover = gtk::Popover::new();
    popover.set_child(Some(&box_));
    popover.set_has_arrow(true);
    popover
}

fn menu_button(label: &str) -> gtk::Button {
    let btn = gtk::Button::with_label(label);
    btn.set_halign(gtk::Align::Fill);
    btn.add_css_class("flat");
    btn.add_css_class("connection-menu-item");
    btn
}

fn close_popover(from: &impl IsA<gtk::Widget>) {
    let mut widget = from.clone().upcast::<gtk::Widget>();
    while let Some(parent) = widget.parent() {
        if let Ok(popover) = parent.clone().downcast::<gtk::Popover>() {
            popover.popdown();
            return;
        }
        widget = parent;
    }
}

fn flatten_sidebar(
    groups: Vec<ConnectionGroup>,
    connections: Vec<ConnectionInfo>,
    connected: &HashMap<String, ConnectionStatus>,
) -> Vec<SidebarKind> {
    let mut by_parent: HashMap<Option<String>, Vec<ConnectionGroup>> = HashMap::new();
    for g in groups {
        by_parent
            .entry(g.parent_group_id.clone())
            .or_default()
            .push(g);
    }
    for list in by_parent.values_mut() {
        list.sort_by(|a, b| a.order.cmp(&b.order).then(a.name.cmp(&b.name)));
    }

    let mut by_group: HashMap<Option<String>, Vec<ConnectionInfo>> = HashMap::new();
    for c in connections {
        by_group.entry(c.group_id.clone()).or_default().push(c);
    }
    for list in by_group.values_mut() {
        list.sort_by(|a, b| a.name.cmp(&b.name));
    }

    let mut out = Vec::new();
    append_level(&mut out, None, 0, &by_parent, &by_group, connected);
    out
}

fn append_level(
    out: &mut Vec<SidebarKind>,
    parent: Option<&str>,
    depth: u32,
    by_parent: &HashMap<Option<String>, Vec<ConnectionGroup>>,
    by_group: &HashMap<Option<String>, Vec<ConnectionInfo>>,
    connected: &HashMap<String, ConnectionStatus>,
) {
    let key = parent.map(str::to_string);
    if let Some(groups) = by_parent.get(&key) {
        for g in groups {
            let collapsed = g.collapsed;
            out.push(SidebarKind::Group {
                id: g.id.clone(),
                name: g.name.clone(),
                color: g.color.clone(),
                collapsed,
                depth,
                order: g.order,
                parent_group_id: g.parent_group_id.clone(),
            });
            if !collapsed {
                append_level(out, Some(&g.id), depth + 1, by_parent, by_group, connected);
            }
        }
    }

    if let Some(conns) = by_group.get(&key) {
        for c in conns {
            let status = connected
                .get(&c.id)
                .copied()
                .unwrap_or(ConnectionStatus::Disconnected);
            out.push(SidebarKind::Connection {
                id: c.id.clone(),
                name: c.name.clone(),
                color_id: c.color_id.clone(),
                db_type: c.db_type.clone(),
                host: c.host.clone(),
                group_id: c.group_id.clone(),
                depth,
                status,
            });
        }
    }
}

/// Snapshot connected flags from the live pool map.
pub fn status_map_from_service(
    service: &Arc<sqlator_service::AppService>,
    ids: &[String],
) -> HashMap<String, ConnectionStatus> {
    let db = service.db_handle();
    let mut map = HashMap::new();
    for id in ids {
        if db.is_connected(id) {
            map.insert(id.clone(), ConnectionStatus::Connected);
        }
    }
    map
}
