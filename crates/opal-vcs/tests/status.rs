//! End-to-end tests of `opalro status`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use opal_core::id::Id;
use opal_core::link::ProjectLink;
use opal_core::listing::{Entry, EntryKind, Listing};
use opal_core::pack::ObjectKind;

mod support;
use support::{Env, main_tip, plant_save, saved_paths, text};

/// A project with `files` saved, each last changed an hour ago so the cache can vouch for it.
fn project(files: &[(&str, &str)]) -> Env {
    let env = Env::empty();
    env.opalro()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    for (name, content) in files {
        write_old(&env.root().join(name), content.as_bytes());
    }
    env.opalro().args(["save", "Files"]).assert().success();
    env
}

/// Writes `path`, then sets its time an hour back.
fn write_old(path: &Path, bytes: &[u8]) {
    if let Some(up) = path.parent() {
        fs::create_dir_all(up).unwrap();
    }
    fs::write(path, bytes).unwrap();
    set_time(path, SystemTime::now() - Duration::from_secs(3600));
}

fn set_time(path: &Path, time: SystemTime) {
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(time).unwrap();
}

/// `opalro --json status` with `args`, which must succeed.
fn status_json(env: &Env, args: &[&str]) -> serde_json::Value {
    let out = env
        .opalro()
        .args(["--json", "status"])
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

/// Each change as `(path, change, kind)`.
fn changes(doc: &serde_json::Value) -> Vec<(String, String, String)> {
    let s = |v: &serde_json::Value| v.as_str().unwrap().to_owned();
    doc["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (s(&c["path"]), s(&c["change"]), s(&c["kind"])))
        .collect()
}

fn change(path: &str, change: &str, kind: &str) -> (String, String, String) {
    (path.into(), change.into(), kind.into())
}

/// Makes `c` another kind of file than it was: executable on Unix, a link where Windows allows
/// one. False where neither can be done.
fn change_kind(c: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(c, fs::Permissions::from_mode(0o755)).unwrap();
        true
    }
    #[cfg(windows)]
    {
        fs::remove_file(c).unwrap();
        if std::os::windows::fs::symlink_file("b.txt", c).is_ok() {
            return true;
        }
        fs::write(c, "c").unwrap();
        false
    }
}

#[test]
fn status_lists_added_modified_removed_and_kind_changes() {
    let env = project(&[
        ("a.txt", "a"),
        ("b.txt", "b"),
        ("c", "c"),
        ("sub/s.txt", "s"),
    ]);
    fs::write(env.root().join("a.txt"), "changed").unwrap();
    fs::remove_file(env.root().join("b.txt")).unwrap();
    fs::remove_dir_all(env.root().join("sub")).unwrap();
    // A name that reads backwards in a terminal unless escaped.
    fs::write(env.root().join("new\u{202e}.txt"), "new").unwrap();
    let kind_changed = change_kind(&env.root().join("c"));
    let doc = status_json(&env, &[]);
    let mut expected = vec![
        change("a.txt", "modified", "file"),
        change("b.txt", "removed", "file"),
    ];
    if kind_changed {
        let kind = if cfg!(unix) { "executable" } else { "symlink" };
        expected.push(change("c", "kind_changed", kind));
    }
    expected.push(change("new\u{202e}.txt", "added", "file"));
    expected.push(change("sub/s.txt", "removed", "file"));
    assert_eq!(changes(&doc), expected, "{doc}");
    assert_eq!(doc["changes"][0]["old_size"], 1);
    assert_eq!(doc["changes"][0]["new_size"], 7);
    assert_eq!(doc["position"], serde_json::json!({"line": "main"}));
    // For people: the position, then a line per change, the name escaped.
    let out = env.opalro().arg("status").output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let said = text(&out.stdout);
    assert!(said.starts_with("On line main at save "), "{said}");
    // The kind column is as wide as its widest word: "kind changed" where a kind changed.
    let width = if kind_changed { 12 } else { 8 };
    for (kind, path) in [
        ("modified", "a.txt"),
        ("removed", "b.txt"),
        ("added", "new\\u{202e}.txt"),
        ("removed", "sub/s.txt"),
    ] {
        let line = format!("    {kind:<width$}  {path} ");
        assert!(said.contains(&line), "{line:?} in {said}");
    }
    assert!(!said.contains('\u{202e}'), "{said}");
    // Saving makes it clean again.
    env.opalro().args(["save", "More"]).assert().success();
    let said = text(&env.opalro().arg("status").output().unwrap().stdout);
    assert!(said.contains("Nothing changed since "), "{said}");
    assert!(changes(&status_json(&env, &[])).is_empty());
}

/// Makes `path` something Opalro can't open, and gives what keeps it so; `None` where nothing
/// can (root reads everything).
fn lock(path: &Path) -> Option<Box<dyn std::any::Any>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        struct Unlock(std::path::PathBuf);
        impl Drop for Unlock {
            fn drop(&mut self) {
                let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o644));
            }
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();
        let unlock = Unlock(path.to_owned());
        fs::read(path)
            .is_err()
            .then(|| Box::new(unlock) as Box<dyn std::any::Any>)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let file = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(path)
            .unwrap();
        Some(Box::new(file))
    }
}

