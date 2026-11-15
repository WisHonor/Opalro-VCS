//! How `opalro status` and `opalro changes` show the places a combined save holds in conflict.
//! Saves are planted with their conflicts, and the folder is written as each place should show.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use opal_core::conflict::{Own, Record, Term};
use opal_core::id::{Id, hash_content};
use opal_core::listing::EntryKind;
use serde_json::Value;

mod support;
use support::{Env, text};

const MINE: &[u8] = b"mine\n";
const BASE: &[u8] = b"base\n";
const THEIRS: &[u8] = b"theirs\n";
const PICTURE: &[u8] = b"picture";
const MARKED: &[u8] = b"a\n<<<<<<< main (kkkk)\nmine\n||||||| where the lines parted (llll)\n\
    base\n=======\ntheirs\n>>>>>>> side (mmmm)\nb\n<<<<<<< main (kkkk)\nmine 2\n\
    ||||||| where the lines parted (llll)\n=======\ntheirs 2\n>>>>>>> side (mmmm)\n";
const HALF: &[u8] = b"a\nmine\nb\n<<<<<<< main (kkkk)\nmine 2\n\
    ||||||| where the lines parted (llll)\n=======\ntheirs 2\n>>>>>>> side (mmmm)\n";
const CLOSING: &[u8] = b"a\nmine\nb\nmine 2\n>>>>>>> side (mmmm)\n";
const FIXED: &[u8] = b"a\nmine\nb\nmine 2\n";

fn term(bytes: &[u8]) -> Term {
    Term {
        kind: EntryKind::File,
        id: hash_content(bytes),
        size: bytes.len() as u64,
    }
}

/// Yours, where the lines parted, theirs.
fn versions(mine: Option<Term>) -> Record {
    Record {
        terms: vec![mine, Some(term(BASE)), Some(term(THEIRS))],
        own: None,
    }
}

/// Shown with markers in the folder.
fn marked() -> Record {
    Record {
        own: Some(Own {
            version: Some(term(MARKED)),
            marker: 7,
        }),
        ..versions(Some(term(MINE)))
    }
}

/// A started project, on `main`.
fn started() -> Env {
    let env = Env::empty();
    env.opalro()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    env
}

/// Moves `main` to a planted save holding `files` and a conflict at each of `conflicts`, with every
/// version they name stored. The folder is left as it is.
fn plant(env: &Env, files: &[(&str, &[u8])], conflicts: &[(&str, Record)]) {
    let files: Vec<(&str, EntryKind, &[u8])> = files
        .iter()
        .map(|&(p, b)| (p, EntryKind::File, b))
        .collect();
    let conflicts: Vec<(&str, &Record)> = conflicts.iter().map(|(p, r)| (*p, r)).collect();
    support::plant_save(env.root(), |pack| {
        for bytes in [MINE, BASE, THEIRS, PICTURE, MARKED] {
            pack.add_content(bytes).unwrap();
        }
        support::listings_with_conflicts(pack, &files, &conflicts)
    });
}

fn put(env: &Env, path: &str, bytes: &[u8]) {
    let at = env.root().join(path);
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::fs::write(at, bytes).unwrap();
}

/// `opalro args` in the project, which must exit with `code`: stdout and stderr.
fn run(env: &Env, args: &[&str], code: i32) -> (String, String) {
    let out = env.opalro().args(args).output().unwrap();
    let (shown, said) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(code), "{args:?}: {shown}{said}");
    (shown, said)
}

fn json(env: &Env, args: &[&str]) -> Value {
    let (shown, _) = run(env, &[args, &["--json"]].concat(), 0);
    serde_json::from_str(&shown).unwrap()
}

/// A place's row: the path, then what it needs, right-aligned.
fn row(path: &str, path_width: usize, needs: &str, needs_width: usize) -> String {
    format!("    {path:<path_width$}  {needs:>needs_width$}")
}

