#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use super::*;
use crate::combine::{self, Combined, Keep, Named};
use crate::conflict::tests::file_of;
use crate::conflict::{Listings, Record, Term};
use crate::diary::ServerLines;
use crate::lines::{self, Target};
use crate::listing::{Entry, EntryKind, Listing};
use crate::pack::ObjectKind;
use crate::save::{EXTRA_GIT_COMMIT, Person};
use crate::snapshot::{SaveOutcome, SaveRequest, save_folder};
use crate::status::Standing;
use crate::status::tests::keep;
use crate::switch::goto;
use crate::test_support::{commit_records, plant_root, plant_with, sample_records, symlink};
use crate::undo::{StepRequest, redo, undo};

fn ada() -> Person {
    Person {
        name: "Ada".into(),
        email: "ada@example.com".into(),
        time_ms: 1_700_000_000_000,
        tz_offset_min: 0,
    }
}

const PIC_BASE: &[u8] = b"\0pic base";
const PIC_MAIN: &[u8] = b"\0pic main";
const PIC_SIDE: &[u8] = b"\0pic side";
const A_SIDE: &[u8] = b"one\ntwo side\nthree\n";
const A_FIXED: &[u8] = b"one\ntwo fixed\nthree\n";
/// Still holding an opening marker line, and holding what no save holds.
const A_HALF: &[u8] = b"one\n<<<<<<< main\ntwo half\nthree\n";

struct Fx {
    dir: tempfile::TempDir,
    repo: Repository,
}

impl Fx {
    fn new() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path(), &ada()).unwrap();
        Fx { dir, repo }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn write(&self, rel: &str, bytes: &[u8]) {
        let at = self.root().join(rel);
        fs::create_dir_all(at.parent().unwrap()).unwrap();
        fs::write(at, bytes).unwrap();
    }

    fn read(&self, rel: &str) -> Option<Vec<u8>> {
        fs::read(self.root().join(rel)).ok()
    }

    fn save(&mut self, message: &str) -> Id {
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
        match save_folder(&mut self.repo, &req, &mut |_| {}, &mut |_| true).unwrap() {
            SaveOutcome::Saved { save_id, .. } => save_id,
            other => panic!("{other:?}"),
        }
    }

    fn line_new(&mut self, name: &str) {
        lines::new_line(&mut self.repo, name, &ada(), &mut |_| {}).unwrap();
    }

    fn goto(&mut self, target: Target) {
        goto(&mut self.repo, &target, &ada(), "goto", None, &mut |_| {}).unwrap();
    }

    fn combine(&mut self, line: &str) -> Id {
        let named = Named {
            raw: line.into(),
            shown: line.into(),
        };
        let target = Target::Line(line.into());
        let who = ada();
        let done = combine::combine(
            &mut self.repo,
            &target,
            &named,
            None,
            &who,
            None,
            Keep::Conflicts,
            &mut |_| {},
        )
        .unwrap();
        assert!(matches!(done.result, Combined::Made { .. }), "{done:?}");
        done.save
    }

    fn tip(&self) -> Id {
        crate::history::current_save(&self.repo).unwrap()
    }

    fn head(&self) -> Id {
        self.repo.head().unwrap()
    }

    fn save_of(&self, id: Id) -> Save {
        self.repo.store().save(&id, false).unwrap()
    }

    fn resolve(
        &mut self,
        paths: Option<&[&str]>,
        keep: Option<Side>,
    ) -> Result<ResolveOutcome, ResolveError> {
        self.resolve_with(paths, keep, false, &mut |_| true)
    }

    fn resolve_with(
        &mut self,
        paths: Option<&[&str]>,
        keep: Option<Side>,
        accept_crlf: bool,
        ask: &mut dyn FnMut(&Ask) -> bool,
    ) -> Result<ResolveOutcome, ResolveError> {
        let paths: Option<Vec<String>> = paths.map(|p| p.iter().map(|s| (*s).to_owned()).collect());
        let who = ada();
        let req = ResolveRequest {
            paths: paths.as_deref(),
            keep,
            who: &who,
            command: "resolve",
            accept_crlf,
            git_excludes: None,
        };
        resolve(&mut self.repo, &req, &mut |_| {}, ask)
    }

    fn undo(&mut self) {
        let who = ada();
        let req = StepRequest {
            n: 1,
            shelve: false,
            who: &who,
            git_excludes: None,
        };
        undo(&mut self.repo, &req, &mut |_| {}).unwrap();
    }

    fn redo(&mut self) {
        let who = ada();
        let req = StepRequest {
            n: 1,
            shelve: false,
            who: &who,
            git_excludes: None,
        };
        redo(&mut self.repo, &req, &mut |_| {}).unwrap();
    }

    fn rows(&self) -> Vec<(String, Standing)> {
        crate::status::status(&self.repo, false, None)
            .unwrap()
            .conflicts
            .into_iter()
            .map(|c| (c.path, c.standing))
            .collect()
    }

    /// The folder, `.opalro` aside: each file's bytes, each folder as `None`.
    fn disk(&self) -> BTreeMap<String, Option<Vec<u8>>> {
        let mut out = BTreeMap::new();
        let mut todo = vec![String::new()];
        while let Some(at) = todo.pop() {
            for e in fs::read_dir(self.root().join(&at)).unwrap() {
                let e = e.unwrap();
                let name = e.file_name().into_string().unwrap();
                if at.is_empty() && name == ".opalro" {
                    continue;
                }
                let path = if at.is_empty() {
                    name
                } else {
                    format!("{at}/{name}")
                };
                if e.file_type().unwrap().is_dir() {
                    out.insert(path.clone(), None);
                    todo.push(path);
                } else {
                    out.insert(path, Some(fs::read(e.path()).unwrap()));
                }
            }
        }
        out
    }

    /// What `goto` writes for `save` into an empty folder: each conflict as what it stands as.
    fn goto_writes(&self, save: Id) -> BTreeMap<String, Option<Vec<u8>>> {
        let unstored = HashMap::new();
        let listings = Listings::standing(&self.repo, &unstored);
        let mut out = BTreeMap::new();
        let mut todo = vec![(String::new(), self.save_of(save).root)];
        while let Some((at, id)) = todo.pop() {
            for e in listings.read(&id).unwrap().entries() {
                let path = if at.is_empty() {
                    e.name.clone()
                } else {
                    format!("{at}/{}", e.name)
                };
                if e.kind == EntryKind::Folder {
                    out.insert(path.clone(), None);
                    todo.push((path, e.id));
                } else {
                    let mut bytes = Vec::new();
                    self.repo.store().copy_content(&e.id, &mut bytes).unwrap();
                    out.insert(path, Some(bytes));
                }
            }
        }
        out
    }

    fn assert_folder_is_goto_of(&self, save: Id) {
        assert_eq!(self.disk(), self.goto_writes(save));
    }

    /// The entry the save holds at `path`, read as stored.
    fn entry(&self, save: Id, path: &str) -> Option<Entry> {
        let unstored = HashMap::new();
        let listings = Listings::stored(self.repo.store(), &unstored);
        let mut tree = crate::writeout::Tree::new(&listings, self.save_of(save).root, EXACT);
        tree.entry(path).unwrap()
    }

    /// The bytes of the ordinary file the save holds at `path`.
    fn file(&self, save: Id, path: &str) -> Vec<u8> {
        let e = self.entry(save, path).unwrap();
        assert_eq!(e.kind, EntryKind::File, "{path}");
        let mut bytes = Vec::new();
        self.repo.store().copy_content(&e.id, &mut bytes).unwrap();
        bytes
    }

    fn reopened(self) -> Fx {
        let Fx { dir, repo } = self;
        drop(repo);
        let repo = Repository::open(dir.path()).unwrap();
        Fx { dir, repo }
    }
}

