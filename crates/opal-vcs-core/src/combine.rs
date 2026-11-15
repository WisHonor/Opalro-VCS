//! `opalro combine`: bringing one line's work into another.

#[cfg(test)]
mod tests;
mod text;
mod tree;

pub use text::{Text, merge_text};
pub use tree::MAX_HELD_TEXT;
pub(crate) use tree::Made;
use tree::{Merged, merge_trees};

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::Path;
use std::time::Duration;

use crate::ancestry::{Starts, common_starts, only_in};
use crate::changes::deadline;
use crate::conflict::markers::{Parted, Sides};
use crate::conflict::{Listings, stands_as, unreadable};
use crate::diary::{Position, State};
use crate::history::tip_of;
use crate::id::{ChangeId, Id};
use crate::kept::{EXACT, Put, overlay_with};
use crate::lines::Target;
use crate::listing::{Entry, Listing, SaveLimits};
use crate::moves::{
    MAX_CHECKED_NOTES, MAX_LEFT_BYTES, MAX_NOTED_BYTES, Version, first_that_fit, followed, is_file,
    room_at, same_contents,
};
use crate::names::PortabilityProblem;
use crate::pack::{ObjectKind, PackSet};
use crate::repo::{ChangeLock, RepoError, Repository};
use crate::rules::{RuleMatcher, read_saved};
use crate::save::{EXTRA_LINE, MAX_MOVE_NOTES, Person, Save};
use crate::snapshot::{NAMED, SaveError, listed, store_listing};
use crate::store::StoreError;
use crate::switch::{Earlier, Planned, SwitchError, landing, locked, make_switch, plan_switch};
use crate::text::{escape, thousands};
use crate::warning::{self, Warning};
use crate::writeout::{Applied, Tree};

/// The most inner combines one combine makes to combine its common starting points.
pub const MAX_INNER: usize = 64;

/// Which of the three cases a combine was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Combined {
    /// The line already had everything the target has.
    Nothing,
    /// The line only moved forward, to the target's save.
    Forward,
    /// A combined save, from these common starting points, with these files merged line by line.
    Made {
        starts: Vec<Id>,
        merged_files: Vec<String>,
        /// The places this combine made, the first 100 in path order, and how many more.
        conflicts: Vec<Conflict>,
        more: u64,
        /// Every kind-6 entry the combined save holds, counted where its result is measured.
        conflict_count: u64,
    },
}

/// Whether a combine keeps the conflicts it meets, or stops as milestone 8's did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    Conflicts,
    OnlyIfClean,
}

/// How a kept conflict stands in the folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stands {
    Markers { places: u64 },
    Yours,
    Nothing,
}

/// Which line holds a conflict where both lines changed a place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Held {
    Ours,
    Theirs,
    Both,
}

/// Why conflicts weren't kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotKept {
    OnlyIfClean,
    NoFeature,
    RulesNotWhole,
}

/// What `combine` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombineOutcome {
    pub line: String,
    pub result: Combined,
    /// The line's newest save now.
    pub save: Id,
    /// The target's save.
    pub with: Id,
    /// As `goto` gives them; default for `Nothing`, which writes nothing.
    pub applied: Applied,
    pub safety_copies: usize,
    pub carried: usize,
}

/// How the target is named: `raw` goes into the default message as it is; `shown` is the same
/// name made safe to print (an escaped line name, `tags::shown_name` for a tag, an ID as is).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    pub raw: String,
    pub shown: String,
}

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
    /// Its `conflict-markers` rule is `false`; `theirs`: only in the target's `opalro.toml`.
    NoMarkers {
        theirs: bool,
    },
    /// It would have been merged, but `opalro.toml` wasn't read whole; `theirs`: only the
    /// target's.
    RulesNotWhole {
        theirs: bool,
    },
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
    ChangedRemoved {
        removed_on: Side,
    },
    BothAdded,
    FileAndFolder {
        folder_on: Side,
    },
    MovedApart {
        ours_to: String,
        theirs_to: String,
    },
    MovedRemoved {
        moved_on: Side,
        to: String,
    },
    MovedOnto {
        moved_on: Side,
        to: String,
    },
    /// Both lines changed it, and where they parted it was in conflict.
    StartUnknown,
}

