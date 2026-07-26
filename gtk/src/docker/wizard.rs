//! Docker connection wizard: SSH → container → credentials → test.

use crate::application::SqlatorApplication;
use crate::connection::{color_css_class, present_connection_form};
use crate::window::SqlatorWindow;
use adw::prelude::*;
use gtk::glib;
use sqlator_core::models::{ConnectionConfig, ConnectionType};
use sqlator_service::{
    classify_docker_service_error, classify_test_service_error, default_port_for_db_type,
    AppService, ClassifiedDockerError, ContainerSummaryInfo, DockerContainerInfo,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

const COLOR_IDS: &[&str] = &[
    "red", "orange", "yellow", "green", "teal", "blue", "violet", "pink", "slate", "white",
];

const DB_TYPES: &[(&str, &str)] = &[
    ("postgres", "PostgreSQL"),
    ("mysql", "MySQL"),
    ("mariadb", "MariaDB"),
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
enum Step {
    Ssh,
    Container,
    Credentials,
    Test,
}

impl Step {
    fn name(self) -> &'static str {
        match self {
            Step::Ssh => "ssh",
            Step::Container => "container",
            Step::Credentials => "credentials",
            Step::Test => "test",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Step::Ssh => "1. SSH",
            Step::Container => "2. Container",
            Step::Credentials => "3. Credentials",
            Step::Test => "4. Test",
        }
    }

    fn next(self) -> Option<Self> {
        match self {
            Step::Ssh => Some(Step::Container),
            Step::Container => Some(Step::Credentials),
            Step::Credentials => Some(Step::Test),
            Step::Test => None,
        }
    }

    fn prev(self) -> Option<Self> {
        match self {
            Step::Ssh => None,
            Step::Container => Some(Step::Ssh),
            Step::Credentials => Some(Step::Container),
            Step::Test => Some(Step::Credentials),
        }
    }
}

struct WizardState {
    step: Cell<Step>,
    /// Index 0 = local Docker (`None`). Rest are remote SSH profile ids.
    ssh_profiles: RefCell<Vec<(Option<String>, String)>>,
    container_info: RefCell<Option<DockerContainerInfo>>,
    color_id: RefCell<String>,
    groups: RefCell<Vec<(Option<String>, String)>>,
}

/// Widgets shared by discover / test / save / footer updates.
struct WizardFields {
    ssh_row: adw::ComboRow,
    container_name_row: adw::EntryRow,
    name_row: adw::EntryRow,
    group_row: adw::ComboRow,
    db_type_row: adw::ComboRow,
    port_row: adw::SpinRow,
    ssl_row: adw::ComboRow,
    database_row: adw::EntryRow,
    username_row: adw::EntryRow,
    password_row: adw::PasswordEntryRow,
    containers_list: gtk::ListBox,
    containers_status: gtk::Label,
    discover_status: gtk::Label,
    discover_hint: gtk::Label,
    test_status: gtk::Label,
    test_hint: gtk::Label,
    status_label: gtk::Label,
    back_btn: gtk::Button,
    next_btn: gtk::Button,
    test_btn: gtk::Button,
    save_btn: gtk::Button,
    step_label: gtk::Label,
    stack: gtk::Stack,
}

/// Present a type picker, then either the direct connection form or this wizard.
pub fn present_new_connection_chooser(window: &SqlatorWindow, app: &SqlatorApplication) {
    let dialog = adw::AlertDialog::new(
        Some("New connection"),
        Some("Choose how you want to connect to the database."),
    );
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("direct", "Direct / SSH");
    dialog.add_response("docker", "Docker container");
    dialog.set_response_appearance("docker", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("direct"));
    dialog.set_close_response("cancel");

    dialog.connect_response(
        None,
        glib::clone!(
            #[strong]
            window,
            #[strong]
            app,
            move |_, response| match response {
                "direct" => present_connection_form(&window, &app, None),
                "docker" => present(&window, &app),
                _ => {}
            }
        ),
    );
    dialog.present(Some(window));
}

/// Present the 4-step Docker wizard.
pub fn present(window: &SqlatorWindow, app: &SqlatorApplication) {
    let service = app.service();
    let dialog = adw::Dialog::new();
    // Track natural height so step changes (esp. container list) resize the dialog.
    // NOTE: follows-content-size ignores content-width/height; width must come from
    // the child's size request (same ~520px content column as the direct form).
    dialog.set_follows_content_size(true);
    dialog.set_title("New Docker Container Connection");

    let toast_overlay = adw::ToastOverlay::new();
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();

    let cancel_btn = gtk::Button::with_label("Cancel");
    cancel_btn.add_css_class("flat");
    header.pack_start(&cancel_btn);

    let step_label = gtk::Label::builder()
        .css_classes(["heading"])
        .label(Step::Ssh.title())
        .build();
    header.set_title_widget(Some(&step_label));
    toolbar.add_top_bar(&header);

    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(520);
    clamp.set_tightening_threshold(400);
    // Floor the natural width so ActionRow titles (e.g. Color) don't wrap to a
    // single-character column when follows-content-size is enabled.
    clamp.set_size_request(520, -1);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(18);
    content.set_margin_start(18);
    content.set_margin_end(18);

    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
    stack.set_vhomogeneous(false);

    // ── Step 1: SSH ───────────────────────────────────────────────────────
    let ssh_page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let ssh_desc = gtk::Label::builder()
        .label(
            "Select an SSH profile for a remote Docker host, or keep “None” to use the local Docker socket.",
        )
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dimmed"])
        .build();
    ssh_page.append(&ssh_desc);
    let ssh_group = adw::PreferencesGroup::new();
    let ssh_row = adw::ComboRow::builder()
        .title("SSH profile")
        .subtitle("None = local Docker")
        .build();
    ssh_group.add(&ssh_row);
    ssh_page.append(&ssh_group);
    stack.add_named(&ssh_page, Some("ssh"));

    // ── Step 2: Container ─────────────────────────────────────────────────
    let container_page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let container_desc = gtk::Label::builder()
        .label("Pick a running database container or type a name and Discover.")
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dimmed"])
        .build();
    container_page.append(&container_desc);

    let containers_frame = gtk::Frame::new(Some("Running database containers"));
    let containers_list = gtk::ListBox::new();
    containers_list.add_css_class("boxed-list");
    containers_list.set_selection_mode(gtk::SelectionMode::None);
    let containers_scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        // Grow with the list; only scroll once the dialog would get too tall.
        .max_content_height(360)
        .propagate_natural_height(true)
        .child(&containers_list)
        .build();
    containers_frame.set_child(Some(&containers_scrolled));
    container_page.append(&containers_frame);

    let containers_status = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .css_classes(["caption", "dimmed"])
        .label("Loading containers…")
        .build();
    container_page.append(&containers_status);

    let container_group = adw::PreferencesGroup::new();
    let container_name_row = adw::EntryRow::builder()
        .title("Container name")
        .show_apply_button(true)
        .build();
    container_group.add(&container_name_row);
    container_page.append(&container_group);

    let discover_btn = gtk::Button::with_label("Discover");
    discover_btn.add_css_class("pill");
    discover_btn.set_halign(gtk::Align::Start);
    container_page.append(&discover_btn);

    let discover_status = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .css_classes(["caption"])
        .build();
    container_page.append(&discover_status);
    let discover_hint = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .selectable(true)
        .css_classes(["caption", "dimmed"])
        .build();
    container_page.append(&discover_hint);
    stack.add_named(&container_page, Some("container"));

    // ── Step 3: Credentials ───────────────────────────────────────────────
    let creds_page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let general = adw::PreferencesGroup::new();
    general.set_title("Connection");
    let name_row = adw::EntryRow::builder().title("Name").build();
    general.add(&name_row);

    let color_row = adw::ActionRow::builder().title("Color").build();
    let color_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    color_box.set_valign(gtk::Align::Center);
    color_row.add_suffix(&color_box);
    general.add(&color_row);

    let group_row = adw::ComboRow::builder().title("Group").build();
    general.add(&group_row);
    creds_page.append(&general);

    let db_group = adw::PreferencesGroup::new();
    db_group.set_title("Database");
    let db_type_row = adw::ComboRow::builder().title("Type").build();
    let db_type_model = gtk::StringList::new(&DB_TYPES.iter().map(|(_, l)| *l).collect::<Vec<_>>());
    db_type_row.set_model(Some(&db_type_model));
    db_group.add(&db_type_row);

    let port_row = adw::SpinRow::builder()
        .title("Container port")
        .adjustment(
            &gtk::Adjustment::builder()
                .lower(1.0)
                .upper(65535.0)
                .step_increment(1.0)
                .page_increment(10.0)
                .value(5432.0)
                .build(),
        )
        .digits(0)
        .build();
    db_group.add(&port_row);

    let ssl_row = adw::ComboRow::builder().title("SSL mode").build();
    let ssl_model = gtk::StringList::new(&SSL_MODES.iter().map(|(_, l)| *l).collect::<Vec<_>>());
    ssl_row.set_model(Some(&ssl_model));
    db_group.add(&ssl_row);

    let database_row = adw::EntryRow::builder().title("Database").build();
    db_group.add(&database_row);
    let username_row = adw::EntryRow::builder().title("Username").build();
    db_group.add(&username_row);
    let password_row = adw::PasswordEntryRow::builder().title("Password").build();
    db_group.add(&password_row);
    creds_page.append(&db_group);
    stack.add_named(&creds_page, Some("credentials"));

    // ── Step 4: Test ──────────────────────────────────────────────────────
    let test_page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let test_desc = gtk::Label::builder()
        .label("Verify connectivity before saving. You can also save without a successful test.")
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dimmed"])
        .build();
    test_page.append(&test_desc);
    let test_status = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .css_classes(["caption"])
        .build();
    test_page.append(&test_status);
    let test_hint = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .selectable(true)
        .css_classes(["caption", "dimmed"])
        .build();
    test_page.append(&test_hint);
    stack.add_named(&test_page, Some("test"));

    content.append(&stack);

    // ── Footer actions ────────────────────────────────────────────────────
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);
    let back_btn = gtk::Button::with_label("Back");
    back_btn.add_css_class("flat");
    let next_btn = gtk::Button::with_label("Next");
    next_btn.add_css_class("suggested-action");
    let test_btn = gtk::Button::with_label("Test connection");
    let save_btn = gtk::Button::with_label("Save");
    save_btn.add_css_class("suggested-action");
    actions.append(&back_btn);
    actions.append(&test_btn);
    actions.append(&next_btn);
    actions.append(&save_btn);
    content.append(&actions);

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
        .propagate_natural_width(true)
        .propagate_natural_height(true)
        .child(&clamp)
        .build();
    toast_overlay.set_child(Some(&scrolled));
    toolbar.set_content(Some(&toast_overlay));
    dialog.set_child(Some(&toolbar));

    let state = Rc::new(WizardState {
        step: Cell::new(Step::Ssh),
        ssh_profiles: RefCell::new(vec![(None, "None (local Docker)".into())]),
        container_info: RefCell::new(None),
        color_id: RefCell::new("blue".into()),
        groups: RefCell::new(vec![(None, "None".into())]),
    });

    let fields = Rc::new(WizardFields {
        ssh_row,
        container_name_row,
        name_row,
        group_row,
        db_type_row,
        port_row,
        ssl_row,
        database_row,
        username_row,
        password_row,
        containers_list,
        containers_status,
        discover_status,
        discover_hint,
        test_status,
        test_hint,
        status_label,
        back_btn,
        next_btn,
        test_btn,
        save_btn,
        step_label,
        stack,
    });

    // Color swatches
    let mut first_color_btn: Option<gtk::ToggleButton> = None;
    for &id in COLOR_IDS {
        let btn = gtk::ToggleButton::new();
        btn.set_tooltip_text(Some(id));
        btn.add_css_class("connection-color-dot");
        btn.add_css_class(color_css_class(id));
        btn.set_size_request(22, 22);
        if id == state.color_id.borrow().as_str() {
            btn.set_active(true);
        }
        if let Some(ref first) = first_color_btn {
            btn.set_group(Some(first));
        } else {
            first_color_btn = Some(btn.clone());
        }
        btn.connect_toggled(glib::clone!(
            #[strong]
            state,
            move |b| {
                if b.is_active() {
                    *state.color_id.borrow_mut() = id.to_string();
                }
            }
        ));
        color_box.append(&btn);
    }

    let update_footer = {
        let state = state.clone();
        let fields = fields.clone();
        Rc::new(move || {
            let step = state.step.get();
            fields.step_label.set_text(step.title());
            fields.stack.set_visible_child_name(step.name());
            fields.back_btn.set_visible(step.prev().is_some());
            let creds_ok = !fields.name_row.text().trim().is_empty()
                && !fields.username_row.text().trim().is_empty()
                && state.container_info.borrow().is_some();
            match step {
                Step::Ssh => {
                    fields.next_btn.set_visible(true);
                    fields.next_btn.set_sensitive(true);
                    fields.test_btn.set_visible(false);
                    fields.save_btn.set_visible(false);
                }
                Step::Container => {
                    fields.next_btn.set_visible(true);
                    fields
                        .next_btn
                        .set_sensitive(state.container_info.borrow().is_some());
                    fields.test_btn.set_visible(false);
                    fields.save_btn.set_visible(false);
                }
                Step::Credentials => {
                    fields.next_btn.set_visible(false);
                    fields.test_btn.set_visible(true);
                    fields.test_btn.set_sensitive(creds_ok);
                    fields.test_btn.set_label("Test connection");
                    fields.save_btn.set_visible(true);
                    fields.save_btn.set_sensitive(creds_ok);
                }
                Step::Test => {
                    fields.next_btn.set_visible(false);
                    fields.test_btn.set_visible(true);
                    fields.test_btn.set_sensitive(creds_ok);
                    fields.test_btn.set_label("Retry test");
                    fields.save_btn.set_visible(true);
                    fields.save_btn.set_sensitive(creds_ok);
                }
            }
        })
    };
    update_footer();

    for row in [&fields.name_row, &fields.username_row] {
        row.connect_changed(glib::clone!(
            #[strong]
            update_footer,
            move |_| update_footer()
        ));
    }

    fields.db_type_row.connect_selected_notify(glib::clone!(
        #[strong]
        fields,
        move |_| {
            let dt = selected_db_type(&fields.db_type_row);
            let port = default_port_for_db_type(&dt);
            if port > 0 {
                fields.port_row.set_value(f64::from(port));
            }
            fields.ssl_row.set_selected(default_ssl_index_for(&dt));
        }
    ));

    // Load SSH profiles + groups.
    {
        let svc_p = service.clone();
        let svc_g = service.clone();
        glib::spawn_future_local(glib::clone!(
            #[strong]
            state,
            #[strong]
            fields,
            async move {
                let profiles = crate::spawn_tokio!(async move { svc_p.get_ssh_profiles() })
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .unwrap_or_default();
                let groups_res = crate::spawn_tokio!(async move { svc_g.get_groups().await })
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .unwrap_or_default();

                let mut profiles_v = vec![(None, "None (local Docker)".into())];
                let mut plist = profiles;
                plist.sort_by(|a, b| a.name.cmp(&b.name));
                for p in plist {
                    profiles_v.push((Some(p.id), p.name));
                }
                let labels: Vec<&str> = profiles_v.iter().map(|(_, n)| n.as_str()).collect();
                fields
                    .ssh_row
                    .set_model(Some(&gtk::StringList::new(&labels)));
                fields.ssh_row.set_selected(0);
                *state.ssh_profiles.borrow_mut() = profiles_v;

                let mut groups = vec![(None, "None".into())];
                let mut glist = groups_res;
                glist.sort_by(|a, b| a.name.cmp(&b.name));
                for g in glist {
                    groups.push((Some(g.id), g.name));
                }
                let labels: Vec<&str> = groups.iter().map(|(_, n)| n.as_str()).collect();
                fields
                    .group_row
                    .set_model(Some(&gtk::StringList::new(&labels)));
                fields.group_row.set_selected(0);
                *state.groups.borrow_mut() = groups;
            }
        ));
    }

    let load_containers = {
        let state = state.clone();
        let service = service.clone();
        let fields = fields.clone();
        let update_footer = update_footer.clone();
        Rc::new(move || {
            while let Some(child) = fields.containers_list.first_child() {
                fields.containers_list.remove(&child);
            }
            fields.containers_status.set_text("Loading containers…");
            *state.container_info.borrow_mut() = None;
            fields.discover_status.set_text("");
            fields.discover_hint.set_text("");
            update_footer();

            let ssh_id = selected_optional_id(&fields.ssh_row, &state.ssh_profiles.borrow());
            let svc = service.clone();
            glib::spawn_future_local(glib::clone!(
                #[strong]
                state,
                #[strong]
                fields,
                #[strong]
                update_footer,
                #[strong]
                service,
                async move {
                    let result = crate::spawn_tokio!(async move {
                        match ssh_id.as_deref() {
                            Some(id) => svc.list_running_containers(id).await,
                            None => svc.list_local_containers().await,
                        }
                    })
                    .await
                    .expect("join list containers");

                    match result {
                        Ok(list) => {
                            let db_only: Vec<ContainerSummaryInfo> = list
                                .into_iter()
                                .filter(|c| c.database_type_hint.is_some())
                                .collect();
                            if db_only.is_empty() {
                                fields.containers_status.set_text(
                                    "No database containers found. Enter a name manually.",
                                );
                            } else {
                                fields
                                    .containers_status
                                    .set_text(&format!("{} database container(s)", db_only.len()));
                            }
                            for c in db_only {
                                let row = adw::ActionRow::builder()
                                    .title(&c.name)
                                    .subtitle(format!(
                                        "{} · {}",
                                        c.image,
                                        c.database_type_hint.as_deref().unwrap_or("?")
                                    ))
                                    .activatable(true)
                                    .build();
                                let name = c.name.clone();
                                row.connect_activated(glib::clone!(
                                    #[strong]
                                    state,
                                    #[strong]
                                    fields,
                                    #[strong]
                                    update_footer,
                                    #[strong]
                                    service,
                                    move |_| {
                                        fields.container_name_row.set_text(&name);
                                        run_discover(
                                            service.clone(),
                                            state.clone(),
                                            fields.clone(),
                                            update_footer.clone(),
                                        );
                                    }
                                ));
                                fields.containers_list.append(&row);
                            }
                        }
                        Err(e) => {
                            let classified = classify_docker_service_error(&e);
                            fields
                                .containers_status
                                .set_text(&format!("Could not list containers: {}", e.message()));
                            apply_classified_hint(&fields.discover_hint, &classified);
                        }
                    }
                }
            ));
        })
    };

    cancel_btn.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        move |_| {
            dialog.close();
        }
    ));

    fields.back_btn.connect_clicked(glib::clone!(
        #[strong]
        state,
        #[strong]
        update_footer,
        move |_| {
            if let Some(prev) = state.step.get().prev() {
                state.step.set(prev);
                update_footer();
            }
        }
    ));

    fields.next_btn.connect_clicked(glib::clone!(
        #[strong]
        state,
        #[strong]
        update_footer,
        #[strong]
        load_containers,
        move |_| {
            let step = state.step.get();
            if step == Step::Ssh {
                state.step.set(Step::Container);
                update_footer();
                load_containers();
                return;
            }
            if step == Step::Container && state.container_info.borrow().is_none() {
                return;
            }
            if let Some(next) = step.next() {
                state.step.set(next);
                update_footer();
            }
        }
    ));

    discover_btn.connect_clicked(glib::clone!(
        #[strong]
        state,
        #[strong]
        service,
        #[strong]
        fields,
        #[strong]
        update_footer,
        move |_| {
            run_discover(
                service.clone(),
                state.clone(),
                fields.clone(),
                update_footer.clone(),
            );
        }
    ));

    // Enter in container name runs Discover.
    fields.container_name_row.connect_apply(glib::clone!(
        #[strong]
        state,
        #[strong]
        service,
        #[strong]
        fields,
        #[strong]
        update_footer,
        move |_| {
            run_discover(
                service.clone(),
                state.clone(),
                fields.clone(),
                update_footer.clone(),
            );
        }
    ));

    let run_test = {
        let state = state.clone();
        let service = service.clone();
        let fields = fields.clone();
        let update_footer = update_footer.clone();
        Rc::new(move || {
            if state.container_info.borrow().is_none() {
                return;
            }
            let url = build_wizard_url(&fields);
            let container_name = fields.container_name_row.text().trim().to_string();
            let container_port = fields.port_row.value() as u16;
            let db_type = selected_db_type(&fields.db_type_row);
            let ssh_id = selected_optional_id(&fields.ssh_row, &state.ssh_profiles.borrow());
            let svc = service.clone();

            // From credentials, advance into the test step so the user sees the result.
            if state.step.get() == Step::Credentials {
                state.step.set(Step::Test);
                update_footer();
            }

            fields.test_btn.set_sensitive(false);
            fields.test_status.set_text("Testing…");
            fields.test_status.remove_css_class("error");
            fields.test_status.remove_css_class("success");
            fields.test_hint.set_text("");

            glib::spawn_future_local(glib::clone!(
                #[strong]
                fields,
                #[strong]
                update_footer,
                async move {
                    let result = crate::spawn_tokio!(async move {
                        match ssh_id.as_deref() {
                            Some(id) => {
                                svc.test_docker_connection(
                                    id,
                                    &container_name,
                                    Some(container_port),
                                    &url,
                                    &db_type,
                                )
                                .await
                            }
                            None => {
                                svc.test_local_docker_connection(
                                    &container_name,
                                    Some(container_port),
                                    &url,
                                    &db_type,
                                )
                                .await
                            }
                        }
                    })
                    .await
                    .expect("join test docker");

                    fields.test_btn.set_sensitive(true);
                    match result {
                        Ok(msg) => {
                            fields.test_status.set_text(&msg);
                            fields.test_status.add_css_class("success");
                            fields.test_hint.set_text("");
                        }
                        Err(e) => {
                            let classified = classify_test_service_error(&e);
                            fields.test_status.set_text(&e.message());
                            fields.test_status.add_css_class("error");
                            apply_classified_hint(&fields.test_hint, &classified);
                        }
                    }
                    update_footer();
                }
            ));
        })
    };

    fields.test_btn.connect_clicked(glib::clone!(
        #[strong]
        run_test,
        move |_| run_test()
    ));

    fields.save_btn.connect_clicked(glib::clone!(
        #[strong]
        state,
        #[strong]
        service,
        #[strong]
        window,
        #[strong]
        fields,
        #[weak]
        dialog,
        #[weak]
        toast_overlay,
        move |_| {
            if state.container_info.borrow().is_none() {
                fields
                    .status_label
                    .set_text("Discover a running container before saving.");
                fields.status_label.add_css_class("error");
                return;
            }
            let name = fields.name_row.text().trim().to_string();
            let username = fields.username_row.text().trim().to_string();
            if name.is_empty() || username.is_empty() {
                fields
                    .status_label
                    .set_text("Name and username are required.");
                fields.status_label.add_css_class("error");
                return;
            }
            let url = build_wizard_url(&fields);
            let ssh_id = selected_optional_id(&fields.ssh_row, &state.ssh_profiles.borrow());
            let is_local = ssh_id.is_none();
            let config = ConnectionConfig {
                name,
                color_id: state.color_id.borrow().clone(),
                url,
                ssh_profile_id: ssh_id,
                group_id: selected_optional_id(&fields.group_row, &state.groups.borrow()),
                connection_type: Some(if is_local {
                    ConnectionType::LocalDockerContainer
                } else {
                    ConnectionType::DockerContainer
                }),
                container_name: Some(fields.container_name_row.text().trim().to_string()),
                container_port: Some(fields.port_row.value() as u16),
            };

            fields.save_btn.set_sensitive(false);
            fields.status_label.set_text("Saving…");
            fields.status_label.remove_css_class("error");
            let svc = service.clone();
            glib::spawn_future_local(glib::clone!(
                #[strong]
                fields,
                #[strong]
                window,
                #[weak]
                dialog,
                #[weak]
                toast_overlay,
                async move {
                    let result =
                        crate::spawn_tokio!(async move { svc.save_connection(config).await })
                            .await
                            .expect("join save_connection");
                    fields.save_btn.set_sensitive(true);
                    match result {
                        Ok(info) => {
                            window.set_selected_connection_id(Some(info.id));
                            window.refresh_connections();
                            dialog.close();
                        }
                        Err(e) => {
                            fields.status_label.set_text(&e.message());
                            fields.status_label.add_css_class("error");
                            toast_overlay.add_toast(adw::Toast::new(&e.message()));
                        }
                    }
                }
            ));
        }
    ));

    dialog.present(Some(window));
}

