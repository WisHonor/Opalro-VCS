#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;

use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};

use super::tree::{MAX_HELD_TEXT, Made, Merged, Unknown, merge_trees};
use super::*;
use crate::ancestry::only_in;
use crate::changes::DEADLINE;
use crate::conflict::markers::{Labels, Parted, Sides, scan};
use crate::conflict::{Record, Term, stands_as};
use crate::diary::{Action, DiaryEntry, Position, State};
use crate::id::{ChangeId, Flavor, Id, hash_record};
use crate::kept::overlay;
use crate::link::ProjectLink;
use crate::listing::{Entry, EntryKind, Listing};
use crate::moves::{
    HELD_PER_PLACE, MAX_CHECKED_NOTES, MAX_LEFT_BYTES, NOTES_CHECKED, Version, followed,
    same_contents,
};
use crate::names::PortabilityProblem;
use crate::pack::{ObjectKind, PackSet};
use crate::presets::{Engine, Found, Preset, opal_toml};
use crate::repo::{ChangeLock, Repository};
use crate::rules::{RuleMatcher, RulesRead};
use crate::save::{EXTRA_LINE, MoveNote, Person, Save};
use crate::snapshot::{SaveOutcome, SaveRequest, save_folder, store_listing};
use crate::store::{Store, counted};

/// What a test puts at a path.
#[derive(Clone, Debug, PartialEq, Eq)]
enum V {
    /// Contents, and whether it is executable.
    File(Vec<u8>, bool),
    Link(String),
    Folder,
    /// A Git submodule's link to a commit of 20 bytes all `n`.
    Project(u8),
    /// A place in conflict, its record's ID.
    Conflict(Id),
}

fn project_link(n: u8) -> ProjectLink {
    ProjectLink::submodule(&[n; 20])
}

fn f(text: &str) -> V {
    V::File(text.as_bytes().to_vec(), false)
}

fn x(text: &str) -> V {
    V::File(text.as_bytes().to_vec(), true)
}

fn link(target: &str) -> V {
    V::Link(target.to_owned())
}

type Tree = BTreeMap<String, V>;

fn tree(items: &[(&str, V)]) -> Tree {
    items
        .iter()
        .map(|(p, v)| ((*p).to_owned(), v.clone()))
        .collect()
}

struct Fixture {
    _dir: tempfile::TempDir,
    repo: Repository,
    lock: ChangeLock,
    empty: Id,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let who = Person {
            name: "Ada".into(),
            email: "a@example.com".into(),
            time_ms: 0,
            tz_offset_min: 0,
        };
        let mut repo = Repository::init(dir.path(), &who).unwrap();
        let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
        let mut packs = repo.new_packs().unwrap();
        let empty = store_listing(repo.store(), &mut packs, &Listing::empty()).unwrap();
        repo.store_packs(&lock, packs).unwrap();
        Fixture {
            _dir: dir,
            repo,
            lock,
            empty,
        }
    }

    /// Stores a tree, built from an empty one, and gives its top listing's ID.
    fn store(&mut self, t: &Tree) -> Id {
        let mut packs = self.repo.new_packs().unwrap();
        let root = self.put(t, &mut packs);
        self.repo.store_packs(&self.lock, packs).unwrap();
        root
    }

    fn put(&self, t: &Tree, packs: &mut PackSet) -> Id {
        put(self.repo.store(), self.empty, t, packs)
    }

    fn three(&mut self, s: &Tree, o: &Tree, t: &Tree) -> [Id; 3] {
        let mut packs = self.repo.new_packs().unwrap();
        let ids = [s, o, t].map(|x| self.put(x, &mut packs));
        self.repo.store_packs(&self.lock, packs).unwrap();
        ids
    }

    fn merge(&mut self, ids: [Id; 3]) -> Result<Merged, CombineError> {
        let rules = RuleMatcher::new(&RulesRead::default(), &RulesRead::default());
        self.merge_with(ids, &rules, false, &mut Made::default())
    }

    /// Merges, then stores what the merge built so that it can be read back.
    fn merge_with(
        &mut self,
        ids: [Id; 3],
        rules: &RuleMatcher,
        inner: bool,
        made: &mut Made,
    ) -> Result<Merged, CombineError> {
        self.merge_unknown(ids, rules, inner, made, &Unknown::new())
    }

    /// `merge_with`, the start unknown at and below each of `unknown`.
    fn merge_unknown(
        &mut self,
        [s, o, t]: [Id; 3],
        rules: &RuleMatcher,
        inner: bool,
        made: &mut Made,
        unknown: &Unknown,
    ) -> Result<Merged, CombineError> {
        let mut packs = self.repo.new_packs().unwrap();
        let store = self.repo.store();
        let merged = merge_trees(
            store,
            &mut packs,
            made,
            inner,
            s,
            o,
            t,
            rules,
            &BTreeMap::new(),
            unknown,
            DEADLINE,
        );
        self.repo.store_packs(&self.lock, packs).unwrap();
        merged
    }

    /// Every path of a stored tree, folders as `Folder`.
    fn read(&self, root: Id) -> Tree {
        self.read_in(root, &HashMap::new())
    }

    /// `read`, with listings from `made` first.
    fn read_in(&self, root: Id, made: &HashMap<Id, Listing>) -> Tree {
        read_tree(self.repo.store(), root, made)
    }

    /// The merge of three trees, which must have no conflicts, read back.
    fn merged(&mut self, s: &Tree, o: &Tree, t: &Tree) -> Tree {
        let ids = self.three(s, o, t);
        let m = self.merge(ids).unwrap();
        assert!(m.conflicts.is_empty(), "{:?}", m.conflicts);
        self.read(m.root)
    }

    /// The conflicts of a merge, described with the sides called `main` and `side`.
    fn conflicts(&mut self, s: &Tree, o: &Tree, t: &Tree) -> Vec<String> {
        let ids = self.three(s, o, t);
        let m = self.merge(ids).unwrap();
        m.conflicts
            .iter()
            .map(|c| c.describe("main", "side"))
            .collect()
    }
}

/// Builds a tree on the stored listing `empty`, its objects in `packs`; gives its top listing's ID.
fn put(store: &Store, empty: Id, t: &Tree, packs: &mut PackSet) -> Id {
    let mut changes = BTreeMap::new();
    for (path, v) in t {
        let put = match v {
            V::File(bytes, exec) => {
                let kind = if *exec {
                    EntryKind::Executable
                } else {
                    EntryKind::File
                };
                (kind, packs.add_content(bytes).unwrap(), bytes.len() as u64)
            }
            V::Link(target) => (
                EntryKind::Symlink,
                packs.add_content(target.as_bytes()).unwrap(),
                target.len() as u64,
            ),
            V::Folder => (EntryKind::Folder, Id([0; 32]), 0),
            V::Project(n) => {
                let encoded = project_link(*n).encode();
                let id = packs.add_record(ObjectKind::ProjectLink, &encoded).unwrap();
                (EntryKind::ProjectLink, id, 0)
            }
            V::Conflict(id) => (EntryKind::Conflict, *id, 0),
        };
        changes.insert(path.clone(), Some(put));
    }
    let unstored = HashMap::new();
    let stored = crate::conflict::Listings::stored(store, &unstored);
    overlay(&stored, empty, &changes, packs).unwrap()
}

/// Every path of a tree, folders as `Folder`, with listings from `made` first.
fn read_tree(store: &Store, root: Id, made: &HashMap<Id, Listing>) -> Tree {
    let mut out = Tree::new();
    let mut todo = vec![(String::new(), root)];
    while let Some((path, id)) = todo.pop() {
        let listing = made
            .get(&id)
            .cloned()
            .unwrap_or_else(|| store.listing(&id).unwrap());
        for e in listing.entries() {
            let p = if path.is_empty() {
                e.name.clone()
            } else {
                format!("{path}/{}", e.name)
            };
            if e.kind == EntryKind::Folder {
                todo.push((p.clone(), e.id));
            }
            out.insert(p, leaf(store, e.kind, e.id));
        }
    }
    out
}

/// What `read_tree` gives for one entry: a folder as `Folder`, a conflict as its record's ID.
fn leaf(store: &Store, kind: EntryKind, id: Id) -> V {
    let mut bytes = Vec::new();
    match kind {
        EntryKind::Folder => V::Folder,
        EntryKind::Conflict => V::Conflict(id),
        EntryKind::Symlink => {
            store.copy_content(&id, &mut bytes).unwrap();
            V::Link(String::from_utf8(bytes).unwrap())
        }
        EntryKind::ProjectLink => {
            let n = (0..=u8::MAX)
                .find(|&n| hash_record(Flavor::ProjectLink, &project_link(n).encode()) == id)
                .unwrap();
            V::Project(n)
        }
        kind => {
            store.copy_content(&id, &mut bytes).unwrap();
            V::File(bytes, kind == EntryKind::Executable)
        }
    }
}

/// A tree's paths as `read` gives them: every folder above a path too.
fn with_folders(t: &Tree) -> Tree {
    let mut out = t.clone();
    for path in t.keys() {
        let mut up = path.as_str();
        while let Some((above, _)) = up.rsplit_once('/') {
            out.insert(above.to_owned(), V::Folder);
            up = above;
        }
    }
    out
}

#[test]
fn one_sides_change_is_taken() {
    let mut fx = Fixture::new();
    // Each side adds a file to `n`, so the top folder is always merged rather than taken whole.
    let around = |mut t: Tree, file: &str| {
        t.insert(format!("n/{file}"), f(file));
        t
    };
    let cases: [(&str, Tree, Tree, Tree, Tree); 6] = [
        (
            "unchanged",
            tree(&[("a", f("1"))]),
            tree(&[("a", f("1"))]),
            tree(&[("a", f("1"))]),
            tree(&[("a", f("1"))]),
        ),
        (
            "edited on ours",
            tree(&[("a", f("1"))]),
            tree(&[("a", f("2"))]),
            tree(&[("a", f("1"))]),
            tree(&[("a", f("2"))]),
        ),
        (
            "added on theirs",
            tree(&[("a", f("1"))]),
            tree(&[("a", f("1"))]),
            tree(&[("a", f("1")), ("b", f("b"))]),
            tree(&[("a", f("1")), ("b", f("b"))]),
        ),
        (
            "removed on ours",
            tree(&[("a", f("1")), ("b", f("b"))]),
            tree(&[("a", f("1"))]),
            tree(&[("a", f("1")), ("b", f("b"))]),
            tree(&[("a", f("1"))]),
        ),
        (
            "the same edit on both",
            tree(&[("a", f("1"))]),
            tree(&[("a", f("2"))]),
            tree(&[("a", f("2"))]),
            tree(&[("a", f("2"))]),
        ),
        (
            "removed on both",
            tree(&[("a", f("1")), ("b", f("b"))]),
            tree(&[("a", f("1"))]),
            tree(&[("a", f("1"))]),
            tree(&[("a", f("1"))]),
        ),
    ];
    for (case, s, o, t, want) in cases {
        let got = fx.merged(&s, &around(o, "o"), &around(t, "t"));
        let want = with_folders(&around(around(want, "o"), "t"));
        assert_eq!(got, want, "{case}");
    }
}

#[test]
fn a_folder_the_same_on_two_sides_is_not_read() {
    let mut fx = Fixture::new();
    let mut s = tree(&[("a.txt", f("a")), ("b.txt", f("b"))]);
    for i in 0..1000 {
        s.insert(format!("big/{i:04}/f"), f("f"));
    }
    let mut o = s.clone();
    o.insert("b.txt".into(), f("b2"));
    let mut t = s.clone();
    t.insert("a.txt".into(), f("a2"));
    // The three top listings, nothing in `big`.
    let got = merged_reading(&mut fx, &s, &o, &t, 3);
    assert_eq!(got["a.txt"], f("a2"));
    assert_eq!(got["b.txt"], f("b2"));
    assert_eq!(got["big/0999/f"], f("f"));

    // Equal on ours and the start only: theirs' `big` is taken whole, unread.
    let mut t = s.clone();
    t.insert("big/0500/f".into(), f("t"));
    let got = merged_reading(&mut fx, &s, &o, &t, 3);
    assert_eq!(got["big/0500/f"], f("t"));

    // Changed inside on both sides: the top and `big` are read, three versions each; `big/0001`
    // and `big/0500` changed on one side each, so they are taken whole.
    let mut o = s.clone();
    o.insert("big/0001/f".into(), f("o"));
    let got = merged_reading(&mut fx, &s, &o, &t, 6);
    assert_eq!(got["big/0001/f"], f("o"));
    assert_eq!(got["big/0500/f"], f("t"));
}

/// The merge of three trees, which must read `want` listings from the store, or one more.
fn merged_reading(fx: &mut Fixture, s: &Tree, o: &Tree, t: &Tree, want: usize) -> Tree {
    let ids = fx.three(s, o, t);
    let before = counted::listings();
    let m = fx.merge(ids).unwrap();
    let read = counted::listings() - before;
    assert!(
        (want..=want + 1).contains(&read),
        "{read} listings read, not {want}"
    );
    assert!(m.conflicts.is_empty());
    fx.read(m.root)
}

#[test]
fn the_executable_mark_and_the_contents_merge_apart() {
    let mut fx = Fixture::new();
    let got = fx.merged(
        &tree(&[("run.sh", f("echo 1\n"))]),
        &tree(&[("run.sh", x("echo 1\n"))]),
        &tree(&[("run.sh", f("echo 2\n"))]),
    );
    assert_eq!(got, tree(&[("run.sh", x("echo 2\n"))]));
}

#[test]
fn a_folder_removed_on_one_side_keeps_only_what_the_other_changed() {
    let mut fx = Fixture::new();
    let s = tree(&[
        ("d/a", f("a")),
        ("d/b", f("b")),
        ("d/c", f("c")),
        ("z", f("z")),
    ]);
    let removed = tree(&[("z", f("z2"))]);
    let edited = tree(&[
        ("d/a", f("a")),
        ("d/b", f("B")),
        ("d/c", f("c")),
        ("z", f("z")),
    ]);
    assert_eq!(
        fx.conflicts(&s, &removed, &edited),
        ["d/b: changed on side, removed on main"]
    );
    let added = tree(&[
        ("d/a", f("a")),
        ("d/b", f("b")),
        ("d/c", f("c")),
        ("d/new", f("new")),
        ("z", f("z")),
    ]);
    assert_eq!(
        fx.merged(&s, &removed, &added),
        tree(&[("d", V::Folder), ("d/new", f("new")), ("z", f("z2"))])
    );
    let mut untouched = s.clone();
    untouched.insert("y".into(), f("y"));
    assert_eq!(
        fx.merged(&s, &removed, &untouched),
        tree(&[("y", f("y")), ("z", f("z2"))])
    );
}

#[test]
fn two_new_files_at_one_path_conflict() {
    let mut fx = Fixture::new();
    let s = tree(&[]);
    for (o, t) in [(f("1"), f("2")), (f("1"), x("1"))] {
        let got = fx.conflicts(&s, &tree(&[("a", o)]), &tree(&[("a", t)]));
        assert_eq!(got, ["a: added on both lines, differently"]);
    }
}

#[test]
fn a_folder_each_side_took_a_different_file_out_of_is_gone() {
    let mut fx = Fixture::new();
    let got = fx.merged(
        &tree(&[("d/a", f("a")), ("d/b", f("b"))]),
        &tree(&[("d/b", f("b"))]),
        &tree(&[("d/a", f("a"))]),
    );
    assert_eq!(got, tree(&[]));
}

#[test]
fn a_folder_one_side_moved_a_file_out_of_and_the_other_removed_one_from_is_gone() {
    let base = tree(&[("doc/a.txt", f("a\n")), ("doc/b.txt", f("b\n"))]);
    let side = tree(&[("a.txt", f("a\n")), ("doc/b.txt", f("b\n"))]);
    let main = tree(&[("doc/a.txt", f("a\n"))]);
    for noted in [true, false] {
        let mut h = Hist::new();
        let got = combined_tree(&mut h, &base, &side, &main, noted);
        assert_eq!(got, tree(&[("a.txt", f("a\n"))]));
    }
}

#[test]
fn a_folder_a_line_holds_empty_stays_when_the_other_lines_changes_leave_nothing_in_it() {
    let base = tree(&[("d/f", f("f\n")), ("d/g", f("g\n"))]);
    let emptied = tree(&[("d", V::Folder)]);
    let one_left = tree(&[("d/f", f("f\n"))]);
    for (side, main) in [(&emptied, &one_left), (&one_left, &emptied)] {
        let got = combined_tree(&mut Hist::new(), &base, side, main, true);
        assert_eq!(got, emptied);
    }
}

#[test]
fn a_folder_one_line_holds_empty_and_the_other_removed_is_gone() {
    let base = tree(&[("d/f", f("f\n")), ("x", f("x\n"))]);
    let emptied = tree(&[("d", V::Folder), ("x", f("x\n"))]);
    let removed = tree(&[("x", f("x\n"))]);
    for (side, main) in [(&emptied, &removed), (&removed, &emptied)] {
        let got = combined_tree(&mut Hist::new(), &base, side, main, true);
        assert_eq!(got, removed);
    }
}

#[test]
fn an_empty_folder_a_line_saved_is_kept() {
    let empty = ("d", V::Folder);
    // Both lines hold it empty.
    let base = tree(&[("d/a", f("a\n"))]);
    let side = tree(&[empty.clone(), ("s", f("s\n"))]);
    let main = tree(&[empty.clone(), ("m", f("m\n"))]);
    let want = tree(&[empty.clone(), ("m", f("m\n")), ("s", f("s\n"))]);
    assert_eq!(
        combined_tree(&mut Hist::new(), &base, &side, &main, true),
        want
    );
    // One emptied it by hand and the other left it alone.
    let base = tree(&[("d/a", f("a\n")), ("x", f("x\n"))]);
    let side = tree(&[empty.clone(), ("x", f("x\n"))]);
    let main = tree(&[("d/a", f("a\n")), ("x", f("x2\n"))]);
    let want = tree(&[empty.clone(), ("x", f("x2\n"))]);
    assert_eq!(
        combined_tree(&mut Hist::new(), &base, &side, &main, true),
        want
    );
    // One emptied it by hand and the other put a new file in.
    let base = tree(&[("d/a", f("a\n"))]);
    let side = tree(std::slice::from_ref(&empty));
    let main = tree(&[("d/a", f("a\n")), ("d/new", f("new\n"))]);
    let want = tree(&[empty.clone(), ("d/new", f("new\n"))]);
    assert_eq!(
        combined_tree(&mut Hist::new(), &base, &side, &main, true),
        want
    );
    // One made it new.
    let base = tree(&[("x", f("x\n"))]);
    let side = tree(&[empty.clone(), ("x", f("x\n"))]);
    let main = tree(&[("x", f("x2\n"))]);
    let want = tree(&[empty, ("x", f("x2\n"))]);
    assert_eq!(
        combined_tree(&mut Hist::new(), &base, &side, &main, true),
        want
    );
}

#[test]
fn a_folder_left_empty_two_levels_deep_is_gone_and_so_is_its_parent_if_nothing_else_is_in_it() {
    let base = tree(&[("a/b/x", f("x\n")), ("a/b/y", f("y\n"))]);
    let side = tree(&[("a/b/y", f("y\n"))]);
    let main = tree(&[("a/b/x", f("x\n"))]);
    let got = combined_tree(&mut Hist::new(), &base, &side, &main, true);
    assert_eq!(got, tree(&[]));
    let keep = ("a/keep", f("keep\n"));
    let with_keep = |t: &Tree| {
        let mut t = t.clone();
        t.insert(keep.0.to_owned(), keep.1.clone());
        t
    };
    let got = combined_tree(
        &mut Hist::new(),
        &with_keep(&base),
        &with_keep(&side),
        &with_keep(&main),
        true,
    );
    assert_eq!(got, with_folders(&tree(&[keep])));
}

#[test]
fn a_file_against_a_folder_conflicts() {
    let mut fx = Fixture::new();
    let got = fx.conflicts(
        &tree(&[]),
        &tree(&[("x", f("x"))]),
        &tree(&[("x/y", f("y"))]),
    );
    assert_eq!(got, ["x: a file on main, a folder on side"]);
    let got = fx.conflicts(
        &tree(&[("x", f("x"))]),
        &tree(&[("x/y", f("y"))]),
        &tree(&[("x", f("x2"))]),
    );
    assert_eq!(got, ["x: a file on side, a folder on main"]);
}

#[test]
fn a_link_changed_two_ways_conflicts() {
    let mut fx = Fixture::new();
    let got = fx.conflicts(
        &tree(&[("l", link("a"))]),
        &tree(&[("l", link("b"))]),
        &tree(&[("l", link("c"))]),
    );
    assert_eq!(
        got,
        ["l: changed on both lines, and only files are merged line by line"]
    );
    // A file where the start had a link is not merged as text either.
    let got = fx.conflicts(
        &tree(&[("l", link("a"))]),
        &tree(&[("l", f("b\n"))]),
        &tree(&[("l", f("c\n"))]),
    );
    assert_eq!(
        got,
        ["l: changed on both lines, and only files are merged line by line"]
    );
}

#[test]
fn changed_on_one_side_removed_on_the_other_conflicts() {
    let mut fx = Fixture::new();
    let got = fx.conflicts(
        &tree(&[("a", f("1")), ("b", f("b"))]),
        &tree(&[("a", f("2")), ("b", f("b"))]),
        &tree(&[("b", f("b2"))]),
    );
    assert_eq!(got, ["a: changed on main, removed on side"]);
}

#[test]
fn text_both_sides_changed_is_merged_line_by_line() {
    let mut fx = Fixture::new();
    let ids = fx.three(
        &tree(&[("src/a.rs", f("one\ntwo\nthree\nfour\nfive\n"))]),
        &tree(&[("src/a.rs", f("ONE\ntwo\nthree\nfour\nfive\n"))]),
        &tree(&[("src/a.rs", f("one\ntwo\nthree\nfour\nFIVE\n"))]),
    );
    let m = fx.merge(ids).unwrap();
    assert!(m.conflicts.is_empty());
    assert_eq!(m.merged_files, ["src/a.rs"]);
    assert!(m.timed_out.is_empty());
    assert_eq!(
        fx.read(m.root)["src/a.rs"],
        f("ONE\ntwo\nthree\nfour\nFIVE\n")
    );
}

/// Three trees with a conflict at `c` and a text merged at `t`.
fn conflict_and_text() -> [Tree; 3] {
    [
        tree(&[("c", f("1\n")), ("t", f("a\nb\nc\nd\ne\nf\n"))]),
        tree(&[("c", f("2\n")), ("t", f("A\nb\nc\nd\ne\nf\n"))]),
        tree(&[("c", f("3\n")), ("t", f("a\nb\nc\nd\nE\nf\n"))]),
    ]
}

