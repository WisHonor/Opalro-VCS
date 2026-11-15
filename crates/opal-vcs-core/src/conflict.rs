//! Conflict records: every version a place in conflict holds (docs/storage-format.md section 8).

use crate::encoding::{DecodeError, Reader, Writer};
use crate::id::{Flavor, Id, hash_record};
use crate::listing::EntryKind;

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
mod tests {
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
