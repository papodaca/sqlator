//! GtkSourceView editor enhancements: completion, snippets, find/replace, vim, SQL helpers.

mod completion;
mod search;
mod snippets;
pub mod sql;

pub use completion::{attach_providers, load_snapshot, SchemaCompletionProvider, SchemaSnapshot};
pub use search::SearchBarState;
pub use snippets::ensure_registered as ensure_snippets_registered;
pub use sql::{is_destructive_sql, sql_at_cursor};

use gtk::gio;
use gtk::prelude::*;
use gtk::EventControllerKey;

/// Build a capture-phase key controller that drives VimIMContext for `editor`.
pub fn vim_key_controller(editor: &sourceview::View) -> EventControllerKey {
    let vim = sourceview::VimIMContext::new();
    vim.set_client_widget(Some(editor.upcast_ref::<gtk::Widget>()));
    let key = EventControllerKey::new();
    key.set_im_context(Some(vim.upcast_ref::<gtk::IMContext>()));
    key.set_propagation_phase(gtk::PropagationPhase::Capture);
    key
}

/// Whether vim mode is enabled in GSettings (default false).
pub fn vim_mode_enabled(settings: &gio::Settings) -> bool {
    settings.get::<bool>(crate::preferences::VIM_MODE_KEY)
}
