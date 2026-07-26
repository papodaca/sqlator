//! Sidebar list rows — groups and connections (no ParamSpecs; phase-0 pattern).

use gio::subclass::prelude::*;
use glib::Object;
use std::cell::{Cell, RefCell};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectionStatus {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Error,
}

#[derive(Debug, Clone)]
pub enum SidebarKind {
    Group {
        id: String,
        name: String,
        color: Option<String>,
        collapsed: bool,
        /// Nesting depth for indentation (0 = root).
        depth: u32,
        order: u32,
        parent_group_id: Option<String>,
    },
    Connection {
        id: String,
        name: String,
        color_id: String,
        db_type: String,
        host: String,
        group_id: Option<String>,
        depth: u32,
        status: ConnectionStatus,
    },
}

impl SidebarKind {
    pub fn id(&self) -> &str {
        match self {
            Self::Group { id, .. } | Self::Connection { id, .. } => id,
        }
    }

    pub fn is_group(&self) -> bool {
        matches!(self, Self::Group { .. })
    }

    pub fn is_connection(&self) -> bool {
        matches!(self, Self::Connection { .. })
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SidebarItem {
        pub kind: RefCell<Option<SidebarKind>>,
        /// Stable position hint unused by GTK; kept for future DnD.
        pub _marker: Cell<u8>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SidebarItem {
        const NAME: &'static str = "SqlatorSidebarItem";
        type Type = super::SidebarItem;
    }

    impl ObjectImpl for SidebarItem {}
}

glib::wrapper! {
    pub struct SidebarItem(ObjectSubclass<imp::SidebarItem>);
}

impl SidebarItem {
    pub fn new(kind: SidebarKind) -> Self {
        let obj: Self = Object::builder().build();
        *obj.imp().kind.borrow_mut() = Some(kind);
        obj
    }

    pub fn kind(&self) -> SidebarKind {
        self.imp()
            .kind
            .borrow()
            .clone()
            .expect("SidebarItem kind set at construction")
    }

    pub fn set_kind(&self, kind: SidebarKind) {
        *self.imp().kind.borrow_mut() = Some(kind);
    }

    pub fn connection_id(&self) -> Option<String> {
        match self.kind() {
            SidebarKind::Connection { id, .. } => Some(id),
            SidebarKind::Group { .. } => None,
        }
    }

    pub fn set_connection_status(&self, status: ConnectionStatus) {
        let mut kind = self.kind();
        if let SidebarKind::Connection { status: slot, .. } = &mut kind {
            *slot = status;
            self.set_kind(kind);
        }
    }

    pub fn connection_status(&self) -> Option<ConnectionStatus> {
        match self.kind() {
            SidebarKind::Connection { status, .. } => Some(status),
            SidebarKind::Group { .. } => None,
        }
    }
}
