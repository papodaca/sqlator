use adw::subclass::prelude::*;
use gtk::gio;
use gtk::prelude::StaticTypeExt;
use gtk::{glib, CompositeTemplate};
use sqlator_service::AppService;
use std::cell::{OnceCell, RefCell};
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::window::SqlatorWindow;

#[derive(Default, CompositeTemplate)]
#[template(resource = "/im/apodaca/SqlatorGtk/ui/query_tab.ui")]
pub struct QueryTab {
    #[template_child]
    pub editor_paned: TemplateChild<gtk::Paned>,
    #[template_child]
    pub editor: TemplateChild<sourceview::View>,
    #[template_child]
    pub toast_overlay: TemplateChild<adw::ToastOverlay>,
    #[template_child]
    pub results_stack: TemplateChild<gtk::Stack>,
    #[template_child]
    pub results_view: TemplateChild<gtk::TextView>,
    #[template_child]
    pub messages_view: TemplateChild<gtk::TextView>,

    pub service: OnceCell<Arc<AppService>>,
    pub window: OnceCell<glib::WeakRef<SqlatorWindow>>,
    pub generation: AtomicU64,
    pub cancel_token: RefCell<Option<CancellationToken>>,
    pub cancel_action: OnceCell<gio::SimpleAction>,
}

#[glib::object_subclass]
impl ObjectSubclass for QueryTab {
    const NAME: &'static str = "QueryTab";
    type Type = super::QueryTab;
    type ParentType = adw::Bin;

    fn class_init(klass: &mut Self::Class) {
        // Ensure SourceView type is registered before template inflation.
        sourceview::View::ensure_type();
        klass.bind_template();
    }

    fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
        obj.init_template();
    }
}

impl ObjectImpl for QueryTab {
    fn dispose(&self) {
        // Cancel any in-flight query when the tab is destroyed.
        if let Some(token) = self.cancel_token.borrow_mut().take() {
            token.cancel();
        }
        self.dispose_template();
    }
}

impl WidgetImpl for QueryTab {}
impl BinImpl for QueryTab {}
