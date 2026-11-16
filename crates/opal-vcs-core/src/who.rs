//! `opalro who`: the save behind each line of a file. It reads and never writes.

use std::borrow::Cow;
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::rc::Rc;

use similar::DiffOp;

use crate::changes::{self, NotLines, Side};
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
    /// The save the line is from, and the path the file had there when it was another. The path
    /// is shared by every line that had it, so a long one is held once.
    Save { id: Id, was: Option<Rc<str>> },
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
    /// Whether the save asked about, or the one the project is at, holds a file at the path.
    pub in_save: bool,
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

/// One version of the file waiting for its turn: where it is, and the lines that reached it, each
/// as its line in the version asked about and its line here. Newest save first, then by ID.
struct Turn {
    time_ms: i64,
    id: Id,
    path: Rc<str>,
    entry: Entry,
    lines: Vec<(usize, usize)>,
}

impl Turn {
    fn key(&self) -> (i64, Id, &str) {
        (self.time_ms, self.id, &self.path)
    }
}

impl PartialEq for Turn {
    fn eq(&self, other: &Turn) -> bool {
        self.key() == other.key()
    }
}

impl Eq for Turn {}

impl PartialOrd for Turn {
    fn partial_cmp(&self, other: &Turn) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Turn {
    fn cmp(&self, other: &Turn) -> std::cmp::Ordering {
        self.key().cmp(&other.key())
    }
}

/// What one comparison found: the stretches `(here, there, len)` of the newer version's lines
/// that the older one has too, in order.
type Stretches = Rc<Vec<(usize, usize, usize)>>;

/// The most comparisons one command remembers, and the most stretches they may hold in all.
const REMEMBERED: usize = 256;
const REMEMBERED_STRETCHES: usize = 1 << 20;

/// Comparisons already made, by the two versions' content IDs, so lines that reach a version
/// after its turn cost a lookup; the oldest is dropped first. No text is kept.
#[derive(Default)]
struct Remembered {
    found: HashMap<(Id, Id), Stretches>,
    order: VecDeque<(Id, Id)>,
    stretches: usize,
}

impl Remembered {
    fn keep(&mut self, key: (Id, Id), found: Stretches) {
        self.stretches = self.stretches.saturating_add(found.len());
        if let Some(old) = self.found.insert(key, found) {
            self.stretches = self.stretches.saturating_sub(old.len());
        } else {
            self.order.push_back(key);
        }
        while self.order.len() > REMEMBERED || self.stretches > REMEMBERED_STRETCHES {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(gone) = self.found.remove(&oldest) {
                self.stretches = self.stretches.saturating_sub(gone.len());
            }
        }
    }
}

#[cfg(test)]
thread_local! {
    /// How many comparisons this thread's `who` made.
    static COMPARED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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
    let mut tree = Tree::new(&standing, root, kept::EXACT);
    // The drive is asked (it probes with files under .opalro/tmp) only for another spelling.
    let found = match tree.entry(typed)? {
        Some(e) => Some((typed.to_owned(), e)),
        None => tree.entry_as_typed(typed, repo.drive()?)?,
    };

    let mut coarse = false;
    let mut heap = BinaryHeap::new();
    let (path, text, in_save) = match req.asked {
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
            let lines = (0..changes::line_count(&text)).map(|i| (i, i)).collect();
            heap.push(Turn {
                time_ms: 0,
                id: at,
                path: path.as_str().into(),
                entry,
                lines,
            });
            (path, text, true)
        }
        Asked::Folder => {
            let mut disk = Folders::open(repo.root()).map_err(|source| SaveError::Unreadable {
                path: "the project folder".to_owned(),
                source,
            })?;
            let side = match changes::in_folder(&mut disk, typed)? {
                Some(side) => side,
                None => {
                    return Err(match found {
                        Some((path, e)) if is_not_a_file(e.kind) => {
                            WhoError::NotAFile { path, kind: e.kind }
                        }
                        other => WhoError::NoFile {
                            path: other.map_or_else(|| typed.to_owned(), |(p, _)| p),
                            save: None,
                        },
                    });
                }
            };
            let saved = found.filter(|(_, e)| is_file(e.kind));
            let path = saved.as_ref().map_or(typed, |(p, _)| p.as_str()).to_owned();
            let text = asked_text(side, &path)?;
            let in_save = saved.is_some();
            if let Some((_, entry)) = saved {
                let mut left = (0..changes::line_count(&text)).map(|i| (i, i)).collect();
                let found = compare(store, Some(&text), &entry, &mut coarse)?;
                let lines = take(&found, &mut left);
                if !lines.is_empty() {
                    heap.push(Turn {
                        time_ms: 0,
                        id: at,
                        path: path.as_str().into(),
                        entry,
                        lines,
                    });
                }
            }
            (path, text, in_save)
        }
    };
    let asked = match req.asked {
        Asked::Save(_) => heap.peek().map(|t| (t.entry.id, text.as_slice())),
        Asked::Folder => None,
    };

