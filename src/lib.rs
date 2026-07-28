//! rediff — a fast Rust TUI git-diff viewer. Library surface shared by the
//! binary and the integration tests.

pub mod cli;
pub mod config;
pub mod diff;
pub mod git;
pub mod highlight;
pub mod lang;
pub mod model;
pub mod pager;
pub mod render;
pub mod review;
pub mod reviewcli;
#[cfg(test)]
mod testutil;
pub mod tui;
