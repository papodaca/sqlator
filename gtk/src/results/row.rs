//! Minimal GObject row wrapper — no ParamSpecs (per phase-0 spike).

use crate::results::cell::CellValue;
use gio::subclass::prelude::*;
use glib::Object;
use std::cell::{Cell, RefCell};
use std::sync::Arc;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct RowObject {
        pub index: Cell<u32>,
        pub values: RefCell<Option<Arc<[CellValue]>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RowObject {
        const NAME: &'static str = "SqlatorRowObject";
        type Type = super::RowObject;
    }

    impl ObjectImpl for RowObject {}
}

glib::wrapper! {
    pub struct RowObject(ObjectSubclass<imp::RowObject>);
}

impl RowObject {
    pub fn new(index: u32, values: Arc<[CellValue]>) -> Self {
        let obj: Self = Object::builder().build();
        obj.imp().index.set(index);
        *obj.imp().values.borrow_mut() = Some(values);
        obj
    }

    pub fn index(&self) -> u32 {
        self.imp().index.get()
    }

    pub fn value(&self, col: usize) -> CellValue {
        self.imp()
            .values
            .borrow()
            .as_ref()
            .and_then(|v| v.get(col).cloned())
            .unwrap_or(CellValue::Null)
    }

    pub fn values(&self) -> Arc<[CellValue]> {
        self.imp()
            .values
            .borrow()
            .clone()
            .unwrap_or_else(|| Arc::from(Vec::new().into_boxed_slice()))
    }

    pub fn set_values(&self, values: Arc<[CellValue]>) {
        *self.imp().values.borrow_mut() = Some(values);
    }
}
