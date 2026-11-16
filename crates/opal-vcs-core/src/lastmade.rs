//! The notes of the saves this computer made, `.opalro/cache/last-made` (storage-format.md
//! section 10): for each line whose newest save it made, that save and the diary entry that
//! recorded it. Only ever a hint: a file that is missing, sealed by another key, edited or
//! unreadable is just not used.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::collections::{BTreeMap, HashSet};
use std::ffi::OsStr;

use crate::diary::State;
use crate::encoding::{DecodeError, Reader, Writer};
use crate::fsio;
use crate::id::Id;
use crate::keyed;
use crate::repo::Repository;
use crate::safe_fs;

const MAGIC: &[u8; 8] = b"OPALMADE";
const VERSION: u8 = 1;
const FILE: &str = "last-made";
/// The most notes the file holds.
pub const MAX_NOTES: u64 = 65_536;
/// The count's varint at its largest.
const COUNT_BYTES: u64 = 3;
/// A save's ID and an entry's.
const NOTE_BYTES: u64 = 2 * size_of::<Id>() as u64;
/// The largest file, seal included.
const FILE_BYTES: u64 =
    MAGIC.len() as u64 + 1 + COUNT_BYTES + MAX_NOTES * NOTE_BYTES + keyed::KEY_LEN as u64;

/// The saves this computer made that are some line's newest, each with the diary entry that
/// recorded it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LastMade {
    notes: BTreeMap<Id, Id>,
}

impl LastMade {
    /// The diary entry that recorded `save`, if there is a note of it.
    #[must_use]
    pub fn entry_of(&self, save: &Id) -> Option<Id> {
        self.notes.get(save).copied()
    }

    /// How many notes there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.notes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// Notes `save` as recorded by `entry`, in place of a note of the same save.
    pub fn insert(&mut self, save: Id, entry: Id) {
        self.notes.insert(save, entry);
    }

