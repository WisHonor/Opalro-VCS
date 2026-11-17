//! The proofs of red-team session RT20 (milestone 8, `opalro combine`, second session): one test
//! per finding, each asserting what should be true; see
//! docs/redteam/2026-11-14-milestone-8-session-2.md.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;
use support::{Env, text};

/// `opalro args`, which must succeed; its stdout.
fn ok(env: &Env, args: &[&str]) -> String {
    let out = env.opalro().args(args).output().unwrap();
    let said = format!("{}{}", text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "{args:?}: {said}");
    text(&out.stdout)
}

/// `opalro combine <line> --json`: its exit code and document.
fn combine(env: &Env, line: &str) -> (i32, serde_json::Value) {
    let out = env
        .opalro()
        .args(["combine", line, "--json"])
        .output()
        .unwrap();
    let doc = serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
    (out.status.code().unwrap_or(-1), doc)
}

fn kinds(doc: &serde_json::Value) -> Vec<String> {
    doc["warnings"]
        .as_array()
        .map(|w| {
            w.iter()
                .map(|w| w["kind"].as_str().unwrap_or_default().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// RT20-1. A rule in `opalro.toml` says a Unity scene is never merged line by line. Another line
/// changes nothing but `opalro.toml`, into a file Opalro can't use, and is combined in: the
/// combine says nothing of it. The next combine then merges the scene line by line, since the
/// folder's rules file is passed over and Opalro's own values apply, and says nothing either.
/// It should leave the scene for the person (a conflict), or at least say the rules file was
/// passed over (`rules_file_skipped`), as `save` does.
#[test]
fn a_combine_with_an_unusable_rules_file_says_so_before_merging_a_protected_file() {
    let env = Env::empty();
    ok(&env, &["start", "--preset", "custom"]);
    support::write(
        &env,
        "opalro.toml",
        "[[rule]]\nfiles = [\"*.unity\"]\nconflict-markers = false\n",
    );
    support::write(&env, "Main.unity", "1\n2\n3\n4\n5\n6\n7\n8\n");
    ok(&env, &["save", "Scene"]);
    // `side` breaks the rules file, and changes nothing else.
    ok(&env, &["line", "new", "side"]);
    support::write(&env, "opalro.toml", "[[rule]\nfiles = [\"*.unity\"]\n");
    ok(&env, &["save", "Rules"]);
    // `art` changes the scene's first line.
    ok(&env, &["goto", "main"]);
    ok(&env, &["line", "new", "art"]);
    support::write(&env, "Main.unity", "one\n2\n3\n4\n5\n6\n7\n8\n");
    ok(&env, &["save", "Art"]);
    // `main` changes its last line, while the rules file is still good.
    ok(&env, &["goto", "main"]);
    support::write(&env, "Main.unity", "1\n2\n3\n4\n5\n6\n7\neight\n");
    ok(&env, &["save", "Level"]);

    let (code, first) = combine(&env, "side");
    assert_eq!(code, 0, "{first}");
    let (code, second) = combine(&env, "art");
    let merged = second["merged_files"]
        .as_array()
        .is_some_and(|files| files.iter().any(|f| f == "Main.unity"));
    let told = [&first, &second]
        .iter()
        .any(|doc| kinds(doc).iter().any(|k| k == "rules_file_skipped"));
    assert!(
        !merged || told,
        "Main.unity was merged line by line against its rule, and no combine said the rules \
         file was passed over (exit {code}):\nfirst: {first}\nsecond: {second}\nscene: {:?}",
        std::fs::read_to_string(env.root().join("Main.unity")).unwrap()
    );
}
