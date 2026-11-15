//! `opalro resolve`: records the places in conflict the person fixed, or a side kept whole, and
//! writes one version of a file in conflict out to look at.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::changeids::ChangeIds;
use crate::conflict::{self, Listings, Record, Recording, Seen, Term};
use crate::diary::{Action, DiaryEntry, Position};
use crate::drive::Drive;
use crate::fsio;
use crate::history::tip_of;
use crate::id::Id;
use crate::kept::overlay_with;
use crate::listing::{Entry, EntryKind, Listing};
use crate::moves::{self, MAX_LEFT_BYTES, MAX_NOTED_BYTES};
use crate::names::Given;
use crate::pack::PackSet;
use crate::repo::{ChangeLock, RepoError, Repository};
use crate::save::{MAX_MOVE_NOTES, Person, Save};
use crate::scope::within;
use crate::snapshot::{
    LargeSave, SaveError, SaveRequest, compose, keep_file_cache, read_and_build, shown_elsewhere,
    store_listing,
};
use crate::status::{self, ConflictAt, Standing};
use crate::store::{Store, StoreError};
use crate::switch::{Earlier, Planned, SwitchError, locked, make_switch};
use crate::text::{escape, thousands};
use crate::warning::Warning;
use crate::writeout::{self, Applied, fold_path};

/// How many places still in conflict an outcome names; it counts the rest.
pub const LISTED: usize = 100;

/// A version of a place in conflict: yours (the record's first), theirs (its last), or where the
/// lines parted (the middle of three).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Mine,
    Theirs,
    Before,
}

pub struct ResolveRequest<'a> {
    /// Places named, a folder standing for every place below it; `None` records every fixed one.
    pub paths: Option<&'a [String]>,
    /// The side kept whole at each place named.
    pub keep: Option<Pick>,
    pub who: &'a Person,
    /// The command as the diary records it; already redacted.
    pub command: &'a str,
    /// Record files Git rewrote with CRLF as they are, as `save --accept-crlf` does.
    pub accept_crlf: bool,
    pub git_excludes: Option<&'a Path>,
}

/// What `resolve` asks before it records.
#[derive(Debug)]
pub enum Ask<'a> {
    /// A file still holding marker lines: `left` opening ones, 0 when only others are.
    MarkersLeft {
        path: &'a str,
        left: u64,
    },
    Large(&'a LargeSave),
}

// Made once per command, so boxing the save would buy nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum ResolveOutcome {
    Recorded {
        save_id: Id,
        save: Save,
        short_change_id: String,
        /// The combined save it replaced, which stays stored.
        replaced: Option<Id>,
        /// The places recorded, as stored.
        recorded: Vec<String>,
        /// The places it met still in conflict: the first 100, and how many in all.
        left: Vec<ConflictAt>,
        left_count: u64,
        /// What a kept side's write-out did; nothing without one.
        applied: Applied,
        safety_copies: usize,
        warnings: Vec<Warning>,
    },
    /// Nothing fixed to record; what is left, with what each needs.
    Nothing {
        left: Vec<ConflictAt>,
        left_count: u64,
    },
    /// The save the project is on holds no conflict.
    NoConflicts { save_id: Id },
}

/// Copied out by `--copy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Copied {
    pub size: u64,
}

