//! How `opalro status` and `opalro changes` show the places a combined save holds in conflict,
//! and how `goto`, `discard`, the shelf, `undo` and `redo` write and protect them. Saves are
//! planted with their conflicts.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use opal_core::conflict::{Own, Record, Term};
use opal_core::id::{Id, hash_content};
use opal_core::listing::{Entry, EntryKind, Listing};
use opal_core::pack::ObjectKind;
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

    // Nothing else changed: the places come straight after the first line, and nothing says
    // that nothing changed.
    put(&env, "plain.txt", b"one\n");
    let (shown, _) = run(&env, &["status"], 0);
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(lines[1..4], ["", "In conflict (7 places):", ""], "{shown}");
    assert_eq!(lines.len(), 11, "{shown}");
    assert!(!shown.contains("Nothing changed"), "{shown}");

    // A name this computer can't hold: listed once on screen, among the places; in `--json`
    // among the entries not written here too.
    if cfg!(windows) {
        let env = started();
        plant(
            &env,
            &[("plain.txt", b"one\n"), ("con.txt", b"x")],
            &[("aux.txt", versions(Some(term(PICTURE))))],
        );
        put(&env, "plain.txt", b"two\n");
        let (shown, _) = run(&env, &["status"], 0);
        let lines: Vec<&str> = shown.lines().collect();
        assert_eq!(
            lines[1..9],
            [
                "",
                "In conflict (1 place):",
                "",
                "    aux.txt  can't be written on this computer, kept as it is",
                "",
                "Changed since then (1 file):",
                "",
                "    modified  plain.txt  4 B -> 4 B",
            ],
            "{shown}"
        );
        assert_eq!(
            lines[9..],
            [
                "",
                "Not written here, because this computer can't hold them as they are (saves \
                 keep them as they were):",
                "",
                "    con.txt",
            ],
            "{shown}"
        );
        let doc = json(&env, &["status"]);
        assert_eq!(
            doc["not_written"],
            serde_json::json!(["aux.txt", "con.txt"])
        );
        assert_eq!(doc["conflicts"][0]["standing"], "not_here");
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
    let markers = "fix the marked places in your editor, then run: opalro resolve";
    let pick = "pick a version with: opalro resolve <path> --keep mine (or theirs); to look at \
                theirs first: opalro resolve <path> --copy theirs --to <file>";

    // Only the first that applies.
    let env = every_state();
    assert_eq!(next_step(&env), fixed);
    let both = |conflicts: &[(&str, Record)], shows: &[(&str, &[u8])]| {
        let env = started();
        plant(&env, &[], conflicts);
        for (path, bytes) in shows {
            put(&env, path, bytes);
        }
        next_step(&env)
    };
    let two = [
        ("a.txt", marked()),
        ("b.png", versions(Some(term(PICTURE)))),
    ];
    assert_eq!(both(&two, &[("a.txt", HALF), ("b.png", PICTURE)]), markers);

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
    let save = "save them with: opalro save \"describe your change\"";
    assert_eq!(next_step(&env), save);

    // Only a place this computer can't hold, beside a change: the save step again.
    if cfg!(windows) {
        let env = started();
        plant(
            &env,
            &[("plain.txt", b"one\n")],
            &[("aux.txt", versions(Some(term(PICTURE))))],
        );
        put(&env, "plain.txt", b"two\n");
        assert_eq!(next_step(&env), save);
    }
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
    assert_eq!(
        lines[1..4],
        ["", "In conflict (150 places):", ""],
        "{shown}"
    );
    let rows: Vec<String> = names
        .iter()
        .map(|n| row(n, 8, "your version, still to decide", 29))
        .collect();
    assert_eq!(lines[4..], rows, "{shown}");

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
fn changes_marks_a_file_below_a_conflict_standing_as_a_folder() {
    let env = started();
    let files: [(&str, EntryKind, &[u8]); 1] = [("x.txt", EntryKind::File, MINE)];
    support::plant_save(env.root(), |pack| {
        for bytes in [BASE, THEIRS] {
            pack.add_content(bytes).unwrap();
        }
        let folder = support::listings(pack, &files);
        let record = versions(Some(Term {
            kind: EntryKind::Folder,
            id: folder,
            size: 0,
        }));
        support::listings_with_conflicts(pack, &[], &[("dir", &record)])
    });
    put(&env, "dir/x.txt", b"mine\nmore\n");
    let (shown, _) = run(&env, &["changes"], 0);
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(
        lines[0],
        "In conflict: dir/x.txt (compared with what your folder held for it after the last save)",
        "{shown}"
    );
    assert!(
        lines[1].starts_with("modified  dir/x.txt  +1 -0  "),
        "{shown}"
    );
    let doc = json(&env, &["changes"]);
    assert_eq!(doc["changes"][0]["path"], "dir/x.txt");
    assert_eq!(doc["changes"][0]["in_conflict"], true);
    // status names the place once, and no change.
    let doc = json(&env, &["status"]);
    assert_eq!(doc["conflicts"][0]["path"], "dir");
    assert_eq!(doc["conflicts"][0]["standing"], "changed");
    assert_eq!(doc["changes"], serde_json::json!([]));
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
        let a = pack.add_content(b"one\n").unwrap();
        let top = Listing::new(vec![
            Entry {
                name: "a.txt".into(),
                kind: EntryKind::File,
                id: a,
                size: 4,
            },
            Entry {
                name: "art".into(),
                kind: EntryKind::Folder,
                id: inner,
                size: 0,
            },
        ])
        .unwrap();
        pack.add_record(ObjectKind::FolderListing, &top.encode())
            .unwrap()
    });
    put(&env, "art/lost.png", b"x");
    // A change met before the bad place, in path order.
    put(&env, "a.txt", b"two\n");
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
    put(&env, "a.txt", b"new\n");
    for args in [&["status"][..], &["changes"]] {
        let (shown, said) = run(&env, args, 1);
        assert_eq!(shown, "", "{args:?}");
        assert_eq!(
            said,
            "error: outer/c.png is a conflict inside a conflict, which Opalro never writes, so \
             this project may be damaged.\n\nhint: Nothing was changed. Ask for help before \
             trying to repair it.\n",
            "{args:?}"
        );
    }
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

