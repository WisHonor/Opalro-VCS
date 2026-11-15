//! Opalro's line-by-line merge against `git merge-file`, on texts drawn at random.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant;

use opal_core::changes::DEADLINE;
use opal_core::combine::{Text, merge_text};
use opal_core::listing::EntryKind;
use opal_core::repo::Repository;
use proptest::prelude::*;
use proptest::sample::Index;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};
use support::{Env, Files, files, git, git_with_input, main_tip, saved_files, text};

const ALPHABET: &[&str] = &["a\n", "b\n", "c\n", "d\n"];

/// Replace, delete or insert a run of 1 to 3 lines somewhere, or (unique lines only) flip one line's
/// ending; `picks` choose the new lines when they come from the alphabet.
#[derive(Clone, Debug)]
struct Edit {
    kind: u8,
    at: prop::sample::Index,
    count: usize,
    picks: [usize; 3],
}

fn edits() -> impl Strategy<Value = Vec<Edit>> {
    prop::collection::vec(
        (
            0..4u8,
            any::<prop::sample::Index>(),
            1..=3usize,
            [0..4usize, 0..4usize, 0..4usize],
        )
            .prop_map(|(kind, at, count, picks)| Edit {
                kind,
                at,
                count,
                picks,
            }),
        0..=4,
    )
}

/// `base` with `edits` made; new lines are fresh (`<side> <n>`) or drawn from the alphabet.
fn edited(base: &[String], edits: &[Edit], side: &str, unique: bool) -> Vec<String> {
    let mut lines = base.to_vec();
    let mut fresh = 0;
    for e in edits {
        let at = e.at.index(lines.len() + 1);
        let new: Vec<String> = (0..e.count)
            .map(|i| {
                if unique {
                    fresh += 1;
                    format!("{side} {fresh}\n")
                } else {
                    ALPHABET[e.picks[i]].to_owned()
                }
            })
            .collect();
        let gone = e.count.min(lines.len() - at);
        match e.kind {
            0 => {
                lines.splice(at..at + gone, new);
            }
            1 => {
                lines.drain(at..at + gone);
            }
            // A `\r` here turns the line's ending the other way in `with_endings`.
            3 if unique => {
                if let Some(line) = lines.get_mut(at) {
                    *line = match line.strip_suffix("\r\n") {
                        Some(body) => format!("{body}\n"),
                        None => line.replace('\n', "\r\n"),
                    };
                }
            }
            _ => {
                lines.splice(at..at, new);
            }
        }
    }
    lines
}

fn three(unique: bool) -> impl Strategy<Value = [Vec<u8>; 3]> {
    let base = if unique {
        (5..=40usize)
            .prop_map(|n| (0..n).map(|i| format!("line {i}\n")).collect::<Vec<_>>())
            .boxed()
    } else {
        prop::collection::vec(prop::sample::select(ALPHABET), 5..=40)
            .prop_map(|lines| lines.into_iter().map(str::to_owned).collect())
            .boxed()
    };
    let endings = (any::<u64>(), any::<[bool; 3]>());
    (base, edits(), edits(), endings).prop_map(move |(base, ours, theirs, (crlf, open))| {
        let ours = edited(&base, &ours, "ours", unique);
        let theirs = edited(&base, &theirs, "theirs", unique);
        let mut texts = [base, ours, theirs].map(|lines| lines.concat().into_bytes());
        if unique {
            for (text, open) in texts.iter_mut().zip(open) {
                *text = with_endings(text, crlf, open);
            }
        }
        texts
    })
}

/// `text` with some lines ending in `\r\n`, chosen by `crlf` and the line's contents so that a
/// line keeps its ending in all three texts unless an edit flipped it, and maybe no line feed at
/// the end.
fn with_endings(text: &[u8], crlf: u64, open: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for line in text.split_inclusive(|&b| b == b'\n') {
        let (body, flipped) = match line.strip_suffix(b"\r\n") {
            Some(body) => (body, true),
            None => (&line[..line.len() - 1], false),
        };
        let sum: u32 = body.iter().map(|&b| u32::from(b)).sum();
        out.extend_from_slice(body);
        if (crlf >> (sum % 64) & 1 == 1) != flipped {
            out.push(b'\r');
        }
        out.push(b'\n');
    }
    if open {
        out.pop();
        if out.ends_with(b"\r") {
            out.pop();
        }
    }
    out
}

