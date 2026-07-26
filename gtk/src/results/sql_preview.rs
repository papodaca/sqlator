//! SQL preview dialog before transactional apply (Svelte `SqlPreviewModal` parity).

use adw::prelude::*;
use gtk::glib;
use sourceview::prelude::*;
use sqlator_core::models::SqlBatch;
use std::cell::RefCell;
use std::rc::Rc;

use super::sql_gen::format_batch_for_preview;

/// Show read-only SQL for a batch. `on_execute` runs when the user confirms.
pub fn present_sql_preview(
    parent: &impl IsA<gtk::Widget>,
    batch: &SqlBatch,
    on_execute: impl Fn() + 'static,
) {
    let dialog = adw::Dialog::new();
    dialog.set_content_width(640);
    dialog.set_content_height(420);
    dialog.set_title("Preview SQL Changes");

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let cancel_btn = gtk::Button::with_label("Cancel");
    cancel_btn.add_css_class("flat");
    let exec_btn = gtk::Button::with_label("Execute");
    exec_btn.add_css_class("suggested-action");
    header.pack_start(&cancel_btn);
    header.pack_end(&exec_btn);

    let count = gtk::Label::new(Some(&format!(
        "{} statement{}",
        batch.statements.len(),
        if batch.statements.len() == 1 { "" } else { "s" }
    )));
    count.add_css_class("dimmed");
    header.pack_end(&count);
    toolbar.add_top_bar(&header);

    let buffer = sourceview::Buffer::new(None);
    buffer.set_text(&format_batch_for_preview(batch));
    buffer.set_highlight_syntax(true);
    if let Some(language) = sourceview::LanguageManager::default().language("sql") {
        buffer.set_language(Some(&language));
    }
    let scheme_name = if adw::StyleManager::default().is_dark() {
        "Adwaita-dark"
    } else {
        "Adwaita"
    };
    if let Some(scheme) = sourceview::StyleSchemeManager::default().scheme(scheme_name) {
        buffer.set_style_scheme(Some(&scheme));
    }

    let view = sourceview::View::with_buffer(&buffer);
    view.set_editable(false);
    view.set_monospace(true);
    view.set_show_line_numbers(true);
    view.set_hexpand(true);
    view.set_vexpand(true);

    let scrolled = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .child(&view)
        .build();
    toolbar.set_content(Some(&scrolled));
    dialog.set_child(Some(&toolbar));

    let on_execute = Rc::new(RefCell::new(Some(on_execute)));

    cancel_btn.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        move |_| {
            dialog.close();
        }
    ));

    exec_btn.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        #[strong]
        on_execute,
        move |_| {
            dialog.close();
            if let Some(cb) = on_execute.borrow_mut().take() {
                cb();
            }
        }
    ));

    dialog.present(Some(parent));
}