// Writing the folder: `goto`, `discard`, the shelf, `undo` and `redo`.

/// A started project at its first save, not on a line, with `main` moved to a planted save
/// holding `plain.txt` and a conflict at each of `conflicts`. The first save's ID, and the
/// conflicted save's.
fn away_from(conflicts: &[(&str, Record)]) -> (Env, Id, Id) {
    let env = started();
    let first = support::main_tip(&env);
    run(&env, &["goto", &first.to_hex()], 0);
    plant(&env, &[("plain.txt", b"one\n")], conflicts);
    let conflicted = support::main_tip(&env);
    (env, first, conflicted)
}

/// Shown with markers, your version, and nothing.
fn three_kinds() -> (Env, Id, Id) {
    away_from(&[
        ("gone.png", versions(None)),
        ("marked.txt", marked()),
        ("yours.png", versions(Some(term(PICTURE)))),
    ])
}

fn written_conflicts() -> support::Files {
    support::Files::from([
        ("marked.txt".to_owned(), MARKED.to_vec()),
        ("plain.txt".to_owned(), b"one\n".to_vec()),
        ("yours.png".to_owned(), PICTURE.to_vec()),
    ])
}

/// The ID of the save the project is on.
fn on_save(env: &Env) -> Id {
    let repo = opal_core::repo::Repository::open(env.root()).unwrap();
    opal_core::history::current_save(&repo).unwrap()
}

#[test]
fn goto_a_conflicted_save_writes_marked_texts_and_your_versions() {
    let (env, _, conflicted) = three_kinds();
    run(&env, &["goto", "main"], 0);
    assert_eq!(support::files(env.root()), written_conflicts());
    // Nothing is unsaved: each place holds what its conflict stands as.
    assert!(support::changed(&env).is_empty());
    let doc = json(&env, &["status"]);
    assert_eq!(doc["conflicts"].as_array().unwrap().len(), 3, "{doc}");
    support::assert_folder_is_goto_of(&env, conflicted);
}

