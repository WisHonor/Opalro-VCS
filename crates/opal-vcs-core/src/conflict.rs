//! Conflict records: every version a place in conflict holds (docs/storage-format.md section 8).

use std::io;

use crate::encoding::{DecodeError, Reader, Writer};
use crate::id::{Flavor, Id, hash_record};
use crate::listing::EntryKind;
use crate::repo::RepoError;
use crate::snapshot::SaveError;
use crate::store::{StoreError, is_damage};
use crate::text::escape;

pub(crate) mod markers;
pub(crate) mod standing;

pub(crate) use markers::Left;
#[expect(unused_imports)]
pub(crate) use standing::Listings;

/// One version a conflict names: an entry's kind, ID and size, without a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Term {
    pub kind: EntryKind,
    pub id: Id,
    pub size: u64,
}

/// The version the folder holds for a conflict, when it is its own: `version` is `None` where the
/// person removed it; `marker` is 0, or 7 to 255 when `version` is a text written with markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Own {
    pub version: Option<Term>,
    pub marker: u8,
}

/// A conflict record (storage-format.md section 8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Side, where they parted, side, …: yours first, theirs last. Odd, 3 to 255.
    pub terms: Vec<Option<Term>>,
    pub own: Option<Own>,
}

pub const MAX_TERMS: u64 = 255;
pub const MIN_MARKER: u8 = 7;
/// The most bytes a conflict record may take; refused before it is unpacked.
pub const MAX_RECORD_BYTES: u64 = 16 << 10;

const VERSION: &str = "unknown conflict-record version";
const TERM_COUNT: &str = "a conflict names an odd number of versions, from 3 to 255";
const UNKNOWN_KIND: &str = "a version in a conflict is of an unknown kind";
const SIZED: &str = "a folder or project link in a conflict has a size";
const BAD_MARKER: &str = "a conflict's marker length is from 1 to 6";
const MARKED_NOT_FILE: &str = "a conflict has markers on a version that isn't a file";

impl Record {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(1);
        w.varint(self.terms.len() as u64);
        for term in &self.terms {
            write_term(&mut w, term.as_ref());
        }
        w.bool(self.own.is_some());
        if let Some(own) = &self.own {
            write_term(&mut w, own.version.as_ref());
            w.u8(own.marker);
        }
        w.into_bytes()
    }

    /// Refuses everything section 8 lists; never fixes anything.
    pub fn decode(bytes: &[u8]) -> Result<Record, DecodeError> {
        let mut r = Reader::new(bytes);
        if r.u8()? != 1 {
            return Err(DecodeError::Invalid(VERSION));
        }
        let count = r.varint()?;
        if count % 2 == 0 || !(3..=MAX_TERMS).contains(&count) {
            return Err(DecodeError::Invalid(TERM_COUNT));
        }
        let terms = (0..count)
            .map(|_| read_term(&mut r))
            .collect::<Result<Vec<_>, _>>()?;
        let own = if r.bool()? {
            let version = read_term(&mut r)?;
            let marker = r.u8()?;
            if (1..MIN_MARKER).contains(&marker) {
                return Err(DecodeError::Invalid(BAD_MARKER));
            }
            let text =
                version.is_some_and(|t| matches!(t.kind, EntryKind::File | EntryKind::Executable));
            if marker != 0 && !text {
                return Err(DecodeError::Invalid(MARKED_NOT_FILE));
            }
            Some(Own { version, marker })
        } else {
            None
        };
        r.finish()?;
        Ok(Record { terms, own })
    }

    #[must_use]
    pub fn id(&self) -> Id {
        hash_record(Flavor::Conflict, &self.encode())
    }

    /// Yours: the first term.
    #[must_use]
    pub fn mine(&self) -> Option<Term> {
        self.terms.first().copied().flatten()
    }

    /// Theirs: the last term.
    #[must_use]
    pub fn theirs(&self) -> Option<Term> {
        self.terms.last().copied().flatten()
    }

    /// Where the lines parted, for a record of three terms; `None` for more.
    #[must_use]
    pub fn before(&self) -> Option<Option<Term>> {
        match self.terms.as_slice() {
            [_, before, _] => Some(*before),
            _ => None,
        }
    }

    /// 0 unless its own version was written with markers.
    #[must_use]
    pub fn marker(&self) -> u8 {
        self.own.map_or(0, |own| own.marker)
    }
}