/// Why nothing was recorded, or written. Paths are untrusted, so messages escape them.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("You're at a save, not on a line, so there is no line to record fixes on.")]
    NotOnALine,
    #[error("{} isn't in conflict, so nothing was recorded.", escape(path, false))]
    NotInConflict { path: String },
    #[error("{}", still_yours(paths))]
    StillYours { paths: Vec<String> },
    #[error("{} still has {}, so nothing was recorded.", escape(path, false), marked(*left))]
    MarkersLeft { path: String, left: u64 },
    /// `copying`: met by `--copy`, whose hint differs.
    #[error(
        "{} is in conflict between more than two lines, so Opalro can't pick one side of it.",
        escape(path, false)
    )]
    CantPickSide { path: String, copying: bool },
    #[error(
        "{} is already there, so nothing was written.",
        escape(&to.display().to_string(), false)
    )]
    AlreadyThere { to: PathBuf },
    #[error("Opalro doesn't write into .opalro, so nothing was written.")]
    IntoOpalro,
    #[error("{}", no_file(path, *pick))]
    NoFile { path: String, pick: Pick },
    #[error("{} is a folder {}; --copy writes one file.", escape(path, false), on(*pick))]
    AFolder { path: String, pick: Pick },
    #[error("{} is a link {}; --copy writes one file.", escape(path, false), on(*pick))]
    ALink { path: String, pick: Pick },
    #[error(transparent)]
    Save(#[from] SaveError),
    #[error(transparent)]
    Switch(SwitchError),
    #[error(transparent)]
    Repo(#[from] RepoError),
}

impl From<StoreError> for ResolveError {
    fn from(e: StoreError) -> Self {
        ResolveError::Repo(e.into())
    }
}

impl From<io::Error> for ResolveError {
    fn from(e: io::Error) -> Self {
        ResolveError::Repo(e.into())
    }
}

impl From<SwitchError> for ResolveError {
    fn from(e: SwitchError) -> Self {
        match e {
            SwitchError::Save(e) => ResolveError::Save(e),
            SwitchError::Repo(e) => ResolveError::Repo(e),
            e => ResolveError::Switch(e),
        }
    }
}

impl ResolveError {
    /// What the user can do next, if there is something to say.
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        Some(match self {
            ResolveError::NotOnALine => "Make a line here first: opalro line new <name>".into(),
            ResolveError::NotInConflict { .. } => {
                "opalro status lists the places in conflict.".into()
            }
            ResolveError::StillYours { .. } => {
                "Say which to keep: opalro resolve <path> --keep mine, or --keep theirs. Or put \
                 the version you want in the folder and run opalro resolve <path>."
                    .into()
            }
            ResolveError::MarkersLeft { .. } => {
                "Fix them first, or run it again with --yes to record the file as it is.".into()
            }
            ResolveError::CantPickSide { copying: false, .. } => {
                "Put the version you want in the folder and run opalro resolve <path>.".into()
            }
            ResolveError::CantPickSide { copying: true, .. } => {
                "It names more than two versions; opalro status lists it.".into()
            }
            ResolveError::Save(e) => return e.hint(),
            ResolveError::Switch(e) => return e.hint(),
            ResolveError::Repo(e) => return e.hint(),
            _ => return None,
        })
    }
}

fn still_yours(paths: &[String]) -> String {
    const WHY: &str = "Opalro won't guess which one you want, so nothing was recorded";
    match paths {
        [one] => format!(
            "{} still holds your version, and {WHY}.",
            escape(one, false)
        ),
        _ => format!(
            "These still hold your version, and {WHY}:{}",
            crate::snapshot::listed(paths.iter().take(LISTED), paths.len() as u64)
        ),
    }
}

fn marked(left: u64) -> String {
    match left {
        0 => "marker lines".into(),
        1 => "1 marked place".into(),
        n => format!("{} marked places", thousands(n)),
    }
}

fn no_file(path: &str, pick: Pick) -> String {
    let path = escape(path, false);
    let rest = "so there is nothing to copy";
    match pick {
        Pick::Mine => format!("Your line holds no file at {path}, {rest}."),
        Pick::Theirs => format!("The other line holds no file at {path}, {rest}."),
        Pick::Before => format!("There was no file at {path} where the lines parted, {rest}."),
    }
}

fn on(pick: Pick) -> &'static str {
    match pick {
        Pick::Mine => "on your line",
        Pick::Theirs => "on the other line",
        Pick::Before => "where the lines parted",
    }
}

impl fmt::Display for Pick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Pick::Mine => "mine",
            Pick::Theirs => "theirs",
            Pick::Before => "before",
        })
    }
}

/// `pick`'s version in `r`: `None` when it has no single one, as a record of more than three
/// terms has none where the lines parted.
fn picked(r: &Record, pick: Pick) -> Option<Option<Term>> {
    match pick {
        Pick::Mine => Some(r.mine()),
        Pick::Theirs => Some(r.theirs()),
        Pick::Before => r.before(),
    }
}

/// Records fixes, under the lock and once a stopped switch is finished: into the line's newest
/// save when it is a combined save nothing else shows, else in a save of its own, as one diary
/// step. Without `keep`, each place is recorded as the folder holds it, through `save`'s own
/// walk and checks; with it, the side is put in the folder whole, as `goto` writes, a safety copy
/// kept first of what no save holds. If any place is refused, nothing is recorded.
pub fn resolve(
    repo: &mut Repository,
    req: &ResolveRequest,
    on_wait: &mut dyn FnMut(&str),
    ask: &mut dyn FnMut(&Ask) -> bool,
) -> Result<ResolveOutcome, ResolveError> {
    locked(repo, "resolve", on_wait, |repo, lock, earlier, _| {
        Ok(resolve_locked(repo, lock, earlier, req, ask))
    })?
}

