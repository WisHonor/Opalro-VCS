//! `opalro combine`: bringing one line's work into another.

#[cfg(test)]
mod tests;
mod text;
mod tree;

pub use text::{Text, merge_text};
pub use tree::{MAX_HELD_TEXT, Made, Merged, merge_trees};

use std::collections::{BTreeMap, BTreeSet};
use std::io;

use crate::changes::DEADLINE;
use crate::kept::{EXACT, Put, overlay_with};
use crate::moves::{Version, followed, is_file, room_at};
use crate::names::PortabilityProblem;
use crate::pack::PackSet;
use crate::repo::{RepoError, Repository};
use crate::rules::RuleMatcher;
use crate::snapshot::{SaveError, store_listing};
use crate::store::StoreError;
use crate::text::escape;
use crate::writeout::Tree;

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

/// Merges `ours` and `theirs` from `start`, following moves. Everything it builds goes to
/// `packs`, its listings to `made` too, and its texts as well only when `inner`; nothing is
/// stored.
#[allow(clippy::too_many_arguments)]
#[allow(dead_code, reason = "`opalro combine` is its caller")]
pub(crate) fn merge_three(
    repo: &Repository,
    packs: &mut PackSet,
    made: &mut Made,
    inner: bool,
    start: &Version,
    ours: &Version,
    theirs: &Version,
    rules: &RuleMatcher,
) -> Result<Merged, CombineError> {
    let store = repo.store();
    let went = [
        followed(repo, made, start, ours)?,
        followed(repo, made, start, theirs)?,
    ];
    // Each version's files moved first, in the order of `Which`.
    let mut puts: [BTreeMap<String, Option<Put>>; 3] = Default::default();
    let mut clashes = Vec::new();
    {
        let mut trees =
            [start, ours, theirs].map(|v| Tree::new(store, &made.listings, v.root, EXACT));
        let froms: BTreeSet<&String> = went[0].keys().chain(went[1].keys()).collect();
        for from in froms {
            let (moved_on, to, apart) = match (went[0].get(from), went[1].get(from)) {
                (Some(o), Some(t)) if o == t => {
                    move_in(&mut trees[0], &mut puts[0], from, o)?;
                    continue;
                }
                (Some(o), Some(t)) => (
                    Side::Ours,
                    o,
                    Some(Clash::MovedApart {
                        ours_to: o.clone(),
                        theirs_to: t.clone(),
                    }),
                ),
                (Some(o), None) => (Side::Ours, o, None),
                (None, Some(t)) => (Side::Theirs, t, None),
                (None, None) => continue,
            };
            let clash = match apart {
                Some(clash) => clash,
                None => {
                    let other = if moved_on == Side::Ours { 2 } else { 1 };
                    match trees[other].entry(from)? {
                        None => Clash::MovedRemoved {
                            moved_on,
                            to: to.clone(),
                        },
                        // Made into something else: the tree merge tells of that.
                        Some(e) if !is_file(&e) => continue,
                        Some(_) if !room_at(&mut trees[other], to)? => Clash::MovedOnto {
                            moved_on,
                            to: to.clone(),
                        },
                        Some(_) => {
                            move_in(&mut trees[0], &mut puts[0], from, to)?;
                            move_in(&mut trees[other], &mut puts[other], from, to)?;
                            continue;
                        }
                    }
                }
            };
            // What any version holds there, a new file a mover made included, is part of this
            // conflict, not one of its own.
            for (tree, puts) in trees.iter_mut().zip(&mut puts) {
                if tree.entry(from)?.is_some() {
                    puts.insert(from.clone(), None);
                }
            }
            clashes.push(Conflict {
                path: from.clone(),
                clash,
            });
        }
    }
    let mut roots = [start.root, ours.root, theirs.root];
    for (root, changes) in roots.iter_mut().zip(&puts) {
        if changes.is_empty() {
            continue;
        }
        let mut built = Vec::new();
        *root = overlay_with(store, &made.listings, *root, changes, &mut |listing| {
            let id = store_listing(store, packs, listing)?;
            built.push((id, listing.clone()));
            Ok(id)
        })?;
        made.listings.extend(built);
    }
    let [s, o, t] = roots;
    let mut merged = merge_trees(store, packs, made, inner, s, o, t, rules, DEADLINE)?;
    if !clashes.is_empty() {
        // Each names a path the tree merge found nothing at, so the two are named together.
        let mut all = std::mem::take(&mut merged.conflicts);
        all.extend(clashes);
        all.sort_by(|a, b| a.path.cmp(&b.path));
        let unnamed = all.len().saturating_sub(tree::MAX_NAMED);
        all.truncate(tree::MAX_NAMED);
        merged.more += unnamed as u64;
        merged.conflicts = all;
    }
    Ok(merged)
}

/// Sets `to` in `puts` to what `tree` holds at `from`, and `from` to nothing.
fn move_in(
    tree: &mut Tree,
    puts: &mut BTreeMap<String, Option<Put>>,
    from: &str,
    to: &str,
) -> Result<(), StoreError> {
    let e = tree.entry(from)?;
    puts.insert(from.to_owned(), None);
    puts.insert(to.to_owned(), e.map(|e| (e.kind, e.id, e.size)));
    Ok(())
}
