//! A stored listing as it stands in the folder: each conflict as what it stands as.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use super::{Record, nested, stands_as, unreadable};
use crate::id::Id;
use crate::listing::{Entry, EntryKind, Listing};
use crate::repo::Repository;
use crate::store::{Store, StoreError};
use crate::text::escape;

/// `e` as it stands in the folder: a conflict whose record is in `records` as what that stands
/// as, or `None` for nothing; anything else, a conflict whose record wasn't read included, as it
/// is, borrowed.
pub(crate) fn as_it_stands<'e>(
    e: &'e Entry,
    records: &HashMap<Id, Record>,
) -> Option<Cow<'e, Entry>> {
    let record = match records.get(&e.id) {
        Some(record) if e.kind == EntryKind::Conflict => record,
        _ => return Some(Cow::Borrowed(e)),
    };
    stands_as(record).map(|t| {
        Cow::Owned(Entry {
            name: e.name.clone(),
            kind: t.kind,
            id: t.id,
            size: t.size,
        })
    })
}

/// What a reader as it stands has read: the records, by ID; the folder listings a conflict
/// stands as, and those below them, where a conflict is nesting; and whether it replaced an
/// entry with a file or link.
#[derive(Default)]
struct Stood {
    records: HashMap<Id, Record>,
    below: HashSet<Id>,
    stood_in: bool,
}

/// How a walk reads a listing: as stored (one save against another), or as it stands in the
/// folder. Records are read only for kind-6 entries of listings read, each once per command.
pub(crate) struct Listings<'a> {
    store: &'a Store,
    unstored: &'a HashMap<Id, Listing>,
    /// `None`: as stored. Else the records read so far, and the folders a conflict stands as
    /// (and those below them), where a kind-6 entry is refused as nesting.
    standing: Option<RefCell<Stood>>,
}

