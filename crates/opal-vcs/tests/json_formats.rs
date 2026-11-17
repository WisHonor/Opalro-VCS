//! Every command's `--json` document, checked against docs/json.md. Only the top-level fields are
//! checked: a field a document has that the spec doesn't list fails, but a field the spec lists
//! that a document lacks doesn't (some are there only in some states). The tables are found by
//! the heading of their section and their place under it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use opal_core::conflict::{Record, Term};
use opal_core::id::hash_content;
use opal_core::listing::EntryKind;
use serde_json::Value;

mod support;
use support::{Env, text};

/// The field names json.md's tables give each document, by the heading of its section and the
/// table's place under it. A table that follows a piece's name (`**A step:**`) describes a piece of
/// a document, not a document, so it isn't counted.
fn fields_in_json_md() -> BTreeMap<(String, usize), BTreeSet<String>> {
    let spec = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/json.md");
    let spec = std::fs::read_to_string(spec).unwrap();
    let mut tables: BTreeMap<(String, usize), BTreeSet<String>> = BTreeMap::new();
    let (mut section, mut count, mut before) = (String::new(), 0, "");
    let mut filling: Option<(String, usize)> = None;
    for line in spec.lines() {
        if !line.starts_with('|') {
            filling = None;
        }
        if line.starts_with("## ") || line.starts_with("### ") {
            section = line.trim_start_matches('#').trim().to_owned();
            count = 0;
        } else if line == "| Field | Type | Meaning |" {
            if !before.starts_with("**A ") && !before.starts_with("**An ") {
                filling = Some((section.clone(), count));
                count += 1;
            } else {
                // A piece's table is kept under its section and the piece's name.
                let piece = before.trim_start_matches("**").split("**").next();
                filling = Some((format!("{section}: {}", piece.unwrap_or_default()), 0));
            }
        } else if let Some(key) = &filling {
            let first = line.split('|').nth(1).unwrap_or_default();
            // `opalro_json`, `result` names two fields; `error.message` is a field of `error`; in
            // `undone` (`undo`) the brackets name a command, not a field.
            let mut outside = String::new();
            let mut depth = 0;
            for c in first.chars() {
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ if depth == 0 => outside.push(c),
                    _ => {}
                }
            }
            for name in outside.split('`').skip(1).step_by(2) {
                let name = name.split('.').next().unwrap_or_default().to_owned();
                tables.entry(key.clone()).or_default().insert(name);
            }
        }
        if !line.trim().is_empty() && !line.starts_with('|') {
            before = line;
        }
    }
    tables
}

