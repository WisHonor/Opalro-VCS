//! `opalro who` against `git blame`, on histories drawn at random from a fixed seed, and on a few
//! fixed ones.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::cell::RefCell;
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

/// The branches merged into `main`, one a round, and the third parents of a round's merge.
const BRANCHES: [&str; 3] = ["side", "feature-1", "feature-2"];
const THIRDS: [&str; 3] = ["extra-1", "extra-2", "extra-3"];

/// One change in a commit; each `Index` picks among what is there when the commit is made.
#[derive(Clone, Debug)]
enum Step {
    /// In one file, replace (0), take out (1) or add (2) a run of 1 to 3 lines, or put back (3)
    /// a line the file had and lost.
    Edit(Index, Vec<(u8, Index, usize)>),
    /// A new file of 0 to 8 lines at a free path.
    Add(Index, usize),
    Remove(Index),
    /// Every line of a file taken out.
    Empty(Index),
    /// A file moved to a free path, the commit's only change.
    Move(Index, Index),
}

/// A round: who may edit each file, `main`'s commits, then the branch's, then the merge.
#[derive(Clone, Debug)]
struct Round {
    owners: Vec<(u8, Index)>,
    main: Vec<Vec<Step>>,
    branch: Vec<Vec<Step>>,
    /// In the first shared file, 1: both sides add the same line at the same place; 2: both
    /// change its middle line, and each merge holds a third text.
    both: u8,
    /// A line the merge writes, in a file and at a place it picks.
    merge_line: Option<(Index, Index)>,
    /// A third parent: a branch of one commit that adds a file of so many lines.
    third: Option<(Index, usize)>,
    /// Halfway, `main` merges the branch and the branch merges `main`, before the last merge.
    criss: bool,
}

/// A history made in Git: a first commit, rounds that each fork a branch and merge it, and how
/// each commit's dates step from the last one's.
#[derive(Clone, Debug)]
struct Merged {
    first: Vec<(String, usize)>,
    rounds: Vec<Round>,
    dates: Vec<(u8, u8)>,
}

/// A history made with both programs' commands: a line of commits with moves, then maybe a
/// branch that is merged.
#[derive(Clone, Debug)]
struct Linear {
    first: Vec<(String, usize)>,
    commits: Vec<Vec<Step>>,
    fork: Option<Fork>,
}

/// Who owns each file, the branch's commits, then `main`'s, then the merge.
#[derive(Clone, Debug)]
struct Fork {
    owners: Vec<bool>,
    side: Vec<Vec<Step>>,
    main: Vec<Vec<Step>>,
}

#[derive(Clone, Debug, Default)]
struct File {
    lines: Vec<String>,
    /// Lines taken out, with the round they went in, which may be put back.
    gone: Vec<(String, usize)>,
    /// Was empty once, while it had had lines.
    emptied: bool,
    moved: bool,
    /// Moved on a branch that is merged.
    moved_on_branch: bool,
    edited_after_move: bool,
}

type Tree = BTreeMap<String, File>;

/// Each line's commit (by the first line of its message) and the path the file had there.
type Named = Vec<(String, String)>;

/// Who may edit a file in a round: `main`, the branch, the third parent, or each its own end of
/// the file, either side of three lines (named by the first and last) neither touches. So no
/// merge conflicts.
#[derive(Clone, Debug)]
enum Owner {
    Main,
    Branch,
    Third,
    Split(String, String),
}

fn first() -> impl Strategy<Value = Vec<(String, usize)>> {
    let paths = PATHS.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>();
    let sizes = prop::collection::vec(2..=10usize, 6);
    (prop::sample::subsequence(paths, 2..=6), sizes)
        .prop_map(|(paths, sizes)| paths.into_iter().zip(sizes).collect())
}

fn commit(smallest: usize) -> impl Strategy<Value = Vec<Step>> {
    let edits = prop::collection::vec((0..4u8, any::<Index>(), 1..=3usize), 1..=3);
    let step = prop_oneof![
        4 => (any::<Index>(), edits).prop_map(|(f, e)| Step::Edit(f, e)),
        1 => (any::<Index>(), smallest..=8usize).prop_map(|(at, n)| Step::Add(at, n)),
        1 => any::<Index>().prop_map(Step::Remove),
        1 => any::<Index>().prop_map(Step::Empty),
    ];
    prop::collection::vec(step, 1..=3)
}

fn merged_history() -> impl Strategy<Value = Merged> {
    let round = (
        prop::collection::vec((0..3u8, any::<Index>()), 10),
        prop::collection::vec(commit(0), 0..=2),
        prop::collection::vec(commit(0), 1..=3),
        0..3u8,
        prop::option::weighted(0.3, (any::<Index>(), any::<Index>())),
        prop::option::weighted(0.2, (any::<Index>(), 1..=4usize)),
        prop::bool::weighted(0.2),
    )
        .prop_map(
            |(owners, main, branch, both, merge_line, third, criss)| Round {
                owners,
                main,
                branch,
                both,
                merge_line,
                third,
                criss,
            },
        );
    (
        first(),
        prop::collection::vec(round, 1..=3),
        prop::collection::vec((0..3u8, 0..3u8), 16),
    )
        .prop_map(|(first, rounds, dates)| Merged {
            first,
            rounds,
            dates,
        })
}

/// A commit of the histories made in both programs: a move, one time in `moves` of 5.
fn moving_commit(moves: u32) -> impl Strategy<Value = Vec<Step>> {
    prop_oneof![
        5 - moves => commit(1),
        moves => (any::<Index>(), any::<Index>()).prop_map(|(f, t)| vec![Step::Move(f, t)]),
    ]
}

