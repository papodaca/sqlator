//! Process-wide Tokio runtime for IO that must not block the GTK main loop.
//!
//! Never call `RUNTIME.block_on` on the UI thread. Never capture GObjects into
//! futures passed to `RUNTIME.spawn` — only owned Rust data crosses the boundary.

use std::sync::LazyLock;

pub static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("sqlator-io")
        .build()
        .expect("failed to build tokio runtime")
});

#[macro_export]
macro_rules! spawn_tokio {
    ($fut:expr) => {
        $crate::runtime::RUNTIME.spawn($fut)
    };
}