/// A project holding one place in conflict in each state this computer can show, and a change.
fn every_state() -> Env {
    let env = started();
    plant(
        &env,
        &[("plain.txt", b"one\n")],
        &[
            ("changed.png", versions(Some(term(PICTURE)))),
            ("closing.txt", marked()),
            ("fixed.txt", marked()),
            ("gone.png", versions(None)),
            ("half.txt", marked()),
            ("marked.txt", marked()),
            ("yours.png", versions(Some(term(PICTURE)))),
        ],
    );
    put(&env, "plain.txt", b"two\n");
    put(&env, "changed.png", b"painted over");
    put(&env, "closing.txt", CLOSING);
    put(&env, "fixed.txt", FIXED);
    put(&env, "half.txt", HALF);
    put(&env, "marked.txt", MARKED);
    put(&env, "yours.png", PICTURE);
    env
}

#[test]
fn status_lists_the_places_in_conflict_first() {
    let env = every_state();
    let (shown, _) = run(&env, &["status"], 0);
    let lines: Vec<&str> = shown.lines().collect();
    assert!(lines[0].starts_with("On line main at save "), "{shown}");
    let (p, n) = (11, 35);
    let want = [
        String::new(),
        "In conflict (7 places):".into(),
        String::new(),
        row("changed.png", p, "changed in place, still to decide", n),
        row("closing.txt", p, "marker lines left to fix", n),
        row("fixed.txt", p, "fixed, not yet recorded", n),
        row("gone.png", p, "not in your folder, still to decide", n),
        row("half.txt", p, "1 marked place left to fix", n),
        row("marked.txt", p, "2 marked places left to fix", n),
        row("yours.png", p, "your version, still to decide", n),
        String::new(),
        "Changed since then (1 file):".into(),
        String::new(),
    ];
    assert_eq!(lines[1..=want.len()], want, "{shown}");
    // The change, and only it: no place in conflict is listed again.
    let rest = &lines[want.len() + 1..];
    assert_eq!(rest.len(), 1, "{shown}");
    assert!(rest[0].starts_with("    modified  plain.txt  "), "{shown}");

    // Nothing else changed: said first, as without conflicts.
    put(&env, "plain.txt", b"one\n");
    let (shown, _) = run(&env, &["status"], 0);
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(lines[1], "Nothing changed since then.", "{shown}");
    assert_eq!(lines[2..5], ["", "In conflict (7 places):", ""], "{shown}");
    assert_eq!(lines.len(), 12, "{shown}");

    // A name this computer can't hold: also among the entries not written here, as any is.
    if cfg!(windows) {
        let env = started();
        plant(&env, &[], &[("aux.txt", versions(Some(term(PICTURE))))]);
        let (shown, _) = run(&env, &["status"], 0);
        let lines: Vec<&str> = shown.lines().collect();
        assert_eq!(
            lines[1..],
            [
                "Nothing changed since then.",
                "",
                "In conflict (1 place):",
                "",
                "    aux.txt  can't be written on this computer, kept as it is",
                "",
                "Not written here, because this computer can't hold them as they are (saves \
                 keep them as they were):",
                "",
                "    aux.txt",
            ],
            "{shown}"
        );
    }
}

/// `status` on a terminal without colour: what it says next, its words on one line.
#[cfg(feature = "fault-injection")]
fn next_step(env: &Env) -> String {
    let out = env
        .opalro()
        .env("OPALRO_FAKE_TERMINAL", "80")
        .env("NO_COLOR", "1")
        .arg("status")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let said = text(&out.stderr);
    let next = said.split("Next: ").nth(1).unwrap_or_default();
    next.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(feature = "fault-injection")]