/// A path the two sides changed in ways that can't both be kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub path: String,
    pub clash: Clash,
    /// `None` when not kept.
    pub stands: Option<Stands>,
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
        let on_theirs = |theirs| {
            if theirs {
                format!(" on {}", name(Side::Theirs))
            } else {
                String::new()
            }
        };
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
                NotMerged::NoMarkers { theirs } => format!(
                    "{both}, and opalro.toml{} says conflict-markers = false for it",
                    on_theirs(*theirs)
                ),
                NotMerged::RulesNotWhole { theirs } => format!(
                    "{both}, and opalro.toml{} can't be read whole, so no file is merged line by \
                     line",
                    on_theirs(*theirs)
                ),
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
            Clash::StartUnknown => {
                format!("{both}, and Opalro can't tell what it held where they parted")
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

/// The line and the target as a combine's words name them, each safe to print: the line's name
/// escaped, the target as [`Named::shown`] has it. Empty until the combine knows them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Names {
    pub line: String,
    pub target: String,
}

/// Why a combine stopped.
#[derive(Debug, thiserror::Error)]
pub enum CombineError {
    #[error("You're at a save, not on a line, so there is no line to combine into.")]
    NotOnALine,
    #[error(
        "{} and {} have no save in common, so there is no starting point to combine them from, \
         and nothing was changed.",
        names.line,
        names.target
    )]
    Unrelated { names: Names },
    /// Too many starting points at one level, or too many inner combines in all.
    #[error(
        "The histories of {} and {} cross each other too many times for Opalro to combine them, \
         so nothing was changed.",
        names.line,
        names.target
    )]
    Tangled { names: Names },
    /// The first 100 conflicts in path order, and how many more there were; `rules_not_whole`
    /// says whose `opalro.toml`, if any, wasn't read whole and left one, named or counted, there.
    #[error("{}", conflicts_words(names, conflicts, *more, *rules_not_whole, *why))]
    Conflicts {
        names: Names,
        conflicts: Vec<Conflict>,
        more: u64,
        rules_not_whole: Option<Side>,
        /// Why conflicts weren't kept: asked not to, the project can't keep them, or rules that
        /// can't be read whole would decide a place (then `rules_not_whole` says whose).
        why: NotKept,
    },
    /// Both lines changed `path`, and `on` holds it in conflict (`at == path`) or holds the
    /// conflict `at` below it.
    #[error("{}", held_words(names, path, at, *on))]
    HeldConflict {
        names: Names,
        path: String,
        at: String,
        on: Held,
    },
    #[error(
        "Combining {} into {} needs their common starting points combined first, and those \
         conflict with each other, which Opalro can't keep yet, so nothing was changed.",
        names.target,
        names.line
    )]
    /// `rules_not_whole`: a file among them would have merged, but that side's `opalro.toml`
    /// wasn't read whole.
    StartsConflict {
        names: Names,
        rules_not_whole: Option<Side>,
    },
    /// A merged folder would hold two names that clash; `folder` is `""` at the top.
    #[error(
        "Combining {} into {} would put two names in one folder that clash, so nothing was \
         changed:\n  {}: {problem}",
        names.target,
        names.line,
        if folder.is_empty() { "in the top folder".to_owned() } else { format!("in {}/", escape(folder, false)) }
    )]
    NamesClash {
        names: Names,
        folder: String,
        problem: Box<PortabilityProblem>,
    },
    /// A version read is past a save's limits.
    #[error(
        "{} is past Opalro's limits for a save ({detail}), so nothing was changed.",
        past_what(names, *which)
    )]
    Past {
        names: Names,
        which: Which,
        detail: String,
    },
    /// Inner combines merged more text than `MAX_HELD_TEXT`.
    #[error(
        "Combining {} into {} needs their common starting points combined first, and that merges \
         more than 256 MiB of text, more than Opalro holds in memory for it, so nothing was \
         changed.",
        names.target,
        names.line
    )]
    HeldTooMuch { names: Names },
    /// The result is past them.
    #[error(
        "The combined save would be past Opalro's limits for a save ({detail}), so nothing was \
         changed."
    )]
    TooBig { detail: String },
    /// Unsaved work, or ignored files, where the switch to the result would write.
    #[error(
        "Your unsaved changes, or files Opalro ignores, are in the way of the combine, so nothing \
         was changed:{}",
        listed(paths.iter().take(NAMED), paths.len() as u64)
    )]
    Collisions { paths: Vec<String> },
    #[error(transparent)]
    Switch(SwitchError),
    #[error(transparent)]
    Repo(RepoError),
}

