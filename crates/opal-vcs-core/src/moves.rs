//! Files that moved between two versions of the project: the same contents gone from one path and
//! present at exactly one new one.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::ancestry::only_in;
use crate::combine::{Made, Unknown, is_unknown};
use crate::conflict::Listings;
use crate::diff::{file_change, for_each_entry_change_in};
use crate::id::Id;
use crate::kept::EXACT;
use crate::listing::{Entry, EntryKind, Listing};
use crate::names::validate_path;
use crate::repo::{RepoError, Repository};
use crate::save::MoveNote;
use crate::scope::ancestors;
use crate::store::{Store, StoreError};
use crate::writeout::Tree;

/// At most this many bytes of paths in one save's notes, both paths of each note counted, so that
/// the notes never make a save's record too large to store.
pub(crate) const MAX_NOTED_BYTES: usize = 16 << 20;

#[cfg(test)]
thread_local! {
    /// How many files that arrived the comparisons on this thread kept.
    pub(crate) static ARRIVALS_KEPT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The path that files with one contents left (or arrived at), by contents: `None` once a
/// second one does.
type Places = HashMap<Id, Option<String>>;

fn seen(places: &mut Places, id: Id, path: &str) {
    places
        .entry(id)
        .and_modify(|p| *p = None)
        .or_insert_with(|| Some(path.to_owned()));
}

/// A file with contents worth pairing: a link is never a move, and every empty file is alike.
fn file(e: Option<&Entry>) -> Option<&Entry> {
    e.filter(|e| matches!(e.kind, EntryKind::File | EntryKind::Executable) && e.size > 0)
}

/// Files that left one path and arrived at another between `from_root` and `to_root` with the
/// same non-empty contents, no other file that left and none that arrived holding those
/// contents; sorted by where they went. A path that left holds nothing at `to_root`, and one
/// that arrived held nothing at `from_root`. Either side's listings may be in `unstored`. None
/// when holding the files that left, and those that arrived with their contents, would take more
/// than `most_left_bytes`.
pub(crate) fn same_contents(
    store: &Store,
    unstored: &HashMap<Id, Listing>,
    from_root: Id,
    to_root: Id,
    most_left_bytes: usize,
) -> Moves {
    moves_by(
        |each| for_each_entry_change_in(store, unstored, Some(from_root), to_root, each),
        most_left_bytes,
    )
}

/// Moves found by contents: none at all when there were too many files to hold.
pub(crate) type Moves = Result<Option<Vec<MoveNote>>, StoreError>;

/// At most this many bytes of memory are taken to hold the files that left, and those that
/// arrived with their contents, to pair them; past it, a save notes no moves and a combine
/// follows none by contents.
pub(crate) const MAX_LEFT_BYTES: usize = 64 << 20;

/// What holding one file that left or arrived takes beside its path, at its peak. While a map
/// grows, its old table and the new one twice as big are both alive: three buckets, each with a
/// control byte. An allocator rounds a short path up to about 32 bytes.
///
/// It is what `3 * (size_of::<(Id, Option<String>)>() + 1) + 32` gives on a 64-bit computer,
/// fixed since whether moves are noted depends on it, and notes are part of a save's fingerprint.
pub(crate) const HELD_PER_PLACE: usize = 203;

#[cfg(test)]
thread_local! {
    /// How many comparisons on this thread walked the folders a second time, for arrivals.
    pub(crate) static SECOND_WALKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many files differ between the two roots, counted as [`crate::diff::count_changes`]
/// counts them, and [`same_contents`]' moves. The folders that differ are walked once, and again
/// only if a file arrived and a file left with contents no other file that left has. The moves
/// have their own result, so a second walk that fails leaves the count. Where `passed_over` says
/// so from the entries on each side, a path and everything below it never left or arrived.
pub(crate) fn count_and_moves(
    store: &Store,
    unstored: &HashMap<Id, Listing>,
    from_root: Id,
    to_root: Id,
    most_left_bytes: usize,
    passed_over: &PassedOver,
) -> Result<(usize, Moves), StoreError> {
    let (mut changed, mut walks) = (0, 0);
    let moves = moves_by(
        |each| {
            walks += 1;
            let counting = walks == 1;
            let mut passed = Passed::new(passed_over);
            for_each_entry_change_in(
                store,
                unstored,
                Some(from_root),
                to_root,
                |path, old, new| {
                    changed += usize::from(counting && file_change(old, new).is_some());
                    if passed.at(path, old, new) {
                        return Ok(());
                    }
                    each(path, old, new)
                },
            )
        },
        most_left_bytes,
    );
    match moves {
        // Only a first walk that failed leaves no count.
        Err(why) if walks == 1 => Err(why),
        moves => Ok((changed, moves)),
    }
}

/// `same_contents`' moves between two sides that `walk` gives: each call of `walk` calls its
/// argument with every path whose entry differs and the entry on each side, the same paths each
/// time, in any order. None when holding the files would take more than `most_left_bytes`.
pub(crate) fn moves_by<E: From<StoreError>>(
    mut walk: impl FnMut(
        &mut dyn FnMut(&str, Option<&Entry>, Option<&Entry>) -> Result<(), E>,
    ) -> Result<(), E>,
    most_left_bytes: usize,
) -> Result<Option<Vec<MoveNote>>, E> {
    // The files that left first: arrivals are worth keeping only for the contents of one, and a
    // save that adds a great deal often has none.
    let (mut left, mut left_bytes, mut arrivals) = (Places::new(), 0usize, false);
    walk(&mut |path, old, new| {
        arrivals |= matches!((old, file(new)), (None, Some(_)));
        if let (Some(f), None) = (file(old), new)
            && left_bytes <= most_left_bytes
        {
            left_bytes = left_bytes.saturating_add(HELD_PER_PLACE + path.len());
            if left_bytes > most_left_bytes {
                left = Places::new();
            } else {
                seen(&mut left, f.id, path);
            }
        }
        Ok(())
    })?;
    if left_bytes > most_left_bytes {
        return Ok(None);
    }
    left.retain(|_, from| from.is_some());
    if left.is_empty() || !arrivals {
        return Ok(Some(vec![]));
    }
    paired(walk, left, most_left_bytes.saturating_sub(left_bytes))
}

/// Whether a comparison passes over a path, from the entries on each side.
pub(crate) type PassedOver = dyn Fn(Option<&Entry>, Option<&Entry>) -> bool;

/// The paths a comparison passes over, as `over` names them, and what is below them, each with a
/// `/` after it, while paths below may still come.
struct Passed<'p> {
    over: &'p PassedOver,
    below: Vec<String>,
}

impl<'p> Passed<'p> {
    fn new(over: &'p PassedOver) -> Passed<'p> {
        Passed {
            over,
            below: Vec::new(),
        }
    }

    /// Whether `path`, coming in path order with `old` and `new`, is passed over.
    fn at(&mut self, path: &str, old: Option<&Entry>, new: Option<&Entry>) -> bool {
        self.below.retain(|f| path.starts_with(f.as_str()));
        if !self.below.is_empty() {
            return true;
        }
        let over = (self.over)(old, new);
        if over {
            self.below.push(format!("{path}/"));
        }
        over
    }
}

/// The moves of the files in `left` (contents to the one path that left with them), from a
/// second walk for the files that arrived with those contents; none when holding those takes
/// more than `room` bytes.
fn paired<E>(
    mut walk: impl FnMut(
        &mut dyn FnMut(&str, Option<&Entry>, Option<&Entry>) -> Result<(), E>,
    ) -> Result<(), E>,
    mut left: Places,
    room: usize,
) -> Result<Option<Vec<MoveNote>>, E> {
    #[cfg(test)]
    SECOND_WALKS.set(SECOND_WALKS.get() + 1);
    let (mut arrived, mut held) = (Places::new(), 0usize);
    walk(&mut |path, old, new| {
        if let (None, Some(f)) = (old, file(new))
            && left.contains_key(&f.id)
            && held <= room
        {
            held = held.saturating_add(HELD_PER_PLACE + path.len());
            if held > room {
                arrived = Places::new();
            } else {
                #[cfg(test)]
                ARRIVALS_KEPT.set(ARRIVALS_KEPT.get() + 1);
                seen(&mut arrived, f.id, path);
            }
        }
        Ok(())
    })?;
    if held > room {
        return Ok(None);
    }
    let mut notes: Vec<MoveNote> = arrived
        .into_iter()
        .filter_map(|(id, to)| {
            Some(MoveNote {
                from: left.remove(&id)??,
                to: to?,
            })
        })
        // A path a save can't hold can't be noted either.
        .filter(|n| validate_path(&n.from).is_ok() && validate_path(&n.to).is_ok())
        .collect();
    notes.sort_unstable_by(|a, b| a.to.cmp(&b.to));
    Ok(Some(notes))
}

/// One version a merge reads: its root listing, and its save, `None` when it is made up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Version {
    pub root: Id,
    pub save: Option<Id>,
}

/// Where the files of a start went on one side.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Followed {
    /// Start path to side path, each move checked.
    pub went: BTreeMap<String, String>,
    /// Paths where the side's notes show another file arriving, and a file is still there at
    /// the side's end, each with where that file started: the start's file at such a path is
    /// removed on that side.
    pub replaced: BTreeMap<String, String>,
    /// Too many files left on the side to match them by their contents, so only notes count.
    pub too_many_left: bool,
    /// The combine's notes were too many to check, so no note counts.
    pub notes_unbelieved: bool,
}