fn linear_history() -> impl Strategy<Value = Linear> {
    let fork = (
        prop::collection::vec(any::<bool>(), 10),
        prop::collection::vec(moving_commit(3), 1..=3),
        prop::collection::vec(moving_commit(2), 0..=2),
    )
        .prop_map(|(owners, side, main)| Fork { owners, side, main });
    (
        first(),
        prop::collection::vec(moving_commit(2), 2..=5),
        prop::option::weighted(0.85, fork),
    )
        .prop_map(|(first, commits, fork)| Linear {
            first,
            commits,
            fork,
        })
}

/// The first commit's files, of lines no other commit writes.
fn first_tree(first: &[(String, usize)]) -> Tree {
    (first.iter().enumerate())
        .map(|(i, (path, n))| {
            let lines = (0..*n).map(|j| format!("file {i} line {j}\n")).collect();
            let file = File {
                lines,
                ..File::default()
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

/// Makes `steps` on `tree` as `main` or the branch, in `round`; returns the lines it put back.
/// New lines are `<who> <n>`, unique in the whole history. A line put back is one this file lost
/// in an earlier commit (back in the same one, it would be a line moved within the file). The
/// branch puts back only a line lost in an earlier round, which neither side holds at the merge;
/// a line `main` puts back may still be in the branch, and the merge sees to that.
fn make(
    tree: &mut Tree,
    owners: &mut BTreeMap<String, Owner>,
    steps: &[Step],
    (on_main, round): (bool, usize),
    who: &str,
    fresh: &mut usize,
) -> Vec<String> {
    let mut put_back = Vec::new();
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
                let was = file.lines.clone();
                for (kind, at, count) in edits {
                    let (lo, hi) = region(file, owner, on_main).unwrap();
                    let place = lo + at.index(hi - lo + 1);
                    let gone = (*count).min(hi - place);
                    match kind {
                        0 | 1 => {
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
                            // A line the branch lost this round is still where it was in
                            // `main`'s parent of the merge. Into a shared file only `main` puts
                            // back, so the two sides can't both put one line back.
                            let can: Vec<usize> = (file.gone.iter().enumerate())
                                .filter(|(_, (l, r))| {
                                    earlier.contains(l) && (on_main || whole(owner) && *r < round)
                                })
                                .map(|(i, _)| i)
                                .collect();
                            if !can.is_empty() {
                                let (line, _) = file.gone.remove(can[at.index(can.len())]);
                                file.lines.insert(place, line.clone());
                                put_back.push(line);
                            }
                        }
                    }
                    file.emptied |= file.lines.is_empty() && !was.is_empty();
                }
                file.edited_after_move |= file.moved && file.lines != was;
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
                    ..File::default()
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
            Step::Empty(at) => {
                let whole_files: Vec<String> = (tree.iter())
                    .filter(|(p, f)| whole(&owners[*p]) && !f.lines.is_empty())
                    .map(|(p, _)| p.clone())
                    .collect();
                if !whole_files.is_empty() {
                    let path = &whole_files[at.index(whole_files.len())];
                    let file = tree.get_mut(path).unwrap();
                    let out = std::mem::take(&mut file.lines);
                    file.gone.extend(out.into_iter().map(|l| (l, round)));
                    file.emptied = true;
                }
            }
            Step::Move(from, to) => {
                let whole_files: Vec<String> = (tree.keys())
                    .filter(|p| whole(&owners[*p]))
                    .cloned()
                    .collect();
                let free: Vec<&str> = (PATHS.iter().copied())
                    .filter(|p| !owners.contains_key(*p) && !tree.contains_key(*p))
                    .collect();
                if whole_files.is_empty() || free.is_empty() {
                    continue;
                }
                let from = &whole_files[from.index(whole_files.len())];
                let to = free[to.index(free.len())].to_owned();
                let mut file = tree.remove(from).unwrap();
                file.moved = true;
                file.moved_on_branch |= !on_main;
                tree.insert(to.clone(), file);
                owners.insert(to, mine.clone());
            }
        }
    }
    put_back
}

/// Each file as its owner left it in `main`, the branch and the third parent; a shared one with
/// `main`'s end, the three lines as `main` has them, and the branch's end.
fn union(main: &Tree, side: &Tree, third: Option<&Tree>, owners: &BTreeMap<String, Owner>) -> Tree {
    let mut merged = Tree::new();
    for (path, owner) in owners {
        let file = match owner {
            Owner::Main => main.get(path).cloned(),
            Owner::Branch => side.get(path).cloned(),
            Owner::Third => third.and_then(|t| t.get(path)).cloned(),
            Owner::Split(_, last) => {
                let (m, s) = (&main[path], &side[path]);
                let at = |f: &File| f.lines.iter().position(|l| l == last).unwrap();
                let mut lines = m.lines[..=at(m)].to_vec();
                lines.extend_from_slice(&s.lines[at(s) + 1..]);
                let mut gone: Vec<(String, usize)> = Vec::new();
                for g in m.gone.iter().chain(&s.gone) {
                    if !lines.contains(&g.0) && !gone.iter().any(|h| h.0 == g.0) {
                        gone.push(g.clone());
                    }
                }
                Some(File {
                    lines,
                    gone,
                    ..m.clone()
                })
            }
        };
        if let Some(file) = file {
            merged.insert(path.clone(), file);
        }
    }
    merged
}

/// The shared file a round's `both` acts on, and the position of its middle line in `file`.
fn middle(file: &File, owner: &Owner) -> usize {
    let Owner::Split(first, _) = owner else {
        panic!("{owner:?} is not shared")
    };
    file.lines.iter().position(|l| l == first).unwrap() + 1
}

/// Writes commits for `git fast-import`, each with a message no other has (`<mark> on
/// <branch>`), and dates that step from the last commit's as the history draws: the same, back or
/// on, with the author's date the same as the committer's or apart from it.
struct Stream<'a> {
    out: String,
    mark: u32,
    clock: i64,
    times: BTreeMap<u32, i64>,
    dates: &'a [(u8, u8)],
    facts: BTreeMap<&'static str, usize>,
}

impl Stream<'_> {
    /// A commit on `branch` with `parents`, from what changed between `before` (its first
    /// parent's files) and `after`; returns its mark and message.
    fn commit(
        &mut self,
        branch: &str,
        parents: &[u32],
        before: &Tree,
        after: &Tree,
    ) -> (u32, String) {
        self.mark += 1;
        let mark = self.mark;
        let (step, apart) = self.dates[mark as usize % self.dates.len()];
        self.clock += [0, -5_000, 60][usize::from(step)];
        let author = self.clock + [0, -86_400, 3_600][usize::from(apart)];
        let at: Vec<i64> = parents.iter().map(|p| self.times[p]).collect();
        let mut fact = |name, yes: bool| *self.facts.entry(name).or_default() += usize::from(yes);
        fact("commits dated as a parent", at.contains(&self.clock));
        fact(
            "commits dated before a parent",
            at.iter().any(|t| *t > self.clock),
        );
        fact(
            "merges of a parent older than the first",
            at.iter().skip(1).any(|t| *t < at[0]),
        );
        self.times.insert(mark, self.clock);
        let message = format!("{mark} on {branch}");
        self.out += &format!(
            "commit refs/heads/{branch}\nmark :{mark}\nauthor Ada <ada@example.com> {author} \
             +0000\ncommitter Ada <ada@example.com> {} +0000\ndata {}\n{message}\n",
            self.clock,
            message.len() + 1
        );
        for (i, p) in parents.iter().enumerate() {
            self.out += &format!("{} :{p}\n", if i == 0 { "from" } else { "merge" });
        }
        for path in before.keys().filter(|p| !after.contains_key(*p)) {
            self.out += &format!("D {path}\n");
        }
        for (path, file) in after {
            if before.get(path).map(|b| &b.lines) != Some(&file.lines) {
                let data = file.lines.concat();
                self.out += &format!("M 100644 inline {path}\ndata {}\n{data}\n", data.len());
            }
        }
        (mark, message)
    }
}