impl CombineError {
    /// What the user can do next, if there is something to say.
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        let hint = match self {
            CombineError::NotOnALine => {
                "Go to a line first (opalro goto <line>), or start one here: opalro line new <name>"
            }
            CombineError::Unrelated { .. } => "Opalro combines only lines that share some history.",
            CombineError::Conflicts {
                rules_not_whole: Some(Side::Ours),
                ..
            }
            | CombineError::StartsConflict {
                rules_not_whole: Some(Side::Ours),
                ..
            } => {
                "opalro setup says what is wrong with opalro.toml. Fix or remove it and save; then \
                 run opalro combine again."
            }
            CombineError::Conflicts {
                rules_not_whole: Some(Side::Theirs),
                ..
            }
            | CombineError::StartsConflict {
                rules_not_whole: Some(Side::Theirs),
                ..
            } => {
                "The other side's opalro.toml has a mistake in it. Go there, fix or remove it \
                 (opalro setup says what is wrong) and save; then run opalro combine again."
            }
            CombineError::Conflicts { .. } => {
                "For each of these files, make the two lines agree and save; then run opalro \
                 combine again."
            }
            CombineError::HeldConflict { on: Held::Both, .. } => {
                "Fix it on both lines first (opalro resolve on each), then run opalro combine again."
            }
            CombineError::HeldConflict { .. } => {
                "Fix it on the line that holds it first (opalro resolve there), then run opalro \
                 combine again."
            }
            CombineError::NamesClash { .. } => {
                "Rename one of them on its line and save; then run opalro combine again."
            }
            CombineError::Collisions { .. } => {
                "Save them first (opalro save \"describe your change\"), shelve them (opalro shelf) \
                 or discard them (opalro discard), and move ignored files somewhere else, then run \
                 opalro combine again."
            }
            CombineError::Switch(e) => return e.hint(),
            CombineError::Repo(e) => return e.hint(),
            CombineError::Tangled { .. }
            | CombineError::StartsConflict { .. }
            | CombineError::Past { .. }
            | CombineError::HeldTooMuch { .. }
            | CombineError::TooBig { .. } => return None,
        };
        Some(hint.to_owned())
    }

    /// This error, naming the line and the target its words print.
    fn named(mut self, given: &Names) -> CombineError {
        match &mut self {
            CombineError::Unrelated { names }
            | CombineError::Tangled { names }
            | CombineError::Conflicts { names, .. }
            | CombineError::HeldConflict { names, .. }
            | CombineError::StartsConflict { names, .. }
            | CombineError::NamesClash { names, .. }
            | CombineError::Past { names, .. }
            | CombineError::HeldTooMuch { names } => *names = given.clone(),
            _ => {}
        }
        self
    }
}

fn conflicts_words(
    names: &Names,
    conflicts: &[Conflict],
    more: u64,
    rules_not_whole: Option<Side>,
    why: NotKept,
) -> String {
    let count = conflicts.len() as u64 + more;
    let mut text = if why == NotKept::RulesNotWhole {
        let places = if count == 1 {
            "1 place".to_owned()
        } else {
            format!("{} places", thousands(count))
        };
        let which = if rules_not_whole == Some(Side::Theirs) {
            format!("the opalro.toml on {}", names.target)
        } else {
            "it".to_owned()
        };
        format!(
            "Combining {} into {} would have to merge {places} that opalro.toml decides, and \
             {which} can't be read whole, so nothing was changed:",
            names.target, names.line
        )
    } else {
        let files = if count == 1 {
            "1 file".to_owned()
        } else {
            format!("{} files", thousands(count))
        };
        format!(
            "Combining {} into {} would leave {files} in conflict, which Opalro can't keep yet, \
             so nothing was changed:",
            names.target, names.line
        )
    };
    for c in conflicts {
        text += "\n  ";
        text += &c.describe(&names.line, &names.target);
    }
    if more > 0 {
        text += &format!("\n  … and {} more", thousands(more));
    }
    text
}

fn held_words(names: &Names, path: &str, at: &str, on: Held) -> String {
    let (line, target) = (&names.line, &names.target);
    let (holds, other) = if on == Held::Theirs {
        (target, line)
    } else {
        (line, target)
    };
    let path = escape(path, false);
    if on == Held::Both {
        return format!(
            "{path} is in conflict on both {line} and {target}, so nothing was changed."
        );
    }
    let at = escape(at, false);
    if at == path {
        format!(
            "{path} is in conflict on {holds}, and {other} changed it too, so nothing was changed."
        )
    } else {
        format!(
            "{at}, inside {path}, is in conflict on {holds}, and {other} changed {path} too, so \
             nothing was changed."
        )
    }
}

