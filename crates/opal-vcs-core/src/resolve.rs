//! `opalro resolve`: records the places in conflict the person fixed, or a side kept whole, and
//! writes one version of a file in conflict out to look at.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::path::{Component, Path, PathBuf};

use opal_platform::dir::Dir;

use crate::changeids::ChangeIds;
use crate::conflict::{self, Listings, Record, Recording, Seen, Term};
use crate::diary::Position;
use crate::drive::Drive;
use crate::fsio;
use crate::history::tip_of;
use crate::id::Id;
use crate::kept::overlay_with;
use crate::listing::{Entry, EntryKind, Listing};
use crate::names::Given;
use crate::pack::PackSet;
use crate::repo::{ChangeLock, RepoError, Repository};
use crate::save::{Person, Save};
use crate::scope::within;
use crate::snapshot::{
    LargeSave, Made, SaveError, SaveRequest, base_limits, compose, keep_file_cache, notes_against,
    read_and_build, save_before, shown_elsewhere, store_listing,
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

/// A side `--keep` can take whole: yours or theirs, never where the lines parted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Mine,
    Theirs,
}

impl From<Side> for Pick {
    fn from(side: Side) -> Pick {
        match side {
            Side::Mine => Pick::Mine,
            Side::Theirs => Pick::Theirs,
        }
    }
}

pub struct ResolveRequest<'a> {
    /// Places named, a folder standing for every place below it; `None` records every fixed one.
    pub paths: Option<&'a [String]>,
    /// The side kept whole at each place named; refused without paths.
    pub keep: Option<Side>,
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
    #[error("Keeping a side needs the places to keep it at, so nothing was recorded.")]
    KeepNeedsPlaces,
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
            ResolveError::KeepNeedsPlaces => {
                "Name them: opalro resolve <path> --keep mine (or theirs).".into()
            }
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
    // A side kept at every place in conflict is never what anyone asked for.
    if req.keep.is_some() && req.paths.is_none() {
        return Err(ResolveError::KeepNeedsPlaces);
    }
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
    // What is recorded must still be what was looked at, the question's wait included.
    fsio::act_point("resolve:chosen");
    let places: Vec<String> = chosen.iter().map(|c| c.path.clone()).collect();
    let records: Vec<Id> = chosen.iter().map(|c| c.record).collect();
    let chosen_now: Vec<(String, Option<(EntryKind, Id)>)> =
        chosen.iter().map(|c| (c.path.clone(), c.now)).collect();
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
    let (save_id, save, short_change_id, applied, safety_copies, warnings) = match req.keep {
        None => {
            let decided_on = chosen_now.into_iter().collect();
            let (save_id, save, short, mut warnings, packs, seen, now) =
                built.as_the_folder_holds(repo, req, &places, decided_on, ask, &mut cache)?;
            fsio::fault_point("resolve:built").map_err(RepoError::from)?;
            built.made(save_id, &save).commit(
                repo,
                lock,
                head,
                &entry.state,
                req.who,
                req.command,
                packs,
                &mut cache,
            )?;
            keep_file_cache(repo, seen, now, &mut warnings);
            (save_id, save, short, Applied::default(), 0, warnings)
        }
        Some(side) => {
            let (save_id, save, short, mut warnings, packs) =
                built.keeping(repo, &places, &records, side.into(), req.who, &mut cache)?;
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
            let made = built.made(save_id, &save);
            let state = made.state(&entry.state);
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
            made.learn(repo, &mut cache, &state);
            warnings.splice(0..0, status.warnings);
            (save_id, save, short, applied, safety_copies, warnings)
        }
    };
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
        None => conflicts
            .iter()
            .filter(|c| c.seen == Some(Seen::Fixed))
            .collect(),
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
    /// The save `save_id` made here.
    fn made<'s>(&'s self, save_id: Id, save: &Save) -> Made<'s> {
        Made {
            line: self.line,
            parent_id: self.parent_id,
            save_id,
            change_id: save.change_id,
            amend: self.amend,
        }
    }

    /// The save with `places` as the folder holds them, walked and checked as `save <paths>`
    /// walks and checks them, not yet committed.
    fn as_the_folder_holds(
        &self,
        repo: &Repository,
        req: &ResolveRequest,
        places: &[String],
        decided_on: BTreeMap<String, Option<(EntryKind, Id)>>,
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
            decided_on,
            ..Recording::default()
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
        // Held to a save's limits as the next save reads it, each conflict as what it stands as,
        // before anything is stored: a side kept whole may be larger than what stood there.
        let read = |id: &Id| match held.get(id) {
            Some(l) => Ok(l.clone()),
            None => store.listing(id),
        };
        let stand = |id: &Id| {
            store
                .met_conflict(id)
                .map(|r| conflict::stands_as(&r))
                .map_err(|e| conflict::unreadable("in the new save", id, e))
        };
        if let Err(detail) = base_limits().add_all_standing(root, 0, 0, read, stand)? {
            return Err(SaveError::TooLarge { detail }.into());
        }
        let before = save_before(repo, self.amend, self.parent);
        let (move_notes, _) = notes_against(repo, &before, Some(&held), root);
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
    // The file is made in the very folder that was checked.
    let (dir, name) = destination(repo, to, drive)?;
    let size = fsio::write_new(&dir, &name, |file| {
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

/// The folder `to` is in, opened once, and the name to make there, refused when that folder is
/// `.opalro` or inside it, or the name is `.opalro`'s. Folders are told apart by identity (the
/// drive's number and the folder's), so another name for `.opalro` is caught; the resolved path
/// is compared by spelling too, as the drive compares names, for what Windows opens as `.opalro`
/// (a stream on it, trailing dots and spaces).
fn destination(
    repo: &Repository,
    to: &Path,
    drive: Drive,
) -> Result<(Dir, OsString), ResolveError> {
    let name = match to.components().next_back() {
        Some(Component::Normal(name)) => name,
        // `..`, `.` or a root names a folder that is there.
        _ => return Err(ResolveError::AlreadyThere { to: to.to_owned() }),
    };
    let up = match to.parent() {
        Some(up) if !up.as_os_str().is_empty() => up,
        _ => Path::new("."),
    };
    let up = std::fs::canonicalize(up)?;
    let opal_path = std::fs::canonicalize(repo.root().join(".opalro"))?;
    let opal = Dir::open_top(&opal_path)?.own_meta()?.file_id;
    let dir = Dir::open_top(&up)?;
    let mut inside = dir.own_meta()?.file_id == opal;
    // A folder above that can't be opened isn't `.opalro`, which was just opened.
    for above in up.ancestors().skip(1) {
        inside = inside
            || Dir::open_top(above)
                .and_then(|d| d.own_meta())
                .is_ok_and(|m| m.file_id == opal);
    }
    let opened = opened_as(&name.to_string_lossy());
    let named = dir
        .metadata(OsStr::new(&opened))
        .is_ok_and(|m| m.file_id == opal);
    let folded = |p: &Path| -> Vec<String> {
        p.components()
            .map(|c| drive.compared(&opened_as(&c.as_os_str().to_string_lossy())))
            .collect()
    };
    let spelled = folded(&up.join(name)).starts_with(&folded(&opal_path));
    if inside || named || spelled {
        return Err(ResolveError::IntoOpalro);
    }
    Ok((dir, name.to_owned()))
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
