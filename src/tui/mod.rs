//! TUI module: declarations and re-exports only (see CLAUDE.md "Module
//! structure"). The terminal lifecycle, event loop, and key dispatch live in
//! `runtime`.

mod app;
mod blame;
mod fuzzy;
mod highlight;
mod keymap;
mod loader;
mod peek;
mod review;
mod reviewlog;
mod rows;
mod runtime;
mod session;
mod sidebar;
mod stream;
#[cfg(test)]
mod testutil;
mod theme;
mod ui;
mod view;

pub use runtime::run;
pub use theme::{Theme, ThemeName};
pub use view::ViewKind;