const EXACT: crate::drive::Drive = crate::drive::Drive {
    ignores_case: false,
    composes_unicode: false,
};

/// `main` and `side` changed `a.txt` in the same place, and each changed `pic.png` and
/// `art/hero.png`; `side` changed `gone.bin`, which `main` removed. The project is on `main`, at
/// the save combining `side` into it, which holds four conflicts: a marked text, two pictures
/// standing as `main`'s, and a place standing as nothing.
fn conflicted() -> (Fx, Id) {
    let mut fx = Fx::new();
    fx.write("a.txt", b"one\ntwo\nthree\n");
    fx.write("pic.png", PIC_BASE);
    fx.write("art/hero.png", b"\0hero base");
    fx.write("gone.bin", b"\0gone base");
    fx.write("notes.txt", b"notes\n");
    fx.save("Start");
    fx.line_new("side");
    fx.write("a.txt", A_SIDE);
    fx.write("pic.png", PIC_SIDE);
    fx.write("art/hero.png", b"\0hero side");
    fx.write("gone.bin", b"\0gone side");
    fx.save("Side work");
    fx.goto(Target::Line("main".into()));
    fx.write("a.txt", b"one\ntwo main\nthree\n");
    fx.write("pic.png", PIC_MAIN);
    fx.write("art/hero.png", b"\0hero main");
    fs::remove_file(fx.root().join("gone.bin")).unwrap();
    fx.save("Main work");
    let combined = fx.combine("side");
    assert_eq!(
        fx.rows(),
        [
            ("a.txt".to_owned(), Standing::Marked { left: 1 }),
            ("art/hero.png".to_owned(), Standing::Yours),
            ("gone.bin".to_owned(), Standing::NotInFolder),
            ("pic.png".to_owned(), Standing::Yours),
        ]
    );
    (fx, combined)
}

/// `main` made the file `dir`, `side` the folder `dir` holding two files. The project is on
/// `main`, at the save combining them, which holds a conflict at `dir` standing as the file.
fn file_against_folder() -> (Fx, Id) {
    let mut fx = Fx::new();
    fx.write("keep.txt", b"k\n");
    fx.save("Start");
    fx.line_new("side");
    fx.write("dir/x.txt", b"x\n");
    fx.write("dir/y.txt", b"y\n");
    fx.write("dir/sub/z.txt", b"z\n");
    fx.save("Side work");
    fx.goto(Target::Line("main".into()));
    fx.write("dir", b"a file\n");
    fx.save("Main work");
    let combined = fx.combine("side");
    assert_eq!(fx.rows(), [("dir".to_owned(), Standing::Yours)]);
    (fx, combined)
}

fn recorded(outcome: ResolveOutcome) -> (Id, Save, Option<Id>, Vec<String>, Vec<String>, u64) {
    match outcome {
        ResolveOutcome::Recorded {
            save_id,
            save,
            replaced,
            recorded,
            left,
            left_count,
            ..
        } => (
            save_id,
            save,
            replaced,
            recorded,
            left.into_iter().map(|c| c.path).collect(),
            left_count,
        ),
        other => panic!("{other:?}"),
    }
}

#[test]
fn resolve_records_every_fixed_conflict_into_the_combined_save() {
    let (mut fx, combined) = conflicted();
    fx.write("a.txt", A_FIXED);
    let before = fx.disk();
    let (id, save, replaced, done, left, count) = recorded(fx.resolve(None, None).unwrap());
    let was = fx.save_of(combined);
    assert_eq!(replaced, Some(combined));
    assert_eq!(save, fx.save_of(id));
    assert_eq!(save.change_id, was.change_id);
    assert_eq!(save.parents, was.parents);
    assert_eq!(save.message, was.message);
    assert_eq!(save.author, was.author);
    assert_eq!(done, ["a.txt"]);
    assert_eq!(left, ["art/hero.png", "gone.bin", "pic.png"]);
    assert_eq!(count, 3);
    assert_eq!(fx.tip(), id);
    assert_eq!(fx.file(id, "a.txt"), A_FIXED);
    assert_eq!(fx.disk(), before, "the folder is unchanged");
    fx.assert_folder_is_goto_of(id);
    let (_, entry) = fx.repo.current().unwrap();
    assert_eq!(entry.command, "resolve");
    assert_eq!(entry.folder_change, None);
}

#[test]
fn resolve_makes_a_new_save_when_the_combined_save_is_shown_elsewhere() {
    for by in ["tag", "line", "server line"] {
        let (mut fx, combined) = conflicted();
        match by {
            "tag" => {
                crate::tags::tag(
                    &mut fx.repo,
                    "v1.0",
                    None,
                    None,
                    &ada(),
                    &mut |_| {},
                    &mut |_| true,
                )
                .unwrap();
            }
            "line" => {
                fx.line_new("other");
                fx.goto(Target::Line("main".into()));
            }
            _ => {
                let servers = ServerLines {
                    servers: BTreeMap::from([(
                        "origin".to_owned(),
                        BTreeMap::from([("main".to_owned(), combined)]),
                    )]),
                };
                let (tags, _) = sample_records();
                commit_records(&mut fx.repo, &tags, &servers);
            }
        }
        fx.write("a.txt", A_FIXED);
        let (id, save, replaced, done, _, _) = recorded(fx.resolve(None, None).unwrap());
        assert_eq!(replaced, None, "{by}");
        assert_eq!(save.parents, [combined], "{by}");
        assert_eq!(save.message, "Fixed conflicts in a.txt", "{by}");
        assert_ne!(save.change_id, fx.save_of(combined).change_id, "{by}");
        assert_eq!(done, ["a.txt"]);
        assert_eq!(fx.tip(), id);
        fx.assert_folder_is_goto_of(id);
    }
}

