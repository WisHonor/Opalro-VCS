//! Which saves one line has in its history that others don't, and where two lines parted,
//! following every parent.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BinaryHeap, HashMap};

use crate::id::Id;
use crate::repo::{RepoError, Repository};

const TIP: u8 = 1;
const OTHERS: u8 = 2;

const OURS: u8 = 1;
const THEIRS: u8 = 2;
const BOTH: u8 = OURS | THEIRS;
/// Under a save both sides have.
const BELOW: u8 = 4;

/// The most common starting points a combine takes at one level.
pub const MAX_STARTS: usize = 16;

/// What `common_starts` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Starts {
    /// The common starting points, in ID order; empty when the histories never met.
    Found(Vec<Id>),
    /// The walk's candidates were more than `MAX_STARTS`: how many.
    TooMany(usize),
}

struct Node {
    marks: u8,
    /// The marks it last gave its parents; 0 before that.
    spread: u8,
    time: i64,
    parents: Vec<Id>,
}

impl Node {
    /// Has marks its parents haven't been given yet.
    fn waiting(&self) -> bool {
        self.marks != self.spread
    }

    fn one_side_waiting(&self) -> bool {
        self.waiting() && (self.marks == TIP || self.marks == OTHERS)
    }

    /// Gained a mark after spreading, so what it gave before needs correcting.
    fn correcting(&self) -> bool {
        self.waiting() && self.spread != 0
    }
}

/// What a walk keeps count of, told of each node before and after it changes.
trait Watch {
    fn add(&mut self, node: &Node);
    fn remove(&mut self, node: &Node);
}

/// What decides whether the walk must go on, kept up to date as nodes change.
#[derive(Default)]
struct Tally {
    one_side_waiting: usize,
    correcting: usize,
    /// The times of the saves counted so far (marked by the tip alone), with how many share each.
    counted: BTreeMap<i64, usize>,
}

impl Watch for Tally {
    fn add(&mut self, node: &Node) {
        self.one_side_waiting += usize::from(node.one_side_waiting());
        self.correcting += usize::from(node.correcting());
        if node.marks == TIP {
            *self.counted.entry(node.time).or_default() += 1;
        }
    }

    fn remove(&mut self, node: &Node) {
        self.one_side_waiting -= usize::from(node.one_side_waiting());
        self.correcting -= usize::from(node.correcting());
        if node.marks == TIP
            && let Some(n) = self.counted.get_mut(&node.time)
        {
            *n -= 1;
            if *n == 0 {
                self.counted.remove(&node.time);
            }
        }
    }
}

impl Tally {
    /// Whether a waiting save of this time must still spread. A shared save older than every
    /// counted one can't sit above any of them when no save is older than its parent.
    fn must_go_on(&self, time: i64) -> bool {
        self.one_side_waiting > 0
            || self.correcting > 0
            || self
                .counted
                .first_key_value()
                .is_some_and(|(&t, _)| time >= t)
    }
}

/// How many waiting saves aren't under a save both sides have.
#[derive(Default)]
struct Unsettled(usize);

impl Watch for Unsettled {
    fn add(&mut self, node: &Node) {
        self.0 += usize::from(node.waiting() && node.marks & BELOW == 0);
    }

    fn remove(&mut self, node: &Node) {
        self.0 -= usize::from(node.waiting() && node.marks & BELOW == 0);
    }
}

struct Walk<'r, W> {
    repo: &'r Repository,
    nodes: HashMap<Id, Node>,
    /// Newest first: every waiting node, and stale entries skipped when met.
    heap: BinaryHeap<(i64, Id)>,
    watch: W,
}

impl<'r, W: Watch> Walk<'r, W> {
    fn new(repo: &'r Repository, watch: W) -> Self {
        Walk {
            repo,
            nodes: HashMap::new(),
            heap: BinaryHeap::new(),
            watch,
        }
    }

    /// Adds `marks` to a save, reading it the first time it is met. Returns all it holds.
    fn mark(&mut self, id: Id, marks: u8) -> Result<u8, RepoError> {
        let node = match self.nodes.entry(id) {
            Entry::Occupied(e) => e.into_mut(),
            Entry::Vacant(e) => {
                let save = self
                    .repo
                    .store()
                    .save(&id, self.repo.signatures_allowed())?;
                e.insert(Node {
                    marks: 0,
                    spread: 0,
                    time: save.saver.time_ms,
                    parents: save.parents,
                })
            }
        };
        if node.marks | marks != node.marks {
            self.watch.remove(node);
            node.marks |= marks;
            self.watch.add(node);
            self.heap.push((node.time, id));
        }
        Ok(node.marks)
    }

