//! Review app state and navigation actions. Navigation is a viewport-offset
//! change over the flat row plan — no per-keystroke re-layout. The app owns a
//! browser-style stack of views (`view::ViewEntry`); switching a view rebuilds
//! the plan and resets highlighting.
//!
//! Declarations and re-exports only (see CLAUDE.md "Module structure"). The
//! `App` type and its impl are split across `types`, `appcore`, `nav`, `overlays`,
//! and `peekview`. `types` holds the public surface (the structs/enums/consts
//! reached as `crate::tui::app::Item`); the other submodules only add `impl App`
//! methods (resolved on the value, so no re-export needed) plus internal helpers
//! reached via `super::<sub>::…`.

mod appcore;
mod comment;
mod nav;
mod overlays;
mod peekview;
mod types;

pub use types::*;
