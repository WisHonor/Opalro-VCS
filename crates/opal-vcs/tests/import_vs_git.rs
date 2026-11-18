//! Differential tests (decision 002): Git repositories made with Git itself, imported, then every
//! commit, tree, file and ref compared with what Git holds. A real repository's turn comes
//! nightly.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;
use std::time::Instant;

use opal_core::id::Id;
use opal_core::import::{ImportRequest, import_git};
use opal_core::names::holdable_here;
use opal_core::repo::Repository;
use opal_core::save::Person;
use proptest::prelude::*;
use proptest::sample::Index;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};
use support::{
    Env, assert_ancestry_matches_git, assert_matches_git, assert_matches_git_counting,
    assert_starts_match_git, copy_dir, git, git_commit, git_with_input, text, top_folder,
};

/// Writes `bytes` into the repository as a blob, and returns its ID.
fn blob(env: &Env, bytes: &[u8]) -> String {
    env.git_input(&["hash-object", "-w", "--stdin"], bytes)
}

#[test]
fn everyday_history_matches_git() {
    let env = Env::new();
    // As on Windows, Git takes no `x` bits from the files, so the mode the index is given below
    // stays in every later commit, on every system.
    env.git(&["config", "core.fileMode", "false"]);
    env.commit("a.txt", "a\n", "First");
    env.commit("src/lib/util.rs", "fn util() {}\n", "Add folders");
    env.overwrite("run.sh", "#!/bin/sh\necho hi\n");
    env.git(&["add", "run.sh"]);
    // Executable as Git marks it, whatever this system does with files.
    env.git(&["update-index", "--chmod=+x", "run.sh"]);
    env.git(&["commit", "-q", "-m", "Add a script"]);
    env.commit("caf\u{e9}/na\u{ef}ve.txt", "\u{e9}\n", "Unicode names");
    env.commit("\u{65e5}\u{672c}.txt", "\u{65e5}\n", "And more");
    env.git(&["mv", "a.txt", "b.txt"]);
    env.git(&["rm", "-q", "src/lib/util.rs"]);
    env.git(&["commit", "-q", "-m", "Rename and delete"]);
    env.git(&["commit", "-q", "--allow-empty", "-m", "Nothing changed"]);

    // A merge, then an octopus merge of two branches at once.
    let base = env.git(&["rev-parse", "HEAD"]);
    for branch in ["one", "two", "three"] {
        env.git(&["checkout", "-q", "-b", branch, &base]);
        env.commit(&format!("{branch}.txt"), "x\n", branch);
    }
    env.git(&["checkout", "-q", "main"]);
    env.git(&["merge", "-q", "--no-ff", "-m", "Merge one", "one"]);
    env.git(&["merge", "-q", "-m", "Merge two and three", "two", "three"]);

    // Entries only the index holds, which no checkout here could make: a link, two names only
    // letter case tells apart, a submodule, and names Opalro leaves out (a `.opalro` folder below the
    // top, and a name Windows takes for `.git`).
    let target = blob(&env, b"b.txt");
    let (upper, lower) = (blob(&env, b"Read me\n"), blob(&env, b"Or me\n"));
    for entry in [
        format!("120000,{target},link"),
        format!("100644,{upper},README"),
        format!("100644,{lower},Readme"),
        format!("160000,{},vendor/lib", "5a".repeat(20)),
        format!("100644,{upper},sub/kept.txt"),
        format!("100644,{upper},sub/.opalro/config"),
        format!("100644,{upper},sub/git~1"),
    ] {
        // Git refuses `git~1` unless told not to guard NTFS.
        let args = ["-c", "core.protectNTFS=false", "update-index", "--add"];
        env.git(&[&args[..], &["--cacheinfo", &entry]].concat());
    }
    env.git(&[
        "commit",
        "-q",
        "-m",
        "A link, twins, a submodule and names Opalro skips",
    ]);

    let head = env.git(&["rev-parse", "HEAD"]);
    // Commits only a server branch, or only a tag, reaches.
    let lone = |message| env.git(&["commit-tree", "HEAD^{tree}", "-p", &base, "-m", message]);
    let (fetched, tagged) = (lone("Only on the server"), lone("Only tagged"));
    env.git(&["branch", "feature+x", &base]);
    env.git(&["branch", "topic/deep/name", "one"]);
    env.git(&["branch", "caf\u{e9}", "two"]);
    env.git(&["update-ref", "refs/remotes/origin/main", &head]);
    env.git(&["update-ref", "refs/remotes/origin/fix+1", &base]);
    env.git(&["update-ref", "refs/remotes/upstream/topic/x", &fetched]);
    env.git(&["tag", "-a", "v2", "-m", "Version 2", &tagged]);
    env.git(&[
        "symbolic-ref",
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/main",
    ]);
    env.git(&["tag", "light", &base]);
    // A tag keeps characters a line's name can't hold.
    env.git(&["tag", "v1+build", &base]);
    env.git(&["tag", "-a", "v1", "-m", "Version 1", &head]);
    env.git(&["tag", "-a", "nested", "-m", "Again", "v1"]);
    env.git(&["tag", "folder", "HEAD^{tree}"]);
    env.git(&["tag", "-a", "folder-note", "-m", "A folder", "HEAD^{tree}"]);
    env.git(&["tag", "file", "HEAD:b.txt"]);
    // Two branches alike but for letter case, which only packed-refs can hold where the drive
    // ignores case.
    let packed = env.root().join(".git/packed-refs");
    assert!(!packed.exists());
    let twins = format!(
        "# pack-refs with: peeled fully-peeled sorted \n{base} refs/heads/Release\n{base} \
         refs/heads/release\n"
    );
    std::fs::write(packed, twins).unwrap();

    let detached = tempfile::tempdir().unwrap();
    copy_dir(env.root(), detached.path());
    env.import();
    // Git wrote every commit itself, so each save rebuilds its commit from its own fields.
    assert_eq!(assert_matches_git(env.root()), 0);
    // Nine lines and three server lines across the merges: held ones, ones that hold some, and
    // ones parted at a fork.
    assert_eq!(assert_ancestry_matches_git(env.root()), (64, true, 0));
    assert_eq!(assert_starts_match_git(env.root()), 64);
    // The renaming rule, pinned: `+` becomes `-`, and of names alike but for letter case, the
    // later one in byte order takes a suffix.
    let (repo, state) = env.state();
    let lines: Vec<&str> = state.lines.keys().map(String::as_str).collect();
    let want = [
        "Release",
        "caf\u{e9}",
        "feature-x",
        "main",
        "one",
        "release-2",
        "three",
        "topic/deep/name",
        "two",
    ];
    assert_eq!(lines, want);
    let servers = repo.server_lines(&state).unwrap().servers;
    let origin: Vec<&str> = servers["origin"].keys().map(String::as_str).collect();
    assert_eq!(origin, ["fix-1", "main"]);

    // The same, with HEAD detached at a commit only it reaches.
    git(detached.path(), &["checkout", "-q", "--detach"]);
    git(
        detached.path(),
        &["commit", "-q", "--allow-empty", "-m", "Only HEAD has this"],
    );
    env.opal_in(detached.path())
        .arg("import")
        .assert()
        .success();
    assert_eq!(assert_matches_git(detached.path()), 0);
    assert_eq!(assert_ancestry_matches_git(detached.path()), (64, true, 0));
    assert_eq!(assert_starts_match_git(detached.path()), 64);
}

