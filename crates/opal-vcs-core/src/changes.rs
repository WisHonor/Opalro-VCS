//! `opalro changes`: what changed inside files, line by line (decisions 058 and 064).

use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use opal_platform::dir::{EntryType, is_link_refusal};
use similar::algorithms::{Capture, Replace, myers};
use similar::{DiffOp, DiffTag, group_diff_ops};

use crate::conflict;
use crate::diff::{ChangeKind, FileChange, change_at, for_each_change, for_each_change_between};
use crate::history::tip_of;
use crate::id::Id;
use crate::listing::{Entry, EntryKind};
use crate::repo::Repository;
use crate::scope::Scope;
use crate::snapshot::{SaveError, link_target, read_folder};
use crate::status::{Difference, PathChange, each_difference};
use crate::store::{Store, StoreError};
use crate::writeout::{self, Folders};

/// A file bigger than this shows without lines, and isn't read.
pub const MAX_LINES_SIZE: u64 = 64 << 20;
/// A file with more lines than this on either side shows without them (decision 064).
pub const MAX_LINES: usize = 1_000_000;
/// A file with a zero byte this near its start is binary, as Git decides.
const BINARY_CHECK: usize = 8000;
/// How long one file's diff may take.
pub const DEADLINE: Duration = Duration::from_secs(2);
const CONTEXT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    Same,
    Removed,
    Added,
}

/// A run of changed lines with the lines around them, as in a unified diff. A start is the first
/// line's number, from 1, or the line before for a side with no lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk<'a> {
    pub old_start: u64,
    pub old_lines: u64,
    pub new_start: u64,
    pub new_lines: u64,
    /// Each line with its ending (`\n`, `\r\n`, or none for a last line without one), borrowed
    /// from the file.
    pub lines: Vec<(Tag, &'a [u8])>,
}

/// What changed inside one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lines<'a> {
    Binary,
    TooBig,
    TooManyLines,
    /// `timed_out` when the diff ran past its deadline, so its hunks are coarser than they could
    /// be.
    Hunks {
        hunks: Vec<Hunk<'a>>,
        timed_out: bool,
    },
}

/// The changed lines between `old` and `new`, with 3 lines of context. Binary when either side
/// has a zero byte in its first 8,000.
#[must_use]
pub fn file_lines<'a>(old: &'a [u8], new: &'a [u8], deadline: Duration) -> Lines<'a> {
    if is_binary(old) || is_binary(new) {
        return Lines::Binary;
    }
    // Counted before anything is held per line.
    if line_count(old) > MAX_LINES || line_count(new) > MAX_LINES {
        return Lines::TooManyLines;
    }
    let (old, new) = (lines_of(old), lines_of(new));
    let (ops, timed_out) = line_ops(&old, &new, deadline);
    let hunks = group_diff_ops(ops, CONTEXT)
        .iter()
        .map(|group| hunk(group, &old, &new))
        .collect();
    Lines::Hunks { hunks, timed_out }
}

/// Myers line by line between `old` and `new` with a deadline: the operations, and whether it
/// ran past the deadline (its answer is then coarser, never wrong).
pub(crate) fn line_ops(old: &[&[u8]], new: &[&[u8]], deadline: Duration) -> (Vec<DiffOp>, bool) {
    let start = Instant::now();
    // Myers alone: the clean-up pass `similar` adds after it has no deadline.
    let mut ops = Replace::new(Capture::new());
    let diffed = myers::diff_deadline(
        &mut ops,
        old,
        0..old.len(),
        new,
        0..new.len(),
        start.checked_add(deadline),
    );
    let ops = match diffed {
        Ok(()) => ops.into_inner().into_ops(),
        Err(never) => match never {},
    };
    // `similar` doesn't say whether it gave up: one that ran past its deadline did.
    (ops, start.elapsed() >= deadline)
}

/// How line endings changed, when nothing else did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endings {
    CrlfToLf,
    LfToCrlf,
    /// Some lines one way, some the other.
    Both,
}

