//! Red-team session RT13 (milestone 6, speed and space, second session): proof tests. Each
//! asserts the safe behaviour, so it fails on the code attacked and passes once its finding is
//! fixed. The report is docs/redteam/2026-11-10-milestone-6-speed-session-2.md.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use support::Env;
#[cfg(target_os = "linux")]
use support::text;

/// The bytes `opalro <args>` read (`rchar`) in `env`, which the shell that waited for it counts;
/// "failed" if it did.
#[cfg(target_os = "linux")]
fn reads(env: &Env, args: &str) -> String {
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c")
        .arg(format!(
            "\"$0\" {args} >/dev/null 2>&1 || echo failed; grep rchar /proc/$$/io"
        ))
        .arg(assert_cmd::cargo::cargo_bin("opalro"))
        .current_dir(env.root())
        .env("OPALRO_USER_NAME", "Test")
        .env("OPALRO_USER_EMAIL", "test@example.com")
        .env("OPALRO_CONFIG_DIR", env.config())
        .env("HOME", env.config());
    text(&cmd.output().unwrap().stdout)
}

#[cfg(target_os = "linux")]
fn rchar(said: &str) -> u64 {
    assert!(!said.contains("failed"), "{said}");
    said.lines()
        .find_map(|l| l.strip_prefix("rchar:"))
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

/// The bytes `opalro discard` read (`rchar`) in `env`, checked to have written `whole` to `big`.
#[cfg(target_os = "linux")]
fn discard_reads(env: &Env, whole: &[u8]) -> u64 {
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c")
        .arg("\"$0\" discard >/dev/null 2>&1 || echo failed; grep rchar /proc/$$/io")
        .arg(assert_cmd::cargo::cargo_bin("opalro"))
        .current_dir(env.root())
        .env("OPALRO_USER_NAME", "Test")
        .env("OPALRO_USER_EMAIL", "test@example.com")
        .env("OPALRO_CONFIG_DIR", env.config())
        .env("HOME", env.config());
    let said = text(&cmd.output().unwrap().stdout);
    assert!(!said.contains("failed"), "{said}");
    assert_eq!(std::fs::read(env.root().join("big")).unwrap(), whole);
    said.lines()
        .find_map(|l| l.strip_prefix("rchar:"))
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

/// RT13-1. RT12-2's fix reads a 64 KiB block at the second read in a row that falls in it. A
/// file whose tiny chunks are stored two by two, each pair in another block than the pair before
/// it, makes every pair read a whole block: about 32 KiB read for each 8-byte chunk, where main
/// read under 500 bytes.
#[cfg(target_os = "linux")]
#[test]
fn rt13_1_small_reads_in_pairs_cost_small_reads() {
    use opal_core::chunks::{Chunk, ChunkList};
    use opal_core::id::hash_content;

    const CHUNKS: usize = 20_000;
    // Columns of pairs, each over 64 KiB, so the next pair is always in another block.
    const COLUMNS: usize = 15;
    let env = Env::empty();
    env.opalro()
        .args(["start", "--preset", "custom"])
        .assert()
        .success();
    let bytes: Vec<Vec<u8>> = (0..CHUNKS as u64)
        .map(|j| j.to_le_bytes().to_vec())
        .collect();
    support::add_pack(env.root(), |pack| {
        for column in 0..COLUMNS {
            for pair in (column..CHUNKS / 2).step_by(COLUMNS) {
                pack.add_content(&bytes[2 * pair]).unwrap();
                pack.add_content(&bytes[2 * pair + 1]).unwrap();
            }
        }
    });
    let whole = bytes.concat();
    let list = ChunkList {
        file: hash_content(&whole),
        size: whole.len() as u64,
        chunks: bytes
            .iter()
            .map(|b| Chunk {
                id: hash_content(b),
                size: b.len() as u64,
            })
            .collect(),
    };
    support::plant_save(env.root(), |pack| {
        pack.add_chunk_list(&list).unwrap();
        support::top_listing(pack, &[("big", list.file, list.size)])
    });
    let read = discard_reads(&env, &whole);
    let per_chunk = read / CHUNKS as u64;
    assert!(
        per_chunk <= 4096,
        "read {read} bytes to write {} bytes: {per_chunk} a chunk",
        whole.len()
    );
}

/// A project imported from Git: `main` of two commits, and `branches` lines of `depth` commits
/// each that parted from it at its first.
fn old_branches(branches: usize, depth: usize) -> Env {
    let env = Env::new();
    let mut stream = b"commit refs/heads/main\nmark :1\ncommitter a <a@b> 1700000000 +0000\n\
data 1\nr\nM 100644 inline f\ndata 2\nr\n\n\
commit refs/heads/main\nmark :2\ncommitter a <a@b> 1700000001 +0000\ndata 1\nm\nfrom :1\n\
M 100644 inline f\ndata 2\nm\n\n"
        .to_vec();
    let mut mark = 2;
    for b in 0..branches {
        let mut prev = 1;
        for i in 0..depth {
            mark += 1;
            let data = format!("b{b}.{i}\n");
            stream.extend_from_slice(
                format!(
                    "commit refs/heads/b{b:03}\nmark :{mark}\ncommitter a <a@b> {} +0000\n\
data 1\nc\nfrom :{prev}\nM 100644 inline g\ndata {}\n{data}\n",
                    1_700_000_002 + mark,
                    data.len()
                )
                .as_bytes(),
            );
            prev = mark;
        }
    }
    env.git_input(&["fast-import", "--quiet"], &stream);
    env.git(&["checkout", "-q", "-f", "main"]);
    env.import();
    env
}

/// RT13-2, as the promise now stands. RT12-1's fix walked every other line on every amend; the
/// cache that fixed this read each line once, on the first amend. An amend of a save Opalro just
/// made now asks nothing of lines that haven't moved since, so it reads none of their saves:
/// a small part of what the same amend reads once the note of the save is gone.
#[cfg(target_os = "linux")]
#[test]
fn an_amend_of_a_save_just_made_reads_a_quarter_or_less_of_what_it_reads_without_its_note() {
    let env = old_branches(40, 500);
    std::fs::write(env.root().join("f"), "saved\n").unwrap();
    rchar(&reads(&env, "save s"));
    std::fs::write(env.root().join("f"), "more 0\n").unwrap();
    let just_made = rchar(&reads(&env, "save --amend m0"));
    std::fs::remove_file(env.root().join(".opalro/cache/last-made")).unwrap();
    std::fs::write(env.root().join("f"), "more 1\n").unwrap();
    let without = rchar(&reads(&env, "save --amend m1"));
    assert!(
        just_made * 4 <= without,
        "the amend right after a save read {just_made} bytes, the one without a note {without}"
    );
}

/// The red team's shape at full size, timed: an amend right after a save, and one with no note.
/// Asserts nothing on a clock; run in a release build for docs/benchmarks.md.
#[test]
#[ignore = "builds 600,000 commits; run by hand in a release build"]
fn amend_in_a_project_of_300_old_lines_timed() {
    let env = old_branches(300, 2000);
    let timed = |args: &[&str]| {
        let started = std::time::Instant::now();
        env.opalro().args(args).assert().success();
        started.elapsed()
    };
    std::fs::write(env.root().join("f"), "saved\n").unwrap();
    timed(&["save", "s"]);
    std::fs::write(env.root().join("f"), "more 0\n").unwrap();
    let just_made = timed(&["save", "--amend", "m0"]);
    std::fs::remove_file(env.root().join(".opalro/cache/last-made")).unwrap();
    std::fs::write(env.root().join("f"), "more 1\n").unwrap();
    let without = timed(&["save", "--amend", "m1"]);
    println!("an amend right after a save: {just_made:?}; an amend with no note: {without:?}");
}
