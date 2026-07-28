//! The canonical encoding of a review's target, and its inverse.
//!
//! The review log records what a review is *about* as a string
//! (`Record::Open.target`). That string has to survive a round trip: `feedback`
//! reads it back and rebuilds the very changeset an anchor was written against.
//! The store itself stays ignorant of git — it records the string and never looks
//! inside it — so encoding and parsing live here, in the CLI layer.
//!
//! ```text
//! worktree                    include_untracked = true,  base = None
//! worktree:<base>             include_untracked = true,  base = Some
//! worktree-tracked            include_untracked = false, base = None
//! worktree-tracked:<base>     include_untracked = false, base = Some
//! staged
//! show:<rev>
//! range:<old>..<new>
//! review:<base>..<target>
//! ```
//!
//! Two separator rules, both deliberate:
//!
//! - The kind is the text before the **first** `:`; everything after is the
//!   revspec, verbatim. A revspec may itself contain colons — `HEAD:path`,
//!   `:/message`, `:0:file` are all legal — so splitting on the last colon, or on
//!   `=`/`;`, would corrupt them.
//! - The two-field forms split at the **first** `..`, matching `split_range` in
//!   `crate::cli`. That leaves one shape unrepresentable: a base that itself
//!   contains `..`. [`encode`] refuses it rather than emitting something that
//!   parses back wrong.

use crate::git::LoadRequest;

/// Why a target string could not be encoded or parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetError {
    /// The kind before the first `:` is not one this build knows.
    UnknownKind(String),
    /// A two-field form was missing its `..` separator.
    MissingRange(String),
    /// A revspec contains `..`, which the two-field forms use as their separator.
    AmbiguousRange(String),
}

impl std::fmt::Display for TargetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TargetError::UnknownKind(s) => {
                write!(f, "unrecognized review target `{s}`")
            }
            TargetError::MissingRange(s) => {
                write!(f, "review target `{s}` is missing its `..` separator")
            }
            TargetError::AmbiguousRange(s) => write!(
                f,
                "revision `{s}` contains `..`, which cannot be encoded in a range target"
            ),
        }
    }
}

impl std::error::Error for TargetError {}

/// Reject a revspec that would collide with the `..` separator.
fn checked(rev: &str) -> Result<&str, TargetError> {
    if rev.contains("..") {
        return Err(TargetError::AmbiguousRange(rev.to_string()));
    }
    Ok(rev)
}

/// Encode a load request into its canonical target string.
///
/// # Errors
/// Returns [`TargetError::AmbiguousRange`] when a range endpoint contains `..`.
pub fn encode(req: &LoadRequest) -> Result<String, TargetError> {
    Ok(match req {
        LoadRequest::WorkingTree {
            include_untracked,
            base,
        } => {
            let kind = if *include_untracked {
                "worktree"
            } else {
                "worktree-tracked"
            };
            match base {
                Some(b) => format!("{kind}:{b}"),
                None => kind.to_string(),
            }
        }
        LoadRequest::Staged => "staged".to_string(),
        LoadRequest::Show { rev } => format!("show:{rev}"),
        LoadRequest::Range { old, new } => format!("range:{}..{new}", checked(old)?),
        LoadRequest::ReviewRange { base, target } => {
            format!("review:{}..{target}", checked(base)?)
        }
    })
}

/// Parse a canonical target string back into a load request.
///
/// # Errors
/// Returns [`TargetError`] when the kind is unknown or a two-field form has no
/// `..`.
pub fn parse(s: &str) -> Result<LoadRequest, TargetError> {
    let (kind, rest) = match s.split_once(':') {
        Some((k, r)) => (k, Some(r)),
        None => (s, None),
    };
    match (kind, rest) {
        ("worktree", base) => Ok(LoadRequest::WorkingTree {
            include_untracked: true,
            base: base.map(ToString::to_string),
        }),
        ("worktree-tracked", base) => Ok(LoadRequest::WorkingTree {
            include_untracked: false,
            base: base.map(ToString::to_string),
        }),
        ("staged", None) => Ok(LoadRequest::Staged),
        ("show", Some(rev)) => Ok(LoadRequest::Show {
            rev: rev.to_string(),
        }),
        ("range", Some(r)) => split_two(r).map(|(old, new)| LoadRequest::Range { old, new }),
        ("review", Some(r)) => {
            split_two(r).map(|(base, target)| LoadRequest::ReviewRange { base, target })
        }
        _ => Err(TargetError::UnknownKind(s.to_string())),
    }
}

