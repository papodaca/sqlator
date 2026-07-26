use crate::application::SqlatorApplication;
use crate::query_tab::QueryTab;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib, CompositeTemplate};

mod imp {
    use super::*;
    use std::cell::RefCell;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/im/apodaca/SqlatorGtk/ui/window.ui")]
    pub struct SqlatorWindow {
        #[template_child]
        pub split_view: TemplateChild<adw::OverlaySplitView>,
        #[template_child]
        pub tab_view: TemplateChild<adw::TabView>,
        #[template_child]
        pub tab_bar: TemplateChild<adw::TabBar>,
        #[template_child]
        pub tab_overview: TemplateChild<adw::TabOverview>,
        #[template_child]
        pub overview_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub connection_list: TemplateChild<gtk::ListView>,

        pub settings: OnceCell<gio::Settings>,
        pub connection_store: RefCell<Option<gio::ListStore>>,
        pub selected_connection_id: RefCell<Option<String>>,
    }

    use std::cell::OnceCell;

    #[glib::object_subclass]
    impl ObjectSubclass for SqlatorWindow {
        const NAME: &'static str = "SqlatorWindow";
        type Type = super::SqlatorWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[gtk::template_callbacks]
    impl SqlatorWindow {
        #[template_callback]
        fn on_tab_overview_create_tab(&self, _overview: &adw::TabOverview) -> adw::TabPage {
            self.obj().add_query_tab(None)
        }
    }

    impl ObjectImpl for SqlatorWindow {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            let settings = gio::Settings::new("im.apodaca.SqlatorGtk");
            let width = settings.get::<i32>("window-width");
            let height = settings.get::<i32>("window-height");
            let maximized = settings.get::<bool>("window-maximized");
            obj.set_default_size(width, height);
            if maximized {
                obj.maximize();
            }

            let sidebar_visible = settings.get::<bool>("sidebar-visible");
            self.split_view.set_show_sidebar(sidebar_visible);
            settings
                .bind("sidebar-visible", &*self.split_view, "show-sidebar")
                .build();

            self.settings
                .set(settings)
                .expect("settings set once in constructed");

            obj.setup_actions();
            obj.setup_tab_view();
            // Connection list + initial tab need GtkWindow:application, which is
            // not readable until after ObjectBuilder::build returns — see ::new().

            // Persist geometry on close.
            obj.connect_close_request(glib::clone!(
                #[weak]
                obj,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_| {
                    if let Some(settings) = obj.imp().settings.get() {
                        let (width, height) = obj.default_size();
                        let _ = settings.set("window-width", width);
                        let _ = settings.set("window-height", height);
                        let _ = settings.set("window-maximized", obj.is_maximized());
                    }
                    glib::Propagation::Proceed
                }
            ));
        }
    }

    impl WidgetImpl for SqlatorWindow {}
    impl WindowImpl for SqlatorWindow {}
    impl ApplicationWindowImpl for SqlatorWindow {}
    impl AdwApplicationWindowImpl for SqlatorWindow {}
}

