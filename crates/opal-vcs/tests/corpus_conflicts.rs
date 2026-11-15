//! Hostile conflict records written out (security.md T53): each case plants a save on `main` whose
//! conflict a hostile `.opalro` could hold, runs `opalro goto main` onto it, and checks that it is
//! refused as damage naming its place, or written within write-out's limits, and that nothing
//! reached the folder beside the project or went into `.git` or `.opalro`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use opal_core::conflict::{Record, Term};
use opal_core::id::{Id, hash_content};
use opal_core::listing::{EntryKind, Listing};
use opal_core::pack::{ObjectKind, PackSet};
use support::corpus::{Case, PAYLOAD, project_entries};
use support::{add_conflict, add_listing, listings, listings_with_conflicts, plant_save};

const THEIRS: &[u8] = b"theirs\n";

/// Yours (`mine`), none where the lines parted, and theirs.
fn record(mine: Term) -> Record {
    Record {
        terms: vec![Some(mine), None, Some(term(EntryKind::File, THEIRS))],
        own: None,
    }
}

fn term(kind: EntryKind, bytes: &[u8]) -> Term {
    Term {
        kind,
        id: hash_content(bytes),
        size: bytes.len() as u64,
    }
}

fn folder(id: Id) -> Term {
    Term {
        kind: EntryKind::Folder,
        id,
        size: 0,
    }
}

/// Plants on `main` a save holding `ok.txt` and a conflict at `art/c`, its record what `build`
/// stores and gives the ID of.
fn plant(case: &Case, build: impl FnOnce(&mut PackSet) -> Id) {
    plant_save(case.root(), |pack| {
        pack.add_content(THEIRS).unwrap();
        let conflict = build(pack);
        let ok = pack.add_content(b"fine").unwrap();
        let art = add_listing(pack, &["c".into()], EntryKind::Conflict, conflict);
        let top = Listing::new(vec![
            opal_core::listing::Entry {
                name: "art".into(),
                kind: EntryKind::Folder,
                id: art,
                size: 0,
            },
            opal_core::listing::Entry {
                name: "ok.txt".into(),
                kind: EntryKind::File,
                id: ok,
                size: 4,
            },
        ])
        .unwrap();
        pack.add_record(ObjectKind::FolderListing, &top.encode())
            .unwrap()
    });
}

/// `opalro goto main` refused: exit 1, `place` named, `art/c` never written, nothing escaped.
/// Refused while planning, the folder is as it was; met while writing, as for any save naming
/// what the store doesn't hold, the switch stops there.
fn refused(case: &Case, place: &str, planning: bool) -> String {
    let before = project_entries(&case.env);
    let (code, doc, _) = case.run_json(&["goto", "main"]);
    assert_eq!(code, Some(1), "{doc}");
    let said = doc["error"]["message"].as_str().unwrap().to_owned();
    assert!(said.contains(place), "{doc}");
    let after = project_entries(&case.env);
    if planning {
        assert_eq!(after, before);
    }
    assert!(!after.contains_key("art/c"), "{after:?}");
    case.assert_nothing_escaped();
    said
}

#[test]
fn a_term_naming_a_missing_object_stops_the_switch_at_its_place() {
    let case = Case::new();
    plant(&case, |pack| {
        let missing = term(EntryKind::File, b"never stored");
        add_conflict(pack, &record(missing))
    });
    let said = refused(&case, "stopped at art/c", false);
    assert!(said.contains("is not in the store"), "{said}");
}

#[test]
fn a_file_term_naming_a_listing_stops_the_switch_at_its_place() {
    let case = Case::new();
    plant(&case, |pack| {
        let inside = listings(pack, &[("pwned", EntryKind::File, PAYLOAD)]);
        let mine = Term {
            kind: EntryKind::File,
            id: inside,
            size: 9,
        };
        add_conflict(pack, &record(mine))
    });
    let said = refused(&case, "stopped at art/c", false);
    assert!(said.contains("is FolderListing, not Content"), "{said}");
}

#[test]
fn a_folder_term_3000_deep_is_held_to_a_saves_limits() {
    let case = Case::new();
    plant(&case, |pack| {
        let mut id = pack
            .add_record(ObjectKind::FolderListing, &Listing::empty().encode())
            .unwrap();
        for _ in 0..3000 {
            id = add_listing(pack, &["d".into()], EntryKind::Folder, id);
        }
        add_conflict(pack, &record(folder(id)))
    });
    let said = refused(&case, "too large", true);
    assert!(
        !said.contains("d/d/d/d/d/d/d/d/d/d/d/d/d/d/d/d/d"),
        "{said}"
    );
}

#[test]
fn a_record_over_16_kib_is_refused_as_damage_naming_its_place() {
    let case = Case::new();
    plant(&case, |pack| {
        pack.add_record(ObjectKind::Conflict, &vec![7u8; 17 * 1024])
            .unwrap()
    });
    let said = refused(&case, "c in folder listing ", true);
    assert!(said.contains("over 16 KiB"), "{said}");
}

#[test]
fn a_conflict_inside_a_folder_a_conflict_stands_as_is_refused_as_damage() {
    let case = Case::new();
    plant(&case, |pack| {
        let inner = record(term(EntryKind::File, THEIRS));
        let held = listings_with_conflicts(pack, &[], &[("deeper", &inner)]);
        add_conflict(pack, &record(folder(held)))
    });
    let said = refused(&case, "deeper", true);
    assert!(said.contains("is a conflict inside a conflict"), "{said}");
}

#[test]
fn a_folder_term_within_limits_is_written_as_any_folder() {
    let case = Case::new();
    plant(&case, |pack| {
        let held = listings(pack, &[("x.txt", EntryKind::File, b"in")]);
        add_conflict(pack, &record(folder(held)))
    });
    let (code, doc, said) = case.run_json(&["goto", "main"]);
    assert_eq!(code, Some(0), "{doc}{said}");
    assert_eq!(doc["holds_conflicts"], false, "{doc}");
    let entries = project_entries(&case.env);
    assert_eq!(
        entries.get("art/c/x.txt"),
        Some(&support::corpus::file(b"in"))
    );
    case.assert_nothing_escaped();
}