    /// Takes the newest waiting save while `go_on` says so for its time, and returns the marks it
    /// now gives its parents, with them.
    fn next(&mut self, go_on: impl Fn(&W, i64) -> bool) -> Option<(u8, Vec<Id>)> {
        while let Some(&(time, id)) = self.heap.peek() {
            let Some(node) = self.nodes.get_mut(&id).filter(|n| n.waiting()) else {
                self.heap.pop();
                continue;
            };
            if !go_on(&self.watch, time) {
                return None;
            }
            self.heap.pop();
            // A mark gained after spreading is passed on too.
            self.watch.remove(node);
            node.spread = node.marks;
            self.watch.add(node);
            return Some((node.marks, node.parents.clone()));
        }
        None
    }
}

/// How many saves `tip` has in its history, itself included, that no save of `others` has in
/// its own. 0 exactly when one of `others` is `tip` or has `tip` in its history.
pub fn only_in(repo: &Repository, tip: Id, others: &[Id]) -> Result<u64, RepoError> {
    let mut walk = Walk::new(repo, Tally::default());
    walk.mark(tip, TIP)?;
    for &other in others {
        walk.mark(other, OTHERS)?;
        if other == tip {
            return Ok(0);
        }
    }
    while let Some((marks, parents)) = walk.next(|tally, time| tally.must_go_on(time)) {
        for parent in parents {
            if walk.mark(parent, marks)? & OTHERS != 0 && parent == tip {
                return Ok(0);
            }
        }
    }
    Ok(walk.nodes.values().filter(|n| n.marks == TIP).count() as u64)
}

