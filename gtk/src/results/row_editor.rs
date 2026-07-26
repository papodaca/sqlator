//! Row-detail editor dialog (preferred over GtkEditableLabel per Phase 0 / 3i plan).

use adw::prelude::*;
use gtk::glib;
use sqlator_core::models::TableMeta;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RowEditorMode {
    Existing,
    Added,
}

pub struct RowEditorResult {
    pub values: HashMap<String, serde_json::Value>,
    pub delete: bool,
}

/// Present a form dialog for one result row. `on_done` receives `None` on cancel.
pub fn present_row_editor(
    parent: &impl IsA<gtk::Widget>,
    table_meta: &TableMeta,
    column_names: &[String],
    current: &HashMap<String, serde_json::Value>,
    mode: RowEditorMode,
    on_done: impl Fn(Option<RowEditorResult>) + 'static,
) {
    let dialog = adw::Dialog::new();
    dialog.set_content_width(480);
    dialog.set_title(match mode {
        RowEditorMode::Existing => "Edit Row",
        RowEditorMode::Added => "New Row",
    });

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let cancel_btn = gtk::Button::with_label("Cancel");
    cancel_btn.add_css_class("flat");
    let apply_btn = gtk::Button::with_label(match mode {
        RowEditorMode::Existing => "Apply",
        RowEditorMode::Added => "Add",
    });
    apply_btn.add_css_class("suggested-action");
    header.pack_start(&cancel_btn);
    header.pack_end(&apply_btn);

    let delete_btn = if mode == RowEditorMode::Existing {
        let btn = gtk::Button::with_label("Mark Deleted");
        btn.add_css_class("destructive-action");
        header.pack_end(&btn);
        Some(btn)
    } else {
        None
    };

    toolbar.add_top_bar(&header);

    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(440);
    let group = adw::PreferencesGroup::new();
    group.set_title(&table_meta.table_name);

    let entries: Rc<RefCell<HashMap<String, (adw::EntryRow, gtk::CheckButton)>>> =
        Rc::new(RefCell::new(HashMap::new()));

    for col in &table_meta.columns {
        if !column_names.iter().any(|n| n == &col.name) {
            continue;
        }
        let editable = match mode {
            RowEditorMode::Added => !col.is_generated,
            RowEditorMode::Existing => col.is_updatable && table_meta.is_editable,
        };

        let row = adw::EntryRow::builder().title(&col.name).build();
        let value = current
            .get(&col.name)
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let is_null = value.is_null();
        if !is_null {
            row.set_text(&json_to_entry_text(&value));
        }
        row.set_sensitive(editable);

        let null_toggle = gtk::CheckButton::with_label("NULL");
        null_toggle.set_active(is_null);
        null_toggle.set_sensitive(editable && col.nullable);
        null_toggle.set_valign(gtk::Align::Center);
        {
            let row_w = row.clone();
            null_toggle.connect_toggled(move |btn| {
                row_w.set_sensitive(!btn.is_active());
                if btn.is_active() {
                    row_w.set_text("");
                }
            });
        }
        if is_null {
            row.set_sensitive(false);
        }
        row.add_suffix(&null_toggle);
        group.add(&row);
        entries
            .borrow_mut()
            .insert(col.name.clone(), (row, null_toggle));
    }

    clamp.set_child(Some(&group));
    let scrolled = gtk::ScrolledWindow::builder()
        .propagate_natural_height(true)
        .max_content_height(420)
        .child(&clamp)
        .build();
    toolbar.set_content(Some(&scrolled));
    dialog.set_child(Some(&toolbar));

    let on_done = Rc::new(RefCell::new(Some(on_done)));

    cancel_btn.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        #[strong]
        on_done,
        move |_| {
            dialog.close();
            if let Some(cb) = on_done.borrow_mut().take() {
                cb(None);
            }
        }
    ));

    let table_meta = table_meta.clone();
    apply_btn.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        #[strong]
        on_done,
        #[strong]
        entries,
        #[strong]
        table_meta,
        move |_| {
            let mut values = HashMap::new();
            for col in &table_meta.columns {
                let Some((entry, null_toggle)) = entries.borrow().get(&col.name).cloned() else {
                    continue;
                };
                if null_toggle.is_active() {
                    values.insert(col.name.clone(), serde_json::Value::Null);
                } else {
                    values.insert(
                        col.name.clone(),
                        entry_text_to_json(entry.text().as_str(), &col.column_type),
                    );
                }
            }
            dialog.close();
            if let Some(cb) = on_done.borrow_mut().take() {
                cb(Some(RowEditorResult {
                    values,
                    delete: false,
                }));
            }
        }
    ));

    if let Some(delete_btn) = delete_btn {
        delete_btn.connect_clicked(glib::clone!(
            #[weak]
            dialog,
            #[strong]
            on_done,
            move |_| {
                dialog.close();
                if let Some(cb) = on_done.borrow_mut().take() {
                    cb(Some(RowEditorResult {
                        values: HashMap::new(),
                        delete: true,
                    }));
                }
            }
        ));
    }

    dialog.present(Some(parent));
}

fn json_to_entry_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

fn entry_text_to_json(text: &str, column_type: &str) -> serde_json::Value {
    let t = column_type.to_ascii_lowercase();
    if t.contains("bool") {
        return match text.trim().to_ascii_lowercase().as_str() {
            "true" | "t" | "1" | "yes" => serde_json::Value::Bool(true),
            "false" | "f" | "0" | "no" => serde_json::Value::Bool(false),
            _ => serde_json::Value::String(text.to_string()),
        };
    }
    if t.contains("int") || t == "integer" || t == "bigint" || t == "smallint" {
        if let Ok(i) = text.trim().parse::<i64>() {
            return serde_json::json!(i);
        }
    }
    if t.contains("float")
        || t.contains("double")
        || t.contains("real")
        || t.contains("numeric")
        || t.contains("decimal")
    {
        if let Ok(f) = text.trim().parse::<f64>() {
            return serde_json::json!(f);
        }
    }
    serde_json::Value::String(text.to_string())
}
