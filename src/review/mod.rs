//! The review store: an append-only log of one human review per worktree.
//!
//! The log is a single file, `rediff.jsonl`, at the worktree root — no
//! directories, nothing inside `.git`, nothing in a global cache, so cleanup is
//! `rm`. Every surface that captures feedback (a TUI review session, a local web
//! page) appends to it, and every consumer replays it; the file, not any one
//! surface, is the contract.

mod anchor;
mod drain;
mod log;
mod record;
mod round;
mod serve;

pub use anchor::{capture, resolve, Resolution, MIN_CONTEXT_MATCH, SEARCH_WINDOW};
pub use drain::{all, drain, undelivered, Delivered, DeliveredSubmit, Delivery};
pub use log::{
    fold, log_path, log_path_in, open_review, CloseRecord, Log, OpenInfo, Opened, ReviewState,
    RoundInfo, ServeRecord, ThreadState, LOG_FILE_NAME,
};
pub use record::{now, Anchor, Record, Side, Submit, Thread, CONTEXT_LINES};
pub use round::{changed_since, content_hash, hash_changeset, open_round, Changed, NO_CONTENT};
pub use serve::{last_serve, record_close, record_serve, ServeState};