/// The saves both sides have in their history that no other such save has in its history;
/// each side is a set of tips. Exact whatever the saves' clocks say, whenever it is `Found`.
pub fn common_starts(repo: &Repository, ours: &[Id], theirs: &[Id]) -> Result<Starts, RepoError> {
    let mut walk = Walk::new(repo, Unsettled::default());
    for (tips, side) in [(ours, OURS), (theirs, THEIRS)] {
        for &tip in tips {
            walk.mark(tip, side)?;
        }
    }
    // Every path down to a starting point runs through saves only one side has, which keep the
    // walk going until both marks arrive.
    while let Some((marks, parents)) = walk.next(|unsettled, _| unsettled.0 > 0) {
        let marks = if marks & BOTH == BOTH {
            marks | BELOW
        } else {
            marks
        };
        for parent in parents {
            walk.mark(parent, marks)?;
        }
    }
    let mut candidates: Vec<Id> = walk
        .nodes
        .iter()
        .filter(|(_, node)| node.marks == BOTH)
        .map(|(&id, _)| id)
        .collect();
    if candidates.len() > MAX_STARTS {
        return Ok(Starts::TooMany(candidates.len()));
    }
    candidates.sort();
    if candidates.len() < 2 {
        return Ok(Starts::Found(candidates));
    }
    // Where saves are dated alike or before a parent, a candidate can be under another; only_in
    // tells that exactly.
    let mut starts = Vec::with_capacity(candidates.len());
    for &c in &candidates {
        let others: Vec<Id> = candidates.iter().copied().filter(|&d| d != c).collect();
        if only_in(repo, c, &others)? != 0 {
            starts.push(c);
        }
    }
    Ok(Starts::Found(starts))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::diary::{Action, DiaryEntry};
    use crate::pack::ObjectKind;
    use crate::save::{Person, Save};
    use proptest::prelude::*;
    use std::collections::{BTreeSet, HashSet};

    fn project() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let who = Person {
            name: "Ada".into(),
            email: "a@example.com".into(),
            time_ms: 0,
            tz_offset_min: 0,
        };
        let repo = Repository::init(dir.path(), &who).unwrap();
        (dir, repo)
    }

    /// Saves to plant: each one's parents, as indexes of earlier ones, and its time.
    type Saves = Vec<(Vec<usize>, i64)>;

    /// Stores one save per `(parents, time)`, its parents given as indexes of earlier ones, all
    /// in one command. Returns their IDs in order.
    fn plant(repo: &mut Repository, saves: &[(Vec<usize>, i64)]) -> Vec<Id> {
        let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
        let (head, entry) = repo.current().unwrap();
        let start = repo
            .store()
            .save(&entry.state.lines["main"], false)
            .unwrap();
        let mut packs = repo.new_packs().unwrap();
        let mut ids: Vec<Id> = Vec::new();
        for (n, (parents, time)) in saves.iter().enumerate() {
            let save = Save {
                parents: parents.iter().map(|&p| ids[p]).collect(),
                saver: Person {
                    time_ms: *time,
                    ..start.saver.clone()
                },
                message: n.to_string(),
                ..start.clone()
            };
            ids.push(packs.add_record(ObjectKind::Save, &save.encode()).unwrap());
        }
        let diary = DiaryEntry {
            parent: Some(head),
            action: Action::Command,
            command: "save".into(),
            who: start.saver.clone(),
            opal_version: "0.1.0".into(),
            state: entry.state.clone(),
            folder_change: None,
        };
        repo.commit(&lock, packs, &diary).unwrap();
        ids
    }

    /// A chain of `len` saves after `from`, each a minute later; the times start after `t`.
    fn chain_after(saves: &mut Vec<(Vec<usize>, i64)>, from: Option<usize>, len: usize, t: i64) {
        let mut parent = from;
        for k in 0..len {
            saves.push((parent.into_iter().collect(), t + 60_000 * (k as i64 + 1)));
            parent = Some(saves.len() - 1);
        }
    }

    #[test]
    fn a_line_another_holds_has_nothing_of_its_own() {
        let (_dir, mut repo) = project();
        let mut saves = Vec::new();
        chain_after(&mut saves, None, 3, 0);
        let [_a, b, c] = plant(&mut repo, &saves)[..] else {
            panic!()
        };
        assert_eq!(only_in(&repo, b, &[c]).unwrap(), 0);
        assert_eq!(only_in(&repo, c, &[b]).unwrap(), 1);
        assert_eq!(only_in(&repo, c, &[]).unwrap(), 3);
        assert_eq!(only_in(&repo, c, &[c]).unwrap(), 0);
    }

    #[test]
    fn a_held_tip_dated_after_its_holder_is_still_held() {
        // The holder is older than the tip, so the tip spreads before the holder's mark reaches it.
        let (_dir, mut repo) = project();
        let ids = plant(
            &mut repo,
            &[(vec![], 1000), (vec![0], 3000), (vec![1], 2000)],
        );
        assert_eq!(only_in(&repo, ids[1], &[ids[2]]).unwrap(), 0);
    }

    #[test]
    fn a_history_dated_backwards_can_count_high_never_low() {
        let (_dir, mut repo) = project();
        // X, then S dated before it, then A merging S and X, and B after S.
        let [_x, _s, a, b] = plant(
            &mut repo,
            &[
                (vec![], 50),
                (vec![0], 10),
                (vec![1, 0], 100),
                (vec![1], 90),
            ],
        )[..] else {
            panic!()
        };
        // The answer is 1 (A alone), but newest first X is read before S, which would have carried
        // B's mark to it, so X is counted as A's too.
        assert_eq!(only_in(&repo, a, &[b]).unwrap(), 2);
    }

    /// `k` saves of their own first, so that each history's IDs, and so its tie order, differ.
    fn filler(k: usize) -> Vec<(Vec<usize>, i64)> {
        (0..k).map(|_| (vec![], 0)).collect()
    }

    #[test]
    fn saves_dated_alike_are_still_counted_right() {
        // Q <- P <- X and P <- Y, all at once; X has itself alone.
        let mut orders = HashSet::new();
        for k in 0..16 {
            let (_dir, mut repo) = project();
            let mut saves = filler(k);
            let q = saves.len();
            let (p, x, y) = (q + 1, q + 2, q + 3);
            saves.extend([(vec![], 5), (vec![q], 5), (vec![p], 5), (vec![p], 5)]);
            let ids = plant(&mut repo, &saves);
            assert_eq!(only_in(&repo, ids[x], &[ids[y]]).unwrap(), 1, "history {k}");
            orders.insert(ids[p] > ids[y]);
        }
        assert_eq!(orders.len(), 2, "both tie orders");
    }

    #[test]
    fn a_shared_save_dated_like_a_counted_one_still_passes_its_marks_on() {
        // A <- B, both at once, and C after them with parents [A, B]. B is held, so only C
        // counts; B must still tell A, though nothing waits with one mark when A has spread.
        let mut orders = HashSet::new();
        for k in 0..16 {
            let (_dir, mut repo) = project();
            let mut saves = filler(k);
            let a = saves.len();
            let (b, c) = (a + 1, a + 2);
            saves.extend([(vec![], 5), (vec![a], 5), (vec![a, b], 6)]);
            let ids = plant(&mut repo, &saves);
            assert_eq!(only_in(&repo, ids[c], &[ids[b]]).unwrap(), 1, "history {k}");
            orders.insert(ids[a] > ids[b]);
        }
        assert_eq!(orders.len(), 2, "both tie orders");
    }

    #[test]
    fn a_tie_where_two_lines_parted_does_not_run_to_the_start() {
        let mut orders = HashSet::new();
        for k in 0..16 {
            let (_dir, mut repo) = project();
            let mut saves = filler(k);
            let first = saves.len();
            chain_after(&mut saves, None, 300, 0);
            let parted = first + 250;
            let from = saves[parted].1;
            // The other side starts at the very time they parted.
            let mut other = parted;
            let other_first = saves.len();
            for later in [0, 20_000, 40_000] {
                saves.push((vec![other], from + later));
                other = saves.len() - 1;
            }
            saves.push((vec![parted], from + 30_000));
            let tip = saves.len() - 1;
            let ids = plant(&mut repo, &saves);
            let before = repo.store().saves_read();
            assert_eq!(
                only_in(&repo, ids[tip], &[ids[other]]).unwrap(),
                1,
                "history {k}"
            );
            let read = repo.store().saves_read() - before;
            assert!(read <= 12, "history {k}: {read} saves read");
            orders.insert(ids[parted] > ids[other_first]);
        }
        assert_eq!(orders.len(), 2, "both tie orders");
    }

    #[test]
    fn a_save_reached_only_through_a_second_parent_is_held() {
        let (_dir, mut repo) = project();
        let ids = plant(
            &mut repo,
            &[(vec![], 1000), (vec![], 2000), (vec![0, 1], 3000)],
        );
        let (y, m) = (ids[1], ids[2]);
        assert_eq!(only_in(&repo, y, &[m]).unwrap(), 0);
    }

    #[test]
    fn both_sides_of_a_fork_are_counted_apart() {
        let (_dir, mut repo) = project();
        // base, a1, a2, b1
        let ids = plant(
            &mut repo,
            &[
                (vec![], 1000),
                (vec![0], 2000),
                (vec![1], 3000),
                (vec![0], 2500),
            ],
        );
        let (a2, b1) = (ids[2], ids[3]);
        assert_eq!(only_in(&repo, a2, &[b1]).unwrap(), 2);
        assert_eq!(only_in(&repo, b1, &[a2]).unwrap(), 1);
    }

    #[test]
    fn shared_history_is_not_read() {
        let (_dir, mut repo) = project();
        let mut saves = Vec::new();
        chain_after(&mut saves, None, 200, 0);
        let fork = saves.len() - 1;
        let t = saves[fork].1;
        chain_after(&mut saves, Some(fork), 3, t);
        let side_a = saves.len() - 1;
        chain_after(&mut saves, Some(fork), 2, t + 30_000);
        let side_b = saves.len() - 1;
        let ids = plant(&mut repo, &saves);
        let before = repo.store().saves_read();
        assert_eq!(only_in(&repo, ids[side_a], &[ids[side_b]]).unwrap(), 3);
        assert!(repo.store().saves_read() - before <= 7);
    }

    /// Main save `k`'s time.
    fn at(k: usize) -> i64 {
        60_000 * (k as i64 + 1)
    }

    /// Three saves after `from`, dated within a minute of it. Returns the newest.
    fn feature_after(saves: &mut Vec<(Vec<usize>, i64)>, from: usize) -> usize {
        let mut parent = from;
        for j in 1..=3 {
            saves.push((vec![parent], saves[from].1 + 10_000 * j));
            parent = saves.len() - 1;
        }
        parent
    }

    #[test]
    fn a_line_combined_into_another_is_answered_at_once() {
        let (_dir, mut repo) = project();
        let mut saves = Vec::new();
        chain_after(&mut saves, None, 1000, 0);
        let feature = feature_after(&mut saves, 990);
        saves.push((vec![999, feature], at(1000)));
        let main = saves.len() - 1;
        let ids = plant(&mut repo, &saves);
        let before = repo.store().saves_read();
        assert_eq!(only_in(&repo, ids[feature], &[ids[main]]).unwrap(), 0);
        let read = repo.store().saves_read() - before;
        assert!(read <= 6, "{read} saves read");
    }

    /// A main line of 1,000 saves with a side line from save 900, dated before save 950 and
    /// combined at 951, and a feature from save 990. Returns the saves, main's saves and the
    /// feature's tip.
    fn side_line_history() -> (Saves, Vec<usize>, usize) {
        let mut saves: Vec<(Vec<usize>, i64)> = Vec::new();
        let mut main: Vec<usize> = Vec::new();
        for k in 0..1000 {
            let mut parents: Vec<usize> = main.last().copied().into_iter().collect();
            if k == 951 {
                let mut side = main[900];
                for j in 1..=3 {
                    saves.push((vec![side], at(900) + 600_000 * j));
                    side = saves.len() - 1;
                }
                parents.push(side);
            }
            saves.push((parents, at(k)));
            main.push(saves.len() - 1);
        }
        let feature = feature_after(&mut saves, main[990]);
        (saves, main, feature)
    }

    #[test]
    fn an_older_side_line_does_not_send_the_walk_to_the_start() {
        let (_dir, mut repo) = project();
        let (saves, main, feature) = side_line_history();
        let ids = plant(&mut repo, &saves);
        let before = repo.store().saves_read();
        assert_eq!(only_in(&repo, ids[feature], &[ids[main[999]]]).unwrap(), 3);
        let read = repo.store().saves_read() - before;
        assert!(read <= 140, "{read} saves read");
        let before = repo.store().saves_read();
        assert_eq!(
            common_starts(&repo, &[ids[feature]], &[ids[main[999]]]).unwrap(),
            Starts::Found(vec![ids[main[990]]])
        );
        let read = repo.store().saves_read() - before;
        // 14: the two tips, main 998 down to 990, the feature's other two, and main 989, which
        // 990 marks as under a shared save before the walk stops. The side line is never read.
        assert!(read <= 16, "{read} saves read by common_starts");
    }

    fn found(mut ids: Vec<Id>) -> Starts {
        ids.sort();
        Starts::Found(ids)
    }

    #[test]
    fn the_save_two_lines_parted_at_is_their_starting_point() {
        let (_dir, mut repo) = project();
        // base, a1, a2, b1
        let [base, _a1, a2, b1] = plant(
            &mut repo,
            &[
                (vec![], 1000),
                (vec![0], 2000),
                (vec![1], 3000),
                (vec![0], 2500),
            ],
        )[..] else {
            panic!()
        };
        assert_eq!(
            common_starts(&repo, &[a2], &[b1]).unwrap(),
            found(vec![base])
        );
    }

    #[test]
    fn a_line_the_other_holds_is_its_own_starting_point() {
        let (_dir, mut repo) = project();
        let mut saves = Vec::new();
        chain_after(&mut saves, None, 3, 0);
        let [_a, b, c] = plant(&mut repo, &saves)[..] else {
            panic!()
        };
        assert_eq!(common_starts(&repo, &[b], &[c]).unwrap(), found(vec![b]));
        assert_eq!(common_starts(&repo, &[c], &[b]).unwrap(), found(vec![b]));
        assert_eq!(common_starts(&repo, &[c], &[c]).unwrap(), found(vec![c]));
    }

    #[test]
    fn histories_that_never_met_have_none() {
        let (_dir, mut repo) = project();
        let [a, b] = plant(&mut repo, &[(vec![], 1000), (vec![], 2000)])[..] else {
            panic!()
        };
        assert_eq!(common_starts(&repo, &[a], &[b]).unwrap(), found(vec![]));
    }

    /// base, x1 and y1 after it, then x2 and y2 each combining both. Returns all five and z after y2.
    fn criss_cross(repo: &mut Repository) -> [Id; 6] {
        let ids = plant(
            repo,
            &[
                (vec![], 1000),
                (vec![0], 2000),
                (vec![0], 2000),
                (vec![1, 2], 3000),
                (vec![2, 1], 3000),
                (vec![4], 4000),
            ],
        );
        ids.try_into().unwrap()
    }

    #[test]
    fn a_criss_cross_has_two_starting_points_in_id_order() {
        let (_dir, mut repo) = project();
        let [_base, x1, y1, x2, y2, _z] = criss_cross(&mut repo);
        assert_eq!(
            common_starts(&repo, &[x2], &[y2]).unwrap(),
            found(vec![x1, y1])
        );
    }

    #[test]
    fn a_set_of_tips_is_one_side() {
        let (_dir, mut repo) = project();
        let [_base, x1, y1, _x2, _y2, z] = criss_cross(&mut repo);
        assert_eq!(
            common_starts(&repo, &[x1, y1], &[z]).unwrap(),
            found(vec![x1, y1])
        );
        assert_eq!(
            common_starts(&repo, &[x1, y1], &[x1]).unwrap(),
            found(vec![x1])
        );
    }

    #[test]
    fn seventeen_candidates_are_too_many() {
        for (roots, want) in [(17, None), (16, Some(16))] {
            let (_dir, mut repo) = project();
            let mut saves: Vec<(Vec<usize>, i64)> =
                (0..roots).map(|r| (vec![], r as i64)).collect();
            let all: Vec<usize> = (0..roots).collect();
            saves.push((all.clone(), 1000));
            saves.push((all, 1000));
            let ids = plant(&mut repo, &saves);
            let got = common_starts(&repo, &[ids[roots]], &[ids[roots + 1]]).unwrap();
            match want {
                None => assert_eq!(got, Starts::TooMany(17)),
                Some(n) => assert_eq!(got, found(ids[..n].to_vec())),
            }
        }
    }

    #[test]
    fn a_start_under_another_dated_alike_is_dropped() {
        // C <- X <- D, and O and T each with parents [D, C], all at once. D is the one starting
        // point; when C is taken before D, D's mark never reaches C and only the drop removes it.
        let mut extra = HashSet::new();
        for k in 0..16 {
            let (_dir, mut repo) = project();
            let mut saves = filler(k);
            let c = saves.len();
            let (x, d, o, t) = (c + 1, c + 2, c + 3, c + 4);
            saves.extend([
                (vec![], 5),
                (vec![c], 5),
                (vec![x], 5),
                (vec![d, c], 5),
                (vec![d, c], 5),
            ]);
            let ids = plant(&mut repo, &saves);
            let got = common_starts(&repo, &[ids[o]], &[ids[t]]).unwrap();
            assert_eq!(got, found(vec![ids[d]]), "history {k}");
            extra.insert(plain_candidates(&saves, &ids, &[o], &[t]));
        }
        assert_eq!(extra, HashSet::from([1, 2]), "both tie orders");
    }

    #[test]
    fn the_walk_stops_where_the_lines_parted() {
        let mut orders = HashSet::new();
        for k in 0..16 {
            let (_dir, mut repo) = project();
            let mut saves = filler(k);
            chain_after(&mut saves, None, 1000, 0);
            let parted = saves.len() - 1;
            let from = saves[parted].1;
            chain_after(&mut saves, Some(parted), 3, from);
            let ours = saves.len() - 1;
            // The other side starts at the very time they parted.
            let other_first = saves.len();
            let mut theirs = parted;
            for later in [0, 20_000] {
                saves.push((vec![theirs], from + later));
                theirs = saves.len() - 1;
            }
            let ids = plant(&mut repo, &saves);
            let before = repo.store().saves_read();
            assert_eq!(
                common_starts(&repo, &[ids[ours]], &[ids[theirs]]).unwrap(),
                found(vec![ids[parted]]),
                "history {k}"
            );
            let read = repo.store().saves_read() - before;
            assert!(read <= 8, "history {k}: {read} saves read");
            orders.insert(ids[parted] > ids[other_first]);
        }
        assert_eq!(orders.len(), 2, "both tie orders");
    }

    /// Every save reachable from `starts`, themselves included.
    fn ancestors(parents: &[Vec<usize>], starts: &[usize]) -> HashSet<usize> {
        let mut seen = HashSet::new();
        let mut todo = starts.to_vec();
        while let Some(n) = todo.pop() {
            if seen.insert(n) {
                todo.extend(&parents[n]);
            }
        }
        seen
    }

    /// How saves are dated against their parents.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Clock {
        /// Each save later than its parents.
        Forwards,
        Random,
        /// Each save earlier than its parents: every save is looked at before its children.
        Backwards,
        /// Each save as late as its newest parent, often to the millisecond.
        Ties,
        /// Each save at the clock's very ends, or at zero.
        Ends,
    }

    /// The time of the next save after `saves`, with these parents, from a drawn `t`.
    fn dated(clock: Clock, saves: &[(Vec<usize>, i64)], parents: &[usize], t: i64) -> i64 {
        match clock {
            Clock::Forwards => saves.len() as i64,
            Clock::Random => t,
            Clock::Backwards => -(saves.len() as i64),
            Clock::Ties => {
                let newest = parents.iter().map(|&p| saves[p].1).max().unwrap_or(0);
                newest + i64::from(t % 4 == 0)
            }
            Clock::Ends => [i64::MIN, i64::MIN + 1, 0, i64::MAX - 1, i64::MAX][(t % 5) as usize],
        }
    }

    /// A history drawn as each save's picks among earlier saves, and a time: the parents table
    /// and the saves to plant.
    type Shape = Vec<(Vec<prop::sample::Index>, i64)>;
    fn drawn(shape: &Shape, clock: Clock) -> (Vec<Vec<usize>>, Saves) {
        let parents: Vec<Vec<usize>> = shape
            .iter()
            .enumerate()
            .map(|(n, (picks, _))| {
                if n == 0 {
                    return Vec::new();
                }
                let set: BTreeSet<usize> = picks.iter().map(|p| p.index(n)).collect();
                set.into_iter().collect()
            })
            .collect();
        let mut saves: Vec<(Vec<usize>, i64)> = Vec::new();
        for (p, (_, t)) in parents.iter().zip(shape) {
            let time = dated(clock, &saves, p, *t);
            saves.push((p.clone(), time));
        }
        (parents, saves)
    }

    /// The walk's candidates, marked as plainly as can be: each turn a scan for the newest
    /// waiting save, until every waiting save is under a common one.
    fn plain_candidates(
        saves: &[(Vec<usize>, i64)],
        ids: &[Id],
        ours: &[usize],
        theirs: &[usize],
    ) -> usize {
        let n = saves.len();
        let (mut marks, mut spread) = (vec![0u8; n], vec![0u8; n]);
        for &o in ours {
            marks[o] |= OURS;
        }
        for &t in theirs {
            marks[t] |= THEIRS;
        }
        loop {
            let waiting: Vec<usize> = (0..n).filter(|&s| marks[s] != spread[s]).collect();
            if waiting.iter().all(|&s| marks[s] & BELOW != 0) {
                break;
            }
            let s = *waiting
                .iter()
                .max_by_key(|&&s| (saves[s].1, ids[s]))
                .unwrap();
            spread[s] = marks[s];
            let give = if marks[s] & BOTH == BOTH {
                marks[s] | BELOW
            } else {
                marks[s]
            };
            for &p in &saves[s].0 {
                marks[p] |= give;
            }
        }
        marks.iter().filter(|&&m| m == BOTH).count()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]
        #[test]
        fn common_starts_agrees_with_a_full_walk(
            shape in prop::collection::vec(
                (prop::collection::vec(any::<prop::sample::Index>(), 0..=3), 0..1000i64),
                1..=40,
            ),
            ours in prop::collection::vec(any::<prop::sample::Index>(), 1..=3),
            theirs in prop::collection::vec(any::<prop::sample::Index>(), 1..=3),
            clock in prop_oneof![
                Just(Clock::Forwards),
                Just(Clock::Ties),
                Just(Clock::Random),
                Just(Clock::Ends),
            ],
            // More first saves than a combine takes, each side holding all of them as parents
            // through one save or two.
            crowd in prop::option::weighted(0.125, (17..=20usize, any::<bool>(), any::<bool>())),
        ) {
            let (mut parents, mut saves) = drawn(&shape, clock);
            let mut ours: Vec<usize> = ours.iter().map(|o| o.index(saves.len())).collect();
            let mut theirs: Vec<usize> = theirs.iter().map(|t| t.index(saves.len())).collect();
            if let Some((roots, ours_split, theirs_split)) = crowd {
                let first = saves.len();
                for r in 0..roots {
                    let time = dated(clock, &saves, &[], r as i64 * 37);
                    parents.push(Vec::new());
                    saves.push((Vec::new(), time));
                }
                let all: Vec<usize> = (first..first + roots).collect();
                for (split, side) in [(ours_split, &mut ours), (theirs_split, &mut theirs)] {
                    let groups = if split {
                        let (a, b) = all.split_at(roots / 2);
                        vec![a.to_vec(), b.to_vec()]
                    } else {
                        vec![all.clone()]
                    };
                    for group in groups {
                        let time = dated(clock, &saves, &group, 500);
                        parents.push(group.clone());
                        saves.push((group, time));
                        side.push(saves.len() - 1);
                    }
                }
            }

            let common: HashSet<usize> = ancestors(&parents, &ours)
                .intersection(&ancestors(&parents, &theirs))
                .copied()
                .collect();
            let (_dir, mut repo) = project();
            let ids = plant(&mut repo, &saves);
            let mut answer: Vec<Id> = common
                .iter()
                .filter(|&&c| {
                    !common
                        .iter()
                        .any(|&d| d != c && ancestors(&parents, &[d]).contains(&c))
                })
                .map(|&c| ids[c])
                .collect();
            answer.sort();
            let candidates = plain_candidates(&saves, &ids, &ours, &theirs);

            let side = |tips: &[usize]| tips.iter().map(|&s| ids[s]).collect::<Vec<Id>>();
            let got = common_starts(&repo, &side(&ours), &side(&theirs)).unwrap();
            match &got {
                Starts::Found(got) => {
                    prop_assert!(candidates <= MAX_STARTS);
                    prop_assert_eq!(got, &answer);
                }
                Starts::TooMany(n) => {
                    prop_assert!(candidates > MAX_STARTS);
                    prop_assert_eq!(*n, candidates);
                }
            }
            // Only saves dated after their parents make the candidates the answer itself; with
            // ties there can be more, which the match above already checks against.
            if clock == Clock::Forwards {
                prop_assert_eq!(
                    matches!(got, Starts::TooMany(_)),
                    answer.len() > MAX_STARTS
                );
            }
        }

        #[test]
        fn only_in_agrees_with_a_full_walk(
            shape in prop::collection::vec(
                (prop::collection::vec(any::<prop::sample::Index>(), 0..=3), 0..1000i64),
                1..=40,
            ),
            tip in any::<prop::sample::Index>(),
            others in prop::collection::vec(any::<prop::sample::Index>(), 0..=3),
            clock in prop_oneof![
                Just(Clock::Forwards),
                Just(Clock::Random),
                Just(Clock::Backwards),
                Just(Clock::Ties),
            ],
            // Random times of a chain of new saves after the tip, its last one held by the others.
            under in prop::option::of(prop::collection::vec(0..1000i64, 1..=3)),
        ) {
            let (mut parents, mut saves) = drawn(&shape, clock);
            let tip = tip.index(saves.len());
            let mut others: Vec<usize> = others.iter().map(|o| o.index(saves.len())).collect();
            if let Some(times) = under {
                let mut parent = tip;
                for t in times {
                    parents.push(vec![parent]);
                    let time = dated(clock, &saves, &[parent], t);
                    saves.push((vec![parent], time));
                    parent = saves.len() - 1;
                }
                others.push(parent);
            }

            let mine = ancestors(&parents, &[tip]);
            let theirs = ancestors(&parents, &others);
            let oracle = mine.difference(&theirs).count() as u64;

            let (_dir, mut repo) = project();
            let ids = plant(&mut repo, &saves);
            let other_ids: Vec<Id> = others.iter().map(|&o| ids[o]).collect();
            let got = only_in(&repo, ids[tip], &other_ids).unwrap();
            if matches!(clock, Clock::Forwards | Clock::Ties) {
                prop_assert_eq!(got, oracle);
            } else {
                prop_assert_eq!(got == 0, oracle == 0);
                prop_assert!(got >= oracle);
            }
        }
    }
}
