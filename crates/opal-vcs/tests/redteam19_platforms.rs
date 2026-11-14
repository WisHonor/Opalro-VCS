//! Red-team session RT19 (decision 024): combines on drives that ignore letter case, which the
//! Linux sandbox can't run. Unrun there; the controller runs them on a throwaway CI branch.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[cfg(any(windows, target_os = "macos"))]
mod support;

#[cfg(any(windows, target_os = "macos"))]
mod case_drives {
    use super::support::{self, Env, text};

    fn ok(env: &Env, args: &[&str]) -> String {
        let out = env.opalro().args(args).output().unwrap();
        let said = format!("{}{}", text(&out.stdout), text(&out.stderr));
        assert_eq!(out.status.code(), Some(0), "{args:?}: {said}");
        said
    }

    /// The names in the top folder as the drive spells them.
    fn names(env: &Env) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(env.root())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n != ".opalro")
            .collect();
        names.sort();
        names
    }

    const START: &str = "1\n2\n3\n4\n5\n6\n7\n8\n";

    /// `readme` saved, then line `side` renames it to `README` (a move by letter case alone), and
    /// `main` edits its last line.
    fn renamed_on_side(env: &Env) {
        ok(env, &["start", "--preset", "custom"]);
        std::fs::write(env.root().join("readme"), START).unwrap();
        ok(env, &["save", "A"]);
        ok(env, &["line", "new", "side"]);
        std::fs::rename(env.root().join("readme"), env.root().join("README")).unwrap();
        ok(env, &["save", "Rename"]);
        ok(env, &["goto", "main"]);
        std::fs::write(env.root().join("readme"), START.replace('8', "eight")).unwrap();
        ok(env, &["save", "Edit"]);
    }

    #[test]
    fn a_combine_that_renames_a_file_by_letter_case_keeps_it_on_disk() {
        let env = Env::empty();
        renamed_on_side(&env);
        let side = env.state().1.lines["side"];
        let saved: Vec<String> = support::saved_files(&env, side).into_keys().collect();
        if saved != ["README"] {
            eprintln!("this drive's save kept the old spelling ({saved:?}); nothing to try");
            return;
        }
        ok(&env, &["combine", "side"]);
        assert_eq!(names(&env), ["README"]);
        assert_eq!(
            std::fs::read_to_string(env.root().join("README")).unwrap(),
            START.replace('8', "eight")
        );
        assert!(
            ok(&env, &["status"]).contains("othing"),
            "the folder is the save"
        );
        ok(&env, &["undo"]);
        assert_eq!(names(&env), ["readme"]);
        ok(&env, &["redo"]);
        assert_eq!(names(&env), ["README"]);
    }
}