fn past_what(names: &Names, which: Which) -> String {
    match which {
        Which::Start => format!("The save where {} and {} parted", names.line, names.target),
        Which::Ours => format!("Line {}'s newest save", names.line),
        Which::Theirs => names.target.clone(),
    }
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
    let went = match followed(repo, made, start, ours)
        .and_then(|o| Ok([o, followed(repo, made, start, theirs)?]))
    {
        Ok(went) => went,
        Err(e) => return Err(past_limits(repo, made, [start, ours, theirs]).unwrap_or(e.into())),
    };
    // Each version's files moved first, in the order of `Which`.
    let mut puts: [BTreeMap<String, Option<Put>>; 3] = Default::default();
    let mut clashes = Vec::new();
    // Each followed file's name at the start, by where it went: its rule is asked under both.
    let mut moved = BTreeMap::new();
    {
        let stored = Listings::stored(store, &made.listings);
        let mut trees = [start, ours, theirs].map(|v| Tree::new(&stored, v.root, EXACT));
        let froms: BTreeSet<&String> = went[0].went.keys().chain(went[1].went.keys()).collect();
        for from in froms {
            let (moved_on, to, apart) = match (went[0].went.get(from), went[1].went.get(from)) {
                (Some(o), Some(t)) if o == t => {
                    move_in(&mut trees[0], &mut puts[0], from, o)?;
                    moved.insert(o.clone(), from.clone());
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
                    // A file its own notes brought there is not the start's.
                    let held = if went[other - 1].replaced.contains(from) {
                        None
                    } else {
                        trees[other].entry(from)?
                    };
                    match held {
                        None => Clash::MovedRemoved {
                            moved_on,
                            to: to.clone(),
                        },
                        // Made into something else: the tree merge tells of that.
                        Some(e) if !is_file(&e) => continue,
                        // The clash at `to` itself is the tree merge's to name as well.
                        Some(_) if !room_at(&mut trees[other], to)? => Clash::MovedOnto {
                            moved_on,
                            to: to.clone(),
                        },
                        Some(_) => {
                            move_in(&mut trees[0], &mut puts[0], from, to)?;
                            move_in(&mut trees[other], &mut puts[other], from, to)?;
                            moved.insert(to.clone(), from.clone());
                            continue;
                        }
                    }
                }
            };
            clash_at(&mut trees, &mut puts, &mut clashes, from, clash)?;
        }
        // Where a side's notes show another file arriving at a start file's place, the start's
        // file is gone on that side for the whole merge, never changed into the one that came.
        let arrived: BTreeSet<&String> = went[0].replaced.iter().chain(&went[1].replaced).collect();
        for at in arrived {
            let was = trees[0].entry(at)?;
            let Some(was) = was.filter(is_file) else {
                continue;
            };
            if went.iter().any(|w| w.went.contains_key(at)) {
                continue;
            }
            // What both sides hold alike is the result, whatever the notes say.
            let version = |e: Option<Entry>| e.map(|e| (e.kind, e.id));
            if version(trees[1].entry(at)?) == version(trees[2].entry(at)?) {
                continue;
            }
            let on = went.each_ref().map(|w| w.replaced.contains(at));
            let clash = match on {
                [true, true] => None,
                [gone_on_ours, _] => {
                    let (removed_on, other) = if gone_on_ours {
                        (Side::Ours, 2)
                    } else {
                        (Side::Theirs, 1)
                    };
                    match trees[other].entry(at)? {
                        None => None,
                        Some(e) if (e.kind, e.id) == (was.kind, was.id) => continue,
                        Some(_) => Some(Clash::ChangedRemoved { removed_on }),
                    }
                }
            };
            match clash {
                // The start holds nothing there, so what the sides hold there is new.
                None => {
                    puts[0].insert(at.clone(), None);
                }
                Some(clash) => clash_at(&mut trees, &mut puts, &mut clashes, at, clash)?,
            }
        }
    }
    let mut roots = [start.root, ours.root, theirs.root];
    for (root, changes) in roots.iter_mut().zip(&puts) {
        if changes.is_empty() {
            continue;
        }
        let mut built = Vec::new();
        // A folder emptied only by these changes (moved files put in place, a conflict's path
        // cleared) is no version's own.
        let stored = Listings::stored(store, &made.listings);
        *root = overlay_with(&stored, *root, changes, true, &mut |listing| {
            let id = store_listing(store, packs, listing)?;
            built.push((id, listing.clone()));
            Ok(id)
        })?;
        made.listings.extend(built);
    }
    let [s, o, t] = roots;
    let time = made.deadline;
    let mut merged = merge_trees(store, packs, made, inner, s, o, t, rules, &moved, time)?;
    merged.unfollowed = [Side::Ours, Side::Theirs]
        .into_iter()
        .zip(&went)
        .filter(|(_, w)| w.too_many_left)
        .map(|(side, _)| side)
        .collect();
    merged.notes_unbelieved = went.iter().any(|w| w.notes_unbelieved);
    if !clashes.is_empty() {
        merged.unkept = true;
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

/// Names `clash` at `path`. What any version holds there, a new file a mover made included, is
/// part of this conflict, not one of its own; a new folder there goes too, as the combine stops.
fn clash_at(
    trees: &mut [Tree; 3],
    puts: &mut [BTreeMap<String, Option<Put>>; 3],
    clashes: &mut Vec<Conflict>,
    path: &str,
    clash: Clash,
) -> Result<(), StoreError> {
    for (tree, puts) in trees.iter_mut().zip(puts) {
        if tree.entry(path)?.is_some() {
            puts.insert(path.to_owned(), None);
        }
    }
    clashes.push(Conflict {
        path: path.to_owned(),
        clash,
        stands: None,
    });
    Ok(())
}

/// The first of the three versions past a save's limits, as the tree merge would name it: what
/// stopped following moves may have been one of them, which that walk can't tell.
fn past_limits(repo: &Repository, made: &Made, three: [&Version; 3]) -> Option<CombineError> {
    let store = repo.store();
    let read = |id: &Id| match made.listings.get(id) {
        Some(l) => Ok(l.clone()),
        None => store.listing(id),
    };
    three.into_iter().zip(tree::WHICH).find_map(|(v, which)| {
        match SaveLimits::default().add_all(v.root, 0, 0, read) {
            Ok(Err(detail)) => Some(CombineError::Past {
                names: Names::default(),
                which,
                detail,
            }),
            _ => None,
        }
    })
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

/// What one combine may take: inner combines, text they hold, time to compare each file, memory
/// to follow moves by contents, move notes to check, and the result's limits.
struct Bounds {
    max_inner: usize,
    max_held: u64,
    deadline: Duration,
    max_left_bytes: usize,
    max_notes: usize,
    limits: SaveLimits,
}

/// What a combine asks for.
struct Asked<'a> {
    target: &'a Target,
    named: &'a Named,
    message: Option<&'a str>,
    who: &'a Person,
    git_excludes: Option<&'a Path>,
    keep: Keep,
}

/// Combines `target` into the line the project is on: nothing to do, the line moved forward to
/// it, or a combined save, which the folder is then switched to as `goto` switches. One diary
/// step, which `undo` takes back. A combine that stops stores nothing, unless unsaved work is in
/// the way of the switch.
#[allow(clippy::too_many_arguments)]
pub fn combine(
    repo: &mut Repository,
    target: &Target,
    named: &Named,
    message: Option<&str>,
    who: &Person,
    git_excludes: Option<&Path>,
    keep: Keep,
    on_wait: &mut dyn FnMut(&str),
) -> Result<CombineOutcome, CombineError> {
    let asked = Asked {
        target,
        named,
        message,
        who,
        git_excludes,
        keep,
    };
    let bounds = Bounds {
        max_inner: MAX_INNER,
        max_held: MAX_HELD_TEXT,
        deadline: deadline(),
        max_left_bytes: MAX_LEFT_BYTES,
        max_notes: MAX_CHECKED_NOTES,
        limits: SaveLimits::default(),
    };
    combine_with(repo, &asked, on_wait, bounds)
}

fn combine_with(
    repo: &mut Repository,
    asked: &Asked,
    on_wait: &mut dyn FnMut(&str),
    bounds: Bounds,
) -> Result<CombineOutcome, CombineError> {
    locked(repo, "combine", on_wait, |repo, lock, earlier, _| {
        Ok(combine_locked(repo, lock, earlier, asked, bounds))
    })
    .map_err(CombineError::Switch)?
}

fn combine_locked(
    repo: &mut Repository,
    lock: &ChangeLock,
    earlier: &mut Earlier,
    asked: &Asked,
    bounds: Bounds,
) -> Result<CombineOutcome, CombineError> {
    let (head, entry) = repo.current()?;
    let Position::OnLine(line) = &entry.state.position else {
        return Err(CombineError::NotOnALine);
    };
    let line = line.clone();
    let names = Names {
        line: escape(&line, false),
        target: asked.named.shown.clone(),
    };
    let from = tip_of(&entry)?;
    let (_, with) = landing(repo, &entry.state, asked.target).map_err(CombineError::Switch)?;
    let outcome = |result, save, (applied, safety_copies, carried)| CombineOutcome {
        line: line.clone(),
        result,
        save,
        with,
        applied,
        safety_copies,
        carried,
    };
    if only_in(repo, with, &[from])? == 0 {
        return Ok(outcome(Combined::Nothing, from, (Applied::default(), 0, 0)));
    }
    let to = Landing {
        head,
        state: &entry.state,
        line: &line,
        from,
    };
    if only_in(repo, from, &[with])? == 0 {
        let mut done = to.switch(repo, lock, earlier, asked, with, vec![])?;
        done.0.warnings.extend(repo.read_rules().warnings());
        return Ok(outcome(Combined::Forward, with, done));
    }
    let made =
        make_save(repo, lock, asked, &line, from, with, bounds).map_err(|e| e.named(&names))?;
    let mut warnings: Vec<Warning> = made
        .timed_out
        .iter()
        .map(|path| {
            let message = format!(
                "Comparing the versions of {} took over 2 seconds, so Opalro compared them \
                 roughly; a closer look might have merged them.",
                escape(path, false)
            );
            Warning::new(warning::MERGE_TIMED_OUT, message)
        })
        .collect();
    warnings.extend(made.unfollowed.iter().map(|side| {
        let on = if *side == Side::Ours {
            &names.line
        } else {
            &names.target
        };
        let message = format!(
            "Too many files left their places on {on} for Opalro to match them by their \
             contents, so files that moved there without a note were not followed."
        );
        Warning::new(warning::MOVES_NOT_FOLLOWED, message)
    }));
    if made.starts_unfollowed {
        let message = "Too many files left their places between the two lines' common starting \
                       points for Opalro to match them by their contents, so files that moved \
                       there without a note were not followed.";
        warnings.push(Warning::new(warning::MOVES_NOT_FOLLOWED, message));
    }
    if made.notes_unbelieved {
        let message = "The saves being combined hold more move notes than Opalro checks in one \
                       combine (1,000,000), so past that many it followed files that moved by \
                       their contents alone.";
        warnings.push(Warning::new(warning::TOO_MANY_MOVE_NOTES, message));
    }
    let mut done = to.switch(repo, lock, earlier, asked, made.save, warnings)?;
    // The rules file the switch left in the folder is the one the next combine reads.
    done.0.warnings.extend(repo.read_rules().warnings());
    let result = Combined::Made {
        starts: made.starts,
        merged_files: made.merged_files,
        conflicts: made.conflicts,
        more: made.more,
        conflict_count: made.conflict_count,
    };
    Ok(outcome(result, made.save, done))
}

/// Where a combine's line is, before its switch.
struct Landing<'a> {
    /// The diary entry `head` names, and its state.
    head: Id,
    state: &'a State,
    line: &'a str,
    /// The line's newest save, which the folder is on.
    from: Id,
}

impl Landing<'_> {
    /// Points the line at `to` and switches the folder there as `goto` does, one diary step
    /// telling `warnings` first. Gives what the switch did, its safety copies, and what came along.
    fn switch(
        &self,
        repo: &mut Repository,
        lock: &ChangeLock,
        earlier: &mut Earlier,
        asked: &Asked,
        to: Id,
        warnings: Vec<Warning>,
    ) -> Result<(Applied, usize, usize), CombineError> {
        let (mut plan, drive) =
            plan_switch(repo, self.from, to, asked.git_excludes).map_err(|e| match e {
                SwitchError::Collisions { paths } => CombineError::Collisions { paths },
                e => CombineError::Switch(e),
            })?;
        plan.warnings.splice(0..0, warnings);
        let mut state = self.state.clone();
        state.lines.insert(self.line.to_owned(), to);
        let planned = Planned {
            head: self.head,
            before: self.state,
            state,
            from: self.from,
            to,
            plan,
            drive,
        };
        make_switch(repo, lock, planned, asked.who, "combine", earlier)
            .map_err(CombineError::Switch)
    }
}

