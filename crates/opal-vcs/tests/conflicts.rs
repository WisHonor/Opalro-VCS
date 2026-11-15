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
    let changed = format!(
        "keep what your folder holds with: opalro resolve <path> (each by its own path), or {pick}"
    );
    assert_eq!(only(versions(Some(term(MINE))), Some(b"edited\n")), changed);
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

    // Between two saves, `in_conflict` says whether the newer one holds a conflict there.
    let doc = json(&env, &["changes", "--save", "main"]);
    let marked: Vec<(&str, &str, bool)> = doc["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["path"].as_str().unwrap(),
                c["change"].as_str().unwrap(),
                c["in_conflict"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        marked,
        [
            ("notes.txt", "in_conflict", true),
            ("plain.txt", "added", false)
        ],
        "{doc}"
    );
    let (shown, _) = run(&env, &["changes", "--save", "main"], 0);
    assert!(!shown.contains("In conflict:"), "{shown}");
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
fn goto_tells_of_a_conflict_standing_as_nothing_or_as_a_folder() {
    let said = "The save you went to holds places in conflict: opalro status lists them.";
    // You deleted it, they changed it: nothing is written, and the person is told all the same.
    let (env, _, _) = away_from(&[("gone.png", versions(None))]);
    let (shown, _) = run(&env, &["goto", "main"], 0);
    assert!(shown.contains(said), "{shown}");
    assert!(!env.root().join("gone.png").exists());
    let (env, _, _) = away_from(&[("gone.png", versions(None))]);
    assert_eq!(json(&env, &["goto", "main"])["holds_conflicts"], true);
    let (env, _) = folder_conflict();
    let (shown, _) = run(&env, &["goto", "main"], 0);
    assert!(shown.contains(said), "{shown}");
    let (env, _) = folder_conflict();
    assert_eq!(json(&env, &["goto", "main"])["holds_conflicts"], true);
}

#[test]
fn discard_removes_a_file_put_where_a_conflict_stands_as_nothing() {
    let (env, _, conflicted) = three_kinds();
    run(&env, &["goto", "main"], 0);
    put(&env, "gone.png", b"new");
    let doc = json(&env, &["discard", "gone.png"]);
    assert_eq!(
        doc["removed_paths"],
        serde_json::json!(["gone.png"]),
        "{doc}"
    );
    assert_eq!(doc["safety_copies"], 1, "{doc}");
    assert!(!env.root().join("gone.png").exists());
    support::assert_folder_is_goto_of(&env, conflicted);
    // The safety copy is the step's "from" save.
    let change = support::head(&env).1.folder_change.unwrap();
    assert_eq!(support::saved_files(&env, change.from)["gone.png"], b"new");
}

/// The folder as written for the conflicted save, with `marked.txt` and `plain.txt` as given.
fn folder_with(marked: &[u8], plain: &[u8]) -> support::Files {
    let mut files = written_conflicts();
    files.insert("marked.txt".into(), marked.to_vec());
    files.insert("plain.txt".into(), plain.to_vec());
    files
}

#[test]
fn undo_and_redo_over_a_discard_and_a_shelf_at_a_conflicts_place() {
    let (env, _, conflicted) = three_kinds();
    run(&env, &["goto", "main"], 0);
    for command in ["discard", "shelf"] {
        put(&env, "marked.txt", HALF);
        if command == "discard" {
            run(&env, &["discard", "marked.txt"], 0);
        } else {
            run(&env, &["shelf"], 0);
        }
        assert_eq!(support::files(env.root()), written_conflicts(), "{command}");
        support::assert_folder_is_goto_of(&env, conflicted);
        // Edited in between, elsewhere: it comes along through the undo and the redo.
        put(&env, "plain.txt", b"edited\n");
        run(&env, &["undo"], 0);
        assert_eq!(
            support::files(env.root()),
            folder_with(HALF, b"edited\n"),
            "{command}"
        );
        run(&env, &["redo"], 0);
        assert_eq!(
            support::files(env.root()),
            folder_with(MARKED, b"edited\n"),
            "{command}"
        );
        run(&env, &["discard", "plain.txt"], 0);
        support::assert_folder_is_goto_of(&env, conflicted);
        assert_eq!(on_save(&env), conflicted);
    }
}

#[test]
fn tidy_keeps_what_kept_saves_picturing_a_conflict_name() {
    let (env, first, conflicted) = three_kinds();
    run(&env, &["goto", "main"], 0);
    put(&env, "marked.txt", HALF);
    run(&env, &["shelf"], 0);
    run(&env, &["goto", &first.to_hex()], 0);
    run(&env, &["tidy"], 0);
    assert_eq!(json(&env, &["verify"])["problems_total"], 0);
    run(&env, &["undo"], 0);
    assert_eq!(on_save(&env), conflicted);
    support::assert_folder_is_goto_of(&env, conflicted);
    run(&env, &["unshelf"], 0);
    assert_eq!(std::fs::read(env.root().join("marked.txt")).unwrap(), HALF);
    run(&env, &["tidy"], 0);
    let doc = json(&env, &["verify"]);
    assert_eq!(doc["problems_total"], 0, "{doc}");
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
        // The discard finishes the switch first; recovery alone must leave nothing to discard.
        let doc = json(&env, &["discard"]);
        assert_eq!(
            doc["restored_paths"],
            serde_json::json!([]),
            "fault {n}: {doc}"
        );
        assert_eq!(
            doc["removed_paths"],
            serde_json::json!([]),
            "fault {n}: {doc}"
        );
        assert_eq!(doc["safety_copies"], 0, "fault {n}: {doc}");
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

#[test]
fn a_conflict_kept_in_a_gone_folder_a_file_now_names_stops_the_save_in_its_own_words() {
    let env = started();
    plant(
        &env,
        &[],
        &[("art/hero.png", versions(Some(term(PICTURE))))],
    );
    put(&env, "art", b"a file where the folder was");
    let before = env.state().1;
    let (shown, said) = run(&env, &["save", "Over"], 1);
    assert_eq!(shown, "");
    assert!(
        said.contains(
            "Opalro can't save art: the last save has a folder there holding art/hero.png, \
             which is still in conflict, and a save can't hold both under one name."
        ),
        "{said}"
    );
    assert!(
        said.contains("Nothing was saved. Rename or move art, then save again."),
        "{said}"
    );
    assert_eq!(env.state().1, before);
}

#[test]
fn a_new_name_clashing_with_a_conflict_kept_with_nothing_there_stops_the_save() {
    let env = started();
    plant(
        &env,
        &[("plain.txt", b"one\n")],
        &[("Hero.png", versions(Some(term(PICTURE))))],
    );
    put(&env, "plain.txt", b"one\n");
    put(&env, "hero.png", b"another picture");
    let before = env.state().1;
    let (shown, said) = run(&env, &["save", "Another"], 1);
    assert_eq!(shown, "");
    assert!(
        said.contains(
            "\"hero.png\" and \"Hero.png\" differ only in upper/lower case, so they clash on \
             Windows and macOS"
        ),
        "{said}"
    );
    assert!(
        said.contains("Nothing was saved. Rename them, or list them in .opalroignore."),
        "{said}"
    );
    assert_eq!(env.state().1, before);
}

// What a save did to its conflicts: `show` and `changes --save`. Made by combining.

const PIC_START: &[u8] = b"\x89PNG\r\n\x1a\n\0start";
const PIC_SIDE: &[u8] = b"\x89PNG\r\n\x1a\n\0side";
const PIC_MAIN: &[u8] = b"\x89PNG\r\n\x1a\n\0main";

/// A project whose `main` holds three saves after its start: `made`, a combine that put `a.txt`,
/// `d.txt` and `pic.png` in conflict; `kept`, a save of `a.txt` changed with its markers left,
/// the picture changed in place and `d.txt` fixed; and `fixed`, a `resolve` keeping your picture.
/// Their fingerprints, in that order.
fn made_kept_fixed() -> (Env, [String; 3]) {
    let env = started();
    let (a, d) = ("1\n2\n3\n", "x\ny\nz\n");
    env.overwrite("a.txt", a);
    env.overwrite("d.txt", d);
    put(&env, "pic.png", PIC_START);
    run(&env, &["save", "Start"], 0);
    run(&env, &["line", "new", "side"], 0);
    env.overwrite("a.txt", &a.replace('2', "side"));
    env.overwrite("d.txt", &d.replace('y', "side"));
    put(&env, "pic.png", PIC_SIDE);
    run(&env, &["save", "On side"], 0);
    run(&env, &["goto", "main"], 0);
    env.overwrite("a.txt", &a.replace('2', "main"));
    env.overwrite("d.txt", &d.replace('y', "main"));
    put(&env, "pic.png", PIC_MAIN);
    run(&env, &["save", "On main"], 0);
    run(&env, &["combine", "side"], 0);
    let made = support::main_tip(&env).to_hex();
    let marked = std::fs::read_to_string(env.root().join("a.txt")).unwrap();
    assert!(marked.contains("<<<<<<<"), "{marked}");
    env.overwrite("a.txt", &format!("0\n{marked}"));
    env.overwrite("d.txt", "x\nboth\nz\n");
    put(&env, "pic.png", b"painted over");
    run(&env, &["save", "Half way"], 0);
    let kept = support::main_tip(&env).to_hex();
    run(&env, &["resolve", "pic.png", "--keep", "mine"], 0);
    let fixed = support::main_tip(&env).to_hex();
    (env, [made, kept, fixed])
}

#[test]
fn show_counts_the_places_a_save_made_kept_and_fixed() {
    let (env, [made, kept, fixed]) = made_kept_fixed();
    let (shown, _) = run(&env, &["show", &made], 0);
    assert!(
        shown.ends_with(
            "\n3 files changed:\n\n    in conflict  a.txt\n    in conflict  d.txt\n    in \
             conflict  pic.png\n\n3 places became conflicts in this save.\n"
        ),
        "{shown}"
    );
    let (shown, _) = run(&env, &["show", &kept], 0);
    assert!(
        shown.ends_with(
            "\n3 files changed:\n\n    still in conflict  a.txt\n    conflict fixed     d.txt\n    \
             still in conflict  pic.png\n\n2 places stayed in conflict, with another version.\n1 \
             conflict was fixed in this save.\n"
        ),
        "{shown}"
    );
    let (shown, _) = run(&env, &["show", &fixed], 0);
    assert!(
        shown.ends_with(
            "\n1 file changed:\n\n    conflict fixed  pic.png\n\n1 conflict was fixed in this \
             save.\n"
        ),
        "{shown}"
    );
    // A save that touched no conflict says nothing of them.
    env.overwrite("d.txt", "x\nagain\nz\n");
    run(&env, &["save", "Plain"], 0);
    let (shown, _) = run(&env, &["show"], 0);
    assert!(
        shown.ends_with("\n1 file changed:\n\n    modified  d.txt  9 B -> 10 B\n"),
        "{shown}"
    );
}

#[test]
fn show_and_changes_save_name_a_conflict_made_still_there_and_fixed() {
    let (env, [made, kept, fixed]) = made_kept_fixed();
    for (save, rows) in [
        (
            &made,
            &[
                "in conflict  a.txt",
                "in conflict  d.txt",
                "in conflict  pic.png",
            ][..],
        ),
        (
            &kept,
            &[
                "still in conflict  a.txt",
                "conflict fixed  d.txt",
                "still in conflict  pic.png",
            ],
        ),
        (&fixed, &["conflict fixed  pic.png"]),
    ] {
        let (shown, _) = run(&env, &["changes", "--save", save], 0);
        let headers: Vec<&str> = shown.lines().filter(|l| !l.is_empty()).collect();
        // No lines are compared for them: each is its header alone.
        assert_eq!(headers, rows, "{shown}");
        let doc = json(&env, &["changes", "--save", save]);
        for c in doc["changes"].as_array().unwrap() {
            assert_eq!(c["content"], "text", "{doc}");
            assert_eq!(c["hunks"], serde_json::json!([]), "{doc}");
            // Whether the newer save holds a conflict there.
            let held = c["change"] != "conflict_fixed";
            assert_eq!(c["in_conflict"], held, "{doc}");
        }
        let doc = json(&env, &["show", save]);
        for c in doc["changes"].as_array().unwrap() {
            assert_eq!(c["in_conflict"], c["change"] != "conflict_fixed", "{doc}");
        }
    }
}

#[test]
fn history_is_unchanged_by_conflicts() {
    let (env, [made, ..]) = made_kept_fixed();
    let (shown, _) = run(&env, &["history"], 0);
    let row = shown
        .lines()
        .find(|l| l.contains("Combine side into main"))
        .unwrap();
    assert!(!row.contains("conflict"), "{row}");
    assert!(row.contains(" [combined "), "{row}");
    let doc = json(&env, &["history"]);
    let saves = doc["saves"].as_array().unwrap();
    let keys = |s: &Value| s.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    let combined = saves.iter().find(|s| s["id"] == made.as_str()).unwrap();
    for save in saves {
        assert_eq!(keys(save), keys(combined), "{doc}");
    }
}

#[test]
fn show_json_counts_them() {
    let (env, [made, kept, fixed]) = made_kept_fixed();
    for (save, counts, changes) in [
        (
            &made,
            [3, 0, 0],
            &["in_conflict", "in_conflict", "in_conflict"][..],
        ),
        (
            &kept,
            [0, 2, 1],
            &["still_in_conflict", "conflict_fixed", "still_in_conflict"],
        ),
        (&fixed, [0, 0, 1], &["conflict_fixed"]),
    ] {
        let doc = json(&env, &["show", save]);
        let got =
            ["conflicts_made", "conflicts_changed", "conflicts_fixed"].map(|k| doc[k].clone());
        assert_eq!(got, counts.map(Value::from), "{doc}");
        let kinds: Vec<&str> = doc["changes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["change"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, changes, "{doc}");
    }
    let doc = json(&env, &["show", &made]);
    assert_eq!(doc["changes"][0]["kind"], "conflict", "{doc}");
}

#[test]
fn without_the_feature_show_names_a_planted_conflict_as_v01_does() {
    let env = started();
    plant(&env, &[], &[("yours.png", versions(Some(term(PICTURE))))]);
    support::without_conflicts_feature(env.root());
    // Whole outputs: the text as v0.1 printed it; in `--json` a conflict's entry is an added
    // file of kind conflict, as in v0.1, with the fields added since at `false` and `0`.
    let doc = json(&env, &["show"]);
    let save = doc["save"].clone();
    let change = serde_json::json!({
        "change": "added",
        "kind": "conflict",
        "new_size": 0,
        "old_size": null,
        "path": "yours.png",
    });
    let mut shown_change = change.clone();
    shown_change["in_conflict"] = false.into();
    assert_eq!(
        doc,
        serde_json::json!({
            "changes": [shown_change],
            "conflicts_changed": 0,
            "conflicts_fixed": 0,
            "conflicts_made": 0,
            "opalro_json": 1,
            "save": save,
            "tags": [],
            "warnings": [],
        })
    );
    let (shown, _) = run(&env, &["show"], 0);
    let short = save["change_id_short"].as_str().unwrap();
    let head = format!("Save {short} \"Planted\"\n\n");
    assert!(shown.starts_with(&head), "{shown}");
    let fingerprint = format!("    Fingerprint  {}\n", save["id"].as_str().unwrap());
    let (_, tail) = shown.split_once(&fingerprint).unwrap();
    assert_eq!(tail, "\n1 file changed:\n\n    added  yours.png  0 B\n");
    let (shown, _) = run(&env, &["changes", "--save", "main"], 0);
    assert_eq!(shown, "added  yours.png  +0 -0  (0 B)\n");
    let doc = json(&env, &["changes", "--save", "main"]);
    let mut file = change.clone();
    for (k, v) in [
        ("content", serde_json::json!("text")),
        ("hunks", serde_json::json!([])),
        ("in_conflict", serde_json::json!(false)),
        ("only_line_endings", serde_json::json!(false)),
        ("timed_out", serde_json::json!(false)),
    ] {
        file[k] = v;
    }
    assert_eq!(
        doc,
        serde_json::json!({
            "changes": [file],
            "opalro_json": 1,
            "save": save,
            "warnings": [],
        })
    );
}

/// `main` and `side` apart from a save holding `dir/x.txt` and `gone.txt`: `side` makes `dir` a
/// file and changes `gone.txt`, `main` changes `dir/x.txt` and removes `gone.txt`; then combined,
/// so `dir` is a conflict standing as your folder and `gone.txt` one standing as nothing.
fn folder_and_gone() -> Env {
    let env = started();
    put(&env, "dir/x.txt", b"x\n");
    put(&env, "gone.txt", b"gone\n");
    run(&env, &["save", "Start"], 0);
    run(&env, &["line", "new", "side"], 0);
    std::fs::remove_dir_all(env.root().join("dir")).unwrap();
    put(&env, "dir", b"a file now\n");
    put(&env, "gone.txt", b"changed\n");
    run(&env, &["save", "On side"], 0);
    run(&env, &["goto", "main"], 0);
    put(&env, "dir/x.txt", b"main\n");
    std::fs::remove_file(env.root().join("gone.txt")).unwrap();
    run(&env, &["save", "On main"], 0);
    run(&env, &["combine", "side"], 0);
    env
}

#[test]
fn a_folder_that_became_a_conflict_and_back_is_one_row() {
    let env = folder_and_gone();
    let (shown, _) = run(&env, &["show"], 0);
    assert!(
        shown.ends_with(
            "\n2 files changed:\n\n    in conflict  dir\n    in conflict  gone.txt\n\n2 places \
             became conflicts in this save.\n"
        ),
        "{shown}"
    );
    let (shown, _) = run(&env, &["changes", "--save", "main"], 0);
    assert_eq!(shown, "in conflict  dir\n\nin conflict  gone.txt\n");
    let doc = json(&env, &["show"]);
    let rows: Vec<(&str, &str, bool)> = doc["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let (p, k) = (c["path"].as_str().unwrap(), c["change"].as_str().unwrap());
            (p, k, c["in_conflict"].as_bool().unwrap())
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("dir", "in_conflict", true),
            ("gone.txt", "in_conflict", true)
        ],
        "{doc}"
    );
    assert_eq!(doc["conflicts_made"], 2, "{doc}");
    // Back to the folder, and the gone file removed: each one row. A save first, so the fix is
    // a save of its own and not folded into the combined one.
    put(&env, "other.txt", b"o\n");
    run(&env, &["save", "Other", "other.txt"], 0);
    run(&env, &["resolve", "dir", "gone.txt", "--keep", "mine"], 0);
    let (shown, _) = run(&env, &["show"], 0);
    assert!(
        shown.ends_with(
            "\n2 files changed:\n\n    conflict fixed  dir\n    conflict fixed  gone.txt\n\n2 \
             conflicts were fixed in this save.\n"
        ),
        "{shown}"
    );
    let doc = json(&env, &["changes", "--save", "main"]);
    let rows: Vec<(&str, &str, bool)> = doc["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let (p, k) = (c["path"].as_str().unwrap(), c["change"].as_str().unwrap());
            (p, k, c["in_conflict"].as_bool().unwrap())
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("dir", "conflict_fixed", false),
            ("gone.txt", "conflict_fixed", false)
        ],
        "{doc}"
    );
    assert_eq!(json(&env, &["show"])["conflicts_fixed"], 2);
}

/// A conflict in a folder the save added is a place that became a conflict; in a folder the save
/// removed, one that stopped being one: the place itself is gone.
#[test]
fn a_conflict_in_a_folder_added_or_removed_is_named_at_its_place() {
    let env = started();
    plant(
        &env,
        &[("art/plain.txt", b"one\n")],
        &[("art/pic.png", versions(Some(term(PICTURE))))],
    );
    let (shown, _) = run(&env, &["show"], 0);
    assert!(
        shown.ends_with(
            "\n2 files changed:\n\n    in conflict  art/pic.png\n    added        \
             art/plain.txt  4 B\n\n1 place became a conflict in this save.\n"
        ),
        "{shown}"
    );
    plant(&env, &[("plain.txt", b"one\n")], &[]);
    let (shown, _) = run(&env, &["show"], 0);
    assert!(
        shown.ends_with(
            "\n3 files changed:\n\n    conflict fixed  art/pic.png\n    removed         \
             art/plain.txt  4 B\n    added           plain.txt      4 B\n\n1 conflict was fixed \
             in this save.\n"
        ),
        "{shown}"
    );
}

#[test]
fn a_conflict_row_keeps_the_paths_own_trailing_spaces() {
    let env = started();
    let (nbsp, plain, wide) = ("b\u{a0}", "c ", "pictures\u{3000}");
    plant(
        &env,
        &[],
        &[
            (nbsp, versions(Some(term(PICTURE)))),
            (plain, versions(Some(term(PICTURE)))),
            (wide, versions(Some(term(PICTURE)))),
        ],
    );
    let (shown, _) = run(&env, &["show"], 0);
    assert!(
        shown.ends_with(&format!(
            "\n    in conflict  {nbsp}\n    in conflict  {plain}\n    in conflict  {wide}\n\n3 \
             places became conflicts in this save.\n"
        )),
        "{shown:?}"
    );
}

/// `main` and `side` apart from a save holding `dir/x.txt` and `sibling`, whose name sorts
/// between `dir` and `dir/`: `side` makes `dir` a file, and both change `sibling`'s line apart;
/// then combined, so both are places in conflict, `dir` standing as your folder.
fn dir_and(sibling: &str) -> Env {
    let env = started();
    put(&env, "dir/x.txt", b"x\n");
    put(&env, sibling, b"one\n");
    run(&env, &["save", "Start"], 0);
    run(&env, &["line", "new", "side"], 0);
    std::fs::remove_dir_all(env.root().join("dir")).unwrap();
    put(&env, "dir", b"a file now\n");
    put(&env, sibling, b"side\n");
    run(&env, &["save", "On side"], 0);
    run(&env, &["goto", "main"], 0);
    put(&env, "dir/x.txt", b"main\n");
    put(&env, sibling, b"main\n");
    run(&env, &["save", "On main"], 0);
    run(&env, &["combine", "side"], 0);
    env
}

/// The `change` and `in_conflict` of each change `--json` gives for `args`, by path.
fn kinds(env: &Env, args: &[&str]) -> Vec<(String, String, bool)> {
    let doc = json(env, args);
    doc["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["path"].as_str().unwrap().to_owned(),
                c["change"].as_str().unwrap().to_owned(),
                c["in_conflict"].as_bool().unwrap(),
            )
        })
        .collect()
}

