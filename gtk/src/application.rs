use crate::window::SqlatorWindow;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use sqlator_service::AppService;
use std::cell::OnceCell;
use std::sync::Arc;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SqlatorApplication {
        pub service: OnceCell<Arc<AppService>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SqlatorApplication {
        const NAME: &'static str = "SqlatorApplication";
        type Type = super::SqlatorApplication;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for SqlatorApplication {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.set_accels_for_action("app.quit", &["<Ctrl>q"]);
            obj.set_accels_for_action("app.preferences", &["<Ctrl>comma"]);
            obj.set_accels_for_action("app.new-connection", &["<Ctrl><Shift>n"]);
            obj.set_accels_for_action("win.toggle-sidebar", &["F9", "<Ctrl>b"]);
            obj.set_accels_for_action("win.new-tab", &["<Ctrl>t"]);
            obj.set_accels_for_action("win.close-tab", &["<Ctrl>w"]);
            obj.set_accels_for_action("win.tab-overview", &["<Ctrl><Shift>o"]);
            obj.set_accels_for_action("tab.run", &["<Ctrl>Return"]);
            obj.set_accels_for_action("tab.run-all", &["<Ctrl><Shift>Return"]);
            obj.set_accels_for_action("tab.run-selection", &["<Ctrl><Alt>Return"]);
            obj.set_accels_for_action("tab.cancel", &["Escape"]);
            obj.set_accels_for_action("tab.format-sql", &["<Ctrl><Shift>f"]);
            obj.set_accels_for_action("tab.find", &["<Ctrl>f"]);
            obj.set_accels_for_action("tab.copy-as-csv", &["<Ctrl><Shift>c"]);
        }
    }

    impl ApplicationImpl for SqlatorApplication {
        fn activate(&self) {
            let application = self.obj();
            let window = if let Some(window) = application.active_window() {
                window
            } else {
                let window = SqlatorWindow::new(&application);
                window.upcast()
            };
            window.present();
        }

        fn startup(&self) {
            self.parent_startup();
            let application = self.obj();

            // Shared AppService for all windows / tabs.
            let service = Arc::new(AppService::new().expect("AppService init"));
            if self.service.set(service).is_err() {
                panic!("service set once at startup");
            }

            // Theme / editor font before any window constructs SourceView buffers.
            let settings = gio::Settings::new("im.apodaca.SqlatorGtk");
            crate::theme::bind_settings(&settings);
            crate::preferences::bind_editor_font(&settings);
            application.setup_color_scheme_action(&settings);

            let quit = gio::SimpleAction::new("quit", None);
            quit.connect_activate(glib::clone!(
                #[weak]
                application,
                move |_, _| {
                    application.quit();
                }
            ));
            application.add_action(&quit);

            let preferences = gio::SimpleAction::new("preferences", None);
            preferences.connect_activate(glib::clone!(
                #[weak]
                application,
                #[strong]
                settings,
                move |_, _| {
                    if let Some(window) = application.active_window() {
                        crate::preferences::present(&window, &settings);
                    }
                }
            ));
            application.add_action(&preferences);

            let new_connection = gio::SimpleAction::new("new-connection", None);
            new_connection.connect_activate(glib::clone!(
                #[weak]
                application,
                move |_, _| {
                    let Some(window) = application.active_window() else {
                        return;
                    };
                    let Ok(window) = window.downcast::<SqlatorWindow>() else {
                        return;
                    };
                    crate::docker::present_new_connection_chooser(&window, &application);
                }
            ));
            application.add_action(&new_connection);
        }
    }

    impl GtkApplicationImpl for SqlatorApplication {}
    impl AdwApplicationImpl for SqlatorApplication {}
}

glib::wrapper! {
    pub struct SqlatorApplication(ObjectSubclass<imp::SqlatorApplication>)
        @extends gio::Application, gtk::Application, adw::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl SqlatorApplication {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("application-id", "im.apodaca.SqlatorGtk")
            .property("flags", gio::ApplicationFlags::default())
            .property("resource-base-path", "/im/apodaca/SqlatorGtk")
            .build()
    }

    pub fn service(&self) -> Arc<AppService> {
        self.imp()
            .service
            .get()
            .expect("AppService available after startup")
            .clone()
    }

    fn setup_color_scheme_action(&self, settings: &gio::Settings) {
        let action = settings.create_action(crate::theme::COLOR_SCHEME_KEY);
        self.add_action(&action);
    }
}

impl Default for SqlatorApplication {
    fn default() -> Self {
        Self::new()
    }
}