#[test]
fn unusual_commits_match_git() {
    let env = Env::new();
    let a = blob(&env, b"a\n");
    let tree = env.git_input(&["mktree"], format!("100644 blob {a}\ta.txt\n").as_bytes());
    let write = |parents: &[&str], rest: &[u8]| {
        let mut raw = format!("tree {tree}\n");
        for parent in parents {
            let _ = writeln!(raw, "parent {parent}");
        }
        let raw = [raw.as_bytes(), rest].concat();
        let args = [
            "hash-object",
            "-t",
            "commit",
            "--literally",
            "-w",
            "--stdin",
        ];
        env.git_input(&args, &raw)
    };
    let people = "author Ida Lark <ida.lark@example.com> 1601234567 -0400\n\
                  committer Omar Fenn <omar.fenn@example.com> 1601234627 +0545\n";
    let side = write(&[], format!("{people}\nA side commit\n").as_bytes());
    let mut last = write(&[], format!("{people}\nPlain\n").as_bytes());
    let tag = format!(
        "mergetag object {side}\n type commit\n tag v1.0\n \
         tagger Vera Toll <vera.toll@example.com> 1601234500 +0000\n \n Version 1.0\n"
    );
    for (others, rest) in [
        // A signature, its lines continued with a space.
        (
            None,
            format!(
                "{people}gpgsig -----BEGIN PGP SIGNATURE-----\n \n iHUEABYKAB0WIQRNmrJ5\n \
                 =Xd3h\n -----END PGP SIGNATURE-----\n\nSigned\n"
            )
            .into_bytes(),
        ),
        // A merge that carries the tag it merged.
        (
            Some(side.as_str()),
            format!("{people}{tag}\nMerge tag v1.0\n").into_bytes(),
        ),
        // A message in Latin-1, as it says, and one in no encoding at all.
        (
            None,
            [
                format!("{people}encoding ISO-8859-1\n\n").as_bytes(),
                b"Caf\xe9\n",
            ]
            .concat(),
        ),
        (None, [people.as_bytes(), b"\nBad \xff byte\n"].concat()),
        // Jujutsu's change ID.
        (
            None,
            format!("{people}change-id zyxwvutsrqponmlkzyxwvutsrqponmlk\n\nChanged\n").into_bytes(),
        ),
        // A zone written -0000.
        (
            None,
            b"author Ida Lark <ida.lark@example.com> 1601234567 -0000\n\
              committer Omar Fenn <omar.fenn@example.com> 1601234627 -0000\n\nZone\n"
                .to_vec(),
        ),
        // An author line no version of Git writes.
        (
            None,
            b"author Ida Lark <ida.lark@example.com>1601234567 -0400\n\
              committer Omar Fenn <omar.fenn@example.com> 1601234627 +0545\n\nOdd\n"
                .to_vec(),
        ),
        // A header Git doesn't know, continued on a second line.
        (
            None,
            format!("{people}x-unknown some value\n more of it\n\nUnknown\n").into_bytes(),
        ),
        // No newline at the end.
        (None, format!("{people}\nNo newline").into_bytes()),
    ] {
        let parents: Vec<&str> = [Some(last.as_str()), others]
            .into_iter()
            .flatten()
            .collect();
        last = write(&parents, &rest);
    }
    // A parent named twice.
    last = write(&[&last, &last], format!("{people}\nTwice\n").as_bytes());
    env.git(&["update-ref", "refs/heads/main", &last]);
    env.import();
    let counted = assert_matches_git_counting(env.root());
    assert_eq!(assert_ancestry_matches_git(env.root()), (0, true, 0));
    assert_eq!(assert_starts_match_git(env.root()), 0);
    // All but the plain two, the one in no encoding and the one without a newline keep their
    // Git header.
    assert_eq!(counted.headers, 8);
    // The odd author line is the only person line not compared with Git's, so a parser that
    // gave up on more of them would show here.
    assert_eq!(counted.odd_people, 1);
}

