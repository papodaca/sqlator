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

    pub fn append_row(&self, row: Arc<[CellValue]>) -> u32 {
        let index = self.n_items();
        self.imp().data.borrow_mut().rows.push(row);
        self.items_changed(index, 0, 1);
        index
    }

    pub fn row_values(&self, index: u32) -> Option<Arc<[CellValue]>> {
        self.imp().data.borrow().rows.get(index as usize).cloned()
    }

    pub fn set_row_values(&self, index: u32, values: Arc<[CellValue]>) {
        let mut data = self.imp().data.borrow_mut();
        let Some(slot) = data.rows.get_mut(index as usize) else {
            return;
        };
        *slot = values;
        drop(data);
        self.imp().cache.borrow_mut().remove(&index);
        self.items_changed(index, 1, 1);
    }

    pub fn remove_row(&self, index: u32) {
        let mut data = self.imp().data.borrow_mut();
        if (index as usize) >= data.rows.len() {
            return;
        }
        data.rows.remove(index as usize);
        drop(data);
        self.imp().cache.borrow_mut().clear();
        self.items_changed(index, 1, 0);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::results::cell::{CellValue, ColumnMeta};
    use gio::prelude::ListModelExt;
    use std::cell::Cell;
    use std::rc::Rc;

    fn fixture_rows(n: usize) -> Vec<Arc<[CellValue]>> {
        (0..n)
            .map(|i| Arc::from(vec![CellValue::Int(i as i64)].into_boxed_slice()))
            .collect()
    }

    #[test]
    fn result_model_reports_row_count_and_items() {
        let model = ResultModel::new();
        model.set_columns(vec![ColumnMeta::from_name("id")]);
        model.append_rows(fixture_rows(1_000));
        assert_eq!(model.n_items(), 1_000);
        let row = model
            .item(999)
            .and_downcast::<RowObject>()
            .expect("row 999");
        assert_eq!(row.value(0), CellValue::Int(999));
        assert_eq!(row.index(), 999);
    }

    #[test]
    fn weak_ref_cache_does_not_return_stale_objects_after_set_row() {
        let model = ResultModel::new();
        model.set_columns(vec![ColumnMeta::from_name("v")]);
        model.append_row(Arc::from(
            vec![CellValue::Text("old".into())].into_boxed_slice(),
        ));

        let first = model
            .item(0)
            .and_downcast::<RowObject>()
            .expect("cached row");
        assert_eq!(first.value(0), CellValue::Text("old".into()));

        model.set_row_values(
            0,
            Arc::from(vec![CellValue::Text("new".into())].into_boxed_slice()),
        );

        let second = model
            .item(0)
            .and_downcast::<RowObject>()
            .expect("fresh row");
        assert_eq!(second.value(0), CellValue::Text("new".into()));
        // Cache entry was dropped on set_row_values; a new GObject is minted.
        assert_ne!(first.as_ptr(), second.as_ptr());
    }

    #[test]
    fn append_rows_emits_single_items_changed_for_batch() {
        let model = ResultModel::new();
        model.set_columns(vec![ColumnMeta::from_name("id")]);

        let emissions = Rc::new(Cell::new(0u32));
        let removed = Rc::new(Cell::new(0u32));
        let added = Rc::new(Cell::new(0u32));
        model.connect_items_changed(glib::clone!(
            #[strong]
            emissions,
            #[strong]
            removed,
            #[strong]
            added,
            move |_, _pos, r, a| {
                emissions.set(emissions.get() + 1);
                removed.set(r);
                added.set(a);
            }
        ));

        model.append_rows(fixture_rows(50));
        assert_eq!(emissions.get(), 1);
        assert_eq!(removed.get(), 0);
        assert_eq!(added.get(), 50);
        assert_eq!(model.n_items(), 50);
    }
}