#[test]
fn goto_away_removes_them_and_goto_back_writes_them_again() {
    let (env, first, conflicted) = three_kinds();
    let before = support::files(env.root());
    run(&env, &["goto", "main"], 0);
    run(&env, &["goto", &first.to_hex()], 0);
    assert_eq!(support::files(env.root()), before);
    support::assert_folder_is_goto_of(&env, first);
    run(&env, &["goto", "main"], 0);
    assert_eq!(support::files(env.root()), written_conflicts());
    support::assert_folder_is_goto_of(&env, conflicted);
}

#[test]
fn an_untouched_conflict_file_is_not_unsaved_work() {
    let (env, first, _) = three_kinds();
    run(&env, &["goto", "main"], 0);
    let doc = json(&env, &["goto", &first.to_hex()]);
    assert_eq!(doc["safety_copies"], 0, "{doc}");
    assert_eq!(on_save(&env), first);
    assert!(!env.root().join("marked.txt").exists());
}

/// `goto <to>` refused for unsaved work at `path`: exit 1, the path named, and nothing changed.
fn refused_for(env: &Env, to: &str, path: &str) {
    let (before, on) = (support::files(env.root()), env.state().1);
    let (shown, said) = run(env, &["goto", to], 1);
    assert_eq!(shown, "");
    assert!(
        said.contains("are in the way, so nothing was changed:"),
        "{said}"
    );
    assert!(said.contains(&format!(":\n  {path}\n\n")), "{said}");
    assert_eq!(support::files(env.root()), before);
    assert_eq!(env.state().1, on);
}

#[test]
fn a_half_fixed_file_stops_a_goto_it_is_in_the_way_of() {
    let (env, first, _) = three_kinds();
    run(&env, &["goto", "main"], 0);
    put(&env, "marked.txt", HALF);
    refused_for(&env, &first.to_hex(), "marked.txt");
}

#[test]
fn a_changed_version_conflict_is_unsaved_work_too() {
    let (env, first, _) = three_kinds();
    run(&env, &["goto", "main"], 0);
    put(&env, "yours.png", b"painted over");
    refused_for(&env, &first.to_hex(), "yours.png");
}

#[test]
fn discard_gives_back_the_marked_text_and_keeps_a_safety_copy() {
    let (env, _, conflicted) = three_kinds();
    run(&env, &["goto", "main"], 0);
    put(&env, "marked.txt", HALF);
    let doc = json(&env, &["discard", "marked.txt"]);
    assert_eq!(doc["restored_paths"], serde_json::json!(["marked.txt"]));
    assert_eq!(doc["safety_copies"], 1, "{doc}");
    assert_eq!(support::files(env.root()), written_conflicts());
    // The step's "from" save holds what the folder held: the half-fixed bytes, and each other
    // place as it stood.
    let change = support::head(&env).1.folder_change.unwrap();
    let held = support::saved_files(&env, change.from);
    assert_eq!(held["marked.txt"], HALF);
    assert_eq!(held["yours.png"], PICTURE);
    assert!(!held.contains_key("gone.png"));
    support::assert_folder_is_goto_of(&env, conflicted);
}

/// A save whose `dir` is a conflict standing as a folder holding `x.txt`, as `three_kinds`.
fn folder_conflict() -> (Env, Id) {
    let env = started();
    let first = support::main_tip(&env);
    run(&env, &["goto", &first.to_hex()], 0);
    let files: [(&str, EntryKind, &[u8]); 1] = [("x.txt", EntryKind::File, MINE)];
    support::plant_save(env.root(), |pack| {
        for bytes in [BASE, THEIRS] {
            pack.add_content(bytes).unwrap();
        }
        let folder = support::listings(pack, &files);
        let record = versions(Some(Term {
            kind: EntryKind::Folder,
            id: folder,
            size: 0,
        }));
        support::listings_with_conflicts(pack, &[], &[("dir", &record)])
    });
    let conflicted = support::main_tip(&env);
    (env, conflicted)
}