/// What `git merge-file -p ours base theirs` gives: the bytes when it merged cleanly.
fn git_merge(texts: &[Vec<u8>; 3]) -> Option<Vec<u8>> {
    let dir = tempfile::tempdir().unwrap();
    for (name, bytes) in ["base", "ours", "theirs"].iter().zip(texts) {
        std::fs::write(dir.path().join(name), bytes).unwrap();
    }
    let out = git_with_input(
        dir.path(),
        &["merge-file", "-p", "ours", "base", "theirs"],
        b"",
    );
    let code = out.status.code().unwrap();
    assert!(
        (0..=127).contains(&code),
        "git merge-file failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    (code == 0).then_some(out.stdout)
}

fn ours_merge(texts: &[Vec<u8>; 3]) -> Option<Vec<u8>> {
    match merge_text(&texts[0], &texts[1], &texts[2], DEADLINE, None).0 {
        Text::Merged(bytes) => Some(bytes),
        _ => None,
    }
}

fn shown(texts: &[Vec<u8>; 3]) -> String {
    let [base, ours, theirs] = texts
        .each_ref()
        .map(|t| String::from_utf8_lossy(t).into_owned());
    format!("base {base:?}\nours {ours:?}\ntheirs {theirs:?}")
}

fn config() -> Config {
    Config {
        cases: 256,
        failure_persistence: None,
        ..Config::default()
    }
}

/// Unique lines, ending in `\n` or `\r\n`, the last maybe in neither: the same answer as Git's,
/// always. The seed is fixed so that both outcomes surely occur often enough.
#[test]
fn text_merge_matches_git_merge_file_on_unique_lines() {
    let counts = RefCell::new((0, 0));
    let rng = TestRng::deterministic_rng(RngAlgorithm::ChaCha);
    TestRunner::new_with_rng(config(), rng)
        .run(&three(true), |texts| {
            let git = git_merge(&texts);
            prop_assert_eq!(&ours_merge(&texts), &git, "{}", shown(&texts));
            let mut c = counts.borrow_mut();
            if git.is_some() {
                c.0 += 1
            } else {
                c.1 += 1
            }
            Ok(())
        })
        .unwrap();
    let (clean, conflicts) = counts.into_inner();
    println!("{clean} clean, {conflicts} conflicts");
    assert!(
        clean >= 25 && conflicts >= 25,
        "{clean} clean, {conflicts} conflicts"
    );
}

/// Where lines repeat, base and a side can be paired in two ways that cost the same, and each
/// program's merge is right for its own pairing: only Opalro merges, only Git does, or both do
/// with different bytes. The seed is fixed and each count kept under a ceiling, so a large change
/// in how lines are paired shows. What holds whatever the pairing is checked in every case.
#[test]
fn text_merge_and_git_merge_file_on_repeating_lines() {
    const KINDS: [&str; 3] = [
        "both merged, different bytes",
        "only Opalro merged",
        "only Git merged",
    ];
    let found: RefCell<[Vec<String>; 3]> = RefCell::default();
    let rng = TestRng::deterministic_rng(RngAlgorithm::ChaCha);
    TestRunner::new_with_rng(config(), rng)
        .run(&three(false), |texts| {
            let [b, o, t] = &texts;
            let merged = |base: &[u8], ours: &[u8], theirs: &[u8]| {
                merge_text(base, ours, theirs, DEADLINE, None).0
            };
            prop_assert_eq!(merged(b, o, o), Text::Merged(o.clone()));
            prop_assert_eq!(merged(b, b, t), Text::Merged(t.clone()));
            prop_assert_eq!(merged(b, o, b), Text::Merged(o.clone()));
            let ours = ours_merge(&texts);
            let swapped = ours_merge(&[b.clone(), t.clone(), o.clone()]);
            prop_assert_eq!(&ours, &swapped, "{}", shown(&texts));
            let kind = match (ours, git_merge(&texts)) {
                (Some(ours), Some(git)) if ours != git => Some(0),
                (Some(_), None) => Some(1),
                (None, Some(_)) => Some(2),
                _ => None,
            };
            if let Some(k) = kind {
                found.borrow_mut()[k].push(shown(&texts));
            }
            Ok(())
        })
        .unwrap();
    let found = found.into_inner();
    for (kind, cases) in KINDS.iter().zip(&found) {
        println!("{kind}: {} cases", cases.len());
        if let Some(case) = cases.first() {
            println!("for example:\n{case}\n");
        }
    }
    // Ceilings, not exact counts: another Git may pair lines its own way.
    let counts = found.map(|c| c.len());
    let ceilings = [3, 6, 24];
    assert!(
        counts.iter().zip(ceilings).all(|(n, most)| *n <= most),
        "{KINDS:?}: {counts:?}, at most {ceilings:?}"
    );
}

// Whole combines against `git merge`.

const FOLDERS: [&str; 4] = ["", "src/", "src/deep/", "doc/"];
const NAMES: [&str; 8] = [
    "a.txt", "b.txt", "c.txt", "d.txt", "e.txt", "f.txt", "g.sh", "h.md",
];

/// Every path a generated history may use; no folder is named like a file.
fn all_paths() -> Vec<String> {
    FOLDERS
        .iter()
        .flat_map(|f| NAMES.iter().map(move |n| format!("{f}{n}")))
        .collect()
}

/// One change in a commit; each `Index` picks among what is there when the commit is made.
#[derive(Clone, Debug)]
enum Step {
    /// Replace (0), remove (1) or insert (2) a run of 1 to 3 lines, in one file.
    Edit(Index, Vec<(u8, Index, usize)>),
    /// A new file at a free path: its number of lines, and whether it is executable.
    Add(Index, usize, bool),
    Remove(Index),
    FlipMark(Index),
}

#[derive(Clone, Debug)]
enum Commit {
    Changes(Vec<Step>),
    /// One file moved to a free path, with nothing else changed; if `true`, one this branch
    /// edited, when there is one.
    Move(Index, Index, bool),
}

#[derive(Clone, Debug)]
struct History {
    /// The first commit's files: path, number of lines, executable.
    first: Vec<(String, usize, bool)>,
    main: Vec<Commit>,
    side: Vec<Commit>,
}

#[derive(Clone, Debug)]
struct File {
    lines: Vec<String>,
    exec: bool,
    /// Where the file was in the first commit, if it was there.
    origin: Option<String>,
    /// Moved on this branch.
    moved: bool,
    /// Edited on this branch before it moved, and after.
    edited: [bool; 2],
}

type Tree = BTreeMap<String, File>;

fn step() -> impl Strategy<Value = Step> {
    let edits = prop::collection::vec((0..3u8, any::<Index>(), 1..=3usize), 1..=3);
    prop_oneof![
        4 => (any::<Index>(), edits).prop_map(|(f, e)| Step::Edit(f, e)),
        1 => (any::<Index>(), 1..=8usize, prop::bool::weighted(0.25))
            .prop_map(|(at, n, x)| Step::Add(at, n, x)),
        1 => any::<Index>().prop_map(Step::Remove),
        1 => any::<Index>().prop_map(Step::FlipMark),
    ]
}

fn commits() -> impl Strategy<Value = Vec<Commit>> {
    let commit = prop_oneof![
        3 => prop::collection::vec(step(), 1..=3).prop_map(Commit::Changes),
        2 => (any::<Index>(), any::<Index>(), any::<bool>())
            .prop_map(|(f, t, edited)| Commit::Move(f, t, edited)),
    ];
    prop::collection::vec(commit, 1..=4)
}

fn history() -> impl Strategy<Value = History> {
    let sizes = prop::collection::vec((3..=12usize, prop::bool::weighted(0.25)), 12);
    let first =
        (prop::sample::subsequence(all_paths(), 3..=12), sizes).prop_map(|(paths, sizes)| {
            paths
                .into_iter()
                .zip(sizes)
                .map(|(p, (n, x))| (p, n, x))
                .collect()
        });
    (first, commits(), commits()).prop_map(|(first, main, side)| History { first, main, side })
}

fn free_path(tree: &Tree, at: &Index) -> Option<String> {
    let free: Vec<String> = all_paths()
        .into_iter()
        .filter(|p| !tree.contains_key(p))
        .collect();
    (!free.is_empty()).then(|| free[at.index(free.len())].clone())
}

fn nth_path(tree: &Tree, at: &Index) -> Option<String> {
    (!tree.is_empty()).then(|| tree.keys().nth(at.index(tree.len())).unwrap().clone())
}

/// Makes `commit` on `tree`. New lines are `<who> <n>`, unique in the whole history, so no file
/// ever holds a line twice and no two files share one.
fn make(tree: &mut Tree, commit: &Commit, who: &str, fresh: &mut usize) {
    let mut new_line = || {
        *fresh += 1;
        format!("{who} {fresh}\n")
    };
    let steps = match commit {
        Commit::Move(from, to, edited) => {
            let was_edited: Tree = tree
                .iter()
                .filter(|(_, f)| f.edited[0])
                .map(|(p, f)| (p.clone(), f.clone()))
                .collect();
            let pick = if *edited && !was_edited.is_empty() {
                nth_path(&was_edited, from)
            } else {
                nth_path(tree, from)
            };
            if let (Some(from), Some(to)) = (pick, free_path(tree, to)) {
                let mut file = tree.remove(&from).unwrap();
                file.moved = true;
                tree.insert(to, file);
            }
            return;
        }
        Commit::Changes(steps) => steps,
    };
    for step in steps {
        match step {
            Step::Edit(at, edits) => {
                let Some(path) = nth_path(tree, at) else {
                    continue;
                };
                let file = tree.get_mut(&path).unwrap();
                let was = file.lines.clone();
                for (kind, at, count) in edits {
                    let lines = &mut file.lines;
                    let at = at.index(lines.len() + 1);
                    let gone = (*count).min(lines.len() - at);
                    match kind {
                        0 => {
                            let new: Vec<String> = (0..*count).map(|_| new_line()).collect();
                            lines.splice(at..at + gone, new);
                        }
                        // Never the last line: an empty file has no contents to follow.
                        1 => {
                            let gone = gone.min(lines.len() - 1);
                            lines.drain(at..at + gone);
                        }
                        _ => {
                            let new: Vec<String> = (0..*count).map(|_| new_line()).collect();
                            lines.splice(at..at, new);
                        }
                    }
                }
                if file.lines != was {
                    file.edited[usize::from(file.moved)] = true;
                }
            }
            Step::Add(at, n, exec) => {
                let Some(path) = free_path(tree, at) else {
                    continue;
                };
                let lines = (0..*n).map(|_| new_line()).collect();
                let file = File {
                    lines,
                    exec: *exec,
                    origin: None,
                    moved: false,
                    edited: [false; 2],
                };
                tree.insert(path, file);
            }
            Step::Remove(at) => {
                if let Some(path) = nth_path(tree, at) {
                    tree.remove(&path);
                }
            }
            Step::FlipMark(at) => {
                if let Some(path) = nth_path(tree, at) {
                    let file = tree.get_mut(&path).unwrap();
                    file.exec = !file.exec;
                }
            }
        }
    }
}

/// A commit for `git fast-import`, from what changed between `before` and `after`.
fn commit_text(branch: &str, mark: u32, from: Option<u32>, before: &Tree, after: &Tree) -> String {
    // Commits share seconds, as imported histories often do.
    let who = format!("Ada <ada@example.com> {} +0000", 1_700_000_000 + mark / 2);
    let mut out = format!(
        "commit refs/heads/{branch}\nmark :{mark}\nauthor {who}\ncommitter {who}\ndata 3\nc{}\n",
        mark % 10
    );
    if let Some(from) = from {
        out += &format!("from :{from}\n");
    }
    for path in before.keys().filter(|p| !after.contains_key(*p)) {
        out += &format!("D {path}\n");
    }
    for (path, file) in after {
        let same = before
            .get(path)
            .is_some_and(|b| b.lines == file.lines && b.exec == file.exec);
        if !same {
            let mode = if file.exec { "100755" } else { "100644" };
            let data = file.lines.concat();
            out += &format!("M {mode} inline {path}\ndata {}\n{data}\n", data.len());
        }
    }
    out
}

/// Files of the first commit moved on a branch and still there at its end, as `[contents as they
/// were, edited only before the move, edited after it]`, each with how many of them the other
/// branch edited.
type MoveCounts = [(usize, usize); 3];

/// A generated history: the `git fast-import` stream, the files of the first commit and of each
/// branch's end (`side`, then `main`), and how many files each branch moved.
struct Generated {
    stream: String,
    first: Tree,
    ends: [Tree; 2],
    moves: MoveCounts,
}

impl Generated {
    /// Whether `path` is where a moved file of the first commit was, or where it went.
    fn moved(&self, path: &str) -> bool {
        self.ends.iter().flatten().any(|(at, f)| {
            f.moved && f.origin.is_some() && (at == path || f.origin.as_deref() == Some(path))
        })
    }

    /// Whether `branch` (0 `side`, 1 `main`) moved the first commit's `from` to `to` and edited
    /// it, and the other branch removed it.
    fn moved_edited_removed(&self, branch: usize, from: &str, to: &str) -> bool {
        let kept = self.ends[branch].get(to).is_some_and(|f| {
            f.moved && f.origin.as_deref() == Some(from) && f.lines != self.first[from].lines
        });
        let gone = !self.ends[1 - branch]
            .values()
            .any(|f| f.origin.as_deref() == Some(from));
        kept && gone
    }
}

/// The history `h` written out.
fn generate(h: &History) -> Generated {
    let mut fresh = 0;
    let first: Tree = h
        .first
        .iter()
        .enumerate()
        .map(|(i, (path, n, exec))| {
            let lines = (0..*n).map(|j| format!("file {i} line {j}\n")).collect();
            let file = File {
                lines,
                exec: *exec,
                origin: Some(path.clone()),
                moved: false,
                edited: [false; 2],
            };
            (path.clone(), file)
        })
        .collect();
    let mut out = commit_text("main", 1, None, &Tree::new(), &first);
    let mut mark = 1;
    let mut ends = Vec::new();
    for (branch, commits) in [("side", &h.side), ("main", &h.main)] {
        let mut tree = first.clone();
        for (i, commit) in commits.iter().enumerate() {
            let before = tree.clone();
            make(&mut tree, commit, branch, &mut fresh);
            mark += 1;
            out += &commit_text(branch, mark, (i == 0).then_some(1), &before, &tree);
        }
        ends.push(tree);
    }
    let edited = |tree: &Tree, origin: &String| {
        tree.values()
            .any(|f| f.origin.as_ref() == Some(origin) && f.lines != first[origin].lines)
    };
    let mut counts = MoveCounts::default();
    for (mine, other) in [(&ends[0], &ends[1]), (&ends[1], &ends[0])] {
        for file in mine.values() {
            if let (Some(origin), true) = (&file.origin, file.moved) {
                let c = &mut counts[match file.edited {
                    [_, true] => 2,
                    [true, false] => 1,
                    [false, false] => 0,
                }];
                c.0 += 1;
                c.1 += usize::from(edited(other, origin));
            }
        }
    }
    let [side, main] = <[Tree; 2]>::try_from(ends).unwrap();
    Generated {
        stream: out,
        first,
        ends: [side, main],
        moves: counts,
    }
}

/// Every file of the save `main` is at, with whether it is executable.
fn opalro_marks(root: &Path) -> BTreeMap<String, bool> {
    let repo = Repository::open(root).unwrap();
    let store = repo.store();
    let save = repo.current().unwrap().1.state.lines["main"];
    let mut found = BTreeMap::new();
    let mut folders = vec![(store.save(&save, false).unwrap().root, String::new())];
    while let Some((id, rel)) = folders.pop() {
        for e in store.listing(&id).unwrap().entries() {
            let path = if rel.is_empty() {
                e.name.clone()
            } else {
                format!("{rel}/{}", e.name)
            };
            if e.kind == EntryKind::Folder {
                folders.push((e.id, path));
            } else {
                found.insert(path, e.kind == EntryKind::Executable);
            }
        }
    }
    found
}

/// Every file of Git's `HEAD` in `dir`, with whether it is executable.
fn git_marks(dir: &Path) -> BTreeMap<String, bool> {
    let listed = git(dir, &["ls-tree", "-r", "-z", "HEAD"]);
    listed
        .split(|&b| b == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let entry = text(entry);
            let (head, path) = entry.split_once('\t').unwrap();
            (path.to_owned(), head.starts_with("100755"))
        })
        .collect()
}