#[test]
fn nothing_is_stored_while_merging() {
    let mut fx = Fixture::new();
    let [s, o, t] = conflict_and_text();
    let [s, o, t] = fx.three(&s, &o, &t);
    let packs_before = fx.repo.store().pack_count();
    let mut packs = fx.repo.new_packs().unwrap();
    let rules = RuleMatcher::new(&RulesRead::default(), &RulesRead::default());
    let mut made = Made::default();
    let store = fx.repo.store();
    let m = merge_trees(
        store,
        &mut packs,
        &mut made,
        false,
        s,
        o,
        t,
        &rules,
        &BTreeMap::new(),
        &Unknown::new(),
        DEADLINE,
    )
    .unwrap();
    assert_eq!(m.conflicts.len(), 1);
    assert_eq!(m.merged_files, ["t"]);
    assert!(!packs.is_empty());
    drop(packs);
    assert_eq!(fx.repo.store().pack_count(), packs_before);
    let left: Vec<_> = std::fs::read_dir(fx.repo.tmp_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn an_outer_merge_keeps_no_text_and_an_inner_one_keeps_its_own() {
    let mut fx = Fixture::new();
    let [s, o, t] = conflict_and_text();
    // Without the conflict, so that the merged text is the only thing made.
    let (s, o, t) = (&s, &o, &t);
    let only_text = |t: &Tree| tree(&[("t", t["t"].clone())]);
    let ids = fx.three(&only_text(s), &only_text(o), &only_text(t));
    let rules = RuleMatcher::new(&RulesRead::default(), &RulesRead::default());

    let mut made = Made::default();
    let m = fx.merge_with(ids, &rules, false, &mut made).unwrap();
    assert_eq!(m.merged_files, ["t"]);
    assert!(made.texts.is_empty());
    assert_eq!(made.held, 0);
    assert!(made.listings.contains_key(&m.root));

    let mut made = Made::default();
    let m = fx.merge_with(ids, &rules, true, &mut made).unwrap();
    // Over the 10 bytes held below.
    let text = b"A\nb\nc\nd\nE\nf\n";
    let id = fx
        .repo
        .store()
        .listing(&m.root)
        .unwrap()
        .get("t")
        .unwrap()
        .id;
    assert_eq!(made.texts.get(&id).map(Vec::as_slice), Some(&text[..]));
    assert_eq!(made.held, text.len() as u64);
    assert_eq!(MAX_HELD_TEXT, 256 << 20);

    let mut made = Made {
        max_held: 10,
        ..Made::default()
    };
    let err = fx.merge_with(ids, &rules, true, &mut made).unwrap_err();
    assert!(matches!(err, CombineError::HeldTooMuch { .. }), "{err:?}");
}

#[test]
fn a_later_merge_reads_what_an_earlier_one_made() {
    // An inner merge's text and listings are read from what it made, never from the store.
    let mut fx = Fixture::new();
    let [s, o, t] = [
        tree(&[("t", f("a\nb\nc\nd\ne\n"))]),
        tree(&[("t", f("A\nb\nc\nd\ne\n"))]),
        tree(&[("t", f("a\nb\nc\nd\nE\n"))]),
    ];
    let [s, o, t] = fx.three(&s, &o, &t);
    let rules = RuleMatcher::new(&RulesRead::default(), &RulesRead::default());
    let mut made = Made::default();
    let mut packs = fx.repo.new_packs().unwrap();
    let store = fx.repo.store();
    let inner = merge_trees(
        store,
        &mut packs,
        &mut made,
        true,
        s,
        o,
        t,
        &rules,
        &BTreeMap::new(),
        &Unknown::new(),
        DEADLINE,
    )
    .unwrap();
    // The made text is in no stored pack: only `made` has it.
    let made_text = made.listings[&inner.root].get("t").unwrap().id;
    assert!(made.texts.contains_key(&made_text));
    let stored = fx.repo.store().copy_content(&made_text, &mut Vec::new());
    assert!(stored.is_err());
    // Both sides change the made-up start's text, so the merge reads it from `made`.
    let later = fx.three(
        &tree(&[]),
        &tree(&[("t", f("A\nb\nc\nd\nE\nours\n"))]),
        &tree(&[("t", f("theirs\nA\nb\nc\nd\nE\n")), ("u", f("u"))]),
    );
    let store = fx.repo.store();
    let m = merge_trees(
        store,
        &mut packs,
        &mut made,
        false,
        inner.root,
        later[1],
        later[2],
        &rules,
        &BTreeMap::new(),
        &Unknown::new(),
        DEADLINE,
    )
    .unwrap();
    assert!(m.conflicts.is_empty(), "{:?}", m.conflicts);
    assert_eq!(m.merged_files, ["t"]);
    fx.repo.store_packs(&fx.lock, packs).unwrap();
    assert_eq!(
        fx.read(m.root),
        tree(&[("t", f("theirs\nA\nb\nc\nd\nE\nours\n")), ("u", f("u"))])
    );
}

#[test]
fn binary_both_changed_conflicts() {
    let mut fx = Fixture::new();
    let got = fx.conflicts(
        &tree(&[("b.bin", f("a\0\n"))]),
        &tree(&[("b.bin", f("b\0\n"))]),
        &tree(&[("b.bin", f("c\0\n"))]),
    );
    assert_eq!(
        got,
        ["b.bin: changed on both lines, and isn't text, so it can't be merged line by line"]
    );
}

#[test]
fn a_rule_with_conflict_markers_false_is_never_merged() {
    let mut fx = Fixture::new();
    let found = Found {
        engines: vec![Engine::Unity],
        languages: vec![],
    };
    let text = opal_toml(Preset::Game, &found).unwrap();
    let (rules, _) = crate::rules::parse(text.as_bytes()).unwrap();
    let rules = RuleMatcher::new(
        &RulesRead {
            rules,
            ..RulesRead::default()
        },
        &RulesRead::default(),
    );
    let base: String = (0..20).map(|i| format!("line {i}\n")).collect();
    let ours = base.replace("line 1\n", "ours\n");
    let theirs = base.replace("line 18\n", "theirs\n");
    let ids = fx.three(
        &tree(&[("Assets/Main.unity", f(&base))]),
        &tree(&[("Assets/Main.unity", f(&ours))]),
        &tree(&[("Assets/Main.unity", f(&theirs))]),
    );
    let m = fx
        .merge_with(ids, &rules, false, &mut Made::default())
        .unwrap();
    assert!(m.merged_files.is_empty());
    assert_eq!(m.conflicts.len(), 1);
    assert_eq!(m.conflicts[0].path, "Assets/Main.unity");
    assert_eq!(
        m.conflicts[0].clash,
        Clash::BothChanged(NotMerged::NoMarkers { theirs: false })
    );
    // The same file merges where no rule says otherwise.
    let m = fx.merge(ids).unwrap();
    assert_eq!(m.merged_files, ["Assets/Main.unity"]);
}

#[test]
fn names_two_sides_bring_that_clash_stop_the_combine() {
    let mut fx = Fixture::new();
    for (ours, theirs) in [("Readme.md", "README.md"), ("\u{e9}", "e\u{301}")] {
        let ids = fx.three(
            &tree(&[]),
            &tree(&[(ours, f("o"))]),
            &tree(&[(theirs, f("t"))]),
        );
        let err = fx.merge(ids).unwrap_err();
        let CombineError::NamesClash {
            folder, problem, ..
        } = &err
        else {
            panic!("{err:?}");
        };
        assert_eq!(folder, "");
        let (PortabilityProblem::CaseCollision { a, b }
        | PortabilityProblem::NormalizationCollision { a, b }) = &**problem
        else {
            panic!("{problem:?}");
        };
        let named: BTreeSet<&str> = [a.as_str(), b.as_str()].into();
        assert_eq!(named, [ours, theirs].into());
    }
    // In a folder below, the folder is named.
    let ids = fx.three(
        &tree(&[("d/x", f("x"))]),
        &tree(&[("d/x", f("x")), ("d/Readme.md", f("o"))]),
        &tree(&[("d/x", f("x")), ("d/README.md", f("t"))]),
    );
    let err = fx.merge(ids).unwrap_err();
    assert!(
        matches!(&err, CombineError::NamesClash { folder, .. } if folder == "d"),
        "{err:?}"
    );
}

#[test]
fn a_clashing_pair_one_side_already_held_is_kept() {
    let mut fx = Fixture::new();
    let s = tree(&[("a", f("a")), ("A", f("A")), ("b", f("b"))]);
    let o = tree(&[("a", f("a")), ("A", f("A")), ("b", f("b2"))]);
    let t = tree(&[("a", f("a")), ("A", f("A")), ("b", f("b")), ("c", f("c"))]);
    let got = fx.merged(&s, &o, &t);
    assert_eq!(
        got,
        tree(&[("a", f("a")), ("A", f("A")), ("b", f("b2")), ("c", f("c"))])
    );
}

/// The two names a `NamesClash` names.
fn clashing(err: &CombineError) -> BTreeSet<String> {
    let CombineError::NamesClash { problem, .. } = err else {
        panic!("{err:?}");
    };
    let (PortabilityProblem::CaseCollision { a, b }
    | PortabilityProblem::NormalizationCollision { a, b }) = &**problem
    else {
        panic!("{problem:?}");
    };
    [a.clone(), b.clone()].into()
}

#[test]
fn three_names_alike_are_checked_pair_by_pair() {
    let mut fx = Fixture::new();
    // Each side holds `README` and one new name: the new names are a pair neither side held.
    let ids = fx.three(
        &tree(&[("README", f("r"))]),
        &tree(&[("README", f("r")), ("Readme", f("o"))]),
        &tree(&[("README", f("r")), ("readme", f("t"))]),
    );
    let err = fx.merge(ids).unwrap_err();
    assert_eq!(
        clashing(&err),
        ["Readme".to_owned(), "readme".to_owned()].into()
    );
    // A side that held all three keeps them, whatever the other changed.
    let all = [("README", f("r")), ("Readme", f("o")), ("readme", f("t"))];
    let mut t = tree(&all);
    t.insert("x".into(), f("x"));
    let got = fx.merged(
        &tree(&all),
        &tree(&[("README", f("r2")), ("Readme", f("o"))]),
        &t,
    );
    assert_eq!(
        got,
        tree(&[("README", f("r2")), ("Readme", f("o")), ("x", f("x"))])
    );
    // Ours brought two of them and holds all three; theirs changed something else.
    let got = fx.merged(
        &tree(&[("README", f("r"))]),
        &tree(&all),
        &tree(&[("README", f("r")), ("x", f("x"))]),
    );
    assert_eq!(got, t);
}

#[test]
fn a_names_clash_stops_the_merge_though_files_conflict_too() {
    let mut fx = Fixture::new();
    let ids = fx.three(
        &tree(&[("c", f("1\n"))]),
        &tree(&[("c", f("2\n")), ("Readme.md", f("o"))]),
        &tree(&[("c", f("3\n")), ("README.md", f("t"))]),
    );
    let err = fx.merge(ids).unwrap_err();
    assert_eq!(
        clashing(&err),
        ["README.md".to_owned(), "Readme.md".to_owned()].into()
    );
}

/// Stores a chain of `levels` folders named `d`, the last holding `files`; gives the first's ID.
fn chain(fx: &mut Fixture, levels: usize, files: &[(&str, &str)]) -> Id {
    let mut packs = fx.repo.new_packs().unwrap();
    let entries = files
        .iter()
        .map(|(name, text)| Entry {
            name: (*name).to_owned(),
            kind: EntryKind::File,
            id: packs.add_content(text.as_bytes()).unwrap(),
            size: text.len() as u64,
        })
        .collect();
    let mut at = Listing::new(entries).unwrap();
    for _ in 1..levels {
        let id = store_listing(fx.repo.store(), &mut packs, &at).unwrap();
        at = Listing::new(vec![folder("d", id)]).unwrap();
    }
    let id = packs
        .add_record(ObjectKind::FolderListing, &at.encode())
        .unwrap();
    fx.repo.store_packs(&fx.lock, packs).unwrap();
    id
}

fn folder(name: &str, id: Id) -> Entry {
    Entry {
        name: name.to_owned(),
        kind: EntryKind::Folder,
        id,
        size: 0,
    }
}

/// Stores a top listing of folders.
fn top(fx: &mut Fixture, folders: &[(&str, Id)]) -> Id {
    let mut packs = fx.repo.new_packs().unwrap();
    let listing = Listing::new(folders.iter().map(|(n, id)| folder(n, *id)).collect()).unwrap();
    let id = store_listing(fx.repo.store(), &mut packs, &listing).unwrap();
    fx.repo.store_packs(&fx.lock, packs).unwrap();
    id
}

#[test]
fn a_version_past_the_limits_is_refused_naming_it() {
    let mut fx = Fixture::new();
    let s = fx.store(&tree(&[("x/f", f("1")), ("y/f", f("1"))]));
    let o = fx.store(&tree(&[("x/f", f("2")), ("y/f", f("2"))]));
    // `x` and `y` are one listing, 4,096 folders deep: met the second time, it is measured whole.
    let deep = chain(&mut fx, 4097, &[("f", "3")]);
    let t = top(&mut fx, &[("x", deep), ("y", deep)]);
    let err = fx.merge([s, o, t]).unwrap_err();
    let CombineError::Past { which, detail, .. } = &err else {
        panic!("{err:?}");
    };
    assert_eq!(*which, Which::Theirs);
    assert!(detail.contains("nest more than 4096 levels"), "{detail}");
    // The same past the start, or ours.
    let err = fx.merge([t, s, o]).unwrap_err();
    assert!(
        matches!(
            err,
            CombineError::Past {
                which: Which::Start,
                ..
            }
        ),
        "{err:?}"
    );
    let err = fx.merge([s, t, o]).unwrap_err();
    assert!(
        matches!(
            err,
            CombineError::Past {
                which: Which::Ours,
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn a_path_past_the_limit_is_refused() {
    // Five folders of 700-byte names: a path below them of 4,096 bytes fits, one longer doesn't.
    let mut fx = Fixture::new();
    let long = "d".repeat(700);
    let deep = |name: &str| format!("{}/{name}", [long.as_str(); 5].join("/"));
    let (fits, past) = (deep(&"h".repeat(591)), deep(&"h".repeat(592)));
    assert_eq!(past.len(), 4097);
    let s = fx.store(&tree(&[(&deep("f"), f("1"))]));
    for (both_add, refused) in [(fits, false), (past, true)] {
        let o = fx.store(&tree(&[(&deep("f"), f("2")), (&both_add, f("o"))]));
        let t = fx.store(&tree(&[(&deep("f"), f("1")), (&both_add, f("t"))]));
        let merged = fx.merge([s, o, t]);
        if !refused {
            assert_eq!(merged.unwrap().conflicts.len(), 1);
            continue;
        }
        let err = merged.unwrap_err();
        let CombineError::Past { which, detail, .. } = &err else {
            panic!("{err:?}");
        };
        assert_eq!(*which, Which::Ours);
        assert_eq!(detail, "a path in it is longer than 4096 bytes");
    }
}

#[test]
fn a_deep_tree_merges_without_recursion() {
    // A path's limit comes before the 4,096 levels: `d/…/d/f` fits it 2,047 folders deep.
    const LEVELS: usize = 2047;
    let mut fx = Fixture::new();
    let s = chain(&mut fx, LEVELS + 1, &[("f", "1"), ("g", "1")]);
    let o = chain(&mut fx, LEVELS + 1, &[("f", "2"), ("g", "1")]);
    let t = chain(&mut fx, LEVELS + 1, &[("f", "1"), ("g", "2")]);
    let m = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(256 << 10)
            .spawn_scoped(scope, || fx.merge([s, o, t]))
            .unwrap()
            .join()
            .unwrap()
    })
    .unwrap();
    assert!(m.conflicts.is_empty());
    let store = fx.repo.store();
    let mut at = m.root;
    for _ in 0..LEVELS {
        at = store.listing(&at).unwrap().get("d").unwrap().id;
    }
    let bottom = store.listing(&at).unwrap();
    let text = |name: &str| {
        let mut bytes = Vec::new();
        store
            .copy_content(&bottom.get(name).unwrap().id, &mut bytes)
            .unwrap();
        bytes
    };
    assert_eq!((text("f"), text("g")), (b"2".to_vec(), b"2".to_vec()));
}

#[test]
fn conflicts_are_named_up_to_100_and_counted_after() {
    let mut fx = Fixture::new();
    let all = |text: &str| -> Tree { (0..150).map(|i| (format!("f{i:03}"), f(text))).collect() };
    let ids = fx.three(&all("1\n"), &all("2\n"), &all("3\n"));
    let m = fx.merge(ids).unwrap();
    assert_eq!(m.conflicts.len(), 100);
    assert_eq!(m.more, 50);
    let paths: Vec<&str> = m.conflicts.iter().map(|c| c.path.as_str()).collect();
    let want: Vec<String> = (0..100).map(|i| format!("f{i:03}")).collect();
    assert_eq!(paths, want);
}

#[test]
fn conflicts_and_merged_files_come_in_path_order() {
    // The folder `a` comes after `a.txt` and before `a0`, as whole paths sort.
    let mut fx = Fixture::new();
    let all = |text: &str| -> Tree {
        ["a0", "a/b", "a.txt"]
            .iter()
            .map(|p| ((*p).to_owned(), f(text)))
            .collect()
    };
    let ids = fx.three(&all("1\n"), &all("2\n"), &all("3\n"));
    let m = fx.merge(ids).unwrap();
    let paths: Vec<&str> = m.conflicts.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(paths, ["a.txt", "a/b", "a0"]);
    let ids = fx.three(
        &all("1\n2\n3\n4\n5\n"),
        &all("o\n2\n3\n4\n5\n"),
        &all("1\n2\n3\n4\nt\n"),
    );
    let m = fx.merge(ids).unwrap();
    assert!(m.conflicts.is_empty(), "{:?}", m.conflicts);
    assert_eq!(m.merged_files, ["a.txt", "a/b", "a0"]);
}

#[test]
fn every_clash_has_its_words() {
    // A path, a moved-to path and a side's name that need escaping.
    let path = "p\u{1b}[31m";
    let (ours, theirs) = ("ma\u{7}in", "side");
    let (o, t) = ("ma\\x07in", "side");
    let c = |clash| Conflict {
        path: path.to_owned(),
        clash,
        stands: None,
    };
    let both = |why| Clash::BothChanged(why);
    let to = |p: &str| p.to_owned();
    let cases = [
        (
            both(NotMerged::Overlap {
                endings_only: false,
            }),
            "changed on both lines, in the same place".to_owned(),
        ),
        (
            both(NotMerged::Overlap { endings_only: true }),
            "changed on both lines, in the same place; the two differ only in line endings"
                .to_owned(),
        ),
        (
            both(NotMerged::Binary),
            "changed on both lines, and isn't text, so it can't be merged line by line".to_owned(),
        ),
        (
            both(NotMerged::TooBig),
            "changed on both lines, and is over 64 MiB, too big to merge line by line".to_owned(),
        ),
        (
            both(NotMerged::TooManyLines),
            "changed on both lines, and has over 1,000,000 lines, too many to merge line by line"
                .to_owned(),
        ),
        (
            both(NotMerged::NoMarkers { theirs: false }),
            "changed on both lines, and opalro.toml says conflict-markers = false for it"
                .to_owned(),
        ),
        (
            both(NotMerged::RulesNotWhole { theirs: false }),
            "changed on both lines, and opalro.toml can't be read whole, so no file is merged line by line"
                .to_owned(),
        ),
        (
            both(NotMerged::NotFiles),
            "changed on both lines, and only files are merged line by line".to_owned(),
        ),
        (
            Clash::ChangedRemoved {
                removed_on: Side::Theirs,
            },
            format!("changed on {o}, removed on {t}"),
        ),
        (
            Clash::ChangedRemoved {
                removed_on: Side::Ours,
            },
            format!("changed on {t}, removed on {o}"),
        ),
        (
            Clash::BothAdded,
            "added on both lines, differently".to_owned(),
        ),
        (
            Clash::FileAndFolder {
                folder_on: Side::Theirs,
            },
            format!("a file on {o}, a folder on {t}"),
        ),
        (
            Clash::FileAndFolder {
                folder_on: Side::Ours,
            },
            format!("a file on {t}, a folder on {o}"),
        ),
        (
            Clash::MovedApart {
                from: to("x\u{7}"),
                moved_on: Side::Ours,
                other_to: to("y\u{7}"),
            },
            format!("moved here from x\\x07 on {o}, and to y\\x07 on {t}"),
        ),
        (
            Clash::MovedApart {
                from: to("x"),
                moved_on: Side::Theirs,
                other_to: to("y"),
            },
            format!("moved here from x on {t}, and to y on {o}"),
        ),
        (
            Clash::MovedRemoved {
                moved_on: Side::Ours,
                from: to("x\u{7}"),
            },
            format!("moved here from x\\x07 on {o}, removed on {t}"),
        ),
        (
            Clash::MovedRemoved {
                moved_on: Side::Theirs,
                from: to("x"),
            },
            format!("moved here from x on {t}, removed on {o}"),
        ),
        (
            Clash::MovedOnto {
                moved_on: Side::Ours,
                from: to("x\u{7}"),
            },
            format!("moved here from x\\x07 on {o}, where {t} has something else"),
        ),
        (
            Clash::MovedOnto {
                moved_on: Side::Theirs,
                from: to("x"),
            },
            format!("moved here from x on {t}, where {o} has something else"),
        ),
        (
            Clash::StartUnknown,
            "changed on both lines, and Opalro can't tell what it held where they parted"
                .to_owned(),
        ),
    ];
    for (clash, words) in cases {
        assert_eq!(
            c(clash).describe(ours, theirs),
            format!("p\\x1b[31m: {words}")
        );
    }
}

// The model: decision 166's rules over whole trees as path maps, written apart from the walk.

/// What a tree holds at `path`: nothing, a file or a link, or a folder with what is below it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Missing,
    Leaf(V),
    Folder(Tree),
}

fn node(t: &Tree, path: &str) -> Node {
    if let Some(v) = t.get(path).filter(|v| **v != V::Folder) {
        return Node::Leaf(v.clone());
    }
    let prefix = format!("{path}/");
    let below: Tree = t
        .iter()
        .filter_map(|(p, v)| p.strip_prefix(&prefix).map(|r| (r.to_owned(), v.clone())))
        .collect();
    if below.is_empty() && !t.contains_key(path) {
        Node::Missing
    } else {
        Node::Folder(below)
    }
}

fn names(t: &Tree) -> BTreeSet<String> {
    t.keys()
        .map(|p| p.split('/').next().unwrap().to_owned())
        .collect()
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Model {
    files: Tree,
    conflicts: BTreeSet<String>,
    clash: bool,
    /// Where conflicts are kept, the five label lines: yours, where the lines parted, where they
    /// parted with nothing there, where what they parted from is unknown, theirs.
    marks: Option<[String; 5]>,
    /// Where the start can't tell what the lines parted from, at and below each path.
    unknown: BTreeSet<String>,
    /// What each kept conflict must hold, by path.
    kept: BTreeMap<String, Want>,
}

/// A kept conflict as the model sees it: the three versions (yours, where the lines parted,
/// theirs), and how it stands.
#[derive(Debug, PartialEq, Eq)]
struct Want {
    terms: [Node; 3],
    shown: Shown,
}

#[derive(Debug, PartialEq, Eq)]
enum Shown {
    Marked(V),
    Yours,
    Nothing,
}

/// What the model puts at a kept conflict's place.
const KEPT: V = V::Conflict(Id([0; 32]));

/// A clash at `path`, named; where conflicts are kept, held with all three versions, shown with
/// markers when two text files clash, else as yours, or nothing where yours is gone. Whether the
/// result has something there. Where the start is unknown, `s` is `Missing`.
fn clash(path: &str, s: &Node, o: &Node, t: &Node, m: &mut Model) -> bool {
    m.conflicts.insert(path.to_owned());
    let Some([ours, parted, nothing, unknown, theirs]) = m.marks.clone() else {
        return false;
    };
    let nothing = if unsure(&m.unknown, path) {
        unknown
    } else {
        nothing
    };
    let marked = |o: &[u8], middle: &str, s: &[u8], t: &[u8], exec: bool| {
        let mut text = format!(
            "<<<<<<< {ours}
"
        )
        .into_bytes();
        text.extend(o);
        text.extend(
            format!(
                "||||||| {middle}
"
            )
            .bytes(),
        );
        text.extend(s);
        text.extend(
            b"=======
",
        );
        text.extend(t);
        text.extend(
            format!(
                ">>>>>>> {theirs}
"
            )
            .bytes(),
        );
        V::File(text, exec)
    };
    let shown = match (s, o, t) {
        (Node::Leaf(V::File(sc, sx)), Node::Leaf(V::File(oc, ox)), Node::Leaf(V::File(tc, tx))) => {
            let exec = if ox == tx || ox == sx { *tx } else { *ox };
            Shown::Marked(marked(oc, &parted, sc, tc, exec))
        }
        (Node::Missing, Node::Leaf(V::File(oc, ox)), Node::Leaf(V::File(tc, _))) if oc != tc => {
            Shown::Marked(marked(oc, &nothing, b"", tc, *ox))
        }
        (_, Node::Missing, _) => Shown::Nothing,
        _ => Shown::Yours,
    };
    m.files.insert(path.to_owned(), KEPT);
    // A folder is held whole: every folder below it too.
    let whole = |n: &Node| match n {
        Node::Folder(below) => Node::Folder(with_folders(below)),
        n => n.clone(),
    };
    let terms = [whole(o), whole(s), whole(t)];
    m.kept.insert(path.to_owned(), Want { terms, shown });
    true
}

/// Merges the folder at `path` (`""` the top) whose contents are `s`, `o` and `t`, and adds to
/// `m` what it holds. A folder below the top is there when something is left in it; an empty one
/// only when a side holds it empty itself.
fn model_folder(path: &str, s: &Tree, o: &Tree, t: &Tree, m: &mut Model) {
    let before = m.files.len();
    let mut kept_names = BTreeSet::new();
    let all: BTreeSet<String> = [s, o, t].iter().flat_map(|x| names(x)).collect();
    for name in all {
        let full = if path.is_empty() {
            name.clone()
        } else {
            format!("{path}/{name}")
        };
        let has = model_entry(&full, &node(s, &name), &node(o, &name), &node(t, &name), m);
        if has {
            kept_names.insert(name);
        }
    }
    // Names alike but for letter case or how `é` is written, unless one side held both.
    let held = |x: &Tree, a: &str, b: &str| names(x).contains(a) && names(x).contains(b);
    let key = |n: &str| n.to_lowercase().replace("e\u{301}", "\u{e9}");
    for a in &kept_names {
        for b in &kept_names {
            if a < b && key(a) == key(b) && !held(o, a, b) && !held(t, a, b) {
                m.clash = true;
            }
        }
    }
    if !path.is_empty() && m.files.len() > before {
        m.files.insert(path.to_owned(), V::Folder);
    }
}

/// Whether the start is unknown at `path`: it, a folder holding it, or something inside it is
/// listed in `unknown`.
fn unsure(unknown: &BTreeSet<String>, path: &str) -> bool {
    unknown.iter().any(|u| {
        u.is_empty()
            || u == path
            || path.starts_with(&format!("{u}/"))
            || u.starts_with(&format!("{path}/"))
    })
}

/// Merges one path; whether the result has something there.
fn model_entry(path: &str, s: &Node, o: &Node, t: &Node, m: &mut Model) -> bool {
    let take = |n: &Node, m: &mut Model| {
        match n {
            Node::Missing => return false,
            Node::Leaf(v) => {
                m.files.insert(path.to_owned(), v.clone());
            }
            Node::Folder(below) => {
                m.files.insert(path.to_owned(), V::Folder);
                for (p, v) in with_folders(below) {
                    m.files.insert(format!("{path}/{p}"), v.clone());
                }
            }
        }
        true
    };
    // Where the start is unknown, here or inside, nothing is unchanged: alike is the result, two
    // folders are merged inside (from nothing where it is unknown here), as is a folder against
    // nothing where it is unknown only inside; anything else is a conflict of the two whole.
    if unsure(&m.unknown, path) {
        if o == t {
            return take(t, m);
        }
        let here = m
            .unknown
            .iter()
            .any(|u| u.is_empty() || u == path || path.starts_with(&format!("{u}/")));
        let start = match s {
            Node::Folder(x) if !here => x.clone(),
            _ => Tree::new(),
        };
        let (a, b) = match (o, t) {
            (Node::Folder(a), Node::Folder(b)) => (a.clone(), b.clone()),
            (Node::Folder(a), Node::Missing) if !here => (a.clone(), Tree::new()),
            (Node::Missing, Node::Folder(b)) if !here => (Tree::new(), b.clone()),
            _ => return clash(path, &Node::Missing, o, t, m),
        };
        let before = m.files.len();
        model_folder(path, &start, &a, &b, m);
        if m.files.len() > before {
            return true;
        }
        let both = matches!((o, t), (Node::Folder(_), Node::Folder(_)));
        if both && (a.is_empty() || b.is_empty()) {
            m.files.insert(path.to_owned(), V::Folder);
            return true;
        }
        return false;
    }
    if o == t || o == s {
        return take(t, m);
    }
    if t == s {
        return take(o, m);
    }
    let empty = Tree::new();
    let folder = |n: &Node| match n {
        Node::Folder(x) => Some(x.clone()),
        _ => None,
    };
    let start = folder(s).unwrap_or_default();
    let merge_folders = |a: &Tree, b: &Tree, m: &mut Model| {
        let before = m.files.len();
        model_folder(path, &start, a, b, m);
        m.files.len() > before
    };
    match (s, o, t) {
        (_, Node::Folder(a), Node::Folder(b)) => {
            if merge_folders(a, b, m) {
                return true;
            }
            // Nothing left: a folder one side holds empty stays empty; else it is gone.
            if a.is_empty() || b.is_empty() {
                m.files.insert(path.to_owned(), V::Folder);
                return true;
            }
            return false;
        }
        (Node::Folder(_), Node::Folder(a), Node::Missing) => return merge_folders(a, &empty, m),
        (Node::Folder(_), Node::Missing, Node::Folder(b)) => return merge_folders(&empty, b, m),
        (Node::Leaf(V::File(sc, sx)), Node::Leaf(V::File(oc, ox)), Node::Leaf(V::File(tc, tx))) => {
            // Contents both sides changed are merged line by line, which the text merge's own
            // tests check; one-line files always conflict.
            let mark = if ox == tx || ox == sx { *tx } else { *ox };
            let contents = if oc == tc || oc == sc {
                tc.clone()
            } else if tc == sc {
                oc.clone()
            } else if let (Text::Merged(both), _) = merge_text(sc, oc, tc, DEADLINE, None) {
                both
            } else {
                return clash(path, s, o, t, m);
            };
            m.files.insert(path.to_owned(), V::File(contents, mark));
            return true;
        }
        _ => {}
    }
    clash(path, s, o, t, m)
}

/// The model's merge; with `marks`, keeping conflicts; the start unknown at and below each of
/// `unknown`.
fn model(
    s: &Tree,
    o: &Tree,
    t: &Tree,
    marks: Option<[String; 5]>,
    unknown: &BTreeSet<String>,
) -> Model {
    let mut m = Model {
        marks,
        unknown: unknown.clone(),
        ..Model::default()
    };
    if !unknown.is_empty() {
        if o == t {
            m.files = with_folders(t);
        } else {
            model_folder("", s, o, t, &mut m);
        }
    } else if o == t || o == s {
        m.files = with_folders(t);
    } else if t == s {
        m.files = with_folders(o);
    } else {
        model_folder("", s, o, t, &mut m);
    }
    m
}

/// `d/e` is a file against a folder holding `b`; `é` written two ways and `É` are three names
/// alike.
const MODEL_PATHS: [&str; 11] = [
    "a", "A", "b", "d/a", "d/c", "d/e", "d/e/b", "D/a", "\u{e9}", "e\u{301}", "\u{c9}",
];

fn value() -> impl Strategy<Value = Option<V>> {
    let text = prop::sample::select(vec!["1\n", "2\n", "3\n", "4\n"]);
    prop_oneof![
        3 => Just(None),
        4 => (text, any::<bool>()).prop_map(|(t, exec)| Some(V::File(t.as_bytes().to_vec(), exec))),
        1 => prop::sample::select(vec!["x", "y"]).prop_map(|t| Some(link(t))),
        1 => Just(Some(V::Folder)),
        1 => (1u8..=2).prop_map(|n| Some(V::Project(n))),
    ]
}

/// Where the start is unknown: nowhere in half the cases, else one to three paths among a
/// folder, its files and its folder's file, and files at the top.
fn unknown_paths() -> impl Strategy<Value = Unknown> {
    let paths = prop::sample::select(vec!["a", "b", "d", "d/a", "d/e", "d/e/b", "\u{e9}"]);
    prop_oneof![
        Just(Unknown::new()),
        prop::collection::btree_set(paths.prop_map(str::to_owned), 1..=3),
    ]
}

/// A start, and each side as the start with some paths changed.
fn three_trees() -> impl Strategy<Value = [Tree; 3]> {
    let values = || prop::collection::vec(value(), MODEL_PATHS.len());
    let changes = || prop::collection::vec(prop::option::weighted(0.4, value()), MODEL_PATHS.len());
    (values(), changes(), changes()).prop_map(|(start, ours, theirs)| {
        // Nothing below a file, as no stored tree has.
        let build = |vs: &[Option<V>]| -> Tree {
            let all: Tree = MODEL_PATHS
                .iter()
                .zip(vs)
                .filter_map(|(p, v)| v.clone().map(|v| ((*p).to_owned(), v)))
                .collect();
            let under_a_file = |p: &str| {
                all.iter()
                    .any(|(up, v)| *v != V::Folder && p.starts_with(&format!("{up}/")))
            };
            all.iter()
                .filter(|(p, _)| !under_a_file(p))
                .map(|(p, v)| (p.clone(), v.clone()))
                .collect()
        };
        let side = |changed: &[Option<Option<V>>]| -> Vec<Option<V>> {
            start
                .iter()
                .zip(changed)
                .map(|(s, c)| c.clone().unwrap_or_else(|| s.clone()))
                .collect()
        };
        [build(&start), build(&side(&ours)), build(&side(&theirs))]
    })
}

#[test]
fn tree_merge_agrees_with_a_path_by_path_model() {
    let fx = std::cell::RefCell::new(Fixture::new());
    let mut runner = TestRunner::new(Config::with_cases(256));
    runner
        .run(&(three_trees(), unknown_paths()), |([s, o, t], unknown)| {
            let want = model(&s, &o, &t, None, &unknown);
            let mut fx = fx.borrow_mut();
            let [s_id, o_id, t_id] = fx.three(&s, &o, &t);
            // Read back from what it made, never stored: no text is merged here.
            let mut packs = fx.repo.new_packs().unwrap();
            let mut made = Made::default();
            let rules = RuleMatcher::new(&RulesRead::default(), &RulesRead::default());
            let store = fx.repo.store();
            let merged = merge_trees(
                store,
                &mut packs,
                &mut made,
                false,
                s_id,
                o_id,
                t_id,
                &rules,
                &BTreeMap::new(),
                &unknown,
                DEADLINE,
            );
            match merged {
                Err(CombineError::NamesClash { .. }) => prop_assert!(want.clash, "{want:?}"),
                Err(e) => prop_assert!(false, "{e:?}"),
                Ok(m) => {
                    prop_assert!(!want.clash, "{want:?}");
                    let got: BTreeSet<String> =
                        m.conflicts.iter().map(|c| c.path.clone()).collect();
                    prop_assert_eq!(&got, &want.conflicts);
                    if got.is_empty() {
                        prop_assert!(m.merged_files.is_empty());
                        prop_assert_eq!(fx.read_in(m.root, &made.listings), want.files);
                    }
                }
            }
            Ok(())
        })
        .unwrap();
    // Keeping conflicts: each place the model calls a clash holds a record of exactly the model's
    // three versions, standing as it says; every other place holds the model's merge.
    let one = ChangeId([3; 16]);
    let [labels, nothing, unsure] = [
        Parted::One(one),
        Parted::Nothing(Some(one)),
        Parted::Unknown,
    ]
    .map(|p| sides().labels(p));
    let marks = [
        labels.ours,
        labels.base,
        nothing.base,
        unsure.base,
        labels.theirs,
    ];
    let seen: std::cell::RefCell<BTreeMap<&str, usize>> = Default::default();
    let mut runner = TestRunner::new_with_rng(
        Config::with_cases(256),
        TestRng::deterministic_rng(RngAlgorithm::ChaCha),
    );
    runner
        .run(&(three_trees(), unknown_paths()), |([s, o, t], unknown)| {
            let want = model(&s, &o, &t, Some(marks.clone()), &unknown);
            let mut fx = fx.borrow_mut();
            let ids = fx.three(&s, &o, &t);
            let mut made = Made {
                keep: Some(sides()),
                ..Made::default()
            };
            let mut seen = seen.borrow_mut();
            // Each unknown place the merge meets first, by what the two lines hold there.
            for u in &unknown {
                if unknown.iter().any(|up| u.starts_with(&format!("{up}/"))) {
                    continue;
                }
                let kind = match (node(&o, u), node(&t, u)) {
                    (a, b) if a == b => "unknown, alike",
                    (Node::Folder(_), Node::Folder(_)) => "unknown, two folders",
                    (Node::Missing, _) | (_, Node::Missing) => "unknown, on one line",
                    _ => "unknown, different",
                };
                *seen.entry(kind).or_default() += 1;
            }
            let m = match fx.merge_unknown(ids, &no_rules(), false, &mut made, &unknown) {
                Err(CombineError::NamesClash { .. }) => {
                    prop_assert!(want.clash, "{want:?}");
                    return Ok(());
                }
                Err(e) => return Err(TestCaseError::fail(format!("{e:?}"))),
                Ok(m) => m,
            };
            prop_assert!(!want.clash, "{want:?}");
            let got: BTreeSet<String> = m.conflicts.iter().map(|c| c.path.clone()).collect();
            prop_assert_eq!(&got, &want.conflicts);
            for c in &m.conflicts {
                let kind = match c.clash {
                    Clash::BothChanged(_) => "both changed",
                    Clash::ChangedRemoved { .. } => "changed and removed",
                    Clash::BothAdded => "both added",
                    Clash::FileAndFolder { .. } => "file and folder",
                    Clash::StartUnknown => "start unknown",
                    _ => "other",
                };
                *seen.entry(kind).or_default() += 1;
                let stands = match c.stands {
                    Some(Stands::Markers { .. }) => "markers",
                    Some(Stands::Yours) => "yours",
                    Some(Stands::Nothing) => "nothing",
                    None => "not kept",
                };
                *seen.entry(stands).or_default() += 1;
            }
            let store = fx.repo.store();
            let mut tree = read_tree(store, m.root, &made.listings);
            for (path, kept) in &want.kept {
                let Some(V::Conflict(id)) = tree.insert(path.clone(), KEPT) else {
                    return Err(TestCaseError::fail(format!("no conflict at {path}")));
                };
                let r = store.conflict(&id).unwrap();
                let terms: Vec<Node> = r.terms.iter().map(|t| term_node(store, *t)).collect();
                prop_assert_eq!(&terms, &kept.terms, "{}", path);
                let own = r
                    .own
                    .map(|own| (own.marker, own.version.map(|v| leaf(store, v.kind, v.id))));
                let shown = match own {
                    Some((7, Some(v))) => Shown::Marked(v),
                    None if r.mine().is_some() => Shown::Yours,
                    None => Shown::Nothing,
                    other => return Err(TestCaseError::fail(format!("{path}: {other:?}"))),
                };
                prop_assert_eq!(&shown, &kept.shown, "{}", path);
            }
            prop_assert_eq!(tree, want.files);
            Ok(())
        })
        .unwrap();
    let seen = seen.into_inner();
    for kind in [
        "both changed",
        "changed and removed",
        "both added",
        "file and folder",
        "start unknown",
        "markers",
        "yours",
        "nothing",
        "unknown, alike",
        "unknown, two folders",
        "unknown, on one line",
        "unknown, different",
    ] {
        assert!(seen.get(kind).is_some_and(|&n| n >= 3), "{seen:?}");
    }
    assert!(
        !seen.contains_key("not kept") && !seen.contains_key("other"),
        "{seen:?}"
    );
}

/// What a record's term holds, as the model sees a path: a folder with everything below it.
fn term_node(store: &Store, t: Option<Term>) -> Node {
    match t {
        None => Node::Missing,
        Some(t) if t.kind == EntryKind::Folder => {
            Node::Folder(read_tree(store, t.id, &HashMap::new()))
        }
        Some(t) => Node::Leaf(leaf(store, t.kind, t.id)),
    }
}

fn ada() -> Person {
    Person {
        name: "Ada".into(),
        email: "a@example.com".into(),
        time_ms: 0,
        tz_offset_min: 0,
    }
}

/// A project whose saves are made from its folder, so they note what `save` notes.
struct Hist {
    dir: tempfile::TempDir,
    repo: Repository,
    /// The project's first save, which holds nothing.
    first: Id,
    /// What its combines keep.
    keep: Keep,
}

impl Hist {
    fn new() -> Hist {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path(), &ada()).unwrap();
        let first = repo.current().unwrap().1.state.lines["main"];
        Hist {
            dir,
            repo,
            first,
            keep: Keep::Conflicts,
        }
    }

    /// Commits a step whose state is the current one as `change` leaves it.
    fn set(&mut self, change: impl FnOnce(&mut State)) {
        let lock = self.repo.lock_for_change("test", &mut |_| {}).unwrap();
        let (head, mut entry) = self.repo.current().unwrap();
        change(&mut entry.state);
        let entry = DiaryEntry {
            parent: Some(head),
            action: Action::Command,
            command: "test".into(),
            folder_change: None,
            ..entry
        };
        let packs = self.repo.new_packs().unwrap();
        self.repo.commit(&lock, packs, &entry).unwrap();
    }

    /// Saves the folder holding `t` on `line`, after `parent`.
    fn save(&mut self, line: &str, parent: Id, t: &Tree) -> Id {
        self.set(|s| {
            s.lines.insert(line.to_owned(), parent);
            s.position = Position::OnLine(line.to_owned());
        });
        self.write(t);
        assert!(matches!(
            self.save_now("Change things"),
            SaveOutcome::Saved { .. }
        ));
        self.repo.current().unwrap().1.state.lines[line]
    }

    /// Saves the folder as it is on the line the project is on.
    fn save_now(&mut self, message: &str) -> SaveOutcome {
        let who = ada();
        let req = SaveRequest {
            message: Some(message),
            who: &who,
            command: "save",
            accept_crlf: false,
            git_excludes: None,
            new_line: None,
            only: None,
            amend: false,
        };
        save_folder(&mut self.repo, &req, &mut |_| {}, &mut |_| true).unwrap()
    }

    /// Makes the folder hold exactly `t`.
    fn write(&self, t: &Tree) {
        let root = self.dir.path();
        for e in fs::read_dir(root).unwrap() {
            let e = e.unwrap();
            if e.file_name() == ".opalro" {
                continue;
            }
            if e.file_type().unwrap().is_dir() {
                fs::remove_dir_all(e.path()).unwrap();
            } else {
                fs::remove_file(e.path()).unwrap();
            }
        }
        for (path, v) in t {
            let at = root.join(path);
            match v {
                V::Folder => fs::create_dir_all(at).unwrap(),
                V::File(bytes, false) => {
                    fs::create_dir_all(at.parent().unwrap()).unwrap();
                    fs::write(at, bytes).unwrap();
                }
                other => panic!("{other:?}"),
            }
        }
    }

    /// Stores a save of `t` after `parents` noting `notes`, or what `save` would note against
    /// the first parent when `None`: a history from Git, a damaged one, or a combined save.
    fn plant(&mut self, parents: &[Id], t: &Tree, notes: Option<&[(&str, &str)]>) -> Id {
        let root = {
            let lock = self.repo.lock_for_change("test", &mut |_| {}).unwrap();
            let first = self.repo.store().save(&self.first, false).unwrap();
            let mut packs = self.repo.new_packs().unwrap();
            let root = put(self.repo.store(), first.root, t, &mut packs);
            self.repo.store_packs(&lock, packs).unwrap();
            root
        };
        let move_notes = match notes {
            Some(notes) => notes
                .iter()
                .map(|(from, to)| MoveNote {
                    from: (*from).to_owned(),
                    to: (*to).to_owned(),
                })
                .collect(),
            None => {
                let before = self.repo.store().save(&parents[0], false).unwrap().root;
                same_contents(
                    self.repo.store(),
                    &HashMap::new(),
                    before,
                    root,
                    MAX_LEFT_BYTES,
                )
                .unwrap()
                .unwrap()
            }
        };
        self.plant_root(parents, root, move_notes)
    }

    /// Stores a save of the stored listing `root` after `parents`, noting `move_notes`.
    fn plant_root(&mut self, parents: &[Id], root: Id, move_notes: Vec<MoveNote>) -> Id {
        let lock = self.repo.lock_for_change("test", &mut |_| {}).unwrap();
        let first = self.repo.store().save(&self.first, false).unwrap();
        let save = Save {
            root,
            parents: parents.to_vec(),
            message: "Planted".into(),
            change_id: ChangeId::random().unwrap(),
            move_notes,
            ..first
        };
        let mut packs = self.repo.new_packs().unwrap();
        let id = packs.add_record(ObjectKind::Save, &save.encode()).unwrap();
        self.repo.store_packs(&lock, packs).unwrap();
        id
    }

    fn version(&self, save: Id) -> Version {
        Version {
            root: self.repo.store().save(&save, false).unwrap().root,
            save: Some(save),
        }
    }

    /// Where `start`'s files went on `side`.
    fn followed(&self, start: Version, side: Version) -> Vec<(String, String)> {
        followed(&self.repo, &mut Made::default(), &start, &side)
            .unwrap()
            .went
            .into_iter()
            .collect()
    }

    /// The merge of three versions, with the conflicts described with the sides called `main`
    /// and `side`, and its result read back.
    fn outcome(&mut self, s: Version, o: Version, t: Version) -> (Vec<String>, Tree) {
        self.outcome_with(
            s,
            o,
            t,
            &RuleMatcher::new(&RulesRead::default(), &RulesRead::default()),
        )
    }

    /// `outcome`, with `rules`.
    fn outcome_with(
        &mut self,
        s: Version,
        o: Version,
        t: Version,
        rules: &RuleMatcher,
    ) -> (Vec<String>, Tree) {
        let lock = self.repo.lock_for_change("test", &mut |_| {}).unwrap();
        let mut packs = self.repo.new_packs().unwrap();
        let mut made = Made::default();
        let m = merge_three(&self.repo, &mut packs, &mut made, false, &s, &o, &t, rules).unwrap();
        self.repo.store_packs(&lock, packs).unwrap();
        let described = m
            .conflicts
            .iter()
            .map(|c| c.describe("main", "side"))
            .collect();
        (
            described,
            read_tree(self.repo.store(), m.root, &HashMap::new()),
        )
    }

    /// The merge of three saves, which must have no conflicts, read back.
    fn merged(&mut self, s: Id, o: Id, t: Id) -> Tree {
        let (conflicts, tree) = self.outcome(self.version(s), self.version(o), self.version(t));
        assert!(conflicts.is_empty(), "{conflicts:?}");
        tree
    }

    fn conflicts(&mut self, s: Id, o: Id, t: Id) -> Vec<String> {
        self.outcome(self.version(s), self.version(o), self.version(t))
            .0
    }
}

const BODY: &str = "one\ntwo\nthree\nfour\nfive\n";

/// `BODY` with line `n` (from 0) replaced.
fn edit(n: usize, to: &str) -> V {
    let mut lines: Vec<&str> = BODY.lines().collect();
    lines[n] = to;
    f(&format!("{}\n", lines.join("\n")))
}

fn pairs(p: &[(&str, &str)]) -> Vec<(String, String)> {
    p.iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect()
}

/// A start holding `Player.cs` and `Other.cs`, saved on `main`.
fn player(h: &mut Hist) -> Id {
    let first = h.first;
    h.save(
        "main",
        first,
        &tree(&[("Player.cs", f(BODY)), ("Other.cs", f("other\n"))]),
    )
}

/// `Player.cs` at `to`, beside `Other.cs`.
fn moved_to(to: &str, v: V) -> Tree {
    tree(&[(to, v), ("Other.cs", f("other\n"))])
}

#[test]
fn a_change_to_a_moved_file_lands_where_it_went() {
    let mut h = Hist::new();
    let s = player(&mut h);
    let o = h.save("main", s, &moved_to("Characters/Player.cs", f(BODY)));
    let t = h.save("side", s, &moved_to("Player.cs", edit(1, "TWO")));
    let want = with_folders(&moved_to("Characters/Player.cs", edit(1, "TWO")));
    assert_eq!(h.merged(s, o, t), want);
    // Either way round.
    assert_eq!(h.merged(s, t, o), want);
}

#[test]
fn both_moved_to_the_same_place_are_merged_there() {
    let mut h = Hist::new();
    let s = player(&mut h);
    let to = "Characters/Player.cs";
    let o = h.save("main", s, &moved_to(to, f(BODY)));
    let o = h.save("main", o, &moved_to(to, edit(0, "ONE")));
    let t = h.save("side", s, &moved_to(to, f(BODY)));
    let t = h.save("side", t, &moved_to(to, edit(4, "FIVE")));
    let both = "ONE\ntwo\nthree\nfour\nFIVE\n";
    assert_eq!(h.merged(s, o, t), with_folders(&moved_to(to, f(both))));
}

#[test]
fn moved_apart_conflicts() {
    let mut h = Hist::new();
    let s = player(&mut h);
    let o = h.save("main", s, &moved_to("A/Player.cs", f(BODY)));
    let t = h.save("side", s, &moved_to("B/Player.cs", f(BODY)));
    assert_eq!(
        h.conflicts(s, o, t),
        [
            "A/Player.cs: moved here from Player.cs on main, and to B/Player.cs on side",
            "B/Player.cs: moved here from Player.cs on side, and to A/Player.cs on main",
        ]
    );
}

#[test]
fn moved_and_removed_conflicts() {
    let mut h = Hist::new();
    let s = player(&mut h);
    let moved = h.save("main", s, &moved_to("Characters/Player.cs", f(BODY)));
    let removed = h.save("side", s, &tree(&[("Other.cs", f("other\n"))]));
    assert_eq!(
        h.conflicts(s, moved, removed),
        ["Characters/Player.cs: moved here from Player.cs on main, removed on side"]
    );
    assert_eq!(
        h.conflicts(s, removed, moved),
        ["Characters/Player.cs: moved here from Player.cs on side, removed on main"]
    );
}

#[test]
fn moved_onto_something_new_conflicts() {
    let mut h = Hist::new();
    let s = player(&mut h);
    let to = "Characters/Player.cs";
    let o = h.save("main", s, &moved_to(to, f(BODY)));
    let mut added = moved_to("Player.cs", edit(1, "TWO"));
    added.insert(to.to_owned(), f("new\n"));
    let t = h.save("side", s, &added);
    assert_eq!(
        h.conflicts(s, o, t),
        [
            "Characters/Player.cs: moved here from Player.cs on main, where side has something else",
            "Player.cs: changed on side, removed on main",
        ]
    );
    // A file where the moved file's folder would be: that clash holds the moved file.
    let mut added = moved_to("Player.cs", f(BODY));
    added.insert("Characters".to_owned(), f("new\n"));
    let t = h.save("side", s, &added);
    assert_eq!(
        h.conflicts(s, o, t),
        ["Characters: a file on side, a folder on main"]
    );
}

#[test]
fn notes_chain_a_to_b_to_c() {
    let mut h = Hist::new();
    let s = player(&mut h);
    let o = h.save("main", s, &moved_to("B.cs", f(BODY)));
    let o = h.save("main", o, &moved_to("C/C.cs", f(BODY)));
    // Edited too, so only the notes can follow it.
    let o = h.save("main", o, &moved_to("C/C.cs", edit(0, "ONE")));
    assert_eq!(
        h.followed(h.version(s), h.version(o)),
        pairs(&[("Player.cs", "C/C.cs")])
    );
    let t = h.save("side", s, &moved_to("Player.cs", edit(4, "FIVE")));
    let both = "ONE\ntwo\nthree\nfour\nFIVE\n";
    assert_eq!(
        h.merged(s, o, t),
        with_folders(&moved_to("C/C.cs", f(both)))
    );
}

#[test]
fn a_history_without_notes_is_followed_by_contents() {
    let mut h = Hist::new();
    let first = h.first;
    let none: &[(&str, &str)] = &[];
    let start = tree(&[("Player.cs", f(BODY)), ("Other.cs", f("other\n"))]);
    let s = h.plant(&[first], &start, Some(none));
    let to = "Characters/Player.cs";
    let o = h.plant(&[s], &moved_to(to, f(BODY)), Some(none));
    let t = h.plant(&[s], &moved_to("Player.cs", edit(1, "TWO")), Some(none));
    assert_eq!(
        h.merged(s, o, t),
        with_folders(&moved_to(to, edit(1, "TWO")))
    );
}

/// A rule that Unity scenes are never merged line by line.
fn scene_rule() -> RuleMatcher {
    let text = "[[rule]]\nfiles = [\"*.unity\"]\nconflict-markers = false\n";
    let (rules, _) = crate::rules::parse(text.as_bytes()).unwrap();
    RuleMatcher::new(
        &RulesRead {
            rules,
            ..RulesRead::default()
        },
        &RulesRead::default(),
    )
}

/// `from` saved on `main`; on `main`, moved to `to` in a save of its own, then edited there; on
/// `side`, edited in another place. Gives the three saves.
fn moved_and_edited(h: &mut Hist, from: &str, to: &str) -> (Id, Id, Id) {
    let first = h.first;
    let s = h.save("main", first, &tree(&[(from, f(BODY))]));
    let o = h.save("main", s, &tree(&[(to, f(BODY))]));
    let o = h.save("main", o, &tree(&[(to, edit(0, "ONE"))]));
    let t = h.save("side", s, &tree(&[(from, edit(4, "FIVE"))]));
    (s, o, t)
}

/// Before a moved file's rule was asked under both its names, a scene renamed to a name no rule
/// covers, and edited on both lines, was merged line by line.
#[test]
fn a_moved_file_is_merged_line_by_line_only_when_its_rule_allows_it_under_both_names() {
    let mut h = Hist::new();
    let (s, o, t) = moved_and_edited(&mut h, "Main.unity", "Main.txt");
    let rules = scene_rule();
    let want = [
        "Main.txt: changed on both lines, and opalro.toml says conflict-markers = false \
                 for it",
    ];
    let [vs, vo, vt] = [s, o, t].map(|id| h.version(id));
    assert_eq!(h.outcome_with(vs, vo, vt, &rules).0, want);
    // The move on the other line.
    assert_eq!(h.outcome_with(vs, vt, vo, &rules).0, want);
    // Both lines moved it to one place.
    let both = h.save("side", s, &tree(&[("Main.txt", f(BODY))]));
    let both = h.save("side", both, &tree(&[("Main.txt", edit(4, "FIVE"))]));
    let vb = h.version(both);
    assert_eq!(h.outcome_with(vs, vo, vb, &rules).0, want);
    // Without the rule, it merges where it went.
    let merged = h.merged(s, o, t);
    assert_eq!(
        merged,
        tree(&[("Main.txt", f("ONE\ntwo\nthree\nfour\nFIVE\n"))])
    );
}

#[test]
fn a_file_moved_into_a_protected_name_is_not_merged() {
    let mut h = Hist::new();
    let (s, o, t) = moved_and_edited(&mut h, "Main.txt", "Main.unity");
    let [vs, vo, vt] = [s, o, t].map(|id| h.version(id));
    assert_eq!(
        h.outcome_with(vs, vo, vt, &scene_rule()).0,
        ["Main.unity: changed on both lines, and opalro.toml says conflict-markers = false for it"]
    );
}

#[test]
fn a_moved_file_whose_names_are_both_free_merges_as_before() {
    let mut h = Hist::new();
    let (s, o, t) = moved_and_edited(&mut h, "a.txt", "b.txt");
    let [vs, vo, vt] = [s, o, t].map(|id| h.version(id));
    let (conflicts, merged) = h.outcome_with(vs, vo, vt, &scene_rule());
    assert!(conflicts.is_empty(), "{conflicts:?}");
    assert_eq!(
        merged,
        tree(&[("b.txt", f("ONE\ntwo\nthree\nfour\nFIVE\n"))])
    );
}

/// A move found by contents leaves the file as it was at the start, so only the other line
/// changed it: nothing is merged line by line, and the rule has nothing to say.
#[test]
fn a_scene_moved_without_a_note_takes_the_other_lines_edit_where_it_went() {
    let mut h = Hist::new();
    let first = h.first;
    let none: &[(&str, &str)] = &[];
    let s = h.plant(&[first], &tree(&[("Main.unity", f(BODY))]), Some(none));
    let o = h.plant(&[s], &tree(&[("Main.txt", f(BODY))]), Some(none));
    let t = h.plant(&[s], &tree(&[("Main.unity", edit(4, "FIVE"))]), Some(none));
    let [vs, vo, vt] = [s, o, t].map(|id| h.version(id));
    let (conflicts, merged) = h.outcome_with(vs, vo, vt, &scene_rule());
    assert!(conflicts.is_empty(), "{conflicts:?}");
    assert_eq!(merged, tree(&[("Main.txt", edit(4, "FIVE"))]));
}

#[test]
fn a_combined_save_on_the_way_is_one_more_step() {
    let mut h = Hist::new();
    let s = player(&mut h);
    let to = "Characters/Player.cs";
    let m = h.save("main", s, &moved_to(to, f(BODY)));
    let mut extra = moved_to("Player.cs", f(BODY));
    extra.insert("Extra.cs".to_owned(), f("extra\n"));
    let e = h.save("extra", s, &extra);
    let mut both = moved_to(to, f(BODY));
    both.insert("Extra.cs".to_owned(), f("extra\n"));
    let c = h.plant(&[m, e], &both, None);
    both.insert(to.to_owned(), edit(0, "ONE"));
    let o = h.save("main", c, &both);
    let t = h.save("side", s, &moved_to("Player.cs", edit(4, "FIVE")));
    both.insert(to.to_owned(), f("ONE\ntwo\nthree\nfour\nFIVE\n"));
    assert_eq!(h.merged(s, o, t), with_folders(&both));
}

#[test]
fn a_start_reached_through_a_second_parent_is_followed_by_contents() {
    let mut h = Hist::new();
    let s = player(&mut h);
    let first = h.first;
    let start = tree(&[("Player.cs", f(BODY)), ("Other.cs", f("other\n"))]);
    let c = h.plant(&[first, s], &start, None);
    let to = "Characters/Player.cs";
    let t = h.save("side", s, &moved_to("Player.cs", edit(4, "FIVE")));
    // Moved and edited in one save: nothing notes it, and the contents differ.
    let o = h.save("main", c, &moved_to(to, edit(0, "ONE")));
    assert_eq!(
        h.conflicts(s, o, t),
        ["Player.cs: changed on side, removed on main"]
    );
    // Moved and edited apart: the notes aren't on the way back to the start.
    let o = h.save("main", c, &moved_to(to, f(BODY)));
    let edited = h.save("main", o, &moved_to(to, edit(0, "ONE")));
    assert_eq!(
        h.conflicts(s, edited, t),
        ["Player.cs: changed on side, removed on main"]
    );
    // Only moved: followed by its contents.
    assert_eq!(
        h.merged(s, o, t),
        with_folders(&moved_to(to, edit(4, "FIVE")))
    );
}

/// `Player.cs` moved to `Z.cs` on `main` and saved, then a new `Player.cs` made there.
fn moved_and_made_again(h: &mut Hist, s: Id) -> Id {
    let o = h.save("main", s, &moved_to("Z.cs", f(BODY)));
    let mut again = moved_to("Z.cs", f(BODY));
    again.insert("Player.cs".to_owned(), f("new\n"));
    h.save("main", o, &again)
}

#[test]
fn moved_away_and_made_again_against_a_move_elsewhere_conflicts() {
    let mut h = Hist::new();
    let s = player(&mut h);
    let o = moved_and_made_again(&mut h, s);
    let t = h.save("side", s, &moved_to("Y.cs", f(BODY)));
    assert_eq!(
        h.conflicts(s, o, t),
        [
            "Y.cs: moved here from Player.cs on side, and to Z.cs on main",
            "Z.cs: moved here from Player.cs on main, and to Y.cs on side",
        ]
    );
}

#[test]
fn a_side_whose_notes_moved_a_file_has_moved_it_though_it_made_a_new_one_there() {
    let mut h = Hist::new();
    let s = player(&mut h);
    let o = moved_and_made_again(&mut h, s);
    assert_eq!(
        h.followed(h.version(s), h.version(o)),
        pairs(&[("Player.cs", "Z.cs")])
    );
    let new = ("Player.cs", f("new\n"));
    // Each row of the table, the new file an addition where the moved one was.
    let t = h.save(
        "side",
        s,
        &tree(&[("Player.cs", f(BODY)), ("Other.cs", f("o2\n"))]),
    );
    let want = tree(&[("Z.cs", f(BODY)), new.clone(), ("Other.cs", f("o2\n"))]);
    assert_eq!(h.merged(s, o, t), want);
    let t = h.save("side", s, &moved_to("Player.cs", edit(1, "TWO")));
    let want = tree(&[
        ("Z.cs", edit(1, "TWO")),
        new.clone(),
        ("Other.cs", f("other\n")),
    ]);
    assert_eq!(h.merged(s, o, t), want);
    let t = h.save("side", s, &moved_to("Z.cs", f(BODY)));
    let want = tree(&[("Z.cs", f(BODY)), new.clone(), ("Other.cs", f("other\n"))]);
    assert_eq!(h.merged(s, o, t), want);
    let t = h.save("side", s, &tree(&[("Other.cs", f("other\n"))]));
    assert_eq!(
        h.conflicts(s, o, t),
        ["Z.cs: moved here from Player.cs on main, removed on side"]
    );
    let mut onto = moved_to("Player.cs", f(BODY));
    onto.insert("Z.cs".to_owned(), f("z\n"));
    let t = h.save("side", s, &onto);
    assert_eq!(
        h.conflicts(s, o, t),
        ["Z.cs: moved here from Player.cs on main, where side has something else"]
    );
    // Without its notes, a new file where one was is that file changed.
    let made_up = Version {
        save: None,
        ..h.version(o)
    };
    assert_eq!(h.followed(h.version(s), made_up), pairs(&[]));
    let t = h.save("side", s, &moved_to("Player.cs", edit(1, "TWO")));
    assert_eq!(
        h.outcome(h.version(s), made_up, h.version(t)).0,
        ["Player.cs: changed on both lines, in the same place"]
    );
}

#[test]
fn a_file_moved_in_by_notes_where_one_was_removed_counts_as_removed() {
    let mut h = Hist::new();
    let s = player(&mut h);
    let o = h.save("main", s, &moved_to("Z.cs", f(BODY)));
    // `Player.cs` removed, then `Other.cs` moved where it was, each saved alone.
    let t = h.save("side", s, &tree(&[("Other.cs", f("other\n"))]));
    let t = h.save("side", t, &tree(&[("Player.cs", f("other\n"))]));
    let side = followed(&h.repo, &mut Made::default(), &h.version(s), &h.version(t)).unwrap();
    assert!(side.went.is_empty(), "{side:?}");
    assert!(side.replaced.contains_key("Player.cs"), "{side:?}");
    assert_eq!(
        h.conflicts(s, o, t),
        ["Z.cs: moved here from Player.cs on main, removed on side"]
    );
    assert_eq!(
        h.conflicts(s, t, o),
        ["Z.cs: moved here from Player.cs on side, removed on main"]
    );
    // Though the side has something where the file went, too.
    let mut onto = tree(&[("Player.cs", f("other\n"))]);
    onto.insert("Z.cs".to_owned(), f("z\n"));
    let t = h.save("side", t, &onto);
    assert_eq!(
        h.conflicts(s, o, t),
        ["Z.cs: moved here from Player.cs on main, where side has something else"]
    );
}

/// On `line` after `s`, which holds `start`: `Player.cs` removed and saved, then `from` moved
/// where it was and saved, so that the second save notes it.
fn moved_in(h: &mut Hist, line: &str, s: Id, start: &Tree, from: &str) -> Id {
    let mut t = start.clone();
    t.remove("Player.cs");
    let gone = h.save(line, s, &t);
    let v = t.remove(from).unwrap();
    t.insert("Player.cs".to_owned(), v);
    h.save(line, gone, &t)
}

#[test]
fn a_file_moved_in_where_one_was_removed_is_a_new_file_for_the_whole_merge() {
    let mut h = Hist::new();
    let first = h.first;
    // Files alike but for a line or two, so that, merged line by line, any of them would take
    // an edit of another without a conflict.
    let start = tree(&[
        ("Player.cs", f(BODY)),
        ("Other.cs", f("ONE\ntwo\nthree\nfour\nFIVE\n")),
        ("Third.cs", edit(2, "THREE")),
    ]);
    let s = h.save("main", first, &start);
    let o = moved_in(&mut h, "main", s, &start, "Other.cs");
    let mut want = start.clone();
    want.remove("Other.cs");
    want.insert("Player.cs".to_owned(), start["Other.cs"].clone());
    let both_ways = |h: &mut Hist, t: Id| {
        [
            h.outcome(h.version(s), h.version(o), h.version(t)),
            h.outcome(h.version(s), h.version(t), h.version(o)),
        ]
    };
    // Changed on the other side: the file that came, moved onto that change.
    let mut edited = start.clone();
    edited.insert("Player.cs".to_owned(), edit(2, "3"));
    let t = h.save("side", s, &edited);
    let [a, b] = both_ways(&mut h, t);
    let onto = |on| {
        format!(
            "Player.cs: moved here from Other.cs on {on}, where {} has something else",
            if on == "main" { "side" } else { "main" }
        )
    };
    assert_eq!(a.0, [onto("main")]);
    assert_eq!(b.0, [onto("side")]);
    // Untouched there: the file that arrived stands.
    let mut untouched = start.clone();
    untouched.insert("Side.cs".to_owned(), f("side\n"));
    let t = h.save("side", s, &untouched);
    let mut got = want.clone();
    got.insert("Side.cs".to_owned(), f("side\n"));
    assert_eq!(both_ways(&mut h, t), [(vec![], got.clone()), (vec![], got)]);
    // Removed there too: it stands as well.
    let mut removed = start.clone();
    removed.remove("Player.cs");
    let t = h.save("side", s, &removed);
    assert_eq!(
        both_ways(&mut h, t),
        [(vec![], want.clone()), (vec![], want.clone())]
    );
    // Another file brought there: two new files.
    let t = moved_in(&mut h, "side", s, &start, "Third.cs");
    let [a, b] = both_ways(&mut h, t);
    assert_eq!(a.0, ["Player.cs: added on both lines, differently"]);
    assert_eq!(b.0, a.0);
    // The same file brought there: taken once.
    let t = moved_in(&mut h, "side", s, &start, "Other.cs");
    assert_eq!(
        both_ways(&mut h, t),
        [(vec![], want.clone()), (vec![], want)]
    );
}

#[test]
fn a_file_replaced_in_place_without_a_note_is_that_file_changed() {
    let mut h = Hist::new();
    let s = player(&mut h);
    let to = "Characters/Player.cs";
    let o = h.save("main", s, &moved_to(to, f(BODY)));
    // In one save, `Player.cs` given `Other.cs`'s contents and `Other.cs` removed: `Player.cs`
    // never emptied, so nothing notes a move, and it is an edit.
    let t = h.save("side", s, &tree(&[("Player.cs", f("other\n"))]));
    let want = with_folders(&tree(&[(to, f("other\n"))]));
    assert_eq!(h.merged(s, o, t), want);
    assert_eq!(h.merged(s, t, o), want);
}

#[test]
fn a_file_noted_as_gone_to_two_places_is_not_followed() {
    let mut h = Hist::new();
    let first = h.first;
    let none: &[(&str, &str)] = &[];
    let start = tree(&[("Player.cs", f(BODY)), ("Other.cs", f("other\n"))]);
    let s = h.plant(&[first], &start, Some(none));
    let t = h.plant(&[s], &moved_to("Player.cs", edit(1, "TWO")), Some(none));
    let two = tree(&[
        ("X.cs", f(BODY)),
        ("Y.cs", f(BODY)),
        ("Other.cs", f("other\n")),
    ]);
    let both: &[(&str, &str)] = &[("Player.cs", "X.cs"), ("Player.cs", "Y.cs")];
    let noted = h.plant(&[s], &two, Some(both));
    let plain = h.plant(&[s], &two, Some(none));
    assert_eq!(h.followed(h.version(s), h.version(noted)), pairs(&[]));
    let want = h.outcome(h.version(s), h.version(plain), h.version(t));
    assert_eq!(
        h.outcome(h.version(s), h.version(noted), h.version(t)),
        want
    );
    // Either note alone is true, and followed.
    let one = h.plant(&[s], &two, Some(&both[..1]));
    assert_eq!(
        h.followed(h.version(s), h.version(one)),
        pairs(&[("Player.cs", "X.cs")])
    );
}

#[test]
fn following_by_contents_holds_a_bounded_amount() {
    let mut h = Hist::new();
    let s = player(&mut h);
    let o = h.save("main", s, &moved_to("Characters/Player.cs", f(BODY)));
    let made_up = Version {
        save: None,
        ..h.version(o)
    };
    // `Player.cs` is the only file that left, with a path of 9 bytes, and it arrived at a path
    // of 20.
    let fits = 2 * HELD_PER_PLACE + 29;
    let at_most = |n: usize| {
        let mut made = Made {
            max_left_bytes: n,
            ..Made::default()
        };
        followed(&h.repo, &mut made, &h.version(s), &made_up).unwrap()
    };
    let past = at_most(fits - 1);
    assert!(past.went.is_empty() && past.too_many_left, "{past:?}");
    let within = at_most(fits);
    assert!(
        within.went.len() == 1 && !within.too_many_left,
        "{within:?}"
    );
    // The merge goes on without the move, and says so.
    let t = h.save("side", s, &moved_to("Player.cs", edit(1, "TWO")));
    let mut packs = h.repo.new_packs().unwrap();
    let mut made = Made {
        max_left_bytes: fits - 1,
        ..Made::default()
    };
    let rules = RuleMatcher::new(&RulesRead::default(), &RulesRead::default());
    let (vs, vt) = (h.version(s), h.version(t));
    let m = merge_three(
        &h.repo, &mut packs, &mut made, false, &vs, &made_up, &vt, &rules,
    )
    .unwrap();
    let got: Vec<String> = m
        .conflicts
        .iter()
        .map(|c| c.describe("main", "side"))
        .collect();
    assert_eq!(got, ["Player.cs: changed on side, removed on main"]);
    assert_eq!(m.unfollowed, [Side::Ours]);
}

/// A save's tree and the notes it carries.
type Step<'a> = (Tree, &'a [(&'a str, &'a str)]);

/// Plants a save after `s` for each step, noting its notes when `noted`; gives the last.
fn planted(h: &mut Hist, s: Id, steps: &[Step], noted: bool) -> Id {
    let none: &[(&str, &str)] = &[];
    steps.iter().fold(s, |at, (t, notes)| {
        h.plant(&[at], t, Some(if noted { notes } else { none }))
    })
}

#[test]
fn a_note_false_of_its_own_save_is_ignored() {
    let mut h = Hist::new();
    let first = h.first;
    let none: &[(&str, &str)] = &[];
    let other = ("Other.cs", f("other\n"));
    let empty = ("Empty", f(""));
    let start = tree(&[("Player.cs", f(BODY)), other.clone(), empty.clone()]);
    let s = h.plant(&[first], &start, Some(none));
    let theirs = tree(&[
        ("Player.cs", edit(4, "FIVE")),
        other.clone(),
        ("Empty", f("e\n")),
    ]);
    let t = h.plant(&[s], &theirs, Some(none));
    let at = |x: V| tree(&[("X.cs", x), other.clone(), empty.clone()]);
    let edited = (at(edit(0, "ONE")), none);
    let p_to_x: &[(&str, &str)] = &[("Player.cs", "X.cs")];
    let e_to_x: &[(&str, &str)] = &[("Empty", "X.cs")];
    let cases: Vec<(&str, Vec<Step>)> = vec![
        (
            "its file had left before",
            vec![
                (tree(&[other.clone(), empty.clone()]), none),
                (at(f(BODY)), p_to_x),
                edited.clone(),
            ],
        ),
        (
            "its file is still there",
            vec![
                (
                    tree(&[
                        ("Player.cs", f(BODY)),
                        ("X.cs", f(BODY)),
                        other.clone(),
                        empty.clone(),
                    ]),
                    p_to_x,
                ),
                edited.clone(),
            ],
        ),
        (
            "something was where it went",
            vec![
                (
                    tree(&[
                        ("Player.cs", f(BODY)),
                        ("X.cs", f("x\n")),
                        other.clone(),
                        empty.clone(),
                    ]),
                    none,
                ),
                (at(f(BODY)), p_to_x),
                edited.clone(),
            ],
        ),
        (
            "nothing arrived where it went",
            vec![
                (
                    tree(&[("Y.cs", f(BODY)), other.clone(), empty.clone()]),
                    p_to_x,
                ),
                edited.clone(),
            ],
        ),
        (
            "other contents arrived",
            vec![(at(f("different\n")), p_to_x)],
        ),
        (
            "an empty file",
            vec![(
                tree(&[("Player.cs", f(BODY)), other.clone(), ("X.cs", f(""))]),
                e_to_x,
            )],
        ),
    ];
    for (why, steps) in cases {
        let noted = planted(&mut h, s, &steps, true);
        let plain = planted(&mut h, s, &steps, false);
        assert_eq!(
            h.followed(h.version(s), h.version(noted)),
            pairs(&[]),
            "{why}"
        );
        let want = h.outcome(h.version(s), h.version(plain), h.version(t));
        let got = h.outcome(h.version(s), h.version(noted), h.version(t));
        assert_eq!(got, want, "{why}");
    }
    // A true note is followed, though the file was edited after.
    let o = planted(&mut h, s, &[(at(f(BODY)), p_to_x), edited], true);
    assert_eq!(
        h.followed(h.version(s), h.version(o)),
        pairs(&[("Player.cs", "X.cs")])
    );
    let want = tree(&[
        ("X.cs", f("ONE\ntwo\nthree\nfour\nFIVE\n")),
        other,
        ("Empty", f("e\n")),
    ]);
    assert_eq!(h.merged(s, o, t), want);
}

#[test]
fn a_file_added_in_a_moved_folder_stays_in_the_old_folder() {
    let mut h = Hist::new();
    let first = h.first;
    let s = h.save(
        "main",
        first,
        &tree(&[("old/a.txt", f(BODY)), ("old/b.txt", f("b\n"))]),
    );
    let o = h.save(
        "main",
        s,
        &tree(&[("new/a.txt", f(BODY)), ("new/b.txt", f("b\n"))]),
    );
    let t = h.save(
        "side",
        s,
        &tree(&[
            ("old/a.txt", edit(1, "TWO")),
            ("old/b.txt", f("b\n")),
            ("old/c.txt", f("c\n")),
        ]),
    );
    let want = tree(&[
        ("new/a.txt", edit(1, "TWO")),
        ("new/b.txt", f("b\n")),
        ("old/c.txt", f("c\n")),
    ]);
    assert_eq!(h.merged(s, o, t), with_folders(&want));
}

#[test]
fn a_note_that_doesnt_fit_is_ignored() {
    let mut h = Hist::new();
    let first = h.first;
    let none: &[(&str, &str)] = &[];
    let start = tree(&[("Player.cs", f(BODY)), ("Other.cs", f("other\n"))]);
    let s = h.plant(&[first], &start, Some(none));
    let t = h.plant(&[s], &moved_to("Player.cs", edit(1, "TWO")), Some(none));
    let gone = tree(&[("Other.cs", f("other\n"))]);
    let mut new = start.clone();
    new.insert("New.cs".to_owned(), f(BODY));
    let mut folder = gone.clone();
    folder.insert("Dir/x".to_owned(), f("x\n"));
    let cases: [(&Tree, (&str, &str)); 4] = [
        // From a path that held nothing.
        (&new, ("Nothing.cs", "New.cs")),
        // To a path holding a folder.
        (&folder, ("Player.cs", "Dir")),
        // To a path a save can't hold.
        (&gone, ("Player.cs", "../x")),
        // To a different file the start held.
        (&gone, ("Player.cs", "Other.cs")),
    ];
    for (ours, note) in cases {
        let noted = h.plant(&[s], ours, Some(&[note]));
        let plain = h.plant(&[s], ours, Some(none));
        assert_eq!(h.followed(h.version(s), h.version(noted)), pairs(&[]));
        let want = h.outcome(h.version(s), h.version(plain), h.version(t));
        let got = h.outcome(h.version(s), h.version(noted), h.version(t));
        assert_eq!(got, want, "{note:?}");
    }
}

#[test]
fn a_move_to_a_place_that_held_something_at_the_start_is_not_a_move() {
    let mut h = Hist::new();
    let s = player(&mut h);
    // `Player.cs` taken over `Other.cs`.
    let onto = tree(&[("Other.cs", f(BODY))]);
    let o = h.save("main", s, &onto);
    assert_eq!(h.followed(h.version(s), h.version(o)), pairs(&[]));
    // Nor when a note says so.
    let noted = h.plant(&[s], &onto, Some(&[("Player.cs", "Other.cs")]));
    assert_eq!(h.followed(h.version(s), h.version(noted)), pairs(&[]));
    // Emptied in one save and moved onto in the next: each save notes it truly.
    let emptied = h.save("main", s, &tree(&[("Player.cs", f(BODY))]));
    let later = h.save("main", emptied, &onto);
    assert_eq!(h.followed(h.version(s), h.version(later)), pairs(&[]));
    let t = h.save("side", s, &moved_to("Player.cs", edit(1, "TWO")));
    for o in [o, noted, later] {
        assert_eq!(
            h.conflicts(s, o, t),
            ["Player.cs: changed on side, removed on main"]
        );
    }
}

#[test]
fn a_made_up_version_follows_contents_only() {
    let mut h = Hist::new();
    let s = player(&mut h);
    let to = "Characters/Player.cs";
    let moved = h.save("main", s, &moved_to(to, f(BODY)));
    let o = h.save("main", moved, &moved_to(to, edit(0, "ONE")));
    let t = h.save("side", s, &moved_to("Player.cs", edit(4, "FIVE")));
    let made_up = |v: Version| Version { save: None, ..v };
    let (vs, vo, vt) = (h.version(s), h.version(o), h.version(t));
    assert_eq!(h.followed(vs, vo), pairs(&[("Player.cs", to)]));
    assert_eq!(h.followed(vs, made_up(vo)), pairs(&[]));
    assert_eq!(h.followed(made_up(vs), vo), pairs(&[]));
    let removed = ["Player.cs: changed on side, removed on main".to_owned()];
    assert_eq!(h.outcome(vs, made_up(vo), vt).0, removed);
    assert_eq!(h.outcome(made_up(vs), vo, vt).0, removed);
    // A move alone is still found by its contents.
    let vm = h.version(moved);
    assert_eq!(h.followed(vs, made_up(vm)), pairs(&[("Player.cs", to)]));
    assert_eq!(h.followed(made_up(vs), vm), pairs(&[("Player.cs", to)]));
}

#[test]
fn the_walk_for_notes_is_bounded() {
    let mut h = Hist::new();
    let lock = h.repo.lock_for_change("test", &mut |_| {}).unwrap();
    let first = h.repo.store().save(&h.first, false).unwrap();
    let mut packs = h.repo.new_packs().unwrap();
    let mut plant = |parents: Vec<Id>, n: usize| {
        let save = Save {
            parents,
            message: n.to_string(),
            ..first.clone()
        };
        packs.add_record(ObjectKind::Save, &save.encode()).unwrap()
    };
    // A thousand saves, then the start; then a side that has the start only through a second
    // parent, a thousand saves past it.
    let mut at = h.first;
    for n in 0..1_000 {
        at = plant(vec![at], n);
    }
    let start = plant(vec![at], 1_000);
    let mut tip = plant(vec![at, start], 1_001);
    for n in 1_002..2_001 {
        tip = plant(vec![tip], n);
    }
    h.repo.store_packs(&lock, packs).unwrap();
    drop(lock);
    let (vs, vt) = (h.version(start), h.version(tip));
    let before = h.repo.store().saves_read();
    only_in(&h.repo, tip, &[start]).unwrap();
    let counting = h.repo.store().saves_read() - before;
    let before = h.repo.store().saves_read();
    assert_eq!(h.followed(vs, vt), pairs(&[]));
    let read = h.repo.store().saves_read() - before;
    assert!(read <= counting + 1_002, "{read} > {counting} + 1,002");
}

/// `n` notes of files that were never there.
fn untrue_notes(n: usize, tag: &str) -> Vec<MoveNote> {
    (0..n)
        .map(|i| MoveNote {
            from: format!("{tag}-gone{i:04}"),
            to: format!("{tag}-here{i:04}"),
        })
        .collect()
}

/// A start with two files alike, `a` and `b`; a side that moves them to `c` and `d` in one save,
/// noted, then `c` on to `e` in another. Only the notes tell the files apart. Returns the start,
/// and the side's tip with the same moves unnoted.
fn moved_alike(h: &mut Hist) -> (Id, Id, Id) {
    let first = h.first;
    let none: &[(&str, &str)] = &[];
    let s = h.plant(
        &[first],
        &tree(&[("a", f(BODY)), ("b", f(BODY))]),
        Some(none),
    );
    let both = tree(&[("c", f(BODY)), ("d", f(BODY))]);
    let t1 = h.plant(&[s], &both, Some(&[("a", "c"), ("b", "d")][..]));
    let on = tree(&[("e", f(BODY)), ("d", f(BODY))]);
    let t2 = h.plant(&[t1], &on, Some(&[("c", "e")][..]));
    let unnoted = h.plant(&[s], &on, Some(none));
    (s, t2, unnoted)
}

/// Where `start`'s files went on `side`, checking notes against `made`, and whether notes were
/// believed.
fn followed_with(h: &Hist, made: &mut Made, start: Id, side: Id) -> (Vec<(String, String)>, bool) {
    let out = followed(&h.repo, made, &h.version(start), &h.version(side)).unwrap();
    (out.went.into_iter().collect(), out.notes_unbelieved)
}

#[test]
fn notes_within_what_a_combine_checks_are_followed() {
    let mut h = Hist::new();
    let (s, tip, unnoted) = moved_alike(&mut h);
    let both = pairs(&[("a", "e"), ("b", "d")]);
    // Three notes, three to check.
    let mut made = Made {
        notes_left: 3,
        ..Made::default()
    };
    assert_eq!(followed_with(&h, &mut made, s, tip), (both.clone(), false));
    assert_eq!((made.notes_left, made.notes_spent), (0, false));
    assert_eq!(h.followed(h.version(s), h.version(tip)), both);
    // Without notes, files alike can't be told apart.
    assert_eq!(h.followed(h.version(s), h.version(unnoted)), pairs(&[]));
}

#[test]
fn a_true_note_past_what_a_combine_checks_is_not_believed() {
    let mut h = Hist::new();
    let (s, tip, _) = moved_alike(&mut h);
    // The newest save's one note fits; the save before it holds two, past what is left, so the
    // chain isn't followed half-way: no note counts.
    let mut made = Made {
        notes_left: 2,
        ..Made::default()
    };
    assert_eq!(followed_with(&h, &mut made, s, tip), (pairs(&[]), true));
    assert_eq!((made.notes_left, made.notes_spent), (0, true));
    // Past what is left at the newest save, the save before it, of two notes, isn't read.
    let (vs, vt) = (h.version(s), h.version(tip));
    let mut made = Made {
        notes_left: 0,
        ..Made::default()
    };
    crate::save::NOTES_BUILT.set(0);
    assert!(
        followed(&h.repo, &mut made, &vs, &vt)
            .unwrap()
            .notes_unbelieved
    );
    assert_eq!(crate::save::NOTES_BUILT.get(), 1);
}

/// A start of two files alike and two others alike, and lines that each move one pair with true
/// notes: `main` `a` and `b`, `side` `p` and `q`. Only notes tell either pair apart.
fn both_lines_noted(h: &mut Hist) -> Tree {
    let first = h.first;
    let none: &[(&str, &str)] = &[];
    let (x, y) = (f(BODY), f("y\n"));
    let at =
        |names: [&str; 4]| tree(&names.map(|n| (n, if n < "m" { x.clone() } else { y.clone() })));
    let s = h.plant(&[first], &at(["a", "b", "p", "q"]), Some(none));
    let main_t = at(["c", "d", "p", "q"]);
    let main = h.plant(&[s], &main_t, Some(&[("a", "c"), ("b", "d")][..]));
    let side = h.plant(
        &[s],
        &at(["a", "b", "r", "t"]),
        Some(&[("p", "r"), ("q", "t")][..]),
    );
    h.set(|st| {
        st.lines.insert("main".into(), main);
        st.lines.insert("side".into(), side);
    });
    h.on("main", &main_t);
    at(["c", "d", "r", "t"])
}

#[test]
fn both_lines_notes_share_what_a_combine_checks() {
    for (most, told, checked) in [(4, 0, 4), (3, 1, 2)] {
        let mut h = Hist::new();
        let want = both_lines_noted(&mut h);
        NOTES_CHECKED.set(0);
        let bounds = Bounds {
            max_notes: most,
            ..bounds()
        };
        let done = combine_by(&mut h, "side", None, bounds).unwrap();
        let warned = done
            .applied
            .warnings
            .iter()
            .filter(|w| w.kind == warning::TOO_MANY_MOVE_NOTES)
            .count();
        // `main`'s two notes fit either way; `side`'s two fit only in four.
        assert_eq!((warned, NOTES_CHECKED.get()), (told, checked), "{most}");
        // Each pair, alike, is the same files whichever went where.
        assert_eq!(h.folder(), want);
    }
}

#[test]
fn notes_spent_while_combining_starting_points_are_told_once() {
    // Three starting points, each moving one file with a note: the first inner combine checks
    // two notes, the second one more.
    let base = tree(&[
        ("Player.cs", f(BODY)),
        ("Other.cs", f("other\n")),
        ("Third.cs", f("third\n")),
    ]);
    let moved = |name: &str| {
        let mut t = base.clone();
        let v = t.remove(name).unwrap();
        t.insert(format!("src/{name}"), v);
        t
    };
    let all = tree(&[
        ("src/Player.cs", f(BODY)),
        ("src/Other.cs", f("other\n")),
        ("src/Third.cs", f("third\n")),
    ]);
    for (most, told) in [(3, 0), (2, 1)] {
        let mut h = Hist::new();
        let starts = [moved("Player.cs"), moved("Other.cs"), moved("Third.cs")];
        let crossed = crossed(&mut h, &base, &starts, &all);
        tips(
            &mut h,
            crossed,
            &with(&all, &[("m.txt", f("m\n"))]),
            &with(&all, &[("s.txt", f("s\n"))]),
        );
        NOTES_CHECKED.set(0);
        let bounds = Bounds {
            max_notes: most,
            ..bounds()
        };
        let done = combine_by(&mut h, "side", None, bounds).unwrap();
        let Combined::Made { starts, .. } = &done.result else {
            panic!("{done:?}");
        };
        assert_eq!(starts.len(), 3);
        let warned = done
            .applied
            .warnings
            .iter()
            .filter(|w| w.kind == warning::TOO_MANY_MOVE_NOTES)
            .count();
        // The budget is one for both inner combines: past it in the second, told once.
        assert_eq!((warned, NOTES_CHECKED.get()), (told, most), "{most}");
        assert!(h.folder().contains_key("src/Third.cs"));
    }
}

#[test]
fn once_the_notes_are_spent_no_save_is_read_for_them_on_either_side() {
    let mut h = Hist::new();
    let (s, tip, _) = moved_alike(&mut h);
    let root = h.version(s).root;
    let many = h.plant_root(&[s], root, untrue_notes(10, "x"));
    let (vs, v_many, v_tip) = (h.version(s), h.version(many), h.version(tip));
    let mut made = Made {
        notes_left: 5,
        ..Made::default()
    };
    let out = followed(&h.repo, &mut made, &vs, &v_many).unwrap();
    assert!(out.notes_unbelieved && made.notes_spent);
    // The other side's notes, which alone would be followed, aren't read.
    let before = h.repo.store().saves_read();
    let other = followed(&h.repo, &mut made, &vs, &v_tip).unwrap();
    assert_eq!(h.repo.store().saves_read(), before);
    assert!(other.went.is_empty() && other.notes_unbelieved, "{other:?}");
}

#[test]
fn untrue_notes_past_what_a_combine_checks_are_told_once_and_moves_follow_contents() {
    let mut h = Hist::new();
    let s = player(&mut h);
    // `side` moves `Player.cs` unnoted, and then five saves each note ten files never there.
    let moved = moved_to("Characters/Player.cs", f(BODY));
    let none: &[(&str, &str)] = &[];
    let mut tip = h.plant(&[s], &moved, Some(none));
    let root = h.version(tip).root;
    for i in 0..5 {
        tip = h.plant_root(&[tip], root, untrue_notes(10, &i.to_string()));
    }
    h.save("main", s, &moved_to("Player.cs", edit(1, "TWO")));
    h.set(|st| {
        st.lines.insert("side".into(), tip);
    });
    NOTES_CHECKED.set(0);
    let tight = Bounds {
        max_notes: 25,
        ..bounds()
    };
    let done = combine_by(&mut h, "side", None, tight).unwrap();
    // Two saves' notes fit; the third's don't, and are never checked.
    assert_eq!(NOTES_CHECKED.get(), 20);
    let told: Vec<&str> = done
        .applied
        .warnings
        .iter()
        .filter(|w| w.kind == warning::TOO_MANY_MOVE_NOTES)
        .map(|w| w.message.as_str())
        .collect();
    assert_eq!(
        told,
        [
            "The saves being combined hold more move notes than Opalro checks in one combine \
             (1,000,000), so past that many it followed files that moved by their contents alone."
        ]
    );
    assert_eq!(h.folder(), moved_to("Characters/Player.cs", edit(1, "TWO")));
    assert_eq!(MAX_CHECKED_NOTES, 1_000_000);
}

#[test]
fn a_walk_over_saves_with_notes_builds_none() {
    let mut h = Hist::new();
    let (s, tip, unnoted) = moved_alike(&mut h);
    crate::save::NOTES_BUILT.set(0);
    assert_eq!(only_in(&h.repo, tip, &[unnoted]).unwrap(), 2);
    let starts = crate::ancestry::common_starts(&h.repo, &[tip], &[unnoted]).unwrap();
    assert_eq!(starts, crate::ancestry::Starts::Found(vec![s]));
    let mut ids = crate::changeids::ChangeIds::default();
    assert_eq!(ids.chain(&h.repo, Some("side"), tip).unwrap().len(), 4);
    assert_eq!(crate::save::NOTES_BUILT.get(), 0);
    // Reading a save whole builds them.
    h.save_of(tip);
    assert_eq!(crate::save::NOTES_BUILT.get(), 1);
}

/// One side's work on a start of files `s0` to `s5`. First up to 3 moves, each saved alone, by
/// the index of the file among those there and of where it goes: onto a start file still there
/// it replaces it unnoted, unless that file is removed in a save of its own first (`true`).
/// Then up to 2 changes saved together: an edit of a file's middle line (0), a removal (1), a new
/// file where a start file no longer is (2), a start file made a folder (3), a file `d` (4), or a
/// copy of the file at `t1` at `t0` or the other way round (5, drawn only on purpose).
/// The side's newest save is merged as a made-up version, followed by contents only, when
/// `made_up`.
#[derive(Debug, Clone)]
struct Work {
    moves: Vec<(usize, usize, bool)>,
    changes: Vec<(usize, u8)>,
    made_up: bool,
}

const MOVE_TO: [&str; 8] = ["t0", "t1", "t2", "d/t0", "d/t1", "d/t2", "s0", "s1"];

fn work() -> impl Strategy<Value = Work> {
    (
        prop::collection::vec((0..6usize, 0..MOVE_TO.len(), any::<bool>()), 0..=3),
        prop::collection::vec((0..6usize, 0..5u8), 0..=2),
        any::<bool>(),
    )
        .prop_map(|(moves, changes, made_up)| Work {
            moves,
            changes,
            made_up,
        })
}

/// Whether `path` can be made in `files`: nothing above it is a file, and nothing is below it.
fn can_make(files: &Tree, path: &str) -> bool {
    crate::scope::ancestors(path).all(|up| !files.contains_key(up))
        && !files.keys().any(|p| p.starts_with(&format!("{path}/")))
}

/// Does `work` on `line` from the save `s` holding `start`: the side's newest save, and what its
/// notes say, each start path to where its file is now (or was last noted).
fn do_work(
    h: &mut Hist,
    line: &str,
    s: Id,
    start: &Tree,
    work: &Work,
) -> (Id, BTreeMap<String, String>) {
    let mut files = start.clone();
    let mut tip = s;
    // The last noted place of each file whose moves were noted, to where it started.
    let mut came_from: BTreeMap<String, String> = BTreeMap::new();
    for &(pick, to, emptied_first) in &work.moves {
        let Some(from) = files.keys().nth(pick % files.len().max(1)).cloned() else {
            continue;
        };
        let to = MOVE_TO[to].to_owned();
        if from == to || !can_make(&files, &to) {
            continue;
        }
        let noted = match files.contains_key(&to) {
            false => true,
            true if !start.contains_key(&to) => continue,
            true if emptied_first => {
                files.remove(&to);
                tip = h.save(line, tip, &files);
                true
            }
            true => false,
        };
        let v = files.remove(&from).unwrap();
        files.insert(to.clone(), v);
        tip = h.save(line, tip, &files);
        if noted {
            let origin = came_from.remove(&from).unwrap_or(from);
            came_from.insert(to, origin);
        }
    }
    let before = files.clone();
    for &(pick, change) in &work.changes {
        let present: Vec<String> = files.keys().cloned().collect();
        let picked = |among: &[String]| among.get(pick % among.len().max(1)).cloned();
        match change {
            0 | 1 => {
                let Some(path) = picked(&present) else {
                    continue;
                };
                if change == 0 {
                    let Some(V::File(was, _)) = files.get(&path) else {
                        continue;
                    };
                    let mut lines: Vec<String> = String::from_utf8(was.clone())
                        .unwrap()
                        .lines()
                        .map(str::to_owned)
                        .collect();
                    let mark = format!("{path} on {line}");
                    match lines.get_mut(2) {
                        Some(middle) => *middle = mark,
                        None => lines = vec![mark],
                    }
                    files.insert(path.clone(), f(&(lines.join("\n") + "\n")));
                } else {
                    files.remove(&path);
                }
            }
            2 => {
                let gone: Vec<String> = start
                    .keys()
                    .filter(|p| !files.contains_key(*p) && can_make(&files, p))
                    .cloned()
                    .collect();
                if let Some(path) = picked(&gone) {
                    files.insert(path.clone(), f(&format!("new {path} on {line}\n")));
                }
            }
            3 => {
                let there: Vec<String> = start
                    .keys()
                    .filter(|p| files.contains_key(*p))
                    .cloned()
                    .collect();
                if let Some(path) = picked(&there) {
                    files.remove(&path);
                    files.insert(format!("{path}/in"), f(&format!("in {path} on {line}\n")));
                }
            }
            5 => {
                // A copy of the file at `t1` put at `t0`, or of the one at `t0` at `t1`.
                for (from, to) in [("t1", "t0"), ("t0", "t1")] {
                    if let Some(v) = files.get(from).cloned()
                        && !files.contains_key(to)
                        && can_make(&files, to)
                    {
                        files.insert(to.to_owned(), v);
                        break;
                    }
                }
            }
            _ => {
                if !files.contains_key("d") && can_make(&files, "d") {
                    files.insert("d".to_owned(), f(&format!("d on {line}\n")));
                }
            }
        }
    }
    if files != before {
        tip = h.save(line, tip, &files);
    }
    let noted = came_from
        .into_iter()
        .filter(|(now, origin)| now != origin)
        .map(|(now, origin)| (origin, now))
        .collect();
    (tip, noted)
}

/// Whether `t` holds nothing at `path` and nothing but folders above it.
fn room(t: &Tree, path: &str) -> bool {
    !t.contains_key(path)
        && crate::scope::ancestors(path).all(|up| t.get(up).is_none_or(|v| *v == V::Folder))
}

fn file_at(t: &Tree, path: &str) -> bool {
    matches!(t.get(path), Some(V::File(..)))
}

fn is_move_clash(c: &Clash) -> bool {
    matches!(
        c,
        Clash::MovedApart { .. } | Clash::MovedRemoved { .. } | Clash::MovedOnto { .. }
    )
}

/// The moves found by contents between `start` and `side`: a non-empty file that left a path,
/// arrived at one where the start held nothing, and is the only one with its contents that did.
fn by_contents(start: &Tree, side: &Tree) -> Vec<(String, String)> {
    let gone_from = |a: &Tree, b: &Tree| -> Vec<(Vec<u8>, String)> {
        a.iter()
            .filter(|(p, _)| !b.contains_key(*p))
            .filter_map(|(p, v)| match v {
                V::File(c, _) if !c.is_empty() => Some((c.clone(), p.clone())),
                _ => None,
            })
            .collect()
    };
    let (left, arrived) = (gone_from(start, side), gone_from(side, start));
    let only = |among: &[(Vec<u8>, String)], c: &[u8]| {
        let mut with = among.iter().filter(|(x, _)| x == c);
        match (with.next(), with.next()) {
            (Some((_, p)), None) => Some(p.clone()),
            _ => None,
        }
    };
    left.iter()
        .filter_map(|(c, a)| Some((only(&left, c)?, only(&arrived, c)?)).filter(|(x, _)| x == a))
        .collect()
}

/// Two lines that moved one file apart, `s<k>` to `t0` on ours and to `t1` on theirs, where one or
/// both of them also hold a copy of their moved file at the other's place: a place held alike.
fn held_alike() -> impl Strategy<Value = (usize, Work, Work)> {
    (1..=6usize, 0..6usize, 0..3u8).prop_map(|(n, k, which)| {
        let copy = |on: bool| if on { vec![(0, 5)] } else { vec![] };
        let ours = worked(&[(k, 0, false)], &copy(which != 1));
        let theirs = worked(&[(k, 1, false)], &copy(which != 0));
        (n, ours, theirs)
    })
}

/// The cases `following_moves_agrees_with_a_model` draws: two lines' work, and on purpose a file
/// that arrived where one was (one line empties `s0` or `s1` and moves another file there, noted,
/// while the other line edits the file that was there), and a place a file went held alike.
fn moves_cases() -> impl Strategy<Value = (usize, Work, Work)> {
    let arrival =
        (2..=6usize, 0..6usize, 6..8usize, any::<bool>()).prop_map(|(n, pick, to, swap)| {
            let arrived = worked(&[(pick, to, true)], &[]);
            let edited = worked(&[], &[(to - 6, 0)]);
            if swap {
                (n, edited, arrived)
            } else {
                (n, arrived, edited)
            }
        });
    prop_oneof![4 => (1..=6usize, work(), work()), 1 => arrival, 1 => held_alike()]
}

#[test]
fn following_moves_agrees_with_a_model() {
    let h = std::cell::RefCell::new(Hist::new());
    let kinds: std::cell::RefCell<BTreeMap<&str, usize>> = Default::default();
    let rng = TestRng::deterministic_rng(RngAlgorithm::ChaCha);
    let mut runner = TestRunner::new_with_rng(Config::with_cases(128), rng);
    runner
        .run(&moves_cases(), |(n, ours, theirs)| {
            let start = start_files(n);
            for (path, clash) in moves_case(&mut h.borrow_mut(), n, &ours, &theirs)? {
                let mut kinds = kinds.borrow_mut();
                *kinds.entry(clash_kind(&clash)).or_default() += 1;
                // A move onto a start file's place is a file that arrived where one was.
                if matches!(clash, Clash::MovedOnto { .. }) && start.contains_key(&path) {
                    *kinds.entry("arrived").or_default() += 1;
                }
            }
            Ok(())
        })
        .unwrap();
    // Each kind of conflict the moves bring is met, so the cases reach every rule.
    let kinds = kinds.into_inner();
    for kind in [
        "moved apart",
        "moved, removed",
        "moved onto",
        "changed, removed",
    ] {
        assert!(kinds.get(kind).is_some_and(|&n| n >= 5), "{kinds:?}");
    }
    assert!(kinds.get("arrived").is_some_and(|&n| n >= 3), "{kinds:?}");
    let alike = ALIKE_PLACES.with(std::cell::Cell::get);
    assert!(alike >= 3, "{alike} places held alike");
}

fn worked(moves: &[(usize, usize, bool)], changes: &[(usize, u8)]) -> Work {
    Work {
        moves: moves.to_vec(),
        changes: changes.to_vec(),
        made_up: false,
    }
}

// The start is `s0`, `s1` and `s2`. On `main`, `s1` is removed in a save of its own, then `s0`
// is moved onto its place, noted.

#[test]
fn a_file_that_arrived_and_was_then_made_a_folder_leaves_the_place_to_the_ordinary_rules() {
    // Then `s1` is made a folder holding `in`; `side` makes it a folder holding another `in`.
    let ours = worked(&[(0, 7, true)], &[(0, 3)]);
    let theirs = worked(&[], &[(4, 3)]);
    let got = moves_case(&mut Hist::new(), 3, &ours, &theirs).unwrap();
    let paths: Vec<&str> = got.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(paths, ["s1/in"]);
}

#[test]
fn a_file_that_arrived_and_was_then_removed_leaves_the_place_to_the_ordinary_rules() {
    // Then `s1` is removed; `side` edits `s1`.
    let ours = worked(&[(0, 7, true)], &[(0, 1)]);
    let theirs = worked(&[], &[(1, 0)]);
    let got = moves_case(&mut Hist::new(), 3, &ours, &theirs).unwrap();
    let removed_on = Side::Ours;
    assert_eq!(
        got,
        [("s1".to_owned(), Clash::ChangedRemoved { removed_on })]
    );
}

#[test]
fn a_file_that_arrived_and_was_then_moved_on_leaves_the_place_to_the_ordinary_rules() {
    // Then `s1` is moved on to `t0`; `side` edits `s1`. `s0` went to `t0` in the end.
    let ours = worked(&[(0, 7, true), (0, 0, false)], &[]);
    let theirs = worked(&[], &[(1, 0)]);
    let got = moves_case(&mut Hist::new(), 3, &ours, &theirs).unwrap();
    let removed_on = Side::Ours;
    assert_eq!(
        got,
        [("s1".to_owned(), Clash::ChangedRemoved { removed_on })]
    );
}

fn clash_kind(c: &Clash) -> &'static str {
    match c {
        Clash::MovedApart { .. } => "moved apart",
        Clash::MovedRemoved { .. } => "moved, removed",
        Clash::MovedOnto { .. } => "moved onto",
        Clash::ChangedRemoved { .. } => "changed, removed",
        _ => "other",
    }
}

/// `n` start files, alike but for their first and last lines, so that an edit of one merges
/// cleanly into another's contents.
fn start_files(n: usize) -> Tree {
    (0..n)
        .map(|i| {
            let text = format!("start {i}\nmid\nmid\nmid\nend {i}\n");
            (format!("s{i}"), f(&text))
        })
        .collect()
}

thread_local! {
    /// How many places a file went two ways that both lines hold alike `moves_case` has met.
    static ALIKE_PLACES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Builds a start of `n` files, does `ours` and `theirs` on two lines, and checks the merge that
/// follows moves against a model of it, without keeping conflicts and keeping them. Gives the
/// conflicts it names without keeping them.
fn moves_case(
    h: &mut Hist,
    n: usize,
    ours: &Work,
    theirs: &Work,
) -> Result<Vec<(String, Clash)>, TestCaseError> {
    let files = start_files(n);
    let first = h.first;
    let s = h.save("main", first, &files);
    let (o, mut noted_o) = do_work(h, "main", s, &files, ours);
    let (t, mut noted_t) = do_work(h, "side", s, &files, theirs);
    let [vs, mut vo, mut vt] = [s, o, t].map(|x| h.version(x));
    for (work, v, noted) in [
        (ours, &mut vo, &mut noted_o),
        (theirs, &mut vt, &mut noted_t),
    ] {
        if work.made_up {
            v.save = None;
            noted.clear();
        }
    }
    let store = h.repo.store();
    let read = |v: Version| read_tree(store, v.root, &HashMap::new());
    let three = [read(vs), read(vo), read(vt)];
    // What each side's notes say, then its contents, checked against the start and the
    // side.
    let went: [BTreeMap<String, String>; 2] = [(&noted_o, 1), (&noted_t, 2)].map(|(n, v)| {
        let fits =
            |a: &str, x: &str| file_at(&three[0], a) && room(&three[0], x) && file_at(&three[v], x);
        let mut went: BTreeMap<String, String> = n
            .iter()
            .filter(|(a, x)| fits(a, x))
            .map(|(a, x)| (a.clone(), x.clone()))
            .collect();
        let taken: BTreeSet<String> = went.values().cloned().collect();
        for (a, x) in by_contents(&three[0], &three[v]) {
            if !went.contains_key(&a) && !taken.contains(&x) && fits(&a, &x) {
                went.insert(a, x);
            }
        }
        went
    });
    // A place a note brought a file to, while a file is still there at the side's end: edits in
    // place keep it the same file.
    let replaced: [BTreeMap<&String, &String>; 2] = [(&noted_o, 1), (&noted_t, 2)].map(|(n, v)| {
        n.iter()
            .filter(|(_, x)| file_at(&three[v], x))
            .map(|(a, x)| (x, a))
            .collect()
    });
    let listed = |m: &BTreeMap<String, String>| m.clone().into_iter().collect::<Vec<_>>();
    prop_assert_eq!(h.followed(vs, vo), listed(&went[0]));
    prop_assert_eq!(h.followed(vs, vt), listed(&went[1]));
    // The table, applied to the three versions as stored: nothing a line holds is cleared.
    let mut was = three.clone();
    // Each place a moved file landed in two ways, with its clash and the start's path.
    let mut places: BTreeMap<String, (Clash, String)> = BTreeMap::new();
    // The name a clash of the tree merge takes where two added things meet.
    let mut renames: BTreeMap<String, Clash> = BTreeMap::new();
    let froms: BTreeSet<&String> = went[0].keys().chain(went[1].keys()).collect();
    for a in froms {
        let (moved_on, to, other) = match (went[0].get(a), went[1].get(a)) {
            (Some(x), Some(y)) if x == y => {
                let v = was[0].remove(a).unwrap();
                was[0].insert(x.clone(), v);
                continue;
            }
            (Some(x), Some(y)) => {
                for (moved_on, at, other_to) in [(Side::Ours, x, y), (Side::Theirs, y, x)] {
                    let clash = Clash::MovedApart {
                        from: a.clone(),
                        moved_on,
                        other_to: other_to.clone(),
                    };
                    places.insert(at.clone(), (clash, a.clone()));
                    let onto = Clash::MovedOnto {
                        moved_on,
                        from: a.clone(),
                    };
                    renames.entry(at.clone()).or_insert(onto);
                }
                // What a line holds where it was is new there, unless another followed file
                // was put there already.
                if was[0].get(a) == three[0].get(a) {
                    was[0].remove(a);
                }
                continue;
            }
            (Some(x), None) => (Side::Ours, x, 2),
            (None, Some(y)) => (Side::Theirs, y, 1),
            (None, None) => unreachable!(),
        };
        let held = if replaced[other - 1].contains_key(a) {
            None
        } else {
            three[other].get(a)
        };
        let to = to.clone();
        let from = a.clone();
        match held {
            None => {
                let onto = Clash::MovedOnto {
                    moved_on,
                    from: from.clone(),
                };
                renames.entry(to.clone()).or_insert(onto);
                places.insert(to, (Clash::MovedRemoved { moved_on, from }, a.clone()));
                if was[0].get(a) == three[0].get(a) {
                    was[0].remove(a);
                }
            }
            Some(v) if !matches!(v, V::File(..)) => {}
            Some(_) if !room(&three[other], &to) => {
                renames.insert(to, Clash::MovedOnto { moved_on, from });
            }
            Some(_) => {
                for v in [0, other] {
                    let moved = was[v].remove(a).unwrap();
                    was[v].insert(to.clone(), moved);
                }
            }
        }
    }
    // A start file where a side's notes brought another is gone on that side; where the other
    // side changed it in place, the two are set against each other whole, from the start's file.
    let mut whole_at: BTreeSet<String> = BTreeSet::new();
    let arrived: BTreeSet<&String> = replaced[0]
        .keys()
        .chain(replaced[1].keys())
        .copied()
        .collect();
    for at in arrived {
        let moved = went.iter().any(|w| w.contains_key(at));
        // Alike means alike all the way down, not just both folders.
        let alike = node(&three[1], at) == node(&three[2], at);
        if !file_at(&three[0], at) || moved || alike {
            continue;
        }
        let came = match [0, 1].map(|i| replaced[i].get(at)) {
            [Some(c), None] => Some((Side::Ours, 2, *c)),
            [None, Some(c)] => Some((Side::Theirs, 1, *c)),
            _ => None,
        };
        if let Some((moved_on, other, came)) = came {
            match three[other].get(at) {
                None => {}
                Some(v) if Some(v) == three[0].get(at) => continue,
                Some(_) => {
                    whole_at.insert(at.clone());
                    let from = came.clone();
                    renames.insert(at.clone(), Clash::MovedOnto { moved_on, from });
                }
            }
        }
        // The model's merge sees two new things there.
        was[0].remove(at);
    }
    // A folder these steps emptied is no version's own: only one a version held empty stays.
    for (v, before) in was.iter_mut().zip(&three) {
        let is_empty = |t: &Tree, p: &str| node(t, p) == Node::Folder(Tree::new());
        loop {
            let emptied: Vec<String> = (v.iter())
                .filter(|(p, x)| **x == V::Folder && is_empty(v, p) && !is_empty(before, p))
                .map(|(p, _)| p.clone())
                .collect();
            if emptied.is_empty() {
                break;
            }
            for p in emptied {
                v.remove(&p);
            }
        }
    }
    let mut out = Vec::new();
    for keep in [false, true] {
        let marks = keep.then(|| {
            let one = ChangeId([3; 16]);
            let [labels, nothing, unsure] = [
                Parted::One(one),
                Parted::Nothing(Some(one)),
                Parted::Unknown,
            ]
            .map(|p| sides().labels(p));
            [
                labels.ours,
                labels.base,
                nothing.base,
                unsure.base,
                labels.theirs,
            ]
        });
        let mut want = model(&was[0], &was[1], &was[2], marks, &Unknown::new());
        // Where a file came onto one the other line changed, the start's file is the middle.
        for at in &whole_at {
            if let Some(kept) = want.kept.get_mut(at) {
                kept.terms[1] = node(&three[0], at);
            }
        }
        // Where two added things clash, the tree merge's conflict is named by what brought them.
        let mut want_clash: BTreeMap<String, Clash> = BTreeMap::new();
        for (at, clash) in &renames {
            let both_added = node(&was[0], at) == Node::Missing
                && matches!(node(&was[1], at), Node::Leaf(_))
                && matches!(node(&was[2], at), Node::Leaf(_));
            if both_added && want.conflicts.contains(at) {
                want_clash.insert(at.clone(), clash.clone());
            }
        }
        // A move clash's place is named, and kept, only where the merge left a plain entry
        // under plain folders: a conflict at it or above it holds what each line has there.
        for (at, (clash, from)) in &places {
            // Both lines hold the very same file there: nothing to settle, it is taken.
            if node(&three[1], at) == node(&three[2], at) {
                if !keep {
                    ALIKE_PLACES.with(|n| n.set(n.get() + 1));
                }
                continue;
            }
            let plain = |p: &str| want.files.get(p).is_some_and(|v| *v == V::Folder);
            let open = crate::scope::ancestors(at).all(plain)
                && want.files.get(at).is_some_and(|v| *v != KEPT);
            if !open {
                continue;
            }
            want.conflicts.insert(at.clone());
            want_clash.insert(at.clone(), clash.clone());
            if keep {
                let whole = |n: Node| match n {
                    Node::Folder(below) => Node::Folder(with_folders(&below)),
                    n => n,
                };
                let terms = [
                    whole(node(&three[1], at)),
                    whole(node(&three[0], from)),
                    whole(node(&three[2], at)),
                ];
                let shown = if terms[0] == Node::Missing {
                    Shown::Nothing
                } else {
                    Shown::Yours
                };
                want.files.insert(at.clone(), KEPT);
                want.kept.insert(at.clone(), Want { terms, shown });
            }
        }
        let lock = h.repo.lock_for_change("test", &mut |_| {}).unwrap();
        let mut packs = h.repo.new_packs().unwrap();
        let mut made = Made {
            keep: keep.then(sides),
            ..Made::default()
        };
        let rules = RuleMatcher::new(&RulesRead::default(), &RulesRead::default());
        let m = merge_three(&h.repo, &mut packs, &mut made, false, &vs, &vo, &vt, &rules).unwrap();
        let got: BTreeSet<String> = m.conflicts.iter().map(|c| c.path.clone()).collect();
        prop_assert_eq!(&got, &want.conflicts);
        for c in &m.conflicts {
            match want_clash.get(&c.path) {
                Some(clash) => prop_assert_eq!(&c.clash, clash),
                None => prop_assert!(!is_move_clash(&c.clash), "{c:?}"),
            }
            prop_assert_eq!(c.stands.is_some(), keep);
        }
        if got.is_empty() || keep {
            // Texts merged line by line are only in the pack set until it is stored.
            h.repo.store_packs(&lock, packs).unwrap();
            let store = h.repo.store();
            let mut merged = read_tree(store, m.root, &made.listings);
            for (path, kept) in &want.kept {
                let Some(V::Conflict(id)) = merged.insert(path.clone(), KEPT) else {
                    return Err(TestCaseError::fail(format!("no conflict at {path}")));
                };
                let r = store.conflict(&id).unwrap();
                let terms: Vec<Node> = r.terms.iter().map(|t| term_node(store, *t)).collect();
                prop_assert_eq!(&terms, &kept.terms, "{}", path);
                let own = r
                    .own
                    .map(|own| (own.marker, own.version.map(|v| leaf(store, v.kind, v.id))));
                let shown = match own {
                    Some((7, Some(v))) => Shown::Marked(v),
                    None if r.mine().is_some() => Shown::Yours,
                    None => Shown::Nothing,
                    other => return Err(TestCaseError::fail(format!("{path}: {other:?}"))),
                };
                // The model marks whole files; where the marks fall is the text merge's to test.
                let kind = std::mem::discriminant::<Shown>;
                prop_assert_eq!(kind(&shown), kind(&kept.shown), "{}", path);
            }
            prop_assert_eq!(merged, want.files);
        }
        if !keep {
            out = m.conflicts.into_iter().map(|c| (c.path, c.clash)).collect();
        }
    }
    Ok(out)
}

fn bounds() -> Bounds {
    Bounds {
        max_inner: MAX_INNER,
        max_held: MAX_HELD_TEXT,
        deadline: DEADLINE,
        max_left_bytes: MAX_LEFT_BYTES,
        max_notes: MAX_CHECKED_NOTES,
        limits: SaveLimits::default(),
    }
}

/// Combines `target`, named as typed, into the line the project is on.
fn combine_in(h: &mut Hist, target: &str) -> Result<CombineOutcome, CombineError> {
    combine_by(h, target, None, bounds())
}

fn combine_by(
    h: &mut Hist,
    target: &str,
    message: Option<&str>,
    bounds: Bounds,
) -> Result<CombineOutcome, CombineError> {
    let to = crate::lines::resolve_target(&h.repo, target).unwrap();
    let named = Named {
        raw: target.to_owned(),
        shown: escape(target, false),
    };
    let who = ada();
    let asked = Asked {
        target: &to,
        named: &named,
        message,
        who: &who,
        git_excludes: None,
        keep: h.keep,
    };
    combine_with(&mut h.repo, &asked, &mut |_| {}, bounds)
}

impl Hist {
    /// Puts the project on `line`, its folder holding `t`.
    fn on(&mut self, line: &str, t: &Tree) {
        self.set(|s| s.position = Position::OnLine(line.to_owned()));
        self.write(t);
    }

    fn entry(&self) -> (Id, DiaryEntry) {
        self.repo.current().unwrap()
    }

    fn save_of(&self, id: Id) -> Save {
        self.repo.store().save(&id, false).unwrap()
    }

    /// The files of the folder.
    fn folder(&self) -> Tree {
        let mut t = Tree::new();
        let mut todo = vec![self.dir.path().to_path_buf()];
        while let Some(at) = todo.pop() {
            for e in fs::read_dir(&at).unwrap() {
                let e = e.unwrap();
                if e.file_name() == ".opalro" {
                    continue;
                }
                if e.file_type().unwrap().is_dir() {
                    todo.push(e.path());
                    continue;
                }
                let path = e.path();
                let rel = path.strip_prefix(self.dir.path()).unwrap();
                let rel = rel.to_string_lossy().replace('\\', "/");
                t.insert(rel, V::File(fs::read(&path).unwrap(), false));
            }
        }
        t
    }

    fn tmp_files(&self) -> Vec<std::ffi::OsString> {
        let tmp = self.dir.path().join(".opalro").join("tmp");
        fs::read_dir(tmp)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect()
    }

    /// Everything a refused combine leaves as it was.
    fn untouched(&self) -> Untouched {
        let state_dir = self.dir.path().join(".opalro").join("state");
        let mut state_files: Vec<(std::ffi::OsString, Vec<u8>)> = fs::read_dir(state_dir)
            .unwrap()
            .map(|e| e.unwrap())
            // The lock says who last took it.
            .filter(|e| e.file_type().unwrap().is_file() && e.file_name() != "lock-info")
            .map(|e| (e.file_name(), fs::read(e.path()).unwrap()))
            .collect();
        state_files.sort();
        let (head, entry) = self.entry();
        Untouched {
            head,
            state: entry.state,
            folder: self.folder(),
            packs: self.repo.store().pack_count(),
            state_files,
            tmp: self.tmp_files(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Untouched {
    head: Id,
    state: State,
    folder: Tree,
    packs: usize,
    state_files: Vec<(std::ffi::OsString, Vec<u8>)>,
    tmp: Vec<std::ffi::OsString>,
}

/// Checks that `h` is as `before`, with nothing left in `tmp`.
fn assert_untouched(h: &Hist, before: &Untouched) {
    let now = h.untouched();
    assert_eq!(&now, before);
    assert!(now.tmp.is_empty(), "{:?}", now.tmp);
}

fn base_tree() -> Tree {
    tree(&[("a.txt", f(BODY)), ("b.txt", f("b\n"))])
}

/// `main` and `side` each changed one file since `base`, `side`'s first; the project is on
/// `main`. Gives `base`, `main` and `side`.
fn diverged(h: &mut Hist) -> (Id, Id, Id) {
    let first = h.first;
    let base = h.save("main", first, &base_tree());
    let side = h.save("side", base, &side_tree());
    let main = h.save("main", base, &main_tree());
    (base, main, side)
}

fn side_tree() -> Tree {
    tree(&[("a.txt", edit(0, "side")), ("b.txt", f("b\n"))])
}

fn main_tree() -> Tree {
    tree(&[("a.txt", f(BODY)), ("b.txt", f("main b\n"))])
}

fn both_tree() -> Tree {
    tree(&[("a.txt", edit(0, "side")), ("b.txt", f("main b\n"))])
}

#[test]
fn nothing_to_combine_makes_no_step() {
    let mut h = Hist::new();
    let first = h.first;
    let base = h.save("main", first, &base_tree());
    h.set(|s| {
        s.lines.insert("side".into(), base);
    });
    let main = h.save("main", base, &main_tree());
    let (head, _) = h.entry();
    for target in ["side", "main"] {
        let done = combine_in(&mut h, target).unwrap();
        assert_eq!(done.result, Combined::Nothing);
        assert_eq!((done.save, done.line.as_str()), (main, "main"));
        assert_eq!(done.applied, Applied::default());
        assert_eq!(h.entry().0, head);
    }
    assert_eq!(combine_in(&mut h, "side").unwrap().with, base);
}

#[test]
fn a_line_that_hasnt_moved_is_moved_forward() {
    let mut h = Hist::new();
    let first = h.first;
    let base = h.save("main", first, &base_tree());
    let side = h.save("side", base, &side_tree());
    h.on("main", &base_tree());
    let (head, _) = h.entry();
    let done = combine_in(&mut h, "side").unwrap();
    assert_eq!(done.result, Combined::Forward);
    assert_eq!((done.save, done.with), (side, side));
    let (_, entry) = h.entry();
    assert_eq!(entry.state.lines["main"], side);
    assert_eq!(entry.state.position, Position::OnLine("main".into()));
    assert_eq!(entry.parent, Some(head));
    assert_eq!(entry.command, "combine");
    assert!(entry.folder_change.is_some());
    assert_eq!(h.folder(), side_tree());
}

#[test]
fn a_combined_save_has_both_parents_and_says_what_it_combined() {
    let mut h = Hist::new();
    let (base, main, side) = diverged(&mut h);
    let (head, _) = h.entry();
    let done = combine_in(&mut h, "side").unwrap();
    assert_eq!(
        done.result,
        Combined::Made {
            starts: vec![base],
            merged_files: vec![],
            conflicts: vec![],
            more: 0,
            conflict_count: 0,
        }
    );
    assert_eq!(done.with, side);
    let save = h.save_of(done.save);
    assert_eq!(save.parents, [main, side]);
    assert_eq!(save.message, "Combine side into main");
    assert_eq!(save.line(), Some("main"));
    assert_eq!((&save.author, &save.saver), (&ada(), &ada()));
    for other in [h.first, base, main, side] {
        assert_ne!(save.change_id, h.save_of(other).change_id);
    }
    assert!(save.move_notes.is_empty());
    let (_, entry) = h.entry();
    assert_eq!(entry.state.lines["main"], done.save);
    assert_eq!(entry.parent, Some(head));
    assert_eq!(entry.command, "combine");
    assert_eq!(h.folder(), both_tree());
    h.assert_as_goto_gives();
}

#[test]
fn a_combined_save_notes_the_moves_it_brought_in() {
    let mut h = Hist::new();
    let s = player(&mut h);
    h.save("side", s, &moved_to("src/Player.cs", f(BODY)));
    let main_t = tree(&[("Player.cs", f(BODY)), ("Other.cs", f("other, changed\n"))]);
    h.save("main", s, &main_t);
    let done = combine_in(&mut h, "side").unwrap();
    let save = h.save_of(done.save);
    let note = MoveNote {
        from: "Player.cs".into(),
        to: "src/Player.cs".into(),
    };
    assert_eq!(save.move_notes, [note]);
    let want = tree(&[
        ("src/Player.cs", f(BODY)),
        ("Other.cs", f("other, changed\n")),
    ]);
    assert_eq!(h.folder(), want);
    h.assert_as_goto_gives();
}

impl Hist {
    /// Every file and folder of the project's folder, as `read_tree` gives a save's.
    fn disk(&self) -> Tree {
        let mut t = self.folder();
        let mut todo = vec![self.dir.path().to_path_buf()];
        while let Some(at) = todo.pop() {
            for e in fs::read_dir(&at).unwrap() {
                let e = e.unwrap();
                if e.file_name() != ".opalro" && e.file_type().unwrap().is_dir() {
                    let path = e.path();
                    let rel = path.strip_prefix(self.dir.path()).unwrap();
                    t.insert(rel.to_string_lossy().replace('\\', "/"), V::Folder);
                    todo.push(path);
                }
            }
        }
        t
    }

    /// Checks that the folder is what `goto` to `main`'s save writes into a clean folder.
    fn assert_as_goto_gives(&mut self) {
        use crate::lines::Target;
        use crate::switch::goto;
        let now = self.disk();
        let first = self.first;
        goto(
            &mut self.repo,
            &Target::Save(first),
            &ada(),
            "goto",
            None,
            &mut |_| {},
        )
        .unwrap();
        self.write(&Tree::new());
        let main = Target::Line("main".into());
        goto(&mut self.repo, &main, &ada(), "goto", None, &mut |_| {}).unwrap();
        assert_eq!(self.disk(), now);
    }
}

/// `base`, then `side` and `main` made from it, saved from the folder (with notes) or planted
/// without notes; `side` combined into `main`. Gives the combined save's files and folders, and
/// checks the folder holds the same.
fn combined_tree(h: &mut Hist, base: &Tree, side: &Tree, main: &Tree, noted: bool) -> Tree {
    let first = h.first;
    if noted {
        let b = h.save("main", first, base);
        h.save("side", b, side);
        h.save("main", b, main);
    } else {
        let b = h.plant(&[first], base, Some(&[]));
        let t = h.plant(&[b], side, Some(&[]));
        let o = h.plant(&[b], main, Some(&[]));
        h.set(|s| {
            s.lines.insert("side".into(), t);
            s.lines.insert("main".into(), o);
        });
        h.on("main", main);
    }
    let done = combine_in(h, "side").unwrap();
    assert!(matches!(done.result, Combined::Made { .. }));
    let saved = read_tree(h.repo.store(), h.save_of(done.save).root, &HashMap::new());
    assert_eq!(h.disk(), saved);
    h.assert_as_goto_gives();
    saved
}

#[test]
fn a_folder_both_lines_moved_the_files_out_of_is_gone() {
    // `side` moves `doc/g.txt` out, `main` moves `doc/b.txt` out.
    let base = tree(&[("doc/b.txt", f("b\n")), ("doc/g.txt", f("g\n"))]);
    let side = tree(&[("doc/b.txt", f("b\n")), ("a.txt", f("g\n"))]);
    let main = tree(&[("e.txt", f("b\n")), ("doc/g.txt", f("g\n"))]);
    let want = tree(&[("a.txt", f("g\n")), ("e.txt", f("b\n"))]);
    for noted in [true, false] {
        let mut h = Hist::new();
        assert_eq!(combined_tree(&mut h, &base, &side, &main, noted), want);
    }
}

#[test]
fn a_folder_two_levels_deep_emptied_by_moves_is_gone() {
    let base = tree(&[("a/b/x.txt", f("x\n")), ("a/b/y.txt", f("y\n"))]);
    let side = tree(&[("x.txt", f("x\n")), ("a/b/y.txt", f("y\n"))]);
    let main = tree(&[("a/b/x.txt", f("x\n")), ("y.txt", f("y\n"))]);
    let want = tree(&[("x.txt", f("x\n")), ("y.txt", f("y\n"))]);
    let mut h = Hist::new();
    assert_eq!(combined_tree(&mut h, &base, &side, &main, true), want);
}

#[test]
fn a_folder_left_holding_a_file_after_the_moves_stays() {
    let keep = ("doc/keep.txt", f("keep\n"));
    let base = tree(&[
        ("doc/b.txt", f("b\n")),
        ("doc/g.txt", f("g\n")),
        keep.clone(),
    ]);
    let side = tree(&[("doc/b.txt", f("b\n")), ("a.txt", f("g\n")), keep.clone()]);
    let main = tree(&[("e.txt", f("b\n")), ("doc/g.txt", f("g\n")), keep.clone()]);
    let want = with_folders(&tree(&[("a.txt", f("g\n")), ("e.txt", f("b\n")), keep]));
    let mut h = Hist::new();
    assert_eq!(combined_tree(&mut h, &base, &side, &main, true), want);
}

#[test]
fn a_folder_the_other_line_put_a_new_file_in_stays() {
    let new = ("doc/new.txt", f("new\n"));
    let base = tree(&[("doc/b.txt", f("b\n")), ("doc/g.txt", f("g\n"))]);
    let side = tree(&[("doc/b.txt", f("b\n")), ("a.txt", f("g\n")), new.clone()]);
    let main = tree(&[("e.txt", f("b\n")), ("doc/g.txt", f("g\n"))]);
    let want = with_folders(&tree(&[("a.txt", f("g\n")), ("e.txt", f("b\n")), new]));
    let mut h = Hist::new();
    assert_eq!(combined_tree(&mut h, &base, &side, &main, true), want);
}

#[test]
fn a_message_given_is_the_saves() {
    let mut h = Hist::new();
    diverged(&mut h);
    let done = combine_by(&mut h, "side", Some("Bring in side"), bounds()).unwrap();
    assert_eq!(h.save_of(done.save).message, "Bring in side");
}

#[test]
fn at_a_save_combine_is_refused() {
    let mut h = Hist::new();
    let (base, ..) = diverged(&mut h);
    h.set(|s| s.position = Position::AtSave(base));
    let before = h.untouched();
    let err = combine_in(&mut h, "side").unwrap_err();
    assert!(matches!(err, CombineError::NotOnALine), "{err:?}");
    assert_eq!(
        err.to_string(),
        "You're at a save, not on a line, so there is no line to combine into."
    );
    assert_eq!(
        err.hint().unwrap(),
        "Go to a line first (opalro goto <line>), or start one here: opalro line new <name>"
    );
    assert_untouched(&h, &before);
}

#[test]
fn unrelated_histories_are_refused() {
    let mut h = Hist::new();
    let first = h.first;
    h.save("main", first, &base_tree());
    let alone = h.plant(&[], &tree(&[("z.txt", f("z\n"))]), Some(&[]));
    h.set(|s| {
        s.lines.insert("side".into(), alone);
    });
    let before = h.untouched();
    let err = combine_in(&mut h, "side").unwrap_err();
    assert!(matches!(err, CombineError::Unrelated { .. }), "{err:?}");
    assert_eq!(
        err.to_string(),
        "main and side have no save in common, so there is no starting point to combine them \
         from, and nothing was changed."
    );
    assert_eq!(
        err.hint().unwrap(),
        "Opalro combines only lines that share some history."
    );
    assert_untouched(&h, &before);
}

/// `main` and `side` changed the same line of `a.txt`, and `b.txt` far apart, so the merge
/// builds something before it stops; the project is on `main`.
fn in_conflict(h: &mut Hist) {
    let first = h.first;
    let base = h.save(
        "main",
        first,
        &tree(&[("a.txt", f(BODY)), ("b.txt", f(BODY))]),
    );
    let b = |n: usize, to: &str| ("b.txt", edit(n, to));
    let side = tree(&[("a.txt", edit(2, "side")), b(0, "side")]);
    let main = tree(&[("a.txt", edit(2, "main")), b(4, "main")]);
    h.save("side", base, &side);
    h.save("main", base, &main);
}

#[test]
fn a_conflict_stops_and_stores_nothing() {
    let mut h = Hist::new();
    // Milestone 8's combine, which stops at a conflict.
    h.keep = Keep::OnlyIfClean;
    in_conflict(&mut h);
    let before = h.untouched();
    let err = combine_in(&mut h, "side").unwrap_err();
    assert!(matches!(err, CombineError::Conflicts { .. }), "{err:?}");
    assert_eq!(
        err.to_string(),
        "Combining side into main would leave 1 place in conflict, so nothing was \
         changed (--only-if-clean):\n  a.txt: changed on both lines, in the same place"
    );
    assert_eq!(err.hint().unwrap(), CONFLICTS_HINT);
    assert_untouched(&h, &before);
}

#[test]
fn a_refused_combine_leaves_nothing_in_tmp() {
    let mut h = Hist::new();
    // Milestone 8's combine, which stops at a conflict.
    h.keep = Keep::OnlyIfClean;
    in_conflict(&mut h);
    assert!(combine_in(&mut h, "side").is_err());
    assert_eq!(h.tmp_files(), Vec::<std::ffi::OsString>::new());
}

#[test]
fn the_words_name_the_line_and_the_target() {
    let names = || Names {
        line: "main".into(),
        target: "\"v\\x1b1\"".into(),
    };
    let clash = |folder: &str| CombineError::NamesClash {
        names: names(),
        folder: folder.into(),
        problem: Box::new(PortabilityProblem::ReservedName("con".into())),
    };
    assert_eq!(
        clash("").to_string(),
        "Combining \"v\\x1b1\" into main would put two names in one folder that clash, so \
         nothing was changed:\n  in the top folder: \"con\" is a reserved device name on Windows"
    );
    assert!(clash("a\u{1b}b").to_string().contains("\n  in a\\x1bb/: "));
    let past = |which| {
        let detail = "its folders nest more than 4096 levels deep".to_owned();
        let names = names();
        CombineError::Past {
            names,
            which,
            detail,
        }
        .to_string()
    };
    let rest = " is past Opalro's limits for a save (its folders nest more than 4096 levels \
                deep), so nothing was changed.";
    let start = "The save where main and \"v\\x1b1\" parted";
    assert_eq!(past(Which::Start), format!("{start}{rest}"));
    assert_eq!(past(Which::Ours), format!("Line main's newest save{rest}"));
    assert_eq!(past(Which::Theirs), format!("\"v\\x1b1\"{rest}"));
}

#[test]
fn many_conflicts_are_counted_after_the_first_100() {
    let names = Names {
        line: "main".into(),
        target: "side".into(),
    };
    let conflict = |path: &str| Conflict {
        path: path.into(),
        clash: Clash::BothAdded,
        stands: None,
    };
    let err = CombineError::Conflicts {
        names,
        conflicts: vec![conflict("a"), conflict("b")],
        more: 1_234,
        rules_not_whole: None,
        why: NotKept::OnlyIfClean,
        held: 0,
        forward: false,
    };
    assert_eq!(
        err.to_string(),
        "Combining side into main would leave 1,236 places in conflict, so nothing was \
         changed (--only-if-clean):\n  a: added on both lines, differently\n  b: added on both \
         lines, differently\n  … and 1,234 more"
    );
}

#[test]
fn only_if_clean_counts_the_places_the_lines_hold_already() {
    let names = Names {
        line: "main".into(),
        target: "side".into(),
    };
    let err = |conflicts: Vec<Conflict>, held, forward| CombineError::Conflicts {
        names: names.clone(),
        conflicts,
        more: 0,
        rules_not_whole: None,
        why: NotKept::OnlyIfClean,
        held,
        forward,
    };
    let made = Conflict {
        path: "b.txt".into(),
        clash: Clash::BothAdded,
        stands: None,
    };
    assert_eq!(
        err(vec![made], 1_000, false).to_string(),
        "Combining side into main would leave 1,001 places in conflict, so nothing was changed \
         (--only-if-clean):\n  b.txt: added on both lines, differently\nThe two lines hold \
         1,000 more in conflict already."
    );
    assert_eq!(
        err(vec![], 1, false).to_string(),
        "Combining side into main would leave 1 place in conflict, so nothing was changed \
         (--only-if-clean).\nThe two lines hold it in conflict already."
    );
    assert_eq!(
        err(vec![], 2, true).to_string(),
        "Combining side into main would leave 2 places in conflict, so nothing was changed \
         (--only-if-clean).\nside holds them in conflict already."
    );
    let hint = "Run it again without --only-if-clean to keep the conflicts and fix them one at a \
                time.";
    assert_eq!(err(vec![], 2, true).hint().unwrap(), hint);
}

/// `main` at a save, `side` one save ahead holding a planted conflict at `x.txt`; the project
/// is on `main`.
fn ahead_onto_a_conflict(h: &mut Hist) {
    let c = plant_record(h, [b"mine\n", b"was\n", b"theirs\n"]);
    let first = h.first;
    let base = tree(&[("a.txt", f("a\n"))]);
    let b = h.plant(&[first], &base, Some(&[]));
    let t = h.plant(&[b], &with(&base, &[("x.txt", V::Conflict(c))]), Some(&[]));
    h.set(|s| {
        s.lines.insert("side".into(), t);
        s.lines.insert("main".into(), b);
    });
    h.on("main", &base);
}

#[test]
fn only_if_clean_stops_a_forward_onto_a_save_that_holds_a_conflict() {
    let mut h = Hist::new();
    h.keep = Keep::OnlyIfClean;
    ahead_onto_a_conflict(&mut h);
    let before = h.untouched();
    let err = combine_in(&mut h, "side").unwrap_err();
    assert!(
        matches!(
            err,
            CombineError::Conflicts {
                held: 1,
                forward: true,
                why: NotKept::OnlyIfClean,
                ..
            }
        ),
        "{err:?}"
    );
    assert_untouched(&h, &before);
    // Kept, the line moves, and the switch says it met a conflict.
    h.keep = Keep::Conflicts;
    let done = combine_in(&mut h, "side").unwrap();
    assert_eq!(done.result, Combined::Forward);
    assert!(done.holds_conflicts);
}

#[test]
fn only_if_clean_stops_a_forward_onto_a_save_past_the_limits() {
    let mut h = Hist::new();
    h.keep = Keep::OnlyIfClean;
    ahead_onto_a_conflict(&mut h);
    let before = h.untouched();
    // The target holds two entries: past a limit of one, it can't be looked through.
    let tight = Bounds {
        limits: SaveLimits::new(crate::listing::MAX_SAVE_DEPTH, 1),
        ..bounds()
    };
    let err = combine_by(&mut h, "side", None, tight).unwrap_err();
    assert!(
        matches!(
            err,
            CombineError::Past {
                which: Which::Theirs,
                ..
            }
        ),
        "{err:?}"
    );
    assert!(
        err.to_string().starts_with("side is past Opalro's limits"),
        "{err}"
    );
    assert_untouched(&h, &before);
}

#[test]
fn a_forward_without_the_flag_reads_nothing_more() {
    // Onto a clean save: as many listings and records read as where conflicts aren't kept.
    let mut reads = vec![];
    for (feature, keep) in [
        (false, Keep::Conflicts),
        (true, Keep::Conflicts),
        (true, Keep::OnlyIfClean),
    ] {
        let mut h = Hist::new();
        if !feature {
            h.repo = crate::test_support::without_conflicts_feature(h.dir.path());
        }
        h.keep = keep;
        let first = h.first;
        let base = tree(&[("a.txt", f("a\n"))]);
        let b = h.plant(&[first], &base, Some(&[]));
        let t = h.plant(&[b], &with(&base, &[("d/x.txt", f("x\n"))]), Some(&[]));
        h.set(|s| {
            s.lines.insert("side".into(), t);
            s.lines.insert("main".into(), b);
        });
        h.on("main", &base);
        let store = h.repo.store();
        let before = (store.listings_read(), store.conflicts_read());
        let done = combine_in(&mut h, "side").unwrap();
        assert_eq!(done.result, Combined::Forward);
        let store = h.repo.store();
        reads.push((
            store.listings_read() - before.0,
            store.conflicts_read() - before.1,
        ));
    }
    assert_eq!(reads[0], reads[1]);
    // Asked for a clean line, the target is looked through: the measure is counted.
    assert!(reads[2].0 > reads[1].0, "{reads:?}");
}

#[test]
fn only_if_clean_stops_for_a_conflict_inside_a_folder_one_line_changed() {
    let mut h = Hist::new();
    h.keep = Keep::OnlyIfClean;
    let c = plant_record(&mut h, [b"mine\n", b"was\n", b"theirs\n"]);
    let base = tree(&[("a.txt", f("a\n")), ("d/k.txt", f("k\n"))]);
    let main = with(&base, &[("a.txt", f("main\n"))]);
    let side = with(&base, &[("d/x.txt", V::Conflict(c))]);
    planted_lines(&mut h, &base, &main, &side);
    let before = h.untouched();
    let err = combine_in(&mut h, "side").unwrap_err();
    assert!(
        matches!(
            &err,
            CombineError::Conflicts {
                conflicts,
                held: 1,
                forward: false,
                why: NotKept::OnlyIfClean,
                ..
            } if conflicts.is_empty()
        ),
        "{err:?}"
    );
    assert_untouched(&h, &before);
}

#[test]
fn without_the_feature_a_forward_is_not_looked_through() {
    let mut reads = vec![];
    for keep in [Keep::Conflicts, Keep::OnlyIfClean] {
        let dir = tempfile::tempdir().unwrap();
        Repository::init(dir.path(), &ada()).unwrap();
        let repo = crate::test_support::without_conflicts_feature(dir.path());
        let first = repo.current().unwrap().1.state.lines["main"];
        let mut h = Hist {
            dir,
            repo,
            first,
            keep,
        };
        ahead_onto_a_conflict(&mut h);
        let before = h.repo.store().listings_read();
        let done = combine_in(&mut h, "side").unwrap();
        assert_eq!(done.result, Combined::Forward);
        reads.push(h.repo.store().listings_read() - before);
    }
    assert_eq!(reads[0], reads[1]);
}

#[test]
fn unsaved_work_elsewhere_comes_along() {
    let mut h = Hist::new();
    diverged(&mut h);
    fs::write(h.dir.path().join("b.txt"), "mine\n").unwrap();
    let done = combine_in(&mut h, "side").unwrap();
    assert_eq!(done.carried, 1);
    let mut want = both_tree();
    want.insert("b.txt".into(), f("mine\n"));
    assert_eq!(h.folder(), want);
}

/// Saves after one of `base` on `main`, one for each of `starts`, each on a line of its own,
/// then a save of `both` after all of them and another after them in the opposite order: two
/// tips that share every one of them as a common starting point.
fn crossed(h: &mut Hist, base: &Tree, starts: &[Tree], both: &Tree) -> (Id, Id) {
    let first = h.first;
    let o = h.save("main", first, base);
    let ids: Vec<Id> = starts
        .iter()
        .enumerate()
        .map(|(i, t)| h.save(&format!("start{i}"), o, t))
        .collect();
    let back: Vec<Id> = ids.iter().rev().copied().collect();
    (h.plant(&ids, both, None), h.plant(&back, both, None))
}

/// From the two tips, `side` saves `side_t` and then `main` saves `main_t`, the project on it.
fn tips(h: &mut Hist, (m, s): (Id, Id), main_t: &Tree, side_t: &Tree) {
    h.save("side", s, side_t);
    h.save("main", m, main_t);
}

fn with(t: &Tree, items: &[(&str, V)]) -> Tree {
    let mut t = t.clone();
    t.extend(tree(items));
    t
}

#[test]
fn a_criss_cross_keeps_a_revert_that_one_starting_point_would_lose() {
    let old = tree(&[("f.txt", f("old\n"))]);
    let b1 = with(&old, &[("one.txt", f("1\n"))]);
    let b2 = tree(&[("f.txt", f("new\n"))]);
    let both = tree(&[("f.txt", f("new\n")), ("one.txt", f("1\n"))]);
    // Built until the starting point that would lose the revert comes first in ID order, so a
    // combine that took the first alone would lose it every time.
    let (mut h, crossed) = loop {
        let mut h = Hist::new();
        let crossed = crossed(&mut h, &old, &[b1.clone(), b2.clone()], &both);
        let lines = h.entry().1.state.lines;
        if lines["start0"] < lines["start1"] {
            break (h, crossed);
        }
    };
    // `main` takes the change back; `side` adds a file.
    let reverted = with(&both, &[("f.txt", f("old\n"))]);
    tips(
        &mut h,
        crossed,
        &reverted,
        &with(&both, &[("s.txt", f("s\n"))]),
    );
    let done = combine_in(&mut h, "side").unwrap();
    let Combined::Made { starts, .. } = &done.result else {
        panic!("{done:?}");
    };
    assert_eq!(starts.len(), 2);
    assert_eq!(h.folder(), with(&reverted, &[("s.txt", f("s\n"))]));
}

/// Two starting points that changed `f.txt` in the same place, the lines combined criss-cross,
/// then `main_f` and `side_f` at `f.txt` on the two lines; the project is on `main`.
fn starts_clash_at_f(h: &mut Hist, main_f: V, side_f: V) {
    let base = tree(&[("f.txt", f(BODY))]);
    let b1 = tree(&[("f.txt", edit(2, "one")), ("one.txt", f("1\n"))]);
    let b2 = tree(&[("f.txt", edit(2, "two"))]);
    let both = tree(&[("f.txt", f("settled\n")), ("one.txt", f("1\n"))]);
    let crossed = crossed(h, &base, &[b1, b2], &both);
    tips(
        h,
        crossed,
        &with(&both, &[("m.txt", f("m\n")), ("f.txt", main_f)]),
        &with(&both, &[("s.txt", f("s\n")), ("f.txt", side_f)]),
    );
}

#[test]
fn starting_points_that_conflict_are_unknown_where_they_clash() {
    // Both lines hold it alike: that is the result.
    let mut h = Hist::new();
    starts_clash_at_f(&mut h, f("settled\n"), f("settled\n"));
    let done = combine_in(&mut h, "side").unwrap();
    let Combined::Made {
        starts, conflicts, ..
    } = &done.result
    else {
        panic!("{done:?}");
    };
    assert_eq!((starts.len(), conflicts.len()), (2, 0));
    let want = tree(&[
        ("f.txt", f("settled\n")),
        ("m.txt", f("m\n")),
        ("one.txt", f("1\n")),
        ("s.txt", f("s\n")),
    ]);
    assert_eq!(h.folder(), want);

    // Differently: the two whole, nothing known in the middle.
    let mut h = Hist::new();
    starts_clash_at_f(&mut h, f("mine\n"), f("theirs\n"));
    let (saved, conflicts, count) = kept_combine(&mut h);
    let markers = Stands::Markers { places: 1 };
    assert_eq!(conflicts, [kept_at("f.txt", Clash::StartUnknown, markers)]);
    assert_eq!(count, 1);
    let V::Conflict(id) = saved["f.txt"] else {
        panic!("{saved:?}");
    };
    let store = h.repo.store();
    let r = store.conflict(&id).unwrap();
    let terms: Vec<Option<V>> = (r.terms.iter())
        .map(|t| t.map(|t| leaf(store, t.kind, t.id)))
        .collect();
    assert_eq!(terms, [Some(f("mine\n")), None, Some(f("theirs\n"))]);
    let version = r.own.unwrap().version.unwrap();
    let V::File(text, false) = leaf(store, version.kind, version.id) else {
        panic!();
    };
    let text = String::from_utf8(text).unwrap();
    assert!(
        text.contains(", which Opalro can't tell here\n=======\n"),
        "{text}"
    );
    assert!(
        text.starts_with("<<<<<<< ") && text.contains("mine\n"),
        "{text}"
    );

    // Asked to stop at a conflict, it names it.
    let mut h = Hist::new();
    h.keep = Keep::OnlyIfClean;
    starts_clash_at_f(&mut h, f("mine\n"), f("theirs\n"));
    let before = h.untouched();
    let err = combine_in(&mut h, "side").unwrap_err();
    let CombineError::Conflicts { conflicts, .. } = &err else {
        panic!("{err:?}");
    };
    assert_eq!(
        conflicts
            .iter()
            .map(|c| c.describe("main", "side"))
            .collect::<Vec<_>>(),
        ["f.txt: changed on both lines, and Opalro can't tell what it held where they parted"]
    );
    assert_untouched(&h, &before);
}

/// A criss-cross whose two starting points added `u.txt` differently, `base` holding the rest,
/// then `main_t` and `side_t` on the two lines; the project is on `main`.
fn starts_added_u_apart(h: &mut Hist, base: &Tree, main_t: &Tree, side_t: &Tree) {
    let b1 = with(base, &[("u.txt", f("1\n"))]);
    let b2 = with(base, &[("u.txt", f("2\n"))]);
    let both = with(base, &[("u.txt", f("both\n"))]);
    let crossed = crossed(h, base, &[b1, b2], &both);
    tips(h, crossed, main_t, side_t);
}

#[test]
fn unknown_is_not_absent() {
    let base = tree(&[("k.txt", f("k\n"))]);
    let held = with(&base, &[("u.txt", f("both\n"))]);
    // Gone on main, still there on side: neither a removal nor an addition is known.
    for (main, side, terms, stands) in [
        (
            with(&base, &[("m.txt", f("m\n"))]),
            with(&held, &[("s.txt", f("s\n"))]),
            [None, None, Some(f("both\n"))],
            Stands::Nothing,
        ),
        (
            with(&held, &[("m.txt", f("m\n"))]),
            with(&base, &[("s.txt", f("s\n"))]),
            [Some(f("both\n")), None, None],
            Stands::Yours,
        ),
    ] {
        let mut h = Hist::new();
        starts_added_u_apart(&mut h, &base, &main, &side);
        let (saved, conflicts, _) = kept_combine(&mut h);
        assert_eq!(conflicts, [kept_at("u.txt", Clash::StartUnknown, stands)]);
        let V::Conflict(id) = saved["u.txt"] else {
            panic!("{saved:?}");
        };
        let store = h.repo.store();
        let r = store.conflict(&id).unwrap();
        let got: Vec<Option<V>> = (r.terms.iter())
            .map(|t| t.map(|t| leaf(store, t.kind, t.id)))
            .collect();
        assert_eq!(got, terms);
        assert_eq!(r.own, None);
    }
}

#[test]
fn a_made_up_folder_equal_to_one_side_is_not_taken_as_unchanged() {
    let base = tree(&[("d/x.txt", f("x\n"))]);
    let b1 = with(&base, &[("d/y.txt", f("1\n"))]);
    let b2 = with(&base, &[("d/y.txt", f("2\n"))]);
    let both = with(&base, &[("d/y.txt", f("both\n"))]);
    let mut h = Hist::new();
    let crossed = crossed(&mut h, &base, &[b1, b2], &both);
    // Main holds exactly the made-up start, at the top and in `d`; side holds `d/y.txt`.
    tips(&mut h, crossed, &base, &with(&both, &[("s.txt", f("s\n"))]));
    let (saved, conflicts, _) = kept_combine(&mut h);
    assert_eq!(
        conflicts,
        [kept_at("d/y.txt", Clash::StartUnknown, Stands::Nothing)]
    );
    assert_eq!(saved["d/x.txt"], f("x\n"));
    assert_eq!(saved["s.txt"], f("s\n"));
}

#[test]
fn unknown_inside_two_folders_is_decided_entry_by_entry() {
    // One starting point made `d` a file, the other changed the folder: `d` is unknown.
    let base = tree(&[("d/a.txt", f("a\n"))]);
    let b1 = tree(&[("d", f("d\n"))]);
    let b2 = with(&base, &[("d/b.txt", f("b\n"))]);
    let both = tree(&[("d/a.txt", f("a\n")), ("d/c.txt", f("c\n"))]);
    let mut h = Hist::new();
    let crossed = crossed(&mut h, &base, &[b1, b2], &both);
    let main = with(&both, &[("d/m.txt", f("m\n"))]);
    let side = with(&both, &[("d/a.txt", f("A\n")), ("d/s.txt", f("s\n"))]);
    tips(&mut h, crossed, &main, &side);
    let (saved, conflicts, _) = kept_combine(&mut h);
    let unknown = Clash::StartUnknown;
    assert_eq!(
        conflicts,
        [
            kept_at("d/a.txt", unknown.clone(), Stands::Markers { places: 1 }),
            kept_at("d/m.txt", unknown.clone(), Stands::Yours),
            kept_at("d/s.txt", unknown, Stands::Nothing),
        ]
    );
    assert_eq!(saved["d/c.txt"], f("c\n"));
}

#[test]
fn a_move_into_or_out_of_an_unknown_place_is_not_followed() {
    let base = tree(&[("k.txt", f("k\n")), ("w.txt", f("w\n"))]);
    let main = with(&base, &[("u.txt", f("both\n")), ("m.txt", f("m\n"))]);
    // `w.txt`'s contents left for `u.txt`, which the made-up start can't tell of.
    let side = tree(&[
        ("k.txt", f("k\n")),
        ("u.txt", f("w\n")),
        ("s.txt", f("s\n")),
    ]);
    let mut h = Hist::new();
    starts_added_u_apart(&mut h, &base, &main, &side);
    let (saved, conflicts, _) = kept_combine(&mut h);
    let markers = Stands::Markers { places: 1 };
    assert_eq!(conflicts, [kept_at("u.txt", Clash::StartUnknown, markers)]);
    // Taken out on side, as any removal.
    assert!(!saved.contains_key("w.txt"), "{saved:?}");
}

/// Three starting points under one save holding `u.txt`: two changed it apart and one left it,
/// each adding a file of its own, combined criss-cross. Built until the one that left it is at
/// `at` in ID order, the order they are combined in. Gives the tips and what they hold.
fn three_starts_untouched_at(at: usize) -> (Hist, (Id, Id), Tree) {
    let base = tree(&[("k.txt", f("k\n")), ("u.txt", f("o\n"))]);
    let starts = [
        with(&base, &[("a.txt", f("a\n")), ("u.txt", f("1\n"))]),
        with(&base, &[("b.txt", f("b\n")), ("u.txt", f("2\n"))]),
        with(&base, &[("c.txt", f("c\n"))]),
    ];
    let both = with(
        &base,
        &[
            ("a.txt", f("a\n")),
            ("b.txt", f("b\n")),
            ("c.txt", f("c\n")),
            ("u.txt", f("both\n")),
        ],
    );
    let (h, crossed) = three_starts(&base, &starts, &both, at);
    (h, crossed, both)
}

/// `crossed` with three starting points, built until the last of `starts` is at `at` in ID order.
fn three_starts(base: &Tree, starts: &[Tree; 3], both: &Tree, at: usize) -> (Hist, (Id, Id)) {
    loop {
        let mut h = Hist::new();
        let crossed = crossed(&mut h, base, starts, both);
        let lines = h.entry().1.state.lines;
        let mut ids: Vec<Id> = (0..3).map(|i| lines[&format!("start{i}")]).collect();
        ids.sort();
        if ids[at] == lines["start2"] {
            return (h, crossed);
        }
    }
}

#[test]
fn a_place_the_side_made_so_far_cant_tell_of_is_no_move() {
    // One starting point copied `u.txt` to `x.txt` and changed `u.txt`; another changed it its own
    // way, so the two together can't tell of `u.txt`. The third changed it too: that change is
    // `u.txt`'s, never carried to `x.txt` as if `u.txt` had moved there.
    let base = tree(&[("k.txt", f("k\n")), ("u.txt", f("o\n"))]);
    let starts = [
        with(&base, &[("u.txt", f("1\n")), ("x.txt", f("o\n"))]),
        with(&base, &[("u.txt", f("2\n"))]),
        with(&base, &[("u.txt", f("3\n"))]),
    ];
    let both = with(&base, &[("u.txt", f("both\n")), ("x.txt", f("o\n"))]);
    let (mut h, crossed) = three_starts(&base, &starts, &both, 2);
    // Side takes `x.txt` away, main leaves it: a removal, where `x.txt` was known as `o`.
    let mut side = with(&both, &[("s.txt", f("s\n"))]);
    side.remove("x.txt");
    tips(&mut h, crossed, &with(&both, &[("m.txt", f("m\n"))]), &side);
    let (saved, conflicts, _) = kept_combine(&mut h);
    assert!(conflicts.is_empty(), "{conflicts:?}");
    assert!(!saved.contains_key("x.txt"), "{saved:?}");
}

#[test]
fn a_place_unknown_in_a_folder_the_next_starting_point_holds_alike_stays_unknown() {
    // Two starting points changed `d/u.txt` apart and the third removed it, so that all three
    // hold `d` alike but for it; the third is combined last.
    let base = tree(&[("d/k.txt", f("k\n")), ("d/u.txt", f("o\n"))]);
    let mut gone = base.clone();
    gone.remove("d/u.txt");
    let starts = [
        with(&base, &[("a.txt", f("a\n")), ("d/u.txt", f("1\n"))]),
        with(&base, &[("b.txt", f("b\n")), ("d/u.txt", f("2\n"))]),
        with(&gone, &[("c.txt", f("c\n"))]),
    ];
    let both = with(
        &base,
        &[
            ("a.txt", f("a\n")),
            ("b.txt", f("b\n")),
            ("c.txt", f("c\n")),
            ("d/u.txt", f("both\n")),
        ],
    );
    let (mut h, crossed) = three_starts(&base, &starts, &both, 2);
    let mut main = with(&both, &[("m.txt", f("m\n"))]);
    main.remove("d/u.txt");
    tips(&mut h, crossed, &main, &with(&both, &[("s.txt", f("s\n"))]));
    let (_, conflicts, _) = kept_combine(&mut h);
    let want = [kept_at("d/u.txt", Clash::StartUnknown, Stands::Nothing)];
    assert_eq!(conflicts, want);
}

#[test]
fn three_starting_points_keep_unknown_whatever_their_order() {
    let markers = Stands::Markers { places: 1 };
    for at in 0..3 {
        // Alike on both lines: taken.
        let (mut h, crossed, both) = three_starts_untouched_at(at);
        let main = with(&both, &[("m.txt", f("m\n"))]);
        tips(&mut h, crossed, &main, &with(&both, &[("s.txt", f("s\n"))]));
        let (saved, conflicts, _) = kept_combine(&mut h);
        assert!(conflicts.is_empty(), "{at}: {conflicts:?}");
        assert_eq!(saved["u.txt"], f("both\n"), "{at}");

        // Differently: the two whole.
        let (mut h, crossed, both) = three_starts_untouched_at(at);
        let main = with(&both, &[("u.txt", f("mine\n"))]);
        let side = with(&both, &[("u.txt", f("theirs\n"))]);
        tips(&mut h, crossed, &main, &side);
        let (_, conflicts, _) = kept_combine(&mut h);
        let want = [kept_at("u.txt", Clash::StartUnknown, markers)];
        assert_eq!(conflicts, want, "{at}");

        // Held on one line only: neither an addition nor a removal.
        let (mut h, crossed, both) = three_starts_untouched_at(at);
        let mut main = with(&both, &[("m.txt", f("m\n"))]);
        main.remove("u.txt");
        tips(&mut h, crossed, &main, &with(&both, &[("s.txt", f("s\n"))]));
        let (saved, conflicts, _) = kept_combine(&mut h);
        let want = [kept_at("u.txt", Clash::StartUnknown, Stands::Nothing)];
        assert_eq!(conflicts, want, "{at}");
        let V::Conflict(id) = saved["u.txt"] else {
            panic!("{at}: {saved:?}");
        };
        let store = h.repo.store();
        let r = store.conflict(&id).unwrap();
        let terms: Vec<Option<V>> = (r.terms.iter())
            .map(|t| t.map(|t| leaf(store, t.kind, t.id)))
            .collect();
        assert_eq!(terms, [None, None, Some(f("both\n"))], "{at}");
    }
}

/// Plants `base`, `b1` and `b2` after it, a save of `both` after those two in each order, then
/// `main` and `side` after those, and puts the project on `main`: two lines whose starting points
/// are `b1` and `b2`.
fn planted_crossed(h: &mut Hist, [base, b1, b2, both, main, side]: [&Tree; 6]) {
    let first = h.first;
    let none: &[(&str, &str)] = &[];
    let b = h.plant(&[first], base, Some(none));
    let one = h.plant(&[b], b1, Some(none));
    let two = h.plant(&[b], b2, Some(none));
    let m = h.plant(&[one, two], both, Some(none));
    let s = h.plant(&[two, one], both, Some(none));
    let o = h.plant(&[m], main, Some(none));
    let t = h.plant(&[s], side, Some(none));
    h.set(|state| {
        state.lines.insert("side".into(), t);
        state.lines.insert("main".into(), o);
    });
    let folder = standing(h, main);
    h.on("main", &folder);
}

#[test]
fn a_start_conflict_under_two_folders_is_unknown_below() {
    let a = || ("d/a.txt", f("a\n"));
    let x = || ("d/x.txt", f("x\n"));
    let want = [kept_at("d/x.txt", Clash::StartUnknown, Stands::Yours)];
    // Where the lines parted, `d` was in conflict; each line made it a folder its own way.
    let mut h = Hist::new();
    let c = plant_record(&mut h, [b"mine\n", b"was\n", b"theirs\n"]);
    let base = tree(&[("d", V::Conflict(c))]);
    planted_lines(&mut h, &base, &tree(&[a(), x()]), &tree(&[a()]));
    let (saved, conflicts, _) = kept_combine(&mut h);
    assert_eq!(conflicts, want);
    assert_eq!(saved["d/a.txt"], f("a\n"));

    // The same between two starting points: `d/x.txt` is unknown in the start they make.
    let mut h = Hist::new();
    let c = plant_record(&mut h, [b"mine\n", b"was\n", b"theirs\n"]);
    let base = tree(&[("d", V::Conflict(c))]);
    let both = tree(&[a(), x()]);
    let main = tree(&[a(), x(), ("m.txt", f("m\n"))]);
    let side = tree(&[a(), ("s.txt", f("s\n"))]);
    let b1 = tree(&[a(), x()]);
    let b2 = tree(&[a()]);
    planted_crossed(&mut h, [&base, &b1, &b2, &both, &main, &side]);
    let (_, conflicts, _) = kept_combine(&mut h);
    assert_eq!(conflicts, want);

    // Without the feature, the entry is no conflict to Opalro: as before, the folders merge
    // from nothing.
    let dir = tempfile::tempdir().unwrap();
    Repository::init(dir.path(), &ada()).unwrap();
    let repo = crate::test_support::without_conflicts_feature(dir.path());
    let first = repo.current().unwrap().1.state.lines["main"];
    let mut h = Hist {
        dir,
        repo,
        first,
        keep: Keep::Conflicts,
    };
    let c = plant_record(&mut h, [b"mine\n", b"was\n", b"theirs\n"]);
    let base = tree(&[("d", V::Conflict(c))]);
    let main = tree(&[a(), x()]);
    planted_lines(&mut h, &base, &main, &tree(&[a()]));
    let done = combine_in(&mut h, "side").unwrap();
    assert!(matches!(&done.result, Combined::Made { conflicts, .. } if conflicts.is_empty()));
    assert_eq!(h.folder(), main);
}

#[test]
fn a_move_clash_between_starting_points_leaves_its_places_unknown() {
    let base = tree(&[("k.txt", f("k\n")), ("p.txt", f(BODY))]);
    let moved = |to: &str| tree(&[("k.txt", f("k\n")), (to, f(BODY))]);
    // Moved apart, and moved against removed.
    for b2 in [moved("b/p.txt"), tree(&[("k.txt", f("k\n"))])] {
        let mut h = Hist::new();
        let both = moved("a/p.txt");
        let crossed = crossed(&mut h, &base, &[moved("a/p.txt"), b2], &both);
        // Main keeps the file where it went, side took it away: neither is known.
        let main = with(&both, &[("m.txt", f("m\n"))]);
        let side = tree(&[("k.txt", f("k\n")), ("s.txt", f("s\n"))]);
        tips(&mut h, crossed, &main, &side);
        let (_, conflicts, _) = kept_combine(&mut h);
        let want = [kept_at("a/p.txt", Clash::StartUnknown, Stands::Yours)];
        assert_eq!(conflicts, want);
    }
}

#[test]
fn a_folder_against_nothing_with_unknown_below_is_gone_into() {
    // The start holds no `d`, and is unknown at `d/u.txt` only: side's new `d/n.txt` is an
    // addition, `d/u.txt` a conflict.
    let base = tree(&[("k.txt", f("k\n"))]);
    let mut h = Hist::new();
    starts_added_at(&mut h, &base, "d/u.txt");
    let (saved, conflicts, _) = kept_combine(&mut h);
    assert_eq!(
        conflicts,
        [kept_at("d/u.txt", Clash::StartUnknown, Stands::Nothing)]
    );
    assert_eq!(saved["d/n.txt"], f("n\n"));

    // The start holds `d`: what it is known to hold there is merged as ever.
    let base = tree(&[("d/j.txt", f("j\n")), ("d/k.txt", f("k\n"))]);
    let mut h = Hist::new();
    starts_added_at(&mut h, &base, "d/u.txt");
    let (saved, conflicts, _) = kept_combine(&mut h);
    let removed = Clash::ChangedRemoved {
        removed_on: Side::Ours,
    };
    assert_eq!(
        conflicts,
        [
            kept_at("d/j.txt", removed, Stands::Nothing),
            kept_at("d/u.txt", Clash::StartUnknown, Stands::Nothing),
        ]
    );
    assert!(!saved.contains_key("d/k.txt"), "{saved:?}");
    assert_eq!(saved["d/n.txt"], f("n\n"));
}

/// Two starting points that added `at` apart to `base`, combined criss-cross; then main takes
/// away everything but a file of its own, and side changes `d/j.txt` (if `base` holds it) and
/// adds `d/n.txt`.
fn starts_added_at(h: &mut Hist, base: &Tree, at: &str) {
    let b1 = with(base, &[(at, f("1\n"))]);
    let b2 = with(base, &[(at, f("2\n"))]);
    let both = with(base, &[(at, f("both\n"))]);
    let crossed = crossed(h, base, &[b1, b2], &both);
    let mut side = with(&both, &[("d/n.txt", f("n\n")), ("s.txt", f("s\n"))]);
    if side.contains_key("d/j.txt") {
        side.insert("d/j.txt".into(), f("J\n"));
    }
    tips(h, crossed, &tree(&[("m.txt", f("m\n"))]), &side);
}

/// Two starting points that changed `a.txt` in different places, the lines combined criss-cross,
/// then a file on each line; the project is on `main`.
fn starts_changed_apart(h: &mut Hist) {
    let base = tree(&[("a.txt", f(BODY))]);
    let b1 = tree(&[("a.txt", edit(0, "ONE"))]);
    let b2 = tree(&[("a.txt", edit(4, "FIVE"))]);
    let both = tree(&[("a.txt", f("ONE\ntwo\nthree\nfour\nFIVE\n"))]);
    let crossed = crossed(h, &base, &[b1, b2], &both);
    tips(
        h,
        crossed,
        &with(&both, &[("m.txt", f("m\n"))]),
        &with(&both, &[("s.txt", f("s\n"))]),
    );
}

#[test]
fn starting_points_that_cant_be_combined_while_the_rules_are_not_read_whole_say_so() {
    let mut h = Hist::new();
    starts_changed_apart(&mut h);
    rules_file(&h, NOT_WHOLE[2]);
    let before = h.untouched();
    let err = combine_in(&mut h, "side").unwrap_err();
    assert!(
        matches!(
            err,
            CombineError::StartsConflict {
                rules_not_whole: Side::Ours,
                ..
            }
        ),
        "{err:?}"
    );
    assert_eq!(
        err.to_string(),
        "Combining side into main needs their common starting points combined first, and \
         opalro.toml would have to decide a file there but can't be read whole, so nothing was \
         changed."
    );
    assert_eq!(err.hint().unwrap(), RULES_HINT);
    assert_untouched(&h, &before);
    // Read whole, the same history combines as before.
    rules_file(&h, "[[rule]]\nfiles = [\"*.unity\"]\n");
    let done = combine_in(&mut h, "side").unwrap();
    assert!(matches!(done.result, Combined::Made { .. }), "{done:?}");
    assert!(done.applied.warnings.is_empty(), "{done:?}");
}

/// `n` common starting points, each adding a file of its own, then a file on each line.
fn many_starts(h: &mut Hist, n: usize) {
    let base = tree(&[("base.txt", f("base\n"))]);
    let starts: Vec<Tree> = (0..n)
        .map(|i| with(&base, &[(&format!("{i}.txt"), f("i\n"))]))
        .collect();
    let mut both = base.clone();
    for t in &starts {
        both.extend(t.clone());
    }
    let crossed = crossed(h, &base, &starts, &both);
    tips(
        h,
        crossed,
        &with(&both, &[("m.txt", f("m\n"))]),
        &with(&both, &[("s.txt", f("s\n"))]),
    );
}

#[test]
fn seventeen_starting_points_are_refused() {
    let mut h = Hist::new();
    many_starts(&mut h, 17);
    let before = h.untouched();
    let err = combine_in(&mut h, "side").unwrap_err();
    assert!(matches!(err, CombineError::Tangled { .. }), "{err:?}");
    assert_eq!(
        err.to_string(),
        "The histories of main and side cross each other too many times for Opalro to combine \
         them, so nothing was changed."
    );
    assert_eq!(err.hint(), None);
    assert_untouched(&h, &before);
}

#[test]
fn the_budget_of_inner_combines_is_kept() {
    let mut h = Hist::new();
    many_starts(&mut h, 3);
    let before = h.untouched();
    let tight = Bounds {
        max_inner: 1,
        ..bounds()
    };
    let err = combine_by(&mut h, "side", None, tight).unwrap_err();
    assert!(matches!(err, CombineError::Tangled { .. }), "{err:?}");
    assert_untouched(&h, &before);
    let done = combine_in(&mut h, "side").unwrap();
    let Combined::Made { starts, .. } = &done.result else {
        panic!("{done:?}");
    };
    assert_eq!(starts.len(), 3);
    assert_eq!(h.folder().len(), 6);
}

#[test]
fn inner_text_past_what_is_held_is_refused() {
    let mut h = Hist::new();
    let text = |first: &str, last: &str| {
        let mut lines: Vec<String> = (0..10).map(|i| format!("text {i:04}\n")).collect();
        lines[0] = format!("{first}\n");
        lines[9] = format!("{last}\n");
        f(&lines.concat())
    };
    let base = tree(&[("t.txt", text("text 0000", "text 0009"))]);
    let b1 = tree(&[("t.txt", text("first 000", "text 0009"))]);
    let b2 = tree(&[("t.txt", text("text 0000", "second 00"))]);
    let both = tree(&[("t.txt", text("first 000", "second 00"))]);
    let crossed = crossed(&mut h, &base, &[b1, b2], &both);
    tips(
        &mut h,
        crossed,
        &with(&both, &[("m.txt", f("m\n"))]),
        &with(&both, &[("s.txt", f("s\n"))]),
    );
    let before = h.untouched();
    // A comparison past its deadline gives up and makes a conflict instead; a computer that
    // stalls mustn't turn the refusal into one.
    let tight = Bounds {
        max_held: 10,
        deadline: std::time::Duration::from_secs(600),
        ..bounds()
    };
    let err = match combine_by(&mut h, "side", None, tight) {
        Err(err) => err,
        Ok(done) => panic!("combined: {done:?}"),
    };
    assert!(
        matches!(err, CombineError::HeldTooMuch { .. }),
        "got: {err}; {err:?}"
    );
    assert_eq!(
        err.to_string(),
        "Combining side into main needs their common starting points combined first, and that \
         merges more than 256 MiB of text, more than Opalro holds in memory for it, so nothing \
         was changed."
    );
    assert_untouched(&h, &before);
    let patient = Bounds {
        deadline: std::time::Duration::from_secs(600),
        ..bounds()
    };
    let done = combine_by(&mut h, "side", None, patient).unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(done.result, Combined::Made { .. }), "{done:?}");
}

#[test]
fn a_result_past_the_limits_is_refused() {
    let mut h = Hist::new();
    let first = h.first;
    let six = |side: &str| -> Tree {
        (0..6)
            .map(|i| (format!("{side}{i}.txt"), f("x\n")))
            .collect()
    };
    h.save("side", first, &six("s"));
    h.save("main", first, &six("m"));
    let before = h.untouched();
    let tight = Bounds {
        limits: SaveLimits::new(crate::listing::MAX_SAVE_DEPTH, 10),
        ..bounds()
    };
    let err = combine_by(&mut h, "side", None, tight).unwrap_err();
    let CombineError::TooBig { detail } = &err else {
        panic!("{err:?}");
    };
    assert_eq!(detail, "it has more than 10 files and folders in total");
    assert_eq!(
        err.to_string(),
        "The combined save would be past Opalro's limits for a save (it has more than 10 files \
         and folders in total), so nothing was changed."
    );
    assert_untouched(&h, &before);
}

#[test]
fn the_folders_rules_decide() {
    let mut h = Hist::new();
    // Milestone 8's combine, which stops at a conflict.
    h.keep = Keep::OnlyIfClean;
    let first = h.first;
    let lines: String = (0..20).map(|i| format!("line {i}\n")).collect();
    let unity = |from: &str, to: &str| tree(&[("Main.unity", f(&lines.replace(from, to)))]);
    let base = h.save("main", first, &unity("", ""));
    h.save("side", base, &unity("line 18\n", "theirs\n"));
    h.save("main", base, &unity("line 1\n", "ours\n"));
    let rules = "[[rule]]\nfiles = [\"*.unity\"]\nconflict-markers = false\n";
    fs::write(h.dir.path().join("opalro.toml"), rules).unwrap();
    let err = combine_in(&mut h, "side").unwrap_err();
    let CombineError::Conflicts { conflicts, .. } = &err else {
        panic!("{err:?}");
    };
    let want = Conflict {
        path: "Main.unity".into(),
        clash: Clash::BothChanged(NotMerged::NoMarkers { theirs: false }),
        stands: None,
    };
    assert_eq!(conflicts, &[want]);
    // Without the rule, the same file merges.
    fs::remove_file(h.dir.path().join("opalro.toml")).unwrap();
    let done = combine_in(&mut h, "side").unwrap();
    let Combined::Made { merged_files, .. } = &done.result else {
        panic!("{done:?}");
    };
    assert_eq!(merged_files, &["Main.unity"]);
}

/// An `opalro.toml` passed over whole, one with a part passed over inside a rule, and one with a
/// part passed over at the top.
const NOT_WHOLE: [&str; 3] = [
    "[[rule]\nfiles = [\"*.unity\"]\n",
    "[[rule]]\nfiles = [\"*.unity\"]\nlockabel = true\n",
    "colour = \"red\"\n",
];

const RULES_HINT: &str = "opalro setup says what is wrong with opalro.toml. Fix or remove it and \
                          save; then run opalro combine again.";

fn rules_file(h: &Hist, text: &str) {
    fs::write(h.dir.path().join("opalro.toml"), text).unwrap();
}

fn kinds(warnings: &[Warning]) -> Vec<&'static str> {
    warnings.iter().map(|w| w.kind).collect()
}

/// `a.txt` changed in different places on `main` and `side`; the project is on `main`.
fn changed_apart(h: &mut Hist) {
    let first = h.first;
    let base = h.save("main", first, &tree(&[("a.txt", f(BODY))]));
    h.save("side", base, &tree(&[("a.txt", edit(0, "side"))]));
    h.save("main", base, &tree(&[("a.txt", edit(4, "main"))]));
}

#[test]
fn while_the_rules_are_not_read_whole_no_file_is_merged_line_by_line() {
    for text in NOT_WHOLE {
        let mut h = Hist::new();
        // Milestone 8's combine, which stops at a conflict.
        h.keep = Keep::OnlyIfClean;
        changed_apart(&mut h);
        rules_file(&h, text);
        let before = h.untouched();
        let err = combine_in(&mut h, "side").unwrap_err();
        let CombineError::Conflicts {
            conflicts,
            more: 0,
            rules_not_whole: Some(Side::Ours),
            ..
        } = &err
        else {
            panic!("{text}: {err:?}");
        };
        let want = Conflict {
            path: "a.txt".into(),
            clash: Clash::BothChanged(NotMerged::RulesNotWhole { theirs: false }),
            stands: None,
        };
        assert_eq!(conflicts, &[want], "{text}");
        assert_eq!(
            err.to_string(),
            "Combining side into main would leave 1 place in conflict, so nothing was changed \
             (--only-if-clean):\n  a.txt: changed on both lines, and opalro.toml can't be read \
             whole, so no file is merged line by line"
        );
        assert_eq!(err.hint().unwrap(), RULES_HINT);
        assert_untouched(&h, &before);
        // Read whole, the same history merges as before, and nothing is said of the rules.
        rules_file(&h, "[[rule]]\nfiles = [\"*.unity\"]\n");
        let done = combine_in(&mut h, "side").unwrap();
        let Combined::Made { merged_files, .. } = &done.result else {
            panic!("{done:?}");
        };
        assert_eq!(merged_files, &["a.txt"]);
        assert!(done.applied.warnings.is_empty(), "{done:?}");
    }
}

/// Before such rules were passed over, the file counted as read whole and `a.txt` merged.
#[test]
fn a_rule_that_can_match_no_file_stops_line_merging_until_it_is_fixed() {
    let rules = [
        "[[rule]]\nfiles = [\"!*.unity\"]\nconflict-markers = false\n",
        "[[rule]]\nfiles = [\"*.unity\", \"x\\ny\"]\nconflict-markers = false\n",
    ];
    for text in rules {
        let mut h = Hist::new();
        // Milestone 8's combine, which stops at a conflict.
        h.keep = Keep::OnlyIfClean;
        changed_apart(&mut h);
        rules_file(&h, text);
        let err = combine_in(&mut h, "side").unwrap_err();
        let (conflict, not_whole) = only_conflict(&err);
        assert_eq!(
            conflict.clash,
            Clash::BothChanged(NotMerged::RulesNotWhole { theirs: false }),
            "{text}"
        );
        assert_eq!(not_whole, Some(Side::Ours), "{text}");
        assert_eq!(err.hint().unwrap(), RULES_HINT, "{text}");
    }
}

#[test]
fn a_file_that_clashes_for_a_reason_of_its_own_keeps_it_while_the_rules_are_not_read_whole() {
    let mut h = Hist::new();
    // Milestone 8's combine, which stops at a conflict.
    h.keep = Keep::OnlyIfClean;
    let first = h.first;
    let bin = |b: u8| V::File(vec![0, b], false);
    let base = h.save(
        "main",
        first,
        &tree(&[("a.txt", f(BODY)), ("c.bin", bin(0))]),
    );
    h.save(
        "side",
        base,
        &tree(&[("a.txt", edit(2, "side")), ("c.bin", bin(1))]),
    );
    h.save(
        "main",
        base,
        &tree(&[("a.txt", edit(2, "main")), ("c.bin", bin(2))]),
    );
    rules_file(&h, NOT_WHOLE[0]);
    let err = combine_in(&mut h, "side").unwrap_err();
    let CombineError::Conflicts {
        conflicts,
        rules_not_whole: None,
        ..
    } = &err
    else {
        panic!("{err:?}");
    };
    let clashes: Vec<&Clash> = conflicts.iter().map(|c| &c.clash).collect();
    assert_eq!(
        clashes,
        [
            &Clash::BothChanged(NotMerged::Overlap {
                endings_only: false
            }),
            &Clash::BothChanged(NotMerged::Binary)
        ]
    );
    assert_eq!(err.hint().unwrap(), CONFLICTS_HINT);
}

#[test]
fn the_hint_names_the_rules_file_when_only_a_counted_conflict_is_there_for_it() {
    // 100 files that overlap come first; `z.txt` would merge, and is only counted.
    let mut h = Hist::new();
    // Milestone 8's combine, which stops at a conflict.
    h.keep = Keep::OnlyIfClean;
    let first = h.first;
    let make = |side: Option<&str>, z: V| {
        let mut t: Tree = (0..100)
            .map(|i| (format!("a{i:03}.txt"), side.map_or(f(BODY), |s| edit(2, s))))
            .collect();
        t.insert("z.txt".into(), z);
        t
    };
    let base = h.save("main", first, &make(None, f(BODY)));
    h.save("side", base, &make(Some("side"), edit(0, "side")));
    h.save("main", base, &make(Some("main"), edit(4, "main")));
    rules_file(&h, NOT_WHOLE[2]);
    let err = combine_in(&mut h, "side").unwrap_err();
    let CombineError::Conflicts {
        conflicts,
        more,
        rules_not_whole,
        ..
    } = &err
    else {
        panic!("{err:?}");
    };
    assert_eq!(
        (conflicts.len(), *more, *rules_not_whole),
        (100, 1, Some(Side::Ours))
    );
    let own = Clash::BothChanged(NotMerged::RulesNotWhole { theirs: false });
    assert!(conflicts.iter().all(|c| c.clash != own));
    assert_eq!(err.hint().unwrap(), RULES_HINT);
}

#[test]
fn a_combine_that_merges_no_file_goes_through_and_warns_of_the_rules_file_it_leaves() {
    let said = [
        warning::RULES_FILE_SKIPPED,
        warning::RULES_IGNORED,
        warning::RULES_IGNORED,
    ];
    for (text, kind) in NOT_WHOLE.into_iter().zip(said) {
        let mut h = Hist::new();
        diverged(&mut h);
        rules_file(&h, text);
        let done = combine_in(&mut h, "side").unwrap();
        assert!(matches!(done.result, Combined::Made { .. }), "{done:?}");
        assert_eq!(kinds(&done.applied.warnings), [kind], "{text}");
    }
}

#[test]
fn a_line_moved_forward_warns_of_the_rules_file_it_leaves_not_the_one_it_started_with() {
    let mut h = Hist::new();
    let first = h.first;
    let good = tree(&[("opalro.toml", f("[[rule]]\nfiles = [\"*.unity\"]\n"))]);
    let broken = tree(&[("opalro.toml", f(NOT_WHOLE[0]))]);
    let base = h.save("main", first, &good);
    let side = h.save("side", base, &broken);
    h.on("main", &good);
    let done = combine_in(&mut h, "side").unwrap();
    assert_eq!(done.result, Combined::Forward);
    assert_eq!(kinds(&done.applied.warnings), [warning::RULES_FILE_SKIPPED]);
    // The fixed file brought in the same way: nothing is said.
    h.save("side", side, &good);
    h.on("main", &broken);
    let done = combine_in(&mut h, "side").unwrap();
    assert_eq!(done.result, Combined::Forward);
    assert!(done.applied.warnings.is_empty(), "{done:?}");
}

const SCENE_RULE: &str = "[[rule]]\nfiles = [\"*.unity\"]\nconflict-markers = false\n";

const THEIRS_HINT: &str = "The other side's opalro.toml has a mistake in it. Go there, fix or \
                           remove it (opalro setup says what is wrong) and save; then run opalro \
                           combine again.";

const CONFLICTS_HINT: &str =
    "Run it again without --only-if-clean to keep the conflicts and fix them one at a time.";

/// `Main.unity` edited near its end on `art`, which also saves `art_rules` as its `opalro.toml`,
/// and near its start on `main`, which saves `main_rules`; the project is on `main`. Gives the
/// two lines' trees.
fn scene_apart(h: &mut Hist, art_rules: Option<&str>, main_rules: Option<&str>) -> (Tree, Tree) {
    let first = h.first;
    let lines: String = (0..20).map(|i| format!("line {i}\n")).collect();
    let scene = |from: &str, to: &str, rules: Option<&str>| {
        let mut t = tree(&[("Main.unity", f(&lines.replace(from, to)))]);
        if let Some(rules) = rules {
            t.insert("opalro.toml".into(), f(rules));
        }
        t
    };
    let base = h.save("main", first, &scene("", "", None));
    let art = scene("line 18\n", "art\n", art_rules);
    let main = scene("line 1\n", "main\n", main_rules);
    h.save("art", base, &art);
    h.save("main", base, &main);
    (art, main)
}

fn only_conflict(err: &CombineError) -> (&Conflict, Option<Side>) {
    let CombineError::Conflicts {
        conflicts,
        more: 0,
        rules_not_whole,
        ..
    } = err
    else {
        panic!("{err:?}");
    };
    assert_eq!(conflicts.len(), 1, "{err:?}");
    (&conflicts[0], *rules_not_whole)
}

/// Before a combine asked the rules of both lines, the scene was merged line by line, and the
/// combined save held the rule that should have stopped it.
#[test]
fn a_rule_only_the_line_combined_in_has_stops_the_combine_and_names_that_line() {
    let mut h = Hist::new();
    // Milestone 8's combine, which stops at a conflict.
    h.keep = Keep::OnlyIfClean;
    let (art, _) = scene_apart(&mut h, Some(SCENE_RULE), None);
    let before = h.untouched();
    let err = combine_in(&mut h, "art").unwrap_err();
    let (conflict, not_whole) = only_conflict(&err);
    assert_eq!(
        conflict.clash,
        Clash::BothChanged(NotMerged::NoMarkers { theirs: true })
    );
    assert_eq!(not_whole, None);
    assert_eq!(
        err.to_string(),
        "Combining art into main would leave 1 place in conflict, so nothing was changed \
         (--only-if-clean):\n  Main.unity: changed on both lines, and opalro.toml on art says \
         conflict-markers = false for it"
    );
    assert_eq!(err.hint().unwrap(), CONFLICTS_HINT);
    assert_untouched(&h, &before);
    // The other way round, the rule is the folder's: today's words.
    h.on("art", &art);
    let err = combine_in(&mut h, "main").unwrap_err();
    let (conflict, _) = only_conflict(&err);
    assert_eq!(
        conflict.clash,
        Clash::BothChanged(NotMerged::NoMarkers { theirs: false })
    );
    assert_eq!(
        err.to_string(),
        "Combining main into art would leave 1 place in conflict, so nothing was \
         changed (--only-if-clean):\n  Main.unity: changed on both lines, and opalro.toml says \
         conflict-markers = false for it"
    );
}

#[test]
fn a_rule_on_both_lines_is_the_folders() {
    let mut h = Hist::new();
    // Milestone 8's combine, which stops at a conflict.
    h.keep = Keep::OnlyIfClean;
    scene_apart(&mut h, Some(SCENE_RULE), Some(SCENE_RULE));
    let err = combine_in(&mut h, "art").unwrap_err();
    assert_eq!(
        only_conflict(&err).0.clash,
        Clash::BothChanged(NotMerged::NoMarkers { theirs: false })
    );
}

/// `a.txt` changed apart on `main` and `side`, `side`'s save also holding `rules` at the top;
/// the project is on `main`.
fn theirs_rules_apart(h: &mut Hist, rules: &[(&str, V)]) {
    let first = h.first;
    let base = h.save("main", first, &tree(&[("a.txt", f(BODY))]));
    let side = with(&tree(&[("a.txt", edit(0, "side"))]), rules);
    let side = h.plant(&[base], &side, Some(&[]));
    h.set(|s| {
        s.lines.insert("side".into(), side);
    });
    h.save("main", base, &tree(&[("a.txt", edit(4, "main"))]));
}

#[test]
fn while_the_other_sides_rules_are_not_read_whole_no_file_is_merged_and_that_side_is_named() {
    let mut over = "#".repeat(1 << 20);
    over.push('\n');
    let cases: [(&str, Vec<(&str, V)>); 6] = [
        ("not TOML", vec![("opalro.toml", f(NOT_WHOLE[0]))]),
        ("a part in a rule", vec![("opalro.toml", f(NOT_WHOLE[1]))]),
        ("a part at the top", vec![("opalro.toml", f(NOT_WHOLE[2]))]),
        ("a link", vec![("opalro.toml", link("real.toml"))]),
        ("a folder", vec![("opalro.toml/a.txt", f("a\n"))]),
        ("over 1 MiB", vec![("opalro.toml", f(&over))]),
    ];
    for (case, rules) in cases {
        let mut h = Hist::new();
        // Milestone 8's combine, which stops at a conflict.
        h.keep = Keep::OnlyIfClean;
        theirs_rules_apart(&mut h, &rules);
        let before = h.untouched();
        let err = combine_in(&mut h, "side").unwrap_err();
        let (conflict, not_whole) = only_conflict(&err);
        assert_eq!(
            conflict.clash,
            Clash::BothChanged(NotMerged::RulesNotWhole { theirs: true }),
            "{case}"
        );
        assert_eq!(not_whole, Some(Side::Theirs), "{case}");
        assert_eq!(
            err.to_string(),
            "Combining side into main would leave 1 place in conflict, so nothing was changed \
             (--only-if-clean):\n  a.txt: changed on both lines, and opalro.toml on side can't \
             be read whole, so no file is merged line by line",
            "{case}"
        );
        assert_eq!(err.hint().unwrap(), THEIRS_HINT, "{case}");
        assert_untouched(&h, &before);
    }
}

#[test]
fn the_other_sides_rules_file_listed_over_1_mib_is_passed_over_unread() {
    let mut h = Hist::new();
    let first = h.first;
    let base = h.save("main", first, &tree(&[("a.txt", f(BODY))]));
    // A rule that allows `a.txt`, under a listing that says it is over 1 MiB: were it read, the
    // file would merge.
    let side = tree(&[("a.txt", edit(0, "side")), ("opalro.toml", f(SCENE_RULE))]);
    let side = h.plant(&[base], &side, Some(&[]));
    let root = {
        let mut entries = h
            .repo
            .store()
            .listing(&h.version(side).root)
            .unwrap()
            .entries()
            .to_vec();
        let rules = entries
            .iter_mut()
            .find(|e| e.name == "opalro.toml")
            .unwrap();
        rules.size = (1 << 20) + 1;
        let lock = h.repo.lock_for_change("test", &mut |_| {}).unwrap();
        let mut packs = h.repo.new_packs().unwrap();
        let listing = Listing::new(entries).unwrap();
        let root = store_listing(h.repo.store(), &mut packs, &listing).unwrap();
        h.repo.store_packs(&lock, packs).unwrap();
        root
    };
    let side = h.plant_root(&[base], root, vec![]);
    h.set(|s| {
        s.lines.insert("side".into(), side);
    });
    h.save("main", base, &tree(&[("a.txt", edit(4, "main"))]));
    let err = combine_in(&mut h, "side").unwrap_err();
    assert_eq!(
        only_conflict(&err),
        (
            &Conflict {
                path: "a.txt".into(),
                clash: Clash::BothChanged(NotMerged::RulesNotWhole { theirs: true }),
                stands: None,
            },
            Some(Side::Theirs)
        )
    );
}

#[test]
fn when_both_rules_files_are_broken_the_folders_is_named() {
    let mut h = Hist::new();
    // Milestone 8's combine, which stops at a conflict.
    h.keep = Keep::OnlyIfClean;
    theirs_rules_apart(&mut h, &[("opalro.toml", f(NOT_WHOLE[0]))]);
    rules_file(&h, NOT_WHOLE[2]);
    let err = combine_in(&mut h, "side").unwrap_err();
    let (conflict, not_whole) = only_conflict(&err);
    assert_eq!(
        conflict.clash,
        Clash::BothChanged(NotMerged::RulesNotWhole { theirs: false })
    );
    assert_eq!(not_whole, Some(Side::Ours));
    assert_eq!(
        err.to_string(),
        "Combining side into main would leave 1 place in conflict, so nothing was changed \
         (--only-if-clean):\n  a.txt: changed on both lines, and opalro.toml can't be read \
         whole, so no file is merged line by line"
    );
    assert_eq!(err.hint().unwrap(), RULES_HINT);
}

#[test]
fn with_no_rules_file_on_either_side_a_file_merges_as_before() {
    let mut h = Hist::new();
    theirs_rules_apart(&mut h, &[]);
    let done = combine_in(&mut h, "side").unwrap();
    let Combined::Made { merged_files, .. } = &done.result else {
        panic!("{done:?}");
    };
    assert_eq!(merged_files, &["a.txt"]);
}

#[test]
fn a_moved_file_whose_old_name_only_the_other_sides_rules_protect_is_not_merged() {
    let mut h = Hist::new();
    // Milestone 8's combine, which stops at a conflict.
    h.keep = Keep::OnlyIfClean;
    let first = h.first;
    let s = h.save("main", first, &tree(&[("Main.unity", f(BODY))]));
    let o = h.save("main", s, &tree(&[("Main.txt", f(BODY))]));
    let main = tree(&[("Main.txt", edit(0, "ONE"))]);
    h.save("main", o, &main);
    let side = tree(&[
        ("Main.unity", edit(4, "FIVE")),
        ("opalro.toml", f(SCENE_RULE)),
    ]);
    h.save("side", s, &side);
    h.on("main", &main);
    let err = combine_in(&mut h, "side").unwrap_err();
    assert_eq!(
        only_conflict(&err),
        (
            &Conflict {
                path: "Main.txt".into(),
                clash: Clash::BothChanged(NotMerged::NoMarkers { theirs: true }),
                stands: None,
            },
            None
        )
    );
}

#[test]
fn starting_points_that_cant_be_combined_for_the_other_sides_rules_say_so() {
    let mut h = Hist::new();
    let base = tree(&[("a.txt", f(BODY))]);
    let b1 = tree(&[("a.txt", edit(0, "ONE"))]);
    let b2 = tree(&[("a.txt", edit(4, "FIVE"))]);
    let both = tree(&[("a.txt", f("ONE\ntwo\nthree\nfour\nFIVE\n"))]);
    let crossed = crossed(&mut h, &base, &[b1, b2], &both);
    let side = with(
        &both,
        &[("s.txt", f("s\n")), ("opalro.toml", f(NOT_WHOLE[0]))],
    );
    tips(&mut h, crossed, &with(&both, &[("m.txt", f("m\n"))]), &side);
    let before = h.untouched();
    let err = combine_in(&mut h, "side").unwrap_err();
    assert!(
        matches!(
            err,
            CombineError::StartsConflict {
                rules_not_whole: Side::Theirs,
                ..
            }
        ),
        "{err:?}"
    );
    assert_eq!(
        err.to_string(),
        "Combining side into main needs their common starting points combined first, and the \
         opalro.toml on side would have to decide a file there but can't be read whole, so \
         nothing was changed."
    );
    assert_eq!(err.hint().unwrap(), THEIRS_HINT);
    assert_untouched(&h, &before);
}

#[test]
fn a_slow_comparison_is_told() {
    let mut h = Hist::new();
    let first = h.first;
    let lines: String = (0..2000).map(|i| format!("line {i}\n")).collect();
    let big = |from: &str, to: &str| tree(&[("big.txt", f(&lines.replace(from, to)))]);
    let base = h.save("main", first, &big("", ""));
    h.save("side", base, &big("line 1990\n", "theirs\n"));
    h.save("main", base, &big("line 10\n", "ours\n"));
    let hurried = Bounds {
        deadline: std::time::Duration::ZERO,
        ..bounds()
    };
    let done = combine_by(&mut h, "side", None, hurried).unwrap();
    let told: Vec<&Warning> = done
        .applied
        .warnings
        .iter()
        .filter(|w| w.kind == warning::MERGE_TIMED_OUT)
        .collect();
    assert_eq!(told.len(), 1, "{:?}", done.applied.warnings);
    assert_eq!(
        told[0].message,
        "Comparing the versions of big.txt took over 2 seconds, so Opalro compared them roughly; \
         a closer look might have merged them."
    );
}

#[test]
fn a_combine_says_when_moves_werent_followed() {
    let mut h = Hist::new();
    let s = player(&mut h);
    let to = "Characters/Player.cs";
    h.save("side", s, &moved_to("Player.cs", edit(1, "TWO")));
    h.save("main", s, &moved_to(to, f(BODY)));
    // Too little to hold even the one file that left main; its note is still followed.
    let short = Bounds {
        max_left_bytes: HELD_PER_PLACE,
        ..bounds()
    };
    let done = combine_by(&mut h, "side", None, short).unwrap();
    assert_eq!(h.folder(), moved_to(to, edit(1, "TWO")));
    let told: Vec<&Warning> = done
        .applied
        .warnings
        .iter()
        .filter(|w| w.kind == warning::MOVES_NOT_FOLLOWED)
        .collect();
    assert_eq!(told.len(), 1, "{:?}", done.applied.warnings);
    assert_eq!(
        told[0].message,
        "Too many files left their places on main for Opalro to match them by their contents, \
         so files that moved there without a note were not followed."
    );
    // Not with room for it.
    let mut h = Hist::new();
    let s = player(&mut h);
    h.save("side", s, &moved_to("Player.cs", edit(1, "TWO")));
    h.save("main", s, &moved_to(to, f(BODY)));
    let done = combine_in(&mut h, "side").unwrap();
    assert!(
        done.applied.warnings.is_empty(),
        "{:?}",
        done.applied.warnings
    );
}

#[test]
fn undo_takes_a_combine_back_and_redo_brings_it_again() {
    use crate::undo::{StepRequest, redo, undo};
    let mut h = Hist::new();
    let (_, main, _) = diverged(&mut h);
    let done = combine_in(&mut h, "side").unwrap();
    let who = ada();
    let req = StepRequest {
        n: 1,
        shelve: false,
        who: &who,
        git_excludes: None,
    };
    undo(&mut h.repo, &req, &mut |_| {}).unwrap();
    assert_eq!(h.entry().1.state.lines["main"], main);
    assert_eq!(h.folder(), main_tree());
    redo(&mut h.repo, &req, &mut |_| {}).unwrap();
    assert_eq!(h.entry().1.state.lines["main"], done.save);
    assert_eq!(h.folder(), both_tree());
}

/// `h` opened again, as the next command would open it.
#[cfg(feature = "fault-injection")]
fn reopened(h: Hist) -> Hist {
    let Hist {
        dir,
        repo,
        first,
        keep,
    } = h;
    drop(repo);
    let repo = Repository::open(dir.path()).unwrap();
    Hist {
        dir,
        repo,
        first,
        keep,
    }
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_combine_stopped_at_every_step_is_finished_or_never_happened() {
    use crate::fsio::fault;
    use crate::journal::Journal;
    use crate::test_support::after_head;
    let mut finished_later = 0;
    for n in 1.. {
        assert!(n < 200, "the combine never finished");
        let mut h = Hist::new();
        let (_, main, _) = diverged(&mut h);
        fault::arm(n);
        let result = combine_in(&mut h, "side");
        fault::disarm();
        let went_on = after_head(&h.repo);
        let mut h = reopened(h);
        // The next command finishes what was started, so the folder is the line's save, whole.
        let outcome = h.save_now("Anything");
        assert!(
            matches!(outcome, SaveOutcome::NothingChanged { .. }),
            "fault {n}: the folder was half switched"
        );
        let (_, entry) = h.entry();
        assert_eq!(entry.state.position, Position::OnLine("main".into()));
        assert_eq!(Journal::read(&h.repo).unwrap(), None, "fault {n}");
        let combined = entry.state.lines["main"] != main;
        let want = if combined { both_tree() } else { main_tree() };
        assert_eq!(h.folder(), want, "fault {n}");
        if combined {
            let again = combine_in(&mut h, "side").unwrap();
            assert_eq!(again.result, Combined::Nothing, "fault {n}");
        }
        if result.is_err() && combined {
            finished_later += 1;
        }
        if result.is_ok() && !went_on {
            assert!(combined);
            assert!(finished_later >= 3, "only {finished_later} finished later");
            break;
        }
    }
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_combine_after_a_stopped_goto_finishes_it_first() {
    use crate::fsio::fault;
    use crate::journal::Journal;
    use crate::lines::Target;
    use crate::switch::goto;
    for n in 1..100 {
        let mut h = Hist::new();
        let (_, main, side) = diverged(&mut h);
        fault::arm(n);
        let to = Target::Line("side".into());
        let stopped = goto(&mut h.repo, &to, &ada(), "goto", None, &mut |_| {});
        fault::disarm();
        let mut h = reopened(h);
        let journal = Journal::read(&h.repo).unwrap();
        // Only a stop between the journal and the head leaves a switch to finish.
        if !journal.is_some_and(|j| j.target != h.repo.head().unwrap()) {
            continue;
        }
        assert!(stopped.is_err(), "fault {n}");
        // Finishing the goto puts the project on `side`, which `main` is then combined into.
        let done = combine_in(&mut h, "main").unwrap();
        let told = h.repo.store().warnings();
        assert!(
            told.iter().any(|w| w.kind == warning::SWITCH_FINISHED),
            "{told:?}"
        );
        assert_eq!(done.line, "side");
        assert_eq!(h.save_of(done.save).parents, [side, main]);
        assert_eq!(h.folder(), both_tree());
        return;
    }
    panic!("no goto stopped between its journal and its head");
}

/// `side` changed `a.txt`, removed `c.txt` and added `out.log`; `main` changed `b.txt` and
/// ignores `out.log`. The project is on `main`.
fn crowded(h: &mut Hist) {
    let first = h.first;
    let c = ("c.txt", f("c\n"));
    let base = tree(&[("a.txt", f(BODY)), ("b.txt", f("b\n")), c.clone()]);
    let b = h.save("main", first, &base);
    let side = tree(&[
        ("a.txt", edit(0, "side")),
        ("b.txt", f("b\n")),
        ("out.log", f("theirs\n")),
    ]);
    h.save("side", b, &side);
    let main = tree(&[
        ("a.txt", f(BODY)),
        ("b.txt", f("main b\n")),
        c,
        (".opalroignore", f("out.log\n")),
    ]);
    h.save("main", b, &main);
}

#[test]
fn unsaved_work_where_the_combine_writes_stops_it() {
    // A change where the combine writes, an ignored file where it adds one, and a change to a
    // file it removes.
    for path in ["a.txt", "out.log", "c.txt"] {
        let mut h = Hist::new();
        crowded(&mut h);
        fs::write(h.dir.path().join(path), "mine\n").unwrap();
        let (head, entry) = h.entry();
        let folder = h.folder();
        let err = combine_in(&mut h, "side").unwrap_err();
        let CombineError::Collisions { paths } = &err else {
            panic!("{path}: {err:?}");
        };
        assert_eq!(paths, &[path]);
        assert_eq!(
            err.to_string(),
            format!(
                "Your unsaved changes, or files Opalro ignores, are in the way of the combine, so \
                 nothing was changed:\n  {path}"
            )
        );
        assert!(
            err.hint()
                .unwrap()
                .ends_with("then run opalro combine again.")
        );
        let (head_now, entry_now) = h.entry();
        assert_eq!((head_now, &entry_now.state), (head, &entry.state), "{path}");
        assert_eq!(h.folder(), folder, "{path}");
    }
}

#[test]
fn undo_takes_a_line_moved_forward_back_and_redo_brings_it_again() {
    use crate::undo::{StepRequest, redo, undo};
    let mut h = Hist::new();
    let first = h.first;
    let base = h.save("main", first, &base_tree());
    let side = h.save("side", base, &side_tree());
    h.on("main", &base_tree());
    assert_eq!(
        combine_in(&mut h, "side").unwrap().result,
        Combined::Forward
    );
    let who = ada();
    let req = StepRequest {
        n: 1,
        shelve: false,
        who: &who,
        git_excludes: None,
    };
    undo(&mut h.repo, &req, &mut |_| {}).unwrap();
    assert_eq!(h.entry().1.state.lines["main"], base);
    assert_eq!(h.folder(), base_tree());
    redo(&mut h.repo, &req, &mut |_| {}).unwrap();
    assert_eq!(h.entry().1.state.lines["main"], side);
    assert_eq!(h.folder(), side_tree());
}

#[test]
fn undo_after_tidy_still_takes_a_combine_back() {
    use crate::undo::{StepRequest, redo, undo};
    use std::time::{SystemTime, UNIX_EPOCH};
    const DAY: i64 = 86_400_000;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let now = i64::try_from(now.as_millis()).unwrap();
    let mut h = Hist::new();
    let (_, main, _) = diverged(&mut h);
    let who = Person {
        time_ms: now,
        ..ada()
    };
    let to = crate::lines::resolve_target(&h.repo, "side").unwrap();
    let named = Named {
        raw: "side".into(),
        shown: "side".into(),
    };
    let asked = Asked {
        target: &to,
        named: &named,
        message: None,
        who: &who,
        git_excludes: None,
        keep: Keep::Conflicts,
    };
    let done = combine_with(&mut h.repo, &asked, &mut |_| {}, bounds()).unwrap();
    // Two days on, inside a 30-day window, and past the day a new pack is left alone.
    crate::tidy::tidy(&mut h.repo, 30, now + 2 * DAY, &mut |_| {}, &mut |_| {}).unwrap();
    let req = StepRequest {
        n: 1,
        shelve: false,
        who: &who,
        git_excludes: None,
    };
    undo(&mut h.repo, &req, &mut |_| {}).unwrap();
    assert_eq!(h.entry().1.state.lines["main"], main);
    assert_eq!(h.folder(), main_tree());
    redo(&mut h.repo, &req, &mut |_| {}).unwrap();
    assert_eq!(h.entry().1.state.lines["main"], done.save);
    assert_eq!(h.folder(), both_tree());
}

/// Two first saves holding `one` and `two`, then a save of `both` after them in each order:
/// two tips whose common starting points share no save.
fn two_roots(h: &mut Hist, one: &Tree, two: &Tree, both: &Tree) -> (Id, Id) {
    let none: &[(&str, &str)] = &[];
    let roots = [h.plant(&[], one, Some(none)), h.plant(&[], two, Some(none))];
    let m = h.plant(&roots, both, Some(none));
    (m, h.plant(&[roots[1], roots[0]], both, Some(none)))
}

#[test]
fn starting_points_with_no_save_in_common_are_unknown_where_they_differ() {
    let mut h = Hist::new();
    let one = tree(&[("same.txt", f("same\n")), ("one.txt", f("1\n"))]);
    let two = tree(&[("same.txt", f("same\n")), ("two.txt", f("2\n"))]);
    let mut both = one.clone();
    both.extend(two.clone());
    let tips_at = two_roots(&mut h, &one, &two, &both);
    // What both hold alike is in the start once, so changing it is a change, not two adds; what
    // only one holds is unknown, so main taking it away is no removal.
    let main = tree(&[("same.txt", f("main\n")), ("two.txt", f("2\n"))]);
    let side = with(&both, &[("s.txt", f("s\n"))]);
    tips(&mut h, tips_at, &main, &side);
    let (saved, conflicts, _) = kept_combine(&mut h);
    assert_eq!(
        conflicts,
        [kept_at("one.txt", Clash::StartUnknown, Stands::Nothing)]
    );
    assert_eq!(saved["same.txt"], f("main\n"));
    assert_eq!(saved["two.txt"], f("2\n"));
    let mut want = with(&main, &[("s.txt", f("s\n"))]);
    assert_eq!(h.folder(), want);

    // A file the two hold differently, which both lines then hold alike, is that.
    let mut h = Hist::new();
    let one = tree(&[("f.txt", f("one\n"))]);
    let two = tree(&[("f.txt", f("two\n"))]);
    let both = tree(&[("f.txt", f("both\n"))]);
    let tips_at = two_roots(&mut h, &one, &two, &both);
    let main = with(&both, &[("m.txt", f("m\n"))]);
    let side = with(&both, &[("s.txt", f("s\n"))]);
    tips(&mut h, tips_at, &main, &side);
    let done = combine_in(&mut h, "side").unwrap();
    let Combined::Made {
        starts, conflicts, ..
    } = &done.result
    else {
        panic!("{done:?}");
    };
    assert_eq!((starts.len(), conflicts.len()), (2, 0), "{conflicts:?}");
    want = with(&main, &[("s.txt", f("s\n"))]);
    assert_eq!(h.folder(), want);
}

#[test]
fn names_alike_inside_a_made_up_start_stop_nothing() {
    let mut h = Hist::new();
    let base = tree(&[("f.txt", f("f\n"))]);
    let b1 = with(&base, &[("Readme.md", f("one\n"))]);
    let b2 = with(&base, &[("README.md", f("two\n"))]);
    let both = with(&base, &[("Readme.md", f("one\n"))]);
    let crossed = crossed(&mut h, &base, &[b1, b2], &both);
    tips(
        &mut h,
        crossed,
        &with(&both, &[("m.txt", f("m\n"))]),
        &with(&both, &[("s.txt", f("s\n"))]),
    );
    let done = combine_in(&mut h, "side").unwrap();
    let Combined::Made { conflicts, .. } = &done.result else {
        panic!("{done:?}");
    };
    assert!(conflicts.is_empty(), "{conflicts:?}");
    let want = with(&both, &[("m.txt", f("m\n")), ("s.txt", f("s\n"))]);
    assert_eq!(h.folder(), want);
}

/// A listing whose folders `x` and `y` are one listing 4,097 folders deep.
fn too_deep(h: &mut Hist) -> Id {
    let lock = h.repo.lock_for_change("test", &mut |_| {}).unwrap();
    let mut packs = h.repo.new_packs().unwrap();
    let store = h.repo.store();
    let text = b"3";
    let file = Entry {
        name: "f".into(),
        kind: EntryKind::File,
        id: packs.add_content(text).unwrap(),
        size: text.len() as u64,
    };
    let mut at = Listing::new(vec![file]).unwrap();
    for _ in 1..4097 {
        let id = store_listing(store, &mut packs, &at).unwrap();
        at = Listing::new(vec![folder("d", id)]).unwrap();
    }
    let deep = store_listing(store, &mut packs, &at).unwrap();
    let top = Listing::new(vec![folder("x", deep), folder("y", deep)]).unwrap();
    let root = store_listing(store, &mut packs, &top).unwrap();
    h.repo.store_packs(&lock, packs).unwrap();
    root
}

#[test]
fn a_starting_point_past_the_limits_is_named_as_where_the_lines_parted() {
    let mut h = Hist::new();
    let first = h.first;
    let o = h.save("main", first, &tree(&[("x/f", f("1")), ("y/f", f("1"))]));
    let two = tree(&[("x/f", f("2")), ("y/f", f("2"))]);
    let b2 = h.save("start1", o, &two);
    let root = too_deep(&mut h);
    let b1 = h.plant_root(&[o], root, vec![]);
    let none: &[(&str, &str)] = &[];
    let m = h.plant(&[b1, b2], &two, Some(none));
    let s = h.plant(&[b2, b1], &two, Some(none));
    tips(
        &mut h,
        (m, s),
        &with(&two, &[("m.txt", f("m\n"))]),
        &with(&two, &[("s.txt", f("s\n"))]),
    );
    let before = h.untouched();
    let err = combine_in(&mut h, "side").unwrap_err();
    assert!(
        matches!(
            err,
            CombineError::Past {
                which: Which::Start,
                ..
            }
        ),
        "{err:?}"
    );
    let words = err.to_string();
    let parted = "The save where main and side parted is past Opalro's limits for a save (";
    assert!(words.starts_with(parted), "{words}");
    assert_untouched(&h, &before);
}

#[test]
fn a_too_large_folder_in_a_made_up_starting_point_reads_as_where_the_lines_parted() {
    let err = of_starts(CombineError::TooBig {
        detail: "a folder in it would hold more than 1,000,000 entries".into(),
    })
    .named(&Names {
        line: "main".into(),
        target: "side".into(),
    });
    assert_eq!(
        err.to_string(),
        "The save where main and side parted is past Opalro's limits for a save (a folder in it \
         would hold more than 1,000,000 entries), so nothing was changed."
    );
}

/// Starting points `c0`… of `b1` and `b2`, the starting points of `main` and `side`: `n` of
/// them, each adding a file of its own.
fn nested(h: &mut Hist, n: usize) {
    let first = h.first;
    let base = tree(&[("base.txt", f("base\n"))]);
    let o = h.save("main", first, &base);
    let mut all = base.clone();
    let cs: Vec<Id> = (0..n)
        .map(|i| {
            let t = with(&base, &[(&format!("{i}.txt"), f("i\n"))]);
            all.extend(t.clone());
            h.save(&format!("c{i}"), o, &t)
        })
        .collect();
    let back: Vec<Id> = cs.iter().rev().copied().collect();
    let none: &[(&str, &str)] = &[];
    let bs = [
        h.plant(&cs, &all, Some(none)),
        h.plant(&back, &all, Some(none)),
    ];
    let m = h.plant(&bs, &all, Some(none));
    let s = h.plant(&[bs[1], bs[0]], &all, Some(none));
    tips(
        h,
        (m, s),
        &with(&all, &[("m.txt", f("m\n"))]),
        &with(&all, &[("s.txt", f("s\n"))]),
    );
}

#[test]
fn the_budget_is_kept_inside_the_starting_points_own_starting_points() {
    let mut h = Hist::new();
    nested(&mut h, 2);
    let before = h.untouched();
    // One inner combine for `b1` and `b2`, and one for their own two starting points.
    let one = Bounds {
        max_inner: 1,
        ..bounds()
    };
    let err = combine_by(&mut h, "side", None, one).unwrap_err();
    assert!(matches!(err, CombineError::Tangled { .. }), "{err:?}");
    assert_untouched(&h, &before);
    let two = Bounds {
        max_inner: 2,
        ..bounds()
    };
    let done = combine_by(&mut h, "side", None, two).unwrap();
    assert!(matches!(done.result, Combined::Made { .. }), "{done:?}");
}

#[test]
fn too_many_starting_points_under_the_starting_points_are_refused() {
    let mut h = Hist::new();
    nested(&mut h, 17);
    let before = h.untouched();
    let err = combine_in(&mut h, "side").unwrap_err();
    assert!(matches!(err, CombineError::Tangled { .. }), "{err:?}");
    assert_untouched(&h, &before);
}

#[test]
fn moves_not_followed_while_combining_starting_points_are_told_once() {
    let mut h = Hist::new();
    let base = tree(&[("Player.cs", f(BODY)), ("Other.cs", f("other\n"))]);
    let b1 = moved_to("src/Player.cs", f(BODY));
    let b2 = tree(&[("Player.cs", f(BODY)), ("Other.cs", f("other, changed\n"))]);
    let both = tree(&[
        ("src/Player.cs", f(BODY)),
        ("Other.cs", f("other, changed\n")),
    ]);
    let crossed = crossed(&mut h, &base, &[b1, b2], &both);
    tips(
        &mut h,
        crossed,
        &with(&both, &[("m.txt", f("m\n"))]),
        &with(&both, &[("s.txt", f("s\n"))]),
    );
    let short = Bounds {
        max_left_bytes: HELD_PER_PLACE,
        ..bounds()
    };
    let done = combine_by(&mut h, "side", None, short).unwrap();
    let told: Vec<&str> = done
        .applied
        .warnings
        .iter()
        .filter(|w| w.kind == warning::MOVES_NOT_FOLLOWED)
        .map(|w| w.message.as_str())
        .collect();
    assert_eq!(
        told,
        [
            "Too many files left their places between the two lines' common starting points for \
          Opalro to match them by their contents, so files that moved there without a note were \
          not followed."
        ]
    );
}

#[test]
fn what_both_sides_hold_alike_is_taken_whatever_the_notes_say() {
    let mut h = Hist::new();
    let first = h.first;
    let (a, b) = (f("a text\n"), f("b text\n"));
    let s = h.save("main", first, &tree(&[("A.txt", a), ("B.txt", b.clone())]));
    // One line gives A.txt the contents of B.txt; the other removes A.txt, then moves B.txt
    // there, which its save notes.
    let o = h.save(
        "main",
        s,
        &tree(&[("A.txt", b.clone()), ("B.txt", b.clone())]),
    );
    let emptied = h.save("side", s, &tree(&[("B.txt", b.clone())]));
    let t = h.save("side", emptied, &tree(&[("A.txt", b.clone())]));
    let note = MoveNote {
        from: "B.txt".into(),
        to: "A.txt".into(),
    };
    assert_eq!(h.save_of(t).move_notes, [note]);
    let want = tree(&[("A.txt", b)]);
    assert_eq!(h.merged(s, o, t), want);
    assert_eq!(h.merged(s, t, o), want);
}

#[test]
fn a_side_past_the_limits_is_named_though_following_moves_meets_it_first() {
    let mut h = Hist::new();
    let first = h.first;
    let o = h.save("main", first, &tree(&[("x/f", f("1")), ("y/f", f("1"))]));
    h.save("side", o, &tree(&[("x/f", f("2")), ("y/f", f("2"))]));
    let root = too_deep(&mut h);
    let deep = h.plant_root(&[o], root, vec![]);
    h.set(|s| {
        s.lines.insert("main".into(), deep);
        s.position = Position::OnLine("main".into());
    });
    let before = h.untouched();
    let err = combine_in(&mut h, "side").unwrap_err();
    let words = err.to_string();
    let newest = "Line main's newest save is past Opalro's limits for a save (";
    assert!(words.starts_with(newest), "{words}");
    assert_untouched(&h, &before);
}

// Kept conflicts.

fn sides() -> Sides {
    Sides {
        ours: "main".into(),
        ours_save: ChangeId([1; 16]),
        theirs: "side".into(),
        theirs_save: ChangeId([2; 16]),
        parted: Parted::One(ChangeId([3; 16])),
    }
}

fn no_rules() -> RuleMatcher {
    RuleMatcher::new(&RulesRead::default(), &RulesRead::default())
}

/// A text marked as the merge marks one clash, between `before` and `after`.
fn marked(labels: &Labels, [before, ours, base, theirs, after]: [&str; 5]) -> Vec<u8> {
    format!(
        "{before}<<<<<<< {}\n{ours}||||||| {}\n{base}=======\n{theirs}>>>>>>> {}\n{after}",
        labels.ours, labels.base, labels.theirs
    )
    .into_bytes()
}

impl Fixture {
    /// The merge of three stored roots keeping conflicts, with `rules`; what it built is stored.
    fn keeping(&mut self, ids: [Id; 3], rules: &RuleMatcher) -> (Merged, Made) {
        let mut made = Made {
            keep: Some(sides()),
            ..Made::default()
        };
        let m = self.merge_with(ids, rules, false, &mut made).unwrap();
        (m, made)
    }

    fn kept(&mut self, s: &Tree, o: &Tree, t: &Tree) -> (Merged, Made) {
        let ids = self.three(s, o, t);
        self.keeping(ids, &no_rules())
    }

    /// The record of the conflict at `path` of the stored tree `root`.
    fn record(&self, root: Id, path: &str) -> Record {
        let V::Conflict(id) = self.read(root)[path] else {
            panic!("no conflict at {path}");
        };
        self.repo.store().conflict(&id).unwrap()
    }

    /// What each term of a record holds.
    fn terms(&self, r: &Record) -> Vec<Option<V>> {
        let store = self.repo.store();
        r.terms
            .iter()
            .map(|t| t.map(|t| leaf(store, t.kind, t.id)))
            .collect()
    }
}

/// The one conflict a merge made: at `path`, `clash`, standing as `stands`.
fn one_kept(m: &Merged, path: &str, clash: Clash, stands: Stands) {
    let want = Conflict {
        path: path.into(),
        clash,
        stands: Some(stands),
    };
    assert_eq!(m.conflicts, [want]);
    assert_eq!(m.more, 0);
}

#[test]
fn both_changed_text_is_kept_with_markers() {
    let mut fx = Fixture::new();
    let (s, o, t) = (f(BODY), edit(2, "main"), edit(2, "side"));
    let one = |v: &V| tree(&[("a.txt", v.clone())]);
    let (m, made) = fx.kept(&one(&s), &one(&o), &one(&t));
    let overlap = NotMerged::Overlap {
        endings_only: false,
    };
    one_kept(
        &m,
        "a.txt",
        Clash::BothChanged(overlap),
        Stands::Markers { places: 1 },
    );
    assert!(m.merged_files.is_empty());
    let top = fx.repo.store().listing(&m.root).unwrap();
    assert_eq!(top.get("a.txt").unwrap().kind, EntryKind::Conflict);
    let r = fx.record(m.root, "a.txt");
    // Yours, where the lines parted, theirs.
    assert_eq!(fx.terms(&r), [Some(o), Some(s), Some(t)]);
    let own = r.own.unwrap();
    assert_eq!(own.marker, 7);
    let labels = sides().labels(Parted::One(ChangeId([3; 16])));
    let want = marked(
        &labels,
        ["one\ntwo\n", "main\n", "three\n", "side\n", "four\nfive\n"],
    );
    let version = own.version.unwrap();
    assert_eq!(version.size, want.len() as u64);
    // Stored as the merge gave it, and it reads back with one marked place.
    let store = fx.repo.store();
    assert_eq!(
        leaf(store, version.kind, version.id),
        V::File(want.clone(), false)
    );
    let left = scan(&mut &want[..], own.marker).unwrap();
    assert_eq!((left.opening, left.closing), (1, 1));
    assert_eq!(made.records.values().collect::<Vec<_>>(), [&r]);
}

/// Three roots each holding `a.txt` as listed: `(contents, size)`.
fn listed(fx: &mut Fixture, three: [(&[u8], u64); 3]) -> [Id; 3] {
    let mut packs = fx.repo.new_packs().unwrap();
    let ids = three.map(|(bytes, size)| {
        let entry = Entry {
            name: "a.txt".into(),
            kind: EntryKind::File,
            id: packs.add_content(bytes).unwrap(),
            size,
        };
        let listing = Listing::new(vec![entry]).unwrap();
        store_listing(fx.repo.store(), &mut packs, &listing).unwrap()
    });
    fx.repo.store_packs(&fx.lock, packs).unwrap();
    ids
}

#[test]
fn both_changed_binary_stands_as_yours() {
    let mut fx = Fixture::new();
    let lines = "\n".repeat(1_000_001);
    let cases = [
        (["a\0\n", "b\0\n", "c\0\n"].map(f), NotMerged::Binary),
        (
            [lines.clone(), format!("{lines}o\n"), format!("{lines}t\n")].map(|t| f(&t)),
            NotMerged::TooManyLines,
        ),
        (
            ["x\n", "y\r\n", "y\n"].map(f),
            NotMerged::Overlap { endings_only: true },
        ),
    ];
    for ([s, o, t], why) in cases {
        let one = |v: &V| tree(&[("a.txt", v.clone())]);
        let (m, _) = fx.kept(&one(&s), &one(&o), &one(&t));
        one_kept(&m, "a.txt", Clash::BothChanged(why), Stands::Yours);
        let r = fx.record(m.root, "a.txt");
        assert_eq!(r.own, None, "{why:?}");
        assert_eq!(fx.terms(&r), [Some(o), Some(s), Some(t)]);
    }
    // Too big: listed past 64 MiB, so nothing is read.
    let big = crate::changes::MAX_LINES_SIZE + 1;
    let ids = listed(&mut fx, [(b"s\n", big), (b"o\n", big), (b"t\n", big)]);
    let (m, _) = fx.keeping(ids, &no_rules());
    one_kept(
        &m,
        "a.txt",
        Clash::BothChanged(NotMerged::TooBig),
        Stands::Yours,
    );
    assert_eq!(fx.record(m.root, "a.txt").own, None);
}

/// The rules `text` gives, read whole.
fn rules_of(text: &str) -> RulesRead {
    let (rules, _) = crate::rules::parse(text.as_bytes()).unwrap();
    RulesRead {
        rules,
        ..RulesRead::default()
    }
}

#[test]
fn a_rule_on_either_line_saying_no_markers_keeps_it_as_yours() {
    let mut fx = Fixture::new();
    let scene = rules_of(SCENE_RULE);
    let none = RulesRead::default();
    let one = |v: V| tree(&[("Main.unity", v)]);
    let ids = fx.three(&one(f(BODY)), &one(edit(2, "main")), &one(edit(2, "side")));
    for (rules, theirs) in [
        (RuleMatcher::new(&scene, &none), false),
        (RuleMatcher::new(&none, &scene), true),
    ] {
        let (m, _) = fx.keeping(ids, &rules);
        let why = NotMerged::NoMarkers { theirs };
        one_kept(&m, "Main.unity", Clash::BothChanged(why), Stands::Yours);
        assert_eq!(fx.record(m.root, "Main.unity").own, None);
    }
}

impl Hist {
    /// The merge of three saves keeping conflicts, with `rules`; what it built is stored.
    fn kept(&mut self, s: Id, o: Id, t: Id, rules: &RuleMatcher) -> Merged {
        let [s, o, t] = [s, o, t].map(|id| self.version(id));
        let lock = self.repo.lock_for_change("test", &mut |_| {}).unwrap();
        let mut packs = self.repo.new_packs().unwrap();
        let mut made = Made {
            keep: Some(sides()),
            ..Made::default()
        };
        let m = merge_three(&self.repo, &mut packs, &mut made, false, &s, &o, &t, rules).unwrap();
        self.repo.store_packs(&lock, packs).unwrap();
        m
    }
}

#[test]
fn a_moved_file_is_marked_only_when_both_its_names_allow_it() {
    let mut h = Hist::new();
    let first = h.first;
    let s = h.save("main", first, &tree(&[("Main.unity", f(BODY))]));
    let o = h.save("main", s, &tree(&[("Main.txt", f(BODY))]));
    let o = h.save("main", o, &tree(&[("Main.txt", edit(0, "ONE"))]));
    let t = h.save("side", s, &tree(&[("Main.unity", edit(0, "UNO"))]));
    let overlap = Clash::BothChanged(NotMerged::Overlap {
        endings_only: false,
    });
    let m = h.kept(s, o, t, &no_rules());
    one_kept(&m, "Main.txt", overlap, Stands::Markers { places: 1 });
    // Its old name is a scene's, which no line merges: no markers.
    let no = NotMerged::NoMarkers { theirs: false };
    let m = h.kept(s, o, t, &scene_rule());
    one_kept(&m, "Main.txt", Clash::BothChanged(no), Stands::Yours);
}

#[test]
fn changed_and_removed_stands_as_your_version_or_nothing() {
    let mut fx = Fixture::new();
    let s = tree(&[("a.txt", f(BODY))]);
    let changed = tree(&[("a.txt", edit(0, "changed"))]);
    let gone = Tree::new();
    let removed_on = |side| Clash::ChangedRemoved { removed_on: side };
    let (m, _) = fx.kept(&s, &changed, &gone);
    one_kept(&m, "a.txt", removed_on(Side::Theirs), Stands::Yours);
    let r = fx.record(m.root, "a.txt");
    assert_eq!(r.own, None);
    assert_eq!(
        fx.terms(&r),
        [Some(edit(0, "changed")), Some(f(BODY)), None]
    );
    let (m, _) = fx.kept(&s, &gone, &changed);
    one_kept(&m, "a.txt", removed_on(Side::Ours), Stands::Nothing);
    let r = fx.record(m.root, "a.txt");
    assert_eq!(r.own, None);
    assert_eq!(
        fx.terms(&r),
        [None, Some(f(BODY)), Some(edit(0, "changed"))]
    );
    assert_eq!(stands_as(&r), None);
}

#[test]
fn both_added_text_gets_markers_with_an_empty_middle() {
    let mut fx = Fixture::new();
    let (m, _) = fx.kept(
        &Tree::new(),
        &tree(&[("a.txt", x("a\n"))]),
        &tree(&[("a.txt", f("b\n"))]),
    );
    one_kept(&m, "a.txt", Clash::BothAdded, Stands::Markers { places: 1 });
    let r = fx.record(m.root, "a.txt");
    assert_eq!(fx.terms(&r), [Some(x("a\n")), None, Some(f("b\n"))]);
    let labels = sides().labels(Parted::Nothing(Some(ChangeId([3; 16]))));
    assert!(labels.base.ends_with(", which held nothing here"));
    let want = marked(&labels, ["", "a\n", "", "b\n", ""]);
    // The marked text is ours' kind.
    let version = r.own.unwrap().version.unwrap();
    let store = fx.repo.store();
    assert_eq!(leaf(store, version.kind, version.id), V::File(want, true));
}

#[test]
fn both_added_binary_stands_as_yours() {
    let mut fx = Fixture::new();
    // Not text, and the same text but for the executable mark.
    for (o, t) in [(f("a\0"), f("b\0")), (x("a\n"), f("a\n"))] {
        let (m, _) = fx.kept(
            &Tree::new(),
            &tree(&[("a.bin", o.clone())]),
            &tree(&[("a.bin", t.clone())]),
        );
        one_kept(&m, "a.bin", Clash::BothAdded, Stands::Yours);
        let r = fx.record(m.root, "a.bin");
        assert_eq!(r.own, None);
        assert_eq!(fx.terms(&r), [Some(o), None, Some(t)]);
    }
}

#[test]
fn a_file_against_a_folder_holds_the_folder_whole() {
    let mut fx = Fixture::new();
    let folder = tree(&[("ui/a.txt", f("a\n")), ("ui/deep/b.txt", f("b\n"))]);
    let file = tree(&[("ui", f("ui\n"))]);
    for (o, t, folder_on) in [(&folder, &file, Side::Ours), (&file, &folder, Side::Theirs)] {
        let ids = fx.three(&Tree::new(), o, t);
        let (m, _) = fx.keeping(ids, &no_rules());
        one_kept(&m, "ui", Clash::FileAndFolder { folder_on }, Stands::Yours);
        // Nothing below the place is a conflict of its own.
        let result = fx.read(m.root);
        assert_eq!(result.keys().collect::<Vec<_>>(), ["ui"]);
        let r = fx.record(m.root, "ui");
        let (at, side) = if folder_on == Side::Ours {
            (0, ids[1])
        } else {
            (2, ids[2])
        };
        let held = fx
            .repo
            .store()
            .listing(&side)
            .unwrap()
            .get("ui")
            .unwrap()
            .id;
        let term = r.terms[at].unwrap();
        assert_eq!(
            (term.kind, term.id, term.size),
            (EntryKind::Folder, held, 0)
        );
        assert_eq!(r.terms[1], None);
        assert_eq!(r.own, None);
    }
}

#[test]
fn two_links_that_differ_are_kept_standing_as_yours() {
    let mut fx = Fixture::new();
    for [s, o, t] in [
        [link("x"), link("y"), link("z")],
        [V::Project(1), V::Project(2), V::Project(3)],
    ] {
        let one = |v: &V| tree(&[("l", v.clone())]);
        let (m, _) = fx.kept(&one(&s), &one(&o), &one(&t));
        let not_files = Clash::BothChanged(NotMerged::NotFiles);
        one_kept(&m, "l", not_files, Stands::Yours);
        let r = fx.record(m.root, "l");
        assert_eq!(fx.terms(&r), [Some(o), Some(s), Some(t)]);
        assert_eq!(r.own, None);
    }
}

#[test]
fn the_marked_text_has_the_executable_mark_the_merge_gives() {
    let mut fx = Fixture::new();
    let marked_as = |v: V, mark: bool| match v {
        V::File(bytes, _) => V::File(bytes, mark),
        v => v,
    };
    // Ours made it executable; then theirs took the mark away.
    for (marks, want) in [([false, true, false], true), ([true, true, false], false)] {
        let three = [f(BODY), edit(2, "main"), edit(2, "side")];
        let [s, o, t] = [0, 1, 2].map(|i| marked_as(three[i].clone(), marks[i]));
        let one = |v: &V| tree(&[("a.txt", v.clone())]);
        let (m, _) = fx.kept(&one(&s), &one(&o), &one(&t));
        let r = fx.record(m.root, "a.txt");
        let version = r.own.unwrap().version.unwrap();
        let kind = if want {
            EntryKind::Executable
        } else {
            EntryKind::File
        };
        assert_eq!(version.kind, kind, "{marks:?}");
    }
}

const BROKEN: &str = "[[rule]\n";

/// `a.txt` changed on both lines: on `side` at line `side_at` (0, far from `main`'s change; 4,
/// where it is), `side` holding `side_rules` too; the project is on `main`.
fn text_both_changed(h: &mut Hist, side_at: usize, side_rules: &[(&str, V)]) {
    let first = h.first;
    let base = h.save("main", first, &tree(&[("a.txt", f(BODY))]));
    let side = with(&tree(&[("a.txt", edit(side_at, "side"))]), side_rules);
    let side = h.plant(&[base], &side, Some(&[]));
    h.set(|s| {
        s.lines.insert("side".into(), side);
    });
    h.save("main", base, &tree(&[("a.txt", edit(4, "main"))]));
}

/// The places a combine stopped at because rules can't be read whole, and whose rules.
fn undecided(err: &CombineError) -> (Vec<&str>, Side) {
    let CombineError::Conflicts {
        conflicts,
        more: 0,
        rules_not_whole: Some(side),
        why: NotKept::RulesNotWhole,
        ..
    } = err
    else {
        panic!("{err:?}");
    };
    (conflicts.iter().map(|c| c.path.as_str()).collect(), *side)
}

#[test]
fn while_the_rules_are_not_read_whole_a_text_both_changed_stops_the_combine() {
    // Changes that would merge, and changes that overlap.
    for side_at in [0, 4] {
        let mut h = Hist::new();
        text_both_changed(&mut h, side_at, &[]);
        rules_file(&h, BROKEN);
        let before = h.untouched();
        let err = combine_in(&mut h, "side").unwrap_err();
        assert_eq!(undecided(&err), (vec!["a.txt"], Side::Ours));
        assert_eq!(
            err.to_string(),
            "Combining side into main would have to merge 1 place that opalro.toml decides, and \
             it can't be read whole, so nothing was changed:\n  a.txt: changed on both lines, \
             and opalro.toml can't be read whole, so no file is merged line by line"
        );
        assert_eq!(err.hint().unwrap(), RULES_HINT);
        assert_untouched(&h, &before);
        // The target's file.
        let mut h = Hist::new();
        text_both_changed(&mut h, side_at, &[("opalro.toml", f(BROKEN))]);
        let before = h.untouched();
        let err = combine_in(&mut h, "side").unwrap_err();
        assert_eq!(undecided(&err), (vec!["a.txt"], Side::Theirs));
        assert_eq!(
            err.to_string(),
            "Combining side into main would have to merge 1 place that opalro.toml decides, and \
             the opalro.toml on side can't be read whole, so nothing was changed:\n  a.txt: \
             changed on both lines, and opalro.toml on side can't be read whole, so no file is \
             merged line by line"
        );
        assert_eq!(err.hint().unwrap(), THEIRS_HINT);
        assert_untouched(&h, &before);
    }
}

#[test]
fn while_the_rules_are_not_read_whole_a_picture_is_still_kept_as_yours() {
    let mut h = Hist::new();
    let first = h.first;
    let base = h.save("main", first, &tree(&[("pic.png", f("base\0"))]));
    h.save("side", base, &tree(&[("pic.png", f("side\0"))]));
    h.save("main", base, &tree(&[("pic.png", f("main\0"))]));
    rules_file(&h, BROKEN);
    let (_, made, count) = kept_combine(&mut h);
    let want = Conflict {
        path: "pic.png".into(),
        clash: Clash::BothChanged(NotMerged::Binary),
        stands: Some(Stands::Yours),
    };
    assert_eq!((made, count), (vec![want], 1));
    assert_eq!(h.folder()["pic.png"], f("main\0"));
}

#[test]
fn the_stop_names_only_the_places_the_rules_would_decide() {
    let mut h = Hist::new();
    let first = h.first;
    let three = |a: V, b: V, pic: &str| tree(&[("a.txt", a), ("b.txt", b), ("pic.png", f(pic))]);
    let base = h.save("main", first, &three(f(BODY), f(BODY), "base\0"));
    let side = three(edit(4, "side"), edit(0, "side"), "side\0");
    h.save("side", base, &side);
    let main = three(edit(4, "main"), edit(4, "main"), "main\0");
    h.save("main", base, &main);
    rules_file(&h, BROKEN);
    let before = h.untouched();
    let err = combine_in(&mut h, "side").unwrap_err();
    // One overlaps, one would merge; the picture is no rule's to decide.
    assert_eq!(undecided(&err), (vec!["a.txt", "b.txt"], Side::Ours));
    let words = err.to_string();
    let first_line = "Combining side into main would have to merge 2 places that opalro.toml";
    assert!(words.starts_with(first_line), "{words}");
    assert!(!words.contains("pic.png"), "{words}");
    assert_untouched(&h, &before);
}

/// Stores a conflict record of three files holding these bytes, and the files; gives its ID.
fn plant_record(h: &mut Hist, versions: [&[u8]; 3]) -> Id {
    let lock = h.repo.lock_for_change("test", &mut |_| {}).unwrap();
    let mut packs = h.repo.new_packs().unwrap();
    let terms = versions
        .iter()
        .map(|bytes| {
            Some(Term {
                kind: EntryKind::File,
                id: packs.add_content(bytes).unwrap(),
                size: bytes.len() as u64,
            })
        })
        .collect();
    let record = Record { terms, own: None };
    let id = packs
        .add_record(ObjectKind::Conflict, &record.encode())
        .unwrap();
    h.repo.store_packs(&lock, packs).unwrap();
    id
}

/// `t` as the folder holds it: each conflict as what it stands as.
fn standing(h: &Hist, t: &Tree) -> Tree {
    let store = h.repo.store();
    let mut out = Tree::new();
    for (path, v) in t {
        let v = match v {
            V::Conflict(id) => match stands_as(&store.conflict(id).unwrap()) {
                Some(term) => leaf(store, term.kind, term.id),
                None => continue,
            },
            v => v.clone(),
        };
        out.insert(path.clone(), v);
    }
    out
}

/// Plants `base`, then `main` and `side` after it, and puts the project on `main`.
fn planted_lines(h: &mut Hist, base: &Tree, main: &Tree, side: &Tree) {
    let first = h.first;
    let b = h.plant(&[first], base, Some(&[]));
    let o = h.plant(&[b], main, Some(&[]));
    let t = h.plant(&[b], side, Some(&[]));
    h.set(|s| {
        s.lines.insert("side".into(), t);
        s.lines.insert("main".into(), o);
    });
    let folder = standing(h, main);
    h.on("main", &folder);
}

/// Combines `side` into `main`, keeping conflicts: the combined save's tree, the conflicts the
/// combine made, and the count of all it holds.
fn kept_combine(h: &mut Hist) -> (Tree, Vec<Conflict>, u64) {
    let done = combine_in(h, "side").unwrap();
    let Combined::Made {
        conflicts,
        more: 0,
        conflict_count,
        ..
    } = done.result
    else {
        panic!("{:?}", done.result);
    };
    let saved = read_tree(h.repo.store(), h.save_of(done.save).root, &HashMap::new());
    (saved, conflicts, conflict_count)
}

#[test]
fn a_conflict_both_lines_hold_alike_is_carried() {
    let mut h = Hist::new();
    let c = plant_record(&mut h, [b"mine\n", b"was\n", b"theirs\n"]);
    let held = ("x.txt", V::Conflict(c));
    let base = tree(&[held.clone(), ("a.txt", f(BODY))]);
    let main = tree(&[held.clone(), ("a.txt", edit(4, "main"))]);
    let side = tree(&[held, ("a.txt", edit(0, "side"))]);
    planted_lines(&mut h, &base, &main, &side);
    let (saved, made, count) = kept_combine(&mut h);
    assert_eq!(saved["x.txt"], V::Conflict(c));
    assert_eq!((made, count), (vec![], 1));
    assert_eq!(h.folder()["x.txt"], f("mine\n"));
}

#[test]
fn a_conflict_one_line_fixed_is_taken_fixed() {
    let mut h = Hist::new();
    let c = plant_record(&mut h, [b"mine\n", b"was\n", b"theirs\n"]);
    let base = tree(&[("x.txt", V::Conflict(c)), ("a.txt", f(BODY))]);
    let main = tree(&[("x.txt", f("fixed\n")), ("a.txt", edit(4, "main"))]);
    let side = tree(&[("x.txt", V::Conflict(c)), ("a.txt", edit(0, "side"))]);
    planted_lines(&mut h, &base, &main, &side);
    let (saved, made, count) = kept_combine(&mut h);
    assert_eq!(saved["x.txt"], f("fixed\n"));
    assert_eq!((made, count), (vec![], 0));
}

#[test]
fn a_conflict_made_on_one_line_only_is_carried() {
    let mut h = Hist::new();
    let c = plant_record(&mut h, [b"mine\n", b"was\n", b"theirs\n"]);
    let base = tree(&[("x.txt", f("was\n")), ("a.txt", f(BODY))]);
    let main = tree(&[("x.txt", f("was\n")), ("a.txt", edit(4, "main"))]);
    let side = tree(&[("x.txt", V::Conflict(c)), ("a.txt", edit(0, "side"))]);
    planted_lines(&mut h, &base, &main, &side);
    let (saved, made, count) = kept_combine(&mut h);
    assert_eq!(saved["x.txt"], V::Conflict(c));
    assert_eq!((made, count), (vec![], 1));
    assert_eq!(h.folder()["x.txt"], f("mine\n"));
}

const HELD_HINT: &str =
    "Fix it on the line that holds it first (opalro resolve there), then run opalro combine again.";

#[test]
fn both_lines_changed_a_place_one_holds_in_conflict_stops_naming_it() {
    let one = "x.txt is in conflict on main, and side changed it too, so nothing was changed.";
    let other = "x.txt is in conflict on side, and main changed it too, so nothing was changed.";
    let both = "x.txt is in conflict on both main and side, so nothing was changed.";
    let both_hint =
        "Fix it on both lines first (opalro resolve on each), then run opalro combine again.";
    for case in 0..3 {
        let mut h = Hist::new();
        let c = plant_record(&mut h, [b"mine\n", b"was\n", b"theirs\n"]);
        let d = plant_record(&mut h, [b"ours\n", b"was\n", b"other\n"]);
        let (main_x, side_x, on, words, hint) = match case {
            0 => (V::Conflict(c), f("side\n"), Held::Ours, one, HELD_HINT),
            1 => (f("main\n"), V::Conflict(c), Held::Theirs, other, HELD_HINT),
            _ => (V::Conflict(c), V::Conflict(d), Held::Both, both, both_hint),
        };
        let base = tree(&[("x.txt", f("was\n"))]);
        let (main, side) = (tree(&[("x.txt", main_x)]), tree(&[("x.txt", side_x)]));
        planted_lines(&mut h, &base, &main, &side);
        let before = h.untouched();
        let err = combine_in(&mut h, "side").unwrap_err();
        let CombineError::HeldConflict {
            path, at, on: by, ..
        } = &err
        else {
            panic!("{err:?}");
        };
        assert_eq!((path.as_str(), at.as_str(), *by), ("x.txt", "x.txt", on));
        assert_eq!(err.to_string(), words);
        assert_eq!(err.hint().unwrap(), hint);
        assert_untouched(&h, &before);
    }
}

#[test]
fn a_folder_holding_a_conflict_below_stops_the_combine_too() {
    for ours_holds in [true, false] {
        let mut h = Hist::new();
        let c = plant_record(&mut h, [b"mine\n", b"was\n", b"theirs\n"]);
        let base = tree(&[("ui/a.txt", f("was\n"))]);
        let held = tree(&[("ui/a.txt", V::Conflict(c))]);
        let file = tree(&[("ui", f("a file now\n"))]);
        let (main, side, on, words) = if ours_holds {
            let words = "ui/a.txt, inside ui, is in conflict on main, and side changed ui too, \
                         so nothing was changed.";
            (&held, &file, Held::Ours, words)
        } else {
            let words = "ui/a.txt, inside ui, is in conflict on side, and main changed ui too, \
                         so nothing was changed.";
            (&file, &held, Held::Theirs, words)
        };
        planted_lines(&mut h, &base, main, side);
        let before = h.untouched();
        let err = combine_in(&mut h, "side").unwrap_err();
        let CombineError::HeldConflict {
            path, at, on: by, ..
        } = &err
        else {
            panic!("{err:?}");
        };
        assert_eq!((path.as_str(), at.as_str(), *by), ("ui", "ui/a.txt", on));
        assert_eq!(err.to_string(), words);
        assert_eq!(err.hint().unwrap(), HELD_HINT);
        assert_untouched(&h, &before);
    }
}

#[test]
fn a_folder_with_no_conflict_below_is_kept_whole() {
    let mut fx = Fixture::new();
    let o = tree(&[
        ("ui/a.txt", f("a\n")),
        ("ui/deep/b.txt", f("b\n")),
        ("z/c.txt", f("c\n")),
    ]);
    let t = tree(&[("ui", f("ui\n")), ("z/c.txt", f("c\n"))]);
    let ids = fx.three(&Tree::new(), &o, &t);
    let read = |fx: &mut Fixture, keep| {
        let mut made = Made {
            keep,
            ..Made::default()
        };
        let before = counted::listings();
        let m = fx.merge_with(ids, &no_rules(), false, &mut made).unwrap();
        (m, counted::listings() - before)
    };
    let (_, clean) = read(&mut fx, None);
    let (m, kept) = read(&mut fx, Some(sides()));
    let clash = Clash::FileAndFolder {
        folder_on: Side::Ours,
    };
    one_kept(&m, "ui", clash, Stands::Yours);
    // `ui` and `ui/deep`, and nothing outside the folder.
    assert_eq!(kept - clean, 2);
}

#[test]
fn the_start_holding_a_conflict_both_lines_changed_is_unknown() {
    let mut fx = Fixture::new();
    // Its record is never read.
    let start = tree(&[("x.txt", V::Conflict(Id([9; 32])))]);
    let one = |v: V| tree(&[("x.txt", v)]);
    let (m, _) = fx.kept(&start, &one(f("main\n")), &one(f("side\n")));
    one_kept(
        &m,
        "x.txt",
        Clash::StartUnknown,
        Stands::Markers { places: 1 },
    );
    let r = fx.record(m.root, "x.txt");
    assert_eq!(fx.terms(&r), [Some(f("main\n")), None, Some(f("side\n"))]);
    let labels = sides().labels(Parted::Unknown);
    assert!(labels.base.ends_with(", which Opalro can't tell here"));
    let version = r.own.unwrap().version.unwrap();
    let want = marked(&labels, ["", "main\n", "", "side\n", ""]);
    let store = fx.repo.store();
    assert_eq!(leaf(store, version.kind, version.id), V::File(want, false));
    assert_eq!(
        m.conflicts[0].describe("main", "side"),
        "x.txt: changed on both lines, and Opalro can't tell what it held where they parted"
    );
    // Not text: as yours.
    let (m, _) = fx.kept(&start, &one(f("main\0")), &one(f("side\0")));
    one_kept(&m, "x.txt", Clash::StartUnknown, Stands::Yours);
    // Alike on both lines: that is the result.
    let same = one(f("same\n"));
    let (m, _) = fx.kept(&start, &same, &same);
    assert!(m.conflicts.is_empty());
    assert_eq!(fx.read(m.root), same);
}

#[test]
fn the_count_is_every_kind_6_entry_in_the_result() {
    let mut h = Hist::new();
    let c = plant_record(&mut h, [b"mine\n", b"was\n", b"theirs\n"]);
    let held = ("x.txt", V::Conflict(c));
    let base = tree(&[held.clone(), ("a.txt", f(BODY))]);
    let main = tree(&[held.clone(), ("a.txt", edit(2, "main"))]);
    let side = tree(&[held, ("a.txt", edit(2, "side"))]);
    planted_lines(&mut h, &base, &main, &side);
    let done = combine_in(&mut h, "side").unwrap();
    let Combined::Made {
        conflicts,
        conflict_count,
        ..
    } = &done.result
    else {
        panic!("{:?}", done.result);
    };
    let paths: Vec<&str> = conflicts.iter().map(|c| c.path.as_str()).collect();
    // One made, one carried.
    assert_eq!((paths, *conflict_count), (vec!["a.txt"], 2));
    // No count goes into the save.
    let save = h.save_of(done.save);
    assert_eq!(save.extra.keys().collect::<Vec<_>>(), [EXTRA_LINE]);
}

#[test]
fn labels_name_the_line_and_target_and_their_saves() {
    let mut h = Hist::new();
    in_conflict(&mut h);
    let lines = h.entry().1.state.lines;
    let (main, side) = (lines["main"], lines["side"]);
    let base = h.save_of(main).parents[0];
    let letters = |id: Id| h.save_of(id).change_id.to_letters()[..8].to_owned();
    let (main_l, base_l, side_l) = (letters(main), letters(base), letters(side));
    let (saved, _, _) = kept_combine(&mut h);
    let V::Conflict(id) = saved["a.txt"] else {
        panic!("{saved:?}");
    };
    let store = h.repo.store();
    let version = store.conflict(&id).unwrap().own.unwrap().version.unwrap();
    let V::File(text, _) = leaf(store, version.kind, version.id) else {
        panic!("not a file");
    };
    let text = String::from_utf8(text).unwrap();
    for line in [
        format!("<<<<<<< main ({main_l})\n"),
        format!("||||||| where the lines parted ({base_l})\n"),
        format!(">>>>>>> side ({side_l})\n"),
    ] {
        assert!(text.contains(&line), "{text}");
    }
    // The folder holds the marked text.
    assert_eq!(h.folder()["a.txt"], f(&text));
    h.assert_as_goto_gives();
}

#[test]
fn without_the_feature_a_conflict_still_stops_the_combine_and_stores_no_record() {
    let dir = tempfile::tempdir().unwrap();
    Repository::init(dir.path(), &ada()).unwrap();
    let repo = crate::test_support::without_conflicts_feature(dir.path());
    let first = repo.current().unwrap().1.state.lines["main"];
    let mut h = Hist {
        dir,
        repo,
        first,
        keep: Keep::Conflicts,
    };
    in_conflict(&mut h);
    let before = h.untouched();
    let err = combine_in(&mut h, "side").unwrap_err();
    let no_feature = matches!(
        err,
        CombineError::Conflicts {
            why: NotKept::NoFeature,
            ..
        }
    );
    assert!(no_feature, "{err:?}");
    assert_eq!(
        err.to_string(),
        "Combining side into main would leave 1 place in conflict, which this project can't \
         keep yet, so nothing was changed:\n  a.txt: changed on both lines, in the same place"
    );
    // No pack, so no record, was stored, and nothing is left in `tmp`.
    assert_untouched(&h, &before);
}

#[test]
fn every_kept_record_reads_back_and_names_stored_versions() {
    let mut h = Hist::new();
    let first = h.first;
    let all = |a: V, pic: &str, gone: Option<&str>, ui: V| {
        let mut t = tree(&[("a.txt", a), ("pic.png", f(pic)), ("ui/x.txt", f("x\n"))]);
        if let Some(g) = gone {
            t.insert("gone.txt".into(), f(g));
        }
        if let V::File(..) = ui {
            t.remove("ui/x.txt");
            t.insert("ui".into(), ui);
        }
        t
    };
    let base = h.save("main", first, &all(f(BODY), "b\0", Some("g\n"), V::Folder));
    let side = all(edit(2, "side"), "s\0", None, f("ui\n"));
    h.save("side", base, &side);
    let mut main = all(edit(2, "main"), "m\0", Some("g, changed\n"), V::Folder);
    main.insert("ui/x.txt".into(), f("x, changed\n"));
    h.save("main", base, &main);
    let (saved, made, count) = kept_combine(&mut h);
    assert_eq!((made.len(), count), (4, 4), "{made:?}");
    let store = h.repo.store();
    for (path, v) in &saved {
        let V::Conflict(id) = v else { continue };
        let r = store.conflict(id).unwrap();
        let own = r.own.and_then(|own| own.version);
        for term in r.terms.iter().flatten().chain(&own) {
            if term.kind == EntryKind::Folder {
                store.listing(&term.id).unwrap();
            } else {
                let mut bytes = Vec::new();
                store.copy_content(&term.id, &mut bytes).unwrap();
                assert_eq!(bytes.len() as u64, term.size, "{path}");
            }
        }
    }
    h.assert_as_goto_gives();
}

/// Ours holding the same folder (one listing) at each of `n` places;
/// theirs a file at each of those places.
fn folders_against_files(n: usize) -> [Tree; 2] {
    let mut o = Tree::new();
    let mut t = Tree::new();
    for i in 0..n {
        o.insert(format!("p{i}/a.txt"), f("a\n"));
        o.insert(format!("p{i}/deep/b.txt"), f("b\n"));
        t.insert(format!("p{i}"), f("a file\n"));
    }
    [o, t]
}

#[test]
fn a_folder_held_at_many_places_is_checked_once() {
    let mut fx = Fixture::new();
    let [o, t] = folders_against_files(10);
    let ids = fx.three(&Tree::new(), &o, &t);
    let read = |fx: &mut Fixture, keep| {
        let mut made = Made {
            keep,
            ..Made::default()
        };
        let before = counted::listings();
        let m = fx.merge_with(ids, &no_rules(), false, &mut made).unwrap();
        (m, counted::listings() - before)
    };
    let (_, clean) = read(&mut fx, None);
    let (m, kept) = read(&mut fx, Some(sides()));
    assert_eq!(m.conflicts.len(), 10);
    // The folder and the one in it, once for all ten places.
    assert_eq!(kept - clean, 2);
}

#[test]
fn the_checks_of_one_version_together_are_held_to_its_limits() {
    let mut fx = Fixture::new();
    // Two different folders of three entries each, against a limit of five.
    let o = tree(&[
        ("p/a.txt", f("a\n")),
        ("p/b.txt", f("b\n")),
        ("p/c.txt", f("c\n")),
        ("q/a.txt", f("d\n")),
        ("q/b.txt", f("e\n")),
        ("q/c.txt", f("f\n")),
    ]);
    let t = tree(&[("p", f("p\n")), ("q", f("q\n"))]);
    let ids = fx.three(&Tree::new(), &o, &t);
    let merge = |fx: &mut Fixture, max_below| {
        let mut made = Made {
            keep: Some(sides()),
            max_below,
            ..Made::default()
        };
        fx.merge_with(ids, &no_rules(), false, &mut made)
    };
    assert_eq!(merge(&mut fx, 6).unwrap().conflicts.len(), 2);
    let err = merge(&mut fx, 5).unwrap_err();
    assert!(
        matches!(
            err,
            CombineError::Past {
                which: Which::Ours,
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn without_the_feature_a_conflict_entry_on_a_line_is_milestone_8s_clash() {
    let dir = tempfile::tempdir().unwrap();
    Repository::init(dir.path(), &ada()).unwrap();
    let repo = crate::test_support::without_conflicts_feature(dir.path());
    let first = repo.current().unwrap().1.state.lines["main"];
    let mut h = Hist {
        dir,
        repo,
        first,
        keep: Keep::Conflicts,
    };
    let c = plant_record(&mut h, [b"mine\n", b"was\n", b"theirs\n"]);
    let base = tree(&[("x.txt", f("was\n"))]);
    let main = tree(&[("x.txt", V::Conflict(c))]);
    let side = tree(&[("x.txt", f("side\n"))]);
    planted_lines(&mut h, &base, &main, &side);
    let before = h.untouched();
    let err = combine_in(&mut h, "side").unwrap_err();
    assert_eq!(
        err.to_string(),
        "Combining side into main would leave 1 place in conflict, which this project can't \
         keep yet, so nothing was changed:\n  x.txt: changed on both lines, and only files are \
         merged line by line"
    );
    assert_untouched(&h, &before);
}

#[test]
fn files_both_added_alike_but_for_the_mark_are_no_question_for_the_rules() {
    let mut fx = Fixture::new();
    let broken = RulesRead {
        skipped: Some("it isn't TOML".into()),
        ..RulesRead::default()
    };
    let none = RulesRead::default();
    let ids = fx.three(
        &Tree::new(),
        &tree(&[("a.sh", x("echo\n"))]),
        &tree(&[("a.sh", f("echo\n"))]),
    );
    for rules in [
        no_rules(),
        RuleMatcher::new(&broken, &none),
        RuleMatcher::new(&none, &broken),
    ] {
        let (m, _) = fx.keeping(ids, &rules);
        one_kept(&m, "a.sh", Clash::BothAdded, Stands::Yours);
        assert_eq!(m.rules_not_whole, None);
    }
}

#[test]
fn while_the_rules_are_not_read_whole_text_both_added_stops_the_combine() {
    let added = |h: &mut Hist, side_rules: &[(&str, V)]| {
        let first = h.first;
        let base = h.save("main", first, &tree(&[("b.txt", f("b\n"))]));
        let side = with(
            &tree(&[("b.txt", f("b\n")), ("a.txt", f("side\n"))]),
            side_rules,
        );
        let side = h.plant(&[base], &side, Some(&[]));
        h.set(|s| {
            s.lines.insert("side".into(), side);
        });
        h.save(
            "main",
            base,
            &tree(&[("b.txt", f("b\n")), ("a.txt", f("main\n"))]),
        );
    };
    let mut h = Hist::new();
    added(&mut h, &[]);
    rules_file(&h, BROKEN);
    let before = h.untouched();
    let err = combine_in(&mut h, "side").unwrap_err();
    assert_eq!(undecided(&err), (vec!["a.txt"], Side::Ours));
    assert_eq!(err.hint().unwrap(), RULES_HINT);
    assert_untouched(&h, &before);
    let mut h = Hist::new();
    added(&mut h, &[("opalro.toml", f(BROKEN))]);
    let before = h.untouched();
    let err = combine_in(&mut h, "side").unwrap_err();
    assert_eq!(undecided(&err), (vec!["a.txt"], Side::Theirs));
    assert_eq!(err.hint().unwrap(), THEIRS_HINT);
    assert_untouched(&h, &before);
}

#[test]
fn a_file_against_a_link_is_kept_standing_as_yours() {
    let mut fx = Fixture::new();
    for [s, o, t] in [
        [f("a\n"), f("b\n"), link("x")],
        [f("a\n"), link("x"), f("b\n")],
    ] {
        let one = |v: &V| tree(&[("l", v.clone())]);
        let (m, _) = fx.kept(&one(&s), &one(&o), &one(&t));
        one_kept(
            &m,
            "l",
            Clash::BothChanged(NotMerged::NotFiles),
            Stands::Yours,
        );
        let r = fx.record(m.root, "l");
        assert_eq!(fx.terms(&r), [Some(o), Some(s), Some(t)]);
        assert_eq!(r.own, None);
    }
}

#[test]
fn a_held_conflict_met_after_places_were_kept_leaves_nothing_behind() {
    let mut h = Hist::new();
    let c = plant_record(&mut h, [b"mine\n", b"was\n", b"theirs\n"]);
    // `a.txt` is kept with markers and `b.bin` as yours before `z.txt` is met.
    let base = tree(&[
        ("a.txt", f(BODY)),
        ("b.bin", f("b\0")),
        ("z.txt", f("was\n")),
    ]);
    let main = tree(&[
        ("a.txt", edit(2, "main")),
        ("b.bin", f("main\0")),
        ("z.txt", V::Conflict(c)),
    ]);
    let side = tree(&[
        ("a.txt", edit(2, "side")),
        ("b.bin", f("side\0")),
        ("z.txt", f("side\n")),
    ]);
    planted_lines(&mut h, &base, &main, &side);
    let before = h.untouched();
    let err = combine_in(&mut h, "side").unwrap_err();
    assert!(
        matches!(&err, CombineError::HeldConflict { path, .. } if path == "z.txt"),
        "{err:?}"
    );
    // No pack, and so no record, marked text or listing, and nothing in `tmp`; the line stays.
    assert_untouched(&h, &before);
}

// Moves keep conflicts at the places the file went.

/// The record at `path` of the stored tree `root`, with what each of its terms holds.
fn record_in(h: &Hist, root: Id, path: &str) -> (Record, Vec<Option<V>>) {
    let store = h.repo.store();
    let V::Conflict(id) = read_tree(store, root, &HashMap::new())[path] else {
        panic!("no conflict at {path}");
    };
    let r = store.conflict(&id).unwrap();
    let terms = r
        .terms
        .iter()
        .map(|t| t.map(|t| leaf(store, t.kind, t.id)))
        .collect();
    (r, terms)
}

fn kept_at(path: &str, clash: Clash, stands: Stands) -> Conflict {
    Conflict {
        path: path.into(),
        clash,
        stands: Some(stands),
    }
}

fn paths_of(h: &Hist, root: Id) -> Vec<String> {
    read_tree(h.repo.store(), root, &HashMap::new())
        .into_keys()
        .collect()
}

#[test]
fn moved_apart_keeps_two_conflicts() {
    let mut h = Hist::new();
    let s = player(&mut h);
    let o = h.save("main", s, &moved_to("A/Player.cs", f(BODY)));
    let t = h.save("side", s, &moved_to("B/Player.cs", f(BODY)));
    let t = h.save("side", t, &moved_to("B/Player.cs", edit(1, "TWO")));
    let m = h.kept(s, o, t, &no_rules());
    let apart = |moved_on, other_to: &str| Clash::MovedApart {
        from: "Player.cs".into(),
        moved_on,
        other_to: other_to.into(),
    };
    assert_eq!(
        m.conflicts,
        [
            kept_at(
                "A/Player.cs",
                apart(Side::Ours, "B/Player.cs"),
                Stands::Yours
            ),
            kept_at(
                "B/Player.cs",
                apart(Side::Theirs, "A/Player.cs"),
                Stands::Nothing
            ),
        ]
    );
    assert_eq!(m.more, 0);
    let (r, terms) = record_in(&h, m.root, "A/Player.cs");
    assert_eq!(r.own, None);
    assert_eq!(terms, [Some(f(BODY)), Some(f(BODY)), None]);
    let (r, terms) = record_in(&h, m.root, "B/Player.cs");
    assert_eq!(r.own, None);
    assert_eq!(terms, [None, Some(f(BODY)), Some(edit(1, "TWO"))]);
    assert_eq!(
        paths_of(&h, m.root),
        ["A", "A/Player.cs", "B", "B/Player.cs", "Other.cs"]
    );
}

/// Ours moved `Player.cs` to `A`; theirs moved it to `B` and holds the very same file at `A` too,
/// where ours has something of its own at `B`. At `A` both lines hold the same file: it is taken
/// once, and only `B` clashes, as two added things.
#[test]
fn moved_apart_where_both_lines_hold_the_same_file_at_one_place_keeps_no_conflict_there() {
    let mut h = Hist::new();
    let s = player(&mut h);
    let mut ours = moved_to("A/Player.cs", f(BODY));
    ours.insert("B/Player.cs".to_owned(), f("mine\n"));
    let o = h.save("main", s, &ours);
    let t = h.save("side", s, &moved_to("B/Player.cs", f(BODY)));
    let mut copied = moved_to("B/Player.cs", f(BODY));
    copied.insert("A/Player.cs".to_owned(), f(BODY));
    let t = h.save("side", t, &copied);
    let m = h.kept(s, o, t, &no_rules());
    let onto = Clash::MovedOnto {
        moved_on: Side::Theirs,
        from: "Player.cs".into(),
    };
    let places: Vec<(&str, &Clash)> = (m.conflicts.iter())
        .map(|c| (c.path.as_str(), &c.clash))
        .collect();
    assert_eq!(places, [("B/Player.cs", &onto)]);
    let merged = read_tree(h.repo.store(), m.root, &HashMap::new());
    assert_eq!(merged["A/Player.cs"], f(BODY));
    let (_, terms) = record_in(&h, m.root, "B/Player.cs");
    assert_eq!(terms, [Some(f("mine\n")), None, Some(f(BODY))]);
}

/// `moved_apart_where_both_lines_hold_the_same_file_at_one_place_keeps_no_conflict_there` the
/// other way round: the place held alike is where theirs moved it.
#[test]
fn moved_apart_where_both_lines_hold_the_same_file_at_theirs_place_keeps_no_conflict_there() {
    let mut h = Hist::new();
    let s = player(&mut h);
    let o = h.save("main", s, &moved_to("A/Player.cs", f(BODY)));
    let mut copied = moved_to("A/Player.cs", f(BODY));
    copied.insert("B/Player.cs".to_owned(), f(BODY));
    let o = h.save("main", o, &copied);
    let mut theirs = moved_to("B/Player.cs", f(BODY));
    theirs.insert("A/Player.cs".to_owned(), f("mine\n"));
    let t = h.save("side", s, &theirs);
    let m = h.kept(s, o, t, &no_rules());
    let onto = Clash::MovedOnto {
        moved_on: Side::Ours,
        from: "Player.cs".into(),
    };
    let places: Vec<(&str, &Clash)> = (m.conflicts.iter())
        .map(|c| (c.path.as_str(), &c.clash))
        .collect();
    assert_eq!(places, [("A/Player.cs", &onto)]);
    let merged = read_tree(h.repo.store(), m.root, &HashMap::new());
    assert_eq!(merged["B/Player.cs"], f(BODY));
    let (_, terms) = record_in(&h, m.root, "A/Player.cs");
    assert_eq!(terms, [Some(f(BODY)), None, Some(f("mine\n"))]);
}

#[test]
fn moved_apart_where_both_lines_hold_the_same_file_at_both_places_keeps_no_conflict() {
    let mut h = Hist::new();
    let s = player(&mut h);
    let both = with_folders(&tree(&[
        ("A/Player.cs", f(BODY)),
        ("B/Player.cs", f(BODY)),
        ("Other.cs", f("other\n")),
    ]));
    let o = h.save("main", s, &moved_to("A/Player.cs", f(BODY)));
    let o = h.save("main", o, &both);
    let t = h.save("side", s, &moved_to("B/Player.cs", f(BODY)));
    let t = h.save("side", t, &both);
    assert_eq!(h.merged(s, o, t), both);
    let m = h.kept(s, o, t, &no_rules());
    assert!(m.conflicts.is_empty(), "{:?}", m.conflicts);
    assert_eq!(read_tree(h.repo.store(), m.root, &HashMap::new()), both);
}

#[test]
fn moved_and_removed_where_the_other_line_holds_the_same_file_where_it_went_keeps_no_conflict() {
    // A second start file with the same contents, so that the line that removed `Player.cs` is
    // not followed by contents to its own copy.
    let mut h = Hist::new();
    let first = h.first;
    let start = [
        ("Player.cs", f(BODY)),
        ("Copy.cs", f(BODY)),
        ("Other.cs", f("other\n")),
    ];
    let s = h.save("main", first, &tree(&start));
    let ours = [
        ("A/Player.cs", f(BODY)),
        ("Copy.cs", f(BODY)),
        ("Other.cs", f("other\n")),
    ];
    let o = h.save("main", s, &tree(&ours));
    let mut added = tree(&start);
    added.insert("A/Player.cs".to_owned(), f(BODY));
    let t = h.save("side", s, &added);
    let theirs = [("A/Player.cs", f(BODY)), ("Other.cs", f("other\n"))];
    let t = h.save("side", t, &tree(&theirs));
    assert_eq!(
        h.followed(h.version(s), h.version(o)),
        pairs(&[("Player.cs", "A/Player.cs")])
    );
    assert_eq!(h.followed(h.version(s), h.version(t)), pairs(&[]));
    let want = with_folders(&tree(&theirs));
    assert_eq!(h.merged(s, o, t), want);
    let m = h.kept(s, o, t, &no_rules());
    assert!(m.conflicts.is_empty(), "{:?}", m.conflicts);
    assert_eq!(read_tree(h.repo.store(), m.root, &HashMap::new()), want);
}

#[test]
fn moved_apart_alike_but_for_the_executable_mark_keeps_the_two_added_files() {
    // Not the very same entry: the two are kept as two added things, standing as yours.
    let mut h = Hist::new();
    let s = player(&mut h);
    let o = h.plant(
        &[s],
        &moved_to("A/Player.cs", f(BODY)),
        Some(&[("Player.cs", "A/Player.cs")]),
    );
    let run = V::File(BODY.as_bytes().to_vec(), true);
    let mut theirs = moved_to("B/Player.cs", f(BODY));
    theirs.insert("A/Player.cs".to_owned(), run.clone());
    let t = h.plant(&[s], &theirs, Some(&[("Player.cs", "B/Player.cs")]));
    let m = h.kept(s, o, t, &no_rules());
    let onto = Clash::MovedOnto {
        moved_on: Side::Ours,
        from: "Player.cs".into(),
    };
    let apart = Clash::MovedApart {
        from: "Player.cs".into(),
        moved_on: Side::Theirs,
        other_to: "A/Player.cs".into(),
    };
    assert_eq!(
        m.conflicts,
        [
            kept_at("A/Player.cs", onto, Stands::Yours),
            kept_at("B/Player.cs", apart, Stands::Nothing),
        ]
    );
    assert_eq!(
        record_in(&h, m.root, "A/Player.cs").1,
        [Some(f(BODY)), None, Some(run)]
    );
}

#[test]
fn moved_apart_into_a_folder_both_lines_hold_alike_keeps_no_conflict_there() {
    // A place is a file; what can be alike above it is a folder, taken once whole.
    let mut h = Hist::new();
    let s = player(&mut h);
    let o = h.save("main", s, &moved_to("d/Player.cs", f(BODY)));
    let t = h.save("side", s, &moved_to("B/Player.cs", f(BODY)));
    let mut copied = moved_to("B/Player.cs", f(BODY));
    copied.insert("d/Player.cs".to_owned(), f(BODY));
    let t = h.save("side", t, &copied);
    let m = h.kept(s, o, t, &no_rules());
    let apart = Clash::MovedApart {
        from: "Player.cs".into(),
        moved_on: Side::Theirs,
        other_to: "d/Player.cs".into(),
    };
    assert_eq!(
        m.conflicts,
        [kept_at("B/Player.cs", apart, Stands::Nothing)]
    );
    let merged = read_tree(h.repo.store(), m.root, &HashMap::new());
    assert_eq!(merged["d/Player.cs"], f(BODY));
}

#[test]
fn moved_and_removed_keeps_the_moved_file_where_it_went() {
    let mut h = Hist::new();
    let s = player(&mut h);
    let at = "Characters/Player.cs";
    let moved = h.save("main", s, &moved_to(at, f(BODY)));
    let removed = h.save("side", s, &tree(&[("Other.cs", f("other\n"))]));
    let clash = |moved_on| Clash::MovedRemoved {
        moved_on,
        from: "Player.cs".into(),
    };
    let m = h.kept(s, moved, removed, &no_rules());
    assert_eq!(m.conflicts, [kept_at(at, clash(Side::Ours), Stands::Yours)]);
    assert_eq!(
        record_in(&h, m.root, at).1,
        [Some(f(BODY)), Some(f(BODY)), None]
    );
    let m = h.kept(s, removed, moved, &no_rules());
    assert_eq!(
        m.conflicts,
        [kept_at(at, clash(Side::Theirs), Stands::Nothing)]
    );
    assert_eq!(
        record_in(&h, m.root, at).1,
        [None, Some(f(BODY)), Some(f(BODY))]
    );
    assert_eq!(paths_of(&h, m.root), ["Characters", at, "Other.cs"]);
}

/// On `main`, `Player.cs` moved to `Characters/Player.cs`; on `side`, something new put there
/// and `Player.cs` held as `old`. Gives the three saves.
fn moved_onto_new(h: &mut Hist, old: V) -> (Id, Id, Id) {
    let s = player(h);
    let o = h.save("main", s, &moved_to("Characters/Player.cs", f(BODY)));
    let mut added = moved_to("Player.cs", old);
    added.insert("Characters/Player.cs".to_owned(), f("new\n"));
    let t = h.save("side", s, &added);
    (s, o, t)
}

#[test]
fn moved_onto_something_new_keeps_two_added_things_with_markers() {
    let mut h = Hist::new();
    let (s, o, t) = moved_onto_new(&mut h, f(BODY));
    let at = "Characters/Player.cs";
    let m = h.kept(s, o, t, &no_rules());
    let onto = Clash::MovedOnto {
        moved_on: Side::Ours,
        from: "Player.cs".into(),
    };
    assert_eq!(
        m.conflicts,
        [kept_at(at, onto, Stands::Markers { places: 1 })]
    );
    let (r, terms) = record_in(&h, m.root, at);
    assert_eq!(terms, [Some(f(BODY)), None, Some(f("new\n"))]);
    assert_eq!(r.own.unwrap().marker, 7);
    // Side left it as it was where it was: it went with the move.
    assert_eq!(paths_of(&h, m.root), ["Characters", at, "Other.cs"]);
}

#[test]
fn moved_onto_while_the_other_line_changed_it_in_place_keeps_that_change_at_the_old_place() {
    let mut h = Hist::new();
    let (s, o, t) = moved_onto_new(&mut h, edit(1, "TWO"));
    let m = h.kept(s, o, t, &no_rules());
    let onto = Clash::MovedOnto {
        moved_on: Side::Ours,
        from: "Player.cs".into(),
    };
    let removed = Clash::ChangedRemoved {
        removed_on: Side::Ours,
    };
    assert_eq!(
        m.conflicts,
        [
            kept_at("Characters/Player.cs", onto, Stands::Markers { places: 1 }),
            kept_at("Player.cs", removed, Stands::Nothing),
        ]
    );
    assert_eq!(
        record_in(&h, m.root, "Player.cs").1,
        [None, Some(f(BODY)), Some(edit(1, "TWO"))]
    );
}

#[test]
fn a_move_clash_place_in_a_new_folder_makes_the_folder() {
    let mut h = Hist::new();
    let s = player(&mut h);
    let at = "New/Deeper/Player.cs";
    let removed = h.save("main", s, &tree(&[("Other.cs", f("other\n"))]));
    let moved = h.save("side", s, &moved_to(at, f(BODY)));
    let m = h.kept(s, removed, moved, &no_rules());
    let clash = Clash::MovedRemoved {
        moved_on: Side::Theirs,
        from: "Player.cs".into(),
    };
    assert_eq!(m.conflicts, [kept_at(at, clash, Stands::Nothing)]);
    let top = h.repo.store().listing(&m.root).unwrap();
    assert_eq!(top.get("New").unwrap().kind, EntryKind::Folder);
    assert_eq!(paths_of(&h, m.root), ["New", "New/Deeper", at, "Other.cs"]);
}

#[test]
fn the_count_includes_move_clash_places() {
    let mut h = Hist::new();
    let s = player(&mut h);
    let ours = moved_to("A/Player.cs", f(BODY));
    h.save("main", s, &ours);
    h.save("side", s, &moved_to("B/Player.cs", f(BODY)));
    h.on("main", &ours);
    let (saved, made, count) = kept_combine(&mut h);
    let paths: Vec<&str> = made.iter().map(|c| c.path.as_str()).collect();
    assert_eq!((paths, count), (vec!["A/Player.cs", "B/Player.cs"], 2));
    assert!(matches!(saved["A/Player.cs"], V::Conflict(_)));
    assert!(matches!(saved["B/Player.cs"], V::Conflict(_)));
    // Standing as yours: your file where you put it, nothing where they did.
    let folder = h.folder();
    assert_eq!(folder.get("A/Player.cs"), Some(&f(BODY)));
    assert!(!folder.contains_key("B/Player.cs"), "{folder:?}");
}

#[test]
fn a_move_clash_below_a_kept_conflict_is_held_by_that_conflict() {
    let mut h = Hist::new();
    let first = h.first;
    let s = h.save("main", first, &tree(&[("a.txt", f(BODY))]));
    let ui = [("ui/a.txt", f(BODY)), ("ui/b.txt", f("b\n"))];
    let o = h.save("main", s, &tree(&ui));
    let theirs = [("c.txt", f(BODY)), ("ui", f("ui\n"))];
    let t = h.save("side", s, &tree(&theirs));
    let m = h.kept(s, o, t, &no_rules());
    let apart = Clash::MovedApart {
        from: "a.txt".into(),
        moved_on: Side::Theirs,
        other_to: "ui/a.txt".into(),
    };
    let folder_on = Side::Ours;
    assert_eq!(
        m.conflicts,
        [
            kept_at("c.txt", apart, Stands::Nothing),
            kept_at("ui", Clash::FileAndFolder { folder_on }, Stands::Yours),
        ]
    );
    // No record below `ui`: your folder there holds the moved file, and `b.txt`.
    assert_eq!(paths_of(&h, m.root), ["c.txt", "ui"]);
    let (r, terms) = record_in(&h, m.root, "ui");
    assert_eq!(terms, [Some(V::Folder), None, Some(f("ui\n"))]);
    let yours = r.terms[0].unwrap();
    let held = read_tree(h.repo.store(), yours.id, &HashMap::new());
    assert_eq!(held, tree(&[("a.txt", f(BODY)), ("b.txt", f("b\n"))]));
    assert_eq!(
        record_in(&h, m.root, "c.txt").1,
        [None, Some(f(BODY)), Some(f(BODY))]
    );
}

#[test]
fn a_scene_moved_onto_a_text_is_kept_as_yours_when_its_old_name_says_no_markers() {
    let mut h = Hist::new();
    let first = h.first;
    let s = h.save("main", first, &tree(&[("Main.unity", f(BODY))]));
    let o = h.save("main", s, &tree(&[("notes.txt", f(BODY))]));
    let both = [("Main.unity", f(BODY)), ("notes.txt", f("notes\n"))];
    let t = h.save("side", s, &tree(&both));
    let onto = Clash::MovedOnto {
        moved_on: Side::Ours,
        from: "Main.unity".into(),
    };
    let m = h.kept(s, o, t, &scene_rule());
    assert_eq!(
        m.conflicts,
        [kept_at("notes.txt", onto.clone(), Stands::Yours)]
    );
    assert_eq!(record_in(&h, m.root, "notes.txt").0.own, None);
    // Its new name alone would have been marked.
    let m = h.kept(s, o, t, &no_rules());
    assert_eq!(
        m.conflicts,
        [kept_at("notes.txt", onto, Stands::Markers { places: 1 })]
    );
}

/// Every entry below the stored listing `root`, by path, folders too, read apart from the merge.
fn entries_below(store: &Store, root: Id) -> Vec<(String, Entry)> {
    let mut out = Vec::new();
    let mut todo = vec![(String::new(), root)];
    while let Some((at, id)) = todo.pop() {
        for e in store.listing(&id).unwrap().entries() {
            let path = if at.is_empty() {
                e.name.clone()
            } else {
                format!("{at}/{}", e.name)
            };
            if e.kind == EntryKind::Folder {
                todo.push((path.clone(), e.id));
            }
            out.push((path, e.clone()));
        }
    }
    out
}

#[test]
fn a_rotation_keeps_the_file_moved_onto_a_place_moved_apart_from_whatever_the_path_order() {
    // `x` moves onto `y`'s place on main, where it is changed, after `y` moved to `t0`; side
    // moves `y` to `t1`. Each way round, so that either file sorts first.
    for (x, y) in [("a.txt", "b.txt"), ("b.txt", "a.txt")] {
        let mut h = Hist::new();
        let first = h.first;
        let why = f("why\n");
        let s = h.save("main", first, &tree(&[(x, f(BODY)), (y, why.clone())]));
        let o = h.save("main", s, &tree(&[(x, f(BODY)), ("t0", why.clone())]));
        let o = h.save("main", o, &tree(&[(y, f(BODY)), ("t0", why.clone())]));
        let o = h.save(
            "main",
            o,
            &tree(&[(y, edit(1, "TWO")), ("t0", why.clone())]),
        );
        let t = h.save("side", s, &tree(&[(x, f(BODY)), ("t1", why.clone())]));
        // Only `y`'s moves are followed: `x` landed where the start holds a file, which no
        // followed move may, so nothing a move puts in the start is ever cleared.
        let start = h.version(s);
        assert_eq!(h.followed(start, h.version(o)), pairs(&[(y, "t0")]));
        assert_eq!(h.followed(start, h.version(t)), pairs(&[(y, "t1")]));
        let m = h.kept(s, o, t, &no_rules());
        let apart = |moved_on, other_to: &str| Clash::MovedApart {
            from: y.into(),
            moved_on,
            other_to: other_to.into(),
        };
        assert_eq!(
            m.conflicts,
            [
                kept_at("t0", apart(Side::Ours, "t1"), Stands::Yours),
                kept_at("t1", apart(Side::Theirs, "t0"), Stands::Nothing),
            ],
            "{x} onto {y}"
        );
        let result = read_tree(h.repo.store(), m.root, &HashMap::new());
        let paths: Vec<&str> = result.keys().map(String::as_str).collect();
        let mut want = vec!["t0", "t1", y];
        want.sort_unstable();
        assert_eq!(paths, want, "{x} onto {y}");
        assert_eq!(result[y], edit(1, "TWO"), "{x} onto {y}");
        assert_eq!(
            record_in(&h, m.root, "t0").1,
            [Some(why.clone()), Some(why.clone()), None]
        );
        assert_eq!(
            record_in(&h, m.root, "t1").1,
            [None, Some(why.clone()), Some(why)]
        );
    }
}

#[test]
fn a_file_moved_onto_a_place_the_other_line_changed_clashes_whole_with_the_start_between() {
    let mut h = Hist::new();
    let first = h.first;
    let q = f("q1\nq2\nq3\n");
    let s = h.save(
        "main",
        first,
        &tree(&[("p.txt", f(BODY)), ("q.txt", q.clone())]),
    );
    // `p.txt` removed, then `q.txt` moved where it was, each saved alone.
    let o = h.save("main", s, &tree(&[("q.txt", q.clone())]));
    let o = h.save("main", o, &tree(&[("p.txt", q.clone())]));
    let changed = edit(2, "side");
    let t = h.save(
        "side",
        s,
        &tree(&[("p.txt", changed.clone()), ("q.txt", q.clone())]),
    );
    let m = h.kept(s, o, t, &no_rules());
    let onto = Clash::MovedOnto {
        moved_on: Side::Ours,
        from: "q.txt".into(),
    };
    assert_eq!(
        m.conflicts,
        [kept_at("p.txt", onto, Stands::Markers { places: 1 })]
    );
    assert_eq!(
        m.conflicts[0].describe("main", "side"),
        "p.txt: moved here from q.txt on main, where side has something else"
    );
    let (r, terms) = record_in(&h, m.root, "p.txt");
    assert_eq!(
        terms,
        [Some(q.clone()), Some(f(BODY)), Some(changed.clone())]
    );
    // Marked whole, with the start's file in the middle, though an edit of one line of it
    // would have merged.
    let labels = sides().labels(Parted::One(ChangeId([3; 16])));
    let text = |v: &V| match v {
        V::File(bytes, _) => String::from_utf8(bytes.clone()).unwrap(),
        other => panic!("{other:?}"),
    };
    let want = marked(&labels, ["", &text(&q), BODY, &text(&changed), ""]);
    let own = r.own.unwrap().version.unwrap();
    assert_eq!(leaf(h.repo.store(), own.kind, own.id), V::File(want, false));
    assert_eq!(paths_of(&h, m.root), ["p.txt"]);
}

#[test]
fn a_start_folder_changed_on_one_line_and_removed_on_the_other_is_merged_entry_by_entry() {
    // On one line `a.txt` moves into `d/` and `d/x` is changed; the other removes `d` and
    // `a.txt`. The folder holds no conflict of its own: its entries are kept each where it is.
    let mut h = Hist::new();
    let first = h.first;
    let keep = ("keep.txt", f("keep\n"));
    let start = [("d/x", f(BODY)), ("a.txt", f("a\n")), keep.clone()];
    let s = h.save("main", first, &tree(&start));
    let a = h.save(
        "main",
        s,
        &tree(&[("d/x", f(BODY)), ("d/a.txt", f("a\n")), keep.clone()]),
    );
    let a = h.save(
        "main",
        a,
        &tree(&[("d/x", edit(0, "A")), ("d/a.txt", f("a\n")), keep.clone()]),
    );
    let b = h.save("side", s, &tree(&[keep]));
    for (o, t, moved_on) in [(a, b, Side::Ours), (b, a, Side::Theirs)] {
        let m = h.kept(s, o, t, &no_rules());
        let (stands, removed_on) = if moved_on == Side::Ours {
            (Stands::Yours, Side::Theirs)
        } else {
            (Stands::Nothing, Side::Ours)
        };
        let moved = Clash::MovedRemoved {
            moved_on,
            from: "a.txt".into(),
        };
        assert_eq!(
            m.conflicts,
            [
                kept_at("d/a.txt", moved, stands),
                kept_at("d/x", Clash::ChangedRemoved { removed_on }, stands),
            ]
        );
        assert_eq!(paths_of(&h, m.root), ["d", "d/a.txt", "d/x", "keep.txt"]);
        // Nothing the line that moved saved is left out, and no record is below another's
        // place: `d` is a plain folder.
        let top = h.repo.store().listing(&m.root).unwrap();
        assert_eq!(top.get("d").unwrap().kind, EntryKind::Folder);
        let lines = |v: Option<V>| {
            if moved_on == Side::Ours {
                [v, None]
            } else {
                [None, v]
            }
        };
        let [yours, theirs] = lines(Some(f("a\n")));
        assert_eq!(
            record_in(&h, m.root, "d/a.txt").1,
            [yours, Some(f("a\n")), theirs]
        );
        let [yours, theirs] = lines(Some(edit(0, "A")));
        assert_eq!(
            record_in(&h, m.root, "d/x").1,
            [yours, Some(f(BODY)), theirs]
        );
    }
}

/// A start of `n` files and two lines' work, as `following_moves_agrees_with_a_model` draws them,
/// and a quarter of the time drawn on purpose so that a move clash's place lands below a
/// conflict: one line moves a file into `d/`, the other makes a file `d` and removes that file or
/// moves it elsewhere, either way round.
fn cases_near_d() -> impl Strategy<Value = (usize, Work, Work)> {
    let nested =
        (1..=6usize, 0..6usize, any::<bool>(), any::<bool>()).prop_map(|(n, k, apart, swap)| {
            let into_d = worked(&[(k, 3, false)], &[]);
            let file_d = if apart {
                worked(&[(k, 0, false)], &[(0, 4)])
            } else {
                worked(&[], &[(k, 1), (0, 4)])
            };
            if swap {
                (n, file_d, into_d)
            } else {
                (n, into_d, file_d)
            }
        });
    prop_oneof![3 => (1..=6usize, work(), work()), 1 => nested, 1 => held_alike()]
}

/// Each non-folder entry below the stored listing `root`, by path, as its kind and ID.
fn versions_below(store: &Store, root: Id) -> BTreeMap<String, (u8, Id)> {
    entries_below(store, root)
        .into_iter()
        .filter(|(_, e)| e.kind != EntryKind::Folder)
        .map(|(p, e)| (p, (e.kind as u8, e.id)))
        .collect()
}

#[test]
fn nothing_a_line_saved_is_left_out() {
    let h = std::cell::RefCell::new(Hist::new());
    let seen: std::cell::RefCell<BTreeMap<&str, usize>> = Default::default();
    let rng = TestRng::deterministic_rng(RngAlgorithm::ChaCha);
    let mut runner = TestRunner::new_with_rng(Config::with_cases(128), rng);
    runner
        .run(&cases_near_d(), |(n, ours, theirs)| {
            let mut h = h.borrow_mut();
            let files = start_files(n);
            let first = h.first;
            let s = h.save("main", first, &files);
            let (o, _) = do_work(&mut h, "main", s, &files, &ours);
            let (t, _) = do_work(&mut h, "side", s, &files, &theirs);
            let [vs, mut vo, mut vt] = [s, o, t].map(|x| h.version(x));
            for (work, v) in [(&ours, &mut vo), (&theirs, &mut vt)] {
                if work.made_up {
                    v.save = None;
                }
            }
            let lock = h.repo.lock_for_change("test", &mut |_| {}).unwrap();
            let mut packs = h.repo.new_packs().unwrap();
            let mut made = Made {
                keep: Some(sides()),
                ..Made::default()
            };
            let rules = no_rules();
            let m = merge_three(&h.repo, &mut packs, &mut made, false, &vs, &vo, &vt, &rules)
                .map_err(|e| TestCaseError::fail(format!("{e:?}")))?;
            h.repo.store_packs(&lock, packs).unwrap();
            let store = h.repo.store();
            let mut seen = seen.borrow_mut();
            for c in &m.conflicts {
                *seen.entry(clash_kind(&c.clash)).or_default() += 1;
            }
            // What the result holds as plain entries, where its conflicts are, and what its
            // records hold for a line: yours and theirs, and everything below a folder there.
            let mut plain_at = BTreeMap::new();
            let mut conflicts_at = BTreeSet::new();
            let mut held = BTreeSet::new();
            for (path, e) in entries_below(store, m.root) {
                if e.kind != EntryKind::Conflict {
                    plain_at.insert(path, (e.kind as u8, e.id));
                    continue;
                }
                let r = store.conflict(&e.id).unwrap();
                for (i, t) in r.terms.iter().enumerate() {
                    let Some(t) = t else { continue };
                    let a_line = i != 1;
                    if a_line {
                        held.insert((t.kind as u8, t.id));
                    }
                    if t.kind != EntryKind::Folder {
                        continue;
                    }
                    for (p, e) in entries_below(store, t.id) {
                        prop_assert!(e.kind != EntryKind::Conflict, "{p} in {path}");
                        if a_line {
                            held.insert((e.kind as u8, e.id));
                        }
                    }
                }
                conflicts_at.insert(path);
            }
            let plain: BTreeSet<(u8, Id)> = plain_at.values().copied().collect();
            let three = [vs, vo, vt].map(|v| versions_below(store, v.root));
            for (i, side) in [(1, "main"), (2, "side")] {
                let other = 3 - i;
                for (p, x) in &three[i] {
                    if three[0].get(p) == Some(x) || plain.contains(x) || held.contains(x) {
                        continue;
                    }
                    // The one merge that keeps no copy of `x`: the line only moved the start's
                    // file from `q`, the other line changed it, there or after moving it to `p`
                    // too, and that change is what the result holds at `p`.
                    let followed = three[0].iter().any(|(q, start)| {
                        let changed = |y: &(u8, Id)| y != x && plain_at.get(p) == Some(y);
                        q != p
                            && start == x
                            && three[i].get(q) != Some(x)
                            && (three[other].get(q).is_some_and(changed)
                                || (!three[other].contains_key(q)
                                    && three[other].get(p).is_some_and(changed)))
                    });
                    prop_assert!(followed, "{side}'s {p} is left out: {:?}", m.conflicts);
                    *seen.entry("moved, changed on the other line").or_default() += 1;
                }
            }
            // Where the two lines' followed moves disagree, the places below a conflict.
            let went = [vo, vt].map(|v| h.followed(vs, v).into_iter().collect::<BTreeMap<_, _>>());
            let froms: BTreeSet<&String> = went[0].keys().chain(went[1].keys()).collect();
            for from in froms {
                let places = match (went[0].get(from), went[1].get(from)) {
                    (Some(x), Some(y)) if x != y => vec![x, y],
                    (Some(x), None) if !three[2].contains_key(from) => vec![x],
                    (None, Some(y)) if !three[1].contains_key(from) => vec![y],
                    _ => vec![],
                };
                for at in places {
                    if three[1].get(at.as_str()) == three[2].get(at.as_str()) {
                        *seen.entry("move clash place held alike").or_default() += 1;
                    }
                    if crate::scope::ancestors(at).any(|up| conflicts_at.contains(up)) {
                        *seen.entry("move clash place below a conflict").or_default() += 1;
                    }
                }
            }
            Ok(())
        })
        .unwrap();
    let seen = seen.into_inner();
    for kind in [
        "moved apart",
        "moved, removed",
        "moved onto",
        "move clash place below a conflict",
        "move clash place held alike",
    ] {
        assert!(seen.get(kind).is_some_and(|&n| n >= 3), "{seen:?}");
    }
}

#[test]
fn a_file_moved_onto_a_new_file_the_other_line_put_there_keeps_only_the_place_it_went() {
    // Side moves `a.txt` into `d/e/b`; main leaves `a.txt` alone and puts a file of its own at
    // `d/e/b`. Main's `a.txt` is the start's: it went with the move, so the old place is clean.
    let mut h = Hist::new();
    let first = h.first;
    let s = h.save("main", first, &tree(&[("a.txt", f(BODY))]));
    let o = h.save(
        "main",
        s,
        &tree(&[("a.txt", f(BODY)), ("d/e/b", f("main\n"))]),
    );
    let t = h.save("side", s, &tree(&[("d/e/b", f(BODY))]));
    assert_eq!(
        h.conflicts(s, o, t),
        ["d/e/b: moved here from a.txt on side, where main has something else"]
    );
    let m = h.kept(s, o, t, &no_rules());
    let onto = Clash::MovedOnto {
        moved_on: Side::Theirs,
        from: "a.txt".into(),
    };
    assert_eq!(
        m.conflicts,
        [kept_at("d/e/b", onto, Stands::Markers { places: 1 })]
    );
    assert_eq!(paths_of(&h, m.root), ["d", "d/e", "d/e/b"]);
    assert_eq!(
        record_in(&h, m.root, "d/e/b").1,
        [Some(f("main\n")), None, Some(f(BODY))]
    );
}

#[test]
fn a_file_moved_where_the_other_line_holds_the_same_file_already_combines_cleanly() {
    // Side moves `a.txt` to `d/c`; main keeps `a.txt` as it was and holds the same contents at
    // `d/c` too. Both lines hold alike where it went, and main never changed it where it was.
    let mut h = Hist::new();
    let first = h.first;
    let s = h.save("main", first, &tree(&[("a.txt", f(BODY))]));
    let ours = [("a.txt", f(BODY)), ("b", f("b\n")), ("d/c", f(BODY))];
    let o = h.save("main", s, &tree(&ours));
    let t = h.save("side", s, &tree(&[("d/c", f(BODY))]));
    let want = with_folders(&tree(&[("b", f("b\n")), ("d/c", f(BODY))]));
    assert_eq!(h.merged(s, o, t), want);
    let m = h.kept(s, o, t, &no_rules());
    assert!(m.conflicts.is_empty(), "{:?}", m.conflicts);
    assert_eq!(read_tree(h.repo.store(), m.root, &HashMap::new()), want);
}