#[cfg(any(windows, feature = "fault-injection"))]
#[test]
fn status_does_not_open_unchanged_files() {
    let env = project(&[("a.txt", "a"), ("locked.txt", "secret")]);
    // The first status reads every file and caches what it saw.
    assert!(changes(&status_json(&env, &[])).is_empty());
    // Opalro can't open it now. Not by taking the right to read it on Unix: that moves its change
    // time, and the cache stops vouching for it.
    #[cfg(windows)]
    let _held = lock(&env.root().join("locked.txt"));
    fs::write(env.root().join("a.txt"), "changed").unwrap();
    let status = |args: &[&str]| {
        env.opalro()
            .env("OPALRO_FAULT_UNREADABLE", "locked.txt")
            .args(args)
            .output()
            .unwrap()
    };
    let out = status(&["--json", "status"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        changes(&doc),
        [change("a.txt", "modified", "file")],
        "{doc}"
    );
    // And a status that must open it can't.
    let out = status(&["status", "--full"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("Can't read locked.txt"));
}

#[test]
fn status_full_reads_everything() {
    let env = project(&[("a.txt", "aaa"), ("b.txt", "b")]);
    assert!(changes(&status_json(&env, &[])).is_empty());
    // Same size, same time, written over in place: its change time tells (decision 069).
    let a = env.root().join("a.txt");
    let time = fs::metadata(&a).unwrap().modified().unwrap();
    fs::write(&a, "bbb").unwrap();
    set_time(&a, time);
    let doc = status_json(&env, &[]);
    assert_eq!(
        changes(&doc),
        [change("a.txt", "modified", "file")],
        "{doc}"
    );
    let doc = status_json(&env, &["--full"]);
    assert_eq!(
        changes(&doc),
        [change("a.txt", "modified", "file")],
        "{doc}"
    );
    // And --full opens the files the cache vouches for.
    let Some(_held) = lock(&env.root().join("b.txt")) else {
        return;
    };
    let out = env.opalro().args(["status", "--full"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    // Status only reads, so it says nothing of saving.
    let said = text(&out.stderr);
    assert!(said.contains("Can't read b.txt"), "{said}");
    assert!(
        said.contains("hint: Let Opalro read it, or list it in .opalroignore."),
        "{said}"
    );
    assert!(!said.contains("save"), "{said}");
}

/// Every file in `.opalro` but the caches and temporary files, with its contents.
fn store_files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let opalro = root.join(".opalro");
    let mut found = BTreeMap::new();
    let mut todo = vec![opalro.clone()];
    while let Some(dir) = todo.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let rel = path
                .strip_prefix(&opalro)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if rel == "cache" || rel == "tmp" {
                continue;
            }
            if path.is_dir() {
                todo.push(path);
            } else {
                found.insert(rel, fs::read(&path).unwrap());
            }
        }
    }
    found
}

#[test]
fn status_never_writes_to_the_store() {
    let env = project(&[("a.txt", "a"), ("sub/b.txt", "b")]);
    fs::write(env.root().join("a.txt"), "changed").unwrap();
    fs::write(env.root().join("sub/new.txt"), "new").unwrap();
    fs::remove_file(env.root().join("sub/b.txt")).unwrap();
    let before = store_files(env.root());
    for args in [&[][..], &["--full"]] {
        assert_eq!(changes(&status_json(&env, args)).len(), 3);
    }
    assert_eq!(store_files(env.root()), before);
    // The file-state cache is all it writes.
    assert!(env.root().join(".opalro/cache/files").is_file());
}

#[test]
fn an_unchanged_status_leaves_the_file_cache_untouched() {
    // Files an hour old, so every entry is trusted and none turns racy.
    let env = project(&[("a.txt", "a"), ("sub/b.txt", "b")]);
    let cache = env.root().join(".opalro/cache/files");
    let look = || {
        let meta = fs::metadata(&cache).unwrap();
        #[cfg(unix)]
        let inode = std::os::unix::fs::MetadataExt::ino(&meta);
        #[cfg(not(unix))]
        let inode = 0;
        (
            meta.modified().unwrap(),
            meta.len(),
            inode,
            fs::read(&cache).unwrap(),
        )
    };
    assert!(changes(&status_json(&env, &[])).is_empty());
    let before = look();
    assert!(changes(&status_json(&env, &[])).is_empty());
    assert_eq!(look(), before);
}

#[test]
fn a_cache_that_cant_be_written_is_a_warning() {
    let env = project(&[("a.txt", "a")]);
    // The save wrote the cache; a folder takes its place.
    fs::remove_file(env.root().join(".opalro/cache/files")).unwrap();
    fs::create_dir_all(env.root().join(".opalro/cache/files/in-the-way")).unwrap();
    let doc = status_json(&env, &[]);
    let kinds: Vec<&str> = doc["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["file_cache_not_stored"], "{doc}");
    let said = doc["warnings"][0]["message"].as_str().unwrap();
    assert!(
        said.starts_with(
            "Opalro couldn't keep what it learned about your files, so the next status or save \
             reads them all again: "
        ),
        "{said}"
    );
    assert!(changes(&doc).is_empty());
}

#[test]
fn ignored_files_and_nested_projects_are_not_listed() {
    let env = project(&[
        (
            ".opalroignore",
            "*.log
",
        ),
        ("a.txt", "a"),
    ]);
    fs::write(env.root().join("x.log"), "noise").unwrap();
    fs::create_dir_all(env.root().join("vendor/.git")).unwrap();
    fs::write(env.root().join("vendor/v.txt"), "v").unwrap();
    let doc = status_json(&env, &[]);
    assert!(changes(&doc).is_empty(), "{doc}");
    let kinds: Vec<&str> = doc["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["nested_projects_left_out"], "{doc}");
    assert!(
        doc["warnings"][0]["message"]
            .as_str()
            .unwrap()
            .contains("vendor")
    );
}

#[test]
fn a_file_changed_too_close_to_the_cache_is_read_again() {
    let env = project(&[("a.txt", "aaa")]);
    // A time the cache can't trust whenever it is written: past the drive's "now".
    let a = env.root().join("a.txt");
    let soon = SystemTime::now() + Duration::from_secs(600);
    set_time(&a, soon);
    assert!(changes(&status_json(&env, &[])).is_empty());
    // Same size, same time: only reading the file tells.
    fs::write(&a, "bbb").unwrap();
    set_time(&a, soon);
    let doc = status_json(&env, &[]);
    assert_eq!(
        changes(&doc),
        [change("a.txt", "modified", "file")],
        "{doc}"
    );
}

/// A save whose top folder holds `a.txt`, the project link `lib`, which no computer holds as it
/// is, and the folder `deps` holding another and `x.txt`.
fn plant_links(root: &Path) {
    plant_save(root, |pack| {
        let a = pack.add_content(b"a").unwrap();
        let link = ProjectLink::submodule(&[1; 20]);
        let link_id = pack
            .add_record(ObjectKind::ProjectLink, &link.encode())
            .unwrap();
        let entry = |name: &str, kind, id, size| Entry {
            name: name.into(),
            kind,
            id,
            size,
        };
        let deps = Listing::new(vec![
            entry("lib", EntryKind::ProjectLink, link_id, 0),
            entry("x.txt", EntryKind::File, a, 1),
        ])
        .unwrap();
        let deps_id = pack
            .add_record(ObjectKind::FolderListing, &deps.encode())
            .unwrap();
        let top = Listing::new(vec![
            entry("a.txt", EntryKind::File, a, 1),
            entry("deps", EntryKind::Folder, deps_id, 0),
            entry("lib", EntryKind::ProjectLink, link_id, 0),
        ])
        .unwrap();
        pack.add_record(ObjectKind::FolderListing, &top.encode())
            .unwrap()
    });
}

#[test]
fn entries_this_computer_cant_hold_are_listed_as_not_written() {
    let env = Env::empty();
    env.opalro()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    write_old(&env.root().join("a.txt"), b"a");
    plant_links(env.root());
    // `deps` is gone from disk, and its link stays where the save has it.
    let doc = status_json(&env, &[]);
    assert_eq!(
        changes(&doc),
        [change("deps/x.txt", "removed", "file")],
        "{doc}"
    );
    assert_eq!(doc["not_written"], serde_json::json!(["deps/lib", "lib"]));
    let said = text(&env.opalro().arg("status").output().unwrap().stdout);
    assert!(
        said.ends_with(
            "Not written here, because this computer can't hold them as they are (saves keep \
             them as they were):\n\n    deps/lib\n    lib\n"
        ),
        "{said}"
    );
    // With the file back, nothing changed, and both stay listed.
    write_old(&env.root().join("deps/x.txt"), b"a");
    let doc = status_json(&env, &[]);
    assert!(changes(&doc).is_empty(), "{doc}");
    assert_eq!(doc["not_written"], serde_json::json!(["deps/lib", "lib"]));
    let said = text(&env.opalro().arg("status").output().unwrap().stdout);
    assert!(said.contains("Nothing changed since "), "{said}");
}

#[test]
fn status_says_in_its_own_words_why_it_stops() {
    let env = Env::empty();
    env.opalro()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    write_old(&env.root().join("a.txt"), b"a");
    plant_links(env.root());
    // The save keeps deps/lib as it is, so deps can't be a file.
    write_old(&env.root().join("deps"), b"a file");
    let out = env.opalro().arg("status").output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    assert_eq!(
        text(&out.stderr),
        "error: Opalro can't show the status: deps: the last save has a folder there holding \
         deps/lib, which every save keeps as it was because this computer can't hold it as it \
         is, and a save can't hold both under one name.\n\nhint: Rename or move deps.\n"
    );
}

#[test]
fn a_save_holding_a_spelling_of_opal_is_named() {
    let env = Env::empty();
    env.opalro()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    // No listing Opalro makes holds `.OPALRO`, so it is spelled `-OPALRO`, then the bytes are changed.
    plant_save(env.root(), |pack| {
        let id = pack.add_content(b"x").unwrap();
        let top = Listing::new(vec![Entry {
            name: "-OPALRO".into(),
            kind: EntryKind::File,
            id,
            size: 1,
        }])
        .unwrap()
        .encode();
        let at = top.windows(7).position(|w| w == b"-OPALRO").unwrap();
        let mut hostile = top.clone();
        hostile[at] = b'.';
        pack.add_record(ObjectKind::FolderListing, &hostile)
            .unwrap()
    });
    let planted = env.state().1.lines["main"];
    let out = env.opalro().arg("status").output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    assert_eq!(
        text(&out.stderr),
        format!(
            "error: Save {} holds a name spelled like Opalro's own folder (.opalro), which Opalro never \
             writes, so it can't be used.\n\nhint: Opalro never makes such a save, so another program \
             put it there. Nothing was changed: ask for help before trying to repair it.\n",
            planted.short_hex()
        )
    );
}

#[test]
fn status_json_matches_its_snapshot() {
    let env = project(&[("a.txt", "a"), ("b.txt", "b")]);
    fs::write(env.root().join("a.txt"), "changed").unwrap();
    fs::remove_file(env.root().join("b.txt")).unwrap();
    fs::write(env.root().join("c.txt"), "c").unwrap();
    fs::create_dir(env.root().join("d")).unwrap();
    let mut doc = status_json(&env, &[]);
    let redact = |v: &mut serde_json::Value| *v = "[redacted]".into();
    let save = &mut doc["save"];
    for key in ["id", "change_id", "change_id_short"] {
        redact(&mut save[key]);
    }
    for parent in save["parents"].as_array_mut().unwrap() {
        redact(parent);
    }
    for person in ["author", "saver"] {
        for key in ["time_ms", "tz_offset_min"] {
            redact(&mut save[person][key]);
        }
    }
    insta::assert_json_snapshot!(doc);
}

/// Each entry of a document's list `key` as `(path, change)`.
fn listed(doc: &serde_json::Value, key: &str) -> Vec<(String, String)> {
    let s = |v: &serde_json::Value| v.as_str().unwrap().to_owned();
    (doc[key].as_array().unwrap().iter())
        .map(|c| (s(&c["path"]), s(&c["change"])))
        .collect()
}

fn pair(path: &str, change: &str) -> (String, String) {
    (path.into(), change.into())
}

#[test]
fn status_shows_an_empty_folder_added_and_removed() {
    let env = project(&[("a.txt", "a")]);
    fs::create_dir(env.root().join("old")).unwrap();
    env.opalro().args(["save", "Old"]).assert().success();
    let status = || {
        let out = env.opalro().arg("status").output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        text(&out.stdout)
    };
    // Only a folder changed, so the folder is counted.
    fs::create_dir_all(env.root().join("new/deep")).unwrap();
    let said = status();
    assert!(
        said.ends_with("\nChanged since then (1 folder):\n\n    added  new/deep  empty folder\n"),
        "{said}"
    );
    #[cfg(feature = "fault-injection")]
    {
        let out = (env.opalro())
            .env("OPALRO_FAKE_TERMINAL", "80")
            .env("NO_COLOR", "1")
            .arg("status")
            .output()
            .unwrap();
        let next = text(&out.stderr);
        assert!(
            next.contains("Next: save them with: opalro save \"describe your change\""),
            "{next}"
        );
    }
    // With a file changed too, only the file is counted, and both rows are listed.
    fs::remove_dir_all(env.root().join("new")).unwrap();
    fs::remove_dir(env.root().join("old")).unwrap();
    fs::write(env.root().join("b.txt"), "bb").unwrap();
    let said = status();
    assert!(
        said.ends_with(
            "\nChanged since then (1 file):\n\n    added    b.txt           2 B\n    removed  old    \
             empty folder\n"
        ),
        "{said}"
    );
}

/// Where a file and an empty folder share a path, the row of the one that went comes first,
/// whichever it is.
#[test]
fn a_file_and_an_empty_folder_at_one_path_list_what_went_first() {
    let env = project(&[("a.txt", "a"), ("p", "pp")]);
    fs::create_dir(env.root().join("q")).unwrap();
    env.opalro().args(["save", "Both"]).assert().success();
    fs::remove_file(env.root().join("p")).unwrap();
    fs::create_dir(env.root().join("p")).unwrap();
    fs::remove_dir(env.root().join("q")).unwrap();
    fs::write(env.root().join("q"), "qqq").unwrap();
    let out = env.opalro().arg("status").output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let said = text(&out.stdout);
    assert!(
        said.ends_with(
            "\nChanged since then (2 files):\n\n    removed  p           2 B\n    added    p  \
             empty folder\n    removed  q  empty folder\n    added    q           3 B\n"
        ),
        "{said}"
    );
}

#[test]
fn status_json_lists_empty_folders() {
    let env = project(&[("a.txt", "a")]);
    fs::create_dir(env.root().join("old")).unwrap();
    env.opalro().args(["save", "Old"]).assert().success();
    let doc = status_json(&env, &[]);
    assert_eq!(doc["empty_folders"], serde_json::json!([]), "{doc}");
    fs::remove_dir(env.root().join("old")).unwrap();
    fs::create_dir_all(env.root().join("new/deep")).unwrap();
    fs::write(env.root().join("a.txt"), "changed").unwrap();
    let doc = status_json(&env, &[]);
    assert_eq!(
        doc["empty_folders"],
        serde_json::json!([
            {"path": "new/deep", "change": "added"},
            {"path": "old", "change": "removed"},
        ]),
        "{doc}"
    );
    // The changes are still files only.
    assert_eq!(
        changes(&doc),
        [change("a.txt", "modified", "file")],
        "{doc}"
    );
}

#[test]
fn status_says_nothing_changed_only_without_folders_either() {
    let env = project(&[("a.txt", "a")]);
    let said = || text(&env.opalro().arg("status").output().unwrap().stdout);
    let nothing = "Nothing changed since then.";
    assert!(said().contains(nothing));
    fs::create_dir(env.root().join("new")).unwrap();
    let now = said();
    assert!(!now.contains(nothing), "{now}");
    // Saved, it is the save's; deleted again, it is a change again.
    env.opalro().args(["save", "New folder"]).assert().success();
    assert!(said().contains(nothing));
    fs::remove_dir(env.root().join("new")).unwrap();
    let now = said();
    assert!(!now.contains(nothing), "{now}");
}

/// The folders among a save's paths, as `saved_paths` gives them, with nothing below them.
fn empty_in(paths: &BTreeMap<String, (char, Id)>) -> Vec<String> {
    let below = |p: &str| paths.keys().any(|q| q.starts_with(&format!("{p}/")));
    (paths.iter())
        .filter(|(p, (kind, _))| *kind == 'd' && !below(p))
        .map(|(p, _)| p.clone())
        .collect()
}

#[test]
fn what_status_lists_is_what_save_records() {
    let env = project(&[
        (".opalroignore", "*.log\n"),
        ("full/f.txt", "f"),
        ("x", "x"),
    ]);
    let root = env.root();
    for folder in ["gone", "y"] {
        fs::create_dir(root.join(folder)).unwrap();
    }
    env.opalro().args(["save", "Folders"]).assert().success();
    let before = main_tip(&env);
    // A folder of only ignored files, one emptied of its files, a saved empty folder deleted, a
    // new one, and an empty folder and a file each taking the other's place.
    write_old(&root.join("logs/today.log"), b"noise");
    fs::remove_file(root.join("full/f.txt")).unwrap();
    fs::remove_dir(root.join("gone")).unwrap();
    fs::create_dir_all(root.join("new/deep")).unwrap();
    fs::remove_file(root.join("x")).unwrap();
    fs::create_dir(root.join("x")).unwrap();
    fs::remove_dir(root.join("y")).unwrap();
    fs::write(root.join("y"), "y").unwrap();
    let status = status_json(&env, &[]);
    let files = vec![
        pair("full/f.txt", "removed"),
        pair("x", "removed"),
        pair("y", "added"),
    ];
    let folders = vec![
        pair("gone", "removed"),
        pair("logs", "added"),
        pair("new/deep", "added"),
        pair("x", "added"),
        pair("y", "removed"),
    ];
    assert_eq!(listed(&status, "changes"), files, "{status}");
    assert_eq!(listed(&status, "empty_folders"), folders, "{status}");
    env.opalro().args(["save", "Everything"]).assert().success();
    // The files are what the save changed.
    let out = env.opalro().args(["--json", "show"]).output().unwrap();
    let show: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(listed(&show, "changes"), files, "{show}");
    // The empty folders are those one side holds empty where the other holds no folder.
    let (old, new) = (saved_paths(&env, before), saved_paths(&env, main_tip(&env)));
    let folder = |paths: &BTreeMap<String, (char, Id)>, p: &str| {
        paths.get(p).is_some_and(|(kind, _)| *kind == 'd')
    };
    let mut recorded: Vec<(String, String)> = (empty_in(&new).into_iter())
        .filter(|p| !folder(&old, p))
        .map(|p| (p, "added".to_owned()))
        .chain(
            (empty_in(&old).into_iter())
                .filter(|p| !folder(&new, p))
                .map(|p| (p, "removed".to_owned())),
        )
        .collect();
    recorded.sort();
    assert_eq!(recorded, folders);
    // And the folder is now as the save holds it.
    let doc = status_json(&env, &[]);
    assert!(listed(&doc, "changes").is_empty(), "{doc}");
    assert!(listed(&doc, "empty_folders").is_empty(), "{doc}");
    let said = text(&env.opalro().arg("status").output().unwrap().stdout);
    assert!(said.contains("Nothing changed since then."), "{said}");
}

#[test]
fn a_reader_that_goes_away_ends_it_quietly() {
    use std::io::Read;
    use std::process::Stdio;
    let env = project(&[]);
    // More than any pipe holds, so status is still printing when the reader leaves.
    for i in 0..4000 {
        fs::write(env.root().join(format!("{i:0>60}.txt")), "x").unwrap();
    }
    let mut child = env
        .opal_process_in(env.root())
        .arg("status")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut first = [0; 16];
    stdout.read_exact(&mut first).unwrap();
    drop(stdout);
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(text(&out.stderr), "");
}
