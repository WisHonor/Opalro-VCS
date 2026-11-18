//! Red-team session RT25 (milestone 11): proofs.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;
use support::{Env, text, write};

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

/// RT25-2: `side` holds an empty folder `p` where `main` holds the file `p`. `goto side` removed
/// the file to make the folder, and said `No file needed changing.`; `--json` said nothing was
/// removed. The file is safe in `main`'s save, but the person was told nothing left the folder.
/// Back on `main`, the folder that gave way to the file wasn't counted either.
#[test]
fn a_switch_that_puts_a_folder_in_a_files_place_counts_the_file_it_removes() {
    let env = Env::empty();
    ok(&env, &["start", "--preset", "custom"]);
    write(&env, "p", "p\n");
    ok(&env, &["save", "A file"]);
    ok(&env, &["line", "new", "side"]);
    std::fs::remove_file(env.root().join("p")).unwrap();
    std::fs::create_dir(env.root().join("p")).unwrap();
    ok(&env, &["save", "A folder"]);
    ok(&env, &["goto", "main"]);
    assert!(env.root().join("p").is_file());

    let said = ok(&env, &["goto", "side"]);
    assert!(env.root().join("p").is_dir());
    ok(&env, &["undo"]);
    assert!(env.root().join("p").is_file());
    let doc: serde_json::Value =
        serde_json::from_str(&ok(&env, &["goto", "side", "--json"])).unwrap();
    let says = |said: &str, line: &str| said.lines().any(|l| l.trim() == line);
    assert!(
        says(&said, "1 file removed.") && doc["removed"] == 1 && doc["written"] == 0,
        "the file p was removed, but goto said: {said}and --json gave removed: {}",
        doc["removed"]
    );

    // The other way round: the folder gives way to the file.
    let said = ok(&env, &["goto", "main"]);
    assert!(env.root().join("p").is_file());
    ok(&env, &["undo"]);
    assert!(env.root().join("p").is_dir());
    let doc: serde_json::Value =
        serde_json::from_str(&ok(&env, &["goto", "main", "--json"])).unwrap();
    assert!(
        says(&said, "1 file written, 1 removed.") && doc["removed"] == 1 && doc["written"] == 1,
        "the folder p was removed, but goto said: {said}and --json gave removed: {}",
        doc["removed"]
    );
}
