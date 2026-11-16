//! The note of the save made last, `.opalro/cache/last-made` (storage-format.md section 10): the
//! save this computer made last on a line and the diary entry that recorded it. Only ever a hint:
//! one that is missing, sealed by another key, edited or unreadable is just not used.
#![deny(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::ffi::OsStr;

use crate::encoding::{DecodeError, Reader, Writer};
use crate::fsio;
use crate::id::Id;
use crate::keyed;
use crate::repo::Repository;
use crate::safe_fs;

const MAGIC: &[u8; 8] = b"OPALMADE";
const VERSION: u8 = 1;
const FILE: &str = "last-made";
/// The magic, the version and two IDs.
const BODY_BYTES: usize = MAGIC.len() + 1 + 2 * size_of::<Id>();
/// The whole file, seal included.
const FILE_BYTES: u64 = (BODY_BYTES + keyed::KEY_LEN) as u64;

/// The save this computer made last on a line, and the diary entry that recorded it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LastMade {
    pub save: Id,
    pub entry: Id,
}

impl LastMade {
    /// The note's body, without its seal.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        for b in MAGIC {
            w.u8(*b);
        }
        w.u8(VERSION);
        w.id(&self.save);
        w.id(&self.entry);
        w.into_bytes()
    }

    /// Reads a body `encode` wrote; one of another length, magic or version is refused.
    pub fn decode(body: &[u8]) -> Result<LastMade, DecodeError> {
        if body.len() != BODY_BYTES {
            return Err(DecodeError::Invalid("a last-made note's length"));
        }
        let rest = body
            .strip_prefix(MAGIC.as_slice())
            .ok_or(DecodeError::Invalid("not a last-made note"))?;
        let mut r = Reader::new(rest);
        if r.u8()? != VERSION {
            return Err(DecodeError::Invalid("last-made note version"));
        }
        let made = LastMade {
            save: r.id()?,
            entry: r.id()?,
        };
        r.finish()?;
        Ok(made)
    }

    /// The note in `repo`, or `None` if the handle has no key, or the note can't be read, isn't
    /// sealed with that key or doesn't decode.
    pub(crate) fn load(repo: &Repository) -> Option<LastMade> {
        let key = repo.cache_key()?;
        let shown = repo.cache_dir().join(FILE);
        let bytes =
            safe_fs::read_capped_in(repo.cache_folder(), OsStr::new(FILE), &shown, FILE_BYTES)
                .ok()?;
        LastMade::decode(keyed::open_sealed(key, &bytes)?).ok()
    }

    /// Writes the note, sealed with the handle's key. A note that can't be written is just not
    /// there next time, so a failure is ignored.
    pub(crate) fn store(repo: &Repository, save: Id, entry: Id) {
        let Some(key) = repo.cache_key() else {
            return;
        };
        let mut buf = LastMade { save, entry }.encode();
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

    const MADE: LastMade = LastMade {
        save: Id([3; 32]),
        entry: Id([4; 32]),
    };

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
    fn a_note_reads_back_as_written() {
        let body = MADE.encode();
        assert_eq!(body.len(), BODY_BYTES);
        assert_eq!(LastMade::decode(&body).unwrap(), MADE);
    }

    #[test]
    fn a_note_that_breaks_its_layout_is_refused() {
        let good = MADE.encode();
        let short = &good[..good.len() - 1];
        assert!(LastMade::decode(short).is_err());
        let mut long = good.clone();
        long.push(0);
        assert!(LastMade::decode(&long).is_err());
        let mut magic = good.clone();
        magic[0] = b'X';
        assert!(LastMade::decode(&magic).is_err());
        let mut version = good;
        version[8] = 2;
        assert!(LastMade::decode(&version).is_err());
    }

    #[test]
    fn store_then_load_gives_the_note() {
        let (dir, repo) = repo();
        LastMade::store(&repo, MADE.save, MADE.entry);
        let file = dir.path().join(".opalro/cache/last-made");
        assert_eq!(std::fs::metadata(&file).unwrap().len(), FILE_BYTES);
        assert_eq!(LastMade::load(&repo), Some(MADE));
        // Written again each time.
        let other = LastMade {
            save: Id([5; 32]),
            entry: Id([6; 32]),
        };
        LastMade::store(&repo, other.save, other.entry);
        assert_eq!(LastMade::load(&repo), Some(other));
    }

    #[test]
    fn load_gives_none_for_anything_it_cannot_trust() {
        let (dir, repo) = repo();
        let file = dir.path().join(".opalro/cache/last-made");
        // No file.
        assert_eq!(LastMade::load(&repo), None);
        // A seal by another key.
        std::fs::write(&file, keyed::seal(&[8; 32], &MADE.encode())).unwrap();
        assert_eq!(LastMade::load(&repo), None);
        // A torn file, and one with a byte too many.
        let sealed = keyed::seal(&[7; 32], &MADE.encode());
        std::fs::write(&file, &sealed[..sealed.len() - 1]).unwrap();
        assert_eq!(LastMade::load(&repo), None);
        let mut longer = sealed.clone();
        longer.push(0);
        std::fs::write(&file, &longer).unwrap();
        assert_eq!(LastMade::load(&repo), None);
        std::fs::write(&file, &sealed).unwrap();
        assert_eq!(LastMade::load(&repo), Some(MADE));
    }

    #[test]
    fn a_handle_without_a_key_neither_reads_nor_writes_it() {
        let (dir, repo) = keyless();
        let file = dir.path().join(".opalro/cache/last-made");
        std::fs::write(&file, keyed::seal(&[7; 32], &MADE.encode())).unwrap();
        assert_eq!(LastMade::load(&repo), None);
        std::fs::remove_file(&file).unwrap();
        LastMade::store(&repo, MADE.save, MADE.entry);
        assert!(!file.exists());
    }

    #[test]
    fn a_folder_in_its_place_is_no_note() {
        let (dir, repo) = repo();
        std::fs::create_dir(dir.path().join(".opalro/cache/last-made")).unwrap();
        assert_eq!(LastMade::load(&repo), None);
        LastMade::store(&repo, MADE.save, MADE.entry);
        assert_eq!(LastMade::load(&repo), None);
    }
}