/// A generated history written for `git fast-import`: the stream, each tip's files (`main`'s
/// first), the merges' messages and those of merges that wrote lines of their own, and what the
/// drawing counted.
struct Generated {
    stream: String,
    tips: Vec<(&'static str, Tree)>,
    merges: BTreeSet<String>,
    own: BTreeSet<String>,
    facts: BTreeMap<&'static str, usize>,
}

fn generate(h: &Merged) -> Generated {
    let mut s = Stream {
        out: String::new(),
        mark: 0,
        clock: 1_700_000_000,
        times: BTreeMap::new(),
        dates: &h.dates,
        facts: BTreeMap::new(),
    };
    let mut fresh = 0;
    let mut fact = BTreeMap::<&'static str, usize>::new();
    let (mut merges, mut own) = (BTreeSet::new(), BTreeSet::new());
    let mut main = first_tree(&h.first);
    let (mut main_mark, _) = s.commit("main", &[], &Tree::new(), &main);
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
        let shared = (owners.iter())
            .find(|(_, o)| matches!(o, Owner::Split(..)))
            .map(|(p, o)| (p.clone(), o.clone()))
            .filter(|_| round.both > 0);
        // Lines the merge's parents may hold from different commits: the one both sides add, and
        // those `main` puts back this round, which the branch may still have where they were.
        let special = RefCell::new(BTreeSet::new());
        let mut side = base.clone();
        let mut side_mark = fork;
        // Both sides' change of the shared file, each in a commit of its own, first.
        if let Some((path, owner)) = &shared {
            fresh += 1;
            let line = format!("both {fresh}\n");
            for (tree, mark, who) in [
                (&mut main, &mut main_mark, "main"),
                (&mut side, &mut side_mark, branch),
            ] {
                let before = tree.clone();
                let file = tree.get_mut(path).unwrap();
                let at = middle(file, owner);
                if round.both == 1 {
                    file.lines.insert(at, line.clone());
                } else {
                    fresh += 1;
                    file.lines[at] = format!("{who} {fresh}\n");
                }
                *mark = s.commit(who, &[*mark], &before, tree).0;
            }
            if round.both == 1 {
                special.borrow_mut().insert(line);
            }
        }
        let halves = |n: usize| if round.criss { n / 2 } else { n };
        let mut put_back = 0;
        let mut merge = |s: &mut Stream,
                         fresh: &mut usize,
                         trees: (&Tree, &Tree, Option<&Tree>),
                         parents: &[(u32, &Tree)],
                         on: &str,
                         owners: &BTreeMap<String, Owner>,
                         last: bool| {
            let mut merged = union(trees.0, trees.1, trees.2, owners);
            let mut wrote = false;
            let mut merge_line = |s: &Stream| {
                *fresh += 1;
                format!("merge {} {fresh}\n", s.mark + 1)
            };
            if let (Some((path, owner)), 2) = (&shared, round.both) {
                let file = merged.get_mut(path).unwrap();
                let at = middle(file, owner);
                file.lines[at] = merge_line(s);
                wrote = true;
            }
            if let (Some((f, at)), true, false) = (round.merge_line, last, merged.is_empty()) {
                let path = merged.keys().nth(f.index(merged.len())).unwrap().clone();
                let file = merged.get_mut(&path).unwrap();
                let place = at.index(file.lines.len() + 1);
                file.lines.insert(place, merge_line(s));
                wrote = true;
            }
            // Git's blame hands a file whole to the first parent with the same version, even a
            // later one; `who` asks the first parent first. Where a line the parents may hold
            // from different commits makes that matter, the merge writes a line of its own.
            let times: Vec<i64> = parents.iter().map(|(m, _)| s.times[m]).collect();
            for (path, file) in &mut merged {
                let same = |t: &Tree| t.get(path).is_some_and(|f| f.lines == file.lines);
                let later = parents[1..].iter().any(|(_, t)| same(t));
                let special = special.borrow();
                let both =
                    (file.lines.iter()).any(|l| l.starts_with("both ") && special.contains(l));
                let special = file.lines.iter().any(|l| special.contains(l));
                if both {
                    let older = times[1..].iter().any(|t| *t < times[0]);
                    *s.facts
                        .entry("merges of a line both sides added, a later parent older")
                        .or_default() += usize::from(older);
                }
                if special && later && !same(parents[0].1) {
                    file.lines.push(merge_line(s));
                    wrote = true;
                }
            }
            let marks: Vec<u32> = parents.iter().map(|(m, _)| *m).collect();
            let (mark, message) = s.commit(on, &marks, parents[0].1, &merged);
            merges.insert(message.clone());
            if wrote {
                own.insert(message);
            }
            (mark, merged)
        };
        for steps in &round.main[..halves(round.main.len())] {
            let before = main.clone();
            let back = make(&mut main, &mut owners, steps, (true, r), "main", &mut fresh);
            put_back += back.len();
            special.borrow_mut().extend(back);
            main_mark = s.commit("main", &[main_mark], &before, &main).0;
        }
        for steps in &round.branch[..halves(round.branch.len())] {
            let before = side.clone();
            put_back += make(
                &mut side,
                &mut owners,
                steps,
                (false, r),
                branch,
                &mut fresh,
            )
            .len();
            side_mark = s.commit(branch, &[side_mark], &before, &side).0;
        }
        if round.criss {
            let parents = [(main_mark, &main), (side_mark, &side)];
            let (m1, crossed) = merge(
                &mut s,
                &mut fresh,
                (&main, &side, None),
                &parents,
                "main",
                &owners,
                false,
            );
            let parents = [(side_mark, &side), (main_mark, &main)];
            let (m2, back) = merge(
                &mut s,
                &mut fresh,
                (&main, &side, None),
                &parents,
                branch,
                &owners,
                false,
            );
            (main_mark, main, side_mark, side) = (m1, crossed, m2, back);
            *fact.entry("rounds that cross merges").or_default() += 1;
            for steps in &round.main[halves(round.main.len())..] {
                let before = main.clone();
                let back = make(&mut main, &mut owners, steps, (true, r), "main", &mut fresh);
                put_back += back.len();
                special.borrow_mut().extend(back);
                main_mark = s.commit("main", &[main_mark], &before, &main).0;
            }
            for steps in &round.branch[halves(round.branch.len())..] {
                let before = side.clone();
                put_back += make(
                    &mut side,
                    &mut owners,
                    steps,
                    (false, r),
                    branch,
                    &mut fresh,
                )
                .len();
                side_mark = s.commit(branch, &[side_mark], &before, &side).0;
            }
        }
        let mut third = None;
        if let Some((at, n)) = round.third {
            let free: Vec<&str> = (PATHS.iter().copied())
                .filter(|p| {
                    !owners.contains_key(*p) && !main.contains_key(*p) && !side.contains_key(*p)
                })
                .collect();
            if !free.is_empty() {
                let path = free[at.index(free.len())].to_owned();
                let mut tree = base.clone();
                let lines = (0..n)
                    .map(|_| {
                        fresh += 1;
                        format!("{} {fresh}\n", THIRDS[r])
                    })
                    .collect();
                let file = File {
                    lines,
                    ..File::default()
                };
                tree.insert(path.clone(), file);
                owners.insert(path, Owner::Third);
                let (mark, _) = s.commit(THIRDS[r], &[fork], &base, &tree);
                third = Some((mark, tree));
            }
        }
        // A file both sides changed, each at its own end.
        for (path, owner) in &owners {
            if let Owner::Split(first, last) = owner {
                let at = |f: &File, l: &String| f.lines.iter().position(|x| x == l).unwrap();
                let (m, s, b) = (&main[path], &side[path], &base[path]);
                let heads = m.lines[..at(m, first)] != b.lines[..at(b, first)];
                let tails = s.lines[at(s, last) + 1..] != b.lines[at(b, last) + 1..];
                *fact.entry("files both sides changed").or_default() += usize::from(heads && tails);
            }
        }
        let mut parents = vec![(main_mark, &main), (side_mark, &side)];
        if let Some((mark, tree)) = &third {
            parents.push((*mark, tree));
        }
        let trees = (&main, &side, third.as_ref().map(|(_, t)| t));
        let (mark, merged) = merge(&mut s, &mut fresh, trees, &parents, "main", &owners, true);
        *fact.entry("lines put back").or_default() += put_back;
        main_mark = mark;
        tips.push((branch, side));
        if let Some((_, tree)) = third {
            tips.push((THIRDS[r], tree));
        }
        main = merged;
    }
    tips.insert(0, ("main", main));
    fact.extend(s.facts);
    Generated {
        stream: s.out,
        tips,
        merges,
        own,
        facts: fact,
    }
}

