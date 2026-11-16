//! Throwaway, gate branch only: imports Git's own repository and writes, for every line of five
//! files, the Git commit `opalro who` names, for whodiff/analyze.py to check against Git.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use opal_core::id::Id;
use opal_core::import::{ImportRequest, import_git};
use opal_core::repo::Repository;
use opal_core::save::Person;
use support::{Env, git, git_commit, text};

#[test]
#[ignore = "gate only: needs WHODIFF_WORK and WHODIFF_OUT"]
fn who_lines_of_a_real_repository() {
    let work = PathBuf::from(std::env::var_os("WHODIFF_WORK").unwrap());
    let out = PathBuf::from(std::env::var_os("WHODIFF_OUT").unwrap());
    let who = Person {
        name: "Test".into(),
        email: "test@example.com".into(),
        time_ms: 0,
        tz_offset_min: 0,
    };
    let report = import_git(
        &work,
        &mut ImportRequest {
            who: &who,
            command: "import",
            git_settings: None,
            progress: &mut |_, _, _| {},
        },
        &mut None,
    );
    let report = report.unwrap_or_else(|e| panic!("{e}"));
    println!(
        "Imported {} commits: {} not exact, {} entries skipped.",
        report.commits, report.inexact_commits, report.skipped_entry_count
    );
    let repo = Repository::open(&work).unwrap();
    let branch = text(&git(&work, &["symbolic-ref", "--short", "HEAD"]));
    let branch = branch.trim();
    let env = Env::empty();
    let mut commits = BTreeMap::new();
    let mut tsv = String::new();
    for file in [
        "Makefile",
        "diff.c",
        "git-compat-util.h",
        "README.md",
        "t/t8002-blame.sh",
    ] {
        let done = (env.opal_in(&work))
            .args(["who", file, "--save", branch, "--json"])
            .output()
            .unwrap();
        assert_eq!(done.status.code(), Some(0), "{file}: {}", text(&done.stderr));
        let doc: serde_json::Value = serde_json::from_slice(&done.stdout).unwrap();
        println!("who {file}: {} lines, coarse {}", doc["lines"], doc["coarse"]);
        for run in doc["runs"].as_array().unwrap() {
            let save = run["save"].as_str().unwrap();
            let commit = (commits.entry(save.to_owned()))
                .or_insert_with(|| git_commit(&repo, &Id::from_hex(save).unwrap()));
            let first = run["first"].as_u64().unwrap();
            let was = run["was"].as_str().unwrap_or("");
            for i in 0..run["lines"].as_u64().unwrap() {
                writeln!(tsv, "{file}\t{}\t{commit}\t{was}", first + i).unwrap();
            }
        }
    }
    std::fs::write(out, tsv).unwrap();
}
