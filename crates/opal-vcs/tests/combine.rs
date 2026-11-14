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

#[test]
fn a_conflict_stops_names_the_files_and_changes_nothing() {
    let env = project();
    apart(
        &env,
        "1\nside\n3\n4\n5\n6\n7\n8\n",
        "1\nmain\n3\n4\n5\n6\n7\n8\n",
    );
    let status = run(&env, &["status"], 0).0;
    let history = run(&env, &["history"], 0).0;
    let before = env.standing();
    let (shown, said) = run(&env, &["combine", "side"], 1);
    assert_eq!(shown, "");
    assert!(said.contains("would leave 1 file in conflict"), "{said}");
    assert!(
        said.contains("\n  a.txt: changed on both lines, in the same place"),
        "{said}"
    );
    assert!(
        said.contains("hint: For each of these files, make the two lines agree and save"),
        "{said}"
    );
    assert_eq!(env.standing(), before);
    assert_eq!(run(&env, &["status"], 0).0, status);
    assert_eq!(run(&env, &["history"], 0).0, history);
    let doc = json(&env, &["combine", "side"], 1);
    let message = doc["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("\n  a.txt: changed on both lines"),
        "{doc}"
    );
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
fn a_hostile_path_in_a_conflict_is_escaped() {
    let env = imported_apart("x\u{1b}[31m\u{202e}.txt");
    let (shown, said) = run(&env, &["combine", "side"], 1);
    assert_eq!(shown, "");
    assert!(
        said.contains("x\\x1b[31m\\u{202e}.txt: changed on both lines"),
        "{said}"
    );
    support::assert_escaped(&said, "combine");
    let doc = json(&env, &["combine", "side"], 1);
    support::assert_escaped(doc["error"]["message"].as_str().unwrap(), "--json");
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