/// Every merge of two parents on `main` that wrote nothing of its own holds what `git merge-tree`
/// makes of them: the drawn merges are clean, and are Git's own. Returns how many it checked.
fn merges_are_gits(dir: &Path, own: &BTreeSet<String>) -> usize {
    let listed = text(&git(dir, &["log", "--merges", "--format=%T %P %s", "main"]));
    let mut checked = 0;
    for merge in listed.lines() {
        let words: Vec<&str> = merge.splitn(4, ' ').collect();
        let [tree, ours, theirs, message] = <[&str; 4]>::try_from(words).unwrap();
        // A third parent puts its own parent's ID before the message.
        if own.contains(message) || message.split(' ').nth(1) != Some("on") {
            continue;
        }
        let made = text(&git(dir, &["merge-tree", "--write-tree", ours, theirs]));
        assert_eq!(made.lines().next(), Some(tree), "{merge}");
        checked += 1;
    }
    checked
}

/// Each line of `path` at `tip` as `who` names it: the first line of its save's message, and the
/// path the file had there. The runs must cover the file's lines once each, in order. Also the
/// whole document.
fn who(env: &Env, tip: &str, path: &str) -> (Named, String) {
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
        assert_eq!(run["first"], lines.len() + 1, "{said}");
        let save = run["save"].as_str().unwrap_or_else(|| panic!("{said}"));
        let message = messages[save].lines().next().unwrap_or_default().to_owned();
        let was = run["was"].as_str().unwrap_or(path).to_owned();
        for _ in 0..run["lines"].as_u64().unwrap() {
            lines.push((message.clone(), was.clone()));
        }
    }
    assert_eq!(doc["lines"], lines.len(), "{said}");
    (lines, said)
}

