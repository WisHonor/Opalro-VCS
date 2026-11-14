//! Files that moved between two versions of the project: the same contents gone from one path and
//! present at exactly one new one.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::ancestry::only_in;
use crate::combine::Made;
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
/// that arrived held nothing at `from_root`. Either side's listings may be in `unstored`.
pub(crate) fn same_contents(
    store: &Store,
    unstored: &HashMap<Id, Listing>,
    from_root: Id,
    to_root: Id,
) -> Result<Vec<MoveNote>, StoreError> {
    count_and_moves(store, unstored, from_root, to_root, usize::MAX)?.1
}

/// At most this many bytes of paths of files that left a save are held to pair them; past it, a
/// save notes no moves.
pub(crate) const MAX_LEFT_BYTES: usize = 64 << 20;

#[cfg(test)]
thread_local! {
    /// How many comparisons on this thread walked the folders a second time, for arrivals.
    pub(crate) static SECOND_WALKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many files differ between the two roots, counted as [`crate::diff::count_changes`]
/// counts them, and [`same_contents`]' moves: none when the paths of the files that left come
/// to more than `most_left_bytes`. The folders that differ are walked once, and again only if a
/// file arrived and a file left with contents no other file that left has. The moves have their
/// own result, so a second walk that fails leaves the count.
pub(crate) fn count_and_moves(
    store: &Store,
    unstored: &HashMap<Id, Listing>,
    from_root: Id,
    to_root: Id,
    most_left_bytes: usize,
) -> Result<(usize, Result<Vec<MoveNote>, StoreError>), StoreError> {
    // The files that left first: arrivals are worth keeping only for the contents of one, and a
    // save that adds a great deal often has none.
    let (mut changed, mut left, mut left_bytes, mut arrivals) = (0, Places::new(), 0usize, false);
    for_each_entry_change_in(
        store,
        unstored,
        Some(from_root),
        to_root,
        |path, old, new| {
            changed += usize::from(file_change(old, new).is_some());
            arrivals |= matches!((old, file(new)), (None, Some(_)));
            if let (Some(f), None) = (file(old), new)
                && left_bytes <= most_left_bytes
            {
                left_bytes = left_bytes.saturating_add(path.len());
                if left_bytes > most_left_bytes {
                    left = Places::new();
                } else {
                    seen(&mut left, f.id, path);
                }
            }
            Ok::<(), StoreError>(())
        },
    )?;
    left.retain(|_, from| from.is_some());
    if left.is_empty() || !arrivals {
        return Ok((changed, Ok(vec![])));
    }
    Ok((changed, paired(store, unstored, from_root, to_root, left)))
}

/// The moves of the files in `left` (contents to the one path that left with them), from a
/// second walk for the files that arrived with those contents.
fn paired(
    store: &Store,
    unstored: &HashMap<Id, Listing>,
    from_root: Id,
    to_root: Id,
    mut left: Places,
) -> Result<Vec<MoveNote>, StoreError> {
    #[cfg(test)]
    SECOND_WALKS.set(SECOND_WALKS.get() + 1);
    let mut arrived = Places::new();
    for_each_entry_change_in(
        store,
        unstored,
        Some(from_root),
        to_root,
        |path, old, new| {
            if let (None, Some(f)) = (old, file(new))
                && left.contains_key(&f.id)
            {
                #[cfg(test)]
                ARRIVALS_KEPT.set(ARRIVALS_KEPT.get() + 1);
                seen(&mut arrived, f.id, path);
            }
            Ok::<(), StoreError>(())
        },
    )?;
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
    Ok(notes)
}

/// One version a merge reads: its root listing, and its save, `None` when it is made up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Version {
    pub root: Id,
    pub save: Option<Id>,
}

