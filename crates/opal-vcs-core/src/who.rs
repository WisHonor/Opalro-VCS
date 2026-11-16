//! `opalro who`: the save behind each line of a file. It reads and never writes.

use std::collections::{BinaryHeap, HashMap};

use similar::DiffOp;

use crate::changes::{self, Side};
use crate::conflict::Listings;
use crate::history;
use crate::id::Id;
use crate::kept;
use crate::listing::{Entry, EntryKind};
use crate::moves;
use crate::names::{self, NameError};
use crate::repo::{RepoError, Repository};
use crate::save::Save;
use crate::snapshot::SaveError;
use crate::store::{Store, StoreError};
use crate::text::escape;
use crate::writeout::{Folders, Tree};

/// Which version of the file is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asked {
    Folder,
    Save(Id),
}

pub struct WhoRequest<'a> {
    /// A project path, as typed.
    pub path: &'a str,
    pub asked: Asked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum By {
    /// The save the line is from, and the path the file had there when it was another.
    Save { id: Id, was: Option<String> },
    /// In the folder only.
    Unsaved,
}

/// Lines that follow each other and are from the same save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    /// Counts from 1.
    pub first: usize,
    pub lines: usize,
    pub by: By,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Whose {
    /// As the save spells it; as typed when no save has it.
    pub path: String,
    /// The save asked about, or the one the project is at.
    pub at: Id,
    /// The version asked about.
    pub text: Vec<u8>,
    /// Every line once, in order.
    pub runs: Vec<Run>,
    /// Some comparison ran past its deadline, so a line may be named for a newer save.
    pub coarse: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum WhoError {
    #[error("{} isn't a path in the project: {reason}.", escape(path, false))]
    NotAPath { path: String, reason: NameError },
    #[error("{}", no_file(path, *save))]
    NoFile { path: String, save: Option<Id> },
    #[error(
        "{} is {}, and opalro who reads one file.",
        escape(path, false),
        kind_words(*kind)
    )]
    NotAFile { path: String, kind: EntryKind },
    #[error(
        "{} isn't text, so it has no lines to name a save for.",
        escape(path, false)
    )]
    NotText { path: String },
    #[error(
        "{} is larger than 64 MiB, the most Opalro compares line by line.",
        escape(path, false)
    )]
    TooBig { path: String },
    #[error(
        "{} has more than 1,000,000 lines, the most Opalro compares line by line.",
        escape(path, false)
    )]
    TooManyLines { path: String },
    #[error(transparent)]
    Repo(#[from] RepoError),
    #[error(transparent)]
    Save(#[from] SaveError),
}

/// A listing that can't be read is damage, named as every walk names it.
impl From<StoreError> for WhoError {
    fn from(e: StoreError) -> Self {
        WhoError::Repo(e.into())
    }
}

fn no_file(path: &str, save: Option<Id>) -> String {
    match save {
        Some(id) => format!(
            "Save {} has no file at {}.",
            id.short_hex(),
            escape(path, false)
        ),
        None => format!("{} isn't a file in your folder.", escape(path, false)),
    }
}

fn kind_words(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Folder => "a folder",
        EntryKind::Symlink => "a link",
        EntryKind::ProjectLink => "a project link",
        EntryKind::Conflict => "a place in conflict",
        EntryKind::File | EntryKind::Executable => "a file",
    }
}

/// One version of the file waiting for its turn: the lines that reached it, each as its line in
/// the version asked about and its line here.
struct Waiting {
    entry: Entry,
    lines: Vec<(usize, usize)>,
}

/// Names the save behind each line of the file at `req.path`, through every parent, the first
/// first, and through the moves saves recorded. Reads only.
pub fn who(repo: &Repository, req: &WhoRequest<'_>) -> Result<Whose, WhoError> {
    let typed = req.path;
    names::validate_path(typed).map_err(|reason| WhoError::NotAPath {
        path: typed.to_owned(),
        reason,
    })?;
    let store = repo.store();
    let unstored = HashMap::new();
    let standing = Listings::standing(repo, &unstored);
    let stored = Listings::stored(store, &unstored);
    let at = match req.asked {
        Asked::Save(id) => id,
        Asked::Folder => history::current_save(repo)?,
    };
    let root = read_save(repo, at)?.root;
    let found = Tree::new(&standing, root, kept::EXACT).entry_as_typed(typed, repo.drive()?)?;

    let mut coarse = false;
    let mut waiting: HashMap<(Id, String), Waiting> = HashMap::new();
    let mut heap = BinaryHeap::new();
    let (path, text, mut by) = match req.asked {
        Asked::Save(_) => {
            let (path, entry) = found.ok_or_else(|| WhoError::NoFile {
                path: typed.to_owned(),
                save: Some(at),
            })?;
            if !is_file(entry.kind) {
                return Err(WhoError::NotAFile {
                    path,
                    kind: entry.kind,
                });
            }
            let side = changes::stored(store, Some(entry.kind), Some(entry.id), Some(entry.size))?;
            let text = asked_text(side, &path)?;
            let n = changes::line_count(&text);
            let lines = (0..n).map(|i| (i, i)).collect();
            heap.push((0, at, path.clone()));
            waiting.insert((at, path.clone()), Waiting { entry, lines });
            (path, text, vec![By::Unsaved; n])
        }
        Asked::Folder => {
            let saved = found.filter(|(_, e)| is_file(e.kind));
            let path = saved.as_ref().map_or(typed, |(p, _)| p.as_str()).to_owned();
            let mut disk = Folders::open(repo.root()).map_err(|source| SaveError::Unreadable {
                path: "the project folder".to_owned(),
                source,
            })?;
            let side = changes::in_folder(&mut disk, typed)?.ok_or_else(|| WhoError::NoFile {
                path: path.clone(),
                save: None,
            })?;
            let text = asked_text(side, &path)?;
            let n = changes::line_count(&text);
            if let Some((_, entry)) = saved {
                let keys = keys_of(&text);
                let mut left = (0..n).map(|i| (i, i)).collect();
                let taken = shared(store, &entry, None, &keys, &mut left, &mut coarse)?;
                if !taken.is_empty() {
                    heap.push((0, at, path.clone()));
                    waiting.insert(
                        (at, path.clone()),
                        Waiting {
                            entry,
                            lines: taken,
                        },
                    );
                }
            }
            (path, text, vec![By::Unsaved; n])
        }
    };

    // Newest save first, so lines reaching one version by two ways are mostly taken together.
    // A date that lies costs a second look, never a wrong answer.
    while let Some((_, id, at_path)) = heap.pop() {
        let Some(w) = waiting.remove(&(id, at_path.clone())) else {
            continue;
        };
        let save = read_save(repo, id)?;
        let mut left = w.lines;
        if let Side::Bytes(here) = changes::stored(
            store,
            Some(w.entry.kind),
            Some(w.entry.id),
            Some(w.entry.size),
        )? && is_text(&here)
        {
            let keys = keys_of(&here);
            for (i, parent_id) in save.parents.iter().enumerate() {
                if left.is_empty() {
                    break;
                }
                let parent = read_save(repo, *parent_id)?;
                let mut there = at_path.clone();
                if i == 0
                    && let Some(from) = moved_from(&stored, &save, &parent, &at_path)?
                {
                    there = from;
                }
                let Some(e) = Tree::new(&standing, parent.root, kept::EXACT).entry(&there)? else {
                    continue;
                };
                let taken = shared(store, &e, Some(w.entry.id), &keys, &mut left, &mut coarse)?;
                if taken.is_empty() {
                    continue;
                }
                let key = (*parent_id, there);
                match waiting.get_mut(&key) {
                    Some(other) => other.lines.extend(taken),
                    None => {
                        heap.push((parent.saver.time_ms, key.0, key.1.clone()));
                        waiting.insert(
                            key,
                            Waiting {
                                entry: e,
                                lines: taken,
                            },
                        );
                    }
                }
            }
        }
        let was = (at_path != path).then_some(at_path);
        for (origin, _) in left {
            if let Some(slot) = by.get_mut(origin) {
                *slot = By::Save {
                    id,
                    was: was.clone(),
                };
            }
        }
    }

    let runs = runs_of(by);
    Ok(Whose {
        path,
        at,
        text,
        runs,
        coarse,
    })
}