    let mut by = vec![By::Unsaved; changes::line_count(&text)];
    // Lines waiting at a version, by its save and path.
    let mut waiting: HashMap<_, Vec<(usize, usize)>> = HashMap::new();
    let mut remembered = Remembered::default();
    // Newest save first, so lines reaching one version by two ways are mostly taken together; a
    // date that lies costs a lookup of what was found, never a wrong answer.
    while let Some(turn) = heap.pop() {
        let Turn {
            id,
            path: at_path,
            entry,
            mut lines,
            ..
        } = turn;
        lines.extend(
            waiting
                .remove(&(id, Rc::clone(&at_path)))
                .unwrap_or_default(),
        );
        let save = read_save(repo, id)?;
        // Read only when a parent holds other contents than this version's.
        let mut here: Option<Option<Cow<'_, [u8]>>> = None;
        for (i, parent_id) in save.parents.iter().enumerate() {
            if lines.is_empty() {
                break;
            }
            let parent = read_save(repo, *parent_id)?;
            let mut there = Rc::clone(&at_path);
            if i == 0
                && let Some(from) = moved_from(&stored, &save, &parent, &at_path)?
            {
                there = from.into();
            }
            let Some(e) = Tree::new(&standing, parent.root, kept::EXACT).entry(&there)? else {
                continue;
            };
            if !is_file(e.kind) {
                continue;
            }
            let taken = if e.id == entry.id {
                std::mem::take(&mut lines)
            } else {
                let pair = (entry.id, e.id);
                let found = match remembered.found.get(&pair) {
                    Some(found) => Rc::clone(found),
                    None => {
                        if here.is_none() {
                            here = Some(version(store, &entry, asked)?);
                        }
                        let text = here.as_ref().and_then(|h| h.as_deref());
                        let found = compare(store, text, &e, &mut coarse)?;
                        remembered.keep(pair, Rc::clone(&found));
                        found
                    }
                };
                take(&found, &mut lines)
            };
            if taken.is_empty() {
                continue;
            }
            let key = (*parent_id, there);
            match waiting.get_mut(&key) {
                Some(other) => other.extend(taken),
                None => {
                    heap.push(Turn {
                        time_ms: parent.saver.time_ms,
                        id: key.0,
                        path: Rc::clone(&key.1),
                        entry: e,
                        lines: Vec::new(),
                    });
                    waiting.insert(key, taken);
                }
            }
        }
        let was = (*at_path != *path).then_some(at_path);
        for (origin, _) in lines {
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
        in_save,
    })
}

fn read_save(repo: &Repository, id: Id) -> Result<Save, RepoError> {
    Ok(repo.store().save(&id, repo.signatures_allowed())?)
}

fn is_file(kind: EntryKind) -> bool {
    matches!(kind, EntryKind::File | EntryKind::Executable)
}

/// What a save holds where the folder has no file, named as what it is.
fn is_not_a_file(kind: EntryKind) -> bool {
    matches!(
        kind,
        EntryKind::Folder | EntryKind::Symlink | EntryKind::ProjectLink
    )
}

/// The version asked about, or why it has no lines to name.
fn asked_text(side: Side, path: &str) -> Result<Vec<u8>, WhoError> {
    let path = path.to_owned();
    let Side::Bytes(bytes) = side else {
        return Err(WhoError::TooBig { path });
    };
    match changes::comparable(&bytes) {
        Ok(()) => Ok(bytes),
        Err(NotLines::TooBig) => Err(WhoError::TooBig { path }),
        Err(NotLines::Binary) => Err(WhoError::NotText { path }),
        Err(NotLines::TooManyLines) => Err(WhoError::TooManyLines { path }),
    }
}

