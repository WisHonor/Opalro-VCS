//! `opalro combine`: bringing another line's work into the line you're on.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use opal_core::id::Id;

mod support;
use support::{Env, text};

/// A started project on `main`, with `a.txt` saved as eight lines.
fn project() -> Env {
    let env = Env::empty();
    env.opalro()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    env.overwrite("a.txt", "1\n2\n3\n4\n5\n6\n7\n8\n");
    run(&env, &["save", "A"], 0);
    env
}

/// `opalro args`, which must exit with `code`; its stdout and stderr.
fn run(env: &Env, args: &[&str], code: i32) -> (String, String) {
    let out = env.opalro().args(args).output().unwrap();
    let (shown, said) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(code), "{args:?}: {shown}{said}");
    (shown, said)
}

/// `opalro args --json`, which must exit with `code`; its document.
fn json(env: &Env, args: &[&str], code: i32) -> serde_json::Value {
    let mut args = args.to_vec();
    args.push("--json");
    serde_json::from_str(&run(env, &args, code).0).unwrap()
}

fn tip(env: &Env, line: &str) -> Id {
    env.state().1.lines[line]
}

/// Line `side` from `main`'s save, with `a.txt` as `side_a` saved on it, then `main` with `a.txt`
/// as `main_a` saved on it; the project is back on `main`.
fn apart(env: &Env, side_a: &str, main_a: &str) {
    run(env, &["line", "new", "side"], 0);
    env.overwrite("a.txt", side_a);
    run(env, &["save", "On side"], 0);
    run(env, &["goto", "main"], 0);
    env.overwrite("a.txt", main_a);
    run(env, &["save", "On main"], 0);
}

/// `side` ahead of `main` by one save that adds `b.txt`; the project is back on `main`.
fn side_ahead(env: &Env) {
    run(env, &["line", "new", "side"], 0);
    env.overwrite("b.txt", "b\n");
    run(env, &["save", "B"], 0);
    run(env, &["goto", "main"], 0);
}

#[test]
fn combine_makes_a_combined_save_and_says_so() {
    let env = project();
    apart(
        &env,
        "one\n2\n3\n4\n5\n6\n7\n8\n",
        "1\n2\n3\n4\n5\n6\n7\neight\n",
    );
    let (main, side) = (tip(&env, "main"), tip(&env, "side"));
    let (shown, said) = run(&env, &["combine", "side"], 0);
    assert!(
        shown.starts_with("Combined side into main: save "),
        "{shown}"
    );
    assert!(
        shown.ends_with(
            " \"Combine side into main\".\n\n    Merged 1 file line by line.\n    1 file written.\n"
        ),
        "{shown}"
    );
    assert_eq!(said, "");
    let history = json(&env, &["history"], 0);
    assert_eq!(history["saves"][0]["message"], "Combine side into main");
    let (repo, state) = env.state();
    let made = repo.store().save(&state.lines["main"], false).unwrap();
    assert_eq!(made.parents, [main, side]);
    assert_eq!(made.line(), Some("main"));
    drop(repo);
    assert_eq!(
        std::fs::read_to_string(env.root().join("a.txt")).unwrap(),
        "one\n2\n3\n4\n5\n6\n7\neight\n"
    );
    let (_, entry) = support::head(&env);
    assert_eq!(entry.command, "combine");
}

#[test]
fn combine_moves_a_line_forward_when_that_is_all_it_takes() {
    let env = project();
    side_ahead(&env);
    let (shown, _) = run(&env, &["combine", "side"], 0);
    assert!(
        shown.starts_with("Moved main forward to side: save "),
        "{shown}"
    );
    assert!(
        shown.contains(" \"B\".\n\n    main had nothing side lacks, so no combined save was needed.\n    1 file written.\n"),
        "{shown}"
    );
    assert!(!shown.contains("your message"), "{shown}");
    assert_eq!(tip(&env, "main"), tip(&env, "side"));
    assert!(env.root().join("b.txt").exists());
    run(&env, &["undo"], 0);
    let (shown, _) = run(&env, &["combine", "side", "Wall running is in"], 0);
    assert!(
        shown.contains("\n    No save was made, so your message wasn't used.\n"),
        "{shown}"
    );
}

#[test]
fn combining_twice_says_nothing_to_combine() {
    let env = project();
    side_ahead(&env);
    run(&env, &["combine", "side"], 0);
    let before = env.standing();
    let (shown, said) = run(&env, &["combine", "side"], 0);
    assert_eq!(
        shown,
        "Nothing to combine: main already has everything side has.\n"
    );
    assert_eq!(said, "");
    assert_eq!(env.standing(), before);
}

const ONLY_IF_CLEAN_HINT: &str =
    "Run it again without --only-if-clean to keep the conflicts and fix them one at a time.";

#[test]
fn only_if_clean_stops_and_changes_nothing() {
    let env = project();
    apart(
        &env,
        "1\nside\n3\n4\n5\n6\n7\n8\n",
        "1\nmain\n3\n4\n5\n6\n7\n8\n",
    );
    let status = run(&env, &["status"], 0).0;
    let history = run(&env, &["history"], 0).0;
    let before = env.standing();
    let (shown, said) = run(&env, &["combine", "side", "--only-if-clean"], 1);
    assert_eq!(shown, "");
    assert_eq!(
        said,
        format!(
            "error: Combining side into main would leave 1 place in conflict, so nothing was \
             changed (--only-if-clean):\n  a.txt: changed on both lines, in the same place\n\n\
             hint: {ONLY_IF_CLEAN_HINT}\n"
        )
    );
    assert_eq!(env.standing(), before);
    assert_eq!(run(&env, &["status"], 0).0, status);
    assert_eq!(run(&env, &["history"], 0).0, history);
    let doc = json(&env, &["combine", "side", "--only-if-clean"], 1);
    let message = doc["error"]["message"].as_str().unwrap();
    assert!(
        message.ends_with("(--only-if-clean):\n  a.txt: changed on both lines, in the same place"),
        "{doc}"
    );
    assert_eq!(doc["error"]["hint"], ONLY_IF_CLEAN_HINT);
    assert_eq!(env.standing(), before);
}

/// `main` and `side` apart from one save holding `a.txt`, `pic.png` and `gone.txt`: both change
/// `a.txt`'s second line and the picture, `side` changes `gone.txt` and `main` removes it.
fn three_conflicts(env: &Env) {
    support::write(env, "pic.png", b"\x89PNG\r\n\x1a\n\0start");
    env.overwrite("gone.txt", "gone\n");
    run(env, &["save", "More"], 0);
    run(env, &["line", "new", "side"], 0);
    env.overwrite("a.txt", "1\nside\n3\n4\n5\n6\n7\n8\n");
    support::write(env, "pic.png", b"\x89PNG\r\n\x1a\n\0side");
    env.overwrite("gone.txt", "changed\n");
    run(env, &["save", "On side"], 0);
    run(env, &["goto", "main"], 0);
    env.overwrite("a.txt", "1\nmain\n3\n4\n5\n6\n7\n8\n");
    support::write(env, "pic.png", b"\x89PNG\r\n\x1a\n\0main");
    std::fs::remove_file(env.root().join("gone.txt")).unwrap();
    run(env, &["save", "On main"], 0);
}