fn read_save(repo: &Repository, id: Id) -> Result<Save, RepoError> {
    Ok(repo.store().save(&id, repo.signatures_allowed())?)
}

fn is_file(kind: EntryKind) -> bool {
    matches!(kind, EntryKind::File | EntryKind::Executable)
}

fn is_text(bytes: &[u8]) -> bool {
    !changes::is_binary(bytes) && changes::line_count(bytes) <= changes::MAX_LINES
}

/// The version asked about, or why it has no lines to name.
fn asked_text(side: Side, path: &str) -> Result<Vec<u8>, WhoError> {
    let path = path.to_owned();
    match side {
        Side::TooBig => Err(WhoError::TooBig { path }),
        Side::Bytes(b) if changes::is_binary(&b) => Err(WhoError::NotText { path }),
        Side::Bytes(b) if changes::line_count(&b) > changes::MAX_LINES => {
            Err(WhoError::TooManyLines { path })
        }
        Side::Bytes(b) => Ok(b),
    }
}

/// Each line without one final `\n`, then without one final `\r`: a save that only changed
/// line endings wrote no line.
fn keys_of(text: &[u8]) -> Vec<&[u8]> {
    changes::lines_of(text)
        .into_iter()
        .map(|l| {
            let l = l.strip_suffix(b"\n").unwrap_or(l);
            l.strip_suffix(b"\r").unwrap_or(l)
        })
        .collect()
}

/// The lines of `left` (each as its line in the version asked about and in `keys`) that the
/// older version `older` has too, each with its line there; they leave `left`. A version that is
/// not a file, or past the limits, has none; one whose contents are `same` has them all.
fn shared(
    store: &Store,
    older: &Entry,
    same: Option<Id>,
    keys: &[&[u8]],
    left: &mut Vec<(usize, usize)>,
    coarse: &mut bool,
) -> Result<Vec<(usize, usize)>, SaveError> {
    if !is_file(older.kind) {
        return Ok(Vec::new());
    }
    if same == Some(older.id) {
        return Ok(std::mem::take(left));
    }
    let Side::Bytes(old) =
        changes::stored(store, Some(older.kind), Some(older.id), Some(older.size))?
    else {
        return Ok(Vec::new());
    };
    if !is_text(&old) {
        return Ok(Vec::new());
    }
    let (ops, late) = changes::line_ops(&keys_of(&old), keys, changes::deadline());
    *coarse |= late;
    let mut there = vec![None; keys.len()];
    for op in ops {
        if let DiffOp::Equal {
            old_index,
            new_index,
            len,
        } = op
        {
            for k in 0..len {
                if let Some(slot) = there.get_mut(new_index + k) {
                    *slot = Some(old_index + k);
                }
            }
        }
    }
    let mut taken = Vec::new();
    left.retain(|&(origin, here)| match there.get(here).copied().flatten() {
        Some(old) => {
            taken.push((origin, old));
            false
        }
        None => true,
    });
    Ok(taken)
}

/// The path `path` had at `parent`, the save's first parent, where the save's move notes say it
/// moved and the note is true of the two.
fn moved_from(
    stored: &Listings<'_>,
    save: &Save,
    parent: &Save,
    path: &str,
) -> Result<Option<String>, RepoError> {
    let Ok(i) = save
        .move_notes
        .binary_search_by(|n| n.to.as_str().cmp(path))
    else {
        return Ok(None);
    };
    let Some(note) = save.move_notes.get(i) else {
        return Ok(None);
    };
    if names::validate_path(&note.from).is_err() || names::validate_path(&note.to).is_err() {
        return Ok(None);
    }
    let mut before = Tree::new(stored, parent.root, kept::EXACT);
    let mut after = Tree::new(stored, save.root, kept::EXACT);
    Ok(moves::true_of(&mut before, &mut after, note)?.then(|| note.from.clone()))
}