/// Each line of `path` at `tip` as `git blame` names it: its commit's summary and the path the
/// file had there; and the line itself.
fn blamed(dir: &Path, tip: &str, path: &str) -> (Named, Vec<String>) {
    let out = text(&git(dir, &["blame", "--line-porcelain", tip, "--", path]));
    let (mut summary, mut filename) = ("", "");
    let (mut named, mut lines) = (Vec::new(), Vec::new());
    for l in out.lines() {
        if let Some(line) = l.strip_prefix('\t') {
            named.push((summary.to_owned(), filename.to_owned()));
            lines.push(format!("{line}\n"));
        } else if let Some(s) = l.strip_prefix("summary ") {
            summary = s;
        } else if let Some(f) = l.strip_prefix("filename ") {
            filename = f;
        }
    }
    (named, lines)
}

/// Every file of `tree`, at `tip`: each line's save in `env` must be the commit `git blame` names
/// in `dir`, by message and by the path the file had there. Counts what was compared in `seen`.
fn compare(
    env: &Env,
    dir: &Path,
    (tip, tree): (&str, &Tree),
    merges: &BTreeSet<String>,
    history: &str,
    seen: &mut BTreeMap<&'static str, usize>,
) -> Result<(), TestCaseError> {
    let mut count = |name, n: usize| *seen.entry(name).or_default() += n;
    for (path, file) in tree {
        let (ours, doc) = who(env, tip, path);
        let (theirs, lines) = blamed(dir, tip, path);
        prop_assert_eq!(&lines, &file.lines, "{} at {}", path, tip);
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
        count("files", 1);
        count("lines", ours.len());
        count("empty files", usize::from(lines.is_empty()));
        count(
            "files emptied and filled again",
            usize::from(file.emptied && !lines.is_empty()),
        );
        count(
            "files moved and edited after",
            usize::from(file.edited_after_move && !lines.is_empty()),
        );
        let moved = |(_, was): &&(String, String)| was != path;
        count(
            "lines named through a move",
            ours.iter().filter(moved).count(),
        );
        if tip != "main" {
            continue;
        }
        let at_main = ours.iter().zip(&lines);
        let off_main = |((m, _), _): &(&(String, String), &String)| !m.ends_with(" on main");
        count(
            "lines through a later parent",
            at_main.clone().filter(off_main).count(),
        );
        count(
            "lines from a third parent",
            at_main
                .clone()
                .filter(|((m, _), _)| m.contains(" on extra-"))
                .count(),
        );
        count(
            "lines named for a merge",
            at_main
                .clone()
                .filter(|((m, _), _)| merges.contains(m))
                .count(),
        );
        count(
            "lines both parents hold",
            at_main
                .clone()
                .filter(|(_, l)| l.starts_with("both "))
                .count(),
        );
        count(
            "lines moved on a merged branch",
            if file.moved_on_branch {
                ours.iter().filter(moved).count()
            } else {
                0
            },
        );
    }
    Ok(())
}

/// Adds one history's counts to `totals`: each thing's total, and how many histories had it.
fn tally(
    totals: &RefCell<BTreeMap<&'static str, (usize, usize)>>,
    seen: BTreeMap<&'static str, usize>,
) {
    let mut totals = totals.borrow_mut();
    for (name, n) in seen {
        let t = totals.entry(name).or_default();
        t.0 += n;
        t.1 += usize::from(n > 0);
    }
    let histories = totals.entry("histories").or_default();
    *histories = (histories.0 + 1, histories.1 + 1);
}