const THREE_ROWS: &str = "  a.txt: changed on both lines, in the same place; marked in the file \
                          (1 place)\n  gone.txt: changed on side, removed on main; nothing is in \
                          your folder\n  pic.png: changed on both lines, and isn't text, so it \
                          can't be merged line by line; your version is in the folder\n";

/// How `opalro status --json` says each place in conflict stands.
fn standing(env: &Env) -> Vec<(String, String)> {
    let doc = json(env, &["status"], 0);
    let conflicts = doc["conflicts"].as_array().unwrap();
    conflicts
        .iter()
        .map(|c| {
            let path = c["path"].as_str().unwrap().to_owned();
            (path, c["standing"].as_str().unwrap().to_owned())
        })
        .collect()
}

#[test]
fn combine_keeps_conflicts_and_says_where() {
    let env = project();
    three_conflicts(&env);
    let (shown, said) = run(&env, &["combine", "side"], 0);
    assert!(
        shown.starts_with("Combined side into main: save "),
        "{shown}"
    );
    assert!(
        shown.contains(&format!(
            " \"Combine side into main\", with 3 places in conflict:\n{THREE_ROWS}\n    1 file \
             written.\n"
        )),
        "{shown}"
    );
    assert!(!shown.contains("held"), "{shown}");
    assert_eq!(said, "");
    let main = tip(&env, "main");
    let (repo, _) = env.state();
    assert_eq!(repo.store().save(&main, false).unwrap().parents.len(), 2);
    drop(repo);
    let marked = std::fs::read_to_string(env.root().join("a.txt")).unwrap();
    assert!(marked.starts_with("1\n<<<<<<< "), "{marked}");
    assert!(
        marked.contains("\nmain\n") && marked.contains("\nside\n"),
        "{marked}"
    );
    assert_eq!(
        std::fs::read(env.root().join("pic.png")).unwrap(),
        b"\x89PNG\r\n\x1a\n\0main"
    );
    assert!(!env.root().join("gone.txt").exists());
    support::assert_folder_is_goto_of(&env, main);
    let want = [
        ("a.txt", "markers"),
        ("gone.txt", "nothing"),
        ("pic.png", "yours"),
    ]
    .map(|(p, s)| (p.to_owned(), s.to_owned()));
    assert_eq!(standing(&env), want);
    let (status, _) = run(&env, &["status"], 0);
    assert!(status.contains("In conflict (3 places):"), "{status}");
    let (_, entry) = support::head(&env);
    assert_eq!(entry.command, "combine");
}

#[test]
fn a_combine_with_conflicts_is_one_step_undo_takes_back() {
    let env = project();
    three_conflicts(&env);
    let main = tip(&env, "main");
    let status = run(&env, &["status"], 0).0;
    let folder = support::files(env.root());
    run(&env, &["combine", "side"], 0);
    let (shown, _) = run(&env, &["undo"], 0);
    assert!(
        shown.starts_with("Undid combine: main is back at "),
        "{shown}"
    );
    assert_eq!(tip(&env, "main"), main);
    assert_eq!(support::files(env.root()), folder);
    assert_eq!(run(&env, &["status"], 0).0, status);
    assert_eq!(standing(&env), []);
}

