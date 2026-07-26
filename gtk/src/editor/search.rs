//! Find / replace bar backed by GtkSource SearchSettings + SearchContext.

use gtk::prelude::*;
use gtk::{glib, Orientation};
use sourceview::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

pub struct SearchBarState {
    pub search_bar: gtk::SearchBar,
    pub search_entry: gtk::SearchEntry,
    pub replace_entry: gtk::Entry,
    pub status_label: gtk::Label,
    settings: sourceview::SearchSettings,
    context: RefCell<Option<sourceview::SearchContext>>,
}

impl SearchBarState {
    pub fn new(editor: &sourceview::View) -> Rc<Self> {
        let settings = sourceview::SearchSettings::new();
        settings.set_wrap_around(true);
        settings.set_case_sensitive(false);

        let search_entry = gtk::SearchEntry::new();
        search_entry.set_hexpand(true);
        search_entry.set_placeholder_text(Some("Find"));

        let replace_entry = gtk::Entry::new();
        replace_entry.set_hexpand(true);
        replace_entry.set_placeholder_text(Some("Replace"));

        let status_label = gtk::Label::new(Some(""));
        status_label.add_css_class("dim-label");
        status_label.set_width_chars(10);

        let prev_btn = gtk::Button::from_icon_name("go-up-symbolic");
        prev_btn.set_tooltip_text(Some("Previous match"));
        let next_btn = gtk::Button::from_icon_name("go-down-symbolic");
        next_btn.set_tooltip_text(Some("Next match"));
        let replace_btn = gtk::Button::with_label("Replace");
        let replace_all_btn = gtk::Button::with_label("All");
        replace_all_btn.set_tooltip_text(Some("Replace all"));

        let case_toggle = gtk::ToggleButton::new();
        case_toggle.set_icon_name("format-text-uppercase-symbolic");
        case_toggle.set_tooltip_text(Some("Match case"));

        let box_ = gtk::Box::new(Orientation::Horizontal, 6);
        box_.set_margin_start(6);
        box_.set_margin_end(6);
        box_.set_margin_top(4);
        box_.set_margin_bottom(4);
        box_.append(&search_entry);
        box_.append(&prev_btn);
        box_.append(&next_btn);
        box_.append(&status_label);
        box_.append(&replace_entry);
        box_.append(&replace_btn);
        box_.append(&replace_all_btn);
        box_.append(&case_toggle);

        let search_bar = gtk::SearchBar::new();
        search_bar.set_child(Some(&box_));
        search_bar.connect_entry(&search_entry);
        search_bar.set_show_close_button(true);
        search_bar.set_key_capture_widget(Some(editor.upcast_ref::<gtk::Widget>()));

        let state = Rc::new(Self {
            search_bar: search_bar.clone(),
            search_entry: search_entry.clone(),
            replace_entry: replace_entry.clone(),
            status_label: status_label.clone(),
            settings: settings.clone(),
            context: RefCell::new(None),
        });

        // Bind buffer → SearchContext when available.
        if let Ok(buffer) = editor.buffer().downcast::<sourceview::Buffer>() {
            let ctx = sourceview::SearchContext::new(&buffer, Some(&settings));
            *state.context.borrow_mut() = Some(ctx);
        }

        search_entry.connect_search_changed(glib::clone!(
            #[weak]
            state,
            #[weak]
            editor,
            move |entry| {
                let text = entry.text();
                state.settings.set_search_text(if text.is_empty() {
                    None
                } else {
                    Some(text.as_str())
                });
                state.update_status();
                if !text.is_empty() {
                    state.find_next(&editor);
                }
            }
        ));

        next_btn.connect_clicked(glib::clone!(
            #[weak]
            state,
            #[weak]
            editor,
            move |_| {
                state.find_next(&editor);
            }
        ));
        prev_btn.connect_clicked(glib::clone!(
            #[weak]
            state,
            #[weak]
            editor,
            move |_| {
                state.find_prev(&editor);
            }
        ));
        replace_btn.connect_clicked(glib::clone!(
            #[weak]
            state,
            #[weak]
            editor,
            move |_| {
                state.replace_one(&editor);
            }
        ));
        replace_all_btn.connect_clicked(glib::clone!(
            #[weak]
            state,
            move |_| {
                state.replace_all();
            }
        ));
        case_toggle.connect_toggled(glib::clone!(
            #[weak]
            state,
            move |btn| {
                state.settings.set_case_sensitive(btn.is_active());
                state.update_status();
            }
        ));

        if let Some(ctx) = state.context.borrow().as_ref() {
            ctx.connect_occurrences_count_notify(glib::clone!(
                #[weak]
                state,
                move |_| {
                    state.update_status();
                }
            ));
        }

        state
    }

    pub fn reveal(&self) {
        self.search_bar.set_search_mode(true);
        self.search_entry.grab_focus();
    }

    fn find_next(&self, editor: &sourceview::View) {
        let Some(ctx) = self.context.borrow().clone() else {
            return;
        };
        let buffer = editor.buffer();
        let start = buffer
            .selection_bounds()
            .map(|(_, e)| e)
            .unwrap_or_else(|| buffer.iter_at_mark(&buffer.get_insert()));
        if let Some((match_start, match_end, _wrapped)) = ctx.forward(&start) {
            buffer.select_range(&match_start, &match_end);
            editor.scroll_to_iter(&mut match_start.clone(), 0.1, false, 0.0, 0.5);
        }
        self.update_status();
    }

    fn find_prev(&self, editor: &sourceview::View) {
        let Some(ctx) = self.context.borrow().clone() else {
            return;
        };
        let buffer = editor.buffer();
        let start = buffer
            .selection_bounds()
            .map(|(s, _)| s)
            .unwrap_or_else(|| buffer.iter_at_mark(&buffer.get_insert()));
        if let Some((match_start, match_end, _wrapped)) = ctx.backward(&start) {
            buffer.select_range(&match_start, &match_end);
            editor.scroll_to_iter(&mut match_start.clone(), 0.1, false, 0.0, 0.5);
        }
        self.update_status();
    }

    fn replace_one(&self, editor: &sourceview::View) {
        let Some(ctx) = self.context.borrow().clone() else {
            return;
        };
        let buffer = editor.buffer();
        let replace = self.replace_entry.text();
        if let Some((mut start, mut end)) = buffer.selection_bounds() {
            let _ = ctx.replace(&mut start, &mut end, replace.as_str());
        }
        self.find_next(editor);
    }

    fn replace_all(&self) {
        let Some(ctx) = self.context.borrow().clone() else {
            return;
        };
        let before = ctx.occurrences_count().max(0);
        let replace = self.replace_entry.text();
        match ctx.replace_all(replace.as_str()) {
            Ok(()) => {
                self.status_label.set_text(&format!("Replaced {before}"));
            }
            Err(e) => {
                tracing::debug!("replace_all failed: {e}");
                self.status_label.set_text("Replace failed");
            }
        }
    }

    fn update_status(&self) {
        let Some(ctx) = self.context.borrow().clone() else {
            self.status_label.set_text("");
            return;
        };
        let count = ctx.occurrences_count();
        if count < 0 {
            self.status_label.set_text("…");
        } else if count == 0 {
            self.status_label.set_text("No matches");
        } else {
            self.status_label.set_text(&format!("{count} matches"));
        }
    }
}