fn resolve_locked(
    repo: &mut Repository,
    lock: &ChangeLock,
    earlier: &mut Earlier,
    req: &ResolveRequest,
    ask: &mut dyn FnMut(&Ask) -> bool,
) -> Result<ResolveOutcome, ResolveError> {
    let (head, entry) = repo.current()?;
    let Position::OnLine(line) = &entry.state.position else {
        return Err(ResolveError::NotOnALine);
    };
    let line = line.clone();
    let status = status::status(repo, false, req.git_excludes)?;
    let parent_id = status.save_id;
    if status.conflicts.is_empty() {
        return Ok(ResolveOutcome::NoConflicts { save_id: parent_id });
    }
    let drive = repo.drive()?;
    let chosen = choose(&status.conflicts, req, drive, ask)?;
    let places: Vec<String> = chosen.iter().map(|c| c.path.clone()).collect();
    let records: Vec<Id> = chosen.iter().map(|c| c.record).collect();
    let (left, left_count) = left(status.conflicts, &places);
    if places.is_empty() {
        return Ok(ResolveOutcome::Nothing { left, left_count });
    }
    let parent = status.save;
    let mut cache = ChangeIds::load(repo);
    let amend = parent.parents.len() > 1 && {
        let by = shown_elsewhere(repo, &mut cache, &entry.state, &line, parent_id, &parent);
        // Kept however it ends: reading the other lines is what costs.
        cache.store(repo, &entry.state);
        by?.is_empty()
    };
    let message = match places.as_slice() {
        _ if amend => parent.message.clone(),
        [one] => format!("Fixed conflicts in {one}"),
        many => format!("Fixed conflicts in {} places", thousands(many.len() as u64)),
    };
    let built = Built {
        line: &line,
        parent_id,
        parent: &parent,
        message: &message,
        amend,
    };
    let mut state = entry.state.clone();
    let (save_id, save, short_change_id, applied, safety_copies, warnings) = match req.keep {
        None => {
            let (save_id, save, short, mut warnings, packs, seen, now) =
                built.as_the_folder_holds(repo, req, &places, ask, &mut cache)?;
            state.lines.insert(line.clone(), save_id);
            let diary = DiaryEntry {
                parent: Some(head),
                action: Action::Command,
                command: req.command.to_owned(),
                who: req.who.clone(),
                opal_version: env!("CARGO_PKG_VERSION").into(),
                state: state.clone(),
                folder_change: None,
            };
            fsio::fault_point("resolve:built").map_err(RepoError::from)?;
            repo.commit(lock, packs, &diary)?;
            keep_file_cache(repo, seen, now, &mut warnings);
            (save_id, save, short, Applied::default(), 0, warnings)
        }
        Some(pick) => {
            let (save_id, save, short, mut warnings, packs) =
                built.keeping(repo, &places, &records, pick, req.who, &mut cache)?;
            // Stored as a combine stores its save before switching to it.
            repo.store_packs(lock, packs)?;
            fsio::fault_point("resolve:built").map_err(RepoError::from)?;
            let only: Vec<String> = places.clone();
            let plan = writeout::plan(
                repo,
                parent_id,
                save_id,
                Some(&only),
                true,
                drive,
                req.git_excludes,
            )?;
            state.lines.insert(line.clone(), save_id);
            let planned = Planned {
                head,
                before: &entry.state,
                state: state.clone(),
                from: parent_id,
                to: save_id,
                plan,
                drive,
            };
            let (applied, safety_copies, _) =
                make_switch(repo, lock, planned, req.who, req.command, earlier)?;
            warnings.splice(0..0, status.warnings);
            (save_id, save, short, applied, safety_copies, warnings)
        }
    };
    if amend {
        cache.replace(&line, parent_id, save_id);
    } else {
        cache.add(&line, parent_id, save_id, save.change_id);
    }
    cache.store(repo, &state);
    Ok(ResolveOutcome::Recorded {
        save_id,
        save,
        short_change_id,
        replaced: amend.then_some(parent_id),
        recorded: places,
        left,
        left_count,
        applied,
        safety_copies,
        warnings,
    })
}

