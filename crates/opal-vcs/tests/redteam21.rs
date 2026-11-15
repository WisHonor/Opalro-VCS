//! The proofs of red-team session RT21 (milestone 9, conflicts): one test per finding, each
//! asserting what should be true; see docs/redteam/2026-11-15-milestone-9.md.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;
use support::{Env, text};

/// `opalro args`: its exit code and everything it printed.
fn run(env: &Env, args: &[&str]) -> (i32, String) {
    let out = env.opalro().args(args).output().unwrap();
    let said = format!("{}{}", text(&out.stdout), text(&out.stderr));
    (out.status.code().unwrap_or(-1), said)
}

fn ok(env: &Env, args: &[&str]) -> String {
    let (code, said) = run(env, args);
    assert_eq!(code, 0, "{args:?}: {said}");
    said
}

/// A started project on `main` whose `a.txt` `main` and `side` changed apart, combined: `a.txt`
/// is marked, and `main` holds it in conflict.
fn combined_with_a_conflict() -> Env {
    let env = Env::empty();
    ok(&env, &["start", "--preset", "custom"]);
    env.overwrite("a.txt", "1\n2\n3\n4\n5\n6\n7\n8\n");
    ok(&env, &["save", "A"]);
    ok(&env, &["line", "new", "side"]);
    env.overwrite("a.txt", "1\nside\n3\n4\n5\n6\n7\n8\n");
    ok(&env, &["save", "On side"]);
    ok(&env, &["goto", "main"]);
    env.overwrite("a.txt", "1\nmain\n3\n4\n5\n6\n7\n8\n");
    ok(&env, &["save", "On main"]);
    ok(&env, &["combine", "side"]);
    env
}

/// RT21-1. `opalro combine <line> --only-if-clean` promises that once it succeeds the save the
/// line is on holds no conflict (decision 181, `docs/json.md`). When the line already has
/// everything the target has, it ends with 0 and says "Nothing to combine" though the line's save
/// holds a conflict: a script running `opalro combine side --only-if-clean && build` goes on.
#[test]
fn only_if_clean_never_succeeds_on_a_line_that_holds_a_conflict() {
    let env = combined_with_a_conflict();
    let (code, said) = run(&env, &["combine", "side", "--only-if-clean"]);
    let (_, status) = run(&env, &["status"]);
    assert!(
        status.contains("In conflict (1 place)"),
        "the setup should hold a conflict: {status}"
    );
    assert_ne!(
        code, 0,
        "--only-if-clean ended with 0 while the line's save holds a conflict:\n{said}\n{status}"
    );
}

/// RT21-4. Two lines fix the same conflict differently and are combined: the save where they
/// parted held `a.txt` (in conflict), so the new conflict's middle is "can't tell" and its
/// marker label says `which Opalro can't tell here`. The record stores that as an absent
/// version, and `resolve a.txt --copy before` then says there was no file there, which is false.
#[test]
fn copy_before_does_not_say_a_file_that_was_there_was_not() {
    let env = combined_with_a_conflict();
    ok(&env, &["line", "new", "fix-one"]);
    env.overwrite("a.txt", "1\nX\n3\n4\n5\n6\n7\n8\n");
    ok(&env, &["resolve"]);
    ok(&env, &["goto", "main"]);
    ok(&env, &["line", "new", "fix-two"]);
    env.overwrite("a.txt", "1\nY\n3\n4\n5\n6\n7\n8\n");
    ok(&env, &["resolve"]);
    ok(&env, &["combine", "fix-one"]);
    let marked = std::fs::read_to_string(env.root().join("a.txt")).unwrap();
    assert!(marked.contains("which Opalro can't tell here"), "{marked}");
    let to = env.config().join("before.txt");
    let (_, said) = run(
        &env,
        &[
            "resolve",
            "a.txt",
            "--copy",
            "before",
            "--to",
            to.to_str().unwrap(),
        ],
    );
    assert!(
        !said.contains("There was no file at a.txt where the lines parted"),
        "{said}"
    );
}

const PIC_START: &[u8] = b"\x89PNG\r\n\x1a\n\0start";
const PIC_SIDE: &[u8] = b"\x89PNG\r\n\x1a\n\0side";
const PIC_MAIN: &[u8] = b"\x89PNG\r\n\x1a\n\0main";

