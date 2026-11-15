//! `opalro status`: what changed in the project folder since the save the project is on.

use std::collections::HashMap;
use std::collections::hash_map::Entry as Slot;
use std::ffi::OsStr;
use std::io;
use std::path::Path;

use crate::conflict::{self, Left, MIN_MARKER, Record, Seen, markers, stands_as};
use crate::diary::Position;
use crate::diff::{ChangeKind, FileChange, change_at, for_each_entry_change_in};
use crate::history::tip_of;
use crate::id::Id;
use crate::listing::{Entry, EntryKind};
use crate::repo::Repository;
use crate::save::Save;
use crate::snapshot::{FolderNow, SaveError, kept_in_the_way, read_folder};
use crate::store::{Store, StoreError};
use crate::text::escape;
use crate::warning::Warning;
use crate::writeout::{self, Folders};

/// What changed since the current save.
#[derive(Debug)]
pub struct Status {
    pub position: Position,
    /// The save the folder was compared with, and its ID.
    pub save_id: Id,
    pub save: Save,
    /// The files that differ from it, in path order.
    pub changes: Vec<PathChange>,
    /// Entries of the save this computer can't hold as they are, by path: saves keep them as
    /// they were.
    pub not_written_here: Vec<String>,
    /// What a stopped switch left to finish, if one did.
    pub unfinished_switch: Option<String>,
    pub warnings: Vec<Warning>,
    /// Every place in conflict the comparison met, in path order.
    pub conflicts: Vec<ConflictAt>,
}

/// A place in conflict, and how it stands in the folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictAt {
    pub path: String,
    pub record: Id,
    pub standing: Standing,
    /// How many versions the record names.
    pub terms: usize,
    /// What the folder shows there, and whether the record has a version of its own: what
    /// `resolve` decides by, which `standing` alone can't tell apart.
    /// `None` for a place this computer can't hold, which the folder can't show.
    pub(crate) seen: Option<Seen>,
    pub(crate) has_own: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// Shown with markers, and markers are left: `left` opening lines (0 when only closing ones are).
    Marked { left: u64 },
    /// Shown with markers, none left: fixed, not yet recorded.
    Fixed,
    /// Your line's version, as the combine left it.
    Yours,
    /// Stands as a version, and the folder holds something else, or a saved change of the person's.
    ChangedInPlace,
    /// Your line has nothing there, and neither has the folder.
    NotInFolder,
    /// In a part of the save this computer can't hold, kept as it is.
    NotHere,
}

/// A file that differs from the save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathChange {
    pub path: String,
    pub change: ChangeKind,
    /// The kind now, or before for a removed file.
    pub kind: EntryKind,
    /// The kind before, if the save has a file there.
    pub old_kind: Option<EntryKind>,
    pub old_size: Option<u64>,
    pub new_size: Option<u64>,
    /// What each side holds, if it has a file.
    pub old_id: Option<Id>,
    pub new_id: Option<Id>,
    /// The path is a place in conflict, or below one standing as a folder.
    pub in_conflict: bool,
}

impl From<FileChange<'_>> for PathChange {
    fn from(c: FileChange<'_>) -> PathChange {
        PathChange {
            path: c.path.to_owned(),
            change: c.change,
            kind: c.kind,
            old_kind: c.old_kind,
            old_size: c.old_size,
            new_size: c.new_size,
            old_id: c.old_id,
            new_id: c.new_id,
            in_conflict: false,
        }
    }
}

/// What differs between a save and the folder at one path.
pub(crate) enum Difference<'a> {
    File(FileChange<'a>),
    /// A place in conflict: its record's ID and record, and the folder's entry there.
    Conflict {
        path: &'a str,
        id: Id,
        record: &'a Record,
        now: Option<&'a Entry>,
    },
}

