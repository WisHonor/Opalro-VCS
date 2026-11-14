//! `opalro combine`: bringing one line's work into another.

mod text;

pub use text::{Text, merge_text};

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
