//! `opalro who` against `git blame`, on histories drawn at random from a fixed seed.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant;

use proptest::prelude::*;
use proptest::sample::Index;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};
use support::{Env, git, git_with_input, text};

/// Every path a generated history may use; no folder is named like a file.
const PATHS: [&str; 10] = [
    "a.txt",
    "b.txt",
    "c.txt",
    "d.md",
    "src/e.txt",
    "src/f.txt",
    "src/deep/g.txt",
    "doc/h.md",
    "doc/i.md",
    "j.sh",
];

/// The branches merged into `main`, one a round.
const BRANCHES: [&str; 3] = ["side", "feature-1", "feature-2"];

/// One change in a commit; each `Index` picks among what is there when the commit is made.
#[derive(Clone, Debug)]
enum Step {
    /// In one file, replace (0), take out (1) or add (2) a run of 1 to 3 lines, or put back (3)
    /// a line the file had and lost.
    Edit(Index, Vec<(u8, Index, usize)>),
    /// A new file of 1 to 8 lines at a free path.
    Add(Index, usize),
    Remove(Index),
    /// A file moved to a free path, the commit's only change.
    Move(Index, Index),
}

/// A round: who may edit each file, `main`'s commits, then the branch's, then the merge.
#[derive(Clone, Debug)]
struct Round {
    owners: Vec<(u8, Index)>,
    main: Vec<Vec<Step>>,
    branch: Vec<Vec<Step>>,
}

/// A history made in Git: a first commit, then rounds that each fork a branch and merge it.
#[derive(Clone, Debug)]
struct Merged {
    first: Vec<(String, usize)>,
    rounds: Vec<Round>,
}

/// A history of one line, with moves, made with both programs' own commands.
#[derive(Clone, Debug)]
struct Linear {
    first: Vec<(String, usize)>,
    commits: Vec<Vec<Step>>,
}

#[derive(Clone, Debug, Default)]
struct File {
    lines: Vec<String>,
    /// Lines taken out, with the round they went in, which may be put back.
    gone: Vec<(String, usize)>,
}

type Tree = BTreeMap<String, File>;

/// Who may edit a file in a round: `main`, the branch, or each its own end of the file, either
/// side of three lines (named by the first and last) neither touches. So no merge conflicts.
#[derive(Clone, Debug)]
enum Owner {
    Main,
    Branch,
    Split(String, String),
}

fn first() -> impl Strategy<Value = Vec<(String, usize)>> {
    let paths = PATHS.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>();
    let sizes = prop::collection::vec(2..=10usize, 6);
    (prop::sample::subsequence(paths, 2..=6), sizes)
        .prop_map(|(paths, sizes)| paths.into_iter().zip(sizes).collect())
}

fn commit() -> impl Strategy<Value = Vec<Step>> {
    let edits = prop::collection::vec((0..4u8, any::<Index>(), 1..=3usize), 1..=3);
    let step = prop_oneof![
        4 => (any::<Index>(), edits).prop_map(|(f, e)| Step::Edit(f, e)),
        1 => (any::<Index>(), 1..=8usize).prop_map(|(at, n)| Step::Add(at, n)),
        1 => any::<Index>().prop_map(Step::Remove),
    ];
    prop::collection::vec(step, 1..=3)
}

fn merged_history() -> impl Strategy<Value = Merged> {
    let round = (
        prop::collection::vec((0..3u8, any::<Index>()), 10),
        prop::collection::vec(commit(), 0..=2),
        prop::collection::vec(commit(), 1..=3),
    )
        .prop_map(|(owners, main, branch)| Round {
            owners,
            main,
            branch,
        });
    (first(), prop::collection::vec(round, 1..=3))
        .prop_map(|(first, rounds)| Merged { first, rounds })
}

fn linear_history() -> impl Strategy<Value = Linear> {
    let commit = prop_oneof![
        3 => commit(),
        2 => (any::<Index>(), any::<Index>()).prop_map(|(f, t)| vec![Step::Move(f, t)]),
    ];
    (first(), prop::collection::vec(commit, 2..=6))
        .prop_map(|(first, commits)| Linear { first, commits })
}

/// The first commit's files, of lines no other commit writes.
fn first_tree(first: &[(String, usize)]) -> Tree {
    (first.iter().enumerate())
        .map(|(i, (path, n))| {
            let lines = (0..*n).map(|j| format!("file {i} line {j}\n")).collect();
            let file = File {
                lines,
                gone: Vec::new(),
            };
            (path.clone(), file)
        })
        .collect()
}