/// Every folder below `dir` but `.opalro` and `.git` at the top, empty ones included.
fn folders(dir: &Path) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut todo = vec![(dir.to_path_buf(), String::new())];
    while let Some((at, rel)) = todo.pop() {
        for e in std::fs::read_dir(&at).unwrap() {
            let e = e.unwrap();
            let name = e.file_name().to_string_lossy().into_owned();
            if !e.file_type().unwrap().is_dir()
                || rel.is_empty() && [".opalro", ".git"].contains(&name.as_str())
            {
                continue;
            }
            let path = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            found.insert(path.clone());
            todo.push((e.path(), path));
        }
    }
    found
}

/// Whether each file below `dir` is executable on disk.
#[cfg(unix)]
fn disk_marks(dir: &Path) -> BTreeMap<String, bool> {
    use std::os::unix::fs::PermissionsExt;
    files(dir)
        .into_keys()
        .map(|p| {
            let mode = std::fs::metadata(dir.join(&p))
                .unwrap()
                .permissions()
                .mode();
            (p, mode & 0o111 != 0)
        })
        .collect()
}

/// The branch Git names in a conflict line: 0 for `side`, 1 for `main`.
fn branch_named(name: &str) -> usize {
    match name {
        "origin/side" => 0,
        "HEAD" => 1,
        _ => panic!("Git named a branch {name:?}"),
    }
}

