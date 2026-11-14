#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;

use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};

use super::tree::{MAX_HELD_TEXT, Made, Merged, merge_trees};
use super::*;
use crate::ancestry::only_in;
use crate::changes::DEADLINE;
use crate::diary::{Action, DiaryEntry, Position, State};
use crate::id::{ChangeId, Flavor, Id, hash_record};
use crate::kept::overlay;
use crate::link::ProjectLink;
use crate::listing::{Entry, EntryKind, Listing};
use crate::moves::{MAX_LEFT_BYTES, Version, followed, same_contents};
use crate::names::PortabilityProblem;
use crate::pack::{ObjectKind, PackSet};
use crate::presets::{Engine, Found, Preset, opal_toml};
use crate::repo::{ChangeLock, Repository};
use crate::rules::{RuleMatcher, Rules};
use crate::save::{MoveNote, Person, Save};
use crate::snapshot::{SaveRequest, save_folder, store_listing};
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
        let rules = RuleMatcher::new(&Rules::default());
        self.merge_with(ids, &rules, false, &mut Made::default())
    }

    /// Merges, then stores what the merge built so that it can be read back.
    fn merge_with(
        &mut self,
        [s, o, t]: [Id; 3],
        rules: &RuleMatcher,
        inner: bool,
        made: &mut Made,
    ) -> Result<Merged, CombineError> {
        let mut packs = self.repo.new_packs().unwrap();
        let store = self.repo.store();
        let merged = merge_trees(store, &mut packs, made, inner, s, o, t, rules, DEADLINE);
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
        };
        changes.insert(path.clone(), Some(put));
    }
    overlay(store, empty, &changes, packs).unwrap()
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
            let mut bytes = Vec::new();
            let v = match e.kind {
                EntryKind::Folder => {
                    todo.push((p.clone(), e.id));
                    V::Folder
                }
                EntryKind::Symlink => {
                    store.copy_content(&e.id, &mut bytes).unwrap();
                    V::Link(String::from_utf8(bytes).unwrap())
                }
                EntryKind::ProjectLink => {
                    let n = (0..=u8::MAX)
                        .find(|&n| {
                            hash_record(Flavor::ProjectLink, &project_link(n).encode()) == e.id
                        })
                        .unwrap();
                    V::Project(n)
                }
                kind => {
                    store.copy_content(&e.id, &mut bytes).unwrap();
                    V::File(bytes, kind == EntryKind::Executable)
                }
            };
            out.insert(p, v);
        }
    }
    out
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
fn two_folders_that_merge_to_nothing_stay_as_an_empty_folder() {
    let mut fx = Fixture::new();
    let got = fx.merged(
        &tree(&[("d/a", f("a")), ("d/b", f("b"))]),
        &tree(&[("d/b", f("b"))]),
        &tree(&[("d/a", f("a"))]),
    );
    assert_eq!(got, tree(&[("d", V::Folder)]));
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
    let rules = RuleMatcher::new(&Rules::default());
    let mut made = Made::default();
    let store = fx.repo.store();
    let m = merge_trees(
        store, &mut packs, &mut made, false, s, o, t, &rules, DEADLINE,
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
    let rules = RuleMatcher::new(&Rules::default());

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
    assert!(matches!(err, CombineError::HeldTooMuch), "{err:?}");
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
    let rules = RuleMatcher::new(&Rules::default());
    let mut made = Made::default();
    let mut packs = fx.repo.new_packs().unwrap();
    let store = fx.repo.store();
    let inner = merge_trees(
        store, &mut packs, &mut made, true, s, o, t, &rules, DEADLINE,
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
        store, &mut packs, &mut made, false, inner.root, later[1], later[2], &rules, DEADLINE,
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
    let rules = RuleMatcher::new(&rules);
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
        Clash::BothChanged(NotMerged::NoMarkers)
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
        let CombineError::NamesClash { folder, problem } = &err else {
            panic!("{err:?}");
        };
        assert_eq!(folder, "");
        let (PortabilityProblem::CaseCollision { a, b }
        | PortabilityProblem::NormalizationCollision { a, b }) = problem
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
    | PortabilityProblem::NormalizationCollision { a, b }) = problem
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
    let CombineError::Past { which, detail } = &err else {
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
        let CombineError::Past { which, detail } = &err else {
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
            both(NotMerged::NoMarkers),
            "changed on both lines, and opalro.toml says conflict-markers = false for it"
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
                ours_to: to("x\u{7}"),
                theirs_to: to("y"),
            },
            format!("moved to x\\x07 on {o}, and to y on {t}"),
        ),
        (
            Clash::MovedRemoved {
                moved_on: Side::Ours,
                to: to("x\u{7}"),
            },
            format!("moved to x\\x07 on {o}, removed on {t}"),
        ),
        (
            Clash::MovedRemoved {
                moved_on: Side::Theirs,
                to: to("x"),
            },
            format!("moved to x on {t}, removed on {o}"),
        ),
        (
            Clash::MovedOnto {
                moved_on: Side::Ours,
                to: to("x\u{7}"),
            },
            format!("moved to x\\x07 on {o}, where {t} has something else"),
        ),
        (
            Clash::MovedOnto {
                moved_on: Side::Theirs,
                to: to("x"),
            },
            format!("moved to x on {t}, where {o} has something else"),
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
}

/// Merges the folder at `path` (`""` the top) whose contents are `s`, `o` and `t`; `keep` when
/// it stays though empty. Adds to `m` what it holds.
fn model_folder(path: &str, s: &Tree, o: &Tree, t: &Tree, keep: bool, m: &mut Model) {
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
    if !path.is_empty() && (keep || m.files.len() > before) {
        m.files.insert(path.to_owned(), V::Folder);
    }
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
    match (s, o, t) {
        (_, Node::Folder(a), Node::Folder(b)) => {
            model_folder(path, &start, a, b, true, m);
            return true;
        }
        (Node::Folder(_), Node::Folder(a), Node::Missing) => {
            let before = m.files.len();
            model_folder(path, &start, a, &empty, false, m);
            return m.files.len() > before;
        }
        (Node::Folder(_), Node::Missing, Node::Folder(b)) => {
            let before = m.files.len();
            model_folder(path, &start, &empty, b, false, m);
            return m.files.len() > before;
        }
        (Node::Leaf(V::File(sc, sx)), Node::Leaf(V::File(oc, ox)), Node::Leaf(V::File(tc, tx))) => {
            // One-line files: contents both sides changed always conflict.
            let mark = if ox == tx || ox == sx { *tx } else { *ox };
            let contents = if oc == tc || oc == sc {
                tc
            } else if tc == sc {
                oc
            } else {
                m.conflicts.insert(path.to_owned());
                return false;
            };
            m.files
                .insert(path.to_owned(), V::File(contents.clone(), mark));
            return true;
        }
        _ => {}
    }
    m.conflicts.insert(path.to_owned());
    false
}

fn model(s: &Tree, o: &Tree, t: &Tree) -> Model {
    let mut m = Model::default();
    if o == t || o == s {
        m.files = with_folders(t);
    } else if t == s {
        m.files = with_folders(o);
    } else {
        model_folder("", s, o, t, true, &mut m);
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
        .run(&three_trees(), |[s, o, t]| {
            let want = model(&s, &o, &t);
            let mut fx = fx.borrow_mut();
            let [s_id, o_id, t_id] = fx.three(&s, &o, &t);
            // Read back from what it made, never stored: no text is merged here.
            let mut packs = fx.repo.new_packs().unwrap();
            let mut made = Made::default();
            let rules = RuleMatcher::new(&Rules::default());
            let store = fx.repo.store();
            let merged = merge_trees(
                store, &mut packs, &mut made, false, s_id, o_id, t_id, &rules, DEADLINE,
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
}

impl Hist {
    fn new() -> Hist {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path(), &ada()).unwrap();
        let first = repo.current().unwrap().1.state.lines["main"];
        Hist { dir, repo, first }
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
        let who = ada();
        let req = SaveRequest {
            message: Some("Change things"),
            who: &who,
            command: "save",
            accept_crlf: false,
            git_excludes: None,
            new_line: None,
            only: None,
            amend: false,
        };
        save_folder(&mut self.repo, &req, &mut |_| {}, &mut |_| true).unwrap();
        self.repo.current().unwrap().1.state.lines[line]
    }

    /// Stores a save of `t` after `parents` noting `notes`, or what `save` would note against
    /// the first parent when `None`: a history from Git, a damaged one, or a combined save.
    fn plant(&mut self, parents: &[Id], t: &Tree, notes: Option<&[(&str, &str)]>) -> Id {
        let lock = self.repo.lock_for_change("test", &mut |_| {}).unwrap();
        let first = self.repo.store().save(&self.first, false).unwrap();
        let mut packs = self.repo.new_packs().unwrap();
        let root = put(self.repo.store(), first.root, t, &mut packs);
        self.repo.store_packs(&lock, packs).unwrap();
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
            }
        };
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
        followed(&self.repo, &Made::default(), &start, &side)
            .unwrap()
            .went
            .into_iter()
            .collect()
    }

    /// The merge of three versions, with the conflicts described with the sides called `main`
    /// and `side`, and its result read back.
    fn outcome(&mut self, s: Version, o: Version, t: Version) -> (Vec<String>, Tree) {
        let lock = self.repo.lock_for_change("test", &mut |_| {}).unwrap();
        let mut packs = self.repo.new_packs().unwrap();
        let mut made = Made::default();
        let rules = RuleMatcher::new(&Rules::default());
        let m = merge_three(&self.repo, &mut packs, &mut made, false, &s, &o, &t, &rules).unwrap();
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
        ["Player.cs: moved to A/Player.cs on main, and to B/Player.cs on side"]
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
        ["Player.cs: moved to Characters/Player.cs on main, removed on side"]
    );
    assert_eq!(
        h.conflicts(s, removed, moved),
        ["Player.cs: moved to Characters/Player.cs on side, removed on main"]
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
            "Characters/Player.cs: added on both lines, differently",
            "Player.cs: moved to Characters/Player.cs on main, where side has something else",
        ]
    );
    // A file where the moved file's folder would be.
    let mut added = moved_to("Player.cs", f(BODY));
    added.insert("Characters".to_owned(), f("new\n"));
    let t = h.save("side", s, &added);
    assert_eq!(
        h.conflicts(s, o, t),
        [
            "Characters: a file on side, a folder on main",
            "Player.cs: moved to Characters/Player.cs on main, where side has something else",
        ]
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
        ["Player.cs: moved to Z.cs on main, and to Y.cs on side"]
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
        ["Player.cs: moved to Z.cs on main, removed on side"]
    );
    let mut onto = moved_to("Player.cs", f(BODY));
    onto.insert("Z.cs".to_owned(), f("z\n"));
    let t = h.save("side", s, &onto);
    assert_eq!(
        h.conflicts(s, o, t),
        [
            "Player.cs: moved to Z.cs on main, where side has something else",
            "Z.cs: added on both lines, differently",
        ]
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
    let side = followed(&h.repo, &Made::default(), &h.version(s), &h.version(t)).unwrap();
    assert!(side.went.is_empty(), "{side:?}");
    assert!(side.replaced.contains("Player.cs"), "{side:?}");
    assert_eq!(
        h.conflicts(s, o, t),
        ["Player.cs: moved to Z.cs on main, removed on side"]
    );
    assert_eq!(
        h.conflicts(s, t, o),
        ["Player.cs: moved to Z.cs on side, removed on main"]
    );
    // Though the side has something where the file went, too.
    let mut onto = tree(&[("Player.cs", f("other\n"))]);
    onto.insert("Z.cs".to_owned(), f("z\n"));
    let t = h.save("side", t, &onto);
    assert_eq!(
        h.conflicts(s, o, t),
        [
            "Player.cs: moved to Z.cs on main, removed on side",
            "Z.cs: added on both lines, differently",
        ]
    );
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
    // `Player.cs` is the only path that left: 9 bytes.
    let at_most = |n: usize| {
        let made = Made {
            max_left_bytes: n,
            ..Made::default()
        };
        followed(&h.repo, &made, &h.version(s), &made_up)
            .unwrap()
            .went
    };
    assert!(at_most(8).is_empty());
    assert_eq!(at_most(9).len(), 1);
    // The merge goes on without the move.
    let t = h.save("side", s, &moved_to("Player.cs", edit(1, "TWO")));
    let mut packs = h.repo.new_packs().unwrap();
    let mut made = Made {
        max_left_bytes: 8,
        ..Made::default()
    };
    let rules = RuleMatcher::new(&Rules::default());
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

/// One side's work on a start of files `s0` to `s5`. First up to 3 moves, each saved alone, by
/// the index of the file among those there and of where it goes: onto a start file still there
/// it replaces it unnoted, unless that file is removed in a save of its own first (`true`).
/// Then up to 2 changes saved together: an edit (0), a removal (1), a new file where a start file
/// no longer is (2), a start file made a folder (3), or a file `d` (4).
#[derive(Debug, Clone)]
struct Work {
    moves: Vec<(usize, usize, bool)>,
    changes: Vec<(usize, u8)>,
}

const MOVE_TO: [&str; 8] = ["t0", "t1", "t2", "d/t0", "d/t1", "d/t2", "s0", "s1"];

fn work() -> impl Strategy<Value = Work> {
    (
        prop::collection::vec((0..6usize, 0..MOVE_TO.len(), any::<bool>()), 0..=3),
        prop::collection::vec((0..6usize, 0..5u8), 0..=2),
    )
        .prop_map(|(moves, changes)| Work { moves, changes })
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
                    files.insert(path.clone(), f(&format!("{path} on {line}\n")));
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

#[test]
fn following_moves_agrees_with_a_model() {
    let h = std::cell::RefCell::new(Hist::new());
    let mut runner = TestRunner::new(Config::with_cases(128));
    runner
        .run(&(1..=6usize, work(), work()), |(n, ours, theirs)| {
            let mut h = h.borrow_mut();
            let files: Tree = (0..n)
                .map(|i| (format!("s{i}"), f(&format!("start {i}\n"))))
                .collect();
            let first = h.first;
            let s = h.save("main", first, &files);
            let (o, noted_o) = do_work(&mut h, "main", s, &files, &ours);
            let (t, noted_t) = do_work(&mut h, "side", s, &files, &theirs);
            let [vs, vo, vt] = [s, o, t].map(|x| h.version(x));
            let store = h.repo.store();
            let read = |v: Version| read_tree(store, v.root, &HashMap::new());
            let three = [read(vs), read(vo), read(vt)];
            // What each side's notes say, checked against the start and the side.
            let went: [BTreeMap<String, String>; 2] =
                [(&noted_o, 1), (&noted_t, 2)].map(|(n, v)| {
                    n.iter()
                        .filter(|(a, x)| {
                            file_at(&three[0], a) && room(&three[0], x) && file_at(&three[v], x)
                        })
                        .map(|(a, x)| (a.clone(), x.clone()))
                        .collect()
                });
            let replaced: [BTreeSet<&String>; 2] =
                [&noted_o, &noted_t].map(|n| n.values().collect());
            let listed = |m: &BTreeMap<String, String>| m.clone().into_iter().collect::<Vec<_>>();
            prop_assert_eq!(h.followed(vs, vo), listed(&went[0]));
            prop_assert_eq!(h.followed(vs, vt), listed(&went[1]));
            // The table, applied to the three versions as stored.
            let mut was = three.clone();
            let mut clashes: BTreeMap<String, Clash> = BTreeMap::new();
            let froms: BTreeSet<&String> = went[0].keys().chain(went[1].keys()).collect();
            for a in froms {
                let (moved_on, to, other) = match (went[0].get(a), went[1].get(a)) {
                    (Some(x), Some(y)) if x == y => {
                        let v = was[0].remove(a).unwrap();
                        was[0].insert(x.clone(), v);
                        continue;
                    }
                    (Some(x), Some(y)) => {
                        let clash = Clash::MovedApart {
                            ours_to: x.clone(),
                            theirs_to: y.clone(),
                        };
                        clashes.insert(a.clone(), clash);
                        continue;
                    }
                    (Some(x), None) => (Side::Ours, x, 2),
                    (None, Some(y)) => (Side::Theirs, y, 1),
                    (None, None) => unreachable!(),
                };
                let held = if replaced[other - 1].contains(a) {
                    None
                } else {
                    three[other].get(a)
                };
                let to = to.clone();
                let clash = match held {
                    None => Clash::MovedRemoved { moved_on, to },
                    Some(v) if !matches!(v, V::File(..)) => continue,
                    Some(_) if !room(&three[other], &to) => Clash::MovedOnto { moved_on, to },
                    Some(_) => {
                        for v in [0, other] {
                            let moved = was[v].remove(a).unwrap();
                            was[v].insert(to.clone(), moved);
                        }
                        continue;
                    }
                };
                clashes.insert(a.clone(), clash);
            }
            // A move's conflict is the only one at its old place.
            for a in clashes.keys() {
                let below = format!("{a}/");
                for v in &mut was {
                    v.retain(|p, _| p != a && !p.starts_with(&below));
                }
            }
            let mut want = model(&was[0], &was[1], &was[2]);
            want.conflicts.extend(clashes.keys().cloned());
            let mut packs = h.repo.new_packs().unwrap();
            let mut made = Made::default();
            let rules = RuleMatcher::new(&Rules::default());
            let m =
                merge_three(&h.repo, &mut packs, &mut made, false, &vs, &vo, &vt, &rules).unwrap();
            let got: BTreeSet<String> = m.conflicts.iter().map(|c| c.path.clone()).collect();
            prop_assert_eq!(&got, &want.conflicts);
            for c in &m.conflicts {
                match clashes.get(&c.path) {
                    Some(clash) => prop_assert_eq!(&c.clash, clash),
                    None => prop_assert!(!is_move_clash(&c.clash), "{c:?}"),
                }
            }
            if got.is_empty() {
                prop_assert_eq!(read_tree(store, m.root, &made.listings), want.files);
            }
            Ok(())
        })
        .unwrap();
}
