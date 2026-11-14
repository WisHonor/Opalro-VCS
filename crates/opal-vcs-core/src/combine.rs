//! `opalro combine`: bringing one line's work into another.

#[cfg(test)]
mod tests;
mod text;
mod tree;

pub use text::{Text, merge_text};
pub use tree::{MAX_HELD_TEXT, Made, Merged, merge_trees};

use std::io;

use crate::names::PortabilityProblem;
use crate::repo::RepoError;
use crate::snapshot::SaveError;
use crate::store::StoreError;
use crate::text::escape;

/// Why both sides' changes to a file aren't merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotMerged {
    /// Text, but changes from the two sides overlap or touch.
    Overlap {
        endings_only: bool,
    },
    Binary,
    TooBig,
    TooManyLines,
    /// Its `conflict-markers` rule is `false`.
    NoMarkers,
    /// A link, a project link, or not a file on every side.
    NotFiles,
}

/// One of the two versions being combined: ours is the line the project is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Ours,
    Theirs,
}

/// What happened to one path on the two sides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clash {
    BothChanged(NotMerged),
    ChangedRemoved { removed_on: Side },
    BothAdded,
    FileAndFolder { folder_on: Side },
    MovedApart { ours_to: String, theirs_to: String },
    MovedRemoved { moved_on: Side, to: String },
    MovedOnto { moved_on: Side, to: String },
}

/// A path the two sides changed in ways that can't both be kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub path: String,
    pub clash: Clash,
}

impl Conflict {
    /// `<path>: <what happened>`, the path and names escaped; `ours` and `theirs` name the sides.
    #[must_use]
    pub fn describe(&self, ours: &str, theirs: &str) -> String {
        let name = |side| escape(if side == Side::Ours { ours } else { theirs }, false);
        // The side that did the first thing, then the other.
        let pair = |first| {
            let other = if first == Side::Ours {
                Side::Theirs
            } else {
                Side::Ours
            };
            (name(first), name(other))
        };
        let both = "changed on both lines";
        let what = match &self.clash {
            Clash::BothChanged(why) => match why {
                NotMerged::Overlap {
                    endings_only: false,
                } => format!("{both}, in the same place"),
                NotMerged::Overlap { endings_only: true } => {
                    format!("{both}, in the same place; the two differ only in line endings")
                }
                NotMerged::Binary => {
                    format!("{both}, and isn't text, so it can't be merged line by line")
                }
                NotMerged::TooBig => {
                    format!("{both}, and is over 64 MiB, too big to merge line by line")
                }
                NotMerged::TooManyLines => {
                    format!("{both}, and has over 1,000,000 lines, too many to merge line by line")
                }
                NotMerged::NoMarkers => {
                    format!("{both}, and opalro.toml says conflict-markers = false for it")
                }
                NotMerged::NotFiles => {
                    format!("{both}, and only files are merged line by line")
                }
            },
            Clash::ChangedRemoved { removed_on } => {
                let (b, a) = pair(*removed_on);
                format!("changed on {a}, removed on {b}")
            }
            Clash::BothAdded => "added on both lines, differently".to_owned(),
            Clash::FileAndFolder { folder_on } => {
                let (b, a) = pair(*folder_on);
                format!("a file on {a}, a folder on {b}")
            }
            Clash::MovedApart { ours_to, theirs_to } => format!(
                "moved to {} on {}, and to {} on {}",
                escape(ours_to, false),
                name(Side::Ours),
                escape(theirs_to, false),
                name(Side::Theirs)
            ),
            Clash::MovedRemoved { moved_on, to } => {
                let (a, b) = pair(*moved_on);
                format!("moved to {} on {a}, removed on {b}", escape(to, false))
            }
            Clash::MovedOnto { moved_on, to } => {
                let (a, b) = pair(*moved_on);
                format!(
                    "moved to {} on {a}, where {b} has something else",
                    escape(to, false)
                )
            }
        };
        format!("{}: {what}", escape(&self.path, false))
    }
}

/// Which of the three versions a merge reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    Start,
    Ours,
    Theirs,
}

/// Why a combine stopped.
#[derive(Debug)]
pub enum CombineError {
    /// A merged folder would hold two names that clash; `folder` is `""` at the top.
    NamesClash {
        folder: String,
        problem: PortabilityProblem,
    },
    /// A version read is past a save's limits.
    Past {
        which: Which,
        detail: String,
    },
    /// Inner combines merged more text than `MAX_HELD_TEXT`.
    HeldTooMuch,
    /// The result is past them.
    TooBig {
        detail: String,
    },
    Repo(RepoError),
}

impl From<RepoError> for CombineError {
    fn from(e: RepoError) -> Self {
        CombineError::Repo(e)
    }
}

impl From<StoreError> for CombineError {
    fn from(e: StoreError) -> Self {
        CombineError::Repo(e.into())
    }
}

impl From<SaveError> for CombineError {
    fn from(e: SaveError) -> Self {
        match e {
            SaveError::Repo(e) => CombineError::Repo(e),
            // Reading and storing what is stored meets only the store's own errors.
            other => CombineError::Repo(io::Error::other(other.to_string()).into()),
        }
    }
}