/// A combined save, stored.
struct MadeSave {
    save: Id,
    starts: Vec<Id>,
    merged_files: Vec<String>,
    timed_out: Vec<String>,
    unfollowed: Vec<Side>,
    /// Whether moves weren't followed by contents while the starting points were combined.
    starts_unfollowed: bool,
    /// Whether the move notes were past what one combine checks.
    notes_unbelieved: bool,
    /// The conflicts it made, named and counted, and every one it holds.
    conflicts: Vec<Conflict>,
    more: u64,
    conflict_count: u64,
}

/// Merges `with` into `from` and stores the combined save, made on `line`. Everything is built in
/// one pack set that becomes part of the store only once the save is known to go ahead.
fn make_save(
    repo: &mut Repository,
    lock: &ChangeLock,
    asked: &Asked,
    line: &str,
    from: Id,
    with: Id,
    mut bounds: Bounds,
) -> Result<MadeSave, CombineError> {
    let (ours, theirs) = (version_of(repo, from)?, version_of(repo, with)?);
    let theirs_rules = read_saved(repo.store(), &repo.store().listing(&theirs.root)?)?;
    let rules = RuleMatcher::new(repo.rules(), &theirs_rules);
    let mut made = Made {
        max_held: bounds.max_held,
        deadline: bounds.deadline,
        max_left_bytes: bounds.max_left_bytes,
        notes_left: bounds.max_notes,
        ..Made::default()
    };
    let starts = match common_starts(repo, &[from], &[with])? {
        Starts::TooMany(_) => return Err(tangled()),
        Starts::Found(starts) if starts.is_empty() => {
            return Err(CombineError::Unrelated {
                names: Names::default(),
            });
        }
        Starts::Found(starts) => starts,
    };
    // Only now: a combine refused before it builds anything makes no temporary file.
    let mut packs = repo.new_packs()?;
    let mut inner = Inner {
        repo,
        packs: &mut packs,
        made: &mut made,
        rules: &rules,
        left: bounds.max_inner,
        unfollowed: false,
    };
    let start = inner.start_from(&starts)?;
    let starts_unfollowed = inner.unfollowed;
    let keeps = repo.keeps_conflicts();
    if asked.keep == Keep::Conflicts && keeps {
        made.keep = Some(sides(repo, asked, line, from, with, &starts)?);
    }
    let merged = merge_three(
        repo, &mut packs, &mut made, false, &start, &ours, &theirs, &rules,
    )?;
    let stopped = |conflicts, more, why| CombineError::Conflicts {
        names: Names::default(),
        conflicts,
        more,
        rules_not_whole: merged.rules_not_whole,
        why,
    };
    if made.keep.is_none() && !merged.conflicts.is_empty() {
        let why = match asked.keep {
            Keep::OnlyIfClean => NotKept::OnlyIfClean,
            Keep::Conflicts => NotKept::NoFeature,
        };
        return Err(stopped(merged.conflicts, merged.more, why));
    }
    if made.keep.is_some() && merged.rules_not_whole.is_some() {
        let (stops, more) = (merged.undecided, merged.undecided_more);
        return Err(stopped(stops, more, NotKept::RulesNotWhole));
    }
    if merged.unkept {
        // Clashes of moved files aren't kept as conflicts yet.
        return Err(stopped(merged.conflicts, merged.more, NotKept::OnlyIfClean));
    }
    let store = repo.store();
    let read = |id: &Id| match made.listings.get(id) {
        Some(l) => Ok(l.clone()),
        None => store.listing(id),
    };
    let measured = if keeps {
        // A conflict counts as what it stands as; the records this combine made aren't stored yet.
        let stand = |id: &Id| match made.records.get(id) {
            Some(r) => Ok(stands_as(r)),
            None => store
                .met_conflict(id)
                .map(|r| stands_as(&r))
                .map_err(|e| unreadable("in the combined save", id, e)),
        };
        bounds
            .limits
            .add_all_standing(merged.root, 0, 0, read, stand)?
    } else {
        bounds.limits.add_all(merged.root, 0, 0, read)?.map(|()| 0)
    };
    let conflict_count = match measured {
        Ok(count) => count,
        Err(detail) => return Err(CombineError::TooBig { detail }),
    };
    // Notes say only what is certain and never stop a combine.
    let mut move_notes = same_contents(
        store,
        &made.listings,
        ours.root,
        merged.root,
        made.max_left_bytes,
    )
    .ok()
    .flatten()
    .unwrap_or_default();
    let most = usize::try_from(MAX_MOVE_NOTES).unwrap_or(usize::MAX);
    first_that_fit(&mut move_notes, most, MAX_NOTED_BYTES);
    let message = match asked.message {
        Some(message) => message.to_owned(),
        None => format!("Combine {} into {line}", asked.named.raw),
    };
    let save = Save {
        root: merged.root,
        parents: vec![from, with],
        author: asked.who.clone(),
        saver: asked.who.clone(),
        message,
        change_id: ChangeId::random()
            .map_err(io::Error::other)
            .map_err(RepoError::from)?,
        move_notes,
        signature: vec![],
        extra: BTreeMap::from([(EXTRA_LINE.to_owned(), line.as_bytes().to_vec())]),
    };
    let encoded = save.encode();
    // Refuse a save that would not read back (an over-long message, say) before storing it.
    Save::decode(&encoded, false).map_err(StoreError::from)?;
    let id = packs
        .add_record(ObjectKind::Save, &encoded)
        .map_err(StoreError::from)?;
    drop(made);
    repo.store_packs(lock, packs)?;
    Ok(MadeSave {
        save: id,
        starts,
        merged_files: merged.merged_files,
        timed_out: merged.timed_out,
        unfollowed: merged.unfollowed,
        starts_unfollowed,
        // Once spent, the budget stays spent for every later merge, this last one included.
        notes_unbelieved: merged.notes_unbelieved,
        conflicts: merged.conflicts,
        more: merged.more,
        conflict_count,
    })
}

