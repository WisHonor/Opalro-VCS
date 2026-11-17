//! The proofs of red-team session RT22 (milestone 9, conflicts, second session): one test per
//! finding, each asserting what should be true; see docs/redteam/2026-11-15-milestone-9-session-2.md.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;
use opal_core::conflict::{Record, Term};
use opal_core::diary::Position;
use opal_core::id::Id;
use opal_core::listing::{Entry, EntryKind, Listing};
use opal_core::pack::ObjectKind;
use opal_core::repo::Repository;
use support::{Env, text};

fn entry(name: &str, kind: EntryKind, id: Id, size: u64) -> Entry {
    Entry {
        name: name.into(),
        kind,
        id,
        size,
    }
}

/// RT22-1. `resolve --keep` carries a folder version through the overlay as a link entry and
/// turns back into a folder every link entry, in each listing it rebuilds, whose ID is that
/// folder's listing ID. A planted save with a conflict `d` (yours: a folder; theirs: a folder)
/// and, beside it, a link `l` whose ID is theirs's listing ID: `resolve d --keep theirs` records
/// `l` as a folder holding theirs's files, though nobody named `l`.
#[test]
fn keeping_a_folder_side_changes_no_other_entry() {
    let env = Env::empty();
    let out = env
        .opalro()
        .args(["start", "--preset", "custom"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let mut theirs = None;
    support::plant_save(env.root(), |pack| {
        let y = pack.add_content(b"mine\n").unwrap();
        let x = pack.add_content(b"theirs\n").unwrap();
        let mine_l = Listing::new(vec![entry("y.txt", EntryKind::File, y, 5)]).unwrap();
        let theirs_l = Listing::new(vec![entry("x.txt", EntryKind::File, x, 7)]).unwrap();
        let mine_id = pack
            .add_record(ObjectKind::FolderListing, &mine_l.encode())
            .unwrap();
        let theirs_id = pack
            .add_record(ObjectKind::FolderListing, &theirs_l.encode())
            .unwrap();
        theirs = Some(theirs_id);
        let folder = |id| {
            Some(Term {
                kind: EntryKind::Folder,
                id,
                size: 0,
            })
        };
        let record = Record {
            terms: vec![folder(mine_id), None, folder(theirs_id)],
            own: None,
        };
        let c = support::add_conflict(pack, &record);
        let root = Listing::new(vec![
            entry("d", EntryKind::Conflict, c, 0),
            entry("l", EntryKind::Symlink, theirs_id, 0),
        ])
        .unwrap();
        pack.add_record(ObjectKind::FolderListing, &root.encode())
            .unwrap()
    });
    support::write(&env, "d/y.txt", "mine\n");
    let out = env
        .opalro()
        .args(["resolve", "d", "--keep", "theirs"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let repo = Repository::open(env.root()).unwrap();
    let (_, e) = repo.current().unwrap();
    let Position::OnLine(line) = &e.state.position else {
        panic!("not on a line")
    };
    let save = repo.store().save(&e.state.lines[line], false).unwrap();
    let root = repo.store().listing(&save.root).unwrap();
    let l = root.entries().iter().find(|e| e.name == "l").unwrap();
    assert_eq!(l.id, theirs.unwrap());
    assert_eq!(
        l.kind,
        EntryKind::Symlink,
        "`resolve d --keep theirs` turned the link l, which nobody named, into a folder"
    );
}