/// The lines of `file` that `on_main` (or the branch) may edit, if any.
fn region(file: &File, owner: &Owner, on_main: bool) -> Option<(usize, usize)> {
    let at = |line: &String| file.lines.iter().position(|l| l == line).unwrap();
    match (owner, on_main) {
        (Owner::Main, true) | (Owner::Branch, false) => Some((0, file.lines.len())),
        (Owner::Split(first, _), true) => Some((0, at(first))),
        (Owner::Split(_, last), false) => Some((at(last) + 1, file.lines.len())),
        _ => None,
    }
}

/// What making a commit drew: lines put back, and the move, if it was one.
#[derive(Default)]
struct Made {
    put_back: usize,
    moved: Option<(String, String)>,
}

/// Makes `steps` on `tree` as `main` or the branch, in `round`. New lines are `<who> <n>`, unique
/// in the whole history; a line put back is one this file lost in an earlier commit, and that
/// neither side can hold at the merge.
#[allow(clippy::too_many_arguments)]
fn make(
    tree: &mut Tree,
    owners: &mut BTreeMap<String, Owner>,
    steps: &[Step],
    on_main: bool,
    round: usize,
    who: &str,
    fresh: &mut usize,
) -> Made {
    let mut made = Made::default();
    let earlier: BTreeSet<String> = (tree.values())
        .flat_map(|f| f.gone.iter().map(|(l, _)| l.clone()))
        .collect();
    let mut new_line = || {
        *fresh += 1;
        format!("{who} {fresh}\n")
    };
    let mine = if on_main { Owner::Main } else { Owner::Branch };
    let whole = |owner: &Owner| {
        matches!(
            (owner, on_main),
            (Owner::Main, true) | (Owner::Branch, false)
        )
    };
    for step in steps {
        match step {
            Step::Edit(at, edits) => {
                let editable: Vec<String> = (tree.iter())
                    .filter(|(p, f)| region(f, &owners[*p], on_main).is_some())
                    .map(|(p, _)| p.clone())
                    .collect();
                if editable.is_empty() {
                    continue;
                }
                let path = &editable[at.index(editable.len())];
                let owner = &owners[path];
                let file = tree.get_mut(path).unwrap();
                for (kind, at, count) in edits {
                    let (lo, hi) = region(file, owner, on_main).unwrap();
                    let place = lo + at.index(hi - lo + 1);
                    let mut gone = (*count).min(hi - place);
                    match kind {
                        0 | 1 => {
                            // Never the last line of a whole file: an empty file has no lines.
                            if *kind == 1 && whole(owner) {
                                gone = gone.min(file.lines.len() - 1);
                            }
                            let new: Vec<String> = if *kind == 0 {
                                (0..*count).map(|_| new_line()).collect()
                            } else {
                                Vec::new()
                            };
                            let out: Vec<String> =
                                file.lines.splice(place..place + gone, new).collect();
                            file.gone.extend(out.into_iter().map(|l| (l, round)));
                        }
                        2 => {
                            let new: Vec<String> = (0..*count).map(|_| new_line()).collect();
                            file.lines.splice(place..place, new);
                        }
                        _ => {
                            // A line lost in this commit would come back moved, and a line the
                            // branch lost this round is still where it was in `main`'s parent of
                            // the merge: both have two right answers. Into a shared file only
                            // `main` puts back, so the two sides can't both put one line back.
                            let can: Vec<usize> = (file.gone.iter().enumerate())
                                .filter(|(_, (l, r))| {
                                    earlier.contains(l) && (on_main || whole(owner) && *r < round)
                                })
                                .map(|(i, _)| i)
                                .collect();
                            if !can.is_empty() {
                                let (line, _) = file.gone.remove(can[at.index(can.len())]);
                                file.lines.insert(place, line);
                                made.put_back += 1;
                            }
                        }
                    }
                }
            }
            Step::Add(at, n) => {
                let free: Vec<&str> = (PATHS.iter().copied())
                    .filter(|p| !owners.contains_key(*p) && !tree.contains_key(*p))
                    .collect();
                if free.is_empty() {
                    continue;
                }
                let path = free[at.index(free.len())].to_owned();
                let lines = (0..*n).map(|_| new_line()).collect();
                let file = File {
                    lines,
                    gone: Vec::new(),
                };
                tree.insert(path.clone(), file);
                owners.insert(path, mine.clone());
            }
            Step::Remove(at) => {
                let whole_files: Vec<String> = (tree.keys())
                    .filter(|p| whole(&owners[*p]))
                    .cloned()
                    .collect();
                if !whole_files.is_empty() {
                    tree.remove(&whole_files[at.index(whole_files.len())]);
                }
            }
            Step::Move(from, to) => {
                let free: Vec<&str> = (PATHS.iter().copied())
                    .filter(|p| !tree.contains_key(*p))
                    .collect();
                if tree.is_empty() || free.is_empty() {
                    continue;
                }
                let from = tree.keys().nth(from.index(tree.len())).unwrap().clone();
                let to = free[to.index(free.len())].to_owned();
                let file = tree.remove(&from).unwrap();
                tree.insert(to.clone(), file);
                made.moved = Some((from, to));
            }
        }
    }
    made
}