/// What every label of this combine is made from: the line, the target, their saves, and where
/// they parted (several, or a made-up starting point, is `Several`).
fn sides(
    repo: &Repository,
    asked: &Asked,
    line: &str,
    from: Id,
    with: Id,
    starts: &[Id],
) -> Result<Sides, CombineError> {
    let change_id = |save: &Id| -> Result<ChangeId, CombineError> {
        Ok(repo
            .store()
            .save(save, repo.signatures_allowed())?
            .change_id)
    };
    let parted = match starts {
        [one] => Parted::One(change_id(one)?),
        _ => Parted::Several,
    };
    Ok(Sides {
        ours: escape(line, false),
        ours_save: change_id(&from)?,
        theirs: asked.named.shown.clone(),
        theirs_save: change_id(&with)?,
        parted,
    })
}

/// A refusal met while combining starting points, in words about them: two names that clash
/// there are their own conflict, and a version past the limits there is under the starting point.
fn of_starts(e: CombineError) -> CombineError {
    match e {
        CombineError::NamesClash { names, .. } => CombineError::StartsConflict {
            names,
            rules_not_whole: None,
        },
        CombineError::Past { names, detail, .. } => CombineError::Past {
            names,
            which: Which::Start,
            detail,
        },
        CombineError::TooBig { detail } => CombineError::Past {
            names: Names::default(),
            which: Which::Start,
            detail,
        },
        e => e,
    }
}