/// The places to record, as stored, in path order. With paths, every place at or below each,
/// matched as the drive matches names; without, every place the folder shows fixed. Without
/// `keep`, a place is recorded only where the folder shows a decision: a version still standing
/// as yours, as the combine left it, is refused, and markers left are asked about.
fn choose<'c>(
    conflicts: &'c [ConflictAt],
    req: &ResolveRequest,
    drive: Drive,
    ask: &mut dyn FnMut(&Ask) -> bool,
) -> Result<Vec<&'c ConflictAt>, ResolveError> {
    let places: Vec<&ConflictAt> = match req.paths {
        None if req.keep.is_none() => conflicts
            .iter()
            .filter(|c| c.seen == Some(Seen::Fixed))
            .collect(),
        None => conflicts.iter().collect(),
        Some(paths) => {
            let folded: Vec<String> = conflicts
                .iter()
                .map(|c| fold_path(&c.path, drive))
                .collect();
            let mut named = BTreeSet::new();
            for path in paths {
                let top = fold_path(path, drive);
                let below: Vec<usize> = (0..conflicts.len())
                    .filter(|&i| within(&folded[i], &top))
                    .collect();
                if below.is_empty() {
                    return Err(ResolveError::NotInConflict { path: path.clone() });
                }
                named.extend(below);
            }
            named.into_iter().map(|i| &conflicts[i]).collect()
        }
    };
    if req.keep.is_some() {
        if let Some(c) = places.iter().find(|c| c.terms > 3) {
            return Err(ResolveError::CantPickSide {
                path: c.path.clone(),
                copying: false,
            });
        }
        return Ok(places);
    }
    if let Some(c) = places.iter().find(|c| c.seen.is_none()) {
        return Err(SaveError::KeptWhole {
            path: c.path.clone(),
        }
        .into());
    }
    let marked = |c: &ConflictAt| match c.standing {
        Standing::Marked { left } => Some(left),
        _ => None,
    };
    let still: Vec<String> = places
        .iter()
        .filter(|c| c.seen == Some(Seen::Untouched) && marked(c).is_none() && !c.has_own)
        .map(|c| c.path.clone())
        .collect();
    if !still.is_empty() {
        return Err(ResolveError::StillYours { paths: still });
    }
    for c in &places {
        if let Some(left) = marked(c)
            && !ask(&Ask::MarkersLeft {
                path: &c.path,
                left,
            })
        {
            return Err(ResolveError::MarkersLeft {
                path: c.path.clone(),
                left,
            });
        }
    }
    Ok(places)
}

/// The places met but not recorded: the first `LISTED`, and how many in all.
fn left(conflicts: Vec<ConflictAt>, recorded: &[String]) -> (Vec<ConflictAt>, u64) {
    let recorded: BTreeSet<&str> = recorded.iter().map(String::as_str).collect();
    let mut rest: Vec<ConflictAt> = conflicts
        .into_iter()
        .filter(|c| !recorded.contains(c.path.as_str()))
        .collect();
    let count = rest.len() as u64;
    rest.truncate(LISTED);
    (rest, count)
}

/// Where a resolve records: the line, its newest save, and whether the new save replaces it.
struct Built<'a> {
    line: &'a str,
    parent_id: Id,
    parent: &'a Save,
    message: &'a str,
    amend: bool,
}

type FromFolder = (
    Id,
    Save,
    String,
    Vec<Warning>,
    PackSet,
    crate::filecache::FileCache,
    Option<i128>,
);