/// Hands `each` every file that differs between the save whose top listing is `root` and the
/// folder as `now` read it, and every place in conflict met on the way, in path order. What is
/// below a place standing as a folder is that place's, so it comes with it and not on its own.
/// A record is read once, into `records`.
pub(crate) fn each_difference<E: From<StoreError>>(
    repo: &Repository,
    now: &FolderNow,
    records: &mut HashMap<Id, Record>,
    root: Id,
    mut each: impl FnMut(Difference<'_>) -> Result<(), E>,
) -> Result<(), E> {
    let (store, keeps) = (repo.store(), repo.keeps_conflicts());
    // The places with a folder on either side whose paths may still come, each with a `/` after
    // it: what is below them is theirs.
    let mut folders: Vec<String> = Vec::new();
    for_each_entry_change_in(store, &now.listings, Some(root), now.root, |path, o, n| {
        folders.retain(|f| path.starts_with(f.as_str()) || path < f.as_str());
        if folders.iter().any(|f| path.starts_with(f.as_str())) {
            return Ok(());
        }
        let Some(o) = o.filter(|o| keeps && o.kind == EntryKind::Conflict) else {
            return change_at(path, o, n).map_or(Ok(()), |c| each(Difference::File(c)));
        };
        let record = record_at(store, records, path, &o.id)?;
        let folder = |kind: Option<EntryKind>| kind == Some(EntryKind::Folder);
        if folder(stands_as(record).map(|t| t.kind)) || folder(n.map(|e| e.kind)) {
            folders.push(format!("{path}/"));
        }
        each(Difference::Conflict {
            path,
            id: o.id,
            record,
            now: n,
        })
    })
}

/// The record `id` of the conflict at `path`, from `records` or else the store, once.
fn record_at<'r>(
    store: &Store,
    records: &'r mut HashMap<Id, Record>,
    path: &str,
    id: &Id,
) -> Result<&'r Record, StoreError> {
    Ok(match records.entry(*id) {
        Slot::Occupied(r) => r.into_mut(),
        Slot::Vacant(v) => {
            let at = conflict::at_path(path);
            v.insert(
                store
                    .conflict(id)
                    .map_err(|e| conflict::unreadable(&at, id, e))?,
            )
        }
    })
}

/// The marker lines of length `len` in the file at `path`, read through `disk`, opened when
/// first needed.
fn markers_at(disk: &mut Option<Folders>, root: &Path, path: &str, len: u8) -> io::Result<Left> {
    let disk = match disk {
        Some(disk) => disk,
        None => disk.insert(Folders::open(root)?),
    };
    let (up, name) = path.rsplit_once('/').unwrap_or(("", path));
    disk.enter(up)?;
    crate::fsio::open_point(path)?;
    let mut file = disk.here().open_file(OsStr::new(name))?;
    markers::scan(&mut file, len)
}

/// How the place in conflict at `path` stands, the folder holding `now` there.
fn conflict_at(
    disk: &mut Option<Folders>,
    root: &Path,
    (path, id, record): (&str, Id, &Record),
    now: Option<&Entry>,
) -> Result<ConflictAt, SaveError> {
    let unreadable = |source| SaveError::Unreadable {
        path: path.to_owned(),
        source,
    };
    let now = now.map(|e| (e.kind, e.id));
    let seen =
        conflict::seen(record, now, |len| markers_at(disk, root, path, len)).map_err(unreadable)?;
    let marker = record.marker();
    let standing = match seen {
        Seen::Untouched if marker >= MIN_MARKER => Standing::Marked {
            left: markers_at(disk, root, path, marker)
                .map_err(unreadable)?
                .opening,
        },
        Seen::Untouched => match (record.own, stands_as(record)) {
            (Some(_), _) => Standing::ChangedInPlace,
            (None, Some(_)) => Standing::Yours,
            (None, None) => Standing::NotInFolder,
        },
        Seen::MarkersLeft(left) => Standing::Marked { left: left.opening },
        Seen::Fixed => Standing::Fixed,
        Seen::ChangedInPlace => Standing::ChangedInPlace,
    };
    Ok(ConflictAt {
        path: path.to_owned(),
        record: id,
        standing,
        terms: record.terms.len(),
        seen: Some(seen),
        has_own: record.own.is_some(),
    })
}

/// Why `status` stopped, in its own words where the save's would talk of saving: a name a save
/// can't hold, something in the way of an entry kept as it was, or a project past a save's limits.
#[must_use]
pub fn why_stopped(e: &SaveError) -> String {
    let why = match e {
        SaveError::InvalidName { path, reason } => format!("{}: {reason}", escape(path, false)),
        SaveError::InTheWay { path, kept } => {
            format!("{}: {}", escape(path, false), kept_in_the_way(kept))
        }
        SaveError::TooLarge { detail } => format!("this project is too large for Opalro: {detail}"),
        other => return other.to_string(),
    };
    format!("Opalro can't show the status: {why}.")
}