    /// The body, without its seal.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        for b in MAGIC {
            w.u8(*b);
        }
        w.u8(VERSION);
        w.varint(self.notes.len() as u64);
        for (save, entry) in &self.notes {
            w.id(save);
            w.id(entry);
        }
        w.into_bytes()
    }

    /// Reads a body `encode` wrote. Another magic or version, too many notes, notes out of order
    /// or twice, and bytes left over are refused.
    pub fn decode(body: &[u8]) -> Result<LastMade, DecodeError> {
        let rest = body
            .strip_prefix(MAGIC.as_slice())
            .ok_or(DecodeError::Invalid("not a last-made note"))?;
        let mut r = Reader::new(rest);
        if r.u8()? != VERSION {
            return Err(DecodeError::Invalid("last-made note version"));
        }
        let count = r.count(MAX_NOTES, "last-made notes")?;
        if count > r.remaining() as u64 / NOTE_BYTES {
            return Err(DecodeError::Invalid(
                "more last-made notes than their bytes hold",
            ));
        }
        let mut made = LastMade::default();
        let mut last: Option<Id> = None;
        for _ in 0..count {
            let (save, entry) = (r.id()?, r.id()?);
            if last.is_some_and(|last| last >= save) {
                return Err(DecodeError::Invalid("last-made notes out of order"));
            }
            last = Some(save);
            made.notes.insert(save, entry);
        }
        r.finish()?;
        Ok(made)
    }

    /// The notes in `repo`; none if the handle has no key, or the file can't be read, isn't
    /// sealed with that key or doesn't decode.
    pub(crate) fn load(repo: &Repository) -> LastMade {
        let Some(key) = repo.cache_key() else {
            return LastMade::default();
        };
        let shown = repo.cache_dir().join(FILE);
        safe_fs::read_capped_in(repo.cache_folder(), OsStr::new(FILE), &shown, FILE_BYTES)
            .ok()
            .and_then(|bytes| LastMade::decode(keyed::open_sealed(key, &bytes)?).ok())
            .unwrap_or_default()
    }

    /// Notes `save`, made by the diary entry `entry` whose state is `state`, keeping only the
    /// notes of saves that are some line's newest in `state`; past the limit, only this one. A
    /// file that can't be written is just not there next time, so a failure is ignored. Nothing
    /// when the handle has no key or stores no caches.
    pub(crate) fn store(repo: &Repository, state: &State, save: Id, entry: Id) {
        let Some(key) = repo.storing_key() else {
            return;
        };
        let mut made = LastMade::load(repo);
        let newest: HashSet<&Id> = state.lines.values().collect();
        made.notes.retain(|save, _| newest.contains(save));
        made.insert(save, entry);
        if made.notes.len() as u64 > MAX_NOTES {
            made = LastMade::default();
            made.insert(save, entry);
        }
        let mut buf = made.encode();
        keyed::seal_in_place(key, &mut buf);
        // Readers don't hold the lock, so another command may clear `tmp/` under this write; it
        // then fails, which is fine.
        let _ = fsio::write_cache(repo.tmp_folder(), repo.cache_folder(), FILE, &buf);
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )]
    use super::*;
    use crate::diary::Position;

    /// Notes of saves 3…3 (entry 4…4) and 5…5 (entry 6…6).
    fn two() -> LastMade {
        let mut made = LastMade::default();
        made.insert(Id([5; 32]), Id([6; 32]));
        made.insert(Id([3; 32]), Id([4; 32]));
        made
    }

    /// A state whose lines are at `saves`.
    fn lines_at(saves: &[Id]) -> State {
        State {
            lines: (saves.iter().enumerate())
                .map(|(n, id)| (format!("line-{n}"), *id))
                .collect(),
            tags: None,
            position: Position::OnLine("line-0".into()),
            shelves: vec![],
            server_lines: None,
        }
    }

    /// A project whose handle has no cache key.
    fn keyless() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let person = crate::save::Person {
            name: "Ada".into(),
            email: "a@example.com".into(),
            time_ms: 0,
            tz_offset_min: 0,
        };
        let repo = Repository::init(dir.path(), &person).unwrap();
        (dir, repo)
    }

    fn repo() -> (tempfile::TempDir, Repository) {
        let (dir, mut repo) = keyless();
        repo.set_cache_key([7; 32]);
        (dir, repo)
    }

    #[test]
    fn notes_read_back_as_written() {
        assert_eq!(crate::encoding::varint_len(MAX_NOTES), COUNT_BYTES);
        let body = two().encode();
        assert_eq!(body.len(), 8 + 1 + 1 + 2 * 64);
        let made = LastMade::decode(&body).unwrap();
        assert_eq!(made, two());
        assert_eq!(made.entry_of(&Id([3; 32])), Some(Id([4; 32])));
        assert_eq!(made.entry_of(&Id([5; 32])), Some(Id([6; 32])));
        assert_eq!(made.entry_of(&Id([4; 32])), None);
        let none = LastMade::default().encode();
        assert!(LastMade::decode(&none).unwrap().is_empty());
    }

    /// A body with `notes` as given, in that order, and a count of `count`.
    fn body(count: u64, notes: &[(u8, u8)]) -> Vec<u8> {
        let mut w = Writer::new();
        for b in MAGIC {
            w.u8(*b);
        }
        w.u8(VERSION);
        w.varint(count);
        for (save, entry) in notes {
            w.id(&Id([*save; 32]));
            w.id(&Id([*entry; 32]));
        }
        w.into_bytes()
    }

    #[test]
    fn notes_that_break_their_layout_are_refused() {
        assert!(LastMade::decode(&body(2, &[(3, 4), (5, 6)])).is_ok());
        // Another magic, another version.
        let mut magic = body(1, &[(3, 4)]);
        magic[0] = b'X';
        assert!(LastMade::decode(&magic).is_err());
        let mut version = body(1, &[(3, 4)]);
        version[8] = 2;
        assert!(LastMade::decode(&version).is_err());
        // A count over the limit, or over what the bytes hold.
        assert!(LastMade::decode(&body(MAX_NOTES + 1, &[])).is_err());
        assert!(LastMade::decode(&body(3, &[(3, 4), (5, 6)])).is_err());
        // Out of order, and a save twice.
        assert!(LastMade::decode(&body(2, &[(5, 6), (3, 4)])).is_err());
        assert!(LastMade::decode(&body(2, &[(3, 4), (3, 6)])).is_err());
        // Bytes left over, and a note cut short.
        let mut longer = body(1, &[(3, 4)]);
        longer.push(0);
        assert!(LastMade::decode(&longer).is_err());
        let short = body(1, &[(3, 4)]);
        assert!(LastMade::decode(&short[..short.len() - 1]).is_err());
    }

    #[test]
    fn store_keeps_the_notes_of_each_line_s_newest_save() {
        let (_dir, repo) = repo();
        let (a, b, c) = (Id([3; 32]), Id([5; 32]), Id([9; 32]));
        LastMade::store(&repo, &lines_at(&[a]), a, Id([4; 32]));
        LastMade::store(&repo, &lines_at(&[a, b]), b, Id([6; 32]));
        assert_eq!(LastMade::load(&repo), two());
        // A note of the same save is replaced.
        LastMade::store(&repo, &lines_at(&[a, b]), b, Id([7; 32]));
        assert_eq!(LastMade::load(&repo).entry_of(&b), Some(Id([7; 32])));
        // A save no line has as its newest any more loses its note.
        LastMade::store(&repo, &lines_at(&[b, c]), c, Id([8; 32]));
        let made = LastMade::load(&repo);
        assert_eq!(made.len(), 2);
        assert_eq!(made.entry_of(&a), None);
        assert_eq!(made.entry_of(&c), Some(Id([8; 32])));
    }

    #[test]
    fn past_the_limit_only_the_new_note_is_kept() {
        let (dir, repo) = repo();
        let mut full = LastMade::default();
        let saves: Vec<Id> = (0..MAX_NOTES)
            .map(|n| {
                let mut id = [0; 32];
                id[..8].copy_from_slice(&n.to_be_bytes());
                Id(id)
            })
            .collect();
        for save in &saves {
            full.insert(*save, Id([1; 32]));
        }
        let file = dir.path().join(".opalro/cache/last-made");
        let sealed = keyed::seal(&[7; 32], &full.encode());
        assert_eq!(sealed.len() as u64, FILE_BYTES);
        std::fs::write(&file, sealed).unwrap();
        assert_eq!(LastMade::load(&repo).len(), saves.len());
        let mut state = lines_at(&saves);
        let new = Id([0xee; 32]);
        state.lines.insert("new".into(), new);
        LastMade::store(&repo, &state, new, Id([2; 32]));
        let made = LastMade::load(&repo);
        assert_eq!(made.len(), 1);
        assert_eq!(made.entry_of(&new), Some(Id([2; 32])));
    }

    #[test]
    fn load_gives_nothing_it_cannot_trust() {
        let (dir, repo) = repo();
        let file = dir.path().join(".opalro/cache/last-made");
        // No file.
        assert!(LastMade::load(&repo).is_empty());
        // A seal by another key.
        std::fs::write(&file, keyed::seal(&[8; 32], &two().encode())).unwrap();
        assert!(LastMade::load(&repo).is_empty());
        // A torn file, and one with a byte too many.
        let sealed = keyed::seal(&[7; 32], &two().encode());
        std::fs::write(&file, &sealed[..sealed.len() - 1]).unwrap();
        assert!(LastMade::load(&repo).is_empty());
        let mut longer = sealed.clone();
        longer.push(0);
        std::fs::write(&file, &longer).unwrap();
        assert!(LastMade::load(&repo).is_empty());
        std::fs::write(&file, &sealed).unwrap();
        assert_eq!(LastMade::load(&repo), two());
    }

    #[test]
    fn a_handle_without_a_key_neither_reads_nor_writes_it() {
        let (dir, repo) = keyless();
        let file = dir.path().join(".opalro/cache/last-made");
        std::fs::write(&file, keyed::seal(&[7; 32], &two().encode())).unwrap();
        assert!(LastMade::load(&repo).is_empty());
        std::fs::remove_file(&file).unwrap();
        let a = Id([3; 32]);
        LastMade::store(&repo, &lines_at(&[a]), a, Id([4; 32]));
        assert!(!file.exists());
    }

    #[test]
    fn a_folder_in_its_place_is_no_note() {
        let (dir, repo) = repo();
        std::fs::create_dir(dir.path().join(".opalro/cache/last-made")).unwrap();
        assert!(LastMade::load(&repo).is_empty());
        let a = Id([3; 32]);
        LastMade::store(&repo, &lines_at(&[a]), a, Id([4; 32]));
        assert!(LastMade::load(&repo).is_empty());
    }
}