/// Prints `totals` and checks that each of `floors` was in at least 3 histories.
fn floors(part: &str, totals: RefCell<BTreeMap<&'static str, (usize, usize)>>, floors: &[&str]) {
    let totals = totals.into_inner();
    println!("{part}:");
    for (name, (n, histories)) in &totals {
        println!("  {name}: {n}, in {histories} histories");
    }
    assert!(
        totals.get("lines").is_some_and(|t| t.0 > 0),
        "{part}: compared no line"
    );
    for name in floors {
        let histories = totals.get(name).map_or(0, |t| t.1);
        assert!(histories >= 3, "{part}: {name} in {histories} histories");
    }
}

/// Histories made in Git and imported, then histories with moves made in both programs: for every
/// file at every tip, each line's save is the commit `git blame` names, found by its message (no
/// two commits share one) and the path the file had there. Every line written is a text no other
/// line has, but for one line both sides of a merge add at the same place; a line put back after
/// it was taken out is the later commit's in both. Merges conflict nowhere: the sides edit
/// different files, or different ends of a file either side of three lines neither touches, and
/// each merge of two parents that wrote nothing of its own holds what `git merge-tree` makes of
/// them. Some merges write lines of their own: one anywhere, or a third text where both sides
/// changed one line. Some have a third parent; some rounds cross merges first. Dates tie, run
/// backwards, and the author's differs from the committer's.
///
/// An imported history carries no move notes, so Opalro follows no move in it while Git's `blame`
/// follows a whole-file move by likeness. The moved files are therefore made with Opalro's own
/// commands (each move saved by itself, which notes it, and a branch merged with `opalro
/// combine`), and the same steps run with Git in a repository beside the project.
///
/// Not drawn: a move with an edit in the same commit (Git guesses and follows, Opalro doesn't); a
/// line repeated in a file (two right answers); a change of line endings only (a save that only
/// changes endings writes no line for `who`, while Git counts it); a file changed under its old
/// name on a branch merged into one that moved it (Opalro's known limit, pinned by the engine's
/// own tests); and a merge whose file is the same as a later parent's version and not the first
/// parent's, while it holds a line the parents may have from different commits (one both sides
/// added, or one `main` put back that the branch still has where it was). Git hands such a file
/// whole to the later parent and `who` asks the first: a known difference, kept, which the fixed
/// tests below show; where the generator meets it, the merge writes a line of its own instead.
/// The seed is fixed.
#[test]
fn who_matches_git_blame_on_unique_lines() {
    let started = Instant::now();
    let config = |cases| Config {
        cases,
        failure_persistence: None,
        ..Config::default()
    };
    let imported = RefCell::new(BTreeMap::new());
    let rng = TestRng::deterministic_rng(RngAlgorithm::ChaCha);
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
            let mut seen = made.facts.clone();
            seen.insert(
                "merges checked with git merge-tree",
                merges_are_gits(env.root(), &made.own),
            );
            env.import();
            for (tip, tree) in &made.tips {
                compare(
                    &env,
                    env.root(),
                    (tip, tree),
                    &made.merges,
                    &made.stream,
                    &mut seen,
                )?;
            }
            let moved = seen.get("lines named through a move").copied();
            prop_assert_eq!(moved.unwrap_or(0), 0, "a move from Git");
            tally(&imported, seen);
            Ok(())
        })
        .unwrap();

    let mirrored = RefCell::new(BTreeMap::new());
    let rng = TestRng::deterministic_rng(RngAlgorithm::ChaCha);
    TestRunner::new_with_rng(config(24), rng)
        .run(&linear_history(), |h| {
            let (ours, theirs, tips, steps) = mirror(&h);
            let mut seen = BTreeMap::new();
            for (tip, tree) in &tips {
                compare(
                    &ours,
                    theirs.root(),
                    (tip, tree),
                    &BTreeSet::new(),
                    &steps,
                    &mut seen,
                )?;
            }
            tally(&mirrored, seen);
            Ok(())
        })
        .unwrap();

    floors(
        "made in Git and imported, compared at every tip",
        imported,
        &[
            "lines through a later parent",
            "lines both parents hold",
            "lines named for a merge",
            "lines from a third parent",
            "rounds that cross merges",
            "commits dated as a parent",
            "commits dated before a parent",
            "merges of a parent older than the first",
            "merges of a line both sides added, a later parent older",
            "empty files",
            "files emptied and filled again",
            "files both sides changed",
            "lines put back",
        ],
    );
    floors(
        "made in both with moves",
        mirrored,
        &[
            "lines named through a move",
            "files moved and edited after",
            "lines moved on a merged branch",
        ],
    );
    println!("took {:.1?}", started.elapsed());
}