/// RT21-3. Decision 177 says a picture in conflict "is never settled by accident": only `--keep`,
/// or naming its path once the person has put the version they want there. A pattern takes only
/// the fixed places it matches. But a folder's path, `.` included, takes every place below it as
/// the folder holds it, so a picture that a program rewrote by itself (a game engine opening the
/// project) is recorded as decided by `opalro resolve .` typed after fixing one text file, the
/// way `git add .` is typed. The pattern `*` in the same folder records only the text.
#[test]
fn resolving_a_folder_records_no_picture_nobody_picked() {
    let env = Env::empty();
    ok(&env, &["start", "--preset", "custom"]);
    env.overwrite("a.txt", "1\n2\n3\n4\n5\n6\n7\n8\n");
    support::write(&env, "pic.png", PIC_START);
    ok(&env, &["save", "A"]);
    ok(&env, &["line", "new", "side"]);
    env.overwrite("a.txt", "1\nside\n3\n4\n5\n6\n7\n8\n");
    support::write(&env, "pic.png", PIC_SIDE);
    ok(&env, &["save", "On side"]);
    ok(&env, &["goto", "main"]);
    env.overwrite("a.txt", "1\nmain\n3\n4\n5\n6\n7\n8\n");
    support::write(&env, "pic.png", PIC_MAIN);
    ok(&env, &["save", "On main"]);
    ok(&env, &["combine", "side"]);
    // The person fixes the text; a program rewrites the picture by itself.
    env.overwrite("a.txt", "1\nboth\n3\n4\n5\n6\n7\n8\n");
    support::write(
        &env,
        "pic.png",
        b"\x89PNG\r\n\x1a\n\0re-exported by the engine",
    );
    let (code, said) = run(&env, &["resolve", "."]);
    let (_, status) = run(&env, &["status"]);
    assert!(
        code != 0 || status.contains("pic.png"),
        "`opalro resolve .` recorded the picture nobody picked (exit {code}):\n{said}\n{status}"
    );
}

/// A record of 255 versions, all absent but the middle one, which differs for each `i`: about
/// 300 bytes stored, before compression.
#[cfg(target_os = "linux")]
fn wide(i: u64) -> opal_core::conflict::Record {
    use opal_core::conflict::{Record, Term};
    use opal_core::id::Id;
    use opal_core::listing::EntryKind;
    let mut terms = vec![None; 255];
    let mut id = [0u8; 32];
    id[..8].copy_from_slice(&i.to_le_bytes());
    terms[127] = Some(Term {
        kind: EntryKind::File,
        id: Id(id),
        size: 1,
    });
    Record { terms, own: None }
}

/// RT21-2. The store remembers conflict records only within a budget, but `status` and `save`
/// each also keep every record they read in a map of their own, with no bound. A planted save of
/// 20,000 conflict entries, each naming its own record of 255 versions (3.4 MB of store), makes
/// each of them hold about 300 MB; 250,000 such entries (43 MB of store) take `status` past 3 GB
/// until it aborts. The same save with ordinary files instead takes about 20 MB.
#[cfg(target_os = "linux")]
#[test]
fn status_and_save_hold_little_for_many_wide_conflict_records() {
    use opal_core::listing::{Entry, EntryKind, Listing};
    use opal_core::pack::ObjectKind;
    let env = Env::empty();
    ok(&env, &["start", "--preset", "custom"]);
    support::plant_save(env.root(), |pack| {
        let mut top = Vec::new();
        for f in 0..20u64 {
            let entries = (0..1000u64)
                .map(|n| Entry {
                    name: format!("c{n:04}"),
                    kind: EntryKind::Conflict,
                    id: support::add_conflict(pack, &wide(f * 1000 + n)),
                    size: 0,
                })
                .collect();
            let listing = Listing::new(entries).unwrap().encode();
            top.push(Entry {
                name: format!("d{f:02}"),
                kind: EntryKind::Folder,
                id: pack
                    .add_record(ObjectKind::FolderListing, &listing)
                    .unwrap(),
                size: 0,
            });
        }
        let top = Listing::new(top).unwrap().encode();
        pack.add_record(ObjectKind::FolderListing, &top).unwrap()
    });
    for args in [&["status"][..], &["save", "Again"][..]] {
        let cmd = support::opal_capped(env.config(), env.root(), args);
        let ended = support::run_limited(cmd, 300);
        assert_eq!(ended.code, Some(0), "{args:?}: {ended:?}");
        assert!(
            ended.peak_mb < 120,
            "{args:?} held {} MB for 20,000 planted conflicts",
            ended.peak_mb
        );
    }
}