/// Generated histories of two branches, combined by both programs. Where both combine cleanly,
/// they leave the same files, folders and marks, on disk and in the saves. Where Opalro asked for
/// a clean line stops, it stopped for conflicts and changed nothing; kept, they are at the places
/// Git reports unmerged, and taking one side everywhere gives what Git's `checkout --ours` or
/// `--theirs` gives. Git pairs a file moved and edited with its old self by likeness, which
/// Opalro never guesses: so Git alone may merge (Opalro's reasons then name moved files), where
/// the other branch removed the file Opalro alone keeps it where Git reports exactly that pair of
/// conflicts, and where both conflict only moved files may tell their places apart (such a case
/// is counted and printed, not compared). The seed is fixed; counts sit between floors and
/// ceilings.
#[test]
fn combine_matches_git_merge() {
    const KINDS: [&str; 4] = [
        "both clean",
        "both conflict",
        "Opalro clean only",
        "Git clean only",
    ];
    let counts = RefCell::new([0usize; 4]);
    let moves = RefCell::new(MoveCounts::default());
    let followed = RefCell::new(0);
    let git_only = RefCell::new(Vec::new());
    let opalro_only = RefCell::new(Vec::new());
    let compared = RefCell::new(0);
    let skipped = RefCell::new(Vec::new());
    let started = Instant::now();
    let rng = TestRng::deterministic_rng(RngAlgorithm::ChaCha);
    let config = Config {
        cases: 96,
        failure_persistence: None,
        ..Config::default()
    };
    TestRunner::new_with_rng(config, rng)
        .run(&history(), |h| {
            let made = generate(&h);
            let fast_import = &made.stream;
            let env = Env::new();
            let out = git_with_input(
                env.root(),
                &["fast-import", "--quiet"],
                fast_import.as_bytes(),
            );
            assert!(out.status.success(), "{}", text(&out.stderr));
            // The import takes the folder as it finds it.
            env.git(&["reset", "-q", "--hard"]);
            env.import();
            let clone = tempfile::tempdir().unwrap();
            git(
                env.root(),
                &["clone", "-q", ".", clone.path().to_str().unwrap()],
            );

            let before = (env.standing(), files(env.root()), folders(env.root()));
            let ours = env
                .opalro()
                .args(["combine", "side", "--only-if-clean"])
                .output()
                .unwrap();
            let ours_code = ours.status.code().unwrap();
            let said = text(&ours.stderr);
            prop_assert!(ours_code <= 1, "{said}\n{fast_import}");
            let why: Vec<&str> = said.lines().filter(|l| l.starts_with("  ")).collect();
            if ours_code == 1 {
                // Stopped for conflicts, not refused or failed, and nothing changed.
                prop_assert!(
                    said.starts_with("error: Combining side into main would leave ")
                        && said.contains(" in conflict")
                        && !why.is_empty(),
                    "{said}\n{fast_import}"
                );
                let after = (env.standing(), files(env.root()), folders(env.root()));
                prop_assert!(after == before, "{said}\n{fast_import}");
            }
            let theirs = git_with_input(
                clone.path(),
                &[
                    "-c",
                    "merge.directoryRenames=false",
                    "-c",
                    "merge.renames=true",
                    "merge",
                    "--no-edit",
                    "origin/side",
                ],
                b"",
            );
            let theirs_code = theirs.status.code().unwrap();
            prop_assert!(theirs_code <= 1, "{}", text(&theirs.stderr));

            let kind = match (ours_code == 0, theirs_code == 0) {
                (true, true) => 0,
                (false, false) => 1,
                (true, false) => 2,
                (false, true) => 3,
            };
            if kind == 0 {
                let on_disk = files(clone.path());
                prop_assert_eq!(&files(env.root()), &on_disk, "{}", fast_import);
                prop_assert_eq!(
                    &saved_files(&env, main_tip(&env)),
                    &on_disk,
                    "{}",
                    fast_import
                );
                prop_assert_eq!(
                    folders(env.root()),
                    folders(clone.path()),
                    "{}",
                    fast_import
                );
                let marks = git_marks(clone.path());
                prop_assert_eq!(&opalro_marks(env.root()), &marks, "{}", fast_import);
                #[cfg(unix)]
                {
                    prop_assert_eq!(&disk_marks(env.root()), &marks, "{}", fast_import);
                    prop_assert_eq!(&disk_marks(clone.path()), &marks, "{}", fast_import);
                }
                *followed.borrow_mut() += made.moves[0].1;
            }
            if kind == 1 {
                // Opalro keeps a conflict at each place it named; where Git reports one, at the
                // same places, unless moves the two follow differently explain every difference.
                opalro(&env, &["combine", "side"], 0);
                let places = conflict_places(env.root());
                let named: BTreeSet<String> = (why.iter())
                    .map(|l| l.trim_start().split_once(": ").unwrap().0.to_owned())
                    .collect();
                prop_assert_eq!(&named, &places, "{}\n{}", said, fast_import);
                let unmerged: BTreeSet<String> = (text(&git(
                    clone.path(),
                    &["diff", "--name-only", "--diff-filter=U"],
                ))
                .lines())
                .map(str::to_owned)
                .collect();
                if unmerged == places {
                    keep_sides_match_git(&env, clone.path(), fast_import)?;
                    *compared.borrow_mut() += 1;
                } else {
                    // Set aside only where Git says it followed a move at every path that tells
                    // the two apart, and only once nothing either line made is lost.
                    let renamed = git_renames(clone.path());
                    let apart: Vec<&String> = unmerged.symmetric_difference(&places).collect();
                    let unexplained: Vec<&&String> =
                        apart.iter().filter(|p| !renamed.contains(**p)).collect();
                    prop_assert!(
                        unexplained.is_empty(),
                        "Git: {unmerged:?}, Opalro: {places:?}; no move Git followed at {unexplained:?}\n{fast_import}"
                    );
                    if let Err(lost) = nothing_lost(&made, &kept_sides(&env)) {
                        prop_assert!(false, "{lost}\n{fast_import}");
                    }
                    skipped.borrow_mut().push(format!(
                        "Git {unmerged:?}, Opalro {places:?}: Git followed a move at {apart:?}"
                    ));
                }
            }
            if kind == 2 {
                // Only a file moved and edited on one branch and removed on the other, which Git
                // reports as a rename/delete of it and a modify/delete at its new place.
                let git_said = text(&theirs.stdout);
                let mut renamed = BTreeMap::new();
                let mut modified = BTreeSet::new();
                for c in git_said
                    .lines()
                    .filter_map(|l| l.strip_prefix("CONFLICT ("))
                {
                    if let Some(c) = c.strip_prefix("rename/delete): ") {
                        let (from, rest) = c.split_once(" renamed to ").unwrap();
                        let (to, rest) = rest.split_once(" in ").unwrap();
                        let by = branch_named(rest.split_once(',').unwrap().0);
                        renamed.insert(to.to_owned(), (from.to_owned(), by));
                    } else if let Some(c) = c.strip_prefix("modify/delete): ") {
                        modified.insert(c.split_once(" deleted in ").unwrap().0.to_owned());
                    } else {
                        prop_assert!(
                            false,
                            "Opalro combined cleanly where Git didn't:\n{git_said}\n{fast_import}"
                        );
                    }
                }
                prop_assert!(
                    !renamed.is_empty()
                        && renamed.keys().cloned().collect::<BTreeSet<_>>() == modified,
                    "Opalro combined cleanly where Git didn't:\n{git_said}\n{fast_import}"
                );
                for (to, (from, by)) in &renamed {
                    prop_assert!(
                        made.moved_edited_removed(*by, from, to),
                        "{from} to {to}:\n{git_said}\n{fast_import}"
                    );
                }
                // Git's stopped merge leaves the kept file at its new place and every other file
                // merged.
                prop_assert_eq!(files(env.root()), files(clone.path()), "{}", fast_import);
                prop_assert_eq!(
                    folders(env.root()),
                    folders(clone.path()),
                    "{}",
                    fast_import
                );
                prop_assert_eq!(
                    opalro_marks(env.root()),
                    git_left_marks(clone.path()),
                    "{}",
                    fast_import
                );
                let ours = opalro_marks(env.root());
                for (to, (from, _)) in &renamed {
                    prop_assert!(ours.contains_key(to) && !ours.contains_key(from), "{to}");
                }
                let result = [(files(env.root()), opalro_marks(env.root()))];
                if let Err(lost) = nothing_lost(&made, &result) {
                    prop_assert!(false, "{lost}\n{fast_import}");
                }
                opalro_only.borrow_mut().push(format!("{renamed:?}"));
            }
            if kind == 3 {
                // Git followed a move Opalro didn't: each file Opalro names was moved.
                for line in &why {
                    let path = line.trim_start().split_once(": ").unwrap().0;
                    prop_assert!(made.moved(path), "{said}\n{fast_import}");
                }
                git_only
                    .borrow_mut()
                    .push(format!("{} (moves {:?})", why.join(";"), made.moves));
            }
            counts.borrow_mut()[kind] += 1;
            let mut m = moves.borrow_mut();
            for (total, case) in m.iter_mut().zip(made.moves) {
                total.0 += case.0;
                total.1 += case.1;
            }
            Ok(())
        })
        .unwrap();
    let counts = counts.into_inner();
    for (kind, n) in KINDS.iter().zip(counts) {
        println!("{kind}: {n}");
    }
    let [pure, before, after] = moves.into_inner();
    let followed = followed.into_inner();
    println!(
        "files moved (in brackets, edited on the other branch): as they were {} ({}, of which {followed} in cases both merged), edited before the move {} ({}), edited after it {} ({})",
        pure.0, pure.1, before.0, before.1, after.0, after.1
    );
    for why in opalro_only.into_inner() {
        println!(
            "Opalro clean only, the accepted case (a file moved and edited on one branch and \
             removed on the other, which Git pairs with its old self by likeness and reports as \
             rename/delete), nothing either branch made lost: {why}"
        );
    }
    for why in git_only.into_inner() {
        println!("Git clean only: {why}");
    }
    let skipped = skipped.into_inner();
    for why in &skipped {
        println!("both conflict, set aside, nothing either branch made lost: {why}");
    }
    let compared = compared.into_inner();
    println!(
        "both conflict: {compared} compared, {} set aside where Git followed a move",
        skipped.len()
    );
    println!("took {:.1?}", started.elapsed());
    assert!(
        compared >= 16,
        "{compared} cases where both conflict compared"
    );
    // Measured with Git 2.50.1: 57 both clean, 34 both conflict (20 compared, 14 set aside by
    // the earlier, wider rule), 1 Opalro clean only (the accepted case; any other fails), 4 Git
    // clean only, 12 moves followed into a file the other branch edited, 5 files edited before
    // their move and on the other branch. Another Git can pair a few files its own way, so each
    // bound has some room.
    assert!(counts[0] >= 40 && counts[3] <= 8, "{KINDS:?}: {counts:?}");
    assert!(
        followed >= 4,
        "{followed} moves followed into an edited file"
    );
    assert!(
        before.1 >= 1,
        "no file edited before its move and on the other branch"
    );
}