#[test]
fn packed_and_loose_objects_match_git() {
    let env = Env::new();
    // Sixty versions of one growing file, so Git packs most as deltas.
    let mut stream = String::new();
    let mut file = String::new();
    for n in 0..60 {
        for line in 0..40 {
            let _ = writeln!(file, "Version {n}, line {line}: and the same words again");
        }
        let message = format!("Version {n}\n");
        let _ = write!(
            stream,
            "commit refs/heads/main\ncommitter Ada <ada@example.com> {} +0000\ndata {}\n{message}\
             M 100644 inline big.txt\ndata {}\n{file}\n",
            1_700_000_000 + n,
            message.len(),
            file.len()
        );
    }
    let made = git_with_input(env.root(), &["fast-import", "--quiet"], stream.as_bytes());
    assert!(made.status.success(), "{}", text(&made.stderr));
    env.git(&["repack", "-adfq", "--depth=50"]);
    env.git(&["reset", "-q", "--hard"]);
    // Then some objects loose beside the pack, and every ref packed.
    env.commit("loose.txt", "loose\n", "Loose");
    env.commit("big.txt", &format!("{file}one more line\n"), "Loose too");
    env.git(&["tag", "-a", "v1", "-m", "Version 1"]);
    env.git(&["branch", "old", "HEAD~30"]);
    env.git(&["pack-refs", "--all"]);
    let counts = env.git(&["count-objects", "-v"]);
    assert!(
        counts.contains("packs: 1") && !counts.contains("count: 0"),
        "{counts}"
    );
    env.import();
    assert_eq!(assert_matches_git(env.root()), 0);
    assert_eq!(assert_ancestry_matches_git(env.root()), (2, true, 0));
    assert_eq!(assert_starts_match_git(env.root()), 2);
}

#[test]
fn a_history_dated_backwards_matches_git() {
    let env = Env::new();
    // The third commit on main is dated before its parent, as Git allows. Apart from main, X, then
    // S dated before it, then a at a merge of S and X, and b after S.
    let mut stream = String::new();
    for (mark, (branch, from, merge, time)) in [
        ("main", None, None, 1_700_002_000),
        ("main", None, None, 1_700_003_000),
        ("main", None, None, 1_700_001_000),
        ("main", None, None, 1_700_004_000),
        ("side", Some(2), None, 1_700_003_500),
        ("a", None, None, 1_600_000_050),
        ("b", Some(6), None, 1_600_000_010),
        ("a", Some(7), Some(6), 1_600_000_100),
        ("b", None, None, 1_600_000_090),
    ]
    .into_iter()
    .enumerate()
    {
        let _ = write!(
            stream,
            "commit refs/heads/{branch}\nmark :{}\ncommitter Ada <ada@example.com> {time} +0000\n\
             data 2\nc{}\n",
            mark + 1,
            mark + 1
        );
        if let Some(from) = from {
            let _ = writeln!(stream, "from :{from}");
        }
        if let Some(merge) = merge {
            let _ = writeln!(stream, "merge :{merge}");
        }
    }
    // A line main holds.
    stream.push_str("reset refs/heads/old\nfrom :1\n\n");
    let made = git_with_input(env.root(), &["fast-import", "--quiet"], stream.as_bytes());
    assert!(made.status.success(), "{}", text(&made.stderr));
    env.git(&["reset", "-q", "--hard"]);
    env.import();
    assert_eq!(assert_matches_git(env.root()), 0);
    // old is held, main and side each hold some, and main's walk from side crosses the commit
    // dated back. From a against b the walk reads X before S and counts it too (2 where Git
    // counts 1): the one count that comes out high.
    let (pairs, exact, higher) = assert_ancestry_matches_git(env.root());
    assert_eq!((pairs, exact, higher), (20, false, 1));
    assert_eq!(assert_starts_match_git(env.root()), 20);
}

#[test]
fn a_criss_cross_matches_git() {
    let env = Env::new();
    env.commit("a.txt", "a\n", "Base");
    env.git(&["branch", "left"]);
    env.git(&["branch", "right"]);
    env.git(&["checkout", "-q", "left"]);
    env.commit("l.txt", "l\n", "Left");
    env.git(&["checkout", "-q", "right"]);
    env.commit("r.txt", "r\n", "Right");
    // Each takes the other's first commit, then goes on.
    env.git(&["merge", "-q", "--no-edit", "left"]);
    env.git(&["checkout", "-q", "left"]);
    env.git(&["merge", "-q", "--no-edit", "right~1"]);
    env.commit("l.txt", "l2\n", "Left again");
    env.git(&["checkout", "-q", "right"]);
    env.commit("r.txt", "r2\n", "Right again");
    let bases = env.git(&["merge-base", "--all", "left", "right"]);
    assert_eq!(bases.lines().count(), 2, "{bases}");
    env.import();
    assert_eq!(assert_matches_git(env.root()), 0);
    assert_eq!(assert_ancestry_matches_git(env.root()), (6, true, 0));
    assert_eq!(assert_starts_match_git(env.root()), 6);
}