#[test]
fn without_the_feature_the_combine_stops_and_names_setup_upgrade() {
    let env = project();
    three_conflicts(&env);
    support::without_conflicts_feature(env.root());
    let before = env.standing();
    let folder = support::files(env.root());
    let (shown, said) = run(&env, &["combine", "side"], 1);
    assert_eq!(shown, "");
    let rows = THREE_ROWS
        .lines()
        .map(|row| row.split("; ").next().unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    let hint = "opalro setup --upgrade lets this project keep conflicts (Opalro 0.1 can't open it \
                afterwards). Or make the two lines agree on these files, save, and run opalro \
                combine again.";
    assert_eq!(
        said,
        format!(
            "error: Combining side into main would leave 3 places in conflict, which this project \
             can't keep yet, so nothing was changed:\n{rows}\n\nhint: {hint}\n"
        )
    );
    assert_eq!(env.standing(), before);
    assert_eq!(support::files(env.root()), folder);
    assert_eq!(
        std::fs::read_to_string(env.root().join(".opalro/format")).unwrap(),
        support::V01_FORMAT
    );
    let doc = json(&env, &["combine", "side"], 1);
    assert_eq!(doc["error"]["hint"], hint);
    assert_eq!(env.standing(), before);
    // Asked for a clean line, it says the same: running it again without the flag won't help.
    let (_, flagged) = run(&env, &["combine", "side", "--only-if-clean"], 1);
    assert_eq!(flagged, said);
    assert_eq!(env.standing(), before);
}

#[test]
fn combining_a_line_that_holds_a_conflict_both_changed_is_refused() {
    let env = project();
    apart(
        &env,
        "1\nside\n3\n4\n5\n6\n7\n8\n",
        "1\nmain\n3\n4\n5\n6\n7\n8\n",
    );
    run(&env, &["combine", "side"], 0);
    run(&env, &["goto", "side"], 0);
    env.overwrite("a.txt", "1\nside again\n3\n4\n5\n6\n7\n8\n");
    run(&env, &["save", "Side again"], 0);
    run(&env, &["goto", "main"], 0);
    let before = env.standing();
    let (shown, said) = run(&env, &["combine", "side"], 1);
    assert_eq!(shown, "");
    assert_eq!(
        said,
        "error: a.txt is in conflict on main, and side changed it too, so nothing was changed.\n\n\
         hint: Fix it on the line that holds it first (opalro resolve there), then run opalro \
         combine again.\n"
    );
    assert_eq!(env.standing(), before);
}

#[test]
fn combine_json_lists_conflicts() {
    let env = project();
    three_conflicts(&env);
    let doc = json(&env, &["combine", "side"], 0);
    assert_eq!(doc["result"], "combined");
    insta::assert_json_snapshot!("kept", support::masked(doc));
    run(&env, &["undo"], 0);
    let doc = json(&env, &["combine", "side", "--only-if-clean"], 1);
    assert!(
        doc["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("Combining side into main would leave 3 places in conflict"),
        "{doc}"
    );
}

#[test]
fn the_count_is_every_place_and_the_rows_the_made_ones() {
    let env = project();
    env.overwrite("b.txt", "b\n");
    run(&env, &["save", "B"], 0);
    run(&env, &["line", "new", "other"], 0);
    env.overwrite("b.txt", "other\n");
    run(&env, &["save", "On other"], 0);
    run(&env, &["goto", "main"], 0);
    apart(
        &env,
        "1\nside\n3\n4\n5\n6\n7\n8\n",
        "1\nmain\n3\n4\n5\n6\n7\n8\n",
    );
    run(&env, &["combine", "side"], 0);
    env.overwrite("b.txt", "main\n");
    run(&env, &["save", "B on main"], 0);
    let (shown, _) = run(&env, &["combine", "other"], 0);
    assert!(
        shown.contains(
            " \"Combine other into main\", with 2 places in conflict:\n  b.txt: changed on both \
             lines, in the same place; marked in the file (1 place)\n\n    The two lines held 1 \
             more place in conflict already; the save keeps it.\n"
        ),
        "{shown}"
    );
    assert!(!shown.contains("  a.txt"), "{shown}");
    let paths: Vec<String> = standing(&env).into_iter().map(|(p, _)| p).collect();
    assert_eq!(paths, ["a.txt", "b.txt"]);
    run(&env, &["undo"], 0);
    let doc = json(&env, &["combine", "other"], 0);
    assert_eq!(doc["conflict_count"], 2, "{doc}");
    assert_eq!(doc["more_conflicts"], 0, "{doc}");
    assert_eq!(doc["holds_conflicts"], true, "{doc}");
    let conflicts = doc["conflicts"].as_array().unwrap();
    assert_eq!(conflicts.len(), 1, "{doc}");
    assert_eq!(conflicts[0]["path"], "b.txt");
    // Asked for a clean line, it names the one it would make and counts the one held.
    run(&env, &["undo"], 0);
    let before = env.standing();
    let (shown, said) = run(&env, &["combine", "other", "--only-if-clean"], 1);
    assert_eq!(shown, "");
    assert_eq!(
        said,
        format!(
            "error: Combining other into main would leave 2 places in conflict, so nothing was \
             changed (--only-if-clean):\n  b.txt: changed on both lines, in the same place\nThe \
             two lines hold 1 more in conflict already.\n\nhint: {ONLY_IF_CLEAN_HINT}\n"
        )
    );
    assert_eq!(env.standing(), before);
}

/// `main` holding the conflicts a kept combine of `side` made at `a.txt`, and at `c.txt` too
/// when `two`, and line `other` from before that combine with `b.txt` added; the project is on
/// `main`.
fn main_holds_conflicts(env: &Env, two: bool) {
    run(env, &["line", "new", "other"], 0);
    env.overwrite("b.txt", "b\n");
    run(env, &["save", "B"], 0);
    run(env, &["goto", "main"], 0);
    if two {
        env.overwrite("c.txt", "c\n");
        run(env, &["save", "C"], 0);
    }
    run(env, &["line", "new", "side"], 0);
    env.overwrite("a.txt", "1\nside\n3\n4\n5\n6\n7\n8\n");
    if two {
        env.overwrite("c.txt", "side\n");
    }
    run(env, &["save", "On side"], 0);
    run(env, &["goto", "main"], 0);
    env.overwrite("a.txt", "1\nmain\n3\n4\n5\n6\n7\n8\n");
    if two {
        env.overwrite("c.txt", "main\n");
    }
    run(env, &["save", "On main"], 0);
    run(env, &["combine", "side"], 0);
}

#[test]
fn a_combine_that_made_none_but_carries_some_says_the_lines_held_them() {
    for (two, places, them) in [(false, "1 place", "it"), (true, "2 places", "them")] {
        let env = project();
        main_holds_conflicts(&env, two);
        let (shown, _) = run(&env, &["combine", "other"], 0);
        assert!(
            shown.contains(&format!(
                " \"Combine other into main\", with {places} in conflict.\n\n    The two lines \
                 held {them} in conflict already; the save keeps {them}.\n    1 file written.\n"
            )),
            "{shown}"
        );
        run(&env, &["undo"], 0);
        let doc = json(&env, &["combine", "other"], 0);
        assert_eq!(doc["conflicts"], serde_json::json!([]), "{doc}");
        assert_eq!(doc["conflict_count"], if two { 2 } else { 1 }, "{doc}");
        assert_eq!(doc["holds_conflicts"], true, "{doc}");
    }
}

#[test]
fn only_if_clean_stops_for_a_conflict_your_line_holds() {
    let env = project();
    main_holds_conflicts(&env, false);
    let before = env.standing();
    let (shown, said) = run(&env, &["combine", "other", "--only-if-clean"], 1);
    assert_eq!(shown, "");
    assert_eq!(
        said,
        format!(
            "error: Combining other into main would leave 1 place in conflict, so nothing was \
             changed (--only-if-clean).\nThe two lines hold it in conflict already.\n\nhint: \
             {ONLY_IF_CLEAN_HINT}\n"
        )
    );
    assert_eq!(env.standing(), before);
}

#[test]
fn only_if_clean_stops_for_a_conflict_carried_from_the_other_line() {
    let env = project();
    run(&env, &["line", "new", "side"], 0);
    env.overwrite("a.txt", "1\nside\n3\n4\n5\n6\n7\n8\n");
    run(&env, &["save", "On side"], 0);
    run(&env, &["goto", "main"], 0);
    // `other` keeps a conflict with side; main stays clean and moves on.
    run(&env, &["line", "new", "other"], 0);
    env.overwrite("a.txt", "1\nother\n3\n4\n5\n6\n7\n8\n");
    run(&env, &["save", "On other"], 0);
    run(&env, &["combine", "side"], 0);
    run(&env, &["goto", "main"], 0);
    env.overwrite("d.txt", "d\n");
    run(&env, &["save", "D"], 0);
    let before = env.standing();
    let (shown, said) = run(&env, &["combine", "other", "--only-if-clean"], 1);
    assert_eq!(shown, "");
    assert_eq!(
        said,
        format!(
            "error: Combining other into main would leave 1 place in conflict, so nothing was \
             changed (--only-if-clean).\nThe two lines hold it in conflict already.\n\nhint: \
             {ONLY_IF_CLEAN_HINT}\n"
        )
    );
    assert_eq!(env.standing(), before);
}

#[test]
fn a_forward_onto_a_save_that_holds_a_conflict_says_so() {
    let env = project();
    main_holds_conflicts(&env, false);
    // `other` takes main's conflict in; main has nothing other lacks.
    run(&env, &["goto", "other"], 0);
    run(&env, &["combine", "main"], 0);
    run(&env, &["goto", "main"], 0);
    let before = env.standing();
    let (shown, said) = run(&env, &["combine", "other", "--only-if-clean"], 1);
    assert_eq!(shown, "");
    assert_eq!(
        said,
        format!(
            "error: Combining other into main would leave 1 place in conflict, so nothing was \
             changed (--only-if-clean).\nother holds it in conflict already.\n\nhint: \
             {ONLY_IF_CLEAN_HINT}\n"
        )
    );
    assert_eq!(env.standing(), before);
    let (shown, _) = run(&env, &["combine", "other"], 0);
    assert!(
        shown.starts_with("Moved main forward to other: save "),
        "{shown}"
    );
    assert!(
        shown.ends_with(
            "\n    The save your line moved to holds places in conflict: opalro status lists \
             them.\n"
        ),
        "{shown}"
    );
    run(&env, &["undo"], 0);
    let doc = json(&env, &["combine", "other"], 0);
    assert_eq!(doc["result"], "forward", "{doc}");
    assert_eq!(doc["holds_conflicts"], true, "{doc}");
    assert_eq!(doc["conflict_count"], serde_json::Value::Null, "{doc}");
}

#[test]
fn only_if_clean_with_nothing_to_combine_on_a_line_in_conflict_says_where_to_go() {
    let env = project();
    main_holds_conflicts(&env, false);
    let before = env.standing();
    let marked = std::fs::read(env.root().join("a.txt")).unwrap();
    // `side` is in main's history already: nothing to combine, but main holds the conflict.
    let (shown, said) = run(&env, &["combine", "side", "--only-if-clean"], 1);
    assert_eq!(shown, "");
    assert_eq!(
        said,
        "error: Combining side into main would leave 1 place in conflict, so nothing was \
         changed (--only-if-clean).\nmain holds it in conflict already.\n\nhint: opalro status \
         lists what is in conflict here, and opalro resolve records your fixes.\n"
    );
    assert_eq!(env.standing(), before);
    assert_eq!(std::fs::read(env.root().join("a.txt")).unwrap(), marked);
    let doc = json(&env, &["combine", "side", "--only-if-clean"], 1);
    assert_eq!(
        doc["error"]["message"],
        "Combining side into main would leave 1 place in conflict, so nothing was changed \
         (--only-if-clean).\nmain holds it in conflict already.",
        "{doc}"
    );
    assert_eq!(
        doc["error"]["hint"],
        "opalro status lists what is in conflict here, and opalro resolve records your fixes."
    );
    assert_eq!(env.standing(), before);
}

#[test]
fn a_combine_with_conflicts_keeps_unsaved_work_in_the_way_as_before() {
    let env = project();
    apart(
        &env,
        "1\nside\n3\n4\n5\n6\n7\n8\n",
        "1\nmain\n3\n4\n5\n6\n7\n8\n",
    );
    env.overwrite("a.txt", "mine\n");
    let (head, main) = (support::head(&env).0, tip(&env, "main"));
    let (shown, said) = run(&env, &["combine", "side"], 1);
    assert_eq!(shown, "");
    assert!(
        said.starts_with(
            "error: Your unsaved changes, or files Opalro ignores, are in the way of the combine, \
             so nothing was changed:\n  a.txt\n"
        ),
        "{said}"
    );
    // As before, the combined save may be stored, but no step is taken and the line stays.
    assert_eq!((support::head(&env).0, tip(&env, "main")), (head, main));
    assert_eq!(
        std::fs::read_to_string(env.root().join("a.txt")).unwrap(),
        "mine\n"
    );
    // Work out of the way comes along, beside the conflict.
    run(&env, &["discard"], 0);
    env.overwrite("notes.txt", "notes\n");
    let (shown, _) = run(&env, &["combine", "side"], 0);
    assert!(
        shown.contains("with 1 place in conflict:\n  a.txt: changed on both lines"),
        "{shown}"
    );
    assert!(
        shown.contains("Your unsaved changes in 1 file came along."),
        "{shown}"
    );
    assert_eq!(
        std::fs::read_to_string(env.root().join("notes.txt")).unwrap(),
        "notes\n"
    );
}

/// `main` and `side` crossed: each holds a save combining the two saves where they parted, which
/// added `u.txt` differently, and each then holds `u.txt` its own way.
fn crossed_at_u(env: &Env) {
    run(env, &["line", "new", "side"], 0);
    env.overwrite("u.txt", "2\n");
    run(env, &["save", "Two"], 0);
    let two = tip(env, "side");
    run(env, &["goto", "main"], 0);
    env.overwrite("u.txt", "1\n");
    run(env, &["save", "One"], 0);
    let one = tip(env, "main");
    env.overwrite("u.txt", "mine\n");
    run(env, &["save", "Mine"], 0);
    support::plant_combined(env.root(), "Combined two", &[two]);
    // Planting goes onto `main`, so `side` takes its name for a moment.
    run(env, &["goto", "side"], 0);
    run(env, &["line", "rename", "main", "away"], 0);
    run(env, &["line", "rename", "main"], 0);
    env.overwrite("u.txt", "theirs\n");
    run(env, &["save", "Theirs"], 0);
    support::plant_combined(env.root(), "Combined one", &[one]);
    run(env, &["line", "rename", "side"], 0);
    run(env, &["line", "rename", "away", "main"], 0);
    run(env, &["goto", "main"], 0);
}

#[test]
fn a_combine_whose_starting_points_disagree_says_it_cant_tell() {
    let env = project();
    crossed_at_u(&env);
    let (shown, _) = run(&env, &["combine", "side"], 0);
    assert!(
        shown.contains(
            "with 1 place in conflict:\n  u.txt: changed on both lines, and Opalro can't tell \
             what it held where they parted; marked in the file (1 place)\n"
        ),
        "{shown}"
    );
    let marked = std::fs::read_to_string(env.root().join("u.txt")).unwrap();
    assert!(
        marked.contains("\n||||||| where the lines parted, which Opalro can't tell here\n"),
        "{marked}"
    );
    assert_eq!(standing(&env), [("u.txt".to_owned(), "markers".to_owned())]);
    support::assert_folder_is_goto_of(&env, tip(&env, "main"));
}

#[test]
fn combine_at_a_save_is_refused() {
    let env = project();
    side_ahead(&env);
    let first = tip(&env, "main").to_hex();
    run(&env, &["goto", &first[..12]], 0);
    let before = env.standing();
    let (shown, said) = run(&env, &["combine", "side"], 1);
    assert_eq!(shown, "");
    assert!(
        said.starts_with(
            "error: You're at a save, not on a line, so there is no line to combine into.\n"
        ),
        "{said}"
    );
    assert!(said.contains("hint: Go to a line first"), "{said}");
    assert_eq!(env.standing(), before);
}

#[test]
fn combine_takes_a_tag_and_an_id() {
    let env = project();
    apart(
        &env,
        "one\n2\n3\n4\n5\n6\n7\n8\n",
        "1\n2\n3\n4\n5\n6\n7\neight\n",
    );
    run(&env, &["tag", "v1.0", "--save", "side"], 0);
    let (shown, _) = run(&env, &["combine", "v1.0"], 0);
    assert!(
        shown.starts_with("Combined v1.0 into main: save "),
        "{shown}"
    );
    let history = json(&env, &["history"], 0);
    assert_eq!(history["saves"][0]["message"], "Combine v1.0 into main");
    run(&env, &["undo"], 0);
    // By its fingerprint, the save is named by its short change ID.
    let side = tip(&env, "side").to_hex();
    let shown_side = json(&env, &["show", &side[..12]], 0);
    let short = shown_side["save"]["change_id_short"].as_str().unwrap();
    let (shown, _) = run(&env, &["combine", &side[..12]], 0);
    assert!(
        shown.starts_with(&format!("Combined {short} into main: save ")),
        "{shown}"
    );
    let history = json(&env, &["history"], 0);
    assert_eq!(
        history["saves"][0]["message"],
        format!("Combine {short} into main")
    );
}

#[test]
fn show_finds_a_save_a_deleted_line_left_behind_a_combine() {
    let env = project();
    apart(
        &env,
        "one\n2\n3\n4\n5\n6\n7\n8\n",
        "1\n2\n3\n4\n5\n6\n7\neight\n",
    );
    let side = tip(&env, "side").to_hex();
    run(&env, &["combine", "side"], 0);
    // Shortened against main as it is after the combine, so no save on it shares the start.
    let short = json(&env, &["show", &side[..12]], 0)["save"]["change_id_short"]
        .as_str()
        .unwrap()
        .to_owned();
    run(&env, &["line", "delete", "side", "--yes"], 0);
    // Only the combined save's second parent reaches it now.
    let doc = json(&env, &["show", &short], 0);
    assert_eq!(doc["save"]["id"], side);
}

#[test]
fn unsaved_work_in_the_way_names_combine_in_the_hint() {
    let env = project();
    apart(
        &env,
        "one\n2\n3\n4\n5\n6\n7\n8\n",
        "1\n2\n3\n4\n5\n6\n7\n8\n9\n",
    );
    env.overwrite("a.txt", "mine\n");
    let (shown, said) = run(&env, &["combine", "side"], 1);
    assert_eq!(shown, "");
    assert!(
        said.contains(
            "error: Your unsaved changes, or files Opalro ignores, are in the way of the combine, \
             so nothing was changed:"
        ),
        "{said}"
    );
    assert!(said.contains("a.txt"), "{said}");
    assert!(said.contains("then run opalro combine again."), "{said}");
    assert_eq!(
        std::fs::read_to_string(env.root().join("a.txt")).unwrap(),
        "mine\n"
    );
}

#[test]
fn combine_json_matches_its_snapshot() {
    let env = project();
    apart(
        &env,
        "one\n2\n3\n4\n5\n6\n7\n8\n",
        "1\n2\n3\n4\n5\n6\n7\neight\n",
    );
    let combined = json(&env, &["combine", "side"], 0);
    assert_eq!(combined["starts"].as_array().unwrap().len(), 1);
    insta::assert_json_snapshot!("combined", support::masked(combined));
    let nothing = json(&env, &["combine", "side"], 0);
    insta::assert_json_snapshot!("nothing", support::masked(nothing));
    run(&env, &["line", "new", "ahead"], 0);
    env.overwrite("b.txt", "b\n");
    run(&env, &["save", "B"], 0);
    run(&env, &["goto", "main"], 0);
    let forward = json(&env, &["combine", "ahead", "Unused"], 0);
    insta::assert_json_snapshot!("forward", support::masked(forward));
}

#[test]
fn undo_says_what_a_combine_took_back() {
    let env = project();
    apart(
        &env,
        "one\n2\n3\n4\n5\n6\n7\n8\n",
        "1\n2\n3\n4\n5\n6\n7\neight\n",
    );
    let before = json(&env, &["history"], 0)["saves"][0]["change_id_short"]
        .as_str()
        .unwrap()
        .to_owned();
    let main = tip(&env, "main");
    run(&env, &["combine", "side"], 0);
    let (shown, _) = run(&env, &["undo"], 0);
    assert_eq!(
        shown,
        format!("Undid combine: main is back at {before}, 1 file written.\n")
    );
    assert_eq!(tip(&env, "main"), main);
    assert_eq!(
        std::fs::read_to_string(env.root().join("a.txt")).unwrap(),
        "1\n2\n3\n4\n5\n6\n7\neight\n"
    );
}

/// Imports a Git history where `main` and `side` both changed `name`, from one commit.
fn imported_apart(name: &str) -> Env {
    let env = Env::new();
    let who = "Ada Lovelace <ada@example.com> 1700000000 +0000";
    let commit = |branch: &str, mark: u32, from: &str, data: &str| {
        format!(
            "commit refs/heads/{branch}\nmark :{mark}\nauthor {who}\ncommitter {who}\ndata 2\nm\n{from}M 100644 inline {name}\ndata {}\n{data}\n",
            data.len()
        )
    };
    let stream = commit("main", 1, "", "base\n")
        + &commit("side", 2, "from :1\n", "side\n")
        + &commit("main", 3, "from :1\n", "main\n");
    // Git checks paths for Windows by default, which a name with an escape fails.
    let args = ["-c", "core.protectNTFS=false", "fast-import", "--quiet"];
    let made = support::git_with_input(env.root(), &args, stream.as_bytes());
    assert!(made.status.success(), "{}", text(&made.stderr));
    env.import();
    env
}

#[test]
fn a_hostile_path_in_a_kept_conflict_is_escaped() {
    let name = "x\u{1b}[31m\u{202e}.txt";
    let env = imported_apart(name);
    // `git fast-import` checks nothing out: the folder is brought to the save first.
    run(&env, &["discard"], 0);
    let (shown, said) = run(&env, &["combine", "side", "--only-if-clean"], 1);
    assert_eq!(shown, "");
    assert!(
        said.contains("x\\x1b[31m\\u{202e}.txt: changed on both lines"),
        "{said}"
    );
    support::assert_escaped(&said, "combine --only-if-clean");
    let doc = json(&env, &["combine", "side", "--only-if-clean"], 1);
    support::assert_escaped(doc["error"]["message"].as_str().unwrap(), "--json");
    let (shown, said) = run(&env, &["combine", "side"], 0);
    assert!(
        shown.contains(
            "\n  x\\x1b[31m\\u{202e}.txt: changed on both lines, in the same place; marked in the \
             file (1 place)\n"
        ),
        "{shown}"
    );
    support::assert_escaped(&shown, "combine");
    support::assert_escaped(&said, "combine");
    // Windows can't hold the name, so nothing is written there; elsewhere the marked text is.
    let stands = if cfg!(windows) {
        "not_here"
    } else {
        let marked = std::fs::read_to_string(env.root().join(name)).unwrap();
        assert!(marked.starts_with("<<<<<<< "), "{marked}");
        "markers"
    };
    assert_eq!(standing(&env), [(name.to_owned(), stands.to_owned())]);
}

#[test]
fn a_name_both_a_line_and_a_tag_is_refused_by_combine_too() {
    let env = Env::new();
    let first = env.commit("a.txt", "one", "one");
    env.git(&["tag", "v1", &first]);
    env.commit("a.txt", "two", "two");
    env.git(&["branch", "v1"]);
    env.import();
    let before = env.standing();
    let (_, said) = run(&env, &["combine", "v1"], 1);
    assert!(
        said.starts_with("error: \"v1\" is the name of a line and also of a tag at another save"),
        "{said}"
    );
    assert_eq!(env.standing(), before);
}

#[test]
fn a_tags_odd_name_is_quoted_in_the_outcome() {
    let env = project();
    apart(
        &env,
        "one\n2\n3\n4\n5\n6\n7\n8\n",
        "1\n2\n3\n4\n5\n6\n7\neight\n",
    );
    // Only an import brings in a tag named like this; the tag command refuses it.
    env.plant_tags(std::iter::once("odd name".to_owned()), tip(&env, "side"));
    let (shown, _) = run(&env, &["combine", "odd name"], 0);
    assert!(
        shown.starts_with("Combined \"odd name\" into main: save "),
        "{shown}"
    );
    let history = json(&env, &["history"], 0);
    assert_eq!(history["saves"][0]["message"], "Combine odd name into main");
}

/// Twelve text files of 30 MiB, each changed far apart on the two lines: the merge holds one
/// file's versions and result at a time, never all twelve results.
#[cfg(target_os = "linux")]
#[test]
fn a_combine_of_many_large_text_files_stays_within_memory() {
    const LINES: usize = (30 << 20) / 40;
    let env = Env::empty();
    run(&env, &["start", "--preset", "custom"], 0);
    let file = |n: usize, edits: &[(usize, &str)]| {
        let mut seed = n as u64 * 0x9e37_79b9 + 1;
        let mut text = String::with_capacity(LINES * 40);
        for at in 0..LINES {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            match edits.iter().find(|(line, _)| *line == at) {
                Some((_, edit)) => text += &format!("{edit:<39}\n"),
                None => text += &format!("{:039x}\n", u128::from(seed)),
            }
        }
        text
    };
    let names: Vec<String> = (0..12).map(|n| format!("big{n}.txt")).collect();
    let write = |edits: &[(usize, &str)]| {
        for (n, name) in names.iter().enumerate() {
            env.overwrite(name, &file(n, edits));
        }
    };
    write(&[]);
    run(&env, &["save", "--yes", "Base"], 0);
    run(&env, &["line", "new", "side"], 0);
    write(&[(LINES - 10, "side")]);
    run(&env, &["save", "--yes", "Side"], 0);
    run(&env, &["goto", "main"], 0);
    write(&[(10, "main")]);
    run(&env, &["save", "--yes", "Main"], 0);
    let mut cmd = env.opal_process_in(env.root());
    cmd.args(["combine", "side"]);
    let ended = support::run_limited(cmd, 600);
    assert_eq!(ended.code, Some(0), "{ended:?}");
    assert!(
        ended.peak_mb < 300,
        "a combine of 12 files of 30 MiB peaked at {} MB",
        ended.peak_mb
    );
    // Every file was merged line by line: each holds both lines' edits.
    for (n, name) in names.iter().enumerate() {
        let merged = std::fs::read_to_string(env.root().join(name)).unwrap();
        assert!(
            merged == file(n, &[(10, "main"), (LINES - 10, "side")]),
            "{name}"
        );
    }
}

/// Twelve text files of 30 MiB, each changed in the same place on the two lines: each is kept
/// with markers, written as it is made, never all twelve held at once.
#[cfg(target_os = "linux")]
#[test]
fn a_combine_of_many_large_conflicted_texts_stays_within_memory() {
    const LINES: usize = (30 << 20) / 40;
    let env = Env::empty();
    run(&env, &["start", "--preset", "custom"], 0);
    let file = |n: usize, edit: Option<&str>| {
        let mut seed = n as u64 * 0x9e37_79b9 + 1;
        let mut text = String::with_capacity(LINES * 40);
        for at in 0..LINES {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            match edit {
                Some(edit) if at == LINES / 2 => text += &format!("{edit:<39}\n"),
                _ => text += &format!("{:039x}\n", u128::from(seed)),
            }
        }
        text
    };
    let names: Vec<String> = (0..12).map(|n| format!("big{n}.txt")).collect();
    let write = |edit: Option<&str>| {
        for (n, name) in names.iter().enumerate() {
            env.overwrite(name, &file(n, edit));
        }
    };
    write(None);
    run(&env, &["save", "--yes", "Base"], 0);
    run(&env, &["line", "new", "side"], 0);
    write(Some("side"));
    run(&env, &["save", "--yes", "Side"], 0);
    run(&env, &["goto", "main"], 0);
    write(Some("main"));
    run(&env, &["save", "--yes", "Main"], 0);
    let mut cmd = env.opal_process_in(env.root());
    cmd.args(["combine", "side"]);
    let ended = support::run_limited(cmd, 600);
    assert_eq!(ended.code, Some(0), "{ended:?}");
    assert!(
        ended.peak_mb < 300,
        "a combine keeping 12 conflicts in files of 30 MiB peaked at {} MB",
        ended.peak_mb
    );
    let doc = json(&env, &["status"], 0);
    let conflicts = doc["conflicts"].as_array().unwrap();
    assert_eq!(conflicts.len(), 12, "{doc}");
    for c in conflicts {
        assert_eq!(c["standing"], "markers", "{c}");
    }
    for name in &names {
        let marked = std::fs::read_to_string(env.root().join(name)).unwrap();
        assert!(marked.contains("\n<<<<<<< "), "{name}");
    }
}

/// `main` and `side` apart, `side`'s save tagged `v1.0`, then `side` combined into `main`.
fn combined(env: &Env) {
    apart(
        env,
        "one\n2\n3\n4\n5\n6\n7\n8\n",
        "1\n2\n3\n4\n5\n6\n7\neight\n",
    );
    run(env, &["tag", "v1.0", "--save", "side"], 0);
    run(env, &["combine", "side"], 0);
}

#[test]
fn history_marks_a_combined_save() {
    let env = project();
    combined(&env);
    let side = tip(&env, "side").to_hex();
    let (shown, _) = run(&env, &["history"], 0);
    let rows: Vec<&str> = shown.lines().skip(2).collect();
    assert!(
        rows[0].ends_with(&format!(
            "Combine side into main  [main]  [combined {}]",
            &side[..12]
        )),
        "{shown}"
    );
    // The line's own saves only: what side made is behind the mark.
    assert_eq!(rows.len(), 4, "{shown}");
    assert!(!shown.contains("On side"), "{shown}");
}

#[test]
fn history_from_a_line_a_tag_or_a_save_lists_from_there() {
    let env = project();
    combined(&env);
    let side = tip(&env, "side");
    let letters = |at: usize| {
        let doc = json(&env, &["history", "side"], 0);
        doc["saves"][at]["change_id"].as_str().unwrap()[..12].to_owned()
    };
    let (first, second) = (letters(0), letters(1));
    for from in ["side", "v1.0", &side.to_hex()[..12]] {
        let (shown, _) = run(&env, &["history", from], 0);
        assert!(
            shown.starts_with(&format!(
                "Saves back from save {first} \"On side\", newest first:\n\n  {first}  "
            )),
            "{from}: {shown}"
        );
        let rows: Vec<&str> = shown.lines().skip(2).collect();
        assert_eq!(rows.len(), 3, "{from}: {shown}");
        assert!(rows[0].ends_with("On side  [side]  [tag v1.0]"), "{shown}");
        assert!(rows[1].starts_with(&format!("  {second}  ")), "{shown}");
        // The project is on main, which this listing doesn't reach.
        assert!(!shown.contains('>'), "{shown}");
        let doc = json(&env, &["history", from], 0);
        assert_eq!(doc["from"], side.to_hex(), "{doc}");
        assert_eq!(doc["saves"][0]["change_id_short"], first, "{doc}");
        assert_eq!(doc["saves"].as_array().unwrap().len(), 3, "{doc}");
    }
    // The line the project is on is listed as plain history lists it.
    assert_eq!(
        run(&env, &["history", "main"], 0),
        run(&env, &["history"], 0)
    );
    assert_eq!(json(&env, &["history"], 0)["from"], serde_json::Value::Null);
    // The place marker is on the save the project is at, wherever it is listed.
    run(&env, &["goto", "side"], 0);
    let (shown, _) = run(&env, &["history", "v1.0"], 0);
    assert!(shown.contains(&format!("\n> {first}  ")), "{shown}");
    // Read as show reads it: refused alike.
    let (_, said) = run(&env, &["history", "nothing-here"], 1);
    let (_, show_said) = run(&env, &["show", "nothing-here"], 1);
    assert_eq!(said, show_said);
}

#[test]
fn show_names_the_other_parent_on_a_combined_row() {
    let env = project();
    combined(&env);
    let (main, side) = {
        let (repo, state) = env.state();
        let made = repo.store().save(&state.lines["main"], false).unwrap();
        (made.parents[0].to_hex(), made.parents[1].to_hex())
    };
    let (shown, _) = run(&env, &["show"], 0);
    assert!(
        shown.contains(&format!(
            "\n    After        {} \"On main\"\n    Combined     {} \"On side\"\n    Fingerprint  ",
            &main[..12],
            &side[..12]
        )),
        "{shown}"
    );
}

#[test]
fn every_save_object_has_its_parents() {
    let env = project();
    combined(&env);
    let (main, side) = {
        let (repo, state) = env.state();
        let made = repo.store().save(&state.lines["main"], false).unwrap();
        (made.parents[0].to_hex(), made.parents[1].to_hex())
    };
    let history = json(&env, &["history"], 0);
    assert_eq!(
        history["saves"][0]["parents"],
        serde_json::json!([main, side])
    );
    let saves = history["saves"].as_array().unwrap();
    assert_eq!(saves.last().unwrap()["parents"], serde_json::json!([]));
    let shown = json(&env, &["show"], 0);
    assert_eq!(shown["save"]["parents"], serde_json::json!([main, side]));
    // side was made after A, the third save on main from the newest.
    let a = saves[2]["id"].as_str().unwrap().to_owned();
    let went = json(&env, &["goto", "side"], 0);
    assert_eq!(went["save"]["parents"], serde_json::json!([a]), "{went}");
}

#[test]
fn show_names_a_combined_parent_it_cannot_read_without_its_message() {
    let env = project();
    let missing = opal_core::id::hash_content(b"no such save");
    support::plant_combined(env.root(), "Combined", &[missing]);
    let (shown, said) = run(&env, &["show"], 0);
    assert!(
        shown.contains(&format!(
            " \"A\"\n    Combined     {}\n    Fingerprint  ",
            missing.short_hex()
        )),
        "{shown}"
    );
    assert_eq!(said, "");
    let doc = json(&env, &["show"], 0);
    assert_eq!(doc["save"]["parents"][1], missing.to_hex());
}

#[test]
fn show_and_history_name_every_parent_of_a_save_with_many() {
    let env = project();
    let others: Vec<Id> = (1..40)
        .map(|n| opal_core::id::hash_content(format!("parent {n}").as_bytes()))
        .collect();
    support::plant_combined(env.root(), "Octopus", &others);
    let (shown, _) = run(&env, &["history"], 0);
    let marks: String = (others.iter())
        .map(|p| format!("  [combined {}]", p.short_hex()))
        .collect();
    let row = shown.lines().nth(2).unwrap();
    assert!(row.ends_with(&format!("Octopus  [main]{marks}")), "{shown}");
    let (shown, _) = run(&env, &["show"], 0);
    let rows: Vec<String> = (others.iter().enumerate())
        .map(|(at, p)| {
            let label = if at == 0 { "Combined" } else { "" };
            format!("    {label:<13}{}\n", p.short_hex())
        })
        .collect();
    assert!(
        shown.contains(&format!(" \"A\"\n{}    Fingerprint  ", rows.concat())),
        "{shown}"
    );
    let doc = json(&env, &["history"], 0);
    assert_eq!(doc["saves"][0]["parents"].as_array().unwrap().len(), 40);
}

const BROKEN_RULES: &str = "[[rule]\nfiles = [\"*.unity\"]\n";
const UNTIL_FIXED: &str =
    "Until it is fixed or removed, opalro combine merges no file line by line.";

#[test]
fn while_opalro_toml_cant_be_read_whole_a_combine_merges_no_file_line_by_line() {
    let env = project();
    env.overwrite("opalro.toml", BROKEN_RULES);
    run(&env, &["save", "Rules"], 0);
    apart(
        &env,
        "one\n2\n3\n4\n5\n6\n7\n8\n",
        "1\n2\n3\n4\n5\n6\n7\neight\n",
    );
    let before = env.standing();
    let (shown, said) = run(&env, &["combine", "side"], 1);
    assert_eq!(shown, "");
    assert!(
        said.starts_with(
            "error: Combining side into main would have to merge 1 place that opalro.toml \
             decides, and it can't be read whole, so nothing was changed:\n  a.txt: changed on \
             both lines, and opalro.toml can't be read whole, so no file is merged line by line\n"
        ),
        "{said}"
    );
    assert!(!said.contains("keep yet"), "{said}");
    assert!(
        said.contains(
            "hint: opalro setup says what is wrong with opalro.toml. Fix or remove it and save; \
             then run opalro combine again."
        ),
        "{said}"
    );
    assert!(!said.contains("warning:"), "{said}");
    assert_eq!(env.standing(), before);
    let doc = json(&env, &["combine", "side"], 1);
    let message = doc["error"]["message"].as_str().unwrap();
    assert!(
        message.ends_with(
            "\n  a.txt: changed on both lines, and opalro.toml can't be read whole, so no file is \
             merged line by line"
        ),
        "{doc}"
    );
    assert_eq!(
        doc["error"]["hint"],
        "opalro setup says what is wrong with opalro.toml. Fix or remove it and save; then run \
         opalro combine again."
    );
    assert_eq!(doc["warnings"], serde_json::json!([]), "{doc}");
    assert_eq!(env.standing(), before);
}

fn warning_kinds(doc: &serde_json::Value) -> Vec<&str> {
    let warnings = doc["warnings"].as_array().unwrap();
    warnings
        .iter()
        .map(|w| w["kind"].as_str().unwrap())
        .collect()
}

#[test]
fn a_combine_that_brings_in_a_broken_opalro_toml_says_so() {
    let env = project();
    run(&env, &["line", "new", "side"], 0);
    env.overwrite("opalro.toml", BROKEN_RULES);
    run(&env, &["save", "Rules"], 0);
    run(&env, &["goto", "main"], 0);
    env.overwrite("a.txt", "1\n2\n3\n4\n5\n6\n7\neight\n");
    run(&env, &["save", "On main"], 0);
    let (shown, said) = run(&env, &["combine", "side"], 0);
    assert!(shown.starts_with("Combined side into main"), "{shown}");
    assert!(
        said.starts_with("warning: Opalro passed over opalro.toml: it isn't TOML"),
        "{said}"
    );
    assert!(said.contains(UNTIL_FIXED), "{said}");
    assert_eq!(said.matches("warning:").count(), 1, "{said}");
    run(&env, &["undo"], 0);
    let doc = json(&env, &["combine", "side"], 0);
    assert_eq!(doc["result"], "combined");
    assert_eq!(warning_kinds(&doc), ["rules_file_skipped"], "{doc}");
    let message = doc["warnings"][0]["message"].as_str().unwrap();
    assert!(message.ends_with(UNTIL_FIXED), "{message}");
}

#[test]
fn a_line_moved_forward_onto_a_broken_opalro_toml_says_so() {
    let env = project();
    run(&env, &["line", "new", "side"], 0);
    env.overwrite("opalro.toml", "colour = \"red\"\n");
    run(&env, &["save", "Rules"], 0);
    run(&env, &["goto", "main"], 0);
    let (shown, said) = run(&env, &["combine", "side"], 0);
    assert!(shown.starts_with("Moved main forward to side"), "{shown}");
    assert!(
        said.starts_with("warning: Opalro passed over these parts of opalro.toml"),
        "{said}"
    );
    assert!(
        said.contains(
            "Fix or remove them, or update Opalro if a newer one wrote them. Until then, opalro \
             combine merges no file line by line."
        ),
        "{said}"
    );
    run(&env, &["undo"], 0);
    let doc = json(&env, &["combine", "side"], 0);
    assert_eq!(doc["result"], "forward");
    assert_eq!(warning_kinds(&doc), ["rules_ignored"], "{doc}");
}

#[test]
fn a_combine_that_brings_in_the_fixed_opalro_toml_says_nothing_of_it() {
    let env = project();
    env.overwrite("opalro.toml", BROKEN_RULES);
    run(&env, &["save", "Rules"], 0);
    run(&env, &["line", "new", "side"], 0);
    env.overwrite("opalro.toml", "[[rule]]\nfiles = [\"*.unity\"]\n");
    run(&env, &["save", "Fixed"], 0);
    run(&env, &["goto", "main"], 0);
    env.overwrite("a.txt", "1\n2\n3\n4\n5\n6\n7\neight\n");
    run(&env, &["save", "On main"], 0);
    let (shown, said) = run(&env, &["combine", "side"], 0);
    assert!(shown.starts_with("Combined side into main"), "{shown}");
    assert_eq!(said, "");
    run(&env, &["undo"], 0);
    let doc = json(&env, &["combine", "side"], 0);
    assert_eq!(doc["warnings"], serde_json::json!([]), "{doc}");
}

/// A scene on `main`; line `art` adds a rule that scenes are never merged line by line and edits
/// the scene near its end, `main` edits it near its start; the project is back on `main`.
fn scene_ruled_on_art(env: &Env) {
    let scene: String = (0..20).map(|i| format!("line {i}\n")).collect();
    env.overwrite("Main.unity", &scene);
    run(env, &["save", "Scene"], 0);
    run(env, &["line", "new", "art"], 0);
    env.overwrite(
        "opalro.toml",
        "[[rule]]\nfiles = [\"*.unity\"]\nconflict-markers = false\n",
    );
    env.overwrite("Main.unity", &scene.replace("line 18\n", "art\n"));
    run(env, &["save", "Scenes are never merged"], 0);
    run(env, &["goto", "main"], 0);
    env.overwrite("Main.unity", &scene.replace("line 1\n", "main\n"));
    run(env, &["save", "On main"], 0);
}

#[test]
fn a_rule_only_the_line_combined_in_has_stops_the_combine_and_names_that_line() {
    let env = project();
    scene_ruled_on_art(&env);
    let row = "\n  Main.unity: changed on both lines, and opalro.toml on art says conflict-markers = \
               false for it";
    let before = env.standing();
    let (shown, said) = run(&env, &["combine", "art", "--only-if-clean"], 1);
    assert_eq!(shown, "");
    assert!(said.contains(&format!("{row}\n")), "{said}");
    assert!(
        said.contains(&format!("hint: {ONLY_IF_CLEAN_HINT}")),
        "{said}"
    );
    assert_eq!(env.standing(), before);
    let doc = json(&env, &["combine", "art", "--only-if-clean"], 1);
    let message = doc["error"]["message"].as_str().unwrap();
    assert!(message.ends_with(row), "{doc}");
    assert_eq!(doc["error"]["hint"], ONLY_IF_CLEAN_HINT);
    assert_eq!(env.standing(), before);
    // Kept, the scene stays whole, as main has it.
    let (shown, _) = run(&env, &["combine", "art"], 0);
    assert!(
        shown.contains(&format!("{row}; your version is in the folder\n")),
        "{shown}"
    );
    let doc = json(&env, &["status"], 0);
    assert_eq!(doc["conflicts"][0]["standing"], "yours", "{doc}");
}

#[test]
fn the_same_two_lines_combined_the_other_way_round_clash_in_the_folders_words() {
    let env = project();
    scene_ruled_on_art(&env);
    run(&env, &["goto", "art"], 0);
    let row = "\n  Main.unity: changed on both lines, and opalro.toml says conflict-markers = false \
               for it";
    let (_, said) = run(&env, &["combine", "main", "--only-if-clean"], 1);
    assert!(said.contains(&format!("{row}\n")), "{said}");
    let doc = json(&env, &["combine", "main", "--only-if-clean"], 1);
    let message = doc["error"]["message"].as_str().unwrap();
    assert!(message.ends_with(row), "{doc}");
    assert_eq!(doc["error"]["hint"], ONLY_IF_CLEAN_HINT);
}

#[test]
fn starting_points_that_cant_be_combined_while_opalro_toml_cant_be_read_whole_say_so() {
    let env = project();
    apart(
        &env,
        "one\n2\n3\n4\n5\n6\n7\n8\n",
        "1\n2\n3\n4\n5\n6\n7\neight\n",
    );
    let main = tip(&env, "main").to_hex();
    // Crossed: each line combines the other's save, so the two saves are both starting points.
    run(&env, &["combine", "side"], 0);
    run(&env, &["goto", "side"], 0);
    run(&env, &["combine", &main], 0);
    env.overwrite("opalro.toml", "colour = \"red\"\n");
    env.overwrite("s.txt", "s\n");
    run(&env, &["save", "S"], 0);
    run(&env, &["goto", "main"], 0);
    env.overwrite("opalro.toml", "colour = \"red\"\n");
    env.overwrite("m.txt", "m\n");
    run(&env, &["save", "M"], 0);
    let before = env.standing();
    let (shown, said) = run(&env, &["combine", "side"], 1);
    assert_eq!(shown, "");
    assert!(
        said.contains(
            "error: Combining side into main needs their common starting points combined first, \
             and opalro.toml would have to decide a file there but can't be read whole, so \
             nothing was changed.\n"
        ),
        "{said}"
    );
    assert!(
        said.contains(
            "hint: opalro setup says what is wrong with opalro.toml. Fix or remove it and save; \
             then run opalro combine again."
        ),
        "{said}"
    );
    let doc = json(&env, &["combine", "side"], 1);
    assert_eq!(
        doc["error"]["message"],
        "Combining side into main needs their common starting points combined first, and \
         opalro.toml would have to decide a file there but can't be read whole, so nothing was \
         changed."
    );
    assert_eq!(
        doc["error"]["hint"],
        "opalro setup says what is wrong with opalro.toml. Fix or remove it and save; then run \
         opalro combine again."
    );
    assert_eq!(env.standing(), before);
}
