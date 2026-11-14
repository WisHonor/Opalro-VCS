//! Files that moved between two versions of the project: the same contents gone from one path and
//! present at exactly one new one.

use std::collections::HashMap;

use crate::diff::{file_change, for_each_entry_change_in};
use crate::id::Id;
use crate::listing::{Entry, EntryKind, Listing};
use crate::names::validate_path;
use crate::save::MoveNote;
use crate::store::{Store, StoreError};

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
#[allow(
    dead_code,
    reason = "a save takes its count too; a combine needs only the moves"
)]
pub(crate) fn same_contents(
    store: &Store,
    unstored: &HashMap<Id, Listing>,
    from_root: Id,
    to_root: Id,
) -> Result<Vec<MoveNote>, StoreError> {
    count_and_moves(store, unstored, from_root, to_root).map(|(_, notes)| notes)
}

/// How many files differ between the two roots, counted as [`crate::diff::count_changes`]
/// counts them, and [`same_contents`]' moves. The folders that differ are walked once, and again
/// only if a file left with contents no other file that left has.
pub(crate) fn count_and_moves(
    store: &Store,
    unstored: &HashMap<Id, Listing>,
    from_root: Id,
    to_root: Id,
) -> Result<(usize, Vec<MoveNote>), StoreError> {
    // The files that left first: arrivals are worth keeping only for the contents of one, and a
    // save that adds a great deal often has none.
    let (mut changed, mut left) = (0, Places::new());
    for_each_entry_change_in(
        store,
        unstored,
        Some(from_root),
        to_root,
        |path, old, new| {
            changed += usize::from(file_change(old, new).is_some());
            if let (Some(f), None) = (file(old), new) {
                seen(&mut left, f.id, path);
            }
            Ok::<(), StoreError>(())
        },
    )?;
    left.retain(|_, from| from.is_some());
    if left.is_empty() {
        return Ok((changed, vec![]));
    }
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
    Ok((changed, notes))
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
            let (got, _) = count_and_moves(&store, &unstored, a, b).unwrap();
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
