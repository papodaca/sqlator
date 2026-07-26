//! Hand-written `ListModel` for query result rows (phase-0 spike pattern).

use crate::results::cell::{CellValue, ColumnMeta};
use crate::results::row::RowObject;
use gio::prelude::*;
use gio::subclass::prelude::*;
use glib::{Object, Type};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Default)]
pub struct ResultSet {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Arc<[CellValue]>>,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ResultModel {
        pub data: RefCell<ResultSet>,
        pub cache: RefCell<HashMap<u32, glib::object::WeakRef<RowObject>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ResultModel {
        const NAME: &'static str = "SqlatorResultModel";
        type Type = super::ResultModel;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for ResultModel {}

    impl ListModelImpl for ResultModel {
        fn item_type(&self) -> Type {
            RowObject::static_type()
        }

        fn n_items(&self) -> u32 {
            self.data.borrow().rows.len() as u32
        }

        fn item(&self, position: u32) -> Option<Object> {
            let data = self.data.borrow();
            let row = data.rows.get(position as usize)?;

            {
                let cache = self.cache.borrow();
                if let Some(weak) = cache.get(&position) {
                    if let Some(obj) = weak.upgrade() {
                        return Some(obj.upcast());
                    }
                }
            }

            let obj = RowObject::new(position, Arc::clone(row));
            self.cache.borrow_mut().insert(position, obj.downgrade());
            Some(obj.upcast())
        }
    }
}

glib::wrapper! {
    pub struct ResultModel(ObjectSubclass<imp::ResultModel>) @implements gio::ListModel;
}

impl ResultModel {
    pub fn new() -> Self {
        Object::builder().build()
    }

    pub fn clear(&self) {
        let n_old = self.n_items();
        self.imp().cache.borrow_mut().clear();
        *self.imp().data.borrow_mut() = ResultSet::default();
        if n_old > 0 {
            self.items_changed(0, n_old, 0);
        }
    }

    pub fn set_columns(&self, columns: Vec<ColumnMeta>) {
        let n_old = self.n_items();
        self.imp().cache.borrow_mut().clear();
        *self.imp().data.borrow_mut() = ResultSet {
            columns,
            rows: Vec::new(),
        };
        if n_old > 0 {
            self.items_changed(0, n_old, 0);
        }
    }

    pub fn columns(&self) -> Vec<ColumnMeta> {
        self.imp().data.borrow().columns.clone()
    }

    pub fn append_rows(&self, rows: Vec<Arc<[CellValue]>>) {
        if rows.is_empty() {
            return;
        }
        let start = self.n_items();
        let added = rows.len() as u32;
        self.imp().data.borrow_mut().rows.extend(rows);
        self.items_changed(start, 0, added);
    }

    pub fn row_count(&self) -> u32 {
        self.n_items()
    }
}

impl Default for ResultModel {
    fn default() -> Self {
        Self::new()
    }
}
