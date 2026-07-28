//! The non-interactive commands over the review store: request a review, report
//! its state, drain its feedback.
//!
//! These are the half an agent talks to. No TUI, no server — those live in the
//! surfaces that write feedback, which this layer only reads back.

mod feedback;
mod request;
mod run;
mod status;
mod target;

pub use feedback::{
    collect as collect_feedback, render as render_feedback, resolution_json, to_json, FeedbackJson,
    ResolutionJson, SubmitJson, ThreadJson,
};
pub use request::{new_review_id, render, request, Outcome, Ready, RequestError};
pub use run::{run_feedback, run_request, run_status};
pub use status::{collect as collect_status, render_human, render_json, ReviewSummary, Status};
pub use target::{encode, parse, TargetError};