/// The most move notes one combine checks, true or not, on both sides of every merge it makes.
pub(crate) const MAX_CHECKED_NOTES: usize = 1_000_000;

#[cfg(test)]
thread_local! {
    /// How many move notes the combines on this thread checked.
    pub(crate) static NOTES_CHECKED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Where the files of `start` went on `side`. Listings in `made` are read before the store's.
pub(crate) fn followed(
    repo: &Repository,
    made: &mut Made,
    start: &Version,
    side: &Version,
) -> Result<Followed, RepoError> {
    let store = repo.store();
    let noted = match (start.save, side.save) {
        (Some(from), Some(tip)) if !made.notes_spent => noted(repo, made, from, start.root, tip)?,
        _ => BTreeMap::new(),
    };
    let mut unknown = made.unknown_in(start);
    if side.save.is_none() {
        // A made-up side can't tell what it holds there either.
        unknown.extend(made.side_unknown.iter().cloned());
    }
    let stored = Listings::stored(store, &made.listings);
    let mut at_start = Tree::new(&stored, start.root, EXACT);
    let mut on_side = Tree::new(&stored, side.root, EXACT);
    let mut out = Followed {
        notes_unbelieved: made.notes_spent,
        ..Followed::default()
    };
    // A side whose notes moved a file has moved it, whatever it holds where the file was.
    for (from, to) in noted {
        // The place counts as replaced only while a file is still there; edits in place keep
        // it the file that arrived.
        if on_side.entry(&to)?.is_some_and(|e| is_file(&e)) {
            out.replaced.insert(to.clone(), from.clone());
        }
        if fits(&mut at_start, &mut on_side, &unknown, &from, &to)? {
            out.went.insert(from, to);
        }
    }
    let taken: HashSet<String> = out.went.values().cloned().collect();
    let left = made.max_left_bytes;
    // Only a path that emptied is matched by contents; a file replaced in place was edited.
    let Some(by_contents) = same_contents(store, &made.listings, start.root, side.root, left)?
    else {
        out.too_many_left = true;
        return Ok(out);
    };
    for n in by_contents {
        if !out.went.contains_key(&n.from)
            && !taken.contains(&n.to)
            && fits(&mut at_start, &mut on_side, &unknown, &n.from, &n.to)?
            && on_side.entry(&n.from)?.is_none()
        {
            out.went.insert(n.from, n.to);
        }
    }
    Ok(out)
}

/// The moves the notes of `tip`'s saves tell, back along first parents to `start` (whose root
/// is `start_root`) and taken one after another; none when that path doesn't reach it. A note
/// counts only when it is true of its own save. Each save's notes are counted against what
/// `made` has left to check before any is; none at all once a save's are more than that.
fn noted(
    repo: &Repository,
    made: &mut Made,
    start: Id,
    start_root: Id,
    tip: Id,
) -> Result<BTreeMap<String, String>, RepoError> {
    let none = Ok(BTreeMap::new());
    if tip == start {
        return none;
    }
    // Every save on that path before the start is one the start doesn't have.
    let most = only_in(repo, tip, &[start])?;
    let read = |id: Id| repo.store().save(&id, repo.signatures_allowed());
    let unstored = HashMap::new();
    let stored = Listings::stored(repo.store(), &unstored);
    // Where each file of the tip was, as the walk goes back: there, to its path at the tip.
    let mut back: HashMap<String, String> = HashMap::new();
    // Paths at the tip whose file arrived there later than the save the walk is at.
    let mut arrived: HashSet<String> = HashSet::new();
    let mut save = read(tip)?;
    let mut reads = 1u64;
    // Each save is read once; its tree and its parent's keep only the listings its notes need.
    let mut after = Tree::new(&stored, save.root, EXACT);
    loop {
        let Some(&parent) = save.parents.first() else {
            return none;
        };
        if parent != start && reads > most {
            return none;
        }
        // A chain followed half-way would be wrong, so a save past the budget spends it all,
        // before anything more is read.
        let Some(left) = made.notes_left.checked_sub(save.move_notes.len()) else {
            made.notes_left = 0;
            made.notes_spent = true;
            return none;
        };
        made.notes_left = left;
        let parent_save = if parent == start {
            None
        } else {
            reads += 1;
            Some(read(parent)?)
        };
        let parent_root = parent_save.as_ref().map_or(start_root, |p| p.root);
        let mut before = Tree::new(&stored, parent_root, EXACT);
        // A save's notes all hold between it and its first parent, so they are taken together.
        // A file removed and made again in a later save is taken for the same file, as an edit.
        let mut true_notes = Vec::new();
        for n in std::mem::take(&mut save.move_notes) {
            #[cfg(test)]
            NOTES_CHECKED.set(NOTES_CHECKED.get() + 1);
            if validate_path(&n.from).is_ok()
                && validate_path(&n.to).is_ok()
                && true_of(&mut before, &mut after, &n)?
            {
                true_notes.push(n);
            }
        }
        // One file gone to two places, alike: neither is certain.
        let mut froms: HashMap<String, usize> = HashMap::new();
        for n in &true_notes {
            *froms.entry(n.from.clone()).or_default() += 1;
        }
        let mut moved = Vec::new();
        for n in true_notes {
            if froms.get(&n.from) != Some(&1) {
                continue;
            }
            let tip_path = match back.remove(&n.to) {
                Some(p) => p,
                None if arrived.insert(n.to.clone()) => n.to,
                None => continue,
            };
            moved.push((n.from, tip_path));
        }
        for (from, tip_path) in moved {
            // Whatever the tip holds there came later.
            arrived.insert(from.clone());
            back.insert(from, tip_path);
        }
        match parent_save {
            None => break,
            Some(p) => (save, after) = (p, before),
        }
    }
    Ok(back.into_iter().filter(|(from, to)| from != to).collect())
}

/// Whether a note is true of the save it is in: its file left `from` for `to` with the same
/// non-empty contents, as a save's own notes are made.
pub(crate) fn true_of(
    before: &mut Tree,
    after: &mut Tree,
    n: &MoveNote,
) -> Result<bool, StoreError> {
    // An untrue note is mostly one whose file was never there: that is answered first.
    let Some(left) = file(before.entry(&n.from)?.as_ref()).map(|e| e.id) else {
        return Ok(false);
    };
    Ok(after.entry(&n.from)?.is_none()
        && before.entry(&n.to)?.is_none()
        && file(after.entry(&n.to)?.as_ref()).is_some_and(|e| e.id == left))
}

/// Whether a move from `from` to `to` counts: the start holds a file at `from` and has room at
/// `to`, neither of them a place `unknown` can't tell of, and the side holds a file at `to`.
fn fits(
    start: &mut Tree,
    side: &mut Tree,
    unknown: &Unknown,
    from: &str,
    to: &str,
) -> Result<bool, StoreError> {
    let is_file = |e: Option<Entry>| e.is_some_and(|e| is_file(&e));
    let known = !is_unknown(unknown, from) && !is_unknown(unknown, to);
    Ok(known && is_file(start.entry(from)?) && room_at(start, to)? && is_file(side.entry(to)?))
}

pub(crate) fn is_file(e: &Entry) -> bool {
    matches!(e.kind, EntryKind::File | EntryKind::Executable)
}

/// Whether `tree` holds nothing at `path` and nothing but folders above it.
pub(crate) fn room_at(tree: &mut Tree, path: &str) -> Result<bool, StoreError> {
    for up in ancestors(path) {
        match tree.entry(up)? {
            None => return Ok(true),
            Some(e) if e.kind == EntryKind::Folder => {}
            Some(_) => return Ok(false),
        }
    }
    Ok(tree.entry(path)?.is_none())
}

/// The first of `notes`, sorted by where the file went, that a save keeps: at most `most`, and at
/// most `most_bytes` of paths in all.
pub(crate) fn first_that_fit(notes: &mut Vec<MoveNote>, most: usize, most_bytes: usize) {
    let mut bytes = 0usize;
    let fit = notes
        .iter()
        .take(most)
        .take_while(|n| {
            bytes = bytes.saturating_add(n.from.len() + n.to.len());
            bytes <= most_bytes
        })
        .count();
    notes.truncate(fit);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::id::hash_content;
    use crate::listing::{Entry, EntryKind, Listing};
    use proptest::prelude::*;
    use std::collections::{BTreeMap, BTreeSet, HashMap};

    /// Listings for `files` (path, kind, bytes), kept in `unstored`; returns the root's ID.
    fn tree(unstored: &mut HashMap<Id, Listing>, files: &[(&str, EntryKind, &[u8])]) -> Id {
        let mut here: BTreeMap<String, Vec<(String, EntryKind, Vec<u8>)>> = BTreeMap::new();
        let mut entries = Vec::new();
        for (path, kind, bytes) in files {
            match path.split_once('/') {
                Some((top, rest)) => here.entry(top.to_owned()).or_default().push((
                    rest.to_owned(),
                    *kind,
                    bytes.to_vec(),
                )),
                None if *kind == EntryKind::Folder => {
                    here.entry((*path).to_owned()).or_default();
                }
                None => entries.push(Entry {
                    name: (*path).to_owned(),
                    kind: *kind,
                    id: hash_content(bytes),
                    size: bytes.len() as u64,
                }),
            }
        }
        for (name, inner) in here {
            let inner: Vec<(&str, EntryKind, &[u8])> = inner
                .iter()
                .map(|(p, k, b)| (p.as_str(), *k, b.as_slice()))
                .collect();
            entries.push(Entry {
                name,
                kind: EntryKind::Folder,
                id: tree(unstored, &inner),
                size: 0,
            });
        }
        let listing = Listing::new(entries).unwrap();
        let id = listing.id();
        unstored.insert(id, listing);
        id
    }

    use EntryKind::{Executable, File, Folder, Symlink};

    /// The moves between `from` and `to`, as (from, to) pairs.
    fn moves(
        from: &[(&str, EntryKind, &[u8])],
        to: &[(&str, EntryKind, &[u8])],
    ) -> Vec<(String, String)> {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let mut unstored = HashMap::new();
        let (a, b) = (tree(&mut unstored, from), tree(&mut unstored, to));
        same_contents(&store, &unstored, a, b, MAX_LEFT_BYTES)
            .unwrap()
            .unwrap()
            .into_iter()
            .map(|n| (n.from, n.to))
            .collect()
    }

    fn pair(from: &str, to: &str) -> (String, String) {
        (from.to_owned(), to.to_owned())
    }

    #[test]
    fn a_file_moved_alone_is_paired() {
        let got = moves(
            &[("a.txt", File, b"hello"), ("b.txt", File, b"stay")],
            &[
                ("Characters/a.txt", File, b"hello"),
                ("b.txt", File, b"stay"),
            ],
        );
        assert_eq!(got, [pair("a.txt", "Characters/a.txt")]);
        // An executable file is a file too.
        let got = moves(
            &[("run", Executable, b"go")],
            &[("bin/run", Executable, b"go")],
        );
        assert_eq!(got, [pair("run", "bin/run")]);
    }

    #[test]
    fn a_move_that_flips_the_executable_mark_is_a_move() {
        // The mark isn't contents.
        let got = moves(&[("run", File, b"go")], &[("bin/run", Executable, b"go")]);
        assert_eq!(got, [pair("run", "bin/run")]);
        let got = moves(&[("run", Executable, b"go")], &[("run.sh", File, b"go")]);
        assert_eq!(got, [pair("run", "run.sh")]);
    }

    #[test]
    fn a_comparison_where_nothing_left_keeps_no_arrivals() {
        ARRIVALS_KEPT.set(0);
        let added: Vec<(String, Vec<u8>)> = (0..50)
            .map(|i| (format!("new/{i}"), format!("{i}").into_bytes()))
            .collect();
        let mut to: Vec<(&str, EntryKind, &[u8])> = vec![("a", File, b"edited")];
        to.extend(added.iter().map(|(p, b)| (p.as_str(), File, b.as_slice())));
        let got = moves(&[("a", File, b"a")], &to);
        assert!(got.is_empty());
        assert_eq!(ARRIVALS_KEPT.get(), 0);
        // Nor any but those with the contents of a file that left.
        let mut to = to.clone();
        to.retain(|(p, ..)| *p != "a");
        to.push(("b", File, b"edited"));
        let got = moves(&[("a", File, b"edited")], &to);
        assert_eq!(got, [pair("a", "b")]);
        assert_eq!(ARRIVALS_KEPT.get(), 1);
    }

    /// `count_and_moves` between `from` and `to`, with at most `most_left_bytes` of paths left.
    fn counted(
        from: &[(&str, EntryKind, &[u8])],
        to: &[(&str, EntryKind, &[u8])],
        most_left_bytes: usize,
    ) -> (usize, Vec<(String, String)>) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let mut unstored = HashMap::new();
        let (a, b) = (tree(&mut unstored, from), tree(&mut unstored, to));
        let (count, notes) =
            count_and_moves(&store, &unstored, a, b, most_left_bytes, &|_, _| false).unwrap();
        let notes = notes.unwrap().unwrap_or_default();
        let notes = notes.into_iter().map(|n| (n.from, n.to)).collect();
        (count, notes)
    }

    #[test]
    fn past_what_the_files_that_left_take_nothing_is_paired_and_all_is_counted() {
        let from: &[(&str, EntryKind, &[u8])] = &[
            ("gone1", File, b"1"),
            ("gone2", File, b"2"),
            ("moved", File, b"m"),
        ];
        let to: &[(&str, EntryKind, &[u8])] = &[("there", File, b"m")];
        // Three files with 15 bytes of paths left, and one arrived at a path of 5 bytes.
        let all = 4 * HELD_PER_PLACE + 20;
        assert_eq!(counted(from, to, all), (4, vec![pair("moved", "there")]));
        assert_eq!(counted(from, to, all - 1), (4, vec![]));
    }

    #[test]
    fn many_short_paths_that_left_are_bounded_by_their_count() {
        let names: Vec<String> = (0..9).map(|i| i.to_string()).collect();
        let mut from: Vec<(&str, EntryKind, &[u8])> = names
            .iter()
            .map(|n| (n.as_str(), File, n.as_bytes()))
            .collect();
        from.push(("m", File, b"moved"));
        let to: &[(&str, EntryKind, &[u8])] = &[("x", File, b"moved")];
        // Ten paths of one byte each left and one arrived: 11 bytes, but eleven entries held.
        let all = 11 * (HELD_PER_PLACE + 1);
        assert_eq!(counted(&from, to, all), (11, vec![pair("m", "x")]));
        assert_eq!(counted(&from, to, all - 1), (11, vec![]));
        assert_eq!(counted(&from, to, 1_000), (11, vec![]));
    }

    #[test]
    fn files_that_arrived_at_long_paths_are_bounded_too() {
        // Two files left from one-byte paths and arrived ten folders of 100 bytes down.
        let deep = vec!["d".repeat(100); 10].join("/");
        let (to_a, to_b) = (format!("{deep}/a"), format!("{deep}/b"));
        let from: &[(&str, EntryKind, &[u8])] = &[("a", File, b"aa"), ("b", File, b"bb")];
        let to: &[(&str, EntryKind, &[u8])] = &[(&to_a, File, b"aa"), (&to_b, File, b"bb")];
        let left = 2 * (HELD_PER_PLACE + 1);
        let all = left + 2 * (HELD_PER_PLACE + to_a.len());
        let both = vec![pair("a", &to_a), pair("b", &to_b)];
        assert_eq!(counted(from, to, all), (4, both));
        assert_eq!(counted(from, to, all - 1), (4, vec![]));
        // Far more than the files that left take: the arrivals alone are past it.
        assert_eq!(
            counted(from, to, left + HELD_PER_PLACE + 1_000),
            (4, vec![])
        );
    }

    #[test]
    fn a_comparison_where_nothing_arrived_walks_once() {
        SECOND_WALKS.set(0);
        let from: &[(&str, EntryKind, &[u8])] = &[("a", File, b"a"), ("b", File, b"b")];
        assert_eq!(counted(from, &[("a", File, b"a")], usize::MAX), (1, vec![]));
        // An edit in place isn't an arrival either.
        assert_eq!(counted(from, &[("a", File, b"A")], usize::MAX), (2, vec![]));
        assert_eq!(SECOND_WALKS.get(), 0);
        let got = counted(from, &[("a", File, b"a"), ("c", File, b"b")], usize::MAX);
        assert_eq!(got, (2, vec![pair("b", "c")]));
        assert_eq!(SECOND_WALKS.get(), 1);
    }

    #[test]
    fn the_count_is_the_one_count_changes_gives() {
        type Files<'a> = &'a [(&'a str, EntryKind, &'a [u8])];
        let cases: [(Files, Files); 3] = [
            (
                &[("a", File, b"a"), ("b", File, b"b"), ("l", Symlink, b"t")],
                &[
                    ("c/a", File, b"a"),
                    ("b", Executable, b"b"),
                    ("l", Symlink, b"u"),
                ],
            ),
            (
                &[("a", File, b"a")],
                &[("a/b", File, b"a"), ("e", Folder, b"")],
            ),
            (
                &[("x/y/z", File, b"z"), ("e", File, b"")],
                &[("x", File, b"z")],
            ),
        ];
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        for (from, to) in cases {
            let mut unstored = HashMap::new();
            let (a, b) = (tree(&mut unstored, from), tree(&mut unstored, to));
            let mut want = 0;
            crate::diff::for_each_change_in(&store, &unstored, Some(a), b, |_| {
                want += 1;
                Ok::<(), StoreError>(())
            })
            .unwrap();
            let (got, _) =
                count_and_moves(&store, &unstored, a, b, usize::MAX, &|_, _| false).unwrap();
            assert_eq!(got, want, "{from:?} -> {to:?}");
        }
    }

    #[test]
    fn a_save_keeps_the_first_notes_that_fit_16_mib_of_paths() {
        use crate::save::{MAX_MOVE_NOTES, Person, Save};
        // 2,100 notes of two 4,000-byte paths is past 16 MiB: the first 2,097 fit.
        let mut notes: Vec<MoveNote> = (0..2_100)
            .map(|i| MoveNote {
                from: format!("{i:05}{}", "a".repeat(3_995)),
                to: format!("{i:05}{}", "b".repeat(3_995)),
            })
            .collect();
        let all = notes.clone();
        first_that_fit(
            &mut notes,
            usize::try_from(MAX_MOVE_NOTES).unwrap(),
            MAX_NOTED_BYTES,
        );
        assert_eq!(notes.len(), MAX_NOTED_BYTES / 8_000);
        assert_eq!(notes[..], all[..notes.len()]);
        // And the save that holds them is written and read back.
        let who = Person {
            name: "A".into(),
            email: "a@b".into(),
            time_ms: 0,
            tz_offset_min: 0,
        };
        let save = Save {
            root: Id([0; 32]),
            parents: vec![],
            author: who.clone(),
            saver: who,
            message: "m".into(),
            change_id: crate::id::ChangeId([0; 16]),
            move_notes: notes.clone(),
            signature: vec![],
            extra: BTreeMap::new(),
        };
        assert_eq!(
            Save::decode(&save.encode(), false).unwrap().move_notes,
            notes
        );
        // The count bounds them too.
        let mut notes = all;
        first_that_fit(&mut notes, 3, MAX_NOTED_BYTES);
        assert_eq!(notes.len(), 3);
    }

    #[test]
    fn a_file_moved_and_changed_is_not() {
        let got = moves(&[("a.txt", File, b"hello")], &[("b.txt", File, b"hello!")]);
        assert!(got.is_empty());
    }

    #[test]
    fn two_identical_files_swapping_pair_nothing() {
        // Two files with the same contents both leave and two arrive: which went where is a guess.
        let got = moves(
            &[("a", File, b"same"), ("b", File, b"same")],
            &[("c", File, b"same"), ("d", File, b"same")],
        );
        assert!(got.is_empty());
        // One left, and the same contents arrived at two places: a copy, not a move.
        let got = moves(
            &[("a", File, b"same")],
            &[("c", File, b"same"), ("d", File, b"same")],
        );
        assert!(got.is_empty());
    }

    #[test]
    fn a_copy_to_two_places_pairs_nothing() {
        // The original stays, so nothing left.
        let got = moves(
            &[("a", File, b"same")],
            &[
                ("a", File, b"same"),
                ("b", File, b"same"),
                ("c", File, b"same"),
            ],
        );
        assert!(got.is_empty());
        // Two left with those contents and one arrived.
        let got = moves(
            &[("a", File, b"same"), ("b", File, b"same")],
            &[("c", File, b"same")],
        );
        assert!(got.is_empty());
    }

    #[test]
    fn an_empty_file_moved_pairs_nothing() {
        let got = moves(&[("a", File, b"")], &[("b", File, b"")]);
        assert!(got.is_empty());
    }

    #[test]
    fn a_moved_folder_pairs_each_file() {
        let got = moves(
            &[
                ("art/z.png", File, b"zz"),
                ("art/a.png", File, b"aa"),
                ("art/sub/m.png", File, b"mm"),
            ],
            &[
                ("Assets/art/z.png", File, b"zz"),
                ("Assets/art/a.png", File, b"aa"),
                ("Assets/art/sub/m.png", File, b"mm"),
            ],
        );
        assert_eq!(
            got,
            [
                pair("art/a.png", "Assets/art/a.png"),
                pair("art/sub/m.png", "Assets/art/sub/m.png"),
                pair("art/z.png", "Assets/art/z.png"),
            ]
        );
    }

    #[test]
    fn a_file_replaced_by_a_folder_is_not_a_place_left() {
        // `a` still holds something, a folder, so its file didn't leave it.
        let got = moves(&[("a", File, b"hello")], &[("a/b", File, b"hello")]);
        assert!(got.is_empty());
        // Nor is a place that held a folder one a file arrived at.
        let got = moves(&[("a/b", File, b"hello")], &[("a", File, b"hello")]);
        assert!(got.is_empty());
        // An empty folder in its place still holds something.
        let got = moves(
            &[("a", File, b"hello")],
            &[("a", Folder, b""), ("b", File, b"hello")],
        );
        assert!(got.is_empty());
    }

    #[test]
    fn a_link_is_not_moved() {
        let got = moves(&[("l", Symlink, b"target")], &[("m", Symlink, b"target")]);
        assert!(got.is_empty());
        // Nor does a link with the same bytes as a file make the file's move unsure.
        let got = moves(
            &[("a", File, b"target")],
            &[("b", File, b"target"), ("l", Symlink, b"target")],
        );
        assert_eq!(got, [pair("a", "b")]);
    }

    /// One side of a random pair: few paths, kinds and contents, so that contents often leave or
    /// arrive twice and links and empty files often move.
    fn side() -> impl Strategy<Value = Vec<(&'static str, EntryKind, &'static [u8])>> {
        const PATHS: [&str; 7] = ["a", "b", "c", "a/b", "d/a", "d/e", "f/g/a"];
        const KINDS: [EntryKind; 6] = [File, File, File, Executable, Symlink, Folder];
        const BYTES: [&[u8]; 3] = [b"", b"1", b"2"];
        let entry = proptest::option::of((0..KINDS.len(), 0..BYTES.len()));
        proptest::collection::vec(entry, PATHS.len()).prop_map(|drawn| {
            let mut side: Vec<(&str, EntryKind, &[u8])> = Vec::new();
            for (path, entry) in PATHS.into_iter().zip(drawn) {
                // A path can't be a file and a folder at once.
                if let Some((k, b)) = entry
                    && !side
                        .iter()
                        .any(|(q, ..)| path.starts_with(&format!("{q}/")))
                {
                    side.push((path, KINDS[k], BYTES[b]));
                }
            }
            side
        })
    }

    #[test]
    fn moves_by_a_walk_in_another_order_gives_the_same_notes() {
        use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let met: std::cell::RefCell<BTreeMap<&str, usize>> = Default::default();
        let config = Config {
            cases: 1024,
            failure_persistence: None,
            ..Config::default()
        };
        let mut runner =
            TestRunner::new_with_rng(config, TestRng::deterministic_rng(RngAlgorithm::ChaCha));
        runner
            .run(&(side(), side()), |(from, to)| {
                let mut unstored = HashMap::new();
                let (a, b) = (tree(&mut unstored, &from), tree(&mut unstored, &to));
                let mut changes = Vec::new();
                for_each_entry_change_in(&store, &unstored, Some(a), b, |path, old, new| {
                    changes.push((path.to_owned(), old.cloned(), new.cloned()));
                    Ok::<(), StoreError>(())
                })
                .unwrap();
                let reversed = |most| {
                    let walk = |each: &mut dyn FnMut(&str, Option<&Entry>, Option<&Entry>) -> _| {
                        for (path, old, new) in changes.iter().rev() {
                            each(path, old.as_ref(), new.as_ref())?;
                        }
                        Ok::<(), StoreError>(())
                    };
                    moves_by(walk, most).unwrap()
                };
                let want = same_contents(&store, &unstored, a, b, MAX_LEFT_BYTES).unwrap();
                prop_assert_eq!(&reversed(MAX_LEFT_BYTES), &want);
                // At a bound some cases pass, the answer doesn't depend on the order either.
                let small = 2 * HELD_PER_PLACE + 4;
                let want_small = same_contents(&store, &unstored, a, b, small).unwrap();
                prop_assert_eq!(&reversed(small), &want_small);
                // What each case met, from the paths that left and arrived.
                let notes = want.unwrap_or_default();
                let (mut left, mut arrived) = (Vec::new(), Vec::new());
                for (path, old, new) in &changes {
                    match (old, new) {
                        (Some(e), None) if e.kind != Folder => left.push((path, e)),
                        (None, Some(e)) if e.kind != Folder => arrived.push((path, e)),
                        _ => {}
                    }
                }
                let alike = |a: &Entry, b: &Entry| {
                    a.id == b.id && (a.kind == Symlink) == (b.kind == Symlink)
                };
                let mut kinds = BTreeSet::new();
                if !notes.is_empty() {
                    kinds.insert("a note");
                }
                if want_small.is_none() {
                    kinds.insert("past a small bound");
                }
                for (path, e) in &left {
                    let went = left.iter().filter(|(_, l)| alike(l, e)).count();
                    let came = arrived.iter().filter(|(_, a)| alike(a, e)).count();
                    let kind = match (e.kind == Symlink || e.size == 0, went, came) {
                        (_, _, 0) => continue,
                        // Contents that left once and arrived once moved.
                        (false, 1, 1) => {
                            prop_assert!(notes.iter().any(|n| n.from == **path), "{path}");
                            continue;
                        }
                        (true, ..) => "a link or empty file passed over",
                        (false, 1, _) => "contents that arrived twice",
                        (false, ..) => "contents that left twice",
                    };
                    prop_assert!(!notes.iter().any(|n| n.from == **path), "{kind}: {path}");
                    kinds.insert(kind);
                }
                for kind in kinds {
                    *met.borrow_mut().entry(kind).or_default() += 1;
                }
                Ok(())
            })
            .unwrap();
        let met = met.into_inner();
        for kind in [
            "a note",
            "contents that left twice",
            "contents that arrived twice",
            "a link or empty file passed over",
            "past a small bound",
        ] {
            assert!(met.get(kind).is_some_and(|&n| n >= 3), "{met:?}");
        }
    }

    /// A walk that fails gives its error back, the first time it is asked or the second, when
    /// the pairing looks for arrivals.
    #[test]
    fn a_walk_that_fails_gives_its_error_back() {
        #[derive(Debug, PartialEq)]
        struct Failed;
        impl From<StoreError> for Failed {
            fn from(_: StoreError) -> Failed {
                Failed
            }
        }
        let file = |name: &str| Entry {
            name: name.into(),
            kind: File,
            id: hash_content(b"1"),
            size: 1,
        };
        // A move, so that the pairing walks twice.
        let changes = [("a", Some(file("a")), None), ("b", None, Some(file("b")))];
        for failing in [1, 2] {
            let mut calls = 0;
            let got = moves_by(
                |each| {
                    calls += 1;
                    for (path, old, new) in &changes {
                        each(path, old.as_ref(), new.as_ref())?;
                    }
                    if calls == failing {
                        Err(Failed)
                    } else {
                        Ok(())
                    }
                },
                MAX_LEFT_BYTES,
            );
            assert_eq!(got, Err(Failed), "failing at call {failing}");
            assert_eq!(calls, failing);
        }
    }
}