/// Where the files of `start` went on `side`: start path to side path. Each move is checked.
/// Listings in `made` are read before the store's.
pub(crate) fn followed(
    repo: &Repository,
    made: &Made,
    start: &Version,
    side: &Version,
) -> Result<BTreeMap<String, String>, RepoError> {
    let store = repo.store();
    let noted = match (start.save, side.save) {
        (Some(from), Some(tip)) => noted(repo, from, start.root, tip)?,
        _ => BTreeMap::new(),
    };
    let mut at_start = Tree::new(store, &made.listings, start.root, EXACT);
    let mut on_side = Tree::new(store, &made.listings, side.root, EXACT);
    let mut went = BTreeMap::new();
    // A side whose notes moved a file has moved it, whatever it holds where the file was.
    for (from, to) in noted {
        if fits(&mut at_start, &mut on_side, &from, &to)? {
            went.insert(from, to);
        }
    }
    let taken: HashSet<String> = went.values().cloned().collect();
    for n in same_contents(store, &made.listings, start.root, side.root)? {
        if !went.contains_key(&n.from)
            && !taken.contains(&n.to)
            && fits(&mut at_start, &mut on_side, &n.from, &n.to)?
            && on_side.entry(&n.from)?.is_none()
        {
            went.insert(n.from, n.to);
        }
    }
    Ok(went)
}

/// The moves the notes of `tip`'s saves tell, back along first parents to `start` (whose root
/// is `start_root`) and taken one after another; none when that path doesn't reach it. A note
/// counts only when it is true of its own save.
fn noted(
    repo: &Repository,
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
    // Where each file of the tip was, as the walk goes back: there, to its path at the tip.
    let mut back: HashMap<String, String> = HashMap::new();
    // Paths at the tip whose file arrived there later than the save the walk is at.
    let mut arrived: HashSet<String> = HashSet::new();
    let mut save = read(tip)?;
    let mut reads = 1u64;
    // Each save is read once; its tree and its parent's keep only the listings its notes need.
    let mut after = Tree::new(repo.store(), &unstored, save.root, EXACT);
    loop {
        let Some(&parent) = save.parents.first() else {
            return none;
        };
        let parent_save = if parent == start {
            None
        } else if reads > most {
            return none;
        } else {
            reads += 1;
            Some(read(parent)?)
        };
        let parent_root = parent_save.as_ref().map_or(start_root, |p| p.root);
        let mut before = Tree::new(repo.store(), &unstored, parent_root, EXACT);
        // A save's notes all hold between it and its first parent, so they are taken together.
        // A file removed and made again in a later save is taken for the same file, as an edit.
        let mut moved = Vec::new();
        for n in std::mem::take(&mut save.move_notes) {
            if validate_path(&n.from).is_err()
                || validate_path(&n.to).is_err()
                || !true_of(&mut before, &mut after, &n)?
            {
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
fn true_of(before: &mut Tree, after: &mut Tree, n: &MoveNote) -> Result<bool, StoreError> {
    let (Some(left), Some(came)) = (
        file(before.entry(&n.from)?.as_ref()).cloned(),
        file(after.entry(&n.to)?.as_ref()).cloned(),
    ) else {
        return Ok(false);
    };
    Ok(left.id == came.id && after.entry(&n.from)?.is_none() && before.entry(&n.to)?.is_none())
}

/// Whether a move from `from` to `to` counts: the start holds a file at `from` and has room at
/// `to`, and the side holds a file at `to`.
fn fits(start: &mut Tree, side: &mut Tree, from: &str, to: &str) -> Result<bool, StoreError> {
    let is_file = |e: Option<Entry>| e.is_some_and(|e| is_file(&e));
    Ok(is_file(start.entry(from)?) && room_at(start, to)? && is_file(side.entry(to)?))
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
    use std::collections::{BTreeMap, HashMap};

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
        same_contents(&store, &unstored, a, b)
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
        let (count, notes) = count_and_moves(&store, &unstored, a, b, most_left_bytes).unwrap();
        let notes = notes.unwrap().into_iter().map(|n| (n.from, n.to)).collect();
        (count, notes)
    }

    #[test]
    fn past_the_bytes_of_paths_left_nothing_is_paired_and_all_is_counted() {
        let from: &[(&str, EntryKind, &[u8])] = &[
            ("gone1", File, b"1"),
            ("gone2", File, b"2"),
            ("moved", File, b"m"),
        ];
        let to: &[(&str, EntryKind, &[u8])] = &[("there", File, b"m")];
        // 15 bytes of paths left: they fit in 15, not in 14.
        assert_eq!(counted(from, to, 15), (4, vec![pair("moved", "there")]));
        assert_eq!(counted(from, to, 14), (4, vec![]));
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
            let (got, _) = count_and_moves(&store, &unstored, a, b, usize::MAX).unwrap();
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
}