/// Split a two-field form at its **first** `..`, matching `cli::split_range`.
fn split_two(r: &str) -> Result<(String, String), TargetError> {
    r.split_once("..")
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .ok_or_else(|| TargetError::MissingRange(r.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worktree(include_untracked: bool, base: Option<&str>) -> LoadRequest {
        LoadRequest::WorkingTree {
            include_untracked,
            base: base.map(ToString::to_string),
        }
    }

    fn round_trip(req: &LoadRequest) -> LoadRequest {
        parse(&encode(req).expect("encodes")).expect("parses")
    }

    #[test]
    fn every_variant_round_trips() {
        let cases = vec![
            worktree(true, None),
            worktree(true, Some("main")),
            worktree(false, None),
            worktree(false, Some("main")),
            LoadRequest::Staged,
            LoadRequest::Show { rev: "HEAD".into() },
            LoadRequest::Range {
                old: "main".into(),
                new: "HEAD".into(),
            },
            LoadRequest::ReviewRange {
                base: "main".into(),
                target: "feature".into(),
            },
        ];
        for req in &cases {
            assert_eq!(&round_trip(req), req, "round trip for {req:?}");
        }
    }

    #[test]
    fn untracked_inclusion_survives() {
        // The whole reason `Changeset.source` could not be reused: it drops this.
        let encoded = encode(&worktree(false, None)).unwrap();
        assert_eq!(encoded, "worktree-tracked");
        assert_eq!(parse(&encoded).unwrap(), worktree(false, None));

        let encoded = encode(&worktree(false, Some("main"))).unwrap();
        assert_eq!(encoded, "worktree-tracked:main");
        assert_eq!(parse(&encoded).unwrap(), worktree(false, Some("main")));
    }

    #[test]
    fn the_combined_base_through_worktree_target_round_trips() {
        // The agent's normal case: commits since a base plus uncommitted work.
        let req = worktree(true, Some("main"));
        assert_eq!(encode(&req).unwrap(), "worktree:main");
        assert_eq!(round_trip(&req), req);
    }

    #[test]
    fn revspecs_survive_verbatim_including_colons() {
        // Splitting at the *first* colon is what makes these safe.
        for rev in [
            "HEAD~2",
            "HEAD^",
            "HEAD:src/main.rs",
            ":/fix the bug",
            ":0:f",
        ] {
            let req = LoadRequest::Show {
                rev: rev.to_string(),
            };
            assert_eq!(round_trip(&req), req, "revspec {rev}");
        }
    }

    #[test]
    fn a_range_endpoint_containing_dotdot_is_refused_not_corrupted() {
        // `review:a..b..feature` would parse back as base="a", target="b..feature".
        let req = LoadRequest::ReviewRange {
            base: "a..b".into(),
            target: "feature".into(),
        };
        assert_eq!(
            encode(&req),
            Err(TargetError::AmbiguousRange("a..b".into()))
        );

        let req = LoadRequest::Range {
            old: "x..y".into(),
            new: "HEAD".into(),
        };
        assert!(matches!(encode(&req), Err(TargetError::AmbiguousRange(_))));
    }

    #[test]
    fn a_range_splits_at_the_first_dotdot() {
        // Matches `cli::split_range`, so `diff a..b..c` and a recorded target agree.
        assert_eq!(
            parse("range:a..b..c").unwrap(),
            LoadRequest::Range {
                old: "a".into(),
                new: "b..c".into()
            }
        );
    }

    #[test]
    fn an_unknown_kind_is_a_typed_error_naming_the_input() {
        for bad in ["telepathy", "telepathy:HEAD", "staged:oops", "show"] {
            let err = parse(bad).expect_err("must not guess");
            assert_eq!(err, TargetError::UnknownKind(bad.to_string()), "{bad}");
            assert!(err.to_string().contains(bad), "message names the input");
        }
    }

    #[test]
    fn a_two_field_form_without_a_separator_is_an_error() {
        let err = parse("range:main").expect_err("no `..`");
        assert_eq!(err, TargetError::MissingRange("main".into()));
        assert!(err.to_string().contains("main"));

        assert!(matches!(
            parse("review:main"),
            Err(TargetError::MissingRange(_))
        ));
    }

    #[test]
    fn error_display_covers_every_variant() {
        assert!(TargetError::UnknownKind("k".into())
            .to_string()
            .contains("unrecognized"));
        assert!(TargetError::MissingRange("r".into())
            .to_string()
            .contains(".."));
        assert!(TargetError::AmbiguousRange("a..b".into())
            .to_string()
            .contains("cannot be encoded"));
    }
}
