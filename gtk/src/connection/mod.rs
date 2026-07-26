//! Connection sidebar: grouped list, colors, connect/disconnect.

mod colors;
mod item;
mod list;

pub use item::ConnectionStatus;
pub use list::{status_map_from_service, ConnectionList};

// Re-exports used by unit tests / future form UI.
#[allow(unused_imports)]
pub use colors::{color_css_class, color_hex, group_color_css_class};
#[allow(unused_imports)]
pub use item::{SidebarItem, SidebarKind};