/// The old and new paths of every move Git's rename detection finds on either branch since where
/// they parted, in the stopped merge in `clone`: the moves Git follows.
fn git_renames(clone: &Path) -> BTreeSet<String> {
    let base = text(&git(clone, &["merge-base", "HEAD", "origin/side"]));
    let mut found = BTreeSet::new();
    for tip in ["HEAD", "origin/side"] {
        let listed = git(
            clone,
            &["diff", "--name-status", "-M", "-z", base.trim(), tip],
        );
        let mut fields = listed.split(|&b| b == 0).filter(|f| !f.is_empty());
        while let Some(status) = fields.next() {
            let two = matches!(status.first(), Some(b'R' | b'C'));
            for _ in 0..if two { 2 } else { 1 } {
                let path = text(fields.next().unwrap());
                if status.first() == Some(&b'R') {
                    found.insert(path);
                }
            }
        }
    }
    found
}

/// The files and executable marks Opalro gives, its conflicts kept, when one side is taken
/// everywhere: `--keep mine`, then after `undo`, `--keep theirs`.
fn kept_sides(env: &Env) -> Vec<(Files, BTreeMap<String, bool>)> {
    let mut sides = Vec::new();
    for side in ["mine", "theirs"] {
        if side == "theirs" {
            opalro(env, &["undo"], 0);
        }
        opalro(env, &["resolve", ".", "--keep", side], 0);
        sides.push((files(env.root()), opalro_marks(env.root())));
    }
    sides
}

