//! Red-team session RT24 (milestone 10, second session): proofs.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;
use support::{Env, text};

fn ok(env: &Env, args: &[&str]) -> String {
    let out = env.opalro().args(args).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{args:?}: {}",
        text(&out.stderr)
    );
    text(&out.stdout)
}

/// RT24-1: `main` moved the folder `d` away; `side` changed `d/x.txt`. The combine writes nothing
/// and makes no folder `d` for the conflict, and keeping the removal leaves none, in the folder
/// or in the new save.
#[test]
fn a_combine_that_writes_nothing_makes_no_folder_a_moved_away_conflict_sits_in() {
    let env = Env::empty();
    ok(&env, &["start", "--preset", "custom"]);
    std::fs::create_dir_all(env.root().join("d")).unwrap();
    env.overwrite("d/x.txt", "one\ntwo\nthree\n");
    env.overwrite("d/y.txt", "why\n");
    ok(&env, &["save", "First"]);
    ok(&env, &["line", "new", "side"]);
    env.overwrite("d/x.txt", "one\nside\nthree\n");
    ok(&env, &["save", "Side"]);
    ok(&env, &["goto", "main"]);
    std::fs::rename(env.root().join("d"), env.root().join("e")).unwrap();
    env.overwrite("e/x.txt", "one\nmain\nthree\n");
    ok(&env, &["save", "Main"]);
    assert!(!env.root().join("d").exists());

    let said = ok(&env, &["combine", "side"]);
    assert!(said.contains("No file needed changing"), "{said}");
    let made = env.root().join("d").exists();
    ok(&env, &["resolve", "--keep", "mine", "d/x.txt"]);
    let left = env.root().join("d").exists();
    let saved = support::saved_paths(&env, support::main_tip(&env)).contains_key("d");
    assert!(
        !made && !left && !saved,
        "a folder d: after the combine {made}, after resolve --keep mine {left}, in the new save \
         {saved}\n{said}"
    );
}
