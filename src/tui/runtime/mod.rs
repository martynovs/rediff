//! TUI runtime: terminal lifecycle, the input/render event loop, and key
//! dispatch. This module is import-only (see CLAUDE.md "Module structure"); the
//! logic lives in the `events` and `keys` submodules.

mod events;
mod keys;

pub use events::*;

#[cfg(test)]
mod peek_tests;
#[cfg(test)]
mod render_tests;
#[cfg(test)]
mod view_tests;

#[cfg(test)]
pub(crate) use keys::handle_key as handle_key_for_test;