/// Whether every version of a file either branch made is in one of Opalro's `results`: whole,
/// with its executable mark, or merged line by line (every line it added in one file there, with
/// the mark too where the branch changed it). Lines are unique in the whole history, so a line
/// found is the line that was made.
fn nothing_lost(
    made: &Generated,
    results: &[(Files, BTreeMap<String, bool>)],
) -> Result<(), String> {
    let texts: Vec<(&Vec<u8>, bool)> = (results.iter())
        .flat_map(|(files, marks)| files.iter().map(move |(p, b)| (b, marks[p])))
        .collect();
    for (branch, end) in made.ends.iter().enumerate() {
        for (path, f) in end {
            let was = f.origin.as_ref().and_then(|o| made.first.get(o));
            if was.is_some_and(|w| w.lines == f.lines && w.exec == f.exec) {
                continue;
            }
            let bytes = f.lines.concat().into_bytes();
            if texts.iter().any(|(b, x)| **b == bytes && *x == f.exec) {
                continue;
            }
            let added: Vec<&String> = (f.lines.iter())
                .filter(|l| was.is_none_or(|w| !w.lines.contains(l)))
                .collect();
            let mark_changed = was.is_some_and(|w| w.exec != f.exec);
            let holds = |(b, x): &(&Vec<u8>, bool)| {
                let text = String::from_utf8_lossy(b);
                added.iter().all(|l| text.contains(l.as_str())) && (!mark_changed || *x == f.exec)
            };
            if added.is_empty() || !texts.iter().any(holds) {
                return Err(format!(
                    "branch {branch}'s {path} ({} lines, executable {}) is in none of Opalro's results",
                    f.lines.len(),
                    f.exec
                ));
            }
        }
    }
    Ok(())
}