#[test]
fn discard_below_a_conflict_that_stands_as_a_folder_finds_the_path() {
    let (env, conflicted) = folder_conflict();
    run(&env, &["goto", "main"], 0);
    assert_eq!(std::fs::read(env.root().join("dir/x.txt")).unwrap(), MINE);
    put(&env, "dir/x.txt", b"changed\n");
    let doc = json(&env, &["discard", "dir/x.txt"]);
    assert_eq!(doc["restored_paths"], serde_json::json!(["dir/x.txt"]));
    assert_eq!(std::fs::read(env.root().join("dir/x.txt")).unwrap(), MINE);
    support::assert_folder_is_goto_of(&env, conflicted);
}

#[test]
fn shelf_and_unshelf_carry_a_half_fixed_file_and_the_conflict_stays_in_the_save() {
    let (env, _, conflicted) = three_kinds();
    run(&env, &["goto", "main"], 0);
    put(&env, "marked.txt", HALF);
    let doc = json(&env, &["shelf"]);
    assert_eq!(doc["shelf"]["files"], 1, "{doc}");
    assert_eq!(support::files(env.root()), written_conflicts());
    let doc = json(&env, &["shelf", "--list"]);
    assert_eq!(doc["shelves"][0]["files"], 1, "{doc}");
    // The conflict stays in the save, and the line stays where it was.
    assert_eq!(support::main_tip(&env), conflicted);
    let top = support::top_folder(env.root());
    assert_eq!(top.get("marked.txt").unwrap().kind, EntryKind::Conflict);
    let doc = json(&env, &["status"]);
    assert_eq!(doc["conflicts"][1]["path"], "marked.txt", "{doc}");
    assert_eq!(doc["changes"], serde_json::json!([]));
    run(&env, &["unshelf"], 0);
    assert_eq!(std::fs::read(env.root().join("marked.txt")).unwrap(), HALF);
    assert_eq!(support::main_tip(&env), conflicted);
}

#[test]
fn undo_and_redo_over_a_goto_to_a_conflicted_save() {
    let (env, first, conflicted) = three_kinds();
    let before = support::files(env.root());
    run(&env, &["goto", "main"], 0);
    run(&env, &["undo"], 0);
    assert_eq!(support::files(env.root()), before);
    assert_eq!(on_save(&env), first);
    support::assert_folder_is_goto_of(&env, first);
    run(&env, &["redo"], 0);
    assert_eq!(support::files(env.root()), written_conflicts());
    assert_eq!(on_save(&env), conflicted);
    support::assert_folder_is_goto_of(&env, conflicted);
    // A half-fixed file is unsaved work an undo stops for, as for any.
    put(&env, "marked.txt", HALF);
    let (_, said) = run(&env, &["undo"], 1);
    assert!(said.contains("marked.txt"), "{said}");
    assert_eq!(std::fs::read(env.root().join("marked.txt")).unwrap(), HALF);
}

#[test]
fn goto_says_when_the_save_gone_to_holds_a_conflict() {
    let (env, first, _) = three_kinds();
    let said = "The save you went to holds places in conflict: opalro status lists them.";
    let (shown, _) = run(&env, &["goto", "main"], 0);
    assert!(shown.contains(said), "{shown}");
    // Leaving it for a save without says nothing of the one left.
    let (shown, _) = run(&env, &["goto", &first.to_hex()], 0);
    assert!(!shown.contains("conflict"), "{shown}");
    assert_eq!(json(&env, &["goto", "main"])["holds_conflicts"], true);
    let doc = json(&env, &["goto", &first.to_hex()]);
    assert_eq!(doc["holds_conflicts"], false, "{doc}");
}

#[test]
fn without_the_feature_write_out_leaves_a_conflicts_place_alone() {
    let (env, _, _) = three_kinds();
    support::without_conflicts_feature(env.root());
    let doc = json(&env, &["goto", "main"]);
    assert_eq!(doc["holds_conflicts"], false, "{doc}");
    let written = support::files(env.root());
    assert_eq!(written["plain.txt"], b"one\n");
    for place in ["gone.png", "marked.txt", "yours.png"] {
        assert!(!written.contains_key(place), "{written:?}");
    }
}