/// `main`'s and the branch's ends merged: each file as its owner left it, a shared one with
/// `main`'s end before the three lines and the branch's after. Also whether the shared file took
/// changes from both sides.
fn merge(main: &Tree, side: &Tree, base: &Tree, owners: &BTreeMap<String, Owner>) -> (Tree, usize) {
    let mut merged = Tree::new();
    let mut mixed = 0;
    for (path, owner) in owners {
        let file = match owner {
            Owner::Main => main.get(path).cloned(),
            Owner::Branch => side.get(path).cloned(),
            Owner::Split(first, last) => {
                let (m, s, b) = (&main[path], &side[path], &base[path]);
                let at = |f: &File, line: &String| f.lines.iter().position(|l| l == line).unwrap();
                let (head, tail) = (at(m, first), at(s, last) + 1);
                let mut lines = m.lines[..=at(m, last)].to_vec();
                lines.extend_from_slice(&s.lines[tail..]);
                let mut gone: Vec<(String, usize)> = Vec::new();
                for g in m.gone.iter().chain(&s.gone) {
                    if !lines.contains(&g.0) && !gone.iter().any(|h| h.0 == g.0) {
                        gone.push(g.clone());
                    }
                }
                let base_head = &b.lines[..at(b, first)];
                let base_tail = &b.lines[at(b, last) + 1..];
                mixed +=
                    usize::from(m.lines[..head] != *base_head && s.lines[tail..] != *base_tail);
                Some(File { lines, gone })
            }
        };
        if let Some(file) = file {
            merged.insert(path.clone(), file);
        }
    }
    (merged, mixed)
}

/// A commit for `git fast-import`, from what changed between `before` and `after`; its message,
/// `<mark> on <branch>`, is the only one of its kind. `from` (then `merge`) are its parents, when
/// it doesn't just follow its branch.
fn commit_text(branch: &str, mark: u32, parents: &[u32], before: &Tree, after: &Tree) -> String {
    let who = format!("Ada <ada@example.com> {} +0000", 1_700_000_000 + mark);
    let message = format!("{mark} on {branch}\n");
    let mut out = format!(
        "commit refs/heads/{branch}\nmark :{mark}\nauthor {who}\ncommitter {who}\ndata {}\n{message}",
        message.len()
    );
    for (i, p) in parents.iter().enumerate() {
        out += &format!("{} :{p}\n", if i == 0 { "from" } else { "merge" });
    }
    for path in before.keys().filter(|p| !after.contains_key(*p)) {
        out += &format!("D {path}\n");
    }
    for (path, file) in after {
        if before.get(path).map(|b| &b.lines) != Some(&file.lines) {
            let data = file.lines.concat();
            out += &format!("M 100644 inline {path}\ndata {}\n{data}\n", data.len());
        }
    }
    out
}

