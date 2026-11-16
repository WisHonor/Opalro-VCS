//! `opalro resolve`: recording the places in conflict a person fixed, keeping one side whole, and
//! copying one version out to look at. Conflicts are made by combining.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

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
    run_in(env, env.root(), args, code)
}

fn run_in(env: &Env, dir: &Path, args: &[&str], code: i32) -> (String, String) {
    let out = env.opal_in(dir).args(args).output().unwrap();
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

/// The short change ID and message of the newest save on the line.
fn newest(env: &Env) -> (String, String) {
    let history = json(env, &["history"], 0);
    let save = &history["saves"][0];
    (
        save["change_id_short"].as_str().unwrap().to_owned(),
        save["message"].as_str().unwrap().to_owned(),
    )
}

/// `main` and `side` changed `a.txt`'s second line apart, combined: `a.txt` is marked.
fn one_conflict(env: &Env) {
    run(env, &["line", "new", "side"], 0);
    env.overwrite("a.txt", "1\nside\n3\n4\n5\n6\n7\n8\n");
    run(env, &["save", "On side"], 0);
    run(env, &["goto", "main"], 0);
    env.overwrite("a.txt", "1\nmain\n3\n4\n5\n6\n7\n8\n");
    run(env, &["save", "On main"], 0);
    run(env, &["combine", "side"], 0);
}

const PIC_START: &[u8] = b"\x89PNG\r\n\x1a\n\0start";
const PIC_SIDE: &[u8] = b"\x89PNG\r\n\x1a\n\0side";
const PIC_MAIN: &[u8] = b"\x89PNG\r\n\x1a\n\0main";

/// `main` and `side` apart from one save holding `a.txt`, `pic.png` and `gone.txt`: both change
/// `a.txt`'s second line and the picture, `side` changes `gone.txt` and `main` removes it; then
/// combined, so `a.txt` is marked, `pic.png` is yours and `gone.txt` is nothing.
fn three_conflicts(env: &Env) {
    support::write(env, "pic.png", PIC_START);
    env.overwrite("gone.txt", "gone\n");
    run(env, &["save", "More"], 0);
    run(env, &["line", "new", "side"], 0);
    env.overwrite("a.txt", "1\nside\n3\n4\n5\n6\n7\n8\n");
    support::write(env, "pic.png", PIC_SIDE);
    env.overwrite("gone.txt", "changed\n");
    run(env, &["save", "On side"], 0);
    run(env, &["goto", "main"], 0);
    env.overwrite("a.txt", "1\nmain\n3\n4\n5\n6\n7\n8\n");
    support::write(env, "pic.png", PIC_MAIN);
    std::fs::remove_file(env.root().join("gone.txt")).unwrap();
    run(env, &["save", "On main"], 0);
    run(env, &["combine", "side"], 0);
}

/// The rows `opalro status` gives the places in conflict, each ending in a line break.
fn status_rows(env: &Env) -> Vec<String> {
    let (shown, _) = run(env, &["status"], 0);
    shown
        .lines()
        .skip_while(|l| !l.starts_with("In conflict ("))
        .skip(2)
        .take_while(|l| !l.is_empty())
        .map(|l| format!("{l}\n"))
        .collect()
}

const FIXED: &str = "1\nboth\n3\n4\n5\n6\n7\n8\n";

#[test]
fn the_usual_path_is_short() {
    let env = project();
    one_conflict(&env);
    let combined = tip(&env, "main");
    let (id, message) = newest(&env);
    assert_eq!(message, "Combine side into main");
    let rows_before = json(&env, &["history"], 0)["saves"]
        .as_array()
        .unwrap()
        .len();
    env.overwrite("a.txt", FIXED);
    let (shown, said) = run(&env, &["resolve"], 0);
    assert_eq!(
        shown,
        format!(
            "Recorded 1 fix in save {id} \"Combine side into main\" on main.\n\n    The combined \
             save now holds it; its ID stays the same.\n"
        )
    );
    assert_eq!(said, "");
    // One combined save, holding the fix, in the place of the one the combine made.
    let fixed = tip(&env, "main");
    assert_ne!(fixed, combined);
    let history = json(&env, &["history"], 0);
    let saves = history["saves"].as_array().unwrap();
    assert_eq!(saves.len(), rows_before);
    assert_eq!(saves[0]["message"], "Combine side into main");
    assert_eq!(saves[0]["change_id_short"], id.as_str());
    assert_eq!(saves[0]["parents"].as_array().unwrap().len(), 2);
    let (status, _) = run(&env, &["status"], 0);
    assert!(
        status.ends_with("\nNothing changed since then.\n"),
        "{status}"
    );
    assert!(!status.contains("In conflict"), "{status}");
    support::assert_folder_is_goto_of(&env, fixed);
    let (_, entry) = support::head(&env);
    assert_eq!(entry.command, "resolve");
}

#[test]
fn a_tag_on_the_combined_save_makes_the_fix_a_save_of_its_own() {
    let env = project();
    one_conflict(&env);
    let combined = tip(&env, "main");
    run(&env, &["tag", "v1.0"], 0);
    env.overwrite("a.txt", FIXED);
    let (shown, _) = run(&env, &["resolve"], 0);
    let (id, _) = newest(&env);
    assert_eq!(
        shown,
        format!(
            "Recorded 1 fix in a new save {id} \"Fixed conflicts in a.txt\" on main.\n\n    The \
             combined save couldn't take it: tag v1.0 shows it too.\n"
        )
    );
    let (repo, state) = env.state();
    let made = repo.store().save(&state.lines["main"], false).unwrap();
    assert_eq!(made.parents, [combined]);
}

#[test]
fn a_save_after_the_combine_makes_the_fix_a_save_of_its_own() {
    let env = project();
    one_conflict(&env);
    env.overwrite("b.txt", "b\n");
    run(&env, &["save", "B"], 0);
    env.overwrite("a.txt", FIXED);
    let (shown, _) = run(&env, &["resolve"], 0);
    let (id, _) = newest(&env);
    assert_eq!(
        shown,
        format!(
            "Recorded 1 fix in a new save {id} \"Fixed conflicts in a.txt\" on main.\n\n    Your \
             newest save isn't a combined save, so the fix is a save of its own.\n"
        )
    );
}

#[test]
fn keep_takes_a_side_whole_and_lists_what_is_left() {
    let env = project();
    three_conflicts(&env);
    let (shown, said) = run(
        &env,
        &["resolve", "pic.png", "gone.txt", "--keep", "theirs"],
        0,
    );
    let (id, _) = newest(&env);
    let rows: String = status_rows(&env)
        .iter()
        .map(|r| format!("    {r}"))
        .collect();
    assert_eq!(rows, "        a.txt  1 marked place left to fix\n");
    assert_eq!(
        shown,
        format!(
            "Recorded 2 fixes in save {id} \"Combine side into main\" on main.\n\n    The \
             combined save now holds them; its ID stays the same.\n    Took theirs for 2 \
             places.\n    1 place is still in conflict:\n{rows}"
        )
    );
    assert_eq!(said, "");
    assert_eq!(std::fs::read(env.root().join("pic.png")).unwrap(), PIC_SIDE);
    assert_eq!(
        std::fs::read_to_string(env.root().join("gone.txt")).unwrap(),
        "changed\n"
    );
    support::assert_folder_is_goto_of(&env, tip(&env, "main"));
}

#[test]
fn keep_over_a_hand_fix_keeps_a_safety_copy_and_says_so() {
    let env = project();
    three_conflicts(&env);
    support::write(&env, "pic.png", b"painted over");
    let (shown, _) = run(&env, &["resolve", "pic.png", "--keep", "mine"], 0);
    assert!(
        shown.contains(
            "\n    Took yours for 1 place.\n    A safety copy of what your folder held was \
             kept.\n    2 places are still in conflict:\n"
        ),
        "{shown}"
    );
    assert_eq!(std::fs::read(env.root().join("pic.png")).unwrap(), PIC_MAIN);
}

#[test]
fn nothing_to_record_lists_what_is_left() {
    let env = project();
    three_conflicts(&env);
    let main = tip(&env, "main");
    let rows: String = status_rows(&env).concat();
    assert_eq!(rows.lines().count(), 3, "{rows}");
    let (shown, said) = run(&env, &["resolve"], 0);
    assert_eq!(
        shown,
        format!("Nothing to record: no place in conflict is fixed in your folder.\n\n{rows}")
    );
    assert_eq!(said, "");
    assert_eq!(tip(&env, "main"), main);
}

#[test]
fn nothing_to_resolve_names_the_save() {
    let env = project();
    let (id, _) = newest(&env);
    let (shown, _) = run(&env, &["resolve"], 0);
    assert_eq!(
        shown,
        format!("Nothing to resolve: save {id} \"A\" holds no conflict.\n")
    );
}

/// `opalro args` must fail with exit code 1 saying `error` then `hint`, and change nothing.
fn refused(env: &Env, args: &[&str], error: &str, hint: &str) {
    let before = env.standing();
    let (shown, said) = run(env, args, 1);
    assert_eq!(shown, "", "{args:?}");
    assert_eq!(
        said,
        format!("error: {error}\n\nhint: {hint}\n"),
        "{args:?}"
    );
    assert_eq!(env.standing(), before, "{args:?}");
}

#[test]
fn each_refusal_says_its_words() {
    let env = project();
    three_conflicts(&env);
    refused(
        &env,
        &["resolve", "other.txt"],
        "other.txt isn't in conflict, so nothing was recorded.",
        "opalro status lists the places in conflict.",
    );
    refused(
        &env,
        &["resolve", "pic.png"],
        "pic.png still holds your version, and Opalro won't guess which one you want, so \
         nothing was recorded.",
        "Say which to keep: opalro resolve <path> --keep mine, or --keep theirs. Or put the \
         version you want in the folder and run opalro resolve <path>.",
    );
    // Without a terminal nothing is asked, so marker lines left stop it.
    refused(
        &env,
        &["resolve", "a.txt"],
        "a.txt still has 1 marked place, so nothing was recorded.",
        "Fix them first, or run it again with --yes to record the file as it is.",
    );
    let (shown, _) = run(&env, &["resolve", "a.txt", "--yes"], 0);
    assert!(shown.starts_with("Recorded 1 fix in save "), "{shown}");
    let first = json(&env, &["history"], 0)["saves"][2]["id"]
        .as_str()
        .unwrap()[..12]
        .to_owned();
    run(&env, &["goto", &first], 0);
    let before = env.standing();
    let (_, said) = run(&env, &["resolve"], 1);
    assert_eq!(
        said,
        "error: You're at a save, not on a line, so there is no line to record fixes on.\n\n\
         hint: Make a line here first: opalro line new <name>\n"
    );
    assert_eq!(env.standing(), before);
}

/// `opalro args` in a faked terminal, with `input` typed: its exit code, stdout and stderr.
#[cfg(feature = "fault-injection")]
fn asked(env: &Env, args: &[&str], input: &str) -> (Option<i32>, String, String) {
    let out = env
        .opalro()
        .args(args)
        .env("OPALRO_FAKE_TERMINAL", "80")
        .env("OPALRO_ASCII", "1")
        .env("NO_COLOR", "1")
        .write_stdin(input)
        .output()
        .unwrap();
    (out.status.code(), text(&out.stdout), text(&out.stderr))
}

#[cfg(feature = "fault-injection")]
#[test]
fn resolve_asks_on_a_terminal_and_stops_without_one() {
    let env = project();
    one_conflict(&env);
    let before = env.standing();
    let question = "a.txt still has 1 marked place.\nRecord it as fixed anyway? [y/N] ";
    // Without a terminal, nothing is asked.
    let (_, said) = run(&env, &["resolve", "a.txt"], 1);
    assert!(!said.contains("Record it"), "{said}");
    // With --json, even at a terminal, nothing is asked.
    let (code, shown, said) = asked(&env, &["resolve", "a.txt", "--json"], "y\n");
    assert_eq!(code, Some(1), "{said}");
    assert!(!said.contains("Record it"), "{said}");
    let doc: serde_json::Value = serde_json::from_str(&shown).unwrap();
    assert_eq!(
        doc["error"]["message"],
        "a.txt still has 1 marked place, so nothing was recorded."
    );
    assert_eq!(env.standing(), before);
    // Answered no, or with nothing, it records nothing.
    for input in ["n\n", "\n"] {
        let (code, shown, said) = asked(&env, &["resolve", "a.txt"], input);
        assert_eq!(code, Some(1), "{said}");
        assert_eq!(shown, "");
        assert!(said.starts_with(question), "{said}");
        assert!(
            said.contains("error: a.txt still has 1 marked place, so nothing was recorded."),
            "{said}"
        );
        assert_eq!(env.standing(), before);
    }
    let (code, shown, said) = asked(&env, &["resolve", "a.txt"], "y\n");
    assert_eq!(code, Some(0), "{said}");
    assert!(said.starts_with(question), "{said}");
    assert!(shown.starts_with("Recorded 1 fix in save "), "{shown}");
    let (status, _) = run(&env, &["status"], 0);
    assert!(!status.contains("In conflict"), "{status}");
}

#[cfg(feature = "fault-injection")]
#[test]
fn yes_on_a_terminal_records_without_asking() {
    let env = project();
    one_conflict(&env);
    let (code, shown, said) = asked(&env, &["resolve", "a.txt", "--yes"], "");
    assert_eq!(code, Some(0), "{said}");
    assert!(!said.contains("Record it"), "{said}");
    assert!(shown.starts_with("Recorded 1 fix in save "), "{shown}");
}

/// A project whose rules ask before a save adds over 1 KiB, with `a.txt` in conflict.
fn asks_over_a_kib() -> Env {
    let env = project();
    env.overwrite("opalro.toml", "ask-before-saving-over = \"1 KiB\"\n");
    run(&env, &["save", "Rules"], 0);
    three_conflicts(&env);
    // A new file of bytes that don't compress, where your line removed it: over 1 KiB.
    support::write(&env, "gone.txt", support::noise(7, 4096));
    env
}

const LARGE_HINT: &str = "Nothing was recorded. To record them, run opalro resolve again with \
                          --yes; to leave some out, list them in .opalroignore.";

#[test]
fn a_large_fix_without_a_terminal_stops_with_resolves_hint() {
    let env = asks_over_a_kib();
    let before = env.standing();
    let (shown, said) = run(&env, &["resolve", "gone.txt"], 1);
    assert_eq!(shown, "");
    assert!(
        said.ends_with(&format!("\n\nhint: {LARGE_HINT}\n")),
        "{said}"
    );
    assert_eq!(env.standing(), before);
    let doc = json(&env, &["resolve", "gone.txt"], 1);
    assert_eq!(doc["error"]["hint"], LARGE_HINT, "{doc}");
    let (shown, _) = run(&env, &["resolve", "gone.txt", "--yes"], 0);
    assert!(shown.starts_with("Recorded 1 fix in save "), "{shown}");
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_large_fix_asks_on_a_terminal() {
    let env = asks_over_a_kib();
    let before = env.standing();
    let (code, _, said) = asked(&env, &["resolve", "gone.txt"], "n\n");
    assert_eq!(code, Some(1), "{said}");
    assert!(said.contains(" Record them? [y/N] "), "{said}");
    assert!(!said.contains("Save them?"), "{said}");
    assert!(
        said.contains("error: Nothing was recorded: you answered no.\n"),
        "{said}"
    );
    assert_eq!(env.standing(), before);
    let (code, shown, said) = asked(&env, &["resolve", "gone.txt"], "y\n");
    assert_eq!(code, Some(0), "{said}");
    assert!(shown.starts_with("Recorded 1 fix in save "), "{shown}");
}

/// `main` and `side` changed `art/b.png` and `art/c.png` apart, combined: both stand as yours.
fn pictures_in_art(env: &Env) {
    pictures(env, &["art/b.png", "art/c.png"]);
}

/// `main` and `side` changed each of `names` apart, combined: each stands as yours.
fn pictures(env: &Env, names: &[&str]) {
    for bytes in [PIC_START, PIC_SIDE, PIC_MAIN] {
        for name in names {
            support::write(env, name, bytes);
        }
        if bytes == PIC_START {
            run(env, &["save", "Pictures"], 0);
            run(env, &["line", "new", "side"], 0);
        } else if bytes == PIC_SIDE {
            run(env, &["save", "On side"], 0);
            run(env, &["goto", "main"], 0);
        } else {
            run(env, &["save", "On main"], 0);
        }
    }
    run(env, &["combine", "side"], 0);
}

/// What `opalro status --json` holds in conflict.
fn in_conflict(env: &Env) -> Vec<String> {
    let doc = json(env, &["status"], 0);
    let places = doc["conflicts"].as_array().unwrap();
    places
        .iter()
        .map(|c| c["path"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_name_that_is_a_place_is_that_place_before_it_is_a_pattern() {
    // Both in conflict: the name means the file of that name.
    let env = project();
    pictures(&env, &["a[1].png", "a1.png"]);
    let doc = json(&env, &["resolve", "a[1].png", "--keep", "theirs"], 0);
    assert_eq!(doc["recorded"], serde_json::json!(["a[1].png"]), "{doc}");
    assert_eq!(
        std::fs::read(env.root().join("a[1].png")).unwrap(),
        PIC_SIDE
    );
    assert_eq!(std::fs::read(env.root().join("a1.png")).unwrap(), PIC_MAIN);
    assert_eq!(in_conflict(&env), ["a1.png"]);

    // Only `a1.png` in conflict: the same words are a pattern, and match it.
    let env = project();
    pictures(&env, &["a1.png"]);
    let doc = json(&env, &["resolve", "a[1].png", "--keep", "theirs"], 0);
    assert_eq!(doc["recorded"], serde_json::json!(["a1.png"]), "{doc}");

    // A folder named with a bracket.
    let env = project();
    pictures(&env, &["f[1]/b.png", "f1/b.png"]);
    let doc = json(&env, &["resolve", "f[1]", "--keep", "theirs"], 0);
    assert_eq!(doc["recorded"], serde_json::json!(["f[1]/b.png"]), "{doc}");
    assert_eq!(in_conflict(&env), ["f1/b.png"]);
}

#[test]
fn a_name_that_is_a_place_is_that_place_without_keep_too() {
    // Both in conflict and changed by hand: only the file of that name is recorded.
    let env = project();
    pictures(&env, &["a[1].png", "a1.png"]);
    support::write(&env, "a[1].png", b"painted over");
    support::write(&env, "a1.png", b"painted over");
    let doc = json(&env, &["resolve", "a[1].png"], 0);
    assert_eq!(doc["recorded"], serde_json::json!(["a[1].png"]), "{doc}");
    assert_eq!(in_conflict(&env), ["a1.png"]);

    // Only `a1.png` in conflict: a pattern, which records only what is fixed, and nothing is.
    let env = project();
    pictures(&env, &["a1.png"]);
    let rows: String = status_rows(&env).concat();
    let (shown, _) = run(&env, &["resolve", "a[1].png"], 0);
    assert_eq!(
        shown,
        format!("Nothing to record: no place in conflict is fixed in your folder.\n\n{rows}")
    );

    // A folder named with a bracket: as a folder's path, it takes no picture changed in place
    // below it; named by its own path, the picture in it is that place.
    let env = project();
    pictures(&env, &["f[1]/b.png", "f1/b.png"]);
    support::write(&env, "f[1]/b.png", b"painted over");
    support::write(&env, "f1/b.png", b"painted over");
    let doc = json(&env, &["resolve", "f[1]"], 0);
    assert_eq!(doc["result"], "nothing", "{doc}");
    let doc = json(&env, &["resolve", "f[1]/b.png"], 0);
    assert_eq!(doc["recorded"], serde_json::json!(["f[1]/b.png"]), "{doc}");
    assert_eq!(in_conflict(&env), ["f1/b.png"]);
}

#[test]
fn several_fixes_that_cant_go_into_the_combined_save_say_them() {
    let env = project();
    three_conflicts(&env);
    run(&env, &["tag", "v1.0"], 0);
    let (shown, _) = run(
        &env,
        &["resolve", "pic.png", "gone.txt", "--keep", "theirs"],
        0,
    );
    assert!(
        shown.contains(
            "\n\n    The combined save couldn't take them: tag v1.0 shows it too.\n    Took theirs \
             for 2 places.\n"
        ),
        "{shown}"
    );
    let env = project();
    three_conflicts(&env);
    env.overwrite("b.txt", "b\n");
    run(&env, &["save", "B"], 0);
    let (shown, _) = run(
        &env,
        &["resolve", "pic.png", "gone.txt", "--keep", "theirs"],
        0,
    );
    assert!(
        shown.contains(
            "\n\n    Your newest save isn't a combined save, so the fixes are a save of their \
             own.\n"
        ),
        "{shown}"
    );
}

#[test]
fn a_pattern_with_keep_takes_every_place_it_matches() {
    let env = project();
    three_conflicts(&env);
    let (shown, _) = run(&env, &["resolve", "*.txt", "--keep", "theirs"], 0);
    assert!(shown.starts_with("Recorded 2 fixes in save "), "{shown}");
    assert!(
        shown.contains("\n    Took theirs for 2 places.\n    1 place is still in conflict:\n"),
        "{shown}"
    );
    assert_eq!(
        std::fs::read_to_string(env.root().join("a.txt")).unwrap(),
        "1\nside\n3\n4\n5\n6\n7\n8\n"
    );
    assert_eq!(
        std::fs::read_to_string(env.root().join("gone.txt")).unwrap(),
        "changed\n"
    );
    support::assert_folder_is_goto_of(&env, tip(&env, "main"));
}

#[test]
fn a_pattern_without_keep_records_the_fixed_places_it_matches() {
    let env = project();
    three_conflicts(&env);
    env.overwrite("a.txt", FIXED);
    let (shown, _) = run(&env, &["resolve", "*.txt"], 0);
    let rows: String = status_rows(&env)
        .iter()
        .map(|r| format!("    {r}"))
        .collect();
    let (id, _) = newest(&env);
    assert_eq!(
        shown,
        format!(
            "Recorded 1 fix in save {id} \"Combine side into main\" on main.\n\n    The \
             combined save now holds it; its ID stays the same.\n    2 places are still in \
             conflict:\n{rows}"
        )
    );
    assert!(
        rows.contains("gone.txt") && rows.contains("pic.png"),
        "{rows}"
    );
}

#[test]
fn a_pattern_that_matches_nothing_is_refused() {
    let env = project();
    three_conflicts(&env);
    refused(
        &env,
        &["resolve", "*.md", "--keep", "theirs"],
        "Nothing in conflict matches *.md, so nothing was recorded.",
        "opalro status lists the places in conflict.",
    );
    refused(
        &env,
        &["resolve", "*.md"],
        "Nothing in conflict matches *.md, so nothing was recorded.",
        "opalro status lists the places in conflict.",
    );
}

#[test]
fn paths_and_patterns_are_read_from_the_folder_you_are_in() {
    let env = project();
    pictures_in_art(&env);
    let art = env.root().join("art");
    // From the top, `*.png` names nothing: the pictures are in art.
    refused(
        &env,
        &["resolve", "*.png", "--keep", "theirs"],
        "Nothing in conflict matches *.png, so nothing was recorded.",
        "opalro status lists the places in conflict.",
    );
    let (shown, _) = run_in(&env, &art, &["resolve", "b.png", "--keep", "theirs"], 0);
    assert!(shown.starts_with("Recorded 1 fix in save "), "{shown}");
    assert!(shown.contains("        art/c.png  "), "{shown}");
    assert_eq!(std::fs::read(art.join("b.png")).unwrap(), PIC_SIDE);
    let (shown, _) = run_in(&env, &art, &["resolve", "*.png", "--keep", "theirs"], 0);
    assert!(shown.starts_with("Recorded 1 fix in save "), "{shown}");
    assert!(!shown.contains("still in conflict"), "{shown}");
    assert_eq!(std::fs::read(art.join("c.png")).unwrap(), PIC_SIDE);
    support::assert_folder_is_goto_of(&env, tip(&env, "main"));
}

#[test]
fn a_path_outside_the_project_is_refused() {
    let env = project();
    three_conflicts(&env);
    refused(
        &env,
        &["resolve", "../elsewhere.txt"],
        "../elsewhere.txt is outside the project.",
        "Name files and folders inside the project, from the folder you are in.",
    );
}

#[test]
fn a_path_in_another_letter_case_is_matched_as_the_drive_matches_names() {
    let env = project();
    three_conflicts(&env);
    if support::corpus::ignores_case(env.root()) {
        let doc = json(&env, &["resolve", "PIC.PNG", "--keep", "theirs"], 0);
        assert_eq!(doc["recorded"], serde_json::json!(["pic.png"]), "{doc}");
    } else {
        // This drive tells the names apart, so `PIC.PNG` is another file.
        refused(
            &env,
            &["resolve", "PIC.PNG", "--keep", "theirs"],
            "PIC.PNG isn't in conflict, so nothing was recorded.",
            "opalro status lists the places in conflict.",
        );
    }
}

#[test]
fn resolve_keep_requires_paths() {
    let env = project();
    one_conflict(&env);
    let before = env.standing();
    let (_, said) = run(&env, &["resolve", "--keep", "theirs"], 2);
    assert!(said.contains("<PATHS>"), "{said}");
    let (_, said) = run(
        &env,
        &[
            "resolve", "a.txt", "--keep", "theirs", "--copy", "theirs", "--to", "x",
        ],
        2,
    );
    assert!(said.contains("cannot be used with"), "{said}");
    assert_eq!(env.standing(), before);
}

#[test]
fn copy_takes_one_path_and_a_to() {
    let env = project();
    three_conflicts(&env);
    let before = env.standing();
    for args in [
        &["resolve", "--copy", "theirs", "--to", "x.png"][..],
        &[
            "resolve", "pic.png", "a.txt", "--copy", "theirs", "--to", "x.png",
        ],
    ] {
        let (shown, said) = run(&env, args, 2);
        assert_eq!(shown, "");
        assert_eq!(said, "error: --copy takes one path.\n", "{args:?}");
    }
    let (_, said) = run(&env, &["resolve", "pic.png", "--copy", "theirs"], 2);
    assert!(said.contains("--to <FILE>"), "{said}");
    let (_, said) = run(&env, &["resolve", "pic.png", "--to", "x.png"], 2);
    assert!(said.contains("--copy <COPY>"), "{said}");
    assert_eq!(env.standing(), before);
    assert!(!env.root().join("x.png").exists());

    // --to is read from the folder you are in, and shown as typed.
    let art = env.root().join("art");
    std::fs::create_dir(&art).unwrap();
    for (pick, words, bytes) in [
        (
            "theirs",
            "Wrote the other line's version of pic.png to theirs.png.\n",
            PIC_SIDE,
        ),
        (
            "mine",
            "Wrote your version of pic.png to mine.png.\n",
            PIC_MAIN,
        ),
        (
            "before",
            "Wrote pic.png as it was where the lines parted to before.png.\n",
            PIC_START,
        ),
    ] {
        let to = format!("{pick}.png");
        let args = ["resolve", "../pic.png", "--copy", pick, "--to", &to];
        let (shown, said) = run_in(&env, &art, &args, 0);
        assert_eq!(shown, words);
        assert_eq!(said, "");
        assert_eq!(std::fs::read(art.join(&to)).unwrap(), bytes);
    }
    // Each version as --json names it.
    for pick in ["mine", "theirs", "before"] {
        let to = format!("{pick}.json.png");
        let args = [
            "resolve",
            "../pic.png",
            "--copy",
            pick,
            "--to",
            &to,
            "--json",
        ];
        let (shown, _) = run_in(&env, &art, &args, 0);
        let doc: serde_json::Value = serde_json::from_str(&shown).unwrap();
        assert_eq!(doc["version"], pick, "{doc}");
        assert_eq!(doc["path"], "pic.png", "{doc}");
        assert_eq!(doc["to"], to.as_str(), "{doc}");
    }
    let (_, said) = run_in(
        &env,
        &art,
        &[
            "resolve",
            "../pic.png",
            "--copy",
            "mine",
            "--to",
            "theirs.png",
        ],
        1,
    );
    assert_eq!(
        said,
        "error: theirs.png is already there, so nothing was written.\n"
    );
    assert_eq!(std::fs::read(art.join("theirs.png")).unwrap(), PIC_SIDE);
}

#[test]
fn resolve_json_matches_its_snapshot() {
    let env = project();
    let none = json(&env, &["resolve"], 0);
    assert_eq!(none["result"], "no_conflicts", "{none}");
    insta::assert_json_snapshot!("no_conflicts", support::masked(none));
    three_conflicts(&env);
    let nothing = json(&env, &["resolve"], 0);
    insta::assert_json_snapshot!("nothing", support::masked(nothing));
    let copied = json(
        &env,
        &[
            "resolve",
            "pic.png",
            "--copy",
            "theirs",
            "--to",
            "pic-theirs.png",
        ],
        0,
    );
    insta::assert_json_snapshot!("copied", support::masked(copied));
    std::fs::remove_file(env.root().join("pic-theirs.png")).unwrap();
    let recorded = json(&env, &["resolve", "pic.png", "--keep", "theirs"], 0);
    insta::assert_json_snapshot!("recorded", support::masked(recorded));
}

#[test]
fn undo_says_what_a_resolve_took_back() {
    let env = project();
    one_conflict(&env);
    let combined = tip(&env, "main");
    let (id, _) = newest(&env);
    env.overwrite("a.txt", FIXED);
    run(&env, &["resolve"], 0);
    let (shown, _) = run(&env, &["undo"], 0);
    assert_eq!(shown, format!("Undid resolve: main is back at {id}.\n"));
    assert_eq!(tip(&env, "main"), combined);
    // A fix made by hand stays in the folder, and the conflict is open again.
    assert_eq!(
        std::fs::read_to_string(env.root().join("a.txt")).unwrap(),
        FIXED
    );
    let rows = status_rows(&env);
    assert_eq!(rows, ["    a.txt  fixed, not yet recorded\n"]);
}

#[test]
fn undo_after_keep_puts_the_folder_back() {
    let env = project();
    three_conflicts(&env);
    let combined = tip(&env, "main");
    let folder = support::files(env.root());
    let rows = status_rows(&env);
    run(
        &env,
        &["resolve", "pic.png", "gone.txt", "--keep", "theirs"],
        0,
    );
    run(&env, &["undo"], 0);
    assert_eq!(tip(&env, "main"), combined);
    assert_eq!(support::files(env.root()), folder);
    assert_eq!(status_rows(&env), rows);
    support::assert_folder_is_goto_of(&env, combined);
}

/// Imports a Git history where `main` and `side` both changed `name`, from one commit, and brings
/// the folder to `main`'s save.
fn imported_apart(name: &str) -> Env {
    let env = Env::new();
    let who = "Ada Lovelace <ada@example.com> 1700000000 +0000";
    let commit = |branch: &str, mark: u32, from: &str, data: &str| {
        format!(
            "commit refs/heads/{branch}\nmark :{mark}\nauthor {who}\ncommitter {who}\ndata 2\nm\n\
             {from}M 100644 inline {name}\ndata {}\n{data}\n",
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
    // `git fast-import` checks nothing out.
    run(&env, &["discard"], 0);
    env
}

#[test]
fn a_hostile_path_is_escaped_in_resolves_rows() {
    let name = "x\u{1b}[31m\u{202e}.txt";
    let shown_name = "x\\x1b[31m\\u{202e}.txt";
    let env = imported_apart(name);
    run(&env, &["combine", "side"], 0);
    let (shown, said) = run(&env, &["resolve"], 0);
    // Windows can't hold the name, so nothing is written there; elsewhere the marked text is.
    let needs = if cfg!(windows) {
        "can't be written on this computer, kept as it is"
    } else {
        "1 marked place left to fix"
    };
    assert_eq!(
        shown,
        format!(
            "Nothing to record: no place in conflict is fixed in your folder.\n\n    \
             {shown_name}  {needs}\n"
        )
    );
    support::assert_escaped(&shown, "resolve");
    support::assert_escaped(&said, "resolve");
    let doc = json(&env, &["resolve"], 0);
    assert_eq!(doc["conflicts"][0]["path"], name);
}

#[test]
fn a_hostile_path_is_escaped_in_the_default_messages_outcome() {
    let name = "x\u{1b}[31m\u{202e}.txt";
    let shown_name = "x\\x1b[31m\\u{202e}.txt";
    let env = imported_apart(name);
    run(&env, &["combine", "side"], 0);
    run(&env, &["tag", "v1.0"], 0);
    // Typed, the name's `[` makes it a pattern, as for save; a class holding `[` names it.
    let (shown, said) = run(
        &env,
        &["resolve", "x\u{1b}[[]31m\u{202e}.txt", "--keep", "theirs"],
        0,
    );
    let (id, message) = newest(&env);
    assert_eq!(message, format!("Fixed conflicts in {name}"));
    assert!(
        shown.starts_with(&format!(
            "Recorded 1 fix in a new save {id} \"Fixed conflicts in {shown_name}\" on main.\n"
        )),
        "{shown}"
    );
    support::assert_escaped(&shown, "resolve --keep");
    support::assert_escaped(&said, "resolve --keep");
}

/// Two lines that parted at a save holding `a.txt` in conflict fix it differently, then are
/// combined: Opalro can't tell what was there where they parted, and `--copy before` says so in
/// the sentence that is true whether nothing was there or Opalro can't tell what was.
#[test]
fn copy_before_where_opalro_cant_tell_what_was_there_says_it_has_no_version() {
    let env = project();
    one_conflict(&env);
    run(&env, &["line", "new", "fix-one"], 0);
    env.overwrite("a.txt", "1\nX\n3\n4\n5\n6\n7\n8\n");
    run(&env, &["resolve"], 0);
    run(&env, &["goto", "main"], 0);
    run(&env, &["line", "new", "fix-two"], 0);
    env.overwrite("a.txt", "1\nY\n3\n4\n5\n6\n7\n8\n");
    run(&env, &["resolve"], 0);
    run(&env, &["combine", "fix-one"], 0);
    let marked = std::fs::read_to_string(env.root().join("a.txt")).unwrap();
    assert!(marked.contains("which Opalro can't tell here"), "{marked}");
    let to = env.config().join("before.txt");
    let to = to.to_str().unwrap();
    let (shown, said) = run(
        &env,
        &["resolve", "a.txt", "--copy", "before", "--to", to],
        1,
    );
    assert_eq!(shown, "");
    assert_eq!(
        said,
        "error: Opalro has no version of a.txt from where the lines parted (nothing was there, or \
         Opalro can't tell what was), so there is nothing to copy.\n"
    );
    assert!(!Path::new(to).exists());
}

/// `d/a.txt` changed on `side` and removed on `main`, combined on `main`: it stands as nothing,
/// in a folder `d` that is empty; then the person deletes `d`.
fn in_a_deleted_folder(env: &Env) {
    std::fs::create_dir_all(env.root().join("d")).unwrap();
    env.overwrite("d/a.txt", "1\n2\n3\n");
    run(env, &["save", "D"], 0);
    run(env, &["line", "new", "side"], 0);
    env.overwrite("d/a.txt", "1\nside\n3\n");
    run(env, &["save", "On side"], 0);
    run(env, &["goto", "main"], 0);
    std::fs::remove_dir_all(env.root().join("d")).unwrap();
    run(env, &["save", "Gone on main"], 0);
    run(env, &["combine", "side"], 0);
    assert!(env.root().join("d").is_dir());
    std::fs::remove_dir(env.root().join("d")).unwrap();
}

/// A place in conflict below a folder the person deleted, kept on the side that holds nothing
/// there: `resolve` takes only that place, so the folder stays deleted, as their other unsaved
/// work stays unsaved; the step records only what changed, and `undo` gives back exactly what
/// was there.
#[test]
fn keeping_nothing_below_a_deleted_folder_leaves_the_folder_deleted() {
    let env = project();
    in_a_deleted_folder(&env);
    let before = support::entries(env.root());
    run(&env, &["resolve", "d/a.txt", "--keep", "mine"], 0);
    assert!(
        !env.root().join("d").exists(),
        "resolve made again the folder d, which only holds the place it was asked about"
    );
    support::assert_step_records(&env, &before);
    // The new save still holds the folder, as the combined save did: only the place changed.
    let saved = support::saved_paths(&env, support::main_tip(&env));
    assert_eq!(saved.get("d").map(|e| e.0), Some('d'));
    assert!(!saved.contains_key("d/a.txt"));
    run(&env, &["undo"], 0);
    assert_eq!(support::entries(env.root()), before);
}

/// The same, kept on the side that holds a file there: the folder is made, to hold it.
#[test]
fn keeping_a_file_below_a_deleted_folder_makes_the_folder_for_it() {
    let env = project();
    in_a_deleted_folder(&env);
    let before = support::entries(env.root());
    run(&env, &["resolve", "d/a.txt", "--keep", "theirs"], 0);
    assert_eq!(
        std::fs::read_to_string(env.root().join("d/a.txt")).unwrap(),
        "1\nside\n3\n"
    );
    support::assert_step_records(&env, &before);
    support::assert_folder_is_goto_of(&env, support::main_tip(&env));
    run(&env, &["undo"], 0);
    assert_eq!(support::entries(env.root()), before);
}