/// `opalro args --json` in `dir`, which must exit with `code`; its document.
fn document(env: &Env, dir: &Path, args: &[&str], code: i32) -> Value {
    let out = env.opal_in(dir).args(args).arg("--json").output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(code),
        "{args:?}: {}{}",
        text(&out.stdout),
        text(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{args:?}: {e}"))
}

/// Every command's document on a small project, each with the json.md section and table that
/// describes it.
fn every_document() -> Vec<(&'static str, usize, Value)> {
    let mut docs = Vec::new();
    let elsewhere = Env::empty();
    let empty = elsewhere.root().to_owned();
    docs.push(("5. Errors", 0, document(&elsewhere, &empty, &["status"], 1)));
    let start = &["start", "--preset", "custom"];
    docs.push((
        "`opalro start --json`",
        0,
        document(&elsewhere, &empty, start, 0),
    ));

    let env = Env::new();
    let root = env.root().to_owned();
    let run = |args: &[&str], code| document(&env, &root, args, code);
    env.commit("a.txt", "one\n", "One");
    docs.push(("`opalro import --json`", 0, run(&["import"], 0)));
    docs.push(("`opalro setup --json`", 0, run(&["setup"], 0)));
    docs.push((
        "`opalro setup --json`",
        2,
        run(&["setup", "--preset", "code"], 1),
    ));
    // As v0.1 made it, so there is something to turn on.
    support::without_conflicts_feature(&root);
    let upgrades = run(&["setup"], 0);
    assert_eq!(upgrades["upgrades"], serde_json::json!(["conflicts"]));
    docs.push(("`opalro setup --json`", 0, upgrades));
    for (args, code) in [
        (&["setup", "--upgrade"][..], 1),
        (&["setup", "--upgrade", "--yes"], 0),
        (&["setup", "--upgrade"], 0),
    ] {
        docs.push(("`opalro setup --json`", 1, run(args, code)));
    }
    env.overwrite("a.txt", "two\n");
    docs.push(("`opalro status --json`", 0, run(&["status"], 0)));
    docs.push(("`opalro changes --json`", 0, run(&["changes"], 0)));
    docs.push(("`opalro save --json`", 0, run(&["save", "Two"], 0)));
    docs.push(("`opalro save --json`", 0, run(&["save", "Two"], 0)));
    let history = run(&["history"], 0);
    let first = history["saves"][1]["id"].as_str().unwrap().to_owned();
    docs.push(("`opalro history --json`", 0, history));
    docs.push(("`opalro show --json`", 0, run(&["show"], 0)));
    let who = "`opalro who --json`";
    docs.push((who, 0, run(&["who", "a.txt"], 0)));
    docs.push((who, 0, run(&["who", "a.txt", "--save", "main"], 0)));
    docs.push(("5. Errors", 0, run(&["who", "none.txt"], 1)));
    docs.push(("`opalro line --json`", 0, run(&["line"], 0)));
    docs.push(("`opalro line new --json`", 0, run(&["line", "new", "x"], 0)));
    docs.push((
        "`opalro line rename --json`",
        0,
        run(&["line", "rename", "x", "y"], 0),
    ));
    run(&["goto", "main"], 0);
    docs.push((
        "`opalro line delete --json`",
        0,
        run(&["line", "delete", "y"], 0),
    ));
    docs.push((
        "`opalro tag <name> --json`",
        0,
        run(&["tag", "v1", "First"], 0),
    ));
    docs.push(("`opalro tag --json`", 0, run(&["tag"], 0)));
    docs.push((
        "`opalro tag --delete --json`",
        0,
        run(&["tag", "--delete", "v1", "--yes"], 0),
    ));
    docs.push(("`opalro goto --json`", 0, run(&["goto", &first], 0)));
    run(&["goto", "main"], 0);
    run(&["line", "new", "side"], 0);
    env.overwrite("b.txt", "b\n");
    run(&["save", "B"], 0);
    run(&["goto", "main"], 0);
    env.overwrite("a.txt", "main\n");
    run(&["save", "Main"], 0);
    let combine = "`opalro combine --json`";
    docs.push((combine, 0, run(&["combine", "side"], 0)));
    docs.push((combine, 0, run(&["combine", "side"], 0)));
    run(&["line", "new", "ahead"], 0);
    env.overwrite("c.txt", "c\n");
    run(&["save", "C"], 0);
    run(&["goto", "main"], 0);
    let forward = run(&["combine", "ahead"], 0);
    assert_eq!(forward["result"], "forward", "{forward}");
    docs.push((combine, 0, forward));
    docs.push(("5. Errors", 0, run(&["combine", "nowhere"], 1)));
    env.overwrite("a.txt", "three\n");
    docs.push(("`opalro discard --json`", 0, run(&["discard"], 0)));
    env.overwrite("a.txt", "four\n");
    docs.push(("`opalro shelf --json`", 0, run(&["shelf"], 0)));
    docs.push((
        "`opalro shelf --list --json`",
        0,
        run(&["shelf", "--list"], 0),
    ));
    docs.push(("`opalro unshelf --json`", 0, run(&["unshelf"], 0)));
    let undo = "`opalro undo --json` and `opalro redo --json`";
    docs.push((undo, 0, run(&["undo"], 0)));
    docs.push((undo, 0, run(&["redo"], 0)));
    let listed = run(&["undo", "--list"], 0);
    let back = listed["undo"][2]["entry"].as_str().unwrap()[..8].to_owned();
    docs.push(("`opalro undo --list --json`", 0, listed));
    let all = run(&["undo", "--list", "--all"], 0);
    docs.push(("`opalro undo --list --json`", 1, all));
    docs.push((
        "`opalro undo --to <entry> --json`",
        0,
        run(&["undo", "--to", &back], 0),
    ));
    docs.push(("`opalro tidy --json`", 0, run(&["tidy"], 0)));
    docs.push(("`opalro verify --json`", 0, run(&["verify"], 0)));
    std::fs::remove_file(root.join(".opalro/state/head")).unwrap();
    let lost = run(&["undo", "--list"], 0);
    docs.push(("`opalro undo --list --json`", 2, lost));

    // A place in conflict, planted, standing in the folder as changed.
    let conflicted = Env::empty();
    let root = conflicted.root().to_owned();
    document(&conflicted, &root, start, 0);
    let mine = b"mine\n";
    let term = |bytes: &[u8]| Term {
        kind: EntryKind::File,
        id: hash_content(bytes),
        size: bytes.len() as u64,
    };
    let record = Record {
        terms: vec![Some(term(mine)), None, Some(term(b"theirs\n"))],
        own: None,
    };
    support::plant_save(&root, |pack| {
        pack.add_content(b"theirs\n").unwrap();
        pack.add_content(mine).unwrap();
        support::listings_with_conflicts(pack, &[], &[("c.txt", &record)])
    });
    std::fs::write(root.join("c.txt"), "mine, changed\n").unwrap();
    let status = document(&conflicted, &root, &["status"], 0);
    assert_eq!(status["conflicts"][0]["standing"], "changed", "{status}");
    docs.push(("`opalro status --json`", 0, status));
    let changes = document(&conflicted, &root, &["changes"], 0);
    assert_eq!(changes["changes"][0]["in_conflict"], true, "{changes}");
    docs.push(("`opalro changes --json`", 0, changes));

    // A combine that keeps a conflict.
    let kept = Env::empty();
    let root = kept.root().to_owned();
    let run = |args: &[&str], code| document(&kept, &root, args, code);
    run(start, 0);
    kept.overwrite("k.txt", "base\n");
    run(&["save", "Base"], 0);
    run(&["line", "new", "side"], 0);
    kept.overwrite("k.txt", "side\n");
    run(&["save", "Side"], 0);
    run(&["goto", "main"], 0);
    kept.overwrite("k.txt", "main\n");
    run(&["save", "Main"], 0);
    let combined = run(&["combine", "side"], 0);
    assert_eq!(combined["conflict_count"], 1, "{combined}");
    let conflict = combined["conflicts"][0].clone();
    docs.push(("`opalro combine --json`", 0, combined));
    docs.push(("`opalro combine --json`: A conflict", 0, conflict));

    // Resolving it: nothing fixed yet, a version copied out, the fix recorded, then no conflict.
    let resolve = "`opalro resolve --json`";
    docs.push((resolve, 0, run(&["resolve"], 0)));
    let to = [
        "resolve",
        "k.txt",
        "--copy",
        "theirs",
        "--to",
        "k-theirs.txt",
    ];
    docs.push((resolve, 1, run(&to, 0)));
    std::fs::remove_file(root.join("k-theirs.txt")).unwrap();
    kept.overwrite("k.txt", "both\n");
    let recorded = run(&["resolve"], 0);
    assert_eq!(recorded["result"], "recorded", "{recorded}");
    docs.push((resolve, 0, recorded));
    docs.push((resolve, 0, run(&["resolve"], 0)));
    docs
}

#[test]
fn every_document_has_only_the_fields_json_md_lists() {
    let spec = fields_in_json_md();
    for (section, table, doc) in every_document() {
        let listed = spec
            .get(&(section.to_owned(), table))
            .unwrap_or_else(|| panic!("json.md has no table {table} under {section}"));
        for field in doc.as_object().unwrap().keys() {
            assert!(
                listed.contains(field) || field == "opalro_json",
                "{section}, table {table}: json.md doesn't list `{field}`: {doc}"
            );
        }
    }
}

#[test]
fn every_document_parses_and_holds_opalro_json_and_warnings() {
    // A piece of a document (`<section>: A conflict`) is no document of its own.
    let documents = every_document().into_iter();
    for (section, _, doc) in documents.filter(|(s, _, _)| !s.contains("`: ")) {
        assert_eq!(doc["opalro_json"], 1, "{section}: {doc}");
        assert!(doc["warnings"].is_array(), "{section}: {doc}");
    }
}

#[test]
fn the_fields_of_a_table_are_read_from_its_first_column() {
    let spec = fields_in_json_md();
    let undo = &spec[&(
        "`opalro undo --json` and `opalro redo --json`".to_owned(),
        0,
    )];
    let names: Vec<&str> = undo.iter().map(String::as_str).collect();
    let want = [
        "finished",
        "opalro_json",
        "position",
        "redone",
        "shelved",
        "undone",
        "warnings",
    ];
    assert_eq!(names, want);
}