fn runs_of(by: Vec<By>) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for (i, b) in by.into_iter().enumerate() {
        match runs.last_mut() {
            Some(run) if run.by == b => run.lines += 1,
            _ => runs.push(Run {
                first: i + 1,
                lines: 1,
                by: b,
            }),
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::collections::{BTreeMap, HashSet};
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::combine::{self, Combined, Keep, Named};
    use crate::id::ChangeId;
    use crate::lines::{self, Target};
    use crate::names::Given;
    use crate::pack::{ObjectKind, PackSet};
    use crate::repo::ChangeLock;
    use crate::resolve::{ResolveOutcome, ResolveRequest, resolve};
    use crate::save::{MoveNote, Person};
    use crate::snapshot::{SaveOutcome, SaveRequest, save_folder};
    use crate::switch::goto;
    use crate::test_support::{add_listing, add_tree, plant_root, plant_save};

    fn person(time_ms: i64) -> Person {
        Person {
            name: "Ada".into(),
            email: "ada@example.com".into(),
            time_ms,
            tz_offset_min: 0,
        }
    }

    /// A project made with the engine's own commands, each a second after the last.
    struct Fx {
        dir: tempfile::TempDir,
        repo: Repository,
        clock: i64,
    }

    impl Fx {
        fn new() -> Fx {
            let dir = tempfile::tempdir().unwrap();
            let repo = Repository::init(dir.path(), &person(1_700_000_000_000)).unwrap();
            Fx {
                dir,
                repo,
                clock: 1_700_000_000_000,
            }
        }

        fn root(&self) -> &Path {
            self.dir.path()
        }

        fn write(&self, rel: &str, bytes: &[u8]) {
            let at = self.root().join(rel);
            fs::create_dir_all(at.parent().unwrap()).unwrap();
            fs::write(at, bytes).unwrap();
        }

        fn now(&mut self) -> Person {
            self.clock += 1000;
            person(self.clock)
        }

        fn save(&mut self, message: &str) -> Id {
            let who = self.now();
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
            let who = self.now();
            lines::new_line(&mut self.repo, name, &who, &mut |_| {}).unwrap();
        }

        fn goto(&mut self, line: &str) {
            let who = self.now();
            let target = Target::Line(line.into());
            goto(&mut self.repo, &target, &who, "goto", None, &mut |_| {}).unwrap();
        }

        fn combine(&mut self, line: &str) -> Id {
            let named = Named {
                raw: line.into(),
                shown: line.into(),
            };
            let who = self.now();
            let target = Target::Line(line.into());
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

        fn resolve(&mut self, paths: &[&str]) -> Id {
            let given: Vec<Given> = paths.iter().map(|p| Given::Path((*p).into())).collect();
            let who = self.now();
            let req = ResolveRequest {
                paths: Some(given.as_slice()),
                keep: None,
                who: &who,
                command: "resolve",
                accept_crlf: false,
                git_excludes: None,
            };
            match resolve(&mut self.repo, &req, &mut |_| {}, &mut |_| true).unwrap() {
                ResolveOutcome::Recorded { save_id, .. } => save_id,
                other => panic!("{other:?}"),
            }
        }

        fn who(&self, path: &str, asked: Asked) -> Result<Whose, WhoError> {
            who(&self.repo, &WhoRequest { path, asked })
        }

        fn folder(&self, path: &str) -> Whose {
            self.who(path, Asked::Folder).unwrap()
        }

        fn at(&self, save: Id, path: &str) -> Whose {
            self.who(path, Asked::Save(save)).unwrap()
        }
    }

    /// Saves stored straight into one pack set, committed together with the line `main` at the
    /// last: for histories the commands would never make.
    struct Plant {
        lock: ChangeLock,
        packs: PackSet,
        base: Id,
        last: Id,
    }

    impl Plant {
        fn new(repo: &mut Repository) -> Plant {
            let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
            let packs = repo.new_packs().unwrap();
            let base = history::current_save(repo).unwrap();
            Plant {
                lock,
                packs,
                base,
                last: base,
            }
        }

        fn save(
            &mut self,
            message: &str,
            parents: &[Id],
            files: &[(&str, &[u8])],
            notes: &[(&str, &str)],
            time_ms: i64,
        ) -> Id {
            let entries: Vec<(&str, EntryKind, &[u8])> = files
                .iter()
                .map(|&(p, b)| (p, EntryKind::File, b))
                .collect();
            let (root, _) = add_tree(&mut self.packs, &entries, &[]);
            let mut move_notes: Vec<MoveNote> = notes
                .iter()
                .map(|&(from, to)| MoveNote {
                    from: from.into(),
                    to: to.into(),
                })
                .collect();
            move_notes.sort_by(|a, b| a.to.cmp(&b.to));
            let save = Save {
                root,
                parents: parents.to_vec(),
                author: person(time_ms),
                saver: person(time_ms),
                message: message.into(),
                change_id: ChangeId::random().unwrap(),
                move_notes,
                signature: vec![],
                extra: BTreeMap::new(),
            };
            self.last = self
                .packs
                .add_record(ObjectKind::Save, &save.encode())
                .unwrap();
            self.last
        }

        fn commit(self, repo: &mut Repository) {
            let (head, mut entry) = repo.current().unwrap();
            entry.parent = Some(head);
            entry.command = "save".into();
            entry.folder_change = None;
            entry.state.lines.insert("main".into(), self.last);
            repo.commit(&self.lock, self.packs, &entry).unwrap();
        }
    }

    fn project() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path(), &person(1_700_000_000_000)).unwrap();
        (dir, repo)
    }

    fn at(repo: &Repository, save: Id, path: &str) -> Whose {
        who(
            repo,
            &WhoRequest {
                path,
                asked: Asked::Save(save),
            },
        )
        .unwrap()
    }

    fn by(id: Id) -> By {
        By::Save { id, was: None }
    }

    fn by_was(id: Id, was: &str) -> By {
        By::Save {
            id,
            was: Some(was.into()),
        }
    }

    /// The runs `(lines, by)` stand for, numbered from 1.
    fn runs(parts: &[(usize, By)]) -> Vec<Run> {
        let mut first = 1;
        parts
            .iter()
            .map(|(lines, by)| {
                let run = Run {
                    first,
                    lines: *lines,
                    by: by.clone(),
                };
                first += lines;
                run
            })
            .collect()
    }

    /// Each line's `By`, from the runs.
    fn each_line(w: &Whose) -> Vec<By> {
        w.runs
            .iter()
            .flat_map(|r| std::iter::repeat_n(r.by.clone(), r.lines))
            .collect()
    }

    #[test]
    fn a_line_is_the_save_that_added_it() {
        let mut fx = Fx::new();
        fx.write("a.txt", b"one\ntwo\n");
        let first = fx.save("First");
        fx.write("a.txt", b"one\nnew\ntwo\n");
        let second = fx.save("Second");
        let w = fx.at(second, "a.txt");
        assert_eq!(
            w.runs,
            runs(&[(1, by(first)), (1, by(second)), (1, by(first))])
        );
        assert_eq!((w.path.as_str(), w.at), ("a.txt", second));
        assert_eq!(w.text, b"one\nnew\ntwo\n");
        assert!(!w.coarse);
        // From the folder, the save the project is at is asked first.
        assert_eq!(fx.folder("a.txt"), w);
    }

    #[test]
    fn a_line_taken_out_and_put_back_is_the_save_that_put_it_back() {
        let mut fx = Fx::new();
        fx.write("a.txt", b"a\nb\nc\n");
        let first = fx.save("First");
        fx.write("a.txt", b"a\nc\n");
        fx.save("Out");
        fx.write("a.txt", b"a\nb\nc\n");
        let back = fx.save("Back");
        assert_eq!(
            fx.at(back, "a.txt").runs,
            runs(&[(1, by(first)), (1, by(back)), (1, by(first))])
        );
    }

    #[test]
    fn a_file_without_a_last_line_ending() {
        let mut fx = Fx::new();
        fx.write("a.txt", b"a\nb");
        let first = fx.save("First");
        fx.write("a.txt", b"a\nb\nc");
        let second = fx.save("Second");
        let w = fx.at(second, "a.txt");
        assert_eq!(w.runs, runs(&[(2, by(first)), (1, by(second))]));
        assert_eq!(w.text, b"a\nb\nc");
    }

    #[test]
    fn a_save_that_only_changed_line_endings_takes_no_line() {
        let cases: [(&[u8], &[u8]); 4] = [
            (b"a\nb\nc\n", b"a\r\nb\r\nc\r\n"),
            (b"a\r\nb\r\nc\r\n", b"a\nb\nc\n"),
            (b"a\nb\nc\n", b"a\nb\nc"),
            (b"a\r\nb\nc", b"a\nb\r\nc\n"),
        ];
        for (before, after) in cases {
            let (_dir, mut repo) = project();
            let mut plant = Plant::new(&mut repo);
            let base = plant.base;
            let first = plant.save("First", &[base], &[("a.txt", before)], &[], 1);
            let second = plant.save("Endings", &[first], &[("a.txt", after)], &[], 2);
            plant.commit(&mut repo);
            assert_eq!(at(&repo, second, "a.txt").runs, runs(&[(3, by(first))]));
        }
        // Everything else counts, spaces too.
        let (_dir, mut repo) = project();
        let mut plant = Plant::new(&mut repo);
        let base = plant.base;
        let first = plant.save("First", &[base], &[("a.txt", b"a\nb\n")], &[], 1);
        let second = plant.save("Space", &[first], &[("a.txt", b"a \nb\n")], &[], 2);
        plant.commit(&mut repo);
        assert_eq!(
            at(&repo, second, "a.txt").runs,
            runs(&[(1, by(second)), (1, by(first))])
        );
    }

    /// `main` and `side` each changed a line of five and added the same line; the project is on
    /// `main`, at the save combining `side` into it.
    #[test]
    fn through_a_combine_each_line_goes_to_the_line_it_came_from() {
        let mut fx = Fx::new();
        fx.write("a.txt", b"1\n2\n3\n4\n5\n");
        let start = fx.save("Start");
        fx.line_new("side");
        fx.write("a.txt", b"1 side\n2\n3\nboth\n4\n5\n");
        let side = fx.save("Side");
        fx.goto("main");
        fx.write("a.txt", b"1\n2\n3\nboth\n4\n5 main\n");
        let main = fx.save("Main");
        let combined = fx.combine("side");
        let w = fx.at(combined, "a.txt");
        assert_eq!(w.text, b"1 side\n2\n3\nboth\n4\n5 main\n");
        assert_eq!(
            w.runs,
            runs(&[
                (1, by(side)),
                (2, by(start)),
                (1, by(main)),
                (1, by(start)),
                (1, by(main)),
            ])
        );
    }

    /// `main` and `side` changed `a.txt`'s second line each their own way; the project is on
    /// `main`, at the save combining them, which keeps the conflict.
    fn conflicted() -> (Fx, [Id; 4]) {
        let mut fx = Fx::new();
        fx.write("a.txt", b"one\ntwo\nthree\n");
        fx.write("notes.txt", b"notes\n");
        let start = fx.save("Start");
        fx.line_new("side");
        fx.write("a.txt", b"one\ntwo side\nthree\n");
        let side = fx.save("Side");
        fx.goto("main");
        fx.write("a.txt", b"one\ntwo main\nthree\n");
        let main = fx.save("Main");
        let combined = fx.combine("side");
        (fx, [start, main, side, combined])
    }

    #[test]
    fn what_a_combine_wrote_is_the_combines() {
        let (mut fx, [start, _, _, _]) = conflicted();
        fx.write("a.txt", b"one\ntwo fixed\nthree\n");
        let fixed = fx.resolve(&["a.txt"]);
        assert_eq!(
            fx.repo.store().save(&fixed, false).unwrap().parents.len(),
            2
        );
        assert_eq!(
            fx.at(fixed, "a.txt").runs,
            runs(&[(1, by(start)), (1, by(fixed)), (1, by(start))])
        );
    }

    #[test]
    fn a_fix_saved_after_a_kept_conflict_goes_through_it_to_both_lines() {
        let (mut fx, [start, main, side, combined]) = conflicted();
        fx.write("notes.txt", b"more notes\n");
        let more = fx.save("More notes");
        fx.write("a.txt", b"one\ntwo main\ntwo side\nthree\n");
        let fixed = fx.resolve(&["a.txt"]);
        let save = fx.repo.store().save(&fixed, false).unwrap();
        assert_eq!(save.parents, [more]);
        assert_ne!(more, combined);
        let w = fx.at(fixed, "a.txt");
        assert_eq!(
            w.runs,
            runs(&[(1, by(start)), (1, by(main)), (1, by(side)), (1, by(start))])
        );
        assert_eq!(fx.folder("a.txt").runs, w.runs);
    }

    #[test]
    fn in_the_folder_a_marked_text_names_each_side_and_the_combine() {
        let (fx, [start, main, side, combined]) = conflicted();
        let mut text = fs::read(fx.root().join("a.txt")).unwrap();
        assert!(text.starts_with(b"one\n<<<<<<< "), "{text:?}");
        text.extend_from_slice(b"typed since\n");
        fx.write("a.txt", &text);
        let mut expected: Vec<(usize, By)> = Vec::new();
        for line in changes::lines_of(&text) {
            let b = match line {
                b"one\n" | b"three\n" => by(start),
                b"two main\n" => by(main),
                b"two side\n" => by(side),
                b"typed since\n" => By::Unsaved,
                // The markers, and the lines only the middle part has.
                _ => by(combined),
            };
            match expected.last_mut() {
                Some((n, last)) if *last == b => *n += 1,
                _ => expected.push((1, b)),
            }
        }
        let w = fx.folder("a.txt");
        assert_eq!(w.at, combined);
        assert_eq!(w.text, text);
        assert_eq!(w.runs, runs(&expected));
        assert!(w.runs.iter().any(|r| r.by == by(combined)));
    }

    #[test]
    fn a_file_moved_by_a_save_of_its_own_is_followed() {
        let mut fx = Fx::new();
        fx.write("a.txt", b"a\nb\n");
        let first = fx.save("First");
        fx.write("a.txt", b"a\nb\nc\n");
        let second = fx.save("Second");
        fs::rename(fx.root().join("a.txt"), fx.root().join("b.txt")).unwrap();
        let moved = fx.save("Moved");
        assert_eq!(
            fx.repo.store().save(&moved, false).unwrap().move_notes,
            [MoveNote {
                from: "a.txt".into(),
                to: "b.txt".into()
            }]
        );
        fx.write("b.txt", b"a\nb\nc\nd\n");
        let fourth = fx.save("Fourth");
        assert_eq!(
            fx.at(fourth, "b.txt").runs,
            runs(&[
                (2, by_was(first, "a.txt")),
                (1, by_was(second, "a.txt")),
                (1, by(fourth)),
            ])
        );
    }

    #[test]
    fn a_move_saved_with_an_edit_is_not_followed() {
        let mut fx = Fx::new();
        fx.write("a.txt", b"a\nb\n");
        fx.save("First");
        fs::remove_file(fx.root().join("a.txt")).unwrap();
        fx.write("b.txt", b"a\nb\nc\n");
        let moved = fx.save("Moved and changed");
        assert!(
            fx.repo
                .store()
                .save(&moved, false)
                .unwrap()
                .move_notes
                .is_empty()
        );
        assert_eq!(fx.at(moved, "b.txt").runs, runs(&[(3, by(moved))]));
    }

    #[test]
    fn a_move_note_that_is_not_true_is_not_followed() {
        let (_dir, mut repo) = project();
        let mut plant = Plant::new(&mut repo);
        let base = plant.base;
        let first = plant.save("First", &[base], &[("a.txt", b"a\nb\n")], &[], 1);
        // Its file changed on the way, so the note can't be true.
        let changed = plant.save(
            "Changed",
            &[first],
            &[("b.txt", b"a\nb\nc\n")],
            &[("a.txt", "b.txt")],
            2,
        );
        // The file it names is still there.
        let kept = plant.save(
            "Kept",
            &[first],
            &[("a.txt", b"a\nb\n"), ("c.txt", b"a\nb\n")],
            &[("a.txt", "c.txt")],
            3,
        );
        // A note naming no path at all.
        let odd = plant.save(
            "Odd",
            &[first],
            &[("d.txt", b"a\nb\n")],
            &[("a.txt/../a.txt", "d.txt")],
            4,
        );
        plant.commit(&mut repo);
        assert_eq!(at(&repo, changed, "b.txt").runs, runs(&[(3, by(changed))]));
        assert_eq!(at(&repo, kept, "c.txt").runs, runs(&[(2, by(kept))]));
        assert_eq!(at(&repo, odd, "d.txt").runs, runs(&[(2, by(odd))]));
    }

    /// The known limit: nothing a save stores says where a second parent had the file.
    #[test]
    fn a_file_moved_on_the_other_line_keeps_the_combine_for_what_was_changed_under_the_old_name() {
        let mut fx = Fx::new();
        fx.write("a.txt", b"1\n2\n3\n");
        let start = fx.save("Start");
        fx.line_new("side");
        fx.write("a.txt", b"1\n2 side\n3\n");
        fx.save("Side");
        fx.goto("main");
        fs::rename(fx.root().join("a.txt"), fx.root().join("b.txt")).unwrap();
        fx.save("Moved");
        let combined = fx.combine("side");
        let w = fx.at(combined, "b.txt");
        assert_eq!(w.text, b"1\n2 side\n3\n");
        assert_eq!(
            w.runs,
            runs(&[
                (1, by_was(start, "a.txt")),
                (1, by(combined)),
                (1, by_was(start, "a.txt")),
            ])
        );
    }

    #[test]
    fn the_folders_new_lines_are_unsaved() {
        let mut fx = Fx::new();
        fx.write("a.txt", b"a\nb\n");
        let first = fx.save("First");
        fx.write("a.txt", b"a\nnew\nb\nnewer");
        let w = fx.folder("a.txt");
        assert_eq!(
            w.runs,
            runs(&[
                (1, by(first)),
                (1, By::Unsaved),
                (1, by(first)),
                (1, By::Unsaved),
            ])
        );
        assert_eq!(w.at, first);
    }

    #[test]
    fn a_file_no_save_has_is_all_unsaved() {
        let mut fx = Fx::new();
        fx.write("a.txt", b"a\n");
        let first = fx.save("First");
        fx.write("new.txt", b"a\nb\n");
        let w = fx.folder("new.txt");
        assert_eq!(w.runs, runs(&[(2, By::Unsaved)]));
        assert_eq!((w.path.as_str(), w.at), ("new.txt", first));
        // An empty file has no runs.
        fx.write("empty.txt", b"");
        assert_eq!(fx.folder("empty.txt").runs, []);
    }

    #[test]
    fn a_path_typed_in_another_case_finds_the_saved_file() {
        let mut fx = Fx::new();
        if !fx.repo.drive().unwrap().ignores_case {
            return;
        }
        fx.write("Dir/a.txt", b"a\nb\n");
        let first = fx.save("First");
        let w = fx.folder("dir/A.TXT");
        assert_eq!(w.path, "Dir/a.txt");
        assert_eq!(w.runs, runs(&[(2, by(first))]));
        assert_eq!(fx.at(first, "DIR/a.txt"), w);
    }

    #[test]
    fn what_isnt_a_file_there_is_refused() {
        let mut fx = Fx::new();
        fx.write("d/e.txt", b"e\n");
        let first = fx.save("First");
        let no_file = |w: Result<Whose, WhoError>| match w {
            Err(WhoError::NoFile { path, save }) => (path, save),
            other => panic!("{other:?}"),
        };
        assert_eq!(
            no_file(fx.who("none.txt", Asked::Folder)),
            ("none.txt".into(), None)
        );
        assert_eq!(
            no_file(fx.who("none.txt", Asked::Save(first))),
            ("none.txt".into(), Some(first))
        );
        assert_eq!(no_file(fx.who("d", Asked::Folder)), ("d".into(), None));
        assert_eq!(
            no_file(fx.who("d/e.txt/f", Asked::Save(first))),
            ("d/e.txt/f".into(), Some(first))
        );
        let e = fx.who("d", Asked::Save(first)).unwrap_err();
        assert!(
            matches!(&e, WhoError::NotAFile { path, kind: EntryKind::Folder } if path == "d"),
            "{e:?}"
        );
        assert_eq!(
            e.to_string(),
            "d is a folder, and opalro who reads one file."
        );
        for path in ["", "../a.txt", "d//e.txt", ".opalro/head"] {
            let e = fx.who(path, Asked::Folder).unwrap_err();
            assert!(matches!(e, WhoError::NotAPath { .. }), "{path}: {e:?}");
        }
    }

    #[test]
    fn links_are_refused() {
        let (_dir, mut repo) = project();
        plant_save(
            &mut repo,
            &[
                ("l", EntryKind::Symlink, b"a.txt"),
                ("p", EntryKind::ProjectLink, &[0x11u8; 20]),
                ("a.txt", EntryKind::File, b"a\n"),
            ],
        );
        let tip = history::current_save(&repo).unwrap();
        for (path, kind) in [("l", EntryKind::Symlink), ("p", EntryKind::ProjectLink)] {
            let e = who(
                &repo,
                &WhoRequest {
                    path,
                    asked: Asked::Save(tip),
                },
            )
            .unwrap_err();
            assert!(
                matches!(&e, WhoError::NotAFile { path: p, kind: k } if p == path && *k == kind),
                "{e:?}"
            );
        }
    }

    #[test]
    fn what_has_no_lines_to_compare_is_refused() {
        let mut fx = Fx::new();
        fx.write("pic.bin", b"a\n\0b\n");
        fx.write("many.txt", &vec![b'\n'; changes::MAX_LINES + 1]);
        let first = fx.save("First");
        let big = fs::File::create(fx.root().join("big.txt")).unwrap();
        big.set_len(changes::MAX_LINES_SIZE + 1).unwrap();
        drop(big);
        let e = fx.who("pic.bin", Asked::Folder).unwrap_err();
        assert!(
            matches!(&e, WhoError::NotText { path } if path == "pic.bin"),
            "{e:?}"
        );
        let e = fx.who("pic.bin", Asked::Save(first)).unwrap_err();
        assert!(
            matches!(&e, WhoError::NotText { path } if path == "pic.bin"),
            "{e:?}"
        );
        let e = fx.who("many.txt", Asked::Folder).unwrap_err();
        assert!(
            matches!(&e, WhoError::TooManyLines { path } if path == "many.txt"),
            "{e:?}"
        );
        let e = fx.who("many.txt", Asked::Save(first)).unwrap_err();
        assert!(matches!(&e, WhoError::TooManyLines { .. }), "{e:?}");
        let e = fx.who("big.txt", Asked::Folder).unwrap_err();
        assert!(
            matches!(&e, WhoError::TooBig { path } if path == "big.txt"),
            "{e:?}"
        );
        // A save whose listing says the file is too big is believed without reading it.
        let mut planted = Id([0; 32]);
        plant_root(&mut fx.repo, |packs| {
            let id = packs.add_content(b"a\n").unwrap();
            let entry = Entry {
                name: "big.txt".into(),
                kind: EntryKind::File,
                id,
                size: changes::MAX_LINES_SIZE + 1,
            };
            planted = add_listing(packs, vec![entry]).0;
            planted
        });
        let tip = history::current_save(&fx.repo).unwrap();
        let e = fx.who("big.txt", Asked::Save(tip)).unwrap_err();
        assert!(matches!(&e, WhoError::TooBig { .. }), "{e:?}");
    }

    #[test]
    fn an_older_version_that_is_not_text_ends_the_trail() {
        let (_dir, mut repo) = project();
        let mut plant = Plant::new(&mut repo);
        let base = plant.base;
        let text = plant.save("Text", &[base], &[("a.txt", b"a\nb\n")], &[], 1);
        let binary = plant.save("Binary", &[text], &[("a.txt", b"a\n\0\nb\n")], &[], 2);
        let again = plant.save("Again", &[binary], &[("a.txt", b"a\nb\nc\n")], &[], 3);
        plant.commit(&mut repo);
        assert_eq!(at(&repo, again, "a.txt").runs, runs(&[(3, by(again))]));
    }

    #[test]
    fn a_save_with_many_parents() {
        let (_dir, mut repo) = project();
        let mut plant = Plant::new(&mut repo);
        let base = plant.base;
        let start = plant.save("Start", &[base], &[("a.txt", b"start\n")], &[], 1);
        let mut parents = Vec::new();
        let mut all = b"start\n".to_vec();
        for i in 0..64 {
            let line = format!("line {i}\n");
            let text = [b"start\n", line.as_bytes()].concat();
            let message = format!("Adds {i}");
            parents.push(plant.save(&message, &[start], &[("a.txt", &text)], &[], 2 + i));
            all.extend_from_slice(line.as_bytes());
        }
        let combined = plant.save("Combined", &parents, &[("a.txt", &all)], &[], 100);
        plant.commit(&mut repo);
        let mut expected = vec![(1, by(start))];
        expected.extend(parents.iter().map(|p| (1, by(*p))));
        assert_eq!(at(&repo, combined, "a.txt").runs, runs(&expected));
    }

    /// One history, planted twice: once with honest dates, once with dates that run backwards.
    #[test]
    fn dates_that_lie_change_nothing() {
        let named = |honest: bool| {
            let (_dir, mut repo) = project();
            let mut plant = Plant::new(&mut repo);
            let base = plant.base;
            let t = |n: i64| if honest { n } else { 100 - n };
            let b = plant.save("B", &[base], &[("a.txt", b"1\n2\n3\n4\n")], &[], t(1));
            let a1 = plant.save("A1", &[b], &[("a.txt", b"1 a\n2\n4\n")], &[], t(2));
            let c1 = plant.save("C1", &[b], &[("a.txt", b"1\n2\n3\n4 c\n")], &[], t(3));
            let a2 = plant.save("A2", &[a1], &[("a.txt", b"1 a\n2\n4\n5 a\n")], &[], t(4));
            let m = plant.save(
                "M",
                &[a2, c1],
                &[("a.txt", b"1 a\n2\n3\n4 c\n5 a\n")],
                &[],
                t(5),
            );
            plant.commit(&mut repo);
            at(&repo, m, "a.txt")
                .runs
                .into_iter()
                .map(|r| match r.by {
                    By::Save { id, .. } => {
                        (r.lines, repo.store().save(&id, false).unwrap().message)
                    }
                    By::Unsaved => panic!("{r:?}"),
                })
                .collect::<Vec<_>>()
        };
        let honest = named(true);
        assert_eq!(
            honest,
            [
                (1, "A1".to_owned()),
                (2, "B".to_owned()),
                (1, "C1".to_owned()),
                (1, "A2".to_owned()),
            ]
        );
        assert_eq!(named(false), honest);
    }

    #[test]
    fn a_save_that_cant_be_read_is_named() {
        let (_dir, mut repo) = project();
        let missing = Id([0xab; 32]);
        let mut plant = Plant::new(&mut repo);
        let orphan = plant.save("Orphan", &[missing], &[("a.txt", b"a\n")], &[], 1);
        plant.commit(&mut repo);
        let e = who(
            &repo,
            &WhoRequest {
                path: "a.txt",
                asked: Asked::Save(orphan),
            },
        )
        .unwrap_err();
        assert!(matches!(e, WhoError::Repo(_)), "{e:?}");
        assert!(e.to_string().contains(&missing.to_hex()), "{e}");
    }

    /// Every file under the project folder, `.opalro` included, with its bytes.
    fn everything(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut out = BTreeMap::new();
        let mut todo = vec![root.to_path_buf()];
        while let Some(dir) = todo.pop() {
            for e in fs::read_dir(&dir).unwrap() {
                let e = e.unwrap();
                if e.file_type().unwrap().is_dir() {
                    out.insert(e.path(), Vec::new());
                    todo.push(e.path());
                } else {
                    out.insert(e.path(), fs::read(e.path()).unwrap());
                }
            }
        }
        out
    }

    #[test]
    fn who_writes_nothing() {
        let (mut fx, [_, _, _, combined]) = conflicted();
        fx.repo.set_cache_key([7; 32]);
        fx.write("a.txt", b"one\nnew\n");
        let before = everything(fx.root());
        fx.folder("a.txt");
        fx.at(combined, "a.txt");
        fx.at(combined, "notes.txt");
        fx.who("none.txt", Asked::Folder).unwrap_err();
        assert_eq!(everything(fx.root()), before);
    }

    #[cfg(feature = "fault-injection")]
    #[test]
    fn a_comparison_past_its_deadline_is_said() {
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["who::tests::past_the_deadline", "--exact", "--ignored"])
            .env("OPALRO_DIFF_DEADLINE_MS", "0")
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{said}");
        assert!(said.contains("1 passed"), "{said}");
    }

    #[cfg(feature = "fault-injection")]
    #[test]
    #[ignore = "a_comparison_past_its_deadline_is_said runs it with a deadline of 0"]
    fn past_the_deadline() {
        assert_eq!(std::env::var("OPALRO_DIFF_DEADLINE_MS").as_deref(), Ok("0"));
        let mut fx = Fx::new();
        fx.write("a.txt", b"a\nb\n");
        fx.save("First");
        fx.write("a.txt", b"a\nnew\nb\n");
        let second = fx.save("Second");
        let w = fx.at(second, "a.txt");
        assert!(w.coarse);
        assert_eq!(w.runs.iter().map(|r| r.lines).sum::<usize>(), 3);
    }

    // The property test: random histories, and what must hold of every answer.

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }

        fn one_in(&mut self, n: usize) -> bool {
            self.below(n) == 0
        }
    }

    #[derive(Clone, PartialEq)]
    struct Text {
        lines: Vec<String>,
        crlf: bool,
        open_end: bool,
    }

    impl Text {
        fn bytes(&self) -> Vec<u8> {
            let end = if self.crlf { "\r\n" } else { "\n" };
            let mut out = self.lines.join(end);
            if !self.open_end && !self.lines.is_empty() {
                out.push_str(end);
            }
            out.into_bytes()
        }
    }

    struct Node {
        parents: Vec<Id>,
        files: BTreeMap<String, Text>,
        moved: Option<(String, String)>,
    }

    const PATHS: [&str; 5] = ["a.txt", "b.txt", "c.txt", "d/e.txt", "d/f.txt"];

    /// Both texts' lines in one, each keeping its order, the lines they share once.
    fn union(a: &[String], b: &[String]) -> Vec<String> {
        let (n, m) = (a.len(), b.len());
        let mut longest = vec![vec![0usize; m + 1]; n + 1];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                longest[i][j] = if a[i] == b[j] {
                    longest[i + 1][j + 1] + 1
                } else {
                    longest[i + 1][j].max(longest[i][j + 1])
                };
            }
        }
        let (mut i, mut j, mut out) = (0, 0, Vec::new());
        while i < n && j < m {
            if a[i] == b[j] {
                out.push(a[i].clone());
                i += 1;
                j += 1;
            } else if longest[i + 1][j] >= longest[i][j + 1] {
                out.push(a[i].clone());
                i += 1;
            } else {
                out.push(b[j].clone());
                j += 1;
            }
        }
        out.extend(a[i..].iter().cloned());
        out.extend(b[j..].iter().cloned());
        out
    }

    struct History {
        _dir: tempfile::TempDir,
        repo: Repository,
        nodes: HashMap<Id, Node>,
    }

    /// A history of saves that add, change and take out lines, move files, change only line
    /// endings, and combine two or three lines; only `main` takes combines, so the lines' order
    /// is one order everywhere. With `unique`, no line is ever written twice.
    fn history(rng: &mut Rng, unique: bool) -> History {
        let (dir, mut repo) = project();
        let mut plant = Plant::new(&mut repo);
        let mut nodes = HashMap::new();
        nodes.insert(
            plant.base,
            Node {
                parents: vec![],
                files: BTreeMap::new(),
                moved: None,
            },
        );
        let mut heads = vec![plant.base];
        let mut written = 0;
        let mut line = |rng: &mut Rng| {
            written += 1;
            if unique {
                format!("line {written}")
            } else {
                ["a", "b", "c", "d"][rng.below(4)].to_owned()
            }
        };
        for step in 0..28 {
            let h = rng.below(heads.len());
            let mut files = nodes[&heads[h]].files.clone();
            let mut parents = vec![heads[h]];
            let mut moved = None;
            match rng.below(10) {
                0..=4 => {
                    let free: Vec<&str> = PATHS
                        .iter()
                        .copied()
                        .filter(|p| !files.contains_key(*p))
                        .collect();
                    if files.is_empty() || (files.len() < 3 && rng.one_in(3)) {
                        let lines = (0..1 + rng.below(3)).map(|_| line(rng)).collect();
                        let text = Text {
                            lines,
                            crlf: false,
                            open_end: false,
                        };
                        files.insert(free[rng.below(free.len())].to_owned(), text);
                    } else {
                        let path = files.keys().nth(rng.below(files.len())).unwrap().clone();
                        let text = files.get_mut(&path).unwrap();
                        for _ in 0..1 + rng.below(3) {
                            let at = rng.below(text.lines.len());
                            match rng.below(3) {
                                0 => text
                                    .lines
                                    .insert(rng.below(text.lines.len() + 1), line(rng)),
                                1 if text.lines.len() > 1 => {
                                    text.lines.remove(at);
                                }
                                _ => text.lines[at] = line(rng),
                            }
                        }
                    }
                }
                5 | 6 => {
                    let free: Vec<&str> = PATHS
                        .iter()
                        .copied()
                        .filter(|p| !files.contains_key(*p))
                        .collect();
                    if files.is_empty() || free.is_empty() {
                        continue;
                    }
                    let from = files.keys().nth(rng.below(files.len())).unwrap().clone();
                    let to = free[rng.below(free.len())].to_owned();
                    let text = files.remove(&from).unwrap();
                    files.insert(to.clone(), text);
                    moved = Some((from, to));
                }
                7 => {
                    let Some(path) = files.keys().nth(rng.below(files.len().max(1))).cloned()
                    else {
                        continue;
                    };
                    let text = files.get_mut(&path).unwrap();
                    if rng.one_in(2) {
                        text.crlf = !text.crlf;
                    } else {
                        text.open_end = !text.open_end;
                    }
                }
                8 if heads.len() < 3 => {
                    heads.push(heads[h]);
                    continue;
                }
                _ => {
                    let mut others: Vec<Id> = heads[1..]
                        .iter()
                        .copied()
                        .filter(|o| *o != heads[0])
                        .collect();
                    others.dedup();
                    if others.is_empty() {
                        continue;
                    }
                    others.truncate(1 + rng.below(2));
                    parents = vec![heads[0]];
                    parents.extend(&others);
                    files = nodes[&heads[0]].files.clone();
                    for other in &others {
                        for (path, theirs) in &nodes[other].files {
                            match files.get_mut(path) {
                                None => {
                                    files.insert(path.clone(), theirs.clone());
                                }
                                Some(ours) if !rng.one_in(4) => {
                                    ours.lines = union(&ours.lines, &theirs.lines);
                                }
                                Some(_) => {}
                            }
                        }
                    }
                    // What the combine itself wrote.
                    if let Some(path) = files.keys().nth(rng.below(files.len().max(1))).cloned() {
                        let text = files.get_mut(&path).unwrap();
                        if rng.one_in(2) {
                            let at = rng.below(text.lines.len() + 1);
                            text.lines.insert(at, line(rng));
                        } else if text.lines.len() > 1 {
                            text.lines.remove(rng.below(text.lines.len()));
                        }
                    }
                }
            }
            let bytes: Vec<(String, Vec<u8>)> =
                files.iter().map(|(p, t)| (p.clone(), t.bytes())).collect();
            let refs: Vec<(&str, &[u8])> = bytes
                .iter()
                .map(|(p, b)| (p.as_str(), b.as_slice()))
                .collect();
            let notes: Vec<(&str, &str)> = moved
                .iter()
                .map(|(f, t): &(String, String)| (f.as_str(), t.as_str()))
                .collect();
            let id = plant.save(&format!("step {step}"), &parents, &refs, &notes, step);
            let into = if parents.len() > 1 { 0 } else { h };
            heads[into] = id;
            nodes.insert(
                id,
                Node {
                    parents,
                    files,
                    moved,
                },
            );
        }
        plant.commit(&mut repo);
        History {
            _dir: dir,
            repo,
            nodes,
        }
    }

    #[derive(Debug, Default)]
    struct Seen {
        answers: usize,
        second_parent: usize,
        moves: usize,
        first_though_second_had_it: usize,
    }

    fn reach(nodes: &HashMap<Id, Node>, from: Id, first_only: bool) -> HashSet<Id> {
        let mut seen = HashSet::new();
        let mut todo = vec![from];
        while let Some(id) = todo.pop() {
            if seen.insert(id) {
                let parents = &nodes[&id].parents;
                let n = if first_only { 1 } else { parents.len() };
                todo.extend(parents.iter().take(n));
            }
        }
        seen
    }

    fn check(h: &History, unique: bool, seen: &mut Seen) {
        for (&asked, node) in &h.nodes {
            let firsts = reach(&h.nodes, asked, true);
            for (path, text) in &node.files {
                let w = at(&h.repo, asked, path);
                seen.answers += 1;
                assert_eq!(w.text, text.bytes());
                assert_eq!(&w.path, path);
                assert!(!w.coarse);
                let mut next = 1;
                for run in &w.runs {
                    assert_eq!(run.first, next, "{:?}", w.runs);
                    assert!(run.lines > 0);
                    next += run.lines;
                }
                let lines = each_line(&w);
                assert_eq!(lines.len(), text.lines.len());
                for (line, b) in text.lines.iter().zip(lines) {
                    let By::Save { id, was } = b else {
                        panic!("{line} unsaved at a save");
                    };
                    assert_ne!(was.as_ref(), Some(path));
                    let there = was.clone().unwrap_or_else(|| path.clone());
                    let from = &h.nodes[&id];
                    assert!(from.files[&there].lines.contains(line), "{line} at {there}");
                    if unique {
                        for (i, parent) in from.parents.iter().enumerate() {
                            let mut back = &there;
                            if let Some((f, t)) = &from.moved
                                && i == 0
                                && *t == there
                            {
                                back = f;
                            }
                            if let Some(older) = h.nodes[parent].files.get(back) {
                                assert!(!older.lines.contains(line), "{line}: a parent has it");
                            }
                        }
                    }
                    if !firsts.contains(&id) {
                        seen.second_parent += 1;
                    }
                    if was.is_some() {
                        seen.moves += 1;
                    }
                    if let [first, second, ..] = node.parents[..] {
                        let has = |p: Id| {
                            h.nodes[&p]
                                .files
                                .get(path)
                                .is_some_and(|t| t.lines.contains(line))
                        };
                        if has(first) && has(second) {
                            seen.first_though_second_had_it += 1;
                            if unique {
                                assert!(reach(&h.nodes, first, false).contains(&id));
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn every_line_has_one_save_that_holds_it() {
        for unique in [false, true] {
            let mut rng = Rng(0x5eed_0f0a_1b2c_3d4e);
            let mut seen = Seen::default();
            for _ in 0..12 {
                let h = history(&mut rng, unique);
                check(&h, unique, &mut seen);
            }
            eprintln!("unique lines {unique}: {seen:?}");
            assert!(seen.second_parent >= 3, "{seen:?}");
            assert!(seen.moves >= 3, "{seen:?}");
            assert!(seen.first_though_second_had_it >= 3, "{seen:?}");
        }
    }
}
