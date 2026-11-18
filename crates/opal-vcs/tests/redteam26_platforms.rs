//! Red-team session RT26 (milestone 11, second session): proofs that need Linux's `/proc`.
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;
use std::process::Stdio;
use std::time::Duration;

use opal_core::git::craft::Craft;
use opal_core::git::{GitId, HashKind, ObjectType};
use support::Env;

/// The most memory the process `pid` has held at once so far, by `/proc` (`VmHWM`).
fn peak_so_far(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let line = status.lines().find_map(|l| l.strip_prefix("VmHWM:"))?;
    let kib: u64 = line.trim().strip_suffix("kB")?.trim().parse().ok()?;
    Some(kib << 10)
}

/// `i` in four letters of base 36, which sort as the numbers do.
fn name36(mut i: usize) -> String {
    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = [0u8; 4];
    for place in out.iter_mut().rev() {
        *place = digits[i % 36];
        i /= 36;
    }
    String::from_utf8(out.to_vec()).unwrap()
}

/// RT26-2: the comparison's list of folders still to compare held each one's whole path. A
/// folder 4,085 bytes down, of 300,000 folders that each hold the file `x`, as one blob in the
/// first commit and another in the second: the comparison listed all 300,000 paths, 4 KB each,
/// before it compared any. In release, with 250,000 of them (a 1.4 MB repository) the import
/// held 1,081 MB where v0.2 held 91 MB; with 1,000,000 (4.9 MB) it passed the sandbox's 4 GB
/// and was killed, where v0.2 imported it holding 312 MB (`widedeep.py`).
#[test]
fn comparing_a_wide_folder_deep_down_holds_no_path_for_each_entry() {
    const FOLDERS: usize = 300_000;
    const MOST: u64 = 512 << 20;
    let env = Env::empty();
    let git = Craft::new(env.root(), HashKind::Sha1);
    let names: Vec<String> = (0..FOLDERS).map(|j| format!("s{}", name36(j))).collect();
    // Five levels of 765 letters and one of 255: the wide folder's path is 4,085 bytes.
    let levels: Vec<String> = (0..6)
        .map(|level| format!("{level}{}", "a".repeat(if level < 5 { 764 } else { 254 })))
        .collect();
    let mut tip: Option<GitId> = None;
    for (i, body) in ["A\n", "B\n"].into_iter().enumerate() {
        let blob = git.object(ObjectType::Blob, body.as_bytes());
        let one = git.tree(&[(b"100644", b"x", blob)]);
        let wide: Vec<(&[u8], &[u8], GitId)> = names
            .iter()
            .map(|n| (b"40000".as_slice(), n.as_bytes(), one))
            .collect();
        let mut top = git.tree(&wide);
        for level in levels.iter().rev() {
            top = git.tree(&[(b"40000", level.as_bytes(), top)]);
        }
        let mut raw = format!("tree {}\n", top.to_hex());
        if let Some(parent) = tip {
            raw += &format!("parent {}\n", parent.to_hex());
        }
        raw += "author T <t@e> 1000000000 +0000\ncommitter T <t@e> 1000000000 +0000\n\n";
        raw += &format!("commit {i}\n");
        tip = Some(git.commit(raw.as_bytes()));
    }
    git.loose_ref("refs/heads/main", &format!("{}\n", tip.unwrap().to_hex()));

    let mut child = env
        .opal_process_in(env.root())
        .arg("import")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut peak = 0;
    let ended = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        peak = peak_so_far(child.id()).unwrap_or(peak);
        if peak > MOST {
            child.kill().unwrap();
            child.wait().unwrap();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        ended.is_some_and(|status| status.success()),
        "the import of two commits of {FOLDERS} one-file folders had held {} MiB, past the {} MiB \
         it should need, and was {}",
        peak >> 20,
        MOST >> 20,
        if ended.is_some() {
            "done"
        } else {
            "still going"
        }
    );
}