#[test]
fn sha256_repositories_match_git() {
    let version = text(&git(&std::env::temp_dir(), &["version"]));
    let mut parts = version
        .split_whitespace()
        .nth(2)
        .unwrap_or_default()
        .split('.');
    let mut number = || match parts.next().map(str::parse::<u32>) {
        Some(Ok(n)) => n,
        _ => panic!("can't read Git's version in {version:?}"),
    };
    if (number(), number()) < (2, 29) {
        println!(
            "Skipped: {} is older than 2.29, the first to make SHA-256 repositories.",
            version.trim()
        );
        return;
    }
    let env = Env::init(&["--object-format=sha256"]);
    env.commit("a.txt", "a\n", "First");
    env.commit("src/b.txt", "b\n", "Second");
    env.git(&["branch", "feature+x", "HEAD~1"]);
    env.git(&["repack", "-adq"]);
    // Then loose objects: a script, a link and a submodule, whose IDs are 32 bytes too.
    env.overwrite("run.sh", "#!/bin/sh\n");
    env.git(&["add", "run.sh"]);
    env.git(&["update-index", "--chmod=+x", "run.sh"]);
    let target = blob(&env, b"a.txt");
    for entry in [
        format!("120000,{target},link"),
        format!("160000,{},vendor/lib", "5a".repeat(32)),
    ] {
        env.git(&["update-index", "--add", "--cacheinfo", &entry]);
    }
    env.git(&["commit", "-q", "-m", "A script, a link and a submodule"]);
    env.git(&["tag", "-a", "v1", "-m", "Version 1"]);
    env.git(&["update-ref", "refs/remotes/origin/main", "HEAD~1"]);
    env.import();
    assert_eq!(assert_matches_git(env.root()), 0);
    assert_eq!(assert_ancestry_matches_git(env.root()), (6, true, 0));
    assert_eq!(assert_starts_match_git(env.root()), 6);
}