/// The history `h` made twice: in an Opalro project with `opalro save` and `opalro combine`, and
/// in a Git repository with `git commit` and `git merge`, the same files and moves each time.
/// Returns the project, the repository, the files at each tip and the steps taken, for a failure
/// to show.
fn mirror(h: &Linear) -> (Env, Env, Vec<(&'static str, Tree)>, String) {
    let ours = Env::empty();
    let opalro = |args: &[&str]| {
        let out = ours.opalro().args(args).output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{args:?}: {}",
            text(&out.stderr)
        );
    };
    opalro(&["start", "--preset", "custom"]);
    let theirs = Env::new();
    let mut fresh = 0;
    let mut log = String::new();
    let mut n = 0;
    // Makes `steps` on `tree`, writes the change in both folders and commits it, unless it changed
    // nothing.
    let mut step = |tree: &mut Tree,
                    owners: &mut BTreeMap<String, Owner>,
                    steps: Option<&Vec<Step>>,
                    (on_main, round): (bool, usize),
                    fresh: &mut usize| {
        let before = tree.clone();
        let who = if on_main { "main" } else { "side" };
        match steps {
            None => *tree = first_tree(&h.first),
            Some(steps) => {
                make(tree, owners, steps, (on_main, round), who, fresh);
            }
        }
        let lines = |t: &Tree| {
            t.iter()
                .map(|(p, f)| (p.clone(), f.lines.clone()))
                .collect::<Vec<_>>()
        };
        if lines(&before) == lines(tree) {
            return;
        }
        let moved = steps.and_then(|s| match s.as_slice() {
            [Step::Move(..)] => {
                let from = before.keys().find(|p| !tree.contains_key(*p)).unwrap();
                let to = tree.keys().find(|p| !before.contains_key(*p)).unwrap();
                Some((from.clone(), to.clone()))
            }
            _ => None,
        });
        for dir in [ours.root(), theirs.root()] {
            if let Some((from, to)) = &moved {
                std::fs::create_dir_all(dir.join(to).parent().unwrap()).unwrap();
                std::fs::rename(dir.join(from), dir.join(to)).unwrap();
                continue;
            }
            for path in before.keys().filter(|p| !tree.contains_key(*p)) {
                std::fs::remove_file(dir.join(path)).unwrap();
            }
            for (path, file) in tree.iter() {
                if before.get(path).map(|b| &b.lines) != Some(&file.lines) {
                    std::fs::create_dir_all(dir.join(path).parent().unwrap()).unwrap();
                    std::fs::write(dir.join(path), file.lines.concat()).unwrap();
                }
            }
        }
        n += 1;
        let message = format!("{n} on {who}");
        opalro(&["save", &message]);
        theirs.git(&["add", "-A"]);
        theirs.git(&["commit", "-q", "-m", &message]);
        log += &match &moved {
            Some((from, to)) => format!("{message}: {from} moved to {to}\n"),
            None => format!("{message}: {:?}\n", lines(tree)),
        };
    };
    let mut tree = Tree::new();
    for commit in std::iter::once(None).chain(h.commits.iter().map(Some)) {
        let mut owners = tree.keys().map(|p| (p.clone(), Owner::Main)).collect();
        step(&mut tree, &mut owners, commit, (true, 0), &mut fresh);
    }
    let mut tips = Vec::new();
    if let Some(fork) = &h.fork {
        let mut owners: BTreeMap<String, Owner> = (tree.keys().zip(fork.owners.iter().cycle()))
            .map(|(p, side)| (p.clone(), if *side { Owner::Branch } else { Owner::Main }))
            .collect();
        let mut side = tree.clone();
        let fork_tree = tree.clone();
        opalro(&["line", "new", "side"]);
        theirs.git(&["checkout", "-q", "-b", "side"]);
        for commit in &fork.side {
            step(&mut side, &mut owners, Some(commit), (false, 1), &mut fresh);
        }
        opalro(&["goto", "main"]);
        theirs.git(&["checkout", "-q", "main"]);
        for commit in &fork.main {
            step(&mut tree, &mut owners, Some(commit), (true, 1), &mut fresh);
        }
        // A branch that saved nothing has nothing to merge.
        if side
            .iter()
            .map(|(p, f)| (p, &f.lines))
            .eq(fork_tree.iter().map(|(p, f)| (p, &f.lines)))
        {
            tips.insert(0, ("main", tree));
            return (ours, theirs, tips, log);
        }
        n += 1;
        let message = format!("{n} on main");
        opalro(&["combine", "side", &message]);
        theirs.git(&["merge", "-q", "--no-ff", "-m", &message, "side"]);
        log += &format!("{message}: side merged\n");
        tree = union(&tree, &side, None, &owners);
        tips.push(("side", side));
    }
    tips.insert(0, ("main", tree));
    (ours, theirs, tips, log)
}

// Fixed histories, made with Git's own commands and imported.

/// `env`'s `path` at `main`, as each program names its lines (by message, and the path there).
fn both_answers(env: &Env, path: &str) -> (Named, Named) {
    env.import();
    (who(env, "main", path).0, blamed(env.root(), "main", path).0)
}

fn named(messages: &[&str], path: &str) -> Named {
    messages
        .iter()
        .map(|m| ((*m).to_owned(), path.to_owned()))
        .collect()
}

/// A commit that moves a line within a file: `L0 L1 L2 L3 L4` becomes `L2 L0 L1 S4 L3 L4`. The
/// shortest diff keeps `L0 L1 L3 L4` (four lines; keeping `L2` keeps at most `L2 L3 L4`), so it
/// takes `L2` out and adds `L2` and `S4`: three changes, and `L2` is new where it lands. Both
/// programs say so.
#[test]
fn a_line_moved_within_a_file_is_new_where_it_lands_in_both() {
    let env = Env::new();
    env.commit("f.txt", "L0\nL1\nL2\nL3\nL4\n", "c1");
    env.commit("f.txt", "L2\nL0\nL1\nS4\nL3\nL4\n", "c2");
    let (ours, theirs) = both_answers(&env, "f.txt");
    assert_eq!(ours, named(&["c2", "c1", "c1", "c2", "c1", "c1"], "f.txt"));
    assert_eq!(ours, theirs);
}