#[test]
fn resolve_makes_a_new_save_when_the_newest_save_is_not_combined() {
    let (mut fx, _) = conflicted();
    fx.write("notes.txt", b"more notes\n");
    let more = fx.save("More notes");
    fx.write("a.txt", A_FIXED);
    fx.write("pic.png", b"\0painted");
    let (id, save, replaced, _, _, _) =
        recorded(fx.resolve(Some(&["a.txt", "pic.png"]), None).unwrap());
    assert_eq!(replaced, None);
    assert_eq!(save.parents, [more]);
    assert_eq!(save.message, "Fixed conflicts in 2 places");
    assert_eq!(fx.file(id, "pic.png"), b"\0painted");
    fx.assert_folder_is_goto_of(id);
}

#[test]
fn resolve_a_path_records_only_that_place_and_leaves_other_unsaved_changes_unsaved() {
    let (mut fx, _) = conflicted();
    fx.write("a.txt", A_FIXED);
    fx.write("pic.png", b"\0painted");
    fx.write("notes.txt", b"changed\n");
    fx.write("new.txt", b"new\n");
    let (id, _, _, done, left, _) = recorded(fx.resolve(Some(&["a.txt"]), None).unwrap());
    assert_eq!(done, ["a.txt"]);
    assert_eq!(left, ["art/hero.png", "gone.bin", "pic.png"]);
    assert_eq!(fx.file(id, "a.txt"), A_FIXED);
    assert_eq!(fx.file(id, "notes.txt"), b"notes\n");
    assert!(fx.entry(id, "new.txt").is_none());
    assert_eq!(fx.entry(id, "pic.png").unwrap().kind, EntryKind::Conflict);
    let status = crate::status::status(&fx.repo, false, None).unwrap();
    let changed: Vec<&str> = status.changes.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(changed, ["new.txt", "notes.txt"]);
    assert_eq!(fx.read("pic.png").unwrap(), b"\0painted");
}

#[test]
fn resolve_a_folder_means_every_conflict_below_it() {
    let (mut fx, _) = conflicted();
    fx.write("art/hero.png", b"\0hero painted");
    fx.write("pic.png", b"\0painted");
    // Typed in another case on a drive that ignores it, it still names the stored place.
    let typed = if fx.repo.drive().unwrap().ignores_case {
        "ART"
    } else {
        "art"
    };
    let (id, _, _, done, left, _) = recorded(fx.resolve(Some(&[typed]), None).unwrap());
    assert_eq!(done, ["art/hero.png"]);
    assert_eq!(left, ["a.txt", "gone.bin", "pic.png"]);
    assert_eq!(fx.file(id, "art/hero.png"), b"\0hero painted");
    // The whole project is a folder too, with a version for each.
    let (_, _, _, done, left, _) = recorded(fx.resolve(Some(&[""]), Some(Side::Theirs)).unwrap());
    assert_eq!(done, ["a.txt", "gone.bin", "pic.png"]);
    assert!(left.is_empty());
}

#[test]
fn resolve_refuses_a_path_not_in_conflict_and_records_nothing() {
    let (mut fx, combined) = conflicted();
    fx.write("a.txt", A_FIXED);
    let head = fx.head();
    let err = fx.resolve(Some(&["a.txt", "notes.txt"]), None).unwrap_err();
    match &err {
        ResolveError::NotInConflict { path } => assert_eq!(path, "notes.txt"),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        err.to_string(),
        "notes.txt isn't in conflict, so nothing was recorded."
    );
    assert_eq!(
        err.hint().unwrap(),
        "opalro status lists the places in conflict."
    );
    assert_eq!(fx.head(), head);
    assert_eq!(fx.tip(), combined);
    // Below a place nothing is in conflict either.
    let err = fx.resolve(Some(&["a.txt/x"]), None).unwrap_err();
    assert!(matches!(err, ResolveError::NotInConflict { .. }), "{err:?}");
}