glib::wrapper! {
    pub struct SqlatorWindow(ObjectSubclass<imp::SqlatorWindow>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
                    gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl SqlatorWindow {
    pub fn new(app: &SqlatorApplication) -> Self {
        let window: Self = glib::Object::builder().property("application", app).build();

        // Load CSS once per display.
        let provider = gtk::CssProvider::new();
        provider.load_from_resource("/im/apodaca/SqlatorGtk/style.css");
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        // Must run after build(): application() is None during constructed().
        window.setup_connection_list();
        window.add_query_tab(None);

        window
    }

    fn settings(&self) -> &gio::Settings {
        self.imp().settings.get().expect("settings initialized")
    }

    fn setup_actions(&self) {
        let toggle_sidebar = gio::SimpleAction::new("toggle-sidebar", None);
        toggle_sidebar.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                let split = &window.imp().split_view;
                split.set_show_sidebar(!split.shows_sidebar());
            }
        ));
        self.add_action(&toggle_sidebar);

        let new_tab = gio::SimpleAction::new("new-tab", None);
        new_tab.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                window.add_query_tab(None);
            }
        ));
        self.add_action(&new_tab);

        let close_tab = gio::SimpleAction::new("close-tab", None);
        close_tab.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                let tab_view = &window.imp().tab_view;
                if let Some(page) = tab_view.selected_page() {
                    tab_view.close_page(&page);
                }
            }
        ));
        self.add_action(&close_tab);

        let tab_overview = gio::SimpleAction::new("tab-overview", None);
        tab_overview.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                window.imp().tab_overview.set_open(true);
            }
        ));
        self.add_action(&tab_overview);
    }

    fn setup_tab_view(&self) {
        let tab_view = self.imp().tab_view.clone();
        // create-tab on TabOverview is wired via #[template_callback].
        tab_view.connect_create_window(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            None,
            move |_| {
                let app = window
                    .application()
                    .and_downcast::<SqlatorApplication>()
                    .expect("SqlatorApplication");
                let new_window = SqlatorWindow::new(&app);
                new_window.present();
                Some(new_window.imp().tab_view.clone())
            }
        ));

        tab_view.connect_close_page(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |tab_view, page| {
                let Some(tab) = page.child().downcast::<QueryTab>().ok() else {
                    return glib::Propagation::Proceed;
                };
                if tab.is_busy() {
                    let dialog = adw::AlertDialog::new(
                        Some("Query running"),
                        Some("A query is still running in this tab. Cancel it and close?"),
                    );
                    dialog.add_response("keep", "Keep Open");
                    dialog.add_response("close", "Cancel & Close");
                    dialog.set_response_appearance("close", adw::ResponseAppearance::Destructive);
                    dialog.set_default_response(Some("keep"));
                    dialog.set_close_response("keep");

                    let page = page.clone();
                    let tab_view = tab_view.clone();
                    dialog.connect_response(
                        None,
                        glib::clone!(
                            #[weak]
                            tab,
                            move |_, response| {
                                if response == "close" {
                                    tab.cancel_query();
                                    tab_view.close_page_finish(&page, true);
                                } else {
                                    tab_view.close_page_finish(&page, false);
                                }
                            }
                        ),
                    );
                    dialog.present(Some(&window));
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
        ));
    }

    fn setup_connection_list(&self) {
        let store = gio::ListStore::new::<gtk::StringObject>();
        *self.imp().connection_store.borrow_mut() = Some(store.clone());

        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let label = gtk::Label::builder().xalign(0.0).build();
            item.set_child(Some(&label));
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let Some(obj) = item.item().and_downcast::<gtk::StringObject>() else {
                return;
            };
            if let Some(label) = item.child().and_downcast::<gtk::Label>() {
                label.set_text(&obj.string());
            }
        });

        let selection = gtk::SingleSelection::new(Some(store.clone()));
        selection.connect_selection_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |selection, _, _| {
                if let Some(obj) = selection
                    .selected_item()
                    .and_downcast::<gtk::StringObject>()
                {
                    // Store displays "name" but we keep id in a parallel refresh.
                    // For skeleton: treat string as "name [id]" encoded below.
                    let text = obj.string();
                    if let Some((name, id)) = text.rsplit_once(" · ") {
                        let _ = name;
                        *window.imp().selected_connection_id.borrow_mut() = Some(id.to_string());
                    }
                }
            }
        ));

        self.imp().connection_list.set_factory(Some(&factory));
        self.imp().connection_list.set_model(Some(&selection));

        self.refresh_connections();
    }

    pub fn refresh_connections(&self) {
        let app = self
            .application()
            .and_downcast::<SqlatorApplication>()
            .expect("SqlatorApplication");
        let service = app.service();
        let store = self
            .imp()
            .connection_store
            .borrow()
            .clone()
            .expect("connection store");

        glib::spawn_future_local(async move {
            let list = crate::spawn_tokio!(async move { service.list_connections().await })
                .await
                .expect("join list_connections");

            store.remove_all();
            match list {
                Ok(conns) => {
                    for c in conns {
                        let label = format!("{} · {}", c.name, c.id);
                        store.append(&gtk::StringObject::new(&label));
                    }
                }
                Err(e) => tracing::warn!("list_connections failed: {e}"),
            }
        });
    }

    pub fn selected_connection_id(&self) -> Option<String> {
        self.imp().selected_connection_id.borrow().clone()
    }

    pub fn add_query_tab(&self, title: Option<&str>) -> adw::TabPage {
        let app = self
            .application()
            .and_downcast::<SqlatorApplication>()
            .expect("SqlatorApplication");
        let tab = QueryTab::new(&app, self);
        let page = self.imp().tab_view.append(&tab);
        page.set_title(title.unwrap_or("Query"));
        page.set_live_thumbnail(true);
        self.imp().tab_view.set_selected_page(&page);
        page
    }

    pub fn editor_results_position(&self) -> i32 {
        self.settings().get::<i32>("editor-results-position")
    }

    pub fn set_editor_results_position(&self, pos: i32) {
        let _ = self.settings().set("editor-results-position", pos);
    }
}