/// The places in conflict in the save the project in `root` is on.
fn conflict_places(root: &Path) -> BTreeSet<String> {
    let repo = Repository::open(root).unwrap();
    let st = opal_core::status::status(&repo, true, None).unwrap();
    st.conflicts.into_iter().map(|c| c.path).collect()
}

/// What Git's index in `dir` holds: each path's stages, with whether each is executable.
fn stages(dir: &Path) -> BTreeMap<String, BTreeMap<u8, bool>> {
    let listed = git(dir, &["ls-files", "-s", "-z"]);
    let mut found: BTreeMap<String, BTreeMap<u8, bool>> = BTreeMap::new();
    for entry in listed.split(|&b| b == 0).filter(|e| !e.is_empty()) {
        let entry = text(entry);
        let (head, path) = entry.split_once('\t').unwrap();
        // `<mode> <id> <stage>`
        let stage = head.rsplit_once(' ').unwrap().1.parse().unwrap();
        let exec = head.starts_with("100755");
        found
            .entry(path.to_owned())
            .or_default()
            .insert(stage, exec);
    }
    found
}

/// `resolve . --keep mine`, then (after `undo`) `--keep theirs`, against Git's stopped merge in
/// `clone` taking the same side everywhere (`git checkout --ours` or `--theirs` at each path in
/// conflict, and the path removed where that side has none): the same files, contents and
/// executable marks.
fn keep_sides_match_git(env: &Env, clone: &Path, history: &str) -> Result<(), TestCaseError> {
    let index = stages(clone);
    let theirs_clone = tempfile::tempdir().unwrap();
    support::copy_dir(clone, theirs_clone.path());
    for (stage, dir, side) in [(2, clone, "mine"), (3, theirs_clone.path(), "theirs")] {
        let mut marks = BTreeMap::new();
        for (path, held) in &index {
            if let Some(&exec) = held.get(&0) {
                marks.insert(path.clone(), exec);
                continue;
            }
            match held.get(&stage) {
                Some(&exec) => {
                    let which = if stage == 2 { "--ours" } else { "--theirs" };
                    git(dir, &["checkout", which, "--", path]);
                    marks.insert(path.clone(), exec);
                }
                None if dir.join(path).exists() => std::fs::remove_file(dir.join(path)).unwrap(),
                None => {}
            }
        }
        if side == "theirs" {
            opalro(env, &["undo"], 0);
        }
        opalro(env, &["resolve", ".", "--keep", side], 0);
        prop_assert_eq!(files(env.root()), files(dir), "{}: {}", side, history);
        prop_assert_eq!(
            opalro_marks(env.root()),
            marks.clone(),
            "{}: {}",
            side,
            history
        );
        #[cfg(unix)]
        prop_assert_eq!(disk_marks(env.root()), marks, "{}: {}", side, history);
    }
    Ok(())
}