fn tangled() -> CombineError {
    CombineError::Tangled {
        names: Names::default(),
    }
}

fn version_of(repo: &Repository, save: Id) -> Result<Version, CombineError> {
    let root = repo.store().save(&save, repo.signatures_allowed())?.root;
    Ok(Version {
        root,
        save: Some(save),
    })
}

/// What combining starting points with each other works with, and how many inner combines are
/// left to make.
struct Inner<'a> {
    repo: &'a Repository,
    packs: &'a mut PackSet,
    made: &'a mut Made,
    rules: &'a RuleMatcher,
    left: usize,
    /// Whether an inner combine followed no moves by contents on a side too many files left.
    unfollowed: bool,
}

impl Inner<'_> {
    /// The version to merge from, given the common starting points of the two sides, in ID
    /// order: one is itself; several are combined with each other first, two at a time, each pair
    /// from its own starting points. Every inner combine is counted, so this goes at most as
    /// many calls deep as there are inner combines left.
    fn start_from(&mut self, starts: &[Id]) -> Result<Version, CombineError> {
        let Some((first, rest)) = starts.split_first() else {
            // Starting points that share no save are combined from nothing.
            let empty = Listing::empty();
            let root = store_listing(self.repo.store(), self.packs, &empty)?;
            self.made.listings.insert(root, empty);
            return Ok(Version { root, save: None });
        };
        let mut acc = version_of(self.repo, *first)?;
        let mut tips = vec![*first];
        for &next in rest {
            self.left = self.left.checked_sub(1).ok_or_else(tangled)?;
            let under = match common_starts(self.repo, &tips, &[next])? {
                Starts::TooMany(_) => return Err(tangled()),
                Starts::Found(under) => under,
            };
            let start = self.start_from(&under)?;
            let theirs = version_of(self.repo, next)?;
            let merged = merge_three(
                self.repo, self.packs, self.made, true, &start, &acc, &theirs, self.rules,
            )
            .map_err(of_starts)?;
            self.unfollowed |= !merged.unfollowed.is_empty();
            if !merged.conflicts.is_empty() {
                return Err(CombineError::StartsConflict {
                    names: Names::default(),
                    rules_not_whole: merged.rules_not_whole,
                });
            }
            acc = Version {
                root: merged.root,
                save: None,
            };
            tips.push(next);
        }
        Ok(acc)
    }
}