/// What a conflict stands as between a save and the folder: its own version if it has one, else
/// its first term. `None`: the folder holds nothing there.
#[must_use]
pub fn stands_as(r: &Record) -> Option<Term> {
    match r.own {
        Some(own) => own.version,
        None => r.mine(),
    }
}

/// What the folder shows at a place in conflict, against what the conflict stands as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Seen {
    Untouched,
    MarkersLeft(Left),
    Fixed,
    ChangedInPlace,
}

/// What the folder shows at the place of `c`: `now` is the folder's entry there as the walk built
/// it; `markers` scans the file there for marker lines of the length it is given, when asked.
/// Markers gone, or the file gone, is fixed; a version that changed is only changed.
pub(crate) fn seen(
    c: &Record,
    now: Option<(EntryKind, Id)>,
    markers: impl FnOnce(u8) -> io::Result<Left>,
) -> io::Result<Seen> {
    if now == stands_as(c).map(|t| (t.kind, t.id)) {
        return Ok(Seen::Untouched);
    }
    let marker = c.marker();
    if marker < MIN_MARKER {
        return Ok(Seen::ChangedInPlace);
    }
    Ok(match now {
        Some((EntryKind::File | EntryKind::Executable, _)) => {
            let left = markers(marker)?;
            if left.opening > 0 || left.closing > 0 {
                Seen::MarkersLeft(left)
            } else {
                Seen::Fixed
            }
        }
        _ => Seen::Fixed,
    })
}

/// `e`, met reading the record `id` of the conflict `at` (`at <path>`, or its name in a folder
/// listing), or reading a version it names, as the damage it is. What isn't damage, such as an
/// I/O error, stays itself.
pub(crate) fn unreadable(at: &str, id: &Id, e: StoreError) -> StoreError {
    let unread = |e: &dyn std::fmt::Display| {
        format!(
            "The conflict {at} can't be read (record {}): {}.",
            id.short_hex(),
            escape(&e.to_string(), false)
        )
    };
    StoreError::DamagedConflict(match e {
        StoreError::RecordTooLarge { .. } => format!(
            "The conflict {at} names a record over 16 KiB, which Opalro never writes, so this \
             project may be damaged."
        ),
        StoreError::DamagedConflict(said) => said,
        e @ (StoreError::Missing(_) | StoreError::WrongKind { .. } | StoreError::Decode(_)) => {
            unread(&e)
        }
        StoreError::Pack(p) if is_damage(&p) => unread(&p),
        e => return e,
    })
}

/// `e`, met at the conflict at `path` (record `id`) reading a version it names, as `unreadable`
/// tells it.
pub(crate) fn at_place(path: &str, id: &Id, e: SaveError) -> SaveError {
    match e {
        SaveError::Repo(RepoError::Store(e)) => unreadable(&at_path(path), id, e).into(),
        e => e,
    }
}

/// A conflict met inside a folder another conflict stands as, `place` naming it.
pub(crate) fn nested(place: &str) -> StoreError {
    StoreError::DamagedConflict(format!(
        "{place} is a conflict inside a conflict, which Opalro never writes, so this project may be \
         damaged."
    ))
}

/// The conflict at the project path `path`, as damage words name it.
pub(crate) fn at_path(path: &str) -> String {
    format!("at {}", escape(path, false))
}

fn write_term(w: &mut Writer, term: Option<&Term>) {
    w.bool(term.is_some());
    if let Some(t) = term {
        w.u8(t.kind as u8);
        w.id(&t.id);
        w.varint(t.size);
    }
}