#[test]
fn resolve_refuses_an_untouched_version_conflict_without_keep() {
    let (mut fx, combined) = conflicted();
    fx.write("a.txt", A_FIXED);
    let head = fx.head();
    let err = fx.resolve(Some(&["a.txt", "pic.png"]), None).unwrap_err();
    assert_eq!(
        err.to_string(),
        "pic.png still holds your version, and Opalro won't guess which one you want, so \
         nothing was recorded."
    );
    assert_eq!(
        err.hint().unwrap(),
        "Say which to keep: opalro resolve <path> --keep mine, or --keep theirs. Or put the \
         version you want in the folder and run opalro resolve <path>."
    );
    // All of them named together; one standing as nothing still holds your version too.
    let err = fx
        .resolve(Some(&["pic.png", "art", "gone.bin"]), None)
        .unwrap_err();
    match &err {
        ResolveError::StillYours { paths } => {
            assert_eq!(paths, &["art/hero.png", "gone.bin", "pic.png"]);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        err.to_string(),
        "These still hold your version, and Opalro won't guess which one you want, so nothing \
         was recorded:\n  art/hero.png\n  gone.bin\n  pic.png"
    );
    assert_eq!(fx.head(), head);
    assert_eq!(fx.tip(), combined);
}

#[test]
fn resolve_without_paths_never_records_a_version_conflict() {
    let (mut fx, combined) = conflicted();
    fx.write("pic.png", b"\0painted");
    fx.write("gone.bin", b"\0put back");
    let head = fx.head();
    match fx.resolve(None, None).unwrap() {
        ResolveOutcome::Nothing { left, left_count } => {
            let rows: Vec<(&str, Standing)> =
                left.iter().map(|c| (c.path.as_str(), c.standing)).collect();
            assert_eq!(
                rows,
                [
                    ("a.txt", Standing::Marked { left: 1 }),
                    ("art/hero.png", Standing::Yours),
                    ("gone.bin", Standing::ChangedInPlace),
                    ("pic.png", Standing::ChangedInPlace),
                ]
            );
            assert_eq!(left_count, 4);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(fx.head(), head);
    // Saved as the conflict's own version, it is still not recorded without its path.
    let saved = fx.save("Painted");
    assert!(matches!(
        fx.resolve(None, None).unwrap(),
        ResolveOutcome::Nothing { left_count: 4, .. }
    ));
    assert_eq!(fx.tip(), saved);
    // Named, it is recorded as that version.
    let (id, _, _, done, _, _) = recorded(fx.resolve(Some(&["pic.png"]), None).unwrap());
    assert_eq!(done, ["pic.png"]);
    assert_eq!(fx.file(id, "pic.png"), b"\0painted");
    assert_ne!(id, combined);
}

#[test]
fn resolve_asks_about_markers_left_and_records_on_yes() {
    let (mut fx, combined) = conflicted();
    let marked = fx.read("a.txt").unwrap();
    let mut asked = Vec::new();
    let outcome = fx
        .resolve_with(Some(&["a.txt"]), None, false, &mut |a| {
            match a {
                Ask::MarkersLeft { path, left } => asked.push(((*path).to_owned(), *left)),
                Ask::Large(_) => panic!("asked about a large save"),
            }
            true
        })
        .unwrap();
    assert_eq!(asked, [("a.txt".to_owned(), 1)]);
    let (id, _, replaced, _, _, _) = recorded(outcome);
    assert_eq!(replaced, Some(combined));
    assert_eq!(fx.file(id, "a.txt"), marked);
    fx.assert_folder_is_goto_of(id);
}

#[test]
fn a_no_records_nothing() {
    let (mut fx, combined) = conflicted();
    fx.write("a.txt", A_HALF);
    let head = fx.head();
    let err = fx
        .resolve_with(Some(&["a.txt"]), None, false, &mut |_| false)
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "a.txt still has 1 marked place, so nothing was recorded."
    );
    assert_eq!(
        err.hint().unwrap(),
        "Fix them first, or run it again with --yes to record the file as it is."
    );
    assert_eq!((fx.head(), fx.tip()), (head, combined));
    assert_eq!(fx.read("a.txt").unwrap(), A_HALF);
}

#[test]
fn resolve_keep_theirs_writes_theirs_whole() {
    let (mut fx, combined) = conflicted();
    let outcome = fx
        .resolve(Some(&["pic.png", "a.txt"]), Some(Side::Theirs))
        .unwrap();
    let ResolveOutcome::Recorded { safety_copies, .. } = &outcome else {
        panic!("{outcome:?}");
    };
    // The marked text is the combined save's own version, so it needs no safety copy.
    assert_eq!(*safety_copies, 0);
    let (id, _, replaced, done, left, _) = recorded(outcome);
    assert_eq!(replaced, Some(combined));
    assert_eq!(done, ["a.txt", "pic.png"]);
    assert_eq!(left, ["art/hero.png", "gone.bin"]);
    assert_eq!(fx.read("pic.png").unwrap(), PIC_SIDE);
    assert_eq!(fx.read("a.txt").unwrap(), A_SIDE);
    assert_eq!(fx.file(id, "a.txt"), A_SIDE);
    fx.assert_folder_is_goto_of(id);
    let (_, entry) = fx.repo.current().unwrap();
    assert_eq!(entry.command, "resolve");
    assert!(entry.folder_change.is_some());
}

#[test]
fn resolve_keep_theirs_keeps_a_safety_copy_of_a_half_fix() {
    let (mut fx, _) = conflicted();
    fx.write("a.txt", A_HALF);
    let outcome = fx.resolve(Some(&["a.txt"]), Some(Side::Theirs)).unwrap();
    let ResolveOutcome::Recorded { safety_copies, .. } = &outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(*safety_copies, 1);
    assert_eq!(fx.read("a.txt").unwrap(), A_SIDE);
    // Undoing it brings the half fix back from the safety copy.
    fx.undo();
    assert_eq!(fx.read("a.txt").unwrap(), A_HALF);
}

#[test]
fn resolve_keep_mine_where_mine_is_nothing_removes_the_file() {
    let (mut fx, _) = conflicted();
    fx.write("gone.bin", b"\0my own");
    let outcome = fx.resolve(Some(&["gone.bin"]), Some(Side::Mine)).unwrap();
    let ResolveOutcome::Recorded { safety_copies, .. } = &outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(*safety_copies, 1);
    let (id, ..) = recorded(outcome);
    assert_eq!(fx.read("gone.bin"), None);
    assert!(fx.entry(id, "gone.bin").is_none());
    fx.assert_folder_is_goto_of(id);
}

#[test]
fn resolve_keep_a_folder_term_writes_the_folder_under_write_outs_rules() {
    let (mut fx, _) = file_against_folder();
    let (id, ..) = recorded(fx.resolve(Some(&["dir"]), Some(Side::Theirs)).unwrap());
    assert_eq!(fx.read("dir/x.txt").unwrap(), b"x\n");
    assert_eq!(fx.read("dir/y.txt").unwrap(), b"y\n");
    assert_eq!(fx.read("dir/sub/z.txt").unwrap(), b"z\n");
    // The side's own folder, whole, under its own listing ID.
    let side = fx.repo.current().unwrap().1.state.lines["side"];
    let theirs = fx.entry(side, "dir").unwrap();
    let kept = fx.entry(id, "dir").unwrap();
    assert_eq!((kept.kind, kept.id), (EntryKind::Folder, theirs.id));
    fx.assert_folder_is_goto_of(id);

    // A planted folder term holding `.OPALRO`, spelled `-OPALRO` and then changed, as no listing
    // Opalro makes holds one.
    let mut fx = Fx::new();
    keep(&mut fx.repo, &[b"mine"]);
    plant_root(&mut fx.repo, |packs| {
        let id = packs.add_content(b"x").unwrap();
        let entry = Entry {
            name: "-OPALRO".into(),
            kind: EntryKind::File,
            id,
            size: 1,
        };
        let mut listing = Listing::new(vec![entry]).unwrap().encode();
        let at = listing.windows(7).position(|w| w == b"-OPALRO").unwrap();
        listing[at] = b'.';
        let hostile = packs
            .add_record(ObjectKind::FolderListing, &listing)
            .unwrap();
        let record = Record {
            terms: vec![
                Some(file_of(b"mine")),
                None,
                Some(Term {
                    kind: EntryKind::Folder,
                    id: hostile,
                    size: 0,
                }),
            ],
            own: None,
        };
        let conflict = packs
            .add_record(ObjectKind::Conflict, &record.encode())
            .unwrap();
        let top = vec![Entry {
            name: "evil".into(),
            kind: EntryKind::Conflict,
            id: conflict,
            size: 0,
        }];
        crate::test_support::add_listing(packs, top).0
    });
    fx.write("evil", b"mine");
    let (head, tip) = (fx.head(), fx.tip());
    let err = fx.resolve(Some(&["evil"]), Some(Side::Theirs)).unwrap_err();
    assert!(err.to_string().contains(".opalro"), "{err}");
    assert_eq!((fx.head(), fx.tip()), (head, tip));
    assert_eq!(fx.read("evil").unwrap(), b"mine");
    assert!(!fx.root().join(".opalro").join("x").exists());
}

/// A project on a save holding a conflict of five terms at `five.bin`, standing as the first.
fn five_terms() -> Fx {
    let mut fx = Fx::new();
    let terms: [&[u8]; 5] = [b"\0mine", b"\0b", b"\0c", b"\0d", b"\0e"];
    keep(&mut fx.repo, &terms);
    let record = Record {
        terms: terms.iter().map(|t| Some(file_of(t))).collect(),
        own: None,
    };
    plant_with(&mut fx.repo, &[], &[("five.bin", record)]);
    fx.write("five.bin", b"\0mine");
    fx
}

#[test]
fn resolve_keep_on_a_record_of_five_terms_is_refused() {
    let mut fx = five_terms();
    let head = fx.head();
    for pick in [Side::Mine, Side::Theirs] {
        let err = fx.resolve(Some(&["five.bin"]), Some(pick)).unwrap_err();
        assert_eq!(
            err.to_string(),
            "five.bin is in conflict between more than two lines, so Opalro can't pick one side \
             of it."
        );
        assert_eq!(
            err.hint().unwrap(),
            "Put the version you want in the folder and run opalro resolve <path>."
        );
    }
    assert_eq!(fx.head(), head);
}

#[test]
fn a_five_term_record_is_recorded_as_it_stands() {
    let mut fx = five_terms();
    fx.write("five.bin", b"\0my pick");
    let (id, ..) = recorded(fx.resolve(Some(&["five.bin"]), None).unwrap());
    assert_eq!(fx.file(id, "five.bin"), b"\0my pick");
    fx.assert_folder_is_goto_of(id);
}

#[test]
fn undo_takes_a_resolve_back_and_a_hand_fix_stays_in_the_folder() {
    let (mut fx, combined) = conflicted();
    fx.write("a.txt", A_FIXED);
    recorded(fx.resolve(None, None).unwrap());
    fx.undo();
    assert_eq!(fx.tip(), combined);
    assert_eq!(fx.read("a.txt").unwrap(), A_FIXED);
    assert_eq!(fx.rows()[0], ("a.txt".to_owned(), Standing::Fixed));
}

#[test]
fn undo_takes_back_a_resolve_keep_and_its_folder_change() {
    let (mut fx, combined) = conflicted();
    recorded(fx.resolve(Some(&["pic.png"]), Some(Side::Theirs)).unwrap());
    assert_eq!(fx.read("pic.png").unwrap(), PIC_SIDE);
    fx.undo();
    assert_eq!(fx.tip(), combined);
    assert_eq!(fx.read("pic.png").unwrap(), PIC_MAIN);
    fx.assert_folder_is_goto_of(combined);
}

#[test]
fn redo_makes_it_again() {
    let (mut fx, _) = conflicted();
    let (id, ..) = recorded(fx.resolve(Some(&["pic.png"]), Some(Side::Theirs)).unwrap());
    fx.undo();
    fx.redo();
    assert_eq!(fx.tip(), id);
    assert_eq!(fx.read("pic.png").unwrap(), PIC_SIDE);
    fx.assert_folder_is_goto_of(id);
}

#[test]
fn resolve_at_a_save_is_refused() {
    let (mut fx, combined) = conflicted();
    fx.goto(Target::Save(combined));
    fx.write("a.txt", A_FIXED);
    let head = fx.head();
    let err = fx.resolve(None, None).unwrap_err();
    assert!(matches!(err, ResolveError::NotOnALine), "{err:?}");
    assert_eq!(
        err.to_string(),
        "You're at a save, not on a line, so there is no line to record fixes on."
    );
    assert_eq!(
        err.hint().unwrap(),
        "Make a line here first: opalro line new <name>"
    );
    assert_eq!(fx.head(), head);
}

/// Commits a save like the line's newest one that says it came from a Git commit.
fn plant_imported(fx: &mut Fx) {
    let lock = fx.repo.lock_for_change("import", &mut |_| {}).unwrap();
    let (head, mut entry) = fx.repo.current().unwrap();
    let tip = fx.tip();
    let mut save = fx.save_of(tip);
    save.parents = vec![tip];
    save.extra
        .insert(EXTRA_GIT_COMMIT.to_owned(), b"0123456789abcdef".to_vec());
    let mut packs = fx.repo.new_packs().unwrap();
    let id = packs.add_record(ObjectKind::Save, &save.encode()).unwrap();
    entry.parent = Some(head);
    entry.folder_change = None;
    entry.state.lines.insert("main".into(), id);
    fx.repo.commit(&lock, packs, &entry).unwrap();
}

#[test]
fn resolve_meets_saves_checks() {
    // Line endings Git rewrote, after an import: stopped, and saved as they are when accepted.
    let mut fx = Fx::new();
    plant_imported(&mut fx);
    keep(&mut fx.repo, &[b"mine\nline\n", b"base", b"theirs"]);
    let record = crate::status::tests::versions(Some(file_of(b"mine\nline\n")));
    plant_with(&mut fx.repo, &[], &[("c.txt", record)]);
    fx.write("c.txt", b"mine\r\nline\r\n");
    let err = fx.resolve(Some(&["c.txt"]), None).unwrap_err();
    assert!(
        matches!(
            err,
            ResolveError::Save(SaveError::LineEndingsChanged { count: 1, .. })
        ),
        "{err:?}"
    );
    let (id, ..) = recorded(
        fx.resolve_with(Some(&["c.txt"]), None, true, &mut |_| true)
            .unwrap(),
    );
    assert_eq!(fx.file(id, "c.txt"), b"mine\r\nline\r\n");

    // A fix that adds unusually much is asked about, and a no records nothing.
    let (mut fx, _) = conflicted();
    fx.write("opalro.toml", b"ask-before-saving-over = \"1 KiB\"\n");
    let rules = fx.save("Rules");
    fx.write("gone.bin", &[7; 2048]);
    let mut asked = 0;
    let err = fx
        .resolve_with(Some(&["gone.bin"]), None, false, &mut |a| {
            assert!(matches!(a, Ask::Large(_)), "{a:?}");
            asked += 1;
            false
        })
        .unwrap_err();
    assert_eq!(asked, 1);
    assert!(
        matches!(err, ResolveError::Save(SaveError::LargeSave(_))),
        "{err:?}"
    );
    assert_eq!(fx.tip(), rules);
    let (id, ..) = recorded(fx.resolve(Some(&["gone.bin"]), None).unwrap());
    assert_eq!(fx.file(id, "gone.bin"), [7; 2048]);

    // A place this computer can't hold can't be recorded from the folder; kept, its version is
    // recorded and write-out says it wasn't written here.
    let drive = Fx::new().repo.drive().unwrap();
    let name = if cfg!(windows) {
        "aux.txt"
    } else if drive.ignores_case {
        "Pic.png"
    } else {
        println!("Skipped: this computer can hold every name the test can plant.");
        return;
    };
    let mut fx = Fx::new();
    keep(&mut fx.repo, &[b"picture", b"base", b"theirs"]);
    let record = crate::status::tests::versions(Some(file_of(b"picture")));
    let files: &[(&str, EntryKind, &[u8])] = &[("pic.png", EntryKind::File, b"picture")];
    plant_with(&mut fx.repo, files, &[(name, record)]);
    fx.write("pic.png", b"picture");
    let head = fx.head();
    let err = fx.resolve(Some(&[name]), None).unwrap_err();
    assert!(
        matches!(&err, ResolveError::Save(SaveError::KeptWhole { path }) if path == name),
        "{err:?}"
    );
    assert_eq!(fx.head(), head);
    match fx.resolve(Some(&[name]), Some(Side::Theirs)).unwrap() {
        ResolveOutcome::Recorded {
            save_id, applied, ..
        } => {
            assert_eq!(fx.file(save_id, name), b"theirs");
            assert_eq!(applied.not_written, [name]);
            let kinds: Vec<&str> = applied.warnings.iter().map(|w| w.kind).collect();
            assert!(
                kinds.contains(&crate::warning::NOT_WRITTEN_HERE),
                "{kinds:?}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(fx.read("pic.png").unwrap(), b"picture");
}

#[test]
fn resolve_says_what_is_left_and_writes_no_extra_field() {
    let (mut fx, combined) = conflicted();
    fx.write("a.txt", A_FIXED);
    let (_, save, ..) = recorded(fx.resolve(None, None).unwrap());
    assert_eq!(save.extra, fx.save_of(combined).extra);

    // Past 100 places, the first 100 are listed and all are counted.
    let mut fx = Fx::new();
    keep(&mut fx.repo, &[b"\0mine", b"base", b"theirs"]);
    let record = crate::status::tests::versions(Some(file_of(b"\0mine")));
    let names: Vec<String> = (0..102).map(|n| format!("p{n:03}.bin")).collect();
    let conflicts: Vec<(&str, Record)> =
        names.iter().map(|n| (n.as_str(), record.clone())).collect();
    plant_with(&mut fx.repo, &[], &conflicts);
    for n in &names {
        fx.write(n, b"\0mine");
    }
    fx.write("p000.bin", b"\0painted");
    let (id, save, _, done, left, count) = recorded(fx.resolve(Some(&["p000.bin"]), None).unwrap());
    assert_eq!(done, ["p000.bin"]);
    assert_eq!(left.len(), 100);
    assert_eq!(left.first().map(String::as_str), Some("p001.bin"));
    assert_eq!(count, 101);
    let parent = fx.save_of(save.parents[0]);
    assert_eq!(fx.save_of(id).extra, parent.extra);
}

#[test]
fn copy_writes_one_version_and_records_nothing() {
    let (fx, _) = conflicted();
    let out = tempfile::tempdir().unwrap();
    let head = fx.head();
    for (pick, want) in [
        (Pick::Mine, PIC_MAIN),
        (Pick::Theirs, PIC_SIDE),
        (Pick::Before, PIC_BASE),
    ] {
        let to = out.path().join(format!("{pick:?}.png"));
        let copied = copy_version(&fx.repo, "pic.png", pick, &to).unwrap();
        assert_eq!(copied.size, want.len() as u64);
        assert_eq!(fs::read(&to).unwrap(), want);
    }
    assert_eq!(fx.head(), head);
    assert_eq!(fx.read("pic.png").unwrap(), PIC_MAIN);
    // A place with no file of the side asked for.
    let err = copy_version(&fx.repo, "gone.bin", Pick::Mine, &out.path().join("g")).unwrap_err();
    assert_eq!(
        err.to_string(),
        "Your line holds no file at gone.bin, so there is nothing to copy."
    );
    let err = copy_version(&fx.repo, "notes.txt", Pick::Mine, &out.path().join("n")).unwrap_err();
    assert!(matches!(err, ResolveError::NotInConflict { .. }), "{err:?}");
}

#[test]
fn copy_never_overwrites_and_never_writes_into_opalro() {
    let (fx, _) = conflicted();
    let out = tempfile::tempdir().unwrap();
    let there = out.path().join("there.png");
    fs::write(&there, b"keep me").unwrap();
    let err = copy_version(&fx.repo, "pic.png", Pick::Theirs, &there).unwrap_err();
    assert!(matches!(err, ResolveError::AlreadyThere { .. }), "{err:?}");
    assert!(
        err.to_string()
            .ends_with(" is already there, so nothing was written.")
    );
    assert_eq!(fs::read(&there).unwrap(), b"keep me");
    let nowhere = out.path().join("nowhere");
    let dangling = out.path().join("dangling.png");
    if symlink(&nowhere, &dangling) {
        let err = copy_version(&fx.repo, "pic.png", Pick::Theirs, &dangling).unwrap_err();
        assert!(matches!(err, ResolveError::AlreadyThere { .. }), "{err:?}");
        assert!(!nowhere.exists());
    }
    let opal = fx.root().join(".opalro");
    let mut into = vec![
        opal.join("x"),
        fx.root().join("art").join("..").join(".opalro").join("x"),
        fx.root().join(".opalro"),
    ];
    // Another name for the same folder, which no spelling rule folds: on Windows, the drive's
    // administrative share, where this account can reach it.
    #[cfg(windows)]
    {
        let real = fs::canonicalize(fx.root()).unwrap();
        let real = real.to_string_lossy();
        let plain = real.trim_start_matches(r"\\?\");
        if let Some((letter, rest)) = plain.split_once(':') {
            let share = std::path::PathBuf::from(format!(r"\\localhost\{letter}${rest}"));
            if share.join(".opalro").is_dir() {
                into.push(share.join(".opalro").join("x"));
            } else {
                println!("Skipped: this account can't reach the administrative share.");
            }
        }
    }
    if fx.repo.drive().unwrap().ignores_case {
        into.push(fx.root().join(".OPALRO").join("x"));
    }
    let link = out.path().join("into");
    #[cfg(windows)]
    {
        crate::test_support::junction(&link, &opal);
        // What Windows opens as `.opalro` itself: a stream on it, and trailing dots and spaces.
        into.push(fx.root().join(".opalro:x"));
        into.push(fx.root().join(".opalro. ").join("x"));
        into.push(link.join("x"));
    }
    #[cfg(not(windows))]
    if symlink(&opal, &link) {
        into.push(link.join("x"));
    }
    for to in into {
        let err = copy_version(&fx.repo, "pic.png", Pick::Theirs, &to).unwrap_err();
        assert!(
            matches!(err, ResolveError::IntoOpalro),
            "{}: {err:?}",
            to.display()
        );
        assert_eq!(
            err.to_string(),
            "Opalro doesn't write into .opalro, so nothing was written."
        );
    }
    assert!(!opal.join("x").exists());
}

#[test]
fn copy_before_with_nothing_there_is_refused() {
    let (fx, _) = file_against_folder();
    let out = tempfile::tempdir().unwrap();
    let to = out.path().join("before");
    let err = copy_version(&fx.repo, "dir", Pick::Before, &to).unwrap_err();
    assert_eq!(
        err.to_string(),
        "There was no file at dir where the lines parted, so there is nothing to copy."
    );
    assert!(!to.exists());
}

#[test]
fn copy_of_a_folder_term_is_refused() {
    let (fx, _) = file_against_folder();
    let out = tempfile::tempdir().unwrap();
    let to = out.path().join("theirs");
    let err = copy_version(&fx.repo, "dir", Pick::Theirs, &to).unwrap_err();
    assert_eq!(
        err.to_string(),
        "dir is a folder on the other line; --copy writes one file."
    );
    assert!(!to.exists());
    // Mine is a file there, and is written.
    let copied = copy_version(&fx.repo, "dir", Pick::Mine, &to).unwrap();
    assert_eq!(copied.size, 7);
    // A record of five terms has no one side to copy.
    let fx = five_terms();
    let err = copy_version(&fx.repo, "five.bin", Pick::Theirs, &out.path().join("f")).unwrap_err();
    assert!(matches!(err, ResolveError::CantPickSide { .. }), "{err:?}");
    assert_eq!(
        err.hint().unwrap(),
        "It names more than two versions; opalro status lists it."
    );
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_resolve_stopped_at_every_step_is_finished_or_never_happened() {
    use crate::fsio::fault;
    use crate::journal::Journal;
    use crate::test_support::after_head;
    for keep in [false, true] {
        let mut finished_later = 0;
        for n in 1.. {
            assert!(n < 300, "the resolve never finished");
            let (mut fx, combined) = conflicted();
            let (bytes, pick) = if keep {
                (A_HALF, Some(Side::Theirs))
            } else {
                (A_FIXED, None)
            };
            fx.write("a.txt", bytes);
            let before = fx.disk();
            fault::arm(n);
            let result = fx.resolve(Some(&["a.txt"]), pick);
            fault::disarm();
            let went_on = after_head(&fx.repo);
            let mut fx = fx.reopened();
            // The next command finishes what was started.
            fx.goto(Target::Line("main".into()));
            assert_eq!(Journal::read(&fx.repo).unwrap(), None, "fault {n}");
            let tip = fx.tip();
            let resolved = tip != combined;
            if resolved {
                let (_, entry) = fx.repo.current().unwrap();
                assert_eq!(entry.command, "resolve", "fault {n}");
                fx.assert_folder_is_goto_of(tip);
                let want = if keep { A_SIDE } else { A_FIXED };
                assert_eq!(fx.read("a.txt").unwrap(), want, "fault {n}");
            } else {
                assert_eq!(fx.disk(), before, "fault {n}: the folder changed");
            }
            if result.is_err() && resolved {
                finished_later += 1;
            }
            if result.is_ok() && !went_on {
                assert!(resolved, "fault {n}");
                if keep {
                    assert!(finished_later >= 3, "only {finished_later} finished later");
                }
                break;
            }
        }
    }
}

/// Resolves `paths` with `change` made to the folder between the read that chose the places and
/// the walk that records them; nothing may be recorded.
#[cfg(feature = "fault-injection")]
fn changed_between_the_reads(
    fx: &mut Fx,
    paths: Option<&[&str]>,
    change: impl FnOnce() + 'static,
) -> ResolveError {
    use crate::fsio::fault;
    let (head, tip) = (fx.head(), fx.tip());
    fault::act_at("resolve:chosen", change);
    let err = fx.resolve(paths, None).unwrap_err();
    assert_eq!((fx.head(), fx.tip()), (head, tip), "something was recorded");
    err
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_place_that_changes_between_the_reads_stops_resolve() {
    let changed = |err: &ResolveError, path: &str| {
        assert!(
            matches!(err, ResolveError::Save(SaveError::ChangedDuringSave { path: p }) if p == path),
            "{err:?}"
        );
        assert_eq!(
            err.to_string(),
            format!("{path} changed while Opalro was reading it.")
        );
    };
    // A fixed text gains markers again.
    let (mut fx, _) = conflicted();
    fx.write("a.txt", A_FIXED);
    let at = fx.root().join("a.txt");
    let err = changed_between_the_reads(&mut fx, None, move || fs::write(at, A_HALF).unwrap());
    changed(&err, "a.txt");
    // A picture changed in place is put back to your line's version.
    let (mut fx, _) = conflicted();
    fx.write("pic.png", b"\0painted");
    let at = fx.root().join("pic.png");
    let err = changed_between_the_reads(&mut fx, Some(&["pic.png"]), move || {
        fs::write(at, PIC_MAIN).unwrap();
    });
    changed(&err, "pic.png");
    // A picture changed in place is removed.
    let (mut fx, _) = conflicted();
    fx.write("pic.png", b"\0painted");
    let at = fx.root().join("pic.png");
    let err = changed_between_the_reads(&mut fx, Some(&["pic.png"]), move || {
        fs::remove_file(at).unwrap();
    });
    changed(&err, "pic.png");
    // A marked text removed, so fixed, comes back.
    let (mut fx, _) = conflicted();
    fs::remove_file(fx.root().join("a.txt")).unwrap();
    let at = fx.root().join("a.txt");
    let err = changed_between_the_reads(&mut fx, None, move || fs::write(at, A_HALF).unwrap());
    changed(&err, "a.txt");
}

#[test]
fn a_file_changed_while_the_markers_question_waits_stops_resolve() {
    let (mut fx, _) = conflicted();
    fx.write("a.txt", A_HALF);
    let (head, tip) = (fx.head(), fx.tip());
    let at = fx.root().join("a.txt");
    let err = fx
        .resolve_with(Some(&["a.txt"]), None, false, &mut |_| {
            fs::write(&at, b"one\nchanged while asked\n<<<<<<< main\n").unwrap();
            true
        })
        .unwrap_err();
    assert!(
        matches!(&err, ResolveError::Save(SaveError::ChangedDuringSave { path }) if path == "a.txt"),
        "{err:?}"
    );
    assert_eq!((fx.head(), fx.tip()), (head, tip));
}

#[test]
fn keeping_a_side_without_places_is_refused() {
    let (mut fx, combined) = conflicted();
    let head = fx.head();
    let err = fx.resolve(None, Some(Side::Theirs)).unwrap_err();
    assert!(matches!(err, ResolveError::KeepNeedsPlaces), "{err:?}");
    assert_eq!(
        err.to_string(),
        "Keeping a side needs the places to keep it at, so nothing was recorded."
    );
    assert_eq!(
        err.hint().unwrap(),
        "Name them: opalro resolve <path> --keep mine (or theirs)."
    );
    assert_eq!((fx.head(), fx.tip()), (head, combined));
    assert_eq!(fx.read("pic.png").unwrap(), PIC_MAIN);
}

#[test]
fn keep_where_the_first_parent_cant_be_read_goes_through_without_notes() {
    // A combined save whose first parent the store doesn't hold, as `save` meets one it can't
    // compare with: it notes nothing and goes on.
    let mut fx = Fx::new();
    keep(&mut fx.repo, &[b"\0mine", b"base", b"\0theirs"]);
    let record = crate::status::tests::versions(Some(file_of(b"\0mine")));
    let record = Record {
        terms: vec![record.terms[0], record.terms[1], Some(file_of(b"\0theirs"))],
        ..record
    };
    plant_with(&mut fx.repo, &[], &[("p.bin", record)]);
    let lock = fx.repo.lock_for_change("test", &mut |_| {}).unwrap();
    let (head, mut entry) = fx.repo.current().unwrap();
    let tip = fx.tip();
    let mut save = fx.save_of(tip);
    save.parents = vec![Id([9; 32]), tip];
    let mut packs = fx.repo.new_packs().unwrap();
    let combined = packs.add_record(ObjectKind::Save, &save.encode()).unwrap();
    entry.parent = Some(head);
    entry.folder_change = None;
    entry.state.lines.insert("main".into(), combined);
    fx.repo.commit(&lock, packs, &entry).unwrap();
    drop(lock);
    fx.write("p.bin", b"\0mine");
    let (id, save, replaced, ..) =
        recorded(fx.resolve(Some(&["p.bin"]), Some(Side::Theirs)).unwrap());
    assert_eq!(replaced, Some(combined));
    assert!(save.move_notes.is_empty());
    assert_eq!(fx.file(id, "p.bin"), b"\0theirs");
    assert_eq!(fx.read("p.bin").unwrap(), b"\0theirs");
}

#[test]
fn a_kept_side_past_a_saves_limits_is_refused() {
    use crate::snapshot::BASE_LIMITS;
    let (mut fx, combined) = file_against_folder();
    let head = fx.head();
    // Two entries now; with their folder, six.
    BASE_LIMITS.set(Some((crate::listing::MAX_SAVE_DEPTH, 3)));
    let result = fx.resolve(Some(&["dir"]), Some(Side::Theirs));
    BASE_LIMITS.set(None);
    let err = result.unwrap_err();
    assert!(
        matches!(err, ResolveError::Save(SaveError::TooLarge { .. })),
        "{err:?}"
    );
    assert!(
        err.to_string()
            .starts_with("This project is too large to save: "),
        "{err}"
    );
    assert_eq!((fx.head(), fx.tip()), (head, combined));
    assert_eq!(fx.read("dir").unwrap(), b"a file\n");
}

#[test]
fn keeping_a_link_term_writes_what_write_out_writes_for_it() {
    // A link leading out of the project is written as a file holding its target.
    let mut fx = Fx::new();
    let held = crate::test_support::plant_save(
        &mut fx.repo,
        &[
            ("l", EntryKind::Symlink, b"../outside"),
            ("t", EntryKind::File, b"mine"),
        ],
    );
    let link = held.get("l").unwrap();
    let record = Record {
        terms: vec![
            Some(file_of(b"mine")),
            None,
            Some(Term {
                kind: EntryKind::Symlink,
                id: link.id,
                size: link.size,
            }),
        ],
        own: None,
    };
    plant_with(&mut fx.repo, &[], &[("l", record)]);
    fx.write("l", b"mine");
    match fx.resolve(Some(&["l"]), Some(Side::Theirs)).unwrap() {
        ResolveOutcome::Recorded {
            save_id, applied, ..
        } => {
            assert_eq!(applied.placeholders, ["l"]);
            assert_eq!(fx.entry(save_id, "l").unwrap().kind, EntryKind::Symlink);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(fx.read("l").unwrap(), b"../outside");
}

#[test]
fn keep_leaves_unsaved_changes_at_other_places_unsaved() {
    let (mut fx, _) = conflicted();
    fx.write("notes.txt", b"changed\n");
    fx.write("new.txt", b"new\n");
    fx.write("art/hero.png", b"\0hero painted");
    let (id, ..) = recorded(fx.resolve(Some(&["pic.png"]), Some(Side::Theirs)).unwrap());
    assert_eq!(fx.read("notes.txt").unwrap(), b"changed\n");
    assert_eq!(fx.read("new.txt").unwrap(), b"new\n");
    assert_eq!(fx.read("art/hero.png").unwrap(), b"\0hero painted");
    assert_eq!(fx.file(id, "notes.txt"), b"notes\n");
    assert!(fx.entry(id, "new.txt").is_none());
    let status = crate::status::status(&fx.repo, false, None).unwrap();
    let changed: Vec<&str> = status.changes.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(changed, ["new.txt", "notes.txt"]);
}

#[test]
fn keep_replaces_a_fix_and_a_saved_own_version() {
    // A fix made by hand is replaced, kept first as a safety copy.
    let (mut fx, _) = conflicted();
    fx.write("a.txt", A_FIXED);
    let outcome = fx.resolve(Some(&["a.txt"]), Some(Side::Theirs)).unwrap();
    let ResolveOutcome::Recorded { safety_copies, .. } = &outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(*safety_copies, 1);
    let (id, ..) = recorded(outcome);
    assert_eq!(fx.read("a.txt").unwrap(), A_SIDE);
    fx.assert_folder_is_goto_of(id);
    fx.undo();
    assert_eq!(fx.read("a.txt").unwrap(), A_FIXED);
    // An own version a save holds is replaced too, and needs no safety copy.
    let (mut fx, _) = conflicted();
    fx.write("pic.png", b"\0painted");
    fx.save("Painted");
    let outcome = fx.resolve(Some(&["pic.png"]), Some(Side::Theirs)).unwrap();
    let ResolveOutcome::Recorded { safety_copies, .. } = &outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(*safety_copies, 0);
    let (id, ..) = recorded(outcome);
    assert_eq!(fx.read("pic.png").unwrap(), PIC_SIDE);
    assert_eq!(fx.file(id, "pic.png"), PIC_SIDE);
    fx.assert_folder_is_goto_of(id);
}