/// Every file Git's stopped merge in `dir` left in its folder, with whether it is executable: the
/// merged entry, or for a file one branch removed, the other's.
fn git_left_marks(dir: &Path) -> BTreeMap<String, bool> {
    let listed = git(dir, &["ls-files", "-s", "-z"]);
    let mut found = BTreeMap::new();
    for entry in listed.split(|&b| b == 0).filter(|e| !e.is_empty()) {
        let entry = text(entry);
        let (head, path) = entry.split_once('\t').unwrap();
        // `<mode> <id> <stage>`; stage 1 is the starting point's.
        if !head.ends_with(" 1") {
            let earlier = found.insert(path.to_owned(), head.starts_with("100755"));
            assert!(earlier.is_none(), "both branches hold {path}: {entry}");
        }
    }
    found
}

/// `opalro args` in `env`, which must exit with `code`; what it said on stderr.
fn opalro(env: &Env, args: &[&str], code: i32) -> String {
    let out = env.opalro().args(args).output().unwrap();
    let said = text(&out.stderr);
    assert_eq!(out.status.code(), Some(code), "{args:?}: {said}");
    said
}

/// A file moved and edited on one line and removed on the other: from Git's history, Opalro sees
/// an old file both lines removed and a new file, and keeps it; Git pairs the two and asks.
#[test]
fn a_file_moved_and_changed_and_removed_on_the_other_line_is_kept_where_git_asks() {
    let env = Env::new();
    env.commit("a.txt", "1\n2\n3\n", "First");
    env.git(&["checkout", "-q", "-b", "side"]);
    env.git(&["rm", "-q", "a.txt"]);
    env.git(&["commit", "-q", "-m", "Side removes a.txt"]);
    env.git(&["checkout", "-q", "main"]);
    env.git(&["mv", "a.txt", "b.txt"]);
    env.git(&["commit", "-q", "-m", "Main moves a.txt"]);
    env.commit("b.txt", "one\n2\n3\n", "Main edits b.txt");
    env.import();
    let clone = tempfile::tempdir().unwrap();
    git(
        env.root(),
        &["clone", "-q", ".", clone.path().to_str().unwrap()],
    );

    opalro(&env, &["combine", "side", "--only-if-clean"], 0);
    assert_eq!(
        files(env.root()),
        support::holding(&[("b.txt", "one\n2\n3\n")])
    );
    let theirs = git_with_input(
        clone.path(),
        &[
            "-c",
            "merge.directoryRenames=false",
            "-c",
            "merge.renames=true",
            "merge",
            "--no-edit",
            "origin/side",
        ],
        b"",
    );
    assert_eq!(theirs.status.code(), Some(1));
    assert!(
        text(&theirs.stdout).contains("CONFLICT (rename/delete): a.txt renamed to b.txt in HEAD"),
        "{}",
        text(&theirs.stdout)
    );
}

/// The same made in Opalro: a move saved by itself is noted, and the combine stops as Git does; a
/// move saved with an edit has no note, and the edited file is kept.
#[test]
fn a_move_saved_by_itself_is_followed_and_one_saved_with_an_edit_is_not() {
    for apart in [true, false] {
        let env = Env::empty();
        opalro(&env, &["start", "--preset", "custom"], 0);
        env.overwrite("a.txt", "1\n2\n3\n");
        opalro(&env, &["save", "First"], 0);
        opalro(&env, &["line", "new", "side"], 0);
        std::fs::remove_file(env.root().join("a.txt")).unwrap();
        opalro(&env, &["save", "Side removes a.txt"], 0);
        opalro(&env, &["goto", "main"], 0);
        std::fs::rename(env.root().join("a.txt"), env.root().join("b.txt")).unwrap();
        if apart {
            opalro(&env, &["save", "Main moves a.txt"], 0);
        }
        env.overwrite("b.txt", "one\n2\n3\n");
        opalro(&env, &["save", "Main edits b.txt"], 0);
        if apart {
            let said = opalro(&env, &["combine", "side", "--only-if-clean"], 1);
            assert!(
                said.contains("\n  b.txt: moved here from a.txt on main, removed on side"),
                "{said}"
            );
        } else {
            let (main, side) = (main_tip(&env), env.state().1.lines["side"]);
            opalro(&env, &["combine", "side", "--only-if-clean"], 0);
            let repo = Repository::open(env.root()).unwrap();
            let combined = repo.store().save(&main_tip(&env), false).unwrap();
            assert_eq!(combined.parents, [main, side]);
            assert!(!env.root().join("a.txt").exists());
            assert_eq!(
                std::fs::read(env.root().join("b.txt")).unwrap(),
                b"one\n2\n3\n"
            );
        }
    }
}