fn run_discover(
    service: Arc<AppService>,
    state: Rc<WizardState>,
    fields: Rc<WizardFields>,
    update_footer: Rc<dyn Fn()>,
) {
    let name = fields.container_name_row.text().trim().to_string();
    if name.is_empty() {
        fields.discover_status.set_text("Enter a container name.");
        fields.discover_status.add_css_class("error");
        return;
    }
    let ssh_id = selected_optional_id(&fields.ssh_row, &state.ssh_profiles.borrow());
    fields.discover_status.set_text("Discovering…");
    fields.discover_status.remove_css_class("error");
    fields.discover_hint.set_text("");
    *state.container_info.borrow_mut() = None;
    update_footer();

    glib::spawn_future_local(async move {
        let name_task = name.clone();
        let result = crate::spawn_tokio!(async move {
            match ssh_id.as_deref() {
                Some(id) => service.discover_container(id, &name_task).await,
                None => service.discover_local_container(&name_task).await,
            }
        })
        .await
        .expect("join discover");

        match result {
            Ok(info) => {
                if info.status != "running" {
                    let (code, msg) = if info.status == "stopped" {
                        (
                            "DOCKER_CONTAINER_STOPPED",
                            format!("Container '{name}' is stopped. Start it and try again."),
                        )
                    } else {
                        (
                            "DOCKER_CONTAINER_NOT_FOUND",
                            format!("Container '{name}' not found. Check the name and try again."),
                        )
                    };
                    let classified = classify_docker_service_error(
                        &sqlator_service::ServiceError::app(code, msg.clone()),
                    );
                    fields.discover_status.set_text(&msg);
                    fields.discover_status.add_css_class("error");
                    apply_classified_hint(&fields.discover_hint, &classified);
                    *state.container_info.borrow_mut() = None;
                    update_footer();
                    return;
                }

                if let Some(hint) = &info.database_type_hint {
                    if let Some(idx) = DB_TYPES.iter().position(|(id, _)| *id == hint.as_str()) {
                        fields.db_type_row.set_selected(idx as u32);
                        let port = default_port_for_db_type(hint);
                        if port > 0 {
                            fields.port_row.set_value(f64::from(port));
                        }
                        fields.ssl_row.set_selected(default_ssl_index_for(hint));
                    }
                }
                if fields.name_row.text().trim().is_empty() {
                    fields
                        .name_row
                        .set_text(fields.container_name_row.text().as_str());
                }
                fields.discover_status.set_text(&format!(
                    "Found {} @ {} ({})",
                    name, info.ip_address, info.status
                ));
                fields.discover_status.remove_css_class("error");
                fields.discover_hint.set_text("");
                *state.container_info.borrow_mut() = Some(info);
                update_footer();
            }
            Err(e) => {
                let classified = classify_docker_service_error(&e);
                fields.discover_status.set_text(&e.message());
                fields.discover_status.add_css_class("error");
                apply_classified_hint(&fields.discover_hint, &classified);
                *state.container_info.borrow_mut() = None;
                update_footer();
            }
        }
    });
}