impl Built<'_> {
    /// The save with `places` as the folder holds them, walked and checked as `save <paths>`
    /// walks and checks them, not yet committed.
    fn as_the_folder_holds(
        &self,
        repo: &Repository,
        req: &ResolveRequest,
        places: &[String],
        ask: &mut dyn FnMut(&Ask) -> bool,
        cache: &mut ChangeIds,
    ) -> Result<FromFolder, ResolveError> {
        let given: Vec<Given> = places.iter().map(|p| Given::Path(p.clone())).collect();
        let save_req = SaveRequest {
            message: Some(self.message),
            who: req.who,
            command: req.command,
            accept_crlf: req.accept_crlf,
            git_excludes: req.git_excludes,
            new_line: None,
            only: Some(&given),
            amend: self.amend,
        };
        let recording = Recording {
            fixed: places.iter().cloned().collect(),
        };
        // Its places come from the save itself, and one standing as nothing matches nothing in
        // the folder.
        let mut walked = read_and_build(
            repo,
            &save_req,
            self.parent,
            &recording,
            false,
            true,
            &mut |large| ask(&Ask::Large(large)),
        )?;
        let (move_notes, _) = walked.moves(repo);
        let (id, save, short) = compose(
            repo,
            self.line,
            self.parent_id,
            self.parent,
            walked.root,
            self.message,
            req.who,
            self.amend,
            move_notes,
            &mut walked.packs,
            cache,
            &mut walked.warnings,
        )?;
        Ok((
            id,
            save,
            short,
            walked.warnings,
            walked.packs,
            walked.seen,
            walked.now,
        ))
    }

    /// The save with each of `places` (whose records are `records`) replaced by `pick`'s
    /// version, read as stored: a folder whole. Its objects are in the packs it gives, to store.
    fn keeping(
        &self,
        repo: &Repository,
        places: &[String],
        records: &[Id],
        pick: Pick,
        who: &Person,
        cache: &mut ChangeIds,
    ) -> Result<(Id, Save, String, Vec<Warning>, PackSet), ResolveError> {
        let store = repo.store();
        let mut read = HashMap::new();
        let mut changes = BTreeMap::new();
        // The overlay builds a folder it is given from the changes below it; a folder term goes
        // in whole, so it is put as a leaf and made a folder again as its listing is stored.
        let mut whole = HashSet::new();
        for (path, id) in places.iter().zip(records) {
            let record = conflict::record_at(store, &mut read, path, id)?;
            let Some(term) = picked(record, pick) else {
                return Err(ResolveError::CantPickSide {
                    path: path.clone(),
                    copying: false,
                });
            };
            let put = term.map(|t| match t.kind {
                EntryKind::Folder => {
                    whole.insert(t.id);
                    (EntryKind::Symlink, t.id, 0)
                }
                kind => (kind, t.id, t.size),
            });
            changes.insert(path.clone(), put);
        }
        let unstored = HashMap::new();
        let listings = Listings::stored(store, &unstored);
        let mut packs = repo.new_packs()?;
        let mut held = HashMap::new();
        let root = overlay_with(&listings, self.parent.root, &changes, false, &mut |l| {
            let folder = |e: &Entry| e.kind == EntryKind::Symlink && whole.contains(&e.id);
            let l = if l.entries().iter().any(folder) {
                let entries = l.entries().iter().map(|e| Entry {
                    kind: if folder(e) { EntryKind::Folder } else { e.kind },
                    ..e.clone()
                });
                Listing::new(entries.collect()).map_err(StoreError::from)?
            } else {
                l.clone()
            };
            let id = store_listing(store, &mut packs, &l)?;
            held.insert(id, l);
            Ok(id)
        })?;
        // Moves noted as `save` notes them, against the save before.
        let before = match (self.amend, self.parent.parents.first()) {
            (true, Some(first)) => Some(store.save(first, repo.signatures_allowed())?.root),
            (true, None) => None,
            (false, _) => Some(self.parent.root),
        };
        let place = |o: Option<&Entry>, n: Option<&Entry>| {
            [o, n]
                .into_iter()
                .flatten()
                .any(|e| e.kind == EntryKind::Conflict)
        };
        let mut move_notes = before
            .and_then(|from| {
                moves::count_and_moves(store, &held, from, root, MAX_LEFT_BYTES, &place).ok()
            })
            .and_then(|(_, notes)| notes.ok().flatten())
            .unwrap_or_default();
        let most = usize::try_from(MAX_MOVE_NOTES).unwrap_or(usize::MAX);
        moves::first_that_fit(&mut move_notes, most, MAX_NOTED_BYTES);
        let mut warnings = Vec::new();
        let (id, save, short) = compose(
            repo,
            self.line,
            self.parent_id,
            self.parent,
            root,
            self.message,
            who,
            self.amend,
            move_notes,
            &mut packs,
            cache,
            &mut warnings,
        )?;
        Ok((id, save, short, warnings, packs))
    }
}