/// A generated history written for `git fast-import`: the stream, each tip's files (`main`'s
/// first), and what the drawing counted.
struct Generated {
    stream: String,
    tips: Vec<(&'static str, Tree)>,
    put_back: usize,
    mixed: usize,
}

fn generate(h: &Merged) -> Generated {
    let mut fresh = 0;
    let (mut put_back, mut mixed) = (0, 0);
    let mut main = first_tree(&h.first);
    let mut stream = commit_text("main", 1, &[], &Tree::new(), &main);
    let (mut mark, mut main_mark) = (1, 1);
    let mut tips = Vec::new();
    for (r, round) in h.rounds.iter().enumerate() {
        let branch = BRANCHES[r];
        let base = main.clone();
        let fork = main_mark;
        let mut owners: BTreeMap<String, Owner> = (base.iter().enumerate())
            .map(|(i, (path, f))| {
                let (kind, at) = &round.owners[i % round.owners.len()];
                let owner = match kind {
                    0 => Owner::Main,
                    1 => Owner::Branch,
                    _ if f.lines.len() >= 3 => {
                        let s = at.index(f.lines.len() - 2);
                        Owner::Split(f.lines[s].clone(), f.lines[s + 2].clone())
                    }
                    _ => Owner::Main,
                };
                (path.clone(), owner)
            })
            .collect();
        for steps in &round.main {
            let before = main.clone();
            put_back += make(&mut main, &mut owners, steps, true, r, "main", &mut fresh).put_back;
            mark += 1;
            stream += &commit_text("main", mark, &[], &before, &main);
            main_mark = mark;
        }
        let mut side = base.clone();
        for (i, steps) in round.branch.iter().enumerate() {
            let before = side.clone();
            put_back += make(&mut side, &mut owners, steps, false, r, branch, &mut fresh).put_back;
            mark += 1;
            let from = if i == 0 { vec![fork] } else { vec![] };
            stream += &commit_text(branch, mark, &from, &before, &side);
        }
        let (merged, both) = merge(&main, &side, &base, &owners);
        mixed += both;
        mark += 1;
        stream += &commit_text("main", mark, &[main_mark, mark - 1], &main, &merged);
        main_mark = mark;
        main = merged;
        tips.push((branch, side));
    }
    tips.insert(0, ("main", main));
    Generated {
        stream,
        tips,
        put_back,
        mixed,
    }
}

/// Every merge on `main` holds what `git merge-tree` makes of its two parents: the drawn merges
/// are clean, and are Git's own. Returns how many there are.
fn merges_are_gits(dir: &Path) -> usize {
    let listed = text(&git(dir, &["log", "--merges", "--format=%T %P", "main"]));
    for merge in listed.lines() {
        let [tree, ours, theirs] =
            <[&str; 3]>::try_from(merge.split(' ').collect::<Vec<_>>()).unwrap();
        let made = text(&git(dir, &["merge-tree", "--write-tree", ours, theirs]));
        assert_eq!(made.lines().next(), Some(tree), "{merge}");
    }
    listed.lines().count()
}

/// Each line of `path` at `tip` as `who` names it: the first line of its save's message, and the
/// path the file had there. Also the whole document.
fn who(env: &Env, tip: &str, path: &str) -> (Vec<(String, String)>, String) {
    let out = (env.opalro())
        .args(["who", path, "--save", tip, "--json"])
        .output()
        .unwrap();
    let said = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let doc: serde_json::Value = serde_json::from_str(&said).unwrap();
    assert_eq!(doc["coarse"], false, "{said}");
    let messages: BTreeMap<&str, &str> = (doc["saves"].as_array().unwrap().iter())
        .map(|s| (s["id"].as_str().unwrap(), s["message"].as_str().unwrap()))
        .collect();
    let mut lines = Vec::new();
    for run in doc["runs"].as_array().unwrap() {
        let save = run["save"].as_str().unwrap_or_else(|| panic!("{said}"));
        let message = messages[save].lines().next().unwrap_or_default().to_owned();
        let was = run["was"].as_str().unwrap_or(path).to_owned();
        for _ in 0..run["lines"].as_u64().unwrap() {
            lines.push((message.clone(), was.clone()));
        }
    }
    (lines, said)
}

/// Each line of `path` at `tip` as `git blame` names it: its commit's summary and the path the
/// file had there.
fn blamed(dir: &Path, tip: &str, path: &str) -> Vec<(String, String)> {
    let out = text(&git(dir, &["blame", "--line-porcelain", tip, "--", path]));
    let (mut summary, mut filename) = ("", "");
    let mut lines = Vec::new();
    for l in out.lines() {
        if l.starts_with('\t') {
            lines.push((summary.to_owned(), filename.to_owned()));
        } else if let Some(s) = l.strip_prefix("summary ") {
            summary = s;
        } else if let Some(f) = l.strip_prefix("filename ") {
            filename = f;
        }
    }
    lines
}

#[derive(Debug, Default)]
struct Counts {
    histories: usize,
    files: usize,
    lines: usize,
    /// Lines `main` has from a branch, through a merge's second parent, and histories with one.
    branch_lines: usize,
    branch_histories: usize,
    /// Lines named at the path the file had before a move, and histories with one.
    moved_lines: usize,
    moved_histories: usize,
}

/// Every file of `tree`, at `tip`: each line's save in `env` must be the commit `git blame` names
/// in `dir`, by message and by the path the file had there.
fn compare(
    env: &Env,
    dir: &Path,
    tip: &str,
    tree: &Tree,
    history: &str,
    counts: &mut Counts,
) -> Result<(usize, usize), TestCaseError> {
    let (mut branch, mut moved) = (0, 0);
    for (path, file) in tree {
        let (ours, doc) = who(env, tip, path);
        let theirs = blamed(dir, tip, path);
        prop_assert_eq!(theirs.len(), file.lines.len(), "{} at {}", path, tip);
        prop_assert_eq!(
            &ours,
            &theirs,
            "{} at {}:\nwho: {}\ngit blame: {:?}\n{}",
            path,
            tip,
            doc,
            theirs,
            history
        );
        counts.files += 1;
        counts.lines += ours.len();
        if tip == "main" {
            branch += ours
                .iter()
                .filter(|(m, _)| !m.ends_with(" on main"))
                .count();
        }
        moved += ours.iter().filter(|(_, was)| was != path).count();
    }
    Ok((branch, moved))
}

/// Histories made in Git and imported, then histories with moves made in both programs: for every
/// file at every tip, each line's save is the commit `git blame` names, found by its message (no
/// two commits share one) and the path the file had there. Every line written is a text no other
/// line has, so there is one right answer for each, whatever comparison either program makes; a
/// line put back after it was taken out is the later commit's in both. Merges conflict nowhere:
/// the two sides edit different files, or different ends of a file either side of three lines
/// neither touches, and each merge holds what `git merge-tree` makes of its parents.
///
/// An imported history carries no move notes, so Opalro follows no move in it while Git's `blame`
/// follows a whole-file move by likeness. The moved files are therefore made with Opalro's own
/// commands (each move saved by itself, which notes it) in a project of one line, and the same
/// steps committed in a Git repository beside it. Not drawn: a move with an edit in the same
/// commit (Git guesses and follows, Opalro doesn't), a line repeated in a file or moved within it
/// (two right answers), and a file changed under its old name on a branch merged into one that
/// moved it (Opalro's known limit, pinned by the engine's own tests). The seed is fixed.
#[test]
fn who_matches_git_blame_on_unique_lines() {
    let started = Instant::now();
    let rng = TestRng::deterministic_rng(RngAlgorithm::ChaCha);
    let config = |cases| Config {
        cases,
        failure_persistence: None,
        ..Config::default()
    };
    let imported = std::cell::RefCell::new(Counts::default());
    let (put_back, mixed, merges) = (
        std::cell::Cell::new(0),
        std::cell::Cell::new(0),
        std::cell::Cell::new(0),
    );
    TestRunner::new_with_rng(config(64), rng)
        .run(&merged_history(), |h| {
            let made = generate(&h);
            let env = Env::new();
            let out = git_with_input(
                env.root(),
                &["fast-import", "--quiet"],
                made.stream.as_bytes(),
            );
            assert!(out.status.success(), "{}", text(&out.stderr));
            merges.set(merges.get() + merges_are_gits(env.root()));
            env.import();
            let counts = &mut *imported.borrow_mut();
            let (mut branch, mut moved) = (0, 0);
            for (tip, tree) in &made.tips {
                let (b, m) = compare(&env, env.root(), tip, tree, &made.stream, counts)?;
                (branch, moved) = (branch + b, moved + m);
            }
            prop_assert_eq!(moved, 0, "a move in a history from Git");
            counts.histories += 1;
            counts.branch_lines += branch;
            counts.branch_histories += usize::from(branch > 0);
            put_back.set(put_back.get() + made.put_back);
            mixed.set(mixed.get() + made.mixed);
            Ok(())
        })
        .unwrap();

    let mirrored = std::cell::RefCell::new(Counts::default());
    let rng = TestRng::deterministic_rng(RngAlgorithm::ChaCha);
    TestRunner::new_with_rng(config(24), rng)
        .run(&linear_history(), |h| {
            let (ours, theirs, tree, steps) = mirror(&h);
            let counts = &mut *mirrored.borrow_mut();
            let (_, moved) = compare(&ours, theirs.root(), "main", &tree, &steps, counts)?;
            counts.histories += 1;
            counts.moved_lines += moved;
            counts.moved_histories += usize::from(moved > 0);
            Ok(())
        })
        .unwrap();

    let (imported, mirrored) = (imported.into_inner(), mirrored.into_inner());
    println!(
        "made in Git and imported: {} lines in {} files of {} histories compared at every tip; \
         at main, {} lines from a branch through a merge's second parent, in {} histories; {} \
         merges, each Git's own, holding {} files both sides changed; {} lines put back",
        imported.lines,
        imported.files,
        imported.histories,
        imported.branch_lines,
        imported.branch_histories,
        merges.get(),
        mixed.get(),
        put_back.get()
    );
    println!(
        "made in both with moves: {} lines in {} files of {} histories compared; {} lines named \
         where the file was before a move, in {} histories",
        mirrored.lines,
        mirrored.files,
        mirrored.histories,
        mirrored.moved_lines,
        mirrored.moved_histories
    );
    println!("took {:.1?}", started.elapsed());
    assert!(imported.lines > 0 && mirrored.lines > 0, "compared no line");
    assert!(
        imported.branch_histories >= 3,
        "{} histories with a line through a merge's second parent",
        imported.branch_histories
    );
    assert!(
        mirrored.moved_histories >= 3,
        "{} histories with a line named through a move",
        mirrored.moved_histories
    );
    assert!(
        mixed.get() >= 3,
        "{} files both sides changed in a merge",
        mixed.get()
    );
    assert!(put_back.get() >= 3, "{} lines put back", put_back.get());
}

/// The history `h` made twice: in an Opalro project with `opalro save`, and in a Git repository
/// with `git commit`, the same files and moves each time. Returns the project, the repository,
/// the files at the end and the steps taken, for a failure to show.
fn mirror(h: &Linear) -> (Env, Env, Tree, String) {
    let ours = Env::empty();
    let started = ours
        .opalro()
        .args(["start", "--preset", "custom"])
        .output()
        .unwrap();
    assert_eq!(started.status.code(), Some(0), "{}", text(&started.stderr));
    let theirs = Env::new();
    let mut tree = Tree::new();
    let mut fresh = 0;
    let mut steps = String::new();
    let firsts = first_tree(&h.first);
    let commits = std::iter::once(None).chain(h.commits.iter().map(Some));
    for (n, commit) in commits.enumerate() {
        let before = tree.clone();
        let mut moved = None;
        match commit {
            None => tree = firsts.clone(),
            Some(commit) => {
                let mut owners = tree.keys().map(|p| (p.clone(), Owner::Main)).collect();
                moved = make(&mut tree, &mut owners, commit, true, 0, "main", &mut fresh).moved;
            }
        }
        let same = |a: &Tree, b: &Tree| {
            a.iter()
                .map(|(p, f)| (p, &f.lines))
                .eq(b.iter().map(|(p, f)| (p, &f.lines)))
        };
        if same(&before, &tree) {
            continue;
        }
        for dir in [ours.root(), theirs.root()] {
            if let Some((from, to)) = &moved {
                std::fs::create_dir_all(dir.join(to).parent().unwrap()).unwrap();
                std::fs::rename(dir.join(from), dir.join(to)).unwrap();
                continue;
            }
            for path in before.keys().filter(|p| !tree.contains_key(*p)) {
                std::fs::remove_file(dir.join(path)).unwrap();
            }
            for (path, file) in &tree {
                if before.get(path).map(|b| &b.lines) != Some(&file.lines) {
                    std::fs::create_dir_all(dir.join(path).parent().unwrap()).unwrap();
                    std::fs::write(dir.join(path), file.lines.concat()).unwrap();
                }
            }
        }
        let message = format!("{n} on main");
        let saved = ours.save(&[&message]);
        assert_eq!(saved.status.code(), Some(0), "{}", text(&saved.stderr));
        theirs.git(&["add", "-A"]);
        theirs.git(&["commit", "-q", "-m", &message]);
        steps += &match &moved {
            Some((from, to)) => format!("{message}: {from} moved to {to}\n"),
            None => format!(
                "{message}: {:?}\n",
                tree.iter().map(|(p, f)| (p, &f.lines)).collect::<Vec<_>>()
            ),
        };
    }
    (ours, theirs, tree, steps)
}