#[test]
fn a_place_is_one_row_whatever_sorts_between_it_and_its_folder() {
    for sibling in ["dir.txt", "dir-old", "dir 2"] {
        let env = dir_and(sibling);
        let (shown, _) = run(&env, &["show"], 0);
        assert!(
            shown.ends_with(&format!(
                "\n2 files changed:\n\n    in conflict  dir\n    in conflict  {sibling}\n\n2 places \
                 became conflicts in this save.\n"
            )),
            "{sibling}: {shown}"
        );
        let (shown, _) = run(&env, &["changes", "--save", "main"], 0);
        assert_eq!(
            shown,
            format!("in conflict  dir\n\nin conflict  {sibling}\n"),
            "{sibling}"
        );
        let made = [
            ("dir".to_owned(), "in_conflict".to_owned(), true),
            (sibling.to_owned(), "in_conflict".to_owned(), true),
        ];
        assert_eq!(kinds(&env, &["show"]), made, "{sibling}");
        assert_eq!(
            kinds(&env, &["changes", "--save", "main"]),
            made,
            "{sibling}"
        );
        assert_eq!(json(&env, &["show"])["conflicts_made"], 2, "{sibling}");
        // Both fixed in one save of their own.
        put(&env, "other.txt", b"o\n");
        run(&env, &["save", "Other", "other.txt"], 0);
        run(&env, &["resolve", "dir", sibling, "--keep", "mine"], 0);
        let (shown, _) = run(&env, &["show"], 0);
        assert!(
            shown.ends_with(&format!(
                "\n2 files changed:\n\n    conflict fixed  dir\n    conflict fixed  {sibling}\n\n2 \
                 conflicts were fixed in this save.\n"
            )),
            "{sibling}: {shown}"
        );
        let fixed = [
            ("dir".to_owned(), "conflict_fixed".to_owned(), false),
            (sibling.to_owned(), "conflict_fixed".to_owned(), false),
        ];
        assert_eq!(kinds(&env, &["show"]), fixed, "{sibling}");
        assert_eq!(
            kinds(&env, &["changes", "--save", "main"]),
            fixed,
            "{sibling}"
        );
        assert_eq!(json(&env, &["show"])["conflicts_fixed"], 2, "{sibling}");
    }
}

#[test]
fn a_path_at_or_below_a_place_selects_the_places_row() {
    let env = dir_and("dir.txt");
    for path in ["dir", "dir/x.txt"] {
        let (shown, _) = run(&env, &["changes", "--save", "main", path], 0);
        assert_eq!(shown, "in conflict  dir\n", "{path}");
    }
}