/// Writes `pick`'s version of the file in conflict at `path` to `to`, which must not exist and
/// must not be inside `.opalro`; records nothing and takes no lock.
pub fn copy_version(
    repo: &Repository,
    path: &str,
    pick: Pick,
    to: &Path,
) -> Result<Copied, ResolveError> {
    let not_in_conflict = || ResolveError::NotInConflict {
        path: path.to_owned(),
    };
    if !repo.keeps_conflicts() {
        return Err(not_in_conflict());
    }
    let (_, entry) = repo.current()?;
    let root = repo
        .store()
        .save(&tip_of(&entry)?, repo.signatures_allowed())?
        .root;
    let drive = repo.drive()?;
    let (place, id) = conflict_on(repo.store(), root, path, drive)?.ok_or_else(not_in_conflict)?;
    let record = conflict::record_at(repo.store(), &mut HashMap::new(), &place, &id)?.clone();
    if record.terms.len() > 3 {
        return Err(ResolveError::CantPickSide {
            path: place,
            copying: true,
        });
    }
    let term = match picked(&record, pick).flatten() {
        None => return Err(ResolveError::NoFile { path: place, pick }),
        Some(t) if t.kind == EntryKind::Folder => {
            return Err(ResolveError::AFolder { path: place, pick });
        }
        Some(t) if matches!(t.kind, EntryKind::Symlink | EntryKind::ProjectLink) => {
            return Err(ResolveError::ALink { path: place, pick });
        }
        Some(t) => t,
    };
    let at = destination(repo, to, drive)?;
    let size = fsio::write_new(&at, |file| {
        repo.store()
            .copy_content(&term.id, file)
            .map_err(ResolveError::from)
    })
    .map_err(|e| match e {
        ResolveError::Repo(RepoError::Io(e)) if e.kind() == io::ErrorKind::AlreadyExists => {
            ResolveError::AlreadyThere { to: to.to_owned() }
        }
        e => e,
    })?;
    Ok(Copied { size })
}

/// The conflict at `path` in the save whose top listing is `root`, read along that path only:
/// its place as stored and its record's ID. A name typed in another spelling the drive takes as
/// the same finds the stored one, when only one is.
fn conflict_on(
    store: &Store,
    root: Id,
    path: &str,
    drive: Drive,
) -> Result<Option<(String, Id)>, ResolveError> {
    let unstored = HashMap::new();
    let listings = Listings::stored(store, &unstored);
    let names: Vec<&str> = path.split('/').filter(|n| !n.is_empty()).collect();
    let (mut at, mut place) = (root, String::new());
    for (i, name) in names.iter().enumerate() {
        let listing = listings.read(&at)?;
        let found = listing.get(name).or_else(|| {
            let want = drive.compared(name);
            let mut alike = listing
                .entries()
                .iter()
                .filter(|e| drive.compared(&e.name) == want);
            match (alike.next(), alike.next()) {
                (Some(e), None) => Some(e),
                _ => None,
            }
        });
        let Some(e) = found else {
            return Ok(None);
        };
        if !place.is_empty() {
            place.push('/');
        }
        place.push_str(&e.name);
        match e.kind {
            EntryKind::Conflict if i + 1 == names.len() => return Ok(Some((place, e.id))),
            EntryKind::Folder if i + 1 < names.len() => at = e.id,
            _ => return Ok(None),
        }
    }
    Ok(None)
}

/// `to` with the folder it is in resolved as the system resolves it (links, `..`), refused when
/// that is `.opalro` or inside it, compared as the drive compares names.
fn destination(repo: &Repository, to: &Path, drive: Drive) -> Result<PathBuf, ResolveError> {
    let already = || ResolveError::AlreadyThere { to: to.to_owned() };
    let name = match to.components().next_back() {
        Some(Component::Normal(name)) => name,
        // `..`, `.` or a root names a folder that is there.
        _ => return Err(already()),
    };
    let up = match to.parent() {
        Some(up) if !up.as_os_str().is_empty() => up,
        _ => Path::new("."),
    };
    let up = std::fs::canonicalize(up).map_err(RepoError::from)?;
    let opal = std::fs::canonicalize(repo.root().join(".opalro")).map_err(RepoError::from)?;
    let folded = |p: &Path| -> Vec<String> {
        p.components()
            .map(|c| drive.compared(&opened_as(&c.as_os_str().to_string_lossy())))
            .collect()
    };
    let at = up.join(name);
    if folded(&at).starts_with(&folded(&opal)) {
        return Err(ResolveError::IntoOpalro);
    }
    Ok(at)
}

/// The name Windows opens for `name`: a stream after `:` and trailing dots and spaces don't count.
fn opened_as(name: &str) -> String {
    if !cfg!(windows) {
        return name.to_owned();
    }
    let name = name.split(':').next().unwrap_or(name);
    match name.trim_end_matches(['.', ' ']) {
        "" => name.to_owned(),
        trimmed => trimmed.to_owned(),
    }
}

#[cfg(test)]
mod tests;