/// A version's text when it can be compared line by line: the one asked about is already held.
fn version<'a>(
    store: &Store,
    entry: &Entry,
    asked: Option<(Id, &'a [u8])>,
) -> Result<Option<Cow<'a, [u8]>>, SaveError> {
    if let Some((id, text)) = asked
        && id == entry.id
    {
        return Ok(Some(Cow::Borrowed(text)));
    }
    let side = changes::stored(store, Some(entry.kind), Some(entry.id), Some(entry.size))?;
    Ok(match side {
        Side::Bytes(b) if changes::comparable(&b).is_ok() => Some(Cow::Owned(b)),
        _ => None,
    })
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

/// Which lines of `here` the older version `older` has too. A version that can't be compared line
/// by line has none.
fn compare(
    store: &Store,
    here: Option<&[u8]>,
    older: &Entry,
    coarse: &mut bool,
) -> Result<Stretches, SaveError> {
    let none = || Rc::new(Vec::new());
    let Some(here) = here else {
        return Ok(none());
    };
    let Side::Bytes(old) =
        changes::stored(store, Some(older.kind), Some(older.id), Some(older.size))?
    else {
        return Ok(none());
    };
    if changes::comparable(&old).is_err() {
        return Ok(none());
    }
    #[cfg(test)]
    COMPARED.set(COMPARED.get() + 1);
    let (ops, late) = changes::line_ops(&keys_of(&old), &keys_of(here), changes::deadline());
    *coarse |= late;
    let found = ops
        .into_iter()
        .filter_map(|op| match op {
            DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => Some((new_index, old_index, len)),
            _ => None,
        })
        .collect();
    Ok(Rc::new(found))
}