#[test]
fn optional_git_files_are_ignored() {
    let env = Env::new();
    env.commit("a.txt", "a\n", "First");
    env.commit("b/c.txt", "c\n", "Second");
    env.git(&["branch", "side", "HEAD~1"]);
    env.git(&["tag", "-a", "v1", "-m", "Version 1"]);
    // One pack with a bitmap and a reverse index, a multi-pack index and a commit-graph over it,
    // only packed refs, and a worktree config that turns nothing on.
    env.git(&["-c", "pack.writeReverseIndex=true", "repack", "-adbq"]);
    env.git(&["multi-pack-index", "write"]);
    env.git(&["commit-graph", "write", "--reachable"]);
    env.git(&["pack-refs", "--all"]);
    env.git(&["config", "extensions.worktreeConfig", "true"]);
    env.git(&["config", "--worktree", "core.sparseCheckout", "false"]);
    let git_dir = env.root().join(".git");
    let packs: Vec<String> = std::fs::read_dir(git_dir.join("objects/pack"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    for made in [".bitmap", ".rev", "multi-pack-index"] {
        assert!(packs.iter().any(|f| f.ends_with(made)), "{made}: {packs:?}");
    }
    for made in [
        "objects/info/commit-graph",
        "config.worktree",
        "packed-refs",
    ] {
        assert!(git_dir.join(made).is_file(), "{made}");
    }
    assert!(!git_dir.join("refs/heads/main").exists());
    env.import();
    assert_eq!(assert_matches_git(env.root()), 0);
    assert_eq!(assert_ancestry_matches_git(env.root()), (2, true, 0));
    assert_eq!(assert_starts_match_git(env.root()), 2);
}

#[test]
fn windows_cant_hold_some_names_and_saving_keeps_them() {
    let env = Env::new();
    env.commit("a.txt", "a\n", "First");
    // A commit as Linux makes one. Git for Windows refuses these names in its index, so the tree
    // is written whole.
    let files = [
        ("README", "Read me\n"),
        ("Readme", "Or me\n"),
        ("a:b", "A colon\n"),
        ("aux.c", "int aux;\n"),
    ];
    let mut tree = env.git(&["ls-tree", "HEAD"]) + "\n";
    for (name, contents) in files {
        let _ = writeln!(
            tree,
            "100644 blob {}\t{name}",
            blob(&env, contents.as_bytes())
        );
    }
    let tree = env.git_input(&["mktree"], tree.as_bytes());
    let names = "Names Windows can't hold";
    let commit = env.git(&["commit-tree", &tree, "-p", "HEAD", "-m", names]);
    env.git(&["update-ref", "refs/heads/main", &commit]);
    env.import();
    assert_eq!(assert_matches_git(env.root()), 0);
    assert_eq!(assert_ancestry_matches_git(env.root()), (0, true, 0));
    assert_eq!(assert_starts_match_git(env.root()), 0);

    // What Git would write out here: each name this system can hold. Where letter case doesn't
    // count, the second of the twins is written over the first.
    for (name, contents) in files {
        if holdable_here(name) {
            env.overwrite(name, contents);
        }
    }
    env.overwrite("new.txt", "new\n");
    let imported = top_folder(env.root());
    let out = env.save(&["Add a file"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let saved = top_folder(env.root());
    for (name, _) in files {
        assert!(saved.get(name).is_some(), "{name}");
        assert_eq!(saved.get(name), imported.get(name), "{name}");
    }
    assert!(saved.get("new.txt").is_some());
}

/// Git writes the link out itself, as a real link wherever it can make one; the first save after
/// the import keeps the target Git stored, though Git for Windows writes its `/` as `\`.
#[test]
fn a_link_git_wrote_keeps_its_target_when_saved() {
    let env = Env::new();
    env.commit("dir/target.txt", "t\n", "First");
    let target = blob(&env, b"dir/target.txt");
    let entry = format!("120000,{target},link");
    env.git(&["update-index", "--add", "--cacheinfo", &entry]);
    env.git(&["commit", "-q", "-m", "A link"]);
    let args = ["-c", "core.symlinks=true", "checkout", "--", "link"];
    let out = git_with_input(env.root(), &args, b"");
    let link = std::fs::symlink_metadata(env.root().join("link"));
    if !link.is_ok_and(|meta| meta.file_type().is_symlink()) {
        // CI's runners can make links, so a skip there would pass for a test that never ran.
        assert!(
            std::env::var_os("CI").is_none(),
            "Git couldn't make the link in CI: {}",
            text(&out.stderr)
        );
        println!("Skipped: Git can't make links here. {}", text(&out.stderr));
        return;
    }
    env.import();
    env.overwrite("new.txt", "new\n");
    let out = env.save(&["Add a file"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let repo = Repository::open(env.root()).unwrap();
    let link = top_folder(env.root()).get("link").cloned().unwrap();
    let mut saved = Vec::new();
    repo.store().copy_content(&link.id, &mut saved).unwrap();
    assert_eq!(text(&saved), "dir/target.txt");
}

// Renames, against the exact ones Git finds.

/// The folders of a generated history, none named like a file. The last three hold no other, so
/// each can be moved whole.
const FOLDERS: [&str; 5] = ["", "src/", "src/deep/", "doc/", "lib/"];
const LEAVES: [&str; 3] = ["src/deep/", "doc/", "lib/"];
const NAMES: [&str; 6] = ["a.txt", "b.txt", "c.md", "d.sh", "e.png", "f.rs"];

fn all_paths() -> Vec<String> {
    FOLDERS
        .iter()
        .flat_map(|f| NAMES.iter().map(move |n| format!("{f}{n}")))
        .collect()
}

/// One change in a commit; each `Index` picks among what is there when the commit is made.
#[derive(Clone, Debug)]
enum Change {
    /// A file's contents replaced.
    Edit(Index),
    /// A new file at a free path, executable if `true`.
    Add(Index, bool),
    /// A new file at a free path holding what another holds.
    Copy(Index, Index),
    Remove(Index),
    /// A file moved to a free path: its mark flipped if the first `bool`, its contents replaced
    /// if the second.
    Move(Index, Index, bool, bool),
    /// Every file of a folder that holds no other, moved to another such folder holding nothing.
    MoveFolder(Index, Index),
}

/// The first commit's files (path, executable), `main`'s commits, then maybe a branch forked from
/// there, `main`'s commits after the fork, and a merge of the branch.
#[derive(Clone, Debug)]
struct Renames {
    first: Vec<(String, bool)>,
    main: Commits,
    side: Option<(Commits, Commits)>,
}

/// Commits, each of its changes.
type Commits = Vec<Vec<Change>>;

/// Each file's contents and whether it is executable.
type Tree = BTreeMap<String, (String, bool)>;

fn change() -> impl Strategy<Value = Change> {
    let flag = |p| prop::bool::weighted(p);
    prop_oneof![
        2 => any::<Index>().prop_map(Change::Edit),
        2 => (any::<Index>(), flag(0.3)).prop_map(|(at, x)| Change::Add(at, x)),
        1 => (any::<Index>(), any::<Index>()).prop_map(|(f, at)| Change::Copy(f, at)),
        1 => any::<Index>().prop_map(Change::Remove),
        4 => (any::<Index>(), any::<Index>(), flag(0.3), flag(0.2))
            .prop_map(|(f, t, flip, edit)| Change::Move(f, t, flip, edit)),
        2 => (any::<Index>(), any::<Index>()).prop_map(|(f, t)| Change::MoveFolder(f, t)),
    ]
}

fn commits(n: impl Into<prop::collection::SizeRange>) -> impl Strategy<Value = Commits> {
    prop::collection::vec(prop::collection::vec(change(), 1..=3), n)
}

fn renames() -> impl Strategy<Value = Renames> {
    let marks = prop::collection::vec(prop::bool::weighted(0.3), 14);
    let first = (prop::sample::subsequence(all_paths(), 4..=14), marks)
        .prop_map(|(paths, marks)| paths.into_iter().zip(marks).collect());
    let side = prop::option::weighted(0.4, (commits(1..=3), commits(0..=2)));
    (first, commits(2..=6), side).prop_map(|(first, main, side)| Renames { first, main, side })
}

/// Makes `change` on `tree`, giving new contents numbers from `fresh`; returns the files a folder
/// moved whole went from and to.
fn make(tree: &mut Tree, change: &Change, fresh: &mut usize) -> Vec<(String, String)> {
    let mut contents = || {
        *fresh += 1;
        format!("contents {fresh}\n")
    };
    let free: Vec<String> = (all_paths().into_iter())
        .filter(|p| !tree.contains_key(p))
        .collect();
    let free_at = |at: &Index| (!free.is_empty()).then(|| free[at.index(free.len())].clone());
    let nth = |tree: &Tree, at: &Index| {
        (!tree.is_empty()).then(|| tree.keys().nth(at.index(tree.len())).unwrap().clone())
    };
    match change {
        Change::Edit(at) => {
            if let Some(path) = nth(tree, at) {
                tree.get_mut(&path).unwrap().0 = contents();
            }
        }
        Change::Add(at, exec) => {
            if let Some(path) = free_at(at) {
                tree.insert(path, (contents(), *exec));
            }
        }
        Change::Copy(from, at) => {
            if let (Some(from), Some(path)) = (nth(tree, from), free_at(at)) {
                let file = tree[&from].clone();
                tree.insert(path, file);
            }
        }
        Change::Remove(at) => {
            if let Some(path) = nth(tree, at) {
                tree.remove(&path);
            }
        }
        Change::Move(from, to, flip, edit) => {
            if let (Some(from), Some(to)) = (nth(tree, from), free_at(to)) {
                let (mut bytes, mut exec) = tree.remove(&from).unwrap();
                exec ^= *flip;
                if *edit {
                    bytes = contents();
                }
                tree.insert(to, (bytes, exec));
            }
        }
        Change::MoveFolder(from, to) => {
            let holding = |leaf: &&str| tree.keys().any(|p| p.starts_with(*leaf));
            let full: Vec<&str> = LEAVES.iter().copied().filter(holding).collect();
            let empty: Vec<&str> = LEAVES.iter().copied().filter(|l| !holding(l)).collect();
            if full.is_empty() || empty.is_empty() {
                return Vec::new();
            }
            let (from, to) = (full[from.index(full.len())], empty[to.index(empty.len())]);
            let moving: Vec<String> = (tree.keys())
                .filter(|p| p.starts_with(from))
                .cloned()
                .collect();
            let mut moved = Vec::new();
            for path in moving {
                let file = tree.remove(&path).unwrap();
                let there = format!("{to}{}", &path[from.len()..]);
                tree.insert(there.clone(), file);
                moved.push((path, there));
            }
            return moved;
        }
    }
    Vec::new()
}

/// A generated history written for `git fast-import`, and the folders each commit moved whole
/// (as the files' moves), by the commit's message: its mark.
struct Written {
    stream: String,
    marks: u32,
    folders: BTreeMap<u32, Vec<Vec<(String, String)>>>,
}

impl Written {
    /// Writes a commit on `branch` with `parents` from what changed between `before` (the first
    /// parent's files) and `after`; returns its mark.
    fn commit(&mut self, branch: &str, parents: &[u32], before: &Tree, after: &Tree) -> u32 {
        self.marks += 1;
        let mark = self.marks;
        let message = format!("{mark}\n");
        let _ = write!(
            self.stream,
            "commit refs/heads/{branch}\nmark :{mark}\ncommitter Ada <ada@example.com> {} +0000\n\
             data {}\n{message}",
            1_700_000_000 + mark,
            message.len()
        );
        for (i, parent) in parents.iter().enumerate() {
            let how = if i == 0 { "from" } else { "merge" };
            let _ = writeln!(self.stream, "{how} :{parent}");
        }
        for path in before.keys().filter(|p| !after.contains_key(*p)) {
            let _ = writeln!(self.stream, "D {path}");
        }
        for (path, file) in after {
            if before.get(path) != Some(file) {
                let mode = if file.1 { "100755" } else { "100644" };
                let _ = write!(
                    self.stream,
                    "M {mode} inline {path}\ndata {}\n{}\n",
                    file.0.len(),
                    file.0
                );
            }
        }
        mark
    }

    /// Makes `changes` on `tree` and commits them on `branch` after `parent`; returns the mark.
    fn changes(
        &mut self,
        branch: &str,
        parent: u32,
        tree: &mut Tree,
        changes: &[Change],
        fresh: &mut usize,
    ) -> u32 {
        let before = tree.clone();
        let mut folders = Vec::new();
        for change in changes {
            let moved = make(tree, change, fresh);
            if !moved.is_empty() {
                folders.push(moved);
            }
        }
        let mark = self.commit(branch, &[parent], &before, tree);
        self.folders.insert(mark, folders);
        mark
    }
}

/// The history `h` written out. A merge holds the branch's files.
fn written(h: &Renames) -> Written {
    let mut fresh = 0;
    let first: Tree = (h.first.iter())
        .map(|(path, exec)| {
            fresh += 1;
            (path.clone(), (format!("contents {fresh}\n"), *exec))
        })
        .collect();
    let mut out = Written {
        stream: String::new(),
        marks: 0,
        folders: BTreeMap::new(),
    };
    let mut main_mark = out.commit("main", &[], &Tree::new(), &first);
    let mut main = first;
    for changes in &h.main {
        main_mark = out.changes("main", main_mark, &mut main, changes, &mut fresh);
    }
    if let Some((branch, after)) = &h.side {
        let (mut side, mut side_mark) = (main.clone(), main_mark);
        for changes in branch {
            side_mark = out.changes("side", side_mark, &mut side, changes, &mut fresh);
        }
        for changes in after {
            main_mark = out.changes("main", main_mark, &mut main, changes, &mut fresh);
        }
        out.commit("main", &[main_mark, side_mark], &main, &side);
    }
    out
}

/// Each file of the commit `commit` in `dir`: its mode and blob ID.
fn git_files(dir: &Path, commit: &str) -> BTreeMap<String, (String, String)> {
    let listed = git(dir, &["ls-tree", "-r", "-z", commit]);
    (listed.split(|&b| b == 0).filter(|r| !r.is_empty()))
        .map(|row| {
            let row = text(row);
            let (head, path) = row.split_once('\t').unwrap();
            let words: Vec<&str> = head.split(' ').collect();
            (path.to_owned(), (words[0].to_owned(), words[2].to_owned()))
        })
        .collect()
}

/// The renames `git diff --name-status -M100%` finds from `from` to `to` in `dir`, all `R100`.
fn exact_renames(dir: &Path, from: &str, to: &str) -> BTreeSet<(String, String)> {
    let listed = git(dir, &["diff", "--name-status", "-z", "-M100%", from, to]);
    let mut fields = (listed.split(|&b| b == 0).filter(|f| !f.is_empty())).map(text);
    let mut found = BTreeSet::new();
    while let Some(status) = fields.next() {
        let first = fields.next().unwrap();
        if status.starts_with('R') || status.starts_with('C') {
            assert_eq!(status, "R100", "{from} to {to}");
            found.insert((first, fields.next().unwrap()));
        }
    }
    found
}

/// Generated histories, made with Git and imported: each save notes exactly the renames
/// `git diff -M100%` finds between its commit and the commit's first parent, leaving out contents
/// that more than one file holds on either side (Git picks one pairing there, and Opalro notes
/// none, or only a file whose twin stayed where it was), and every note is one of those renames.
/// Commits edit, add, copy and remove files and move them, some with their executable mark
/// flipped or their contents replaced, move folders whole, and may merge a branch. The seed is
/// fixed.
#[test]
fn notes_match_gits_exact_renames() {
    const FLOORS: [&str; 3] = [
        "a folder moved whole",
        "a rename of an executable",
        "contents held twice",
    ];
    let histories = RefCell::new(BTreeMap::<&str, usize>::new());
    let compared = RefCell::new(0);
    let config = Config {
        cases: 32,
        failure_persistence: None,
        ..Config::default()
    };
    let rng = TestRng::deterministic_rng(RngAlgorithm::ChaCha);
    TestRunner::new_with_rng(config, rng)
        .run(&renames(), |h| {
            let made = written(&h);
            let env = Env::new();
            let args = ["fast-import", "--quiet"];
            let out = git_with_input(env.root(), &args, made.stream.as_bytes());
            assert!(out.status.success(), "{}", text(&out.stderr));
            env.git(&["reset", "-q", "--hard"]);
            env.import();
            let (repo, state) = env.state();
            let mut todo: Vec<Id> = state.lines.values().copied().collect();
            let (mut seen, mut met) = (BTreeSet::new(), BTreeSet::new());
            while let Some(id) = todo.pop() {
                if !seen.insert(id) {
                    continue;
                }
                let save = repo.store().save(&id, false).unwrap();
                todo.extend(save.parents.iter().copied());
                let Some(first) = save.parents.first() else {
                    continue;
                };
                let (commit, parent) = (git_commit(&repo, &id), git_commit(&repo, first));
                let ours: BTreeSet<(String, String)> = (save.move_notes.iter())
                    .map(|n| (n.from.clone(), n.to.clone()))
                    .collect();
                let theirs = exact_renames(env.root(), &parent, &commit);
                prop_assert!(
                    ours.is_subset(&theirs),
                    "{}: Opalro noted {:?}, Git renamed {:?}\n{}",
                    save.message,
                    ours,
                    theirs,
                    made.stream
                );
                let (before, after) = (
                    git_files(env.root(), &parent),
                    git_files(env.root(), &commit),
                );
                let held = |files: &BTreeMap<String, (String, String)>, blob: &String| {
                    files.values().filter(|(_, b)| b == blob).count()
                };
                let once = |(from, _): &&(String, String)| {
                    let blob = &before[from].1;
                    held(&before, blob) == 1 && held(&after, blob) == 1
                };
                let sure: BTreeSet<_> = theirs.iter().filter(once).cloned().collect();
                let noted: BTreeSet<_> = ours.iter().filter(once).cloned().collect();
                prop_assert_eq!(&noted, &sure, "{}\n{}", save.message, made.stream);
                *compared.borrow_mut() += sure.len();
                if sure.len() < theirs.len() {
                    met.insert(FLOORS[2]);
                }
                if sure.iter().any(|(from, _)| before[from].0 == "100755") {
                    met.insert(FLOORS[1]);
                }
                let mark: u32 = save.message.trim().parse().unwrap();
                let whole = |moved: &Vec<(String, String)>| {
                    moved.iter().filter(|m| sure.contains(*m)).count() >= 2
                };
                if made.folders.get(&mark).is_some_and(|f| f.iter().any(whole)) {
                    met.insert(FLOORS[0]);
                }
            }
            for kind in met {
                *histories.borrow_mut().entry(kind).or_default() += 1;
            }
            Ok(())
        })
        .unwrap();
    let (histories, compared) = (histories.into_inner(), compared.into_inner());
    println!("{compared} renames compared; histories with each: {histories:?}");
    assert!(compared > 0, "compared no rename");
    for kind in FLOORS {
        let n = histories.get(kind).copied().unwrap_or(0);
        assert!(n >= 3, "{kind} in {n} histories");
    }
}

#[test]
#[ignore = "nightly: needs OPALRO_DIFF_REPO, a full clone of a real repository"]
fn a_real_repository_matches_git() {
    let clone = std::env::var_os("OPALRO_DIFF_REPO").expect("OPALRO_DIFF_REPO names a clone");
    let shallow = text(&git(
        Path::new(&clone),
        &["rev-parse", "--is-shallow-repository"],
    ));
    assert_eq!(
        shallow.trim(),
        "false",
        "OPALRO_DIFF_REPO must be a full clone"
    );
    // Its .git copied, so the clone stays as it was and the test can run again.
    let work = tempfile::tempdir().unwrap();
    copy_dir(&Path::new(&clone).join(".git"), &work.path().join(".git"));
    let who = Person {
        name: "Test".into(),
        email: "test@example.com".into(),
        time_ms: 0,
        tz_offset_min: 0,
    };
    let began = Instant::now();
    // Here in this process, so the peak memory below is the import's own.
    let report = import_git(
        work.path(),
        &mut ImportRequest {
            who: &who,
            command: "import",
            git_settings: None,
            progress: &mut |_, _, _| {},
        },
        &mut None,
    );
    let took = began.elapsed();
    let report = report.unwrap_or_else(|e| panic!("{e}\n{}", e.hint().unwrap_or_default()));
    println!(
        "Imported {} commits in {:.1} s, peak memory {}: {} lines, {} server lines, {} tags, {} \
         commits not exact, {} entries skipped.",
        report.commits,
        took.as_secs_f64(),
        peak_memory(),
        report.lines,
        report.server_lines,
        report.tags,
        report.inexact_commits,
        report.skipped_entry_count
    );
    let began = Instant::now();
    let headers = assert_matches_git(work.path());
    println!(
        "Every commit, tree, file and ref matches Git; {headers} commits keep their Git header \
         ({:.1} s to compare).",
        began.elapsed().as_secs_f64()
    );
    // A clone's server lines give the walk tips enough, without a branch of its own beside main.
    let (pairs, ..) = assert_ancestry_matches_git(work.path());
    assert_eq!(assert_starts_match_git(work.path()), pairs);
    assert!(
        pairs > 0,
        "no pairs of tips to compare the ancestry walk on"
    );
    // A change ID no save has reads every save the lines and tags reach, through every parent.
    let repo = Repository::open(work.path()).unwrap();
    let began = Instant::now();
    let missed = opal_core::history::resolve(&repo, "zzzzzzzzzzzzzzzz");
    assert!(missed.is_err());
    println!(
        "A change ID no save has took {:.2} s to look for.",
        began.elapsed().as_secs_f64()
    );
    who_against_git_blame(&repo, work.path());
}

/// Times `opalro who` against `git blame` on five files of different ages and sizes at the
/// current branch's tip, and prints the share of lines on which the two name the same commit.
/// Only that `who` names a save for every line is asserted.
fn who_against_git_blame(repo: &Repository, dir: &Path) {
    let branch = text(&git(dir, &["symbolic-ref", "--short", "HEAD"]));
    let branch = branch.trim();
    let tip = text(&git(dir, &["rev-parse", branch]));
    let env = Env::empty();
    let mut commits = BTreeMap::new();
    for file in [
        "Makefile",
        "diff.c",
        "git-compat-util.h",
        "README.md",
        "t/t8002-blame.sh",
    ] {
        let began = Instant::now();
        let out = (env.opal_in(dir))
            .args(["who", file, "--save", branch, "--json"])
            .output()
            .unwrap();
        let who_took = began.elapsed();
        assert_eq!(out.status.code(), Some(0), "{file}: {}", text(&out.stderr));
        let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let mut ours = Vec::new();
        for run in doc["runs"].as_array().unwrap() {
            let save = run["save"].as_str();
            let save = save.unwrap_or_else(|| panic!("{file}: a line with no save: {run}"));
            let commit = (commits.entry(save.to_owned()))
                .or_insert_with(|| git_commit(repo, &Id::from_hex(save).unwrap()));
            ours.extend(std::iter::repeat_n(
                commit.clone(),
                run["lines"].as_u64().unwrap() as usize,
            ));
        }
        assert!(!ours.is_empty(), "{file} has no lines");
        assert_eq!(ours.len() as u64, doc["lines"].as_u64().unwrap(), "{file}");

        let began = Instant::now();
        let blamed = text(&git(dir, &["blame", "--porcelain", tip.trim(), "--", file]));
        let git_took = began.elapsed();
        // Each line's header (its commit first) comes before the line, which starts with a tab.
        let mut theirs = Vec::new();
        let mut header = true;
        for line in blamed.lines() {
            if header {
                theirs.push(line.split(' ').next().unwrap().to_owned());
                header = false;
            } else if line.starts_with('\t') {
                header = true;
            }
        }
        // A save for every line of the file, which has as many lines as Git reads in it.
        assert_eq!(ours.len(), theirs.len(), "{file}");
        let same = ours.iter().zip(&theirs).filter(|(a, b)| a == b).count();
        println!(
            "who {file}: {} lines, {:.2} s; git blame: {} lines, {:.2} s; the same commit on \
             {:.1}% of lines{}",
            ours.len(),
            who_took.as_secs_f64(),
            theirs.len(),
            git_took.as_secs_f64(),
            100.0 * same as f64 / ours.len() as f64,
            if doc["coarse"] == true {
                " (who was coarse)"
            } else {
                ""
            }
        );
    }
}

/// The most memory this process has held, as Linux reports it (VmHWM). Other systems need a
/// system call to tell.
fn peak_memory() -> String {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    match status.lines().find_map(|line| line.strip_prefix("VmHWM:")) {
        Some(kb) => {
            let kb: u64 = kb.trim().trim_end_matches("kB").trim().parse().unwrap();
            format!("{} MiB", kb / 1024)
        }
        None => "not measured on this system".to_owned(),
    }
}