/// The history the generator first failed on, at its smallest. The merge's `f.txt` is the
/// branch's (`L2 S6 S7 S8 L3 L4`), and `main`'s is the first commit's (`L0 L1 L2 L3 L4`). Git
/// hands the file whole to the branch, whose version is the same, so `L2` is `c3`'s, where it
/// moved to the top. `who` compares the merge with `main` first: the only shortest diff keeps `L2
/// L3 L4`, so `L2` goes to `main`'s side and is `c1`'s. Both wrote `L2`.
#[test]
fn where_a_merge_took_a_later_parents_file_whole_who_asks_the_first_parent_and_git_the_later() {
    let env = Env::new();
    env.commit(
        "f.txt",
        "L0
L1
L2
L3
L4
",
        "c1",
    );
    env.git(&["checkout", "-q", "-b", "side"]);
    env.commit(
        "f.txt",
        "L2
L0
L1
S4
L3
L4
",
        "c3",
    );
    env.commit(
        "f.txt",
        "L2
S6
S7
S8
L3
L4
",
        "c4",
    );
    env.git(&["checkout", "-q", "main"]);
    env.commit(
        "g.txt", "x
", "c2",
    );
    env.git(&["merge", "-q", "--no-edit", "-m", "merge", "side"]);
    let (ours, theirs) = both_answers(&env, "f.txt");
    // Git hands a merge's file whole to a parent that holds it unchanged; `who` asks the first
    // parent first.
    assert_eq!(ours, named(&["c1", "c4", "c4", "c4", "c1", "c1"], "f.txt"));
    assert_eq!(
        theirs,
        named(&["c3", "c4", "c4", "c4", "c1", "c1"], "f.txt")
    );
}

/// Both sides add `X` at the same place; the branch also adds `Y`, so the merge's file is the
/// branch's. Git hands it whole to the branch: `X` is the branch's commit. `who` finds `X` in
/// `main`, the first parent, by the only shortest diff: `X` is `main`'s commit. Both wrote `X`.
#[test]
fn a_line_both_sides_added_is_the_first_parents_in_who_and_the_later_parents_in_git_where_the_merge_took_its_file()
 {
    let env = Env::new();
    env.commit(
        "f.txt",
        "L0
L1
L2
L3
",
        "c1",
    );
    env.git(&["checkout", "-q", "-b", "side"]);
    env.commit(
        "f.txt",
        "L0
X
L1
L2
L3
",
        "side adds X",
    );
    env.commit(
        "f.txt",
        "L0
X
L1
L2
Y
L3
",
        "side adds Y",
    );
    env.git(&["checkout", "-q", "main"]);
    env.commit(
        "f.txt",
        "L0
X
L1
L2
L3
",
        "main adds X",
    );
    env.git(&["merge", "-q", "--no-edit", "-m", "merge", "side"]);
    let (ours, theirs) = both_answers(&env, "f.txt");
    // Git hands a merge's file whole to a parent that holds it unchanged; `who` asks the first
    // parent first.
    let who = ["c1", "main adds X", "c1", "c1", "side adds Y", "c1"];
    let git = ["c1", "side adds X", "c1", "c1", "side adds Y", "c1"];
    assert_eq!(ours, named(&who, "f.txt"));
    assert_eq!(theirs, named(&git, "f.txt"));
}

/// The same where `main`'s file differs from the merge's too: Git then compares line by line,
/// first parent first, as `who` does, and both name `main`'s commit for `X`.
#[test]
fn a_line_both_sides_added_goes_to_the_first_parent_in_both() {
    let env = Env::new();
    env.commit("f.txt", "L0\nL1\nL2\nL3\n", "c1");
    env.git(&["checkout", "-q", "-b", "side"]);
    env.commit("f.txt", "L0\nX\nL1\nL2\nY\nL3\n", "side adds X and Y");
    env.git(&["checkout", "-q", "main"]);
    env.commit("f.txt", "Z\nL0\nX\nL1\nL2\nL3\n", "main adds Z and X");
    env.git(&["merge", "-q", "--no-edit", "-m", "merge", "side"]);
    let (ours, theirs) = both_answers(&env, "f.txt");
    let main = "main adds Z and X";
    let names = [main, "c1", main, "c1", "c1", "side adds X and Y", "c1"];
    assert_eq!(ours, named(&names, "f.txt"));
    assert_eq!(ours, theirs);
}

/// The branch takes `L1` out and puts it back at the end, and the merge's file is the branch's
/// (`L0 L2 L1`). `L0 L1 L2` against `L0 L2 L1` is a tie: two shortest diffs keep two lines (`L0
/// L1` or `L0 L2`). `who` compares the merge with `main` first and keeps `L0 L1`, so `L1` is
/// `c1`'s. Git names the commit that put `L1` back, which fits handing the file whole to the
/// branch, and equally a first-parent pass that keeps `L0 L2`: this history alone proves neither.
/// Both wrote `L1`.
#[test]
fn where_two_diffs_tie_at_a_merge_a_line_the_branch_put_back_may_be_named_apart() {
    let env = Env::new();
    env.commit(
        "f.txt",
        "L0
L1
L2
",
        "c1",
    );
    env.git(&["checkout", "-q", "-b", "side"]);
    env.commit(
        "f.txt",
        "L0
L2
",
        "side takes L1 out",
    );
    env.commit(
        "f.txt",
        "L0
L2
L1
",
        "side puts L1 back",
    );
    env.git(&["checkout", "-q", "main"]);
    env.commit(
        "g.txt", "x
", "c2",
    );
    env.git(&["merge", "-q", "--no-edit", "-m", "merge", "side"]);
    let (ours, theirs) = both_answers(&env, "f.txt");
    // Where two line-ups are equally short, or a later parent holds the file unchanged, the two
    // programs may name different saves that wrote the line.
    assert_eq!(ours, named(&["c1", "c1", "c1"], "f.txt"));
    assert_eq!(theirs, named(&["c1", "c1", "side puts L1 back"], "f.txt"));
}
