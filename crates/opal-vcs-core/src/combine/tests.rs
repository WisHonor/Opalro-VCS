#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet, HashMap};

use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};

use super::tree::{MAX_HELD_TEXT, Made, Merged, merge_trees};
use super::*;
use crate::changes::DEADLINE;
use crate::id::{Flavor, Id, hash_record};
use crate::kept::overlay;
use crate::link::ProjectLink;
use crate::listing::{Entry, EntryKind, Listing};
use crate::names::PortabilityProblem;
use crate::pack::{ObjectKind, PackSet};
use crate::presets::{Engine, Found, Preset, opal_toml};
use crate::repo::{ChangeLock, Repository};
use crate::rules::{RuleMatcher, Rules};
use crate::save::Person;
use crate::snapshot::store_listing;
use crate::store::counted;

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
        overlay(self.repo.store(), self.empty, &changes, packs).unwrap()
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
        let store = self.repo.store();
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
