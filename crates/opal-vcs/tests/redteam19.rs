//! The proofs of red-team session RT19 (milestone 8, `opalro combine`): one test per finding,
//! each asserting what should be true; see docs/redteam/2026-11-14-milestone-8.md.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Instant;

use opal_core::diary::{Action, DiaryEntry};
use opal_core::id::{ChangeId, Id};
use opal_core::pack::ObjectKind;
use opal_core::repo::Repository;
use opal_core::save::{MoveNote, Save};

mod support;
use support::{Env, text};

fn run(env: &Env, args: &[&str]) -> (i32, String) {
    let out = env.opalro().args(args).output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        format!("{}{}", text(&out.stdout), text(&out.stderr)),
    )
}

/// A started project on `main` with `a.txt` saved.
fn project() -> Env {
    let env = Env::empty();
    assert_eq!(run(&env, &["start", "--preset", "custom"]).0, 0);
    support::write(&env, "a.txt", "a\n");
    assert_eq!(run(&env, &["save", "A"]).0, 0);
    env
}

/// Plants saves through Opalro's own library, all holding main's folder: `build` adds them with
/// the given (parents, time, notes) and names the new `main` and `side`.
fn plant(
    env: &Env,
    build: impl FnOnce(Id, &mut dyn FnMut(Vec<Id>, i64, Vec<MoveNote>) -> Id) -> (Id, Id),
) {
    let mut repo = Repository::open(env.root()).unwrap();
    let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
    let (head, entry) = repo.current().unwrap();
    let first = entry.state.lines["main"];
    let made_on = repo.store().save(&first, false).unwrap();
    let mut pack = repo.new_packs().unwrap();
    let mut add = |parents: Vec<Id>, time: i64, move_notes: Vec<MoveNote>| {
        let mut who = made_on.author.clone();
        who.time_ms = time;
        let save = Save {
            root: made_on.root,
            parents,
            author: who.clone(),
            saver: who,
            message: "Planted".into(),
            change_id: ChangeId::random().unwrap(),
            move_notes,
            signature: vec![],
            extra: made_on.extra.clone(),
        };
        pack.add_record(ObjectKind::Save, &save.encode()).unwrap()
    };
    let (main, side) = build(first, &mut add);
    let mut state = entry.state.clone();
    state.lines.insert("main".into(), main);
    state.lines.insert("side".into(), side);
    let diary = DiaryEntry {
        parent: Some(head),
        action: Action::Command,
        command: "save".into(),
        who: made_on.author,
        opal_version: "0.1.0".into(),
        state,
        folder_change: None,
    };
    repo.commit(&lock, pack, &diary).unwrap();
}

/// RT19-1. Sixteen feature lines W1..W16 are combined into V, Z is saved on V, and `main` and
/// `side` each part from Z and then combine every Wi again, two parents at a time. The W saves
/// were made on a computer whose clock ran ahead, so they are dated after Z. The lines parted
/// once, at Z, yet the walk's candidates (Z and every Wi) were counted before the exact drop, and
/// the combine was refused as "crossing too many times".
#[test]
fn lines_that_parted_once_combine_whatever_their_saves_dates() {
    let env = project();
    let t = 1_700_000_000_000;
    plant(&env, |first, add| {
        let ws: Vec<Id> = (0..16)
            .map(|_| add(vec![first], t + 100_000, vec![]))
            .collect();
        let v = add(ws.clone(), t, vec![]);
        let z = add(vec![v], t, vec![]);
        let mut tips = [z, z];
        for (i, tip) in tips.iter_mut().enumerate() {
            *tip = add(vec![*tip], t + 200_000 + i as i64, vec![]);
            for w in &ws {
                *tip = add(vec![*tip, *w], t + 300_000, vec![]);
            }
        }
        (tips[0], tips[1])
    });
    let (code, said) = run(&env, &["combine", "side"]);
    assert_eq!(code, 0, "{said}");
    assert!(said.starts_with("Combined side into main"), "{said}");
}

/// RT19-2. Fifty planted saves of 100,000 untrue move notes each made a combine slow: it checked
/// every note, and read each large save several times. A combine now checks at most a million
/// notes; past that it follows moves by contents, goes through, and says so.
#[test]
fn a_combine_past_a_million_move_notes_goes_through_and_says_so() {
    let env = project();
    assert_eq!(run(&env, &["line", "new", "side"]).0, 0);
    support::write(&env, "b.txt", "b\n");
    assert_eq!(run(&env, &["save", "B"]).0, 0);
    assert_eq!(run(&env, &["goto", "main"]).0, 0);
    let notes: Vec<MoveNote> = (0..100_000)
        .map(|i| MoveNote {
            from: format!("f{i:06}"),
            to: format!("t{i:06}"),
        })
        .collect();
    let side = env.state().1.lines["side"];
    plant(&env, |first, add| {
        let mut tip = first;
        for i in 0..50 {
            tip = add(vec![tip], 1_700_000_000_000 + i, notes.clone());
        }
        (tip, side)
    });
    let started = Instant::now();
    let (code, said) = run(&env, &["combine", "side"]);
    println!("combine took {:.1?}", started.elapsed());
    assert_eq!(code, 0, "{said}");
    assert!(
        said.contains(
            "The saves being combined hold more move notes than Opalro checks in one combine \
             (1,000,000), so past that many it followed files that moved by their contents alone."
        ),
        "{said}"
    );
}
