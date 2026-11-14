//! Files that moved between two versions of the project: the same contents gone from one path and
//! present at exactly one new one.

use std::collections::HashMap;

use crate::diff::for_each_entry_change_in;
use crate::id::Id;
use crate::listing::{Entry, EntryKind, Listing};
use crate::names::validate_path;
use crate::save::MoveNote;
use crate::store::{Store, StoreError};

/// Where files with one contents left and arrived: `None` once a second one does.
#[derive(Default)]
struct Places {
    left: Option<Option<String>>,
    arrived: Option<Option<String>>,
}

fn seen(place: &mut Option<Option<String>>, path: &str) {
    *place = match place {
        None => Some(Some(path.to_owned())),
        Some(_) => Some(None),
    };
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
    // Only files that came or went are kept, by what they hold; no file is read.
    let mut by_contents: HashMap<Id, Places> = HashMap::new();
    for_each_entry_change_in(
        store,
        unstored,
        Some(from_root),
        to_root,
        |path, old, new| {
            match (old, new) {
                (Some(_), None) => {
                    if let Some(f) = file(old) {
                        seen(&mut by_contents.entry(f.id).or_default().left, path);
                    }
                }
                (None, Some(_)) => {
                    if let Some(f) = file(new) {
                        seen(&mut by_contents.entry(f.id).or_default().arrived, path);
                    }
                }
                _ => {}
            }
            Ok::<(), StoreError>(())
        },
    )?;
    let mut notes: Vec<MoveNote> = by_contents
        .into_values()
        .filter_map(|p| match (p.left, p.arrived) {
            (Some(Some(from)), Some(Some(to))) => Some(MoveNote { from, to }),
            _ => None,
        })
        // A path a save can't hold can't be noted either.
        .filter(|n| validate_path(&n.from).is_ok() && validate_path(&n.to).is_ok())
        .collect();
    notes.sort_unstable_by(|a, b| a.to.cmp(&b.to));
    Ok(notes)
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