/// Compares the project folder with the current save, walking it as a save would. Only files the
/// file-state cache doesn't vouch for are read, every file with `full`. Nothing is stored; only
/// the cache is written. `git_excludes` is the user's own Git ignore list, as for a save.
pub fn status(
    repo: &Repository,
    full: bool,
    git_excludes: Option<&Path>,
) -> Result<Status, SaveError> {
    let (_, entry) = repo.current()?;
    let save_id = tip_of(&entry)?;
    let save = repo.store().save(&save_id, repo.signatures_allowed())?;
    let mut now = read_folder(repo, save.root, full, git_excludes, None)
        .map_err(|e| e.in_save(save_id, true))?;
    let mut warnings = std::mem::take(&mut now.warnings);
    let unfinished = writeout::unfinished_warning(repo);
    let unfinished_switch = unfinished.as_ref().map(|w| w.message.clone());
    warnings.extend(unfinished);
    let mut changes = Vec::new();
    let mut conflicts = Vec::new();
    let mut records = std::mem::take(&mut now.records);
    let mut disk = None;
    each_difference(repo, &now, &mut records, save.root, |d| {
        match d {
            Difference::File(c) => changes.push(PathChange::from(c)),
            Difference::Conflict {
                path,
                id,
                record,
                now,
            } => conflicts.push(conflict_at(
                &mut disk,
                repo.root(),
                (path, id, record),
                now,
            )?),
        }
        Ok::<(), SaveError>(())
    })
    .map_err(|e| e.in_save(save_id, true))?;
    for (path, id) in &now.kept_conflicts {
        let record = record_at(repo.store(), &mut records, path, id)?;
        conflicts.push(ConflictAt {
            path: path.clone(),
            record: *id,
            standing: Standing::NotHere,
            terms: record.terms.len(),
            seen: None,
            has_own: record.own.is_some(),
        });
    }
    conflicts.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Status {
        position: entry.state.position,
        save_id,
        save,
        changes,
        not_written_here: now.not_written_here,
        unfinished_switch,
        warnings,
        conflicts,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::conflict::tests::{file_of, plant_conflicted};
    use crate::conflict::{Own, Record, Term};
    use crate::save::Person;
    use crate::test_support::{plant_save, without_conflicts_feature};
    use std::path::Path;

    pub(crate) fn project() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let who = Person {
            name: "Ada".into(),
            email: "ada@example.com".into(),
            time_ms: 1_700_000_000_000,
            tz_offset_min: 0,
        };
        let mut repo = Repository::init(dir.path(), &who).unwrap();
        repo.set_cache_key([7; 32]);
        (dir, repo)
    }

    pub(crate) fn write(root: &Path, rel: &str, bytes: &[u8]) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    pub(crate) const MARKED: &[u8] =
        b"a\n<<<<<<< main (kkkk)\nmine\n||||||| where the lines parted (llll)\n\
        base\n=======\ntheirs\n>>>>>>> side (mmmm)\nb\n<<<<<<< main (kkkk)\nmine 2\n\
        ||||||| where the lines parted (llll)\n=======\ntheirs 2\n>>>>>>> side (mmmm)\n";
    pub(crate) const HALF: &[u8] = b"a\nmine\nb\n<<<<<<< main (kkkk)\nmine 2\n\
        ||||||| where the lines parted (llll)\n=======\ntheirs 2\n>>>>>>> side (mmmm)\n";

    /// Yours, where the lines parted, theirs: three files the store holds once `keep` has run.
    pub(crate) fn versions(mine: Option<Term>) -> Record {
        Record {
            terms: vec![mine, Some(file_of(b"base")), Some(file_of(b"theirs"))],
            own: None,
        }
    }

    pub(crate) fn marked() -> Record {
        Record {
            own: Some(Own {
                version: Some(file_of(MARKED)),
                marker: 7,
            }),
            ..versions(Some(file_of(b"mine\n")))
        }
    }

    /// Plants a save holding `bytes` so the store has them.
    pub(crate) fn keep(repo: &mut Repository, bytes: &[&[u8]]) {
        let names: Vec<String> = (0..bytes.len()).map(|n| format!("kept{n}")).collect();
        let entries: Vec<(&str, EntryKind, &[u8])> = names
            .iter()
            .zip(bytes)
            .map(|(n, b)| (n.as_str(), EntryKind::File, *b))
            .collect();
        plant_save(repo, &entries);
    }

    fn rows(s: &Status) -> Vec<(&str, Standing)> {
        s.conflicts
            .iter()
            .map(|c| (c.path.as_str(), c.standing))
            .collect()
    }

    fn changed(s: &Status) -> Vec<&str> {
        s.changes.iter().map(|c| c.path.as_str()).collect()
    }

    fn status_of(repo: &Repository) -> Status {
        status(repo, false, None).unwrap()
    }

    #[test]
    fn status_lists_conflicts_with_how_each_stands() {
        let (dir, mut repo) = project();
        let all: &[&[u8]] = &[b"mine\n", b"base", b"theirs", MARKED, b"picture"];
        keep(&mut repo, all);
        let root = dir.path();
        let picture = versions(Some(file_of(b"picture")));
        let conflicts = [
            ("changed.png", picture.clone()),
            ("fixed.txt", marked()),
            ("gone.png", versions(None)),
            ("half.txt", marked()),
            ("marked.txt", marked()),
            ("yours.png", picture),
        ];
        plant_conflicted(&mut repo, &[], &conflicts);
        write(root, "changed.png", b"painted over");
        write(root, "fixed.txt", b"a\nmine\nb\nmine 2\n");
        write(root, "half.txt", HALF);
        write(root, "marked.txt", MARKED);
        write(root, "yours.png", b"picture");
        let s = status_of(&repo);
        assert_eq!(
            rows(&s),
            [
                ("changed.png", Standing::ChangedInPlace),
                ("fixed.txt", Standing::Fixed),
                ("gone.png", Standing::NotInFolder),
                ("half.txt", Standing::Marked { left: 1 }),
                ("marked.txt", Standing::Marked { left: 2 }),
                ("yours.png", Standing::Yours),
            ]
        );
        assert!(s.conflicts.iter().all(|c| c.terms == 3));
        assert!(s.changes.is_empty(), "{:?}", s.changes);
        // A version of its own, as it stands: changed in place, though nothing changed since.
        let own = Record {
            own: Some(Own {
                version: Some(file_of(b"theirs")),
                marker: 0,
            }),
            ..versions(Some(file_of(b"picture")))
        };
        plant_conflicted(&mut repo, &[], &[("kept.png", own)]);
        for gone in [
            "changed.png",
            "fixed.txt",
            "half.txt",
            "marked.txt",
            "yours.png",
        ] {
            std::fs::remove_file(root.join(gone)).unwrap();
        }
        write(root, "kept.png", b"theirs");
        let s = status_of(&repo);
        assert_eq!(rows(&s), [("kept.png", Standing::ChangedInPlace)]);
        assert!(s.conflicts[0].has_own && s.conflicts[0].seen == Some(Seen::Untouched));
    }

    #[test]
    fn a_conflicts_place_is_not_listed_among_the_changes() {
        let (dir, mut repo) = project();
        keep(&mut repo, &[b"mine\n", b"base", b"theirs"]);
        let files: &[(&str, EntryKind, &[u8])] = &[("plain.txt", EntryKind::File, b"one")];
        plant_conflicted(&mut repo, files, &[("c.txt", versions(None))]);
        write(dir.path(), "plain.txt", b"two");
        write(dir.path(), "c.txt", b"new");
        let s = status_of(&repo);
        assert_eq!(changed(&s), ["plain.txt"]);
        assert_eq!(rows(&s), [("c.txt", Standing::ChangedInPlace)]);
    }

    #[test]
    fn a_change_at_a_conflicts_place_shows_in_its_row() {
        let (dir, mut repo) = project();
        keep(&mut repo, &[b"picture", b"base", b"theirs"]);
        let c = versions(Some(file_of(b"picture")));
        plant_conflicted(&mut repo, &[], &[("art/hero.png", c)]);
        write(dir.path(), "art/hero.png", b"picture, painted over");
        let s = status_of(&repo);
        assert!(s.changes.is_empty(), "{:?}", s.changes);
        assert_eq!(rows(&s), [("art/hero.png", Standing::ChangedInPlace)]);
    }

    #[test]
    fn status_lists_every_conflict() {
        let (dir, mut repo) = project();
        keep(&mut repo, &[b"picture", b"base", b"theirs"]);
        let c = versions(Some(file_of(b"picture")));
        let names: Vec<String> = (0..150).map(|n| format!("c{n:03}.png")).collect();
        let conflicts: Vec<(&str, Record)> =
            names.iter().map(|n| (n.as_str(), c.clone())).collect();
        plant_conflicted(&mut repo, &[], &conflicts);
        for n in &names {
            write(dir.path(), n, b"picture");
        }
        let asked = repo.store().conflicts_read();
        let s = status_of(&repo);
        // One record at 150 places is read once.
        assert_eq!(repo.store().conflicts_read() - asked, 1);
        assert_eq!(s.conflicts.len(), 150);
        assert!(s.conflicts.iter().all(|c| c.standing == Standing::Yours));
        let paths: Vec<&str> = s.conflicts.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(paths, names.iter().map(String::as_str).collect::<Vec<_>>());
    }

    #[test]
    fn status_lists_a_conflict_in_a_folder_gone_from_disk() {
        let (_dir, mut repo) = project();
        keep(&mut repo, &[b"picture", b"base", b"theirs"]);
        let c = versions(Some(file_of(b"picture")));
        plant_conflicted(&mut repo, &[], &[("art/deep/hero.png", c)]);
        let s = status_of(&repo);
        assert_eq!(rows(&s), [("art/deep/hero.png", Standing::ChangedInPlace)]);
        assert!(s.changes.is_empty(), "{:?}", s.changes);
    }

    #[test]
    fn status_lists_a_conflict_kept_as_it_was() {
        let drive =
            crate::drive::probe(&opal_platform::dir::Dir::open_top(&std::env::temp_dir()).unwrap())
                .unwrap();
        if !drive.ignores_case {
            println!("Skipped: this drive holds names alike but for letter case as two files.");
            return;
        }
        let (dir, mut repo) = project();
        keep(&mut repo, &[b"picture", b"base", b"theirs"]);
        let c = versions(Some(file_of(b"picture")));
        let mut conflicts = vec![("Pic.png", c.clone()), ("Dir/c.png", c.clone())];
        if cfg!(windows) {
            conflicts.push(("aux.txt", c));
        }
        let files: &[(&str, EntryKind, &[u8])] = &[
            ("pic.png", EntryKind::File, b"picture"),
            ("dir/x.txt", EntryKind::File, b"x"),
        ];
        plant_conflicted(&mut repo, files, &conflicts);
        write(dir.path(), "pic.png", b"picture");
        let s = status_of(&repo);
        let mut want = vec![
            ("Dir/c.png", Standing::NotHere),
            ("Pic.png", Standing::NotHere),
        ];
        if cfg!(windows) {
            want.push(("aux.txt", Standing::NotHere));
        }
        assert_eq!(rows(&s), want);
    }

    #[test]
    fn an_ignored_folder_a_conflict_stands_as_is_treated_as_an_ignored_saved_folder() {
        let rules: &[u8] = b"build/\n";
        // Once a plain saved folder, once a conflict standing as that folder.
        let mut seen = Vec::new();
        for as_conflict in [false, true] {
            let (dir, mut repo) = project();
            let root = dir.path();
            let held = plant_save(&mut repo, &[("build/x.txt", EntryKind::File, b"x")]);
            let build = Term {
                kind: EntryKind::Folder,
                id: held.get("build").unwrap().id,
                size: 0,
            };
            keep(&mut repo, &[b"base", b"theirs"]);
            let files: &[(&str, EntryKind, &[u8])] = &[
                (".opalroignore", EntryKind::File, rules),
                ("build/x.txt", EntryKind::File, b"x"),
            ];
            if as_conflict {
                plant_conflicted(&mut repo, &files[..1], &[("build", versions(Some(build)))]);
            } else {
                plant_save(&mut repo, files);
            }
            write(root, ".opalroignore", rules);
            write(root, "build/x.txt", b"x");
            write(root, "build/new.txt", b"new, and ignored");
            let before = status_of(&repo);
            write(root, "build/x.txt", b"x, changed");
            let after = status_of(&repo);
            seen.push((
                changed(&before).len(),
                rows(&before)
                    .into_iter()
                    .map(|(_, s)| s)
                    .collect::<Vec<_>>(),
                changed(&after)
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
                rows(&after).into_iter().map(|(_, s)| s).collect::<Vec<_>>(),
            ));
        }
        // Nothing new is taken from it, and a change to what was saved there is met.
        assert_eq!(seen[0], (0, vec![], vec!["build/x.txt".to_owned()], vec![]));
        assert_eq!(
            seen[1],
            (
                0,
                vec![Standing::Yours],
                vec![],
                vec![Standing::ChangedInPlace]
            )
        );
    }

    #[cfg(feature = "fault-injection")]
    #[test]
    fn an_untouched_conflict_file_is_vouched_for_once_read() {
        let (dir, mut repo) = project();
        keep(&mut repo, &[b"picture", b"base", b"theirs"]);
        let c = versions(Some(file_of(b"picture")));
        plant_conflicted(&mut repo, &[], &[("hero.png", c)]);
        write(dir.path(), "hero.png", b"picture");
        // Old enough for the cache to trust.
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(dir.path().join("hero.png"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert_eq!(rows(&status_of(&repo)), [("hero.png", Standing::Yours)]);
        crate::fsio::fault::unreadable(Some("hero.png"));
        let again = status(&repo, false, None);
        crate::fsio::fault::unreadable(None);
        assert_eq!(rows(&again.unwrap()), [("hero.png", Standing::Yours)]);
        // Unvouched, it is read: the fault is met.
        crate::fsio::fault::unreadable(Some("hero.png"));
        let full = status(&repo, true, None);
        crate::fsio::fault::unreadable(None);
        assert!(full.is_err());
    }

    /// Sets the file at `rel` back a minute, so the file cache trusts what it learns of it.
    #[cfg(feature = "fault-injection")]
    fn age(root: &Path, rel: &str) {
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        let file = std::fs::File::options().write(true).open(root.join(rel));
        file.unwrap().set_modified(old).unwrap();
    }

    #[cfg(feature = "fault-injection")]
    #[test]
    fn a_marked_text_is_read_once_a_status_to_count_its_places() {
        use crate::fsio::{fault, opened};
        let (dir, mut repo) = project();
        let root = dir.path();
        keep(&mut repo, &[b"mine\n", b"base", b"theirs", MARKED]);
        plant_conflicted(&mut repo, &[], &[("story.txt", marked())]);
        write(root, "story.txt", MARKED);
        age(root, "story.txt");
        status_of(&repo);
        // The cache vouches for it, so only the count of its places reads it, once.
        let before = opened("story.txt");
        let s = status_of(&repo);
        assert_eq!(opened("story.txt") - before, 1);
        assert_eq!(rows(&s), [("story.txt", Standing::Marked { left: 2 })]);
        fault::unreadable(Some("story.txt"));
        let refused = status(&repo, false, None);
        fault::unreadable(None);
        assert!(refused.is_err());
        // Fixed and saved, the place is no longer in conflict: the file isn't opened.
        let fixed: &[u8] = b"a\nmine\nb\nmine 2\n";
        plant_save(&mut repo, &[("story.txt", EntryKind::File, fixed)]);
        write(root, "story.txt", fixed);
        age(root, "story.txt");
        status_of(&repo);
        fault::unreadable(Some("story.txt"));
        let s = status(&repo, false, None);
        fault::unreadable(None);
        let s = s.unwrap();
        assert!(s.conflicts.is_empty() && s.changes.is_empty());
    }

    #[test]
    fn a_conflict_standing_as_a_folder_is_compared_with_that_folder() {
        let (dir, mut repo) = project();
        let root = dir.path();
        let held = plant_save(
            &mut repo,
            &[
                ("dir/a.txt", EntryKind::File, b"a"),
                ("dir/sub/b.txt", EntryKind::File, b"b"),
            ],
        );
        let folder = Term {
            kind: EntryKind::Folder,
            id: held.get("dir").unwrap().id,
            size: 0,
        };
        keep(&mut repo, &[b"base", b"theirs"]);
        let beside: &[(&str, EntryKind, &[u8])] = &[
            ("dir-2/x", EntryKind::File, b"x"),
            ("dir.txt", EntryKind::File, b"t"),
            ("dir0", EntryKind::File, b"0"),
        ];
        plant_conflicted(&mut repo, beside, &[("dir", versions(Some(folder)))]);
        write(root, "dir/a.txt", b"a");
        write(root, "dir/sub/b.txt", b"b");
        for (p, _, b) in beside {
            write(root, p, b);
        }
        let s = status_of(&repo);
        assert_eq!(rows(&s), [("dir", Standing::Yours)]);
        assert!(s.changes.is_empty(), "{:?}", s.changes);
        write(root, "dir/sub/b.txt", b"b, changed");
        write(root, "dir/new.txt", b"new");
        // What is beside the place, before and after it by path, is still listed.
        for (p, _, _) in beside {
            write(root, p, b"changed");
        }
        let s = status_of(&repo);
        assert_eq!(rows(&s), [("dir", Standing::ChangedInPlace)]);
        assert_eq!(changed(&s), ["dir-2/x", "dir.txt", "dir0"]);
    }

    #[test]
    fn a_record_naming_another_kind_is_damage_at_its_place() {
        // Standing as a folder, it names a file's content.
        let (dir, mut repo) = project();
        keep(&mut repo, &[b"picture", b"base", b"theirs"]);
        let wrong = Term {
            kind: EntryKind::Folder,
            id: file_of(b"picture").id,
            size: 0,
        };
        let record = versions(Some(wrong));
        let id = record.id();
        plant_conflicted(&mut repo, &[], &[("art/hero", record)]);
        write(dir.path(), "art/hero/x.png", b"x");
        let e = status(&repo, false, None).unwrap_err();
        let want = format!(
            "The conflict at art/hero can't be read (record {}): object ",
            id.short_hex()
        );
        assert!(e.to_string().starts_with(&want), "{e}");
        assert_eq!(
            e.hint_when_reading().as_deref(),
            Some("Nothing was changed. Ask for help before trying to repair it.")
        );
    }

    #[test]
    fn a_conflict_that_cant_be_followed_stops_status_naming_its_place() {
        // A record the store doesn't hold.
        let (dir, mut repo) = project();
        let missing = Id([0xab; 32]);
        crate::test_support::plant_root(&mut repo, |packs| {
            let entry = Entry {
                name: "lost.png".into(),
                kind: EntryKind::Conflict,
                id: missing,
                size: 0,
            };
            let (inner, _) = crate::test_support::add_listing(packs, vec![entry]);
            let folder = Entry {
                name: "art".into(),
                kind: EntryKind::Folder,
                id: inner,
                size: 0,
            };
            crate::test_support::add_listing(packs, vec![folder]).0
        });
        write(dir.path(), "art/lost.png", b"x");
        let e = status(&repo, false, None).unwrap_err().to_string();
        let want = format!(
            "The conflict at art/lost.png can't be read (record {}): ",
            missing.short_hex()
        );
        assert!(e.starts_with(&want), "{e}");
        // A conflict inside the folder another conflict stands as.
        let (dir, mut repo) = project();
        keep(&mut repo, &[b"base", b"theirs"]);
        let inner = plant_conflicted(&mut repo, &[], &[("dir/c.png", versions(None))]);
        let folder = Term {
            kind: EntryKind::Folder,
            id: repo.store().listing(&inner).unwrap().get("dir").unwrap().id,
            size: 0,
        };
        plant_conflicted(&mut repo, &[], &[("outer", versions(Some(folder)))]);
        write(dir.path(), "outer/c.png", b"x");
        let e = status(&repo, false, None).unwrap_err();
        assert_eq!(
            e.to_string(),
            "outer/c.png is a conflict inside a conflict, which Opalro never writes, so this \
             project may be damaged."
        );
        assert_eq!(
            e.hint_when_reading().as_deref(),
            Some("Nothing was changed. Ask for help before trying to repair it.")
        );
    }

    /// The listings and records `status` reads in the project at `root`, planted with `plant`.
    fn reads(plant: impl FnOnce(&mut Repository, &Path), without: bool) -> (usize, usize) {
        let (dir, mut repo) = project();
        plant(&mut repo, dir.path());
        let repo = if without {
            without_conflicts_feature(dir.path())
        } else {
            Repository::open(dir.path()).unwrap()
        };
        let before = (repo.store().listings_read(), repo.store().conflicts_read());
        status(&repo, false, None).unwrap();
        (
            repo.store().listings_read() - before.0,
            repo.store().conflicts_read() - before.1,
        )
    }

    #[test]
    fn a_project_without_conflicts_reads_nothing_more() {
        let paths: Vec<String> = (0..4)
            .flat_map(|a| (0..3).map(move |b| format!("f{a}/g{b}/h/file{a}{b}.txt")))
            .collect();
        let paths = &paths;
        let deep = "f1/g2/h/deep.png";
        let plain = |repo: &mut Repository, root: &Path| {
            let mut files: Vec<(&str, EntryKind, &[u8])> = paths
                .iter()
                .map(|p| (p.as_str(), EntryKind::File, &b"same"[..]))
                .collect();
            files.push((deep, EntryKind::File, b"picture"));
            plant_save(repo, &files[..3]);
            plant_save(repo, &files);
            for (p, _, b) in &files {
                write(root, p, b);
            }
        };
        let with = reads(plain, false);
        let without = reads(plain, true);
        assert_eq!(with, without);
        assert_eq!(with.1, 0);
        // Each folder of the save once, as the walk goes into it: the top, 4, 12 and 12.
        assert_eq!(with.0, 29);
        // One conflict deep in one folder, standing as a file: no more listings, one record.
        let conflicted = |folder: bool| {
            move |repo: &mut Repository, root: &Path| {
                let mut files: Vec<(&str, EntryKind, &[u8])> = paths
                    .iter()
                    .map(|p| (p.as_str(), EntryKind::File, &b"same"[..]))
                    .collect();
                plant_save(repo, &files[..3]);
                let held = plant_save(
                    repo,
                    &[
                        ("in/x.png", EntryKind::File, b"picture"),
                        ("picture", EntryKind::File, b"picture"),
                        ("b", EntryKind::File, b"base"),
                        ("t", EntryKind::File, b"theirs"),
                    ],
                );
                let mine = if folder {
                    Term {
                        kind: EntryKind::Folder,
                        id: held.get("in").unwrap().id,
                        size: 0,
                    }
                } else {
                    file_of(b"picture")
                };
                plant_conflicted(repo, &files, &[(deep, versions(Some(mine)))]);
                if folder {
                    files.push(("f1/g2/h/deep.png/x.png", EntryKind::File, b"picture"));
                } else {
                    files.push((deep, EntryKind::File, b"picture"));
                }
                for (p, _, b) in &files {
                    write(root, p, b);
                }
            }
        };
        let (listings, records) = reads(conflicted(false), false);
        assert_eq!((listings, records), (with.0, 1));
        let (listings, records) = reads(conflicted(true), false);
        assert_eq!((listings, records), (with.0 + 1, 1));
    }

    #[test]
    fn status_says_why_it_stops_without_talk_of_saving() {
        let bad_name = SaveError::InvalidName {
            path: "a/b\u{fffd}".into(),
            reason: "its name is not valid UTF-8".into(),
        };
        assert_eq!(
            why_stopped(&bad_name),
            "Opalro can't show the status: a/b\u{fffd}: its name is not valid UTF-8."
        );
        let too_large = SaveError::TooLarge {
            detail: "its folders nest more than 4096 levels deep".into(),
        };
        assert_eq!(
            why_stopped(&too_large),
            "Opalro can't show the status: this project is too large for Opalro: its folders nest \
             more than 4096 levels deep."
        );
        // What says nothing of saving stays as it is.
        let changing = SaveError::ChangedDuringSave {
            path: "a.txt".into(),
        };
        assert_eq!(why_stopped(&changing), changing.to_string());
    }
}