#[test]
fn status_says_what_to_do_next_for_what_is_left() {
    let fixed = "record what you fixed with: opalro resolve";
    let markers = "Fix the marked places in your editor, then run opalro resolve.";
    let pick = "To pick a version: opalro resolve <path> --keep mine (or theirs); to look at theirs \
                first: opalro resolve <path> --copy theirs --to <file>";

    let env = every_state();
    assert_eq!(next_step(&env), format!("{fixed} {markers} {pick}"));

    let only = |conflict: Record, shows: Option<&[u8]>| {
        let env = started();
        plant(&env, &[("plain.txt", b"one\n")], &[("c.txt", conflict)]);
        if let Some(bytes) = shows {
            put(&env, "c.txt", bytes);
        }
        // A change too: the places' step comes first, and alone.
        put(&env, "plain.txt", b"two\n");
        next_step(&env)
    };
    assert_eq!(only(marked(), Some(FIXED)), fixed);
    assert_eq!(only(marked(), Some(HALF)), markers);
    assert_eq!(only(marked(), Some(CLOSING)), markers);
    assert_eq!(only(versions(Some(term(MINE))), Some(MINE)), pick);
    assert_eq!(only(versions(Some(term(MINE))), Some(b"edited\n")), pick);
    assert_eq!(only(versions(None), None), pick);

    // No place in conflict: the save step, as before.
    let env = started();
    plant(&env, &[("plain.txt", b"one\n")], &[]);
    put(&env, "plain.txt", b"two\n");
    assert_eq!(
        next_step(&env),
        "save them with: opalro save \"describe your change\""
    );
}

