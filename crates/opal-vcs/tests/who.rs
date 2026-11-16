//! `opalro who`: the save behind each line of a file, on the command line.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

mod support;
use support::{Env, git_with_input, text};

/// `moves.txt` in three Git commits by two people in two time zones, then in the folder with a
/// line not saved yet.
const VERSIONS: [(&str, &str, &str, &str); 3] = [
    (
        "Ana Lima <ana@example.com>",
        "1700000000 +0100",
        "First",
        "jump:\n\theight 2\n\tsound hop\nend\n",
    ),
    (
        "Ben Okafor <ben@example.com>",
        "1700100000 -0500",
        "Higher",
        "jump:\n\theight 3\n\tsound hop\nend\nfall:\n",
    ),
    (
        "Ana Lima <ana@example.com>",
        "1700200000 +0100",
        "Heading",
        "# moves\njump:\n\theight 3\n\tsound hop\nend\nfall:\n",
    ),
];
const FOLDER: &str = "# moves\njump:\n\theight 3\n\tsound hop\nend\n\tcoyote 0.1\nfall:\n";

/// The three commits as `git fast-import` takes them, so their IDs, and the change IDs Opalro
/// gives them, are the same on every computer.
fn stream() -> String {
    let mut stream = String::new();
    for (n, (who, when, message, contents)) in VERSIONS.iter().enumerate() {
        let (blob, commit) = (2 * n + 1, 2 * n + 2);
        stream += &format!("blob\nmark :{blob}\ndata {}\n{contents}\n", contents.len());
        stream += &format!(
            "commit refs/heads/main\nmark :{commit}\nauthor {who} {when}\ncommitter {who} \
             {when}\ndata {}\n{message}\n",
            message.len()
        );
        if n > 0 {
            stream += &format!("from :{}\n", commit - 2);
        }
        stream += &format!("M 100644 :{blob} moves.txt\n\n");
    }
    stream
}

/// The three commits imported, and `moves.txt` in the folder with a line added.
fn imported() -> Env {
    let env = Env::new();
    let made = git_with_input(env.root(), &["fast-import", "--quiet"], stream().as_bytes());
    assert!(made.status.success(), "{}", text(&made.stderr));
    env.import();
    env.overwrite("moves.txt", FOLDER);
    env
}