/// Wherever a goto to a conflicted save stops, the next command that changes the project
/// finishes it, or it never happened: the folder is what a goto to the save it is on writes.
#[cfg(feature = "fault-injection")]
#[test]
fn a_goto_to_a_conflicted_save_stopped_at_every_step_is_finished_or_never_happened() {
    let (template, first, conflicted) = three_kinds();
    let mut stopped = 0;
    for n in 1.. {
        assert!(n < 200, "the goto never finished");
        let env = Env::empty();
        support::copy_dir(template.root(), env.root());
        let out = env
            .opalro()
            .args(["goto", "main"])
            .env("OPALRO_FAULT_AT", n.to_string())
            .output()
            .unwrap();
        if !text(&out.stderr).contains("injected crash") {
            assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
            break;
        }
        stopped += 1;
        run(&env, &["discard"], 0);
        let journal = env.root().join(".opalro/state/journal");
        assert!(!journal.exists(), "fault {n}");
        let on = on_save(&env);
        assert!(on == first || on == conflicted, "fault {n}");
        support::assert_folder_is_goto_of(&env, on);
    }
    assert!(stopped > 1, "no fault point was reached");
}

/// A project on `main` at a planted save holding `plain.txt`, a conflict shown with markers at
/// `marked.txt` and one standing as your version at `yours.png`, the folder half fixed.
fn half_fixed() -> Env {
    let env = started();
    plant(
        &env,
        &[("plain.txt", b"one\n")],
        &[
            ("fixed.txt", marked()),
            ("marked.txt", marked()),
            ("yours.png", versions(Some(term(PICTURE)))),
        ],
    );
    put(&env, "plain.txt", b"two\n");
    put(&env, "fixed.txt", FIXED);
    put(&env, "marked.txt", HALF);
    put(&env, "yours.png", PICTURE);
    env
}

/// `opalro save` with `args`, which must make a save that `goto` would write as the folder is.
fn saved(env: &Env, args: &[&str]) -> String {
    let (shown, _) = run(env, &[&["save"], args].concat(), 0);
    assert!(shown.starts_with("Saved "), "{shown}");
    support::assert_folder_is_goto_of(env, support::main_tip(env));
    shown
}

#[test]
fn save_says_how_many_conflicts_it_met() {
    let env = half_fixed();
    // `fixed.txt` is fixed; `marked.txt` keeps its half-fixed text, `yours.png` stays as it is.
    let shown = saved(&env, &["Half way"]);
    assert!(
        shown.contains("\n    2 places are still in conflict: opalro status lists them.\n"),
        "{shown}"
    );
    // A save of some paths speaks only of those.
    put(&env, "yours.png", b"painted over");
    let shown = saved(&env, &["Painted", "yours.png"]);
    assert!(
        shown.contains(
            "\n    1 of the places you saved is still in conflict: opalro status lists every \
             place in conflict.\n"
        ),
        "{shown}"
    );
    put(&env, "plain.txt", b"three\n");
    let shown = saved(&env, &["Plain", "plain.txt"]);
    assert!(!shown.contains("conflict"), "{shown}");
    put(&env, "marked.txt", FIXED);
    let shown = saved(&env, &["Fixed"]);
    assert!(
        shown.contains("\n    1 place is still in conflict: opalro status lists it.\n"),
        "{shown}"
    );
    let doc = json(&env, &["status"]);
    let places: Vec<&str> = doc["conflicts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["path"].as_str().unwrap())
        .collect();
    assert_eq!(places, ["yours.png"], "{doc}");
}

#[test]
fn save_json_counts_them() {
    let env = half_fixed();
    let doc = json(&env, &["save", "Half way"]);
    assert_eq!(doc["result"], "saved", "{doc}");
    assert_eq!(doc["conflicts"], 2, "{doc}");
    support::assert_folder_is_goto_of(&env, support::main_tip(&env));
    put(&env, "plain.txt", b"three\n");
    let doc = json(&env, &["save", "Plain", "plain.txt"]);
    assert_eq!(doc["conflicts"], 0, "{doc}");
    support::assert_folder_is_goto_of(&env, support::main_tip(&env));
}