/// The lines of `left` that `found` says the older version has, each with its line there; they
/// leave `left`.
fn take(found: &[(usize, usize, usize)], left: &mut Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    let mut taken = Vec::new();
    left.retain(|&(origin, here)| {
        let i = found.partition_point(|&(new, _, len)| new.saturating_add(len) <= here);
        match found.get(i) {
            Some(&(new, old, _)) if new <= here => {
                taken.push((origin, old + (here - new)));
                false
            }
            _ => true,
        }
    });
    taken
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

    /// A combine whose file is exactly its second parent's: a line both parents wrote, each on its
    /// own, is the first parent's, as parents are asked in order. Git gives it to the second.
    #[test]
    fn a_line_both_parents_wrote_is_the_first_parents_even_when_the_file_is_the_seconds() {
        let (_dir, mut repo) = project();
        let mut plant = Plant::new(&mut repo);
        let base = plant.base;
        let start = plant.save("Start", &[base], &[("a.txt", b"a\n")], &[], 1);
        let one = plant.save("One", &[start], &[("a.txt", b"a\nx\n")], &[], 2);
        let two = plant.save("Two", &[start], &[("a.txt", b"a\nx\ny\n")], &[], 3);
        let combined = plant.save("Combined", &[one, two], &[("a.txt", b"a\nx\ny\n")], &[], 4);
        plant.commit(&mut repo);
        assert_eq!(
            at(&repo, combined, "a.txt").runs,
            runs(&[(1, by(start)), (1, by(one)), (1, by(two))])
        );
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

    /// A path can be long and a file can have many lines: the old path is held once, not per line.
    #[test]
    fn a_moved_files_old_path_is_held_once_for_all_its_lines() {
        let mut fx = Fx::new();
        let text = |edited: bool| -> Vec<u8> {
            (0..1000)
                .map(|i| match edited && i % 2 == 1 {
                    true => format!("{i} edited\n"),
                    false => format!("{i}\n"),
                })
                .collect::<String>()
                .into_bytes()
        };
        fx.write("a.txt", &text(false));
        let first = fx.save("First");
        fx.write("a.txt", &text(true));
        let second = fx.save("Second");
        fs::rename(fx.root().join("a.txt"), fx.root().join("b.txt")).unwrap();
        let moved = fx.save("Moved");
        let w = fx.at(moved, "b.txt");
        let expected: Vec<(usize, By)> = (0..1000)
            .map(|i| match i % 2 {
                0 => (1, by_was(first, "a.txt")),
                _ => (1, by_was(second, "a.txt")),
            })
            .collect();
        assert_eq!(w.runs, runs(&expected));
        let held: HashSet<*const u8> = w
            .runs
            .iter()
            .map(|r| match &r.by {
                By::Save { was: Some(was), .. } => was.as_ptr(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(held.len(), 1);
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
        assert!(w.in_save);
        // Every line new, but the save still has the file.
        fx.write("a.txt", b"x\ny\n");
        let w = fx.folder("a.txt");
        assert_eq!(w.runs, runs(&[(2, By::Unsaved)]));
        assert!(w.in_save);
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
        assert!(!w.in_save);
        // An empty file has no runs.
        fx.write("empty.txt", b"");
        assert_eq!(fx.folder("empty.txt").runs, []);
    }

    /// Windows and macOS drives ignore letter case; Linux's don't, so it doesn't run there.
    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn a_path_typed_in_another_case_finds_the_saved_file() {
        let mut fx = Fx::new();
        assert!(fx.repo.drive().unwrap().ignores_case);
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
        assert_eq!(
            no_file(fx.who("d/e.txt/f", Asked::Folder)),
            ("d/e.txt/f".into(), None)
        );
        // A folder in the folder, where the save has one too, is named as one.
        let e = fx.who("d", Asked::Folder).unwrap_err();
        assert!(
            matches!(&e, WhoError::NotAFile { path, kind: EntryKind::Folder } if path == "d"),
            "{e:?}"
        );
        // One the save doesn't have is no file.
        fs::create_dir(fx.root().join("new")).unwrap();
        assert_eq!(no_file(fx.who("new", Asked::Folder)), ("new".into(), None));
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
        // At the save, and in the folder (which has nothing there) as the save has it.
        for asked in [Asked::Save(tip), Asked::Folder] {
            for (path, kind) in [("l", EntryKind::Symlink), ("p", EntryKind::ProjectLink)] {
                let e = who(&repo, &WhoRequest { path, asked }).unwrap_err();
                assert!(
                    matches!(&e, WhoError::NotAFile { path: p, kind: k } if p == path && *k == kind),
                    "{e:?}"
                );
            }
        }
    }

    /// Windows refuses a name such as `*.txt` itself: no file is there.
    #[cfg(windows)]
    #[test]
    fn a_name_the_system_cant_hold_is_no_file() {
        let mut fx = Fx::new();
        fx.write("a.txt", b"a\n");
        fx.save("First");
        let e = fx.who("*.txt", Asked::Folder).unwrap_err();
        assert!(
            matches!(&e, WhoError::NoFile { path, save: None } if path == "*.txt"),
            "{e:?}"
        );
    }

    /// In a project that doesn't keep conflicts, a conflict's entry is no file: asked about, it
    /// is refused, and as an older version it takes no lines. Where conflicts are kept, it is
    /// what it stands as.
    #[test]
    fn a_conflict_entry_where_conflicts_are_not_kept_is_no_file() {
        let (dir, mut repo) = project();
        let mine = crate::conflict::tests::file_of(b"a\nb\n");
        let record = crate::conflict::Record {
            terms: vec![
                Some(mine),
                None,
                Some(crate::conflict::tests::file_of(b"x\n")),
            ],
            own: None,
        };
        // `keep.txt` stores the bytes the conflict stands as.
        crate::test_support::plant_with(
            &mut repo,
            &[("keep.txt", EntryKind::File, b"a\nb\n")],
            &[("c.txt", record)],
        );
        let conflicted = history::current_save(&repo).unwrap();
        plant_save(&mut repo, &[("c.txt", EntryKind::File, b"a\nb\nc\n")]);
        let after = history::current_save(&repo).unwrap();
        assert_eq!(
            at(&repo, after, "c.txt").runs,
            runs(&[(2, by(conflicted)), (1, by(after))])
        );
        drop(repo);
        let repo = crate::test_support::without_conflicts_feature(dir.path());
        assert_eq!(at(&repo, after, "c.txt").runs, runs(&[(3, by(after))]));
        let e = who(
            &repo,
            &WhoRequest {
                path: "c.txt",
                asked: Asked::Save(conflicted),
            },
        )
        .unwrap_err();
        assert!(
            matches!(
                &e,
                WhoError::NotAFile {
                    kind: EntryKind::Conflict,
                    ..
                }
            ),
            "{e:?}"
        );
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
            // Every parent adds `common` too.
            let text = [b"start\n", line.as_bytes(), b"common\n"].concat();
            let message = format!("Adds {i}");
            parents.push(plant.save(&message, &[start], &[("a.txt", &text)], &[], 2 + i));
            all.extend_from_slice(line.as_bytes());
        }
        all.extend_from_slice(b"common\n");
        let combined = plant.save("Combined", &parents, &[("a.txt", &all)], &[], 100);
        plant.commit(&mut repo);
        let mut expected = vec![(1, by(start))];
        expected.extend(parents.iter().map(|p| (1, by(*p))));
        expected.push((1, by(parents[0])));
        assert_eq!(at(&repo, combined, "a.txt").runs, runs(&expected));
    }

    /// Of three parents, only the second and third have a line: it is the second's.
    #[test]
    fn a_line_the_first_parent_lacks_goes_to_the_next_that_has_it() {
        let (_dir, mut repo) = project();
        let mut plant = Plant::new(&mut repo);
        let base = plant.base;
        let start = plant.save("Start", &[base], &[("a.txt", b"start\n")], &[], 1);
        let one = plant.save("One", &[start], &[("a.txt", b"start\none\n")], &[], 2);
        let two = plant.save("Two", &[start], &[("a.txt", b"start\nboth\n")], &[], 3);
        let three = plant.save("Three", &[start], &[("a.txt", b"start\nboth\n3\n")], &[], 4);
        let combined = plant.save(
            "Combined",
            &[one, two, three],
            &[("a.txt", b"start\none\nboth\n3\n")],
            &[],
            5,
        );
        plant.commit(&mut repo);
        assert_eq!(
            at(&repo, combined, "a.txt").runs,
            runs(&[(1, by(start)), (1, by(one)), (1, by(two)), (1, by(three))])
        );
    }

    /// The file changes in the first two saves, then not for 40: it is read for the version asked
    /// about (the same contents as the save that last changed it) and for that save's parent.
    #[test]
    fn a_file_that_did_not_change_is_not_read_again() {
        let (_dir, mut repo) = project();
        let mut plant = Plant::new(&mut repo);
        let base = plant.base;
        let first = plant.save("First", &[base], &[("a.txt", b"a\n")], &[], 1);
        let changed = plant.save("Changed", &[first], &[("a.txt", b"a\nb\n")], &[], 2);
        let mut tip = changed;
        for i in 0..40 {
            let other = format!("{i}\n");
            let files: [(&str, &[u8]); 2] = [("a.txt", b"a\nb\n"), ("o.txt", other.as_bytes())];
            tip = plant.save("Other", &[tip], &files, &[], 3 + i);
        }
        plant.commit(&mut repo);
        let before = crate::store::counted::contents();
        let w = at(&repo, tip, "a.txt");
        assert_eq!(crate::store::counted::contents() - before, 2);
        assert_eq!(w.runs, runs(&[(1, by(first)), (1, by(changed))]));
    }

    /// Lines that reach a version after its turn: `m` combines eight saves each keeping one of
    /// its lines from `v`, and `v` has ten saves behind it, each dated newer than the one after
    /// it, so `v` is reached eight times.
    #[test]
    fn lines_that_arrive_late_are_not_compared_again() {
        let (_dir, mut repo) = project();
        let mut plant = Plant::new(&mut repo);
        let all: String = (1..=8).map(|i| format!("line {i}\n")).collect();
        let mut below = plant.base;
        let mut oldest = None;
        for j in (1..=10).rev() {
            let text = format!("{all}older {j}\n");
            below = plant.save(
                "Older",
                &[below],
                &[("a.txt", text.as_bytes())],
                &[],
                100 + j,
            );
            oldest.get_or_insert(below);
        }
        let v = plant.save("V", &[below], &[("a.txt", all.as_bytes())], &[], 50);
        let kept: Vec<Id> = (1..=8)
            .map(|i| {
                let one = format!("line {i}\n");
                plant.save("Keeps one", &[v], &[("a.txt", one.as_bytes())], &[], 20)
            })
            .collect();
        let m = plant.save("M", &kept, &[("a.txt", all.as_bytes())], &[], 10);
        plant.commit(&mut repo);
        let before = COMPARED.get();
        let w = at(&repo, m, "a.txt");
        // One for each pair of a version and a parent: m with each of the eight, each of them
        // with v, v with the save before it, and nine more down to the first.
        assert_eq!(COMPARED.get() - before, 8 + 8 + 10);
        assert_eq!(w.runs, runs(&[(8, by(oldest.unwrap()))]));
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

    /// Paths as stored never ask the drive. A path no save has exactly (`none.txt` here, or one
    /// typed in another letter case) does, and its probe file under `.opalro/tmp` is gone again.
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
                    assert_ne!(was.as_deref(), Some(path.as_str()));
                    let there = was.as_deref().map_or_else(|| path.clone(), str::to_owned);
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