impl<'a> Listings<'a> {
    #[cfg_attr(not(test), expect(dead_code))]
    pub(crate) fn stored(store: &'a Store, unstored: &'a HashMap<Id, Listing>) -> Listings<'a> {
        Listings {
            store,
            unstored,
            standing: None,
        }
    }

    /// As it stands, in a project that keeps conflicts; as stored in one that doesn't.
    #[cfg_attr(not(test), expect(dead_code))]
    pub(crate) fn standing(
        repo: &'a Repository,
        unstored: &'a HashMap<Id, Listing>,
    ) -> Listings<'a> {
        Listings {
            store: repo.store(),
            unstored,
            standing: repo.keeps_conflicts().then(RefCell::default),
        }
    }

    /// The listing `id`: from `unstored`, else the store; as it stands, each kind-6 entry
    /// replaced by what its record stands as (its name, the term's kind, ID and size) or left
    /// out when that is nothing. The listing keeps the ID it was asked by. A kind-6 entry in a
    /// listing below a folder a conflict stands as is damage ("a conflict inside a conflict").
    #[cfg_attr(not(test), expect(dead_code))]
    pub(crate) fn read(&self, id: &Id) -> Result<Listing, StoreError> {
        let listing = match self.unstored.get(id) {
            Some(listing) => listing.clone(),
            None => self.store.listing(id)?,
        };
        let Some(standing) = &self.standing else {
            return Ok(listing);
        };
        let below = standing.borrow().below.contains(id);
        let mut entries = Vec::with_capacity(listing.entries().len());
        for e in listing.entries() {
            let place = || {
                format!(
                    "{} in folder listing {}",
                    escape(&e.name, false),
                    id.short_hex()
                )
            };
            if e.kind != EntryKind::Conflict {
                if below && e.kind == EntryKind::Folder {
                    standing.borrow_mut().below.insert(e.id);
                }
                entries.push(e.clone());
                continue;
            }
            if below {
                return Err(nested(&place()));
            }
            let record = self
                .record(&e.id)
                .map_err(|err| unreadable(&place(), &e.id, err))?;
            let mut stood = standing.borrow_mut();
            let Some(t) = stands_as(&record) else {
                continue;
            };
            match t.kind {
                EntryKind::Folder => {
                    stood.below.insert(t.id);
                }
                EntryKind::File | EntryKind::Executable | EntryKind::Symlink => {
                    stood.stood_in = true;
                }
                _ => {}
            }
            entries.push(Entry {
                name: e.name.clone(),
                kind: t.kind,
                id: t.id,
                size: t.size,
            });
        }
        Ok(Listing::new(entries)?)
    }

    /// The record `id`, read once per command.
    pub(crate) fn record(&self, id: &Id) -> Result<Record, StoreError> {
        if let Some(r) = self
            .standing
            .as_ref()
            .and_then(|s| s.borrow().records.get(id).cloned())
        {
            return Ok(r);
        }
        let record = self.store.conflict(id)?;
        if let Some(s) = &self.standing {
            s.borrow_mut().records.insert(*id, record.clone());
        }
        Ok(record)
    }

    /// Whether any entry this reader replaced was written as a file or link: for `goto`'s words.
    #[cfg_attr(not(test), expect(dead_code))]
    pub(crate) fn stood_in(&self) -> bool {
        self.standing.as_ref().is_some_and(|s| s.borrow().stood_in)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::conflict::tests::{file_of, plant_conflicted};
    use crate::conflict::{Own, Term};
    use crate::save::Person;
    use crate::test_support::{HOSTILE, assert_hostile_text_escaped, plant_save};

    fn project() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let who = Person {
            name: "Ada".into(),
            email: "ada@example.com".into(),
            time_ms: 1_700_000_000_000,
            tz_offset_min: 0,
        };
        let repo = Repository::init(dir.path(), &who).unwrap();
        (dir, repo)
    }

    fn mine_theirs(mine: Option<Term>) -> Record {
        Record {
            terms: vec![mine, None, Some(file_of(b"theirs"))],
            own: None,
        }
    }

    fn entry_of(l: &Listing, name: &str) -> Option<(EntryKind, Id, u64)> {
        l.get(name).map(|e| (e.kind, e.id, e.size))
    }

    #[test]
    fn a_listing_as_it_stands_replaces_each_conflict_and_keeps_its_id() {
        let (_dir, mut repo) = project();
        plant_save(
            &mut repo,
            &[
                ("a", EntryKind::File, b"mine"),
                ("t", EntryKind::File, b"theirs"),
            ],
        );
        let own = Record {
            terms: vec![Some(file_of(b"mine")), None, Some(file_of(b"theirs"))],
            own: Some(Own {
                version: Some(file_of(b"theirs")),
                marker: 0,
            }),
        };
        let conflicts = [("c", mine_theirs(Some(file_of(b"mine")))), ("d", own)];
        let root = plant_conflicted(&mut repo, &[("x", EntryKind::File, b"x")], &conflicts);
        let unstored = HashMap::new();
        let reader = Listings::standing(&repo, &unstored);
        let stood = reader.read(&root).unwrap();
        let mine = file_of(b"mine");
        assert_eq!(entry_of(&stood, "c"), Some((EntryKind::File, mine.id, 4)));
        let theirs = file_of(b"theirs");
        assert_eq!(entry_of(&stood, "d"), Some((EntryKind::File, theirs.id, 6)));
        assert_eq!(entry_of(&stood, "x").unwrap().0, EntryKind::File);
        assert!(reader.stood_in());
        // Its stored ID is where it came from: the stored listing is unchanged.
        assert_eq!(
            repo.store().listing(&root).unwrap().get("c").unwrap().kind,
            EntryKind::Conflict
        );
    }

    #[test]
    fn a_conflict_standing_as_nothing_leaves_no_name() {
        let (_dir, mut repo) = project();
        let root = plant_conflicted(&mut repo, &[], &[("c", mine_theirs(None))]);
        let unstored = HashMap::new();
        let reader = Listings::standing(&repo, &unstored);
        assert!(reader.read(&root).unwrap().entries().is_empty());
        assert!(!reader.stood_in());
    }

    #[test]
    fn a_conflict_standing_as_a_folder_reads_as_that_folder() {
        let (_dir, mut repo) = project();
        let held = plant_save(&mut repo, &[("dir/inner", EntryKind::File, b"in")]);
        let folder = held.get("dir").unwrap().id;
        let term = Term {
            kind: EntryKind::Folder,
            id: folder,
            size: 0,
        };
        let root = plant_conflicted(&mut repo, &[], &[("c", mine_theirs(Some(term)))]);
        let unstored = HashMap::new();
        let reader = Listings::standing(&repo, &unstored);
        let stood = reader.read(&root).unwrap();
        assert_eq!(entry_of(&stood, "c"), Some((EntryKind::Folder, folder, 0)));
        assert!(reader.read(&folder).unwrap().get("inner").is_some());
        assert!(!reader.stood_in());
    }

    #[test]
    fn as_stored_reads_kind_6_as_it_is() {
        let (_dir, mut repo) = project();
        let root = plant_conflicted(&mut repo, &[], &[("c", mine_theirs(None))]);
        let unstored = HashMap::new();
        let asked = repo.store().conflicts_read();
        let stood = Listings::stored(repo.store(), &unstored)
            .read(&root)
            .unwrap();
        assert_eq!(stood.get("c").unwrap().kind, EntryKind::Conflict);
        assert_eq!(repo.store().conflicts_read(), asked);
    }

    #[test]
    fn without_the_feature_as_it_stands_is_as_stored() {
        let (dir, mut repo) = project();
        let root = plant_conflicted(&mut repo, &[], &[("c", mine_theirs(None))]);
        let repo = crate::test_support::without_conflicts_feature(dir.path());
        let unstored = HashMap::new();
        let stood = Listings::standing(&repo, &unstored).read(&root).unwrap();
        assert_eq!(stood.get("c").unwrap().kind, EntryKind::Conflict);
        assert_eq!(repo.store().conflicts_read(), 0);
    }

    #[test]
    fn each_record_is_read_once_per_reader() {
        let (_dir, mut repo) = project();
        let same = mine_theirs(None);
        let conflicts = [("c", same.clone()), ("d/c", same)];
        let root = plant_conflicted(&mut repo, &[], &conflicts);
        let unstored = HashMap::new();
        let reader = Listings::standing(&repo, &unstored);
        let asked = repo.store().conflicts_read();
        let top = reader.read(&root).unwrap();
        reader.read(&top.get("d").unwrap().id).unwrap();
        reader.read(&root).unwrap();
        assert_eq!(repo.store().conflicts_read(), asked + 1);
    }

    #[test]
    fn a_conflict_inside_a_folder_a_conflict_stands_as_is_damage() {
        let (_dir, mut repo) = project();
        let inner = mine_theirs(None);
        let held = plant_conflicted(&mut repo, &[], &[("dir/deeper/c", inner)]);
        let folder = repo.store().listing(&held).unwrap().get("dir").unwrap().id;
        let term = Term {
            kind: EntryKind::Folder,
            id: folder,
            size: 0,
        };
        let root = plant_conflicted(&mut repo, &[], &[("outer", mine_theirs(Some(term)))]);
        let unstored = HashMap::new();
        let reader = Listings::standing(&repo, &unstored);
        reader.read(&root).unwrap();
        let dir = reader.read(&folder).unwrap();
        let deeper = dir.get("deeper").unwrap().id;
        let e = reader.read(&deeper).unwrap_err();
        let StoreError::DamagedConflict(said) = &e else {
            panic!("{e:?}");
        };
        assert!(said.starts_with("c in folder listing "), "{said}");
        assert!(said.ends_with("is a conflict inside a conflict, which Opalro never writes, so this project may be damaged."), "{said}");
        // As stored, nothing is refused.
        Listings::stored(repo.store(), &unstored)
            .read(&deeper)
            .unwrap();
    }

    #[test]
    fn a_record_that_cant_be_read_is_damage_naming_it() {
        let (_dir, mut repo) = project();
        // A listing naming a record the store doesn't hold, under a hostile name.
        let missing = Id([0xab; 32]);
        let entry = Entry {
            name: HOSTILE.into(),
            kind: EntryKind::Conflict,
            id: missing,
            size: 0,
        };
        let mut planted = Id([0; 32]);
        crate::test_support::plant_root(&mut repo, |packs| {
            planted = crate::test_support::add_listing(packs, vec![entry]).0;
            planted
        });
        let unstored = HashMap::new();
        let e = Listings::standing(&repo, &unstored)
            .read(&planted)
            .unwrap_err();
        let StoreError::DamagedConflict(said) = &e else {
            panic!("{e:?}");
        };
        assert_hostile_text_escaped(said);
        assert!(
            said.contains(&format!("can't be read (record {})", missing.short_hex())),
            "{said}"
        );
        assert!(said.contains(&planted.short_hex()), "{said}");
    }
}