/// How line endings changed, if the hunks change nothing else: every line they remove or add is
/// one of a pair [`eol_only`] marks.
#[must_use]
pub fn only_line_endings(hunks: &[Hunk<'_>]) -> Option<Endings> {
    let mut found = None;
    for h in hunks {
        let pairs = ending_pairs(&h.lines);
        let changed = h.lines.iter().filter(|(t, _)| *t != Tag::Same).count();
        if pairs.len() * 2 != changed {
            return None;
        }
        for (_, _, way) in pairs {
            found = match found {
                Some(seen) if seen != way => Some(Endings::Both),
                _ => Some(way),
            };
        }
    }
    found
}

/// For each line of `hunk`, whether it is one of a removed and an added line that differ only in
/// their ending, `\r\n` for `\n` or the other way round. The pairs are the diff's own: the i-th of
/// a run of removed lines with the i-th of the added lines right after it.
#[must_use]
pub fn eol_only(hunk: &Hunk<'_>) -> Vec<bool> {
    let mut marked = vec![false; hunk.lines.len()];
    for (old, new, _) in ending_pairs(&hunk.lines) {
        for at in [old, new] {
            if let Some(m) = marked.get_mut(at) {
                *m = true;
            }
        }
    }
    marked
}

/// The pairs [`eol_only`] marks, by where they are among `lines`, with the way each changed.
fn ending_pairs(lines: &[(Tag, &[u8])]) -> Vec<(usize, usize, Endings)> {
    let run = |from: usize, tag| {
        lines
            .iter()
            .skip(from)
            .take_while(|(t, _)| *t == tag)
            .count()
    };
    let mut found = Vec::new();
    let mut at = 0;
    while at < lines.len() {
        let removed = run(at, Tag::Removed);
        let added = run(at + removed, Tag::Added);
        for k in 0..removed.min(added) {
            let (old, new) = (at + k, at + removed + k);
            let way = match (lines.get(old), lines.get(new)) {
                (Some((_, a)), Some((_, b))) => ending_change(a, b),
                _ => None,
            };
            found.extend(way.map(|way| (old, new, way)));
        }
        at += (removed + added).max(1);
    }
    found
}

/// How `old` became `new`, if only its ending changed.
fn ending_change(old: &[u8], new: &[u8]) -> Option<Endings> {
    match (ending(old), ending(new)) {
        ((a, b"\r\n"), (b, b"\n")) if a == b => Some(Endings::CrlfToLf),
        ((a, b"\n"), (b, b"\r\n")) if a == b => Some(Endings::LfToCrlf),
        _ => None,
    }
}

/// A line split into what it holds and its ending.
fn ending(line: &[u8]) -> (&[u8], &[u8]) {
    for end in [&b"\r\n"[..], b"\n"] {
        if let Some(body) = line.strip_suffix(end) {
            return (body, end);
        }
    }
    (line, b"")
}

/// Hands `each` every unsaved change, as `status` finds them, with its lines: the folder against
/// the save the project is on, and at a place in conflict against what the conflict stands as.
/// Only `paths` and what is in them, if given. Nothing is stored; only the file-state cache is
/// written. The folder walk's warnings go to the store's. One file is held at a time.
pub fn unsaved_changes<E: From<SaveError> + From<StoreError>>(
    repo: &Repository,
    paths: Option<&[String]>,
    git_excludes: Option<&Path>,
    each: &mut dyn FnMut(&str, &PathChange, Lines<'_>) -> Result<(), E>,
) -> Result<(), E> {
    let store = repo.store();
    let (_, entry) = repo.current().map_err(SaveError::from)?;
    let tip = tip_of(&entry).map_err(SaveError::from)?;
    let save = store.save(&tip, repo.signatures_allowed())?;
    let mut now = read_folder(repo, save.root, false, git_excludes, None)
        .map_err(|e| e.in_save(tip, true))?;
    // No switch follows, so the cache's body isn't needed.
    now.cache_body = None;
    for w in std::mem::take(&mut now.warnings)
        .into_iter()
        .chain(writeout::unfinished_warning(repo))
    {
        store.note(w);
    }
    let scope = scope(repo, paths)?;
    let mut disk = Folders::open(repo.root()).map_err(|e| unreadable("the project folder", e))?;
    let mut emit = |c: PathChange| -> Result<(), Stopped<E>> {
        if !scope.as_ref().is_none_or(|s| s.holds(&c.path)) {
            return Ok(());
        }
        let old = stored(store, c.old_kind, c.old_id, c.old_size)?;
        let new = on_disk(&mut disk, &c)?;
        each(&c.path, &c, lines(&old, &new)).map_err(Stopped::Each)
    };
    let mut records = std::mem::take(&mut now.records);
    let walked = each_difference(repo, &now, &mut records, save.root, |d| match d {
        Difference::File(c) => emit(PathChange::from(c)),
        Difference::Conflict {
            path,
            id,
            record,
            now: here,
        } => (|| {
            // Against what it stands as: a folder file by file, as one save against another.
            let name = path.rsplit_once('/').map_or(path, |(_, name)| name);
            let was = record.stands_as.map(|t| Entry {
                name: name.to_owned(),
                kind: t.kind,
                id: t.id,
                size: t.size,
            });
            let in_conflict = |mut c: PathChange| {
                c.in_conflict = true;
                c
            };
            if let Some(c) = change_at(path, was.as_ref(), here) {
                emit(in_conflict(PathChange::from(c)))?;
            }
            let folder =
                |e: Option<&Entry>| e.filter(|e| e.kind == EntryKind::Folder).map(|e| e.id);
            let (old, new) = (folder(was.as_ref()), folder(here));
            if old == new {
                return Ok(());
            }
            for_each_change_between(store, &now.listings, old, new, |c| {
                let mut c = in_conflict(PathChange::from(c));
                c.path = format!("{path}/{}", c.path);
                emit(c)
            })
        })()
        // What the record names that can't be read is damage at its place.
        .map_err(|stopped| match stopped {
            Stopped::Reading(e) => Stopped::Reading(conflict::at_place(path, &id, e)),
            each => each,
        }),
    });
    // A spelling of `.opalro` met reading is in the save the project is on, as for `status`.
    walked.map_err(|stopped| match stopped {
        Stopped::Reading(e) => e.in_save(tip, true).into(),
        Stopped::Each(e) => e,
    })
}

/// What stopped [`unsaved_changes`]' walk: reading, or the caller's `each`.
enum Stopped<E> {
    Reading(SaveError),
    Each(E),
}

impl<E> From<SaveError> for Stopped<E> {
    fn from(e: SaveError) -> Self {
        Stopped::Reading(e)
    }
}

impl<E> From<StoreError> for Stopped<E> {
    fn from(e: StoreError) -> Self {
        Stopped::Reading(e.into())
    }
}

/// Hands `each` every file the save `save` changed, compared with its first parent (every file,
/// for a save without one), with its lines. Only `paths` and what is in them, if given. One file
/// is held at a time.
pub fn save_changes<E: From<SaveError> + From<StoreError>>(
    repo: &Repository,
    save: Id,
    paths: Option<&[String]>,
    each: &mut dyn FnMut(&str, &PathChange, Lines<'_>) -> Result<(), E>,
) -> Result<(), E> {
    let store = repo.store();
    let save = store.save(&save, repo.signatures_allowed())?;
    let old_root = match save.parents.first() {
        Some(p) => Some(store.save(p, repo.signatures_allowed())?.root),
        None => None,
    };
    let scope = scope(repo, paths)?;
    for_each_save_change(repo, old_root, save.root, |c| {
        // A conflict's place speaks for everything below it, so a path asked there selects it.
        let asked =
            |s: &Scope| s.holds(c.path) || (c.change.is_conflict() && s.reaches_below(c.path));
        if !scope.as_ref().is_none_or(asked) {
            return Ok(());
        }
        let mut c = PathChange::from(c);
        if c.change.is_conflict() {
            // Whether the newer save holds a conflict there.
            c.in_conflict = c.change != ChangeKind::ConflictFixed;
            let none = Lines::Hunks {
                hunks: Vec::new(),
                timed_out: false,
            };
            return each(&c.path, &c, none);
        }
        let old = stored(store, c.old_kind, c.old_id, c.old_size)?;
        let new = stored(store, Some(c.kind), c.new_id, c.new_size)?;
        each(&c.path, &c, lines(&old, &new))
    })
}

/// [`for_each_change`] between two saves, as `show` and `changes --save` tell it: in a project
/// that keeps conflicts, a place that became one, stayed one with another record, or stopped
/// being one has a kind of its own. Old and new entries are enough to tell; no record is read.
pub fn for_each_save_change<E: From<StoreError>>(
    repo: &Repository,
    old_root: Option<Id>,
    new_root: Id,
    mut each: impl FnMut(FileChange<'_>) -> Result<(), E>,
) -> Result<(), E> {
    let keeps = repo.keeps_conflicts();
    // The places of conflicts handed out, `/` after each: a folder on the other side is the
    // conflict's, so its files aren't changes of their own. A place's own path comes before the
    // paths below it, but a sibling such as `dir.txt` can come between them, so a place is kept
    // until the walk has passed everything below it.
    let mut places: Vec<String> = Vec::new();
    for_each_change(repo.store(), old_root, new_root, |mut c| {
        if !keeps {
            return each(c);
        }
        places.retain(|p| c.path.starts_with(p.as_str()) || c.path < p.as_str());
        if places.iter().any(|p| c.path.starts_with(p.as_str())) {
            return Ok(());
        }
        let held = |k: Option<EntryKind>| k == Some(EntryKind::Conflict);
        // `kind` is the new side's when there is one.
        let new = c.new_id.map(|_| c.kind);
        c.change = match (held(c.old_kind), held(new)) {
            (false, true) => ChangeKind::InConflict,
            (true, true) => ChangeKind::StillInConflict,
            (true, false) => ChangeKind::ConflictFixed,
            (false, false) => return each(c),
        };
        places.push(format!("{}/", c.path));
        each(c)
    })
}

fn scope(repo: &Repository, paths: Option<&[String]>) -> Result<Option<Scope>, SaveError> {
    Ok(match paths {
        Some(paths) => Some(Scope::paths(paths, repo.drive()?)),
        None => None,
    })
}

/// One side of a file: what it holds, or too much to read.
pub(crate) enum Side {
    Bytes(Vec<u8>),
    TooBig,
}

fn lines<'a>(old: &'a Side, new: &'a Side) -> Lines<'a> {
    match (old, new) {
        (Side::Bytes(old), Side::Bytes(new)) => file_lines(old, new, deadline()),
        _ => Lines::TooBig,
    }
}

/// How long a file's diff may take; crash tests can shorten it.
pub(crate) fn deadline() -> Duration {
    #[cfg(feature = "fault-injection")]
    if let Some(ms) = std::env::var("OPALRO_DIFF_DEADLINE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        return Duration::from_millis(ms);
    }
    DEADLINE
}

/// Files and links have contents; a project link or conflict shows as empty.
fn has_content(kind: EntryKind) -> bool {
    matches!(
        kind,
        EntryKind::File | EntryKind::Executable | EntryKind::Symlink
    )
}

/// What the store holds for one side, read whole unless it is over the limit. The size a listing
/// gives is checked first, and the read stops past the limit whatever the listing said.
pub(crate) fn stored(
    store: &Store,
    kind: Option<EntryKind>,
    id: Option<Id>,
    size: Option<u64>,
) -> Result<Side, SaveError> {
    let (Some(kind), Some(id)) = (kind, id) else {
        return Ok(Side::Bytes(Vec::new()));
    };
    if !has_content(kind) {
        return Ok(Side::Bytes(Vec::new()));
    }
    if size.unwrap_or(0) > MAX_LINES_SIZE {
        return Ok(Side::TooBig);
    }
    Ok(read_capped(store, &id, MAX_LINES_SIZE)?.map_or(Side::TooBig, Side::Bytes))
}

/// A file's bytes from the store, or `None` once they pass `limit`, whatever its listing said.
pub(crate) fn read_capped(
    store: &Store,
    id: &Id,
    limit: u64,
) -> Result<Option<Vec<u8>>, StoreError> {
    let mut out = Capped {
        bytes: Vec::new(),
        over: false,
        limit,
    };
    match store.copy_content(id, &mut out) {
        Ok(()) => Ok(Some(out.bytes)),
        Err(_) if out.over => Ok(None),
        Err(e) => Err(e),
    }
}

/// Bytes, refusing to hold more than `limit`.
struct Capped {
    bytes: Vec<u8>,
    over: bool,
    limit: u64,
}

impl Write for Capped {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let room = usize::try_from(self.limit)
            .unwrap_or(usize::MAX)
            .saturating_sub(self.bytes.len());
        if buf.len() > room {
            self.over = true;
            return Err(io::Error::other("over the size Opalro reads it at"));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// What the folder holds for the new side of an unsaved change.
fn on_disk(disk: &mut Folders, c: &PathChange) -> Result<Side, SaveError> {
    if c.change == ChangeKind::Removed || !has_content(c.kind) {
        return Ok(Side::Bytes(Vec::new()));
    }
    if c.new_size.unwrap_or(0) > MAX_LINES_SIZE {
        return Ok(Side::TooBig);
    }
    if c.kind == EntryKind::Symlink {
        let (up, name) = c.path.rsplit_once('/').unwrap_or(("", &c.path));
        disk.enter(up).map_err(|e| unreadable(&c.path, e))?;
        return Ok(Side::Bytes(
            link_target(disk.here(), name, &c.path)?.into_bytes(),
        ));
    }
    // The walk saw a file here, so one gone since is unreadable, not "no file".
    match read_file(disk, &c.path) {
        Ok(side) => Ok(side),
        Err(Unread::NoFile(e) | Unread::Failed(e)) => Err(e),
    }
}

/// The folder's file at `path`, never through a link: its bytes, `TooBig` past
/// `MAX_LINES_SIZE`, or `None` when no regular file is there.
pub(crate) fn in_folder(disk: &mut Folders, path: &str) -> Result<Option<Side>, SaveError> {
    match read_file(disk, path) {
        Ok(side) => Ok(Some(side)),
        Err(Unread::NoFile(_)) => Ok(None),
        Err(Unread::Failed(e)) => Err(e),
    }
}

/// Why a file of the folder wasn't read: no regular file there, or one that couldn't be read.
enum Unread {
    NoFile(SaveError),
    Failed(SaveError),
}

fn read_file(disk: &mut Folders, path: &str) -> Result<Side, Unread> {
    let (up, name) = path.rsplit_once('/').unwrap_or(("", path));
    let failed = |e| Unread::Failed(unreadable(path, e));
    if let Err(e) = disk.enter(up) {
        // A folder on the way that is missing, a file, or a link.
        let gone = matches!(
            e.kind(),
            io::ErrorKind::NotFound | io::ErrorKind::NotADirectory | io::ErrorKind::InvalidFilename
        ) || is_link_refusal(&e);
        return Err(if gone {
            Unread::NoFile(unreadable(path, e))
        } else {
            failed(e)
        });
    }
    let dir = disk.here();
    let name = OsStr::new(name);
    // Opening never follows a link; what failed to open is looked at for what it is.
    let file = dir.open_file(name).map_err(|e| {
        // A name the system can't hold at all is no file there either.
        let no_file = matches!(
            e.kind(),
            io::ErrorKind::NotFound | io::ErrorKind::InvalidFilename
        ) || match dir.metadata(name) {
            Ok(now) => now.kind != EntryType::File,
            Err(now) => now.kind() == io::ErrorKind::NotFound,
        };
        if no_file {
            Unread::NoFile(unreadable(path, e))
        } else {
            failed(e)
        }
    })?;
    // It may have grown since the walk.
    let mut bytes = Vec::new();
    file.take(MAX_LINES_SIZE.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(failed)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_LINES_SIZE {
        return Ok(Side::TooBig);
    }
    Ok(Side::Bytes(bytes))
}

fn unreadable(path: &str, source: io::Error) -> SaveError {
    SaveError::Unreadable {
        path: path.to_owned(),
        source,
    }
}

/// Why a text can't be compared line by line, the first reason first: too big, not text, too
/// many lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum NotLines {
    TooBig,
    Binary,
    TooManyLines,
}

/// Whether `bytes` can be compared line by line, and if not, why.
pub(crate) fn comparable(bytes: &[u8]) -> Result<(), NotLines> {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_LINES_SIZE {
        return Err(NotLines::TooBig);
    }
    if is_binary(bytes) {
        return Err(NotLines::Binary);
    }
    // Counted before anything is held per line.
    if line_count(bytes) > MAX_LINES {
        return Err(NotLines::TooManyLines);
    }
    Ok(())
}

pub(crate) fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(BINARY_CHECK).any(|&b| b == 0)
}

/// How many lines `bytes` holds, a last one without `\n` included.
pub(crate) fn line_count(bytes: &[u8]) -> usize {
    let breaks = bytes.iter().filter(|&&b| b == b'\n').count();
    breaks.saturating_add(usize::from(bytes.last().is_some_and(|&b| b != b'\n')))
}

/// Each line with its `\n`; a last line without one stays as it is.
pub(crate) fn lines_of(bytes: &[u8]) -> Vec<&[u8]> {
    bytes.split_inclusive(|&b| b == b'\n').collect()
}

fn hunk<'a>(group: &[DiffOp], old: &[&'a [u8]], new: &[&'a [u8]]) -> Hunk<'a> {
    let mut lines = Vec::new();
    let mut push = |tag, side: &[&'a [u8]], range: std::ops::Range<usize>| {
        lines.extend(
            side.get(range)
                .unwrap_or_default()
                .iter()
                .map(|l| (tag, *l)),
        );
    };
    for op in group {
        let (kind, o, n) = op.as_tag_tuple();
        match kind {
            DiffTag::Equal => push(Tag::Same, old, o),
            DiffTag::Delete => push(Tag::Removed, old, o),
            DiffTag::Insert => push(Tag::Added, new, n),
            DiffTag::Replace => {
                push(Tag::Removed, old, o);
                push(Tag::Added, new, n);
            }
        }
    }
    let count = |side: fn(&DiffOp) -> std::ops::Range<usize>| -> (u64, u64) {
        let first = group.first().map_or(0, |op| side(op).start);
        let len: usize = group.iter().map(|op| side(op).len()).sum();
        let start = if len == 0 {
            first
        } else {
            first.saturating_add(1)
        };
        (number(start), number(len))
    };
    let (old_start, old_lines) = count(DiffOp::old_range);
    let (new_start, new_lines) = count(DiffOp::new_range);
    Hunk {
        old_start,
        old_lines,
        new_start,
        new_lines,
        lines,
    }
}

fn number(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hunks_of<'a>(old: &'a str, new: &'a str) -> Vec<Hunk<'a>> {
        match file_lines(old.as_bytes(), new.as_bytes(), DEADLINE) {
            Lines::Hunks { hunks, .. } => hunks,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn hunks_have_their_numbers_and_context() {
        let hunks = hunks_of(
            "1\n2\n3\n4\n5\n6\n7\n8\n9\n",
            "1\n2\n3\n4\nfive\n6\n7\n8\n9\n",
        );
        assert_eq!(hunks.len(), 1);
        let h = &hunks[0];
        assert_eq!(
            (h.old_start, h.old_lines, h.new_start, h.new_lines),
            (2, 7, 2, 7)
        );
        assert_eq!(h.lines[3], (Tag::Removed, &b"5\n"[..]));
        assert_eq!(h.lines[4], (Tag::Added, &b"five\n"[..]));
        // A new file starts after line 0.
        let hunks = hunks_of("", "a");
        assert_eq!((hunks[0].old_start, hunks[0].old_lines), (0, 0));
        assert_eq!(hunks[0].lines, [(Tag::Added, &b"a"[..])]);
        assert!(hunks_of("", "").is_empty());
    }

    #[test]
    fn a_zero_byte_in_the_first_8000_is_binary() {
        let mut late = vec![b'a'; BINARY_CHECK];
        late.push(0);
        assert!(matches!(
            file_lines(&late, b"", DEADLINE),
            Lines::Hunks { .. }
        ));
        late.insert(0, 0);
        assert_eq!(file_lines(b"", &late, DEADLINE), Lines::Binary);
    }

    #[test]
    fn more_than_a_million_lines_on_a_side_shows_none() {
        let at_limit = vec![b'\n'; MAX_LINES];
        assert!(matches!(
            file_lines(b"", &at_limit, DEADLINE),
            Lines::Hunks { .. }
        ));
        let mut over = at_limit;
        over.push(b'x');
        assert_eq!(file_lines(&over, b"", DEADLINE), Lines::TooManyLines);
        assert_eq!(line_count(b"a\nb"), 2);
        assert_eq!(line_count(b""), 0);
    }

    #[test]
    fn a_diff_past_its_deadline_says_so() {
        let old: String = (0..2000).map(|n| format!("{n}\n")).collect();
        let new: String = (0..2000).map(|n| format!("{}\n", n * 7 % 2000)).collect();
        let Lines::Hunks { hunks, timed_out } =
            file_lines(old.as_bytes(), new.as_bytes(), Duration::ZERO)
        else {
            panic!("not lines");
        };
        assert!(timed_out && !hunks.is_empty());
    }

    #[test]
    fn a_diff_that_would_take_long_stops_at_its_deadline() {
        // No line in common: Myers' work grows with the square of the lines, minutes here.
        let old: String = (0..60_000).map(|n| format!("a{n}\n")).collect();
        let new: String = (0..60_000).map(|n| format!("b{n}\n")).collect();
        let started = Instant::now();
        let lines = file_lines(old.as_bytes(), new.as_bytes(), Duration::from_millis(100));
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
        let Lines::Hunks { hunks, timed_out } = lines else {
            panic!("not lines");
        };
        assert!(timed_out);
        // Coarser, still whole: every line goes and comes.
        let count = |tag| {
            hunks
                .iter()
                .flat_map(|h| &h.lines)
                .filter(|(t, _)| *t == tag)
                .count()
        };
        assert_eq!((count(Tag::Removed), count(Tag::Added)), (60_000, 60_000));
    }

    #[test]
    fn only_line_endings_is_told_apart() {
        let way = |old, new| only_line_endings(&hunks_of(old, new));
        assert_eq!(way("a\r\nb\r\n", "a\nb\n"), Some(Endings::CrlfToLf));
        assert_eq!(way("a\nb\n", "a\r\nb\n"), Some(Endings::LfToCrlf));
        assert_eq!(way("a\r\nb\n", "a\nb\r\n"), Some(Endings::Both));
        assert_eq!(way("a\r\n", "b\n"), None);
        assert_eq!(way("a", "a\n"), None);
        assert_eq!(way("a\n", "a\n"), None);
        // The endings changed, and a line moved too.
        assert_eq!(way("a\r\nb\r\ns\n", "a\ns\nb\n"), None);
    }

    #[test]
    fn a_pair_that_differs_only_in_its_ending_is_marked() {
        let marked =
            |old, new| -> Vec<Vec<bool>> { hunks_of(old, new).iter().map(eol_only).collect() };
        assert_eq!(
            marked("one\r\ntwo\nthree\n", "one\ntwo\nTHREE\n"),
            [[true, true, false, false, false]]
        );
        // The i-th line of each run of removed lines goes with the i-th of the added lines after it.
        assert_eq!(
            marked("a\r\nb\r\ns\n", "a\ns\nb\n"),
            [[true, false, true, false, false]]
        );
        // A line without an ending, or two that are the same, aren't such a pair.
        assert_eq!(marked("a\r\n", "a"), [[false, false]]);
        assert_eq!(marked("a\n", "b\n"), [[false, false]]);
    }

    #[test]
    fn changes_at_a_conflicts_place_compare_with_what_it_stands_as() {
        use crate::conflict::Term;
        use crate::conflict::tests::{file_of, plant_conflicted};
        use crate::status::tests::{HALF, MARKED, keep, marked, project, versions, write};
        let (dir, mut repo) = project();
        let held =
            crate::test_support::plant_save(&mut repo, &[("dir/a.txt", EntryKind::File, b"a\n")]);
        let folder = Term {
            kind: EntryKind::Folder,
            id: held.get("dir").unwrap().id,
            size: 0,
        };
        keep(&mut repo, &[b"mine\n", b"base", b"theirs", MARKED]);
        let conflicts = [("dir", versions(Some(folder))), ("story.txt", marked())];
        let files: &[(&str, EntryKind, &[u8])] = &[("plain.txt", EntryKind::File, b"one\n")];
        plant_conflicted(&mut repo, files, &conflicts);
        write(dir.path(), "plain.txt", b"two\n");
        write(dir.path(), "story.txt", HALF);
        write(dir.path(), "dir/a.txt", b"a, changed\n");
        let mut seen = Vec::new();
        unsaved_changes::<SaveError>(&repo, None, None, &mut |path, c, lines| {
            let removed: Vec<String> = match lines {
                Lines::Hunks { hunks, .. } => hunks
                    .iter()
                    .flat_map(|h| &h.lines)
                    .filter(|(t, _)| *t == Tag::Removed)
                    .map(|(_, l)| String::from_utf8_lossy(l).into_owned())
                    .collect(),
                other => panic!("{other:?}"),
            };
            seen.push((path.to_owned(), c.change, c.in_conflict, c.old_id, removed));
            Ok(())
        })
        .unwrap();
        let paths: Vec<(&str, bool)> = seen.iter().map(|s| (s.0.as_str(), s.2)).collect();
        assert_eq!(
            paths,
            [
                ("dir/a.txt", true),
                ("plain.txt", false),
                ("story.txt", true)
            ]
        );
        // The old side is the marked text: the markers of the place fixed are what went.
        let story = &seen[2];
        assert_eq!(story.1, ChangeKind::Modified);
        assert_eq!(story.3, Some(file_of(MARKED).id));
        assert!(
            story.4.contains(&"<<<<<<< main (kkkk)\n".to_owned()),
            "{:?}",
            story.4
        );
        assert_eq!(seen[0].1, ChangeKind::Modified);
        // Untouched, a place shows nothing.
        write(dir.path(), "story.txt", MARKED);
        write(dir.path(), "dir/a.txt", b"a\n");
        let mut paths = Vec::new();
        unsaved_changes::<SaveError>(&repo, None, None, &mut |path, _, _| {
            paths.push(path.to_owned());
            Ok(())
        })
        .unwrap();
        assert_eq!(paths, ["plain.txt"]);
    }

    #[test]
    fn a_folder_at_a_conflicts_place_is_listed_once() {
        use crate::conflict::tests::{file_of, plant_conflicted};
        use crate::status::tests::{keep, project, versions, write};
        let (dir, mut repo) = project();
        keep(&mut repo, &[b"mine\n", b"base", b"theirs"]);
        // One standing as a file, one standing as nothing: a folder made at each.
        let conflicts = [
            ("gone.txt", versions(None)),
            ("notes.txt", versions(Some(file_of(b"mine\n")))),
        ];
        plant_conflicted(&mut repo, &[], &conflicts);
        write(dir.path(), "gone.txt/b.txt", b"b\n");
        write(dir.path(), "notes.txt/a.txt", b"a\n");
        let mut seen = Vec::new();
        unsaved_changes::<SaveError>(&repo, None, None, &mut |path, c, _| {
            seen.push((path.to_owned(), c.change, c.in_conflict));
            Ok(())
        })
        .unwrap();
        assert_eq!(
            seen,
            [
                ("gone.txt/b.txt".to_owned(), ChangeKind::Added, true),
                ("notes.txt".to_owned(), ChangeKind::Removed, true),
                ("notes.txt/a.txt".to_owned(), ChangeKind::Added, true),
            ]
        );
    }

    #[test]
    fn a_version_naming_another_kind_is_damage_at_its_place() {
        use crate::conflict::Term;
        use crate::conflict::tests::plant_conflicted;
        use crate::status::tests::{keep, project, versions, write};
        let (dir, mut repo) = project();
        let held =
            crate::test_support::plant_save(&mut repo, &[("dir/a.txt", EntryKind::File, b"a\n")]);
        keep(&mut repo, &[b"base", b"theirs"]);
        // Standing as a file, it names a folder's listing.
        let wrong = Term {
            kind: EntryKind::File,
            id: held.get("dir").unwrap().id,
            size: 3,
        };
        let record = versions(Some(wrong));
        let id = record.id();
        plant_conflicted(&mut repo, &[], &[("notes.txt", record)]);
        write(dir.path(), "notes.txt", b"new\n");
        let e = unsaved_changes::<SaveError>(&repo, None, None, &mut |_, _, _| Ok(()))
            .unwrap_err()
            .to_string();
        let want = format!(
            "The conflict at notes.txt can't be read (record {}): ",
            id.short_hex()
        );
        assert!(e.starts_with(&want), "{e}");
    }

    #[test]
    fn capped_stops_at_the_limit() {
        let mut out = Capped {
            bytes: vec![0; usize::try_from(MAX_LINES_SIZE).unwrap() - 1],
            over: false,
            limit: MAX_LINES_SIZE,
        };
        assert!(out.write_all(b"a").is_ok() && !out.over);
        assert!(out.write_all(b"b").is_err() && out.over);
    }

    /// A place in a planted save: a file holding these bytes, or a conflict with them as yours.
    #[derive(Clone, Copy)]
    enum Place {
        File(&'static [u8]),
        Conflict(&'static [u8]),
    }

    /// What `changes --save` hands out for a save holding `after` at `d/a`, `d/b` and `d/c`
    /// after one holding `before`, and the listings and records it and `show`'s walk read.
    fn compared(
        before: [Place; 3],
        after: [Place; 3],
    ) -> (Vec<(String, ChangeKind)>, usize, usize) {
        use crate::status::tests::{project, versions};
        let (_dir, mut repo) = project();
        let mut plant = |places: [Place; 3]| {
            let (mut files, mut conflicts) = (Vec::new(), Vec::new());
            for (path, place) in ["d/a", "d/b", "d/c"].into_iter().zip(places) {
                match place {
                    Place::File(b) => files.push((path, EntryKind::File, b)),
                    Place::Conflict(b) => {
                        conflicts.push((path, versions(Some(conflict::tests::file_of(b)))));
                    }
                }
            }
            crate::test_support::plant_with(&mut repo, &files, &conflicts);
            crate::history::current_save(&repo).unwrap()
        };
        plant(before);
        let id = plant(after);
        let store = repo.store();
        let save = store.save(&id, false).unwrap();
        let old = store.save(&save.parents[0], false).unwrap().root;
        let before = (store.listings_read(), store.conflicts_read());
        let mut seen = Vec::new();
        save_changes(&repo, id, None, &mut |path, c, _| {
            seen.push((path.to_owned(), c.change));
            Ok::<(), SaveError>(())
        })
        .unwrap();
        for_each_save_change(&repo, Some(old), save.root, |_| Ok::<(), StoreError>(())).unwrap();
        let read = (store.listings_read(), store.conflicts_read());
        (seen, read.0 - before.0, read.1 - before.1)
    }

    #[test]
    fn telling_a_saves_conflicts_reads_no_record_and_no_more_listings() {
        use Place::{Conflict as C, File as F};
        // `d/a` made, `d/b` kept with another version, `d/c` fixed.
        let (seen, listings, records) = compared(
            [F(b"a0"), C(b"b0"), C(b"c0")],
            [C(b"a1"), C(b"b1"), F(b"c1")],
        );
        let kinds = [
            ChangeKind::InConflict,
            ChangeKind::StillInConflict,
            ChangeKind::ConflictFixed,
        ];
        let paths = ["d/a", "d/b", "d/c"].map(String::from);
        assert_eq!(
            seen,
            paths.clone().into_iter().zip(kinds).collect::<Vec<_>>()
        );
        assert_eq!(records, 0);
        // The same save with ordinary files at those places.
        let (seen, plain, records) = compared(
            [F(b"a0"), F(b"b0"), F(b"c0")],
            [F(b"a1"), F(b"b1"), F(b"c1")],
        );
        assert_eq!(seen.len(), 3);
        assert_eq!((listings, records), (plain, 0));
    }

    fn read_in(root: &std::path::Path, path: &str) -> Option<Side> {
        let mut disk = Folders::open(root).unwrap();
        in_folder(&mut disk, path).unwrap()
    }

    #[test]
    fn a_file_of_the_folder_is_read_by_itself() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("d/e")).unwrap();
        std::fs::write(dir.path().join("d/e/f.txt"), b"one\ntwo\n").unwrap();
        std::fs::write(dir.path().join("top"), b"").unwrap();
        assert!(matches!(
            read_in(dir.path(), "d/e/f.txt"),
            Some(Side::Bytes(b)) if b == b"one\ntwo\n"
        ));
        assert!(matches!(read_in(dir.path(), "top"), Some(Side::Bytes(b)) if b.is_empty()));
        // Nothing there, a folder, and a path through a file or a missing folder: no file.
        for path in [
            "d/e/gone.txt",
            "d/e",
            "d",
            "top/x",
            "missing/f.txt",
            "d/missing/f",
        ] {
            assert!(read_in(dir.path(), path).is_none(), "{path}");
        }
    }

    #[test]
    fn a_file_past_the_limit_is_too_big() {
        let dir = tempfile::tempdir().unwrap();
        let file = std::fs::File::create(dir.path().join("big")).unwrap();
        file.set_len(MAX_LINES_SIZE + 1).unwrap();
        drop(file);
        assert!(matches!(read_in(dir.path(), "big"), Some(Side::TooBig)));
    }

    #[test]
    fn a_link_in_the_folder_is_never_followed() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("f.txt"), b"outside").unwrap();
        // A folder link on the way: a junction on Windows, which this account can make.
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), dir.path().join("away")).unwrap();
        #[cfg(windows)]
        crate::test_support::junction(&dir.path().join("away"), outside.path());
        assert!(read_in(dir.path(), "away/f.txt").is_none());
        assert!(read_in(dir.path(), "away").is_none());
        // A link at the last name.
        let link = dir.path().join("link.txt");
        if !crate::test_support::symlink(&outside.path().join("f.txt"), &link) {
            return; // this account can't make links: CI runs it
        }
        assert!(read_in(dir.path(), "link.txt").is_none());
    }
}
