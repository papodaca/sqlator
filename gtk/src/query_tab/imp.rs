use adw::subclass::prelude::*;
use gtk::gio;
use gtk::prelude::StaticTypeExt;
use gtk::{glib, CompositeTemplate};
use sqlator_service::AppService;
use std::cell::{OnceCell, RefCell};
use std::rc::Rc;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::editor::{SchemaCompletionProvider, SearchBarState};
use super::paging::QueryPaging;
use crate::results::{EditState, ResultsGrid};
use crate::window::SqlatorWindow;

#[derive(Default, CompositeTemplate)]
#[template(resource = "/im/apodaca/SqlatorGtk/ui/query_tab.ui")]
pub struct QueryTab {
    #[template_child]
    pub editor_paned: TemplateChild<gtk::Paned>,
    #[template_child]
    pub editor_column: TemplateChild<gtk::Box>,
    #[template_child]
    pub editor: TemplateChild<sourceview::View>,
    #[template_child]
    pub toast_overlay: TemplateChild<adw::ToastOverlay>,
    #[template_child]
    pub results_stack: TemplateChild<gtk::Stack>,
    #[template_child]
    pub results_grid: TemplateChild<ResultsGrid>,
    #[template_child]
    pub messages_view: TemplateChild<gtk::TextView>,

    pub service: OnceCell<Arc<AppService>>,
    pub window: OnceCell<glib::WeakRef<SqlatorWindow>>,
    /// Connection this editor runs against (Svelte per-connection query tab).
    pub connection_id: RefCell<Option<String>>,
    /// Stable id for session persistence (camelCase `PersistedQueryTab.id`).
    pub persist_id: RefCell<String>,
    pub generation: AtomicU64,
    pub cancel_token: RefCell<Option<CancellationToken>>,
    /// Dedicated cancel for scroll-driven page fetches (KTD-9); not the tab spinner.
    pub paging_cancel: RefCell<Option<CancellationToken>>,
    pub(crate) paging: RefCell<QueryPaging>,
    /// Column names from the current result (sort whitelist).
    pub result_columns: RefCell<Vec<String>>,
    pub cancel_action: OnceCell<gio::SimpleAction>,
    /// Per-tab editable results state (never a process singleton).
    pub edit_state: RefCell<EditState>,
    /// SQL from the most recent successful SELECT (for re-execute after save).
    pub last_select_sql: RefCell<Option<String>>,
    pub schema_provider: OnceCell<SchemaCompletionProvider>,
    pub search: RefCell<Option<Rc<SearchBarState>>>,
    pub vim_controller: RefCell<Option<gtk::EventControllerKey>>,
}

#[glib::object_subclass]
impl ObjectSubclass for QueryTab {
    const NAME: &'static str = "QueryTab";
    type Type = super::QueryTab;
    type ParentType = adw::Bin;

    fn class_init(klass: &mut Self::Class) {
        // Ensure custom / foreign types are registered before template inflation.
        sourceview::View::ensure_type();
        ResultsGrid::ensure_type();
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
        if let Some(token) = self.paging_cancel.borrow_mut().take() {
            token.cancel();
        }
        self.dispose_template();
    }
}

impl WidgetImpl for QueryTab {}
impl BinImpl for QueryTab {}