fn read_term(r: &mut Reader<'_>) -> Result<Option<Term>, DecodeError> {
    if !r.bool()? {
        return Ok(None);
    }
    let kind = match EntryKind::from_u8(r.u8()?) {
        Some(EntryKind::Conflict) | None => return Err(DecodeError::Invalid(UNKNOWN_KIND)),
        Some(kind) => kind,
    };
    let id = r.id()?;
    let size = r.varint()?;
    if matches!(kind, EntryKind::Folder | EntryKind::ProjectLink) && size != 0 {
        return Err(DecodeError::Invalid(SIZED));
    }
    Ok(Some(Term { kind, id, size }))
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use proptest::prelude::*;
    use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    fn term(kind: EntryKind, byte: u8, size: u64) -> Term {
        Term {
            kind,
            id: Id([byte; 32]),
            size,
        }
    }

    fn file(byte: u8) -> Term {
        term(EntryKind::File, byte, 5)
    }

    fn three(own: Option<Own>) -> Record {
        Record {
            terms: vec![Some(file(1)), Some(file(2)), Some(file(3))],
            own,
        }
    }

    /// A term as section 3 allows it: sizes only for files and symlinks.
    fn any_term() -> impl Strategy<Value = Option<Term>> {
        let kinds = prop_oneof![
            Just(EntryKind::File),
            Just(EntryKind::Executable),
            Just(EntryKind::Symlink),
            Just(EntryKind::Folder),
            Just(EntryKind::ProjectLink),
        ];
        prop::option::weighted(
            0.8,
            (kinds, any::<[u8; 32]>(), any::<u64>()).prop_map(|(kind, id, size)| {
                let sized = matches!(
                    kind,
                    EntryKind::File | EntryKind::Executable | EntryKind::Symlink
                );
                Term {
                    kind,
                    id: Id(id),
                    size: if sized { size } else { 0 },
                }
            }),
        )
    }

    fn any_record() -> impl Strategy<Value = Record> {
        let terms =
            (1usize..=4).prop_flat_map(|half| prop::collection::vec(any_term(), half * 2 + 1));
        let own = prop::option::of((any_term(), 7u8..=255, any::<bool>()).prop_map(
            |(version, marker, marked)| {
                let text = version
                    .is_some_and(|t| matches!(t.kind, EntryKind::File | EntryKind::Executable));
                Own {
                    version,
                    marker: if text && marked { marker } else { 0 },
                }
            },
        ));
        (terms, own).prop_map(|(terms, own)| Record { terms, own })
    }

    fn runner(cases: u32) -> TestRunner {
        TestRunner::new_with_rng(
            Config::with_cases(cases),
            TestRng::deterministic_rng(RngAlgorithm::ChaCha),
        )
    }

    #[test]
    fn a_record_round_trips() {
        let seen: RefCell<BTreeMap<&str, usize>> = RefCell::default();
        runner(256)
            .run(&any_record(), |r| {
                let mut seen = seen.borrow_mut();
                let mut count = |what| *seen.entry(what).or_default() += 1;
                if r.terms.iter().any(Option::is_none) {
                    count("absent term");
                }
                if r.terms
                    .iter()
                    .flatten()
                    .any(|t| t.kind == EntryKind::Folder)
                {
                    count("folder term");
                }
                match r.own {
                    Some(Own {
                        version: Some(_), ..
                    }) => count("own present"),
                    Some(Own { version: None, .. }) => count("own absent"),
                    None => {}
                }
                if r.marker() >= 7 {
                    count("marker");
                }
                prop_assert_eq!(Record::decode(&r.encode()), Ok(r));
                Ok(())
            })
            .unwrap();
        let seen = seen.into_inner();
        for what in [
            "absent term",
            "folder term",
            "own present",
            "own absent",
            "marker",
        ] {
            assert!(seen.get(what).is_some_and(|&n| n >= 3), "{seen:?}");
        }
    }

    #[test]
    fn every_refusal_section_8_lists_is_made() {
        let good = three(Some(Own {
            version: Some(file(4)),
            marker: 7,
        }))
        .encode();
        assert!(Record::decode(&good).is_ok());
        let with = |at: usize, byte: u8| {
            let mut bytes = good.clone();
            bytes[at] = byte;
            bytes
        };
        // A term is 1 + 1 + 32 + 1 bytes here; the first starts at 2.
        let own_at = 2 + 3 * 35;
        let kind_of = |n: usize| 2 + n * 35 + 1;
        let unknown_kind = || Err(DecodeError::Invalid(UNKNOWN_KIND));
        let sized = || Err(DecodeError::Invalid(SIZED));
        let bad_marker = || Err(DecodeError::Invalid(BAD_MARKER));
        let marked = || Err(DecodeError::Invalid(MARKED_NOT_FILE));
        let own = |version: Option<Term>, marker: u8| three(Some(Own { version, marker })).encode();
        let mut trailing = good.clone();
        trailing.push(0);
        let mut long_count = good.clone();
        long_count.splice(1..2, [0x83, 0x00]);
        let mut cases: Vec<(&str, Vec<u8>, Result<Record, DecodeError>)> = vec![
            ("version 2", with(0, 2), Err(DecodeError::Invalid(VERSION))),
            ("version 0", with(0, 0), Err(DecodeError::Invalid(VERSION))),
            ("a kind of 0", with(kind_of(0), 0), unknown_kind()),
            ("a kind of 6", with(kind_of(1), 6), unknown_kind()),
            ("a kind of 7", with(kind_of(2), 7), unknown_kind()),
            (
                "a present flag of 2",
                with(2, 2),
                Err(DecodeError::InvalidBool(2)),
            ),
            (
                "an own flag of 2",
                with(own_at, 2),
                Err(DecodeError::InvalidBool(2)),
            ),
            (
                "an own present flag of 2",
                with(own_at + 1, 2),
                Err(DecodeError::InvalidBool(2)),
            ),
            ("a trailing byte", trailing, Err(DecodeError::TrailingBytes)),
            (
                "cut in an ID",
                good[..20].to_vec(),
                Err(DecodeError::UnexpectedEnd),
            ),
            (
                "a long count",
                long_count,
                Err(DecodeError::NonMinimalVarint),
            ),
            (
                "a folder of size 1",
                Record {
                    terms: vec![Some(term(EntryKind::Folder, 1, 1)), None, None],
                    own: None,
                }
                .encode(),
                sized(),
            ),
            (
                "a project link of size 1",
                own(Some(term(EntryKind::ProjectLink, 4, 1)), 0),
                sized(),
            ),
            ("marker 1", own(Some(file(4)), 1), bad_marker()),
            ("marker 6", own(Some(file(4)), 6), bad_marker()),
            ("marker 7 on an absent version", own(None, 7), marked()),
            (
                "marker 7 on a folder",
                own(Some(term(EntryKind::Folder, 4, 0)), 7),
                marked(),
            ),
            (
                "marker 7 on a symlink",
                own(Some(term(EntryKind::Symlink, 4, 5)), 7),
                marked(),
            ),
            (
                "marker 7 on a project link",
                own(Some(term(EntryKind::ProjectLink, 4, 0)), 7),
                marked(),
            ),
        ];
        for count in [0u64, 1, 2, 4, 256, 257] {
            let mut bytes = vec![1];
            let mut w = Writer::new();
            w.varint(count);
            bytes.extend(w.into_bytes());
            // Enough absent terms that only the count is wrong.
            bytes.extend(std::iter::repeat_n(0, count as usize));
            bytes.push(0);
            cases.push(("a term count", bytes, Err(DecodeError::Invalid(TERM_COUNT))));
        }
        for (what, bytes, refused) in cases {
            assert_eq!(Record::decode(&bytes), refused, "{what}");
        }
    }

    #[test]
    fn stands_as_takes_the_own_version_then_the_first_term() {
        let own = |version| Some(Own { version, marker: 0 });
        assert_eq!(stands_as(&three(own(Some(file(9))))), Some(file(9)));
        assert_eq!(stands_as(&three(own(None))), None);
        assert_eq!(stands_as(&three(None)), Some(file(1)));
        let mut yours_gone = three(None);
        yours_gone.terms[0] = None;
        assert_eq!(stands_as(&yours_gone), None);
    }

    #[test]
    fn the_sides_are_the_first_and_last_terms() {
        let mut r = three(None);
        assert_eq!(r.mine(), Some(file(1)));
        assert_eq!(r.before(), Some(Some(file(2))));
        assert_eq!(r.theirs(), Some(file(3)));
        assert_eq!(r.marker(), 0);
        r.terms.extend([None, Some(file(5))]);
        assert_eq!(r.before(), None);
        assert_eq!(r.theirs(), Some(file(5)));
        assert_eq!(r.id(), hash_record(Flavor::Conflict, &r.encode()));
    }

    /// Commits a save on the current line holding `files` (as `plant_save` takes them) and a
    /// conflict at each path of `conflicts`, its record stored with it. Gives the top listing's ID.
    pub(crate) fn plant_conflicted(
        repo: &mut crate::repo::Repository,
        files: &[(&str, EntryKind, &[u8])],
        conflicts: &[(&str, Record)],
    ) -> Id {
        crate::test_support::plant_with(repo, files, conflicts).id()
    }

    /// A file term holding `bytes`, which a planted save must hold for it to be read.
    pub(crate) fn file_of(bytes: &[u8]) -> Term {
        Term {
            kind: EntryKind::File,
            id: crate::id::hash_content(bytes),
            size: bytes.len() as u64,
        }
    }

    #[test]
    fn a_record_over_16_kib_is_damage_at_its_place() {
        let id = Id([3; 32]);
        let big = StoreError::RecordTooLarge {
            id,
            limit: MAX_RECORD_BYTES,
        };
        let StoreError::DamagedConflict(said) = unreadable(&at_path("art/\u{1b}.png"), &id, big)
        else {
            panic!("not damage");
        };
        assert_eq!(
            said,
            "The conflict at art/\\x1b.png names a record over 16 KiB, which Opalro never writes, \
             so this project may be damaged."
        );
    }

    #[test]
    fn only_damage_is_damage_at_a_conflicts_place() {
        let id = Id([3; 32]);
        let at = at_path("a.png");
        let io = StoreError::Pack(crate::pack::PackError::Io(io::Error::other("busy")));
        assert!(matches!(unreadable(&at, &id, io), StoreError::Pack(_)));
        let missing = unreadable(&at, &id, StoreError::Missing(Id([4; 32])));
        let StoreError::DamagedConflict(said) = missing else {
            panic!("not damage");
        };
        assert!(
            said.starts_with("The conflict at a.png can't be read (record 030303030303): "),
            "{said}"
        );
    }

    #[test]
    fn seen_follows_point_4_of_the_folder_decision() {
        let marked = three(Some(Own {
            version: Some(file(4)),
            marker: 7,
        }));
        let at = |t: Term| Some((t.kind, t.id));
        let left = |opening, closing| move |_| Ok(Left { opening, closing });
        let never = |_| -> io::Result<Left> { panic!("scanned") };
        let seen_with =
            |r: &Record, now, scan: &dyn Fn(u8) -> io::Result<Left>| seen(r, now, scan).unwrap();
        // Untouched: the marked text as written, never scanned.
        assert_eq!(seen_with(&marked, at(file(4)), &never), Seen::Untouched);
        // Markers left, and none left.
        assert_eq!(
            seen_with(&marked, at(file(5)), &left(1, 0)),
            Seen::MarkersLeft(Left {
                opening: 1,
                closing: 0
            })
        );
        assert_eq!(seen_with(&marked, at(file(5)), &left(0, 0)), Seen::Fixed);
        // Removed, or a folder there.
        assert_eq!(seen_with(&marked, None, &never), Seen::Fixed);
        let folder = Some((EntryKind::Folder, Id([5; 32])));
        assert_eq!(seen_with(&marked, folder, &never), Seen::Fixed);
        // Only the executable mark changed, markers left: scanned with the record's length.
        let exec = Some((EntryKind::Executable, Id([4; 32])));
        let asked = |len| {
            assert_eq!(len, 7);
            Ok(Left {
                opening: 2,
                closing: 2,
            })
        };
        assert!(matches!(
            seen_with(&marked, exec, &asked),
            Seen::MarkersLeft(_)
        ));
        // A version (marker 0): changed is only changed, whatever the file holds.
        let version = three(None);
        assert_eq!(seen_with(&version, at(file(1)), &never), Seen::Untouched);
        assert_eq!(
            seen_with(&version, at(file(9)), &never),
            Seen::ChangedInPlace
        );
        assert_eq!(seen_with(&version, None, &never), Seen::ChangedInPlace);
        // Standing as nothing, and nothing there.
        let gone = three(Some(Own {
            version: None,
            marker: 0,
        }));
        assert_eq!(seen_with(&gone, None, &never), Seen::Untouched);
    }

    #[test]
    fn what_decodes_encodes_back_to_the_same_bytes() {
        let seen: RefCell<(usize, usize)> = RefCell::default();
        let random = prop::collection::vec(any::<u8>(), 0..300);
        let changed =
            (any_record(), any::<prop::sample::Index>(), any::<u8>()).prop_map(|(r, at, byte)| {
                let mut bytes = r.encode();
                let i = at.index(bytes.len());
                bytes[i] = byte;
                bytes
            });
        runner(1024)
            .run(&prop_oneof![random, changed], |bytes| {
                let mut seen = seen.borrow_mut();
                match Record::decode(&bytes) {
                    Ok(r) => {
                        seen.0 += 1;
                        prop_assert_eq!(r.encode(), bytes);
                    }
                    Err(_) => seen.1 += 1,
                }
                Ok(())
            })
            .unwrap();
        let (decoded, refused) = seen.into_inner();
        assert!(decoded >= 3 && refused >= 3, "{decoded} {refused}");
    }
}