fn apply_classified_hint(hint: &gtk::Label, info: &ClassifiedDockerError) {
    if info.hint.is_empty() {
        hint.set_text("");
        return;
    }
    let mut text = info.hint.clone();
    if info.transient {
        text.push_str("\n(This may be transient — retry may help.)");
    }
    hint.set_text(&text);
}

fn selected_db_type(row: &adw::ComboRow) -> String {
    let idx = row.selected() as usize;
    DB_TYPES
        .get(idx)
        .map(|(id, _)| (*id).to_string())
        .unwrap_or_else(|| "postgres".into())
}

fn default_ssl_index_for(db_type: &str) -> u32 {
    // MySQL/MariaDB: disable (rustls cipher issues; tunnel already secures remote).
    if db_type == "mysql" || db_type == "mariadb" {
        1 // disable
    } else {
        0 // prefer
    }
}

fn selected_ssl_mode(row: &adw::ComboRow) -> &'static str {
    let idx = row.selected() as usize;
    SSL_MODES.get(idx).map(|(id, _)| *id).unwrap_or("prefer")
}

fn selected_optional_id(row: &adw::ComboRow, items: &[(Option<String>, String)]) -> Option<String> {
    let idx = row.selected() as usize;
    items.get(idx).and_then(|(id, _)| id.clone())
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

fn encode_userinfo(s: &str) -> String {
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

fn build_wizard_url(fields: &WizardFields) -> String {
    let db_type = selected_db_type(&fields.db_type_row);
    let port = fields.port_row.value() as u16;
    let database = fields.database_row.text();
    let username = fields.username_row.text();
    let password = fields.password_row.text();
    let ssl = selected_ssl_mode(&fields.ssl_row);
    let user_part = if username.is_empty() {
        String::new()
    } else if password.is_empty() {
        format!("{}@", encode_userinfo(&username))
    } else {
        format!(
            "{}:{}@",
            encode_userinfo(&username),
            encode_userinfo(&password)
        )
    };
    let db_part = if database.is_empty() {
        String::new()
    } else {
        format!("/{database}")
    };
    let ssl_param = build_ssl_param(&db_type, ssl);
    // Placeholder host — backend rewrites to container IP on connect.
    format!("{db_type}://{user_part}localhost:{port}{db_part}{ssl_param}")
}