#[test]
fn status_json_lists_conflicts() {
    let env = every_state();
    let doc = json(&env, &["status"]);
    let paths: Vec<&str> = doc["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["plain.txt"]);
    let rows: Vec<(&str, &str, Value)> = doc["conflicts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            assert_eq!(c["terms"], 3, "{c}");
            (
                c["path"].as_str().unwrap(),
                c["standing"].as_str().unwrap(),
                c["places_left"].clone(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("changed.png", "changed", Value::Null),
            ("closing.txt", "markers", 0.into()),
            ("fixed.txt", "fixed", Value::Null),
            ("gone.png", "nothing", Value::Null),
            ("half.txt", "markers", 1.into()),
            ("marked.txt", "markers", 2.into()),
            ("yours.png", "yours", Value::Null),
        ]
    );
    insta::assert_json_snapshot!("status_with_conflicts", support::masked(doc));

    // None: an empty list, the rest as before.
    let env = started();
    let doc = json(&env, &["status"]);
    assert_eq!(doc["conflicts"], serde_json::json!([]));
}

#[test]
fn status_lists_150_conflicts_all() {
    let env = started();
    let names: Vec<String> = (0..150).map(|n| format!("c{n:03}.png")).collect();
    let conflicts: Vec<(&str, Record)> = names
        .iter()
        .map(|n| (n.as_str(), versions(Some(term(PICTURE)))))
        .collect();
    plant(&env, &[], &conflicts);
    for n in &names {
        put(&env, n, PICTURE);
    }
    let (shown, _) = run(&env, &["status"], 0);
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(lines[3], "In conflict (150 places):", "{shown}");
    let rows: Vec<String> = names
        .iter()
        .map(|n| row(n, 8, "your version, still to decide", 29))
        .collect();
    assert_eq!(lines[5..], rows, "{shown}");

    let doc = json(&env, &["status"]);
    let listed: Vec<&str> = doc["conflicts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["path"].as_str().unwrap())
        .collect();
    assert_eq!(listed, names);
    assert_eq!(doc["changes"], serde_json::json!([]));
}

#[test]
fn changes_marks_a_file_in_conflict() {
    let env = started();
    plant(
        &env,
        &[("plain.txt", b"one\n")],
        &[("notes.txt", versions(Some(term(MINE))))],
    );
    put(&env, "notes.txt", b"mine\nmore\n");
    put(&env, "plain.txt", b"two\n");
    let (shown, _) = run(&env, &["changes"], 0);
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(
        lines[0],
        "In conflict: notes.txt (compared with what your folder held for it after the last save)"
    );
    assert!(
        lines[1].starts_with("modified  notes.txt  +1 -0  "),
        "{shown}"
    );
    // The other file has no such line.
    let plain = lines.iter().position(|l| l.contains("plain.txt")).unwrap();
    assert!(lines[plain].starts_with("modified  plain.txt"), "{shown}");
    assert_eq!(lines[plain - 1], "", "{shown}");
    assert_eq!(shown.matches("In conflict").count(), 1, "{shown}");

    let doc = json(&env, &["changes"]);
    let marked: Vec<(&str, bool)> = doc["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["path"].as_str().unwrap(),
                c["in_conflict"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(marked, [("notes.txt", true), ("plain.txt", false)]);

    // A save's own changes are never in conflict.
    let doc = json(&env, &["changes", "--save", "main"]);
    let changes = doc["changes"].as_array().unwrap();
    assert!(!changes.is_empty(), "{doc}");
    assert!(changes.iter().all(|c| c["in_conflict"] == false), "{doc}");
}

#[test]
fn a_hostile_path_in_conflict_is_escaped_in_status() {
    let env = started();
    let name = "evil\u{1b}[2J\u{202e}.png";
    plant(&env, &[], &[(name, versions(None))]);
    let (shown, said) = run(&env, &["status"], 0);
    support::assert_escaped(&(shown.clone() + &said), "status");
    assert!(shown.contains("    evil\\x1b[2J\\u{202e}.png  "), "{shown}");
    // As data, the path is given as it is.
    let doc = json(&env, &["status"]);
    assert_eq!(doc["conflicts"][0]["path"], name);
}

#[test]
fn a_record_that_cant_be_read_stops_status_naming_the_place() {
    let env = started();
    let missing = Id([0xab; 32]);
    support::plant_save(env.root(), |pack| {
        let inner = support::add_listing(pack, &["lost.png".into()], EntryKind::Conflict, missing);
        support::add_listing(pack, &["art".into()], EntryKind::Folder, inner)
    });
    put(&env, "art/lost.png", b"x");
    for args in [&["status"][..], &["changes"]] {
        let (shown, said) = run(&env, args, 1);
        assert_eq!(shown, "", "{args:?}");
        let want = format!(
            "error: The conflict at art/lost.png can't be read (record {}): ",
            missing.short_hex()
        );
        assert!(said.starts_with(&want), "{args:?}: {said}");
        assert!(
            said.ends_with(
                "\n\nhint: Nothing was changed. Ask for help before trying to repair it.\n"
            ),
            "{args:?}: {said}"
        );
    }
}

#[test]
fn a_planted_conflict_inside_a_conflict_stops_status_as_damage() {
    let env = started();
    support::plant_save(env.root(), |pack| {
        for bytes in [BASE, THEIRS] {
            pack.add_content(bytes).unwrap();
        }
        let inner = support::listings_with_conflicts(pack, &[], &[("c.png", &versions(None))]);
        let outer = versions(Some(Term {
            kind: EntryKind::Folder,
            id: inner,
            size: 0,
        }));
        support::listings_with_conflicts(pack, &[], &[("outer", &outer)])
    });
    put(&env, "outer/c.png", b"x");
    let (shown, said) = run(&env, &["status"], 1);
    assert_eq!(shown, "");
    assert_eq!(
        said,
        "error: outer/c.png is a conflict inside a conflict, which Opalro never writes, so this \
         project may be damaged.\n\nhint: Nothing was changed. Ask for help before trying to \
         repair it.\n"
    );
}

#[test]
fn without_the_feature_status_shows_a_planted_conflict_as_v01_does() {
    let env = started();
    plant(&env, &[], &[("yours.png", versions(Some(term(PICTURE))))]);
    put(&env, "yours.png", PICTURE);
    support::without_conflicts_feature(env.root());
    let (shown, _) = run(&env, &["status"], 0);
    assert!(!shown.contains("In conflict"), "{shown}");
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(
        lines[1..4],
        ["", "Changed since then (1 file):", ""],
        "{shown}"
    );
    assert!(
        lines[4].starts_with("    kind changed  yours.png  conflict "),
        "{shown}"
    );
    let doc = json(&env, &["status"]);
    assert_eq!(doc["conflicts"], serde_json::json!([]));
    assert_eq!(doc["changes"][0]["change"], "kind_changed");
    let doc = json(&env, &["changes"]);
    assert_eq!(doc["changes"][0]["in_conflict"], false);
}