/// A started project with `a.txt` saved as three lines.
fn started() -> Env {
    let env = Env::empty();
    run(&env, &["start", "--preset", "custom"], 0);
    env.overwrite("a.txt", "one\ntwo\nthree\n");
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

/// `opalro args`, which must be refused with exit code 1 and print nothing on stdout; its stderr.
fn refused(env: &Env, args: &[&str]) -> String {
    let (shown, said) = run(env, args, 1);
    assert_eq!(shown, "", "{args:?}");
    said
}

#[test]
fn who_names_the_save_behind_each_line() {
    let env = imported();
    let (shown, said) = run(&env, &["who", "moves.txt"], 0);
    assert_eq!(said, "");
    insta::assert_snapshot!("folder", shown);
    // Seven rows, between the outcome line and the count of lines not saved yet.
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(lines[0], "Each line of moves.txt and the save it is from:");
    assert_eq!(lines.len(), 1 + 1 + 7 + 1 + 1, "{shown}");
    assert_eq!(lines[10], "1 line isn't saved yet.");
    // The day is the author's own: Ben's save is the 16th in UTC, the 15th where he was.
    assert!(
        lines[4].contains("Ben Okafor  2023-11-15  3 | \theight 3"),
        "{shown}"
    );
    assert!(lines[7].starts_with("not saved yet "), "{shown}");
    assert!(lines[7].ends_with(" 6 | \tcoyote 0.1"), "{shown}");

    let doc = json(&env, &["who", "moves.txt"], 0);
    insta::assert_json_snapshot!("folder_json", support::masked(doc.clone()));
    let history = json(&env, &["history"], 0);
    let ids: Vec<&str> = (0..3)
        .map(|n| history["saves"][n]["id"].as_str().unwrap())
        .collect();
    // Runs name fingerprints; each save is listed once, in the order of its first run.
    let runs: Vec<(u64, u64, Option<&str>)> = doc["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            assert_eq!(r["was"], serde_json::Value::Null);
            (
                r["first"].as_u64().unwrap(),
                r["lines"].as_u64().unwrap(),
                r["save"].as_str(),
            )
        })
        .collect();
    let (heading, higher, first) = (Some(ids[0]), Some(ids[1]), Some(ids[2]));
    assert_eq!(
        runs,
        [
            (1, 1, heading),
            (2, 1, first),
            (3, 1, higher),
            (4, 2, first),
            (6, 1, None),
            (7, 1, higher),
        ]
    );
    let saves: Vec<&str> = doc["saves"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(saves, [ids[0], ids[2], ids[1]]);
    assert_eq!(doc["save"]["id"], ids[0]);
    assert_eq!(doc["from"], "folder");
    assert_eq!(doc["lines"], 7);
    assert_eq!(doc["coarse"], false);
}

#[test]
fn save_reads_the_file_as_that_save_has_it() {
    let env = imported();
    let history = json(&env, &["history"], 0);
    let higher = &history["saves"][1];
    let short = higher["change_id_short"].as_str().unwrap();
    let fingerprint = &higher["id"].as_str().unwrap()[..12];
    let (shown, said) = run(&env, &["who", "moves.txt", "--save", fingerprint], 0);
    assert_eq!(said, "");
    insta::assert_snapshot!("at_a_save", shown);
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(
        lines[0],
        format!("Each line of moves.txt as save {short} has it, and the save it is from:")
    );
    assert_eq!(lines.len(), 2 + 5, "{shown}");
    assert!(!shown.contains("not saved yet"), "{shown}");
    let doc = json(&env, &["who", "moves.txt", "--save", fingerprint], 0);
    assert_eq!(doc["from"], "save");
    assert_eq!(doc["save"]["id"], higher["id"]);
    assert_eq!(doc["lines"], 5);
    // A word `show` refuses, `who` refuses alike.
    let shows = refused(&env, &["show", "nowhere"]);
    assert_eq!(
        refused(&env, &["who", "moves.txt", "--save", "nowhere"]),
        shows
    );
}

#[test]
fn each_refusal_says_why_and_ends_with_1() {
    let env = started();
    assert_eq!(
        refused(&env, &["who", "none.txt"]),
        "error: none.txt isn't a file in your folder.\n\nhint: To read it as a save has it: \
         opalro who <file> --save <save>\n"
    );
    let tip = support::main_tip(&env).short_hex();
    assert_eq!(
        refused(&env, &["who", "none.txt", "--save", "main"]),
        format!("error: Save {tip} has no file at none.txt.\n")
    );
    support::write(&env, "art/a.txt", "a\n");
    run(&env, &["save", "Art"], 0);
    assert_eq!(
        refused(&env, &["who", "art", "--save", "main"]),
        "error: art is a folder, and opalro who reads one file.\n"
    );
    // In the folder, a folder is no file.
    assert_eq!(
        refused(&env, &["who", "art"]),
        "error: art isn't a file in your folder.\n\nhint: To read it as a save has it: opalro \
         who <file> --save <save>\n"
    );
    support::write(&env, "pic.dat", b"PNG\0data\n");
    assert_eq!(
        refused(&env, &["who", "pic.dat"]),
        "error: pic.dat isn't text, so it has no lines to name a save for.\n"
    );
    support::write(&env, "many.txt", "\n".repeat(1_000_001));
    assert_eq!(
        refused(&env, &["who", "many.txt"]),
        "error: many.txt has more than 1,000,000 lines, the most Opalro compares line by line.\n"
    );
    support::write(&env, "big.txt", vec![b'a'; (64 << 20) + 1]);
    assert_eq!(
        refused(&env, &["who", "big.txt"]),
        "error: big.txt is larger than 64 MiB, the most Opalro compares line by line.\n"
    );
    std::fs::remove_file(env.root().join("big.txt")).unwrap();
    let outside = refused(&env, &["who", "../a.txt"]);
    let changes = refused(&env, &["changes", "../a.txt"]);
    assert_eq!(outside, changes);
    assert_eq!(
        outside,
        "error: ../a.txt is outside the project.\n\nhint: Name files and folders inside the \
         project, from the folder you are in.\n"
    );
    let doc = json(&env, &["who", "none.txt"], 1);
    assert_eq!(
        doc["error"]["message"],
        "none.txt isn't a file in your folder."
    );
}

#[cfg(unix)]
#[test]
fn a_link_in_a_save_is_refused() {
    let env = started();
    std::os::unix::fs::symlink("a.txt", env.root().join("to-a")).unwrap();
    run(&env, &["save", "Link"], 0);
    assert_eq!(
        refused(&env, &["who", "to-a", "--save", "main"]),
        "error: to-a is a link, and opalro who reads one file.\n"
    );
}

#[test]
fn an_empty_file_and_one_no_save_has() {
    let env = started();
    env.overwrite("empty.txt", "");
    assert_eq!(
        run(&env, &["who", "empty.txt"], 0).0,
        "empty.txt is empty.\n"
    );
    run(&env, &["save", "Empty"], 0);
    assert_eq!(
        run(&env, &["who", "empty.txt", "--save", "main"], 0).0,
        "empty.txt is empty.\n"
    );
    env.overwrite("new.txt", "x\ny\n");
    assert_eq!(
        run(&env, &["who", "new.txt"], 0).0,
        "new.txt isn't saved yet: none of its 2 lines is from a save.\n"
    );
    env.overwrite("one.txt", "x");
    assert_eq!(
        run(&env, &["who", "one.txt"], 0).0,
        "one.txt isn't saved yet: its 1 line isn't from a save.\n"
    );
    let doc = json(&env, &["who", "one.txt"], 0);
    assert_eq!(doc["lines"], 1);
    assert_eq!(doc["saves"], serde_json::json!([]));
    assert_eq!(doc["runs"][0]["save"], serde_json::Value::Null);
}

#[test]
fn a_path_is_named_from_the_folder_you_are_in_and_never_a_pattern() {
    let env = started();
    support::write(&env, "sub/notes.txt", "n\n");
    run(&env, &["save", "Notes"], 0);
    let (shown, _) = run_in(&env, &env.root().join("sub"), &["who", "notes.txt"], 0);
    assert!(
        shown.starts_with("Each line of sub/notes.txt and the save it is from:\n"),
        "{shown}"
    );
    let (shown, _) = run_in(&env, &env.root().join("sub"), &["who", "../a.txt"], 0);
    assert!(shown.starts_with("Each line of a.txt and"), "{shown}");
    // `*` is a character of the name: `a.txt` isn't matched, and the name is refused as typed.
    let said = refused(&env, &["who", "*.txt"]);
    assert!(said.starts_with("error: "), "{said}");
    assert!(said.contains("*.txt"), "{said}");
    assert!(!said.contains("a.txt"), "{said}");
}

/// Where a name may hold `*`, a file so named is read as itself.
#[cfg(unix)]
#[test]
fn a_name_holding_a_star_is_read_as_itself() {
    let env = started();
    env.overwrite("a*.txt", "star\n");
    let (shown, _) = run(&env, &["who", "a*.txt"], 0);
    assert_eq!(
        shown,
        "a*.txt isn't saved yet: its 1 line isn't from a save.\n"
    );
    assert_eq!(
        refused(&env, &["who", "*.txt"]),
        "error: *.txt isn't a file in your folder.\n\nhint: To read it as a save has it: opalro \
         who <file> --save <save>\n"
    );
}

#[test]
fn a_long_name_and_repeated_saves_are_left_whole_and_plain_off_a_terminal() {
    let env = Env::empty();
    run(&env, &["start", "--preset", "custom"], 0);
    env.overwrite("a.txt", "one\ntwo\n");
    let name = "Maximiliana Wolkenstein-Ravensworth";
    let out = env
        .opalro()
        .env("OPALRO_USER_NAME", name)
        .args(["save", "A"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let (shown, _) = run(&env, &["who", "a.txt"], 0);
    let rows: Vec<&str> = shown.lines().skip(2).collect();
    assert_eq!(rows.len(), 2, "{shown}");
    for row in rows {
        assert!(row.contains(&format!("  {name}  ")), "{row}");
    }
    assert!(!shown.contains('\x1b'), "{shown:?}");
}

#[test]
fn usage_errors_end_with_2() {
    let env = started();
    for args in [
        &["who"][..],
        &["who", "a.txt", "b.txt"],
        &["who", "--since", "x", "a.txt"],
        &["blame"],
    ] {
        let (shown, said) = run(&env, args, 2);
        assert_eq!(shown, "", "{args:?}");
        assert!(said.starts_with("error:"), "{args:?}: {said}");
    }
}

#[test]
fn blame_and_annotate_run_who() {
    let env = imported();
    let who = run(&env, &["who", "moves.txt"], 0);
    for word in ["blame", "annotate"] {
        assert_eq!(run(&env, &[word, "moves.txt"], 0), who, "{word}");
        assert_eq!(
            json(&env, &[word, "moves.txt"], 0),
            json(&env, &["who", "moves.txt"], 0)
        );
    }
}

/// `opalro` on an 80-column terminal, with colour unless `colour` is false.
#[cfg(feature = "fault-injection")]
fn on_a_terminal(env: &Env, args: &[&str], colour: bool) -> (String, String) {
    let mut cmd = env.opalro();
    cmd.env("OPALRO_FAKE_TERMINAL", "80")
        .env("OPALRO_PROGRESS_QUIET_MS", "600000")
        .env_remove("NO_COLOR");
    if !colour {
        cmd.env("NO_COLOR", "1");
    }
    let out = cmd.args(args).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    (text(&out.stdout), text(&out.stderr))
}

#[cfg(feature = "fault-injection")]
#[test]
fn the_hint_for_blame_and_annotate_shows_once() {
    let env = imported();
    for word in ["blame", "annotate"] {
        let (shown, said) = on_a_terminal(&env, &[word, "moves.txt"], false);
        assert!(shown.starts_with("Each line of moves.txt"), "{shown}");
        assert_eq!(
            said,
            format!("\nhint: in Opalro, {word} is who: opalro who <file>\n")
        );
        let (_, again) = on_a_terminal(&env, &[word, "moves.txt"], false);
        assert_eq!(again, "", "{word}");
    }
    assert_eq!(
        std::fs::read_to_string(env.config().join("git-words")).unwrap(),
        "blame\nannotate\n"
    );
}

#[cfg(feature = "fault-injection")]
#[test]
fn on_a_terminal_a_long_name_is_cut_and_a_repeated_save_is_dim() {
    let env = Env::empty();
    run(&env, &["start", "--preset", "custom"], 0);
    env.overwrite("a.txt", "one\ntwo\n");
    let out = env
        .opalro()
        .env("OPALRO_USER_NAME", "Maximiliana Wolkenstein-Ravensworth")
        .args(["save", "A"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let (plain, _) = on_a_terminal(&env, &["who", "a.txt"], false);
    let rows: Vec<&str> = plain.lines().skip(2).collect();
    for row in &rows {
        // 20 columns: the name's start and the cut mark, `…` or `...` as the terminal shows.
        assert!(
            row.contains("  Maximiliana Wolkens…  ") || row.contains("  Maximiliana Wolke...  "),
            "{row}"
        );
    }
    let (painted, _) = on_a_terminal(&env, &["who", "a.txt"], true);
    let rows: Vec<&str> = painted.lines().skip(2).collect();
    // The first row's ID is the change ID's colour; the second, repeating it, is dim.
    assert!(rows[0].starts_with("\x1b[1;35m"), "{rows:?}");
    assert!(rows[1].starts_with("\x1b[2m"), "{rows:?}");
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_comparison_past_its_deadline_is_a_warning_and_coarse() {
    let env = started();
    env.overwrite("a.txt", "one\nnew\ntwo\nthree\n");
    run(&env, &["save", "B"], 0);
    let out = env
        .opalro()
        .env("OPALRO_DIFF_DEADLINE_MS", "0")
        .args(["who", "a.txt"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        text(&out.stderr),
        "warning: Comparing some versions took too long, so a line may be named for a newer \
         save than the one that first had it.\n"
    );
    let out = env
        .opalro()
        .env("OPALRO_DIFF_DEADLINE_MS", "0")
        .args(["who", "a.txt", "--json"])
        .output()
        .unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["coarse"], true);
    assert_eq!(doc["warnings"], serde_json::json!([]));
    assert_eq!(text(&out.stderr), "");
}

#[test]
fn who_changes_nothing() {
    let env = started();
    run(&env, &["line", "new", "side"], 0);
    env.overwrite("a.txt", "one\nside\nthree\n");
    run(&env, &["save", "On side"], 0);
    run(&env, &["goto", "main"], 0);
    env.overwrite("a.txt", "one\nmain\nthree\n");
    run(&env, &["save", "On main"], 0);
    run(&env, &["combine", "side"], 0);
    let everything = || {
        (
            support::entries(env.root()),
            support::entries(&env.root().join(".opalro")),
        )
    };
    let before = everything();
    for args in [
        &["who", "a.txt"][..],
        &["who", "a.txt", "--save", "side"],
        &["who", "a.txt", "--json"],
        &["who", "none.txt"],
    ] {
        let _ = env.opalro().args(args).output().unwrap();
    }
    let (shown, _) = run(&env, &["who", "a.txt"], 0);
    // The markers of the kept conflict are the combine's lines.
    assert!(shown.contains("<<<<<<<"), "{shown}");
    assert_eq!(everything(), before);
}
