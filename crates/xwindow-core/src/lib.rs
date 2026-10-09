//! Runtime-neutral pieces shared by every xwindow adapter.
//!
//! Windows stay in Rust and cross language boundaries as generational
//! integer handles. A window reaches a GPU library as a platform code and
//! four integers (`raw`), which xgpu's `GpuInstance.surface` takes.

mod cursor;
mod handles;
mod motion;
mod keys;
pub mod raw;

pub use cursor::{CURSOR_ICONS, css as cursor_css};
pub use handles::{Kind, Slab};
pub use keys::{KEY_CODES, NAMED_KEYS};
pub use motion::{CursorMove, CursorMoves};
