# Milestone 8 ("Combine"): Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task by task. Steps use checkbox (`- [ ]`) syntax for tracking. Every subagent runs on Opus.

**Goal:** `opalro combine <target>` brings another line's work into the line you're on when nothing conflicts: it finds where the two parted, merges the two versions of the project file by file and text line by line, follows files that moved, and only moves the line forward when that is all it takes. A combine that would conflict stops, names the files, and changes nothing. `save` notes the moves it is sure of, and `history`, `show`, `goto` and every place a save is asked for reach the saves a combine brought in.

**Architecture:** A combine is one diary step, `combine`, made the way `goto` makes a switch: the combined save is stored first, then the folder is switched to it under `switch::locked` (plan, collisions stop it, safety copies, journal, write-out). There is no "combine in progress" state, so undo, redo, `tidy` and crash recovery already cover it. `ancestry.rs` gains `common_starts`; a new module `combine.rs` holds the command, with `combine/text.rs` (line by line) and `combine/tree.rs` (entry by entry); a new module `moves.rs` holds the one rule for "the same contents left one path and arrived at one other", used by `save` and by `combine`; `rules.rs` gains the matcher for `[[rule]]` patterns. Nothing new is stored: move notes and saves with two parents are format 1 already, and a project used with this build still opens in v0.1.

**Tech Stack:** Rust 1.99.1 and the crates already in the workspace (`similar` for the line comparison). No new dependency.

**Spec:** [decision 164](../decisions/164-where-two-lines-parted.md) (where two lines parted), [decision 165](../decisions/165-combine-the-command.md) (`combine`, the command), [decision 166](../decisions/166-merging-two-versions-of-the-project.md) (merging two versions of the project), [decision 167](../decisions/167-merging-text-line-by-line.md) (merging text line by line), [decision 168](../decisions/168-moves-in-saves-and-in-combines.md) (moves in saves and in combines), [decision 169](../decisions/169-saves-reached-through-a-combine.md) (saves reached through a combine), with the rulings made before the build folded in below (R1 to R6 and the rest, in [milestone 8's rulings](2026-11-14-milestone-8-rulings.md) under "Before the build"); decisions 006, 007, 012, 050, 058, 075, 105, 114, 116, 157 and 160 for background; [storage-format.md](../storage-format.md) §3 (limits), §4 (the save and its move notes), §12 (`conflict-markers`); [terminal.md](../terminal.md) and [json.md](../json.md). Each task that adds words a person sees writes them into terminal.md, json.md and the guide in the same commit.

**Rulings made while building:** [2026-11-14-milestone-8-rulings.md](2026-11-14-milestone-8-rulings.md), kept as the build goes, verbatim. Every ruling's premise is checked in the code before it is made.

**Branches:** tasks are built on `milestone-8`, made from `v0.2` once milestone 7 is merged into it. It is merged into `v0.2` after the red-team session (Task 14). `main` is never touched.

## Global Constraints

- Everything in CLAUDE.md applies.
  - `#![forbid(unsafe_code)]` in the engine and the command-line crate. **No task adds `unsafe`.**
  - No `unwrap`, `expect` or `panic` outside tests.
  - One place per rule: starting points and ancestry only in `ancestry.rs`; which files a `[[rule]]` matches only in `rules.rs`; what counts as text and the line comparison (8,000 bytes, 64 MiB, 1,000,000 lines, the two-second deadline, Myers through `similar`) only in `changes.rs`; "same contents left one path and arrived at one other" only in `moves.rs`; the merge rules only in `combine/`; names two systems can't tell apart only in `names.rs`; Git's words only in `git_words.rs`; colour, symbols and widths only in `style.rs`.
- **Nothing new on disk.** No task changes an encoding, a limit, `.opalro/format` or a cache's format. Move notes and two-parent saves are written as storage-format.md §4 already describes them. A task that finds it needs more stops and says so.
- **Written clean** (decision 131). Nothing is written from another program's source. The starting-point walk, the text merge and the tree merge are written from this plan and the decisions, and checked from the outside against `git merge-base --all`, `git merge-file` and `git merge`.
- Code comments are short and human: the why in a line or two, no process words, no task or decision numbers in code.
- **Everything read from a save is untrusted** (R5). Walks over saves are iterative, with no call per folder level or per save; each version of the project is held to a save's limits on its own; counts and sizes are checked before memory is taken for them; a move note that names a path that doesn't fit is ignored. Every refusal names what it is about: the file, the save, the line.
- **Untrusted text is escaped exactly as today** (`opal_core::text::escape`, per piece, at the edge): paths, line names, messages, wherever they are printed, in outcome lines and error lists. **A stored tag's name** is written as text only through `tags::shown_name` (as it is when it fits the line-name rules, else quoted and escaped). **No hint puts a stored name, a path or any other text from the project inside a command.**
- **Names are read by the one resolver** (`lines::resolve_target`, built on `history::find`): a line, a server line, a tag, a change ID, a fingerprint, in that order; a name that is both a line (or server line) and a tag at two saves is refused there already (`ResolveError::LineOrTag`, `ServerLineOrTag`). `combine` adds no reading of its own.
- **A combine that stops for conflicts, for too many starting points or for unrelated histories stores nothing**: what it built is in its own pack set, written to a temporary file in `.opalro/tmp` that is removed when the set is dropped (or by the next command after a crash), so nothing becomes part of the store. Only listings, and the texts of inner combines (at most `MAX_HELD_TEXT`, 256 MiB), are held in memory meanwhile (Task 4 says why). The one exception: past 1,048,576 objects a pack set moves a full pack into the store at once, which a refusal then leaves named by nothing. A result past a save's limits is found before anything is stored, so it stores nothing either. Only a combine that stops for unsaved work in the way, which needs the combined save stored for the switch's plan, may leave records nothing names, which `tidy` frees, as `shelf` and `goto` can. "Nothing was changed" in every message means the lines, the tags, where the project is, and the folder.
- **No message promises a milestone or a version.**
- **`--json` never asks and never carries colour, symbols, progress or "Next:" lines.** `opalro_json` stays `1`: documents and fields are added, none is changed (decision 117).
- **`combine` records one diary entry** whose command is exactly `combine`, with the folder change its switch made, and **finishes a stopped switch first**, through `switch::locked`, passing on the warnings that made.
- **Exit codes:** 0 done (also "nothing to combine"), 1 failed or stopped (a conflict too), 2 a command line Opalro couldn't use.
- **Subtle algorithms get three guards** (`common_starts`, the text merge, the tree merge, following moves): a property test against a brute-force or independent answer, with a generator that is deliberately hostile (saves dated alike are common in imported history; clocks run backwards; lines repeat); a comparison with Git; and the implementer breaks the code on purpose in the ways the task names, runs the tests, reports which test caught each break, and puts the code back.
- **Test names that read as IDs:** no test resolves a line or tag name made only of the letters `k` to `z`, or of 8 or more hex digits: it can be the start of a save's ID and the test then fails at random. Use `side`, `wall-run`, `v1.0`.
- **One writer at a time:** tasks run strictly one after another, and no two agents write in the working tree at once. `git add` names paths; never `git add -A` while anyone else is in the folder.
- **Values:**

  | What | Value |
  |---|---|
  | Common starting points per level | at most 16 (`ancestry::MAX_STARTS`); more candidates is `Starts::TooMany`, refused |
  | Inner combines in one `opalro combine` | at most 64 (`combine::MAX_INNER`); more is refused |
  | Text a file may be to merge line by line | each version: no zero byte in its first 8,000 bytes, at most 64 MiB (`changes::MAX_LINES_SIZE`), at most 1,000,000 lines (`changes::MAX_LINES`) |
  | Time per line comparison | `changes::DEADLINE`, 2 s; past it the answer is coarser, never wrong |
  | `conflict-markers` for a path | what the last `[[rule]]` matching it and giving that key says; `true` when none does; patterns match ignoring ASCII letter case, on every system |
  | Move notes in one save | at most 100,000 (`save::MAX_MOVE_NOTES`): the first in order of where the file went |
  | Conflicting files named in one refusal | the first 100 in path order, then `… and N more`; never an unbounded list |
  | A combined save's message | the one given, else `Combine <target> into <line>` |
  | A save named on screen | its short change ID with its message's first line in quotes, as `goto` names one |
  | A save's other parents in `history` | `[combined <id>]`, the first 12 hex digits of its fingerprint |

- **Local runs on this machine:** `export PATH="$HOME/.cargo/bin:/c/Program Files/GitHub CLI:$PATH"` and `export OPALRO_CONFIG_DIR="$(mktemp -d)"` before any test. Build with `-j 2`, test with `-- --test-threads=2`, one test file at a time (`cargo test -j 2 -p opal-vcs --all-features --test combine -- --test-threads=2`; engine tests: `cargo test -j 2 -p opal-vcs-core --all-features <filter> -- --test-threads=2`). Run `cargo fmt` after every edit and `cargo clippy -j 2 --workspace --all-targets --all-features -- -D warnings` before every commit. Unix-only code can't be checked here: watch CI. Test files live in a temporary folder, never in the repository.
- Docs are edited with the Edit tool or `sed`, never Python in text mode; every edited doc is checked with `tr -cd '\r' < file | wc -c`, which must print 0.
- The untracked `.codex/` and `AGENTS.md` are not ours: never staged, edited or deleted.
- One commit per task plus fix commits, with no Claude attribution, on `milestone-8`. The controller pushes at task boundaries; CI runs on Windows, macOS and Linux, and a task is done only once they pass. `v0.2` comes in by merge, never by rebase.

## Review Focus

1. **Histories that crossed, with clocks running backwards.** Two lines each combined into the other (a criss-cross), imported from Git with some commits dated before their parents. Expected: the starting points are exactly `git merge-base --all`'s; the two are combined with each other first, so a change one line reverted after both had taken it stays reverted. Tests: Task 1 `common_starts_agrees_with_a_full_walk`, Task 7 `a_criss_cross_keeps_a_revert_that_one_starting_point_would_lose`.
2. **Unsaved work in the way.** A changed file the combine would write, and another it wouldn't. Expected: with the first, the combine stops before storing a step, the folder and the line as they were, and the error names the file and says `opalro combine` again; with only the second, the combine happens and the work comes along. Tests: Task 7 `unsaved_work_where_the_combine_writes_stops_it`, `unsaved_work_elsewhere_comes_along`.
3. **A file moved on one line and edited on the other.** `Player.cs` moved into `Characters/` on `main`, saved alone; fixed in place on `wall-run`. Expected: the fix lands in `Characters/Player.cs` and nothing is left at `Player.cs`; moved on one side and deleted on the other is a conflict, not a quiet deletion. Tests: Task 6 `a_change_to_a_moved_file_lands_where_it_went`, `moved_and_removed_conflicts`.
4. **Two names one folder can't hold.** `Readme.md` added on one line and `README.md` on the other. Expected: refused, naming both paths, nothing changed; a pair that one side already held (from an import) is left as it is. Tests: Task 4 `names_two_sides_bring_that_clash_stop_the_combine`, `a_clashing_pair_one_side_already_held_is_kept`.
5. **A crash in the middle.** A combine stopped at every write. Expected: the next command finishes it or it never happened; running the same combine again then says `Nothing to combine`. Test: Task 7 `a_combine_stopped_at_every_step_is_finished_or_never_happened`.

---

## File structure

| File | Responsibility |
|---|---|
| `crates/opal-vcs-core/src/ancestry.rs` | `common_starts` beside `only_in` |
| `crates/opal-vcs-core/src/rules.rs` | `RuleMatcher`: which `[[rule]]` decides `conflict-markers` for a path |
| `crates/opal-vcs-core/src/changes.rs` | The text limits and line comparison, opened to the merge (`pub(crate)`) |
| `crates/opal-vcs-core/src/combine.rs` (new) | The command: the three cases, the starting points and their budget, the switch; the shared types |
| `crates/opal-vcs-core/src/combine/text.rs`, `tree.rs`, `tests.rs` (new) | Text line by line; the project entry by entry; the engine tests |
| `crates/opal-vcs-core/src/moves.rs` (new) | The move rule for `save`'s notes and a combine's pairing; following notes along a side |
| `crates/opal-vcs-core/src/snapshot.rs` | `save` notes moves |
| `crates/opal-vcs-core/src/switch.rs` | `landing`, the target read again under the lock, shared by `goto` and `combine` |
| `crates/opal-vcs-core/src/history.rs` | Finding a save through every parent; history from another place |
| `crates/opal-vcs/src/main.rs`, `combine_output.rs` (new), `help.rs`, `git_words.rs`, `history_output.rs`, `show_output.rs`, `output.rs` | The command line and its words |
| `crates/opal-vcs/tests/combine.rs` (new), `merge_vs_git.rs` (new), `model.rs`, `import_vs_git.rs`, `support/vs_git.rs`, `json_formats.rs`, `cli.rs`, `terminal.rs`, `git_words.rs`, `undo.rs` | Tests |

---

### Task 1: The common starting points

**Files:**
- Modify: `crates/opal-vcs-core/src/ancestry.rs` (code and its `tests` module)
- Modify: `crates/opal-vcs/tests/support/vs_git.rs`, `crates/opal-vcs/tests/import_vs_git.rs`

**Interfaces:**
- Consumes: `repo.store().save(&id, repo.signatures_allowed())` (a save's `parents` and `saver.time_ms`); `only_in` for the exact drop.
- Produces:

```rust
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

/// The saves both sides have in their history that no other such save has in its history;
/// each side is a set of tips. Exact whatever the saves' clocks say, whenever it is `Found`.
pub fn common_starts(repo: &Repository, ours: &[Id], theirs: &[Id]) -> Result<Starts, RepoError>
```

**The walk** (R1). The signature and tests don't determine it, so here it is:

- Marks per save: `OURS = 1`, `THEIRS = 2`, `BELOW = 4` ("under a save both sides have"). A node per save met, as `only_in` keeps one: `marks`, `spread` (the marks it last gave its parents; 0 before), its saver time and parents, read once when first met.
- Every tip of `ours` starts with `OURS`, every tip of `theirs` with `THEIRS` (a save in both starts with both).
- A save **waits** while `marks != spread`. A max-heap of `(saver time, id)` gives the newest waiting save, ties by ID, as `only_in` does. Each turn: pop it, set `spread = marks`, and give each parent its marks, adding `BELOW` when the save holds both `OURS` and `THEIRS`. `BELOW` travels on like any mark.
- **Go on while some waiting save lacks `BELOW`.** Keep a count of such saves up to date as marks change, as `only_in`'s `Tally` does, rather than scanning the heap.
- **Candidates:** the saves that hold `OURS` and `THEIRS` and not `BELOW`, whether or not they were ever popped.
- The candidates are counted first: more than `MAX_STARTS` is `TooMany(count)`, without the drop (that bounds its cost). Otherwise, with two or more, drop each candidate `c` for which some other candidate `d` has `only_in(repo, c, &[d])? == 0`, and give the rest as `Found`, sorted by ID. With clocks that run backwards, extra candidates can make `TooMany` where the true starting points are 16 or fewer: the combine is then refused, which is safe, never wrong.
- No recursion. A save that can't be read is the store's error, passed on.

Why it is exact: a mark only lands on a save in that side's history; `BELOW` only lands on a save under a common one; every path from a tip to a true starting point runs through saves only one side has, which wait without `BELOW` and keep the walk going until the mark arrives. With saves dated no earlier than their parents the candidates are already exact; with clocks that run backwards there can be extra candidates, which the drop removes exactly (`only_in`'s zero is exact whatever the clocks).

- [ ] **Step 1: Write the failing tests** in `ancestry.rs`'s `tests`, building histories with `plant` and `chain_after` as the `only_in` tests do:

  (`Found(..)` below is written as the list it holds.)

  - `the_save_two_lines_parted_at_is_their_starting_point`: `base ← a1 ← a2`, `base ← b1`: `common_starts([a2], [b1]) == Found([base])`.
  - `a_line_the_other_holds_is_its_own_starting_point`: `a ← b ← c`: `([b], [c]) == [b]`, `([c], [b]) == [b]`, `([c], [c]) == [c]`.
  - `histories_that_never_met_have_none`: two first saves: `Found([])`.
  - `a_criss_cross_has_two_starting_points_in_id_order`: `base ← x1`, `base ← y1`, `x2 = [x1, y1]`, `y2 = [y1, x1]`: `([x2], [y2])` is `[x1, y1]` sorted by ID.
  - `a_set_of_tips_is_one_side`: the criss-cross above plus `z = [y2]`: `([x1, y1], [z]) == [x1, y1]` sorted; `([x1, y1], [x1]) == [x1]`.
  - `seventeen_candidates_are_too_many`: 17 first saves `r0..r16`, `o` and `t` each with all 17 as parents: `TooMany(17)`; with 16 of them, `Found` holds the 16.
  - `the_walk_stops_where_the_lines_parted`: 1,000 saves in a chain, then 3 on one side and 2 on the other: the answer is the fork, and `saves_read` grows by at most 8. Run it over 16 filler histories so both tie orders occur, with the other side starting at the very time they parted (as `a_tie_where_two_lines_parted_does_not_run_to_the_start` does).
  - `an_older_side_line_does_not_send_the_walk_to_the_start`: `only_in`'s history of that name: `common_starts([feature], [main[999]]) == [main[990]]`, `saves_read` grows by at most 140.
  - `common_starts_agrees_with_a_full_walk` (proptest, 256 cases): up to 40 saves, each with 0 to 3 parents among earlier ones; `ours` and `theirs` each 1 to 3 tips. Four clocks: `Forwards` (each save later than its parents), `Ties` (as late as its newest parent, often equal), `Random` (0..1000), `Ends` (each time drawn from `i64::MIN`, `i64::MIN + 1`, `0`, `i64::MAX - 1`, `i64::MAX`). One case in eight is built to have more than 16 true starting points (17 to 20 first saves, each tip side holding all of them as parents through one or two saves). The oracle: `common` = ancestors of `ours` ∩ ancestors of `theirs` (from the parents table); the answer = the members of `common` in no other member's history, sorted; and, as a second oracle, the walk's candidates by brute force: R1's marking re-run plainly in the test over the parents table (a linear scan for the newest waiting save, no heap, no tally), its candidates taken at the end (for `Forwards` and `Ties` they are the answer itself). Assert: `Found(got)` only when those candidates are 16 or fewer, and then `got == answer`; `TooMany(n)` only when they are more than 16, and then `n` is their count. For `Forwards` and `Ties`, `TooMany` exactly when the answer has more than 16.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs-core --all-features ancestry -- --test-threads=2`
Expected: does not compile (`common_starts` not found).

- [ ] **Step 3: Implement `Starts` and `common_starts`** as described.

- [ ] **Step 4: Run the tests and see them pass.** Then break it on purpose three ways, run the tests after each, and report which test caught it: give `BELOW` to every parent of a save holding one mark; stop when no save with exactly one mark waits; leave out the exact drop.

- [ ] **Step 5: Compare with Git.** In `vs_git.rs`, take the choice of pairs out of `assert_ancestry_matches_git` into `fn tip_pairs(repo, dir) -> Vec<((String, Id, String), (String, Id, String))>` (name, save, Git commit), used by both, and add:

```rust
/// Asserts that `common_starts([a], [b])`, as the Git commits its saves record, is what
/// `git merge-base --all a b` prints, for the pairs `assert_ancestry_matches_git` takes.
/// Returns how many pairs it compared.
pub fn assert_starts_match_git(dir: &Path) -> usize
```

  Both lists are sorted before comparing; when Git prints more than 16, `common_starts` must be `TooMany`; when it is `TooMany` and Git prints 16 or fewer, the pair is counted and printed apart (clocks that run backwards; `dated_before_a_parent` must say so for it). Call it from every test in `import_vs_git.rs` that calls `assert_ancestry_matches_git`, asserting it returns that test's pair count (64, 0, 2, 6, 2, 0, as pinned there), and from the ignored `a_real_repository_matches_git`. Add `a_criss_cross_matches_git`: a Git history where `left` and `right` are each merged into the other, then one more commit on each: `git merge-base --all left right` prints two commits and the pair count is pinned.

Run: `cargo test -j 2 -p opal-vcs --all-features --test import_vs_git -- --test-threads=2`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/opal-vcs-core/src/ancestry.rs crates/opal-vcs/tests/support/vs_git.rs crates/opal-vcs/tests/import_vs_git.rs
git commit -m "Find where two lines parted: every common starting point, exact whatever the clocks say, and the same as git merge-base --all"
```

### Task 2: Which files a `[[rule]]` matches

**Files:**
- Modify: `docs/storage-format.md` §12 (first, before the code), `crates/opal-vcs-core/src/rules.rs` (code and tests)

**Interfaces:**
- Consumes: `rules::{Rules, Rule}`; `ignore::{Rules as IgnoreRules, Matcher, Decision}`; `scope::ancestors`; `drive::Drive`.
- Produces:

```rust
/// The project's `[[rule]]` tables, each turned into a matcher once.
pub(crate) struct RuleMatcher { /* one (ignore::Matcher, Rule's settings) per [[rule]] */ }

impl RuleMatcher {
    pub(crate) fn new(rules: &Rules) -> RuleMatcher;
    /// Whether a file at `path` (`a/b/c`) may be merged line by line: what the last rule that
    /// matches it and gives `conflict-markers` says, `true` when none does.
    pub(crate) fn conflict_markers(&self, path: &str) -> bool;
}
```

- **A rule matches a file** when its `files`, read as the lines of one `.opalroignore` at the top, would keep the file out: the file itself `decide(path, false) == Ignored`, or any folder above it `decide(folder, true) == Ignored` (a `!` line re-includes as it does there; `assets/` matches every file below `assets`).
- **ASCII letter case is ignored, on every system** (a stored result must not depend on the computer): each `Matcher` is made with `Drive { ignores_case: true, composes_unicode: false }`, a constant beside `RuleMatcher`, never with `repo.drive()`. Only ASCII letters fold, as in ignore files; other characters compare byte for byte.
- A pattern holding `\n` or `\r` matches nothing: it is left out before the patterns are joined into one rule file.
- The rules are `repo.rules().rules`, as the last `lock_for_change` read them from the folder (decision 114).
- storage-format.md §12 gains, after "Rules are read top to bottom": `A rule's patterns match a file as the lines of one .opalroignore at the top would keep it out (a file below a matched folder matches), ignoring ASCII letter case on every system; a pattern holding a line break matches nothing.`

- [ ] **Step 1: Write the failing tests** in `rules.rs`:

  - `no_rule_means_markers`: no rules: `conflict_markers("a.txt")` is true.
  - `the_last_matching_rule_that_gives_the_key_decides`: rules `[*.unity → false]`, `[Assets/Keep/*.unity → true]`, `[*.unity, lockable only]`: `Assets/a.unity` false, `Assets/Keep/b.unity` true, `a.txt` true.
  - `patterns_match_as_opalroignore_lines_do`: `*.meta` matches `a/b/c.meta`; `/top.txt` matches `top.txt`, not `a/top.txt`; `art/**/*.png` matches `art/x/y.png`; `["*.unity", "!keep.unity"]` doesn't match `keep.unity`.
  - `a_folder_pattern_matches_the_files_below_it`: `Assets/Scenes/` matches `Assets/Scenes/x/y.unity`.
  - `ascii_letter_case_is_ignored_everywhere`: `*.UNITY` matches `Main.unity` and `*.unity` matches `MAIN.UNITY`, whatever `repo.drive()` says; `É.txt` doesn't match `é.txt`.
  - `a_pattern_holding_a_line_break_matches_nothing`: `["a\nb"]` matches neither `a` nor `b`.
  - `the_game_presets_scenes_prefabs_and_meta_files_get_no_markers`: the Game preset's rules parsed: `Assets/Main.unity`, `Assets/P.prefab`, `Assets/P.prefab.meta` false; `Assets/Player.cs` true.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs-core --all-features rules -- --test-threads=2`
Expected: does not compile.

- [ ] **Step 3: Edit storage-format.md §12, then implement `RuleMatcher`.**

- [ ] **Step 4: Run the tests and see them pass**

- [ ] **Step 5: Commit**

```bash
git add crates/opal-vcs-core/src/rules.rs docs/storage-format.md
git commit -m "Tell which [[rule]] in opalro.toml decides conflict-markers for a file, matching as .opalroignore does, ignoring letter case everywhere"
```

### Task 3: Merging text line by line

**Files:**
- Create: `crates/opal-vcs-core/src/combine.rs` (for now: `mod text;`, `pub use text::{merge_text, Text};` and `NotMerged`), `crates/opal-vcs-core/src/combine/text.rs`
- Modify: `crates/opal-vcs-core/src/lib.rs` (`pub mod combine;`), `crates/opal-vcs-core/src/changes.rs`
- Create: `crates/opal-vcs/tests/merge_vs_git.rs`

**Interfaces:**
- Consumes, from `changes.rs`, made `pub(crate)` and used as they are: `is_binary`, `line_count`, `lines_of`, `deadline()`, `file_lines`, `only_line_endings`; and one new piece taken out of `file_lines` so both use it:

```rust
/// Myers line by line between `old` and `new` with a deadline: the operations, and whether it
/// ran past the deadline (its answer is then coarser, never wrong).
pub(crate) fn line_ops(old: &[&[u8]], new: &[&[u8]], deadline: Duration) -> (Vec<DiffOp>, bool)
```

- Produces:

```rust
/// Why both sides' changes to a file aren't merged.
pub enum NotMerged {
    /// Text, but changes from the two sides overlap or touch.
    Overlap { endings_only: bool },
    Binary, TooBig, TooManyLines,
    /// Its `conflict-markers` rule is `false`.
    NoMarkers,
    /// A link, a project link, or not a file on every side.
    NotFiles,
}
pub enum Text { Merged(Vec<u8>), Not(NotMerged) }
/// The three-way merge of `base`, `ours` and `theirs` line by line;
/// the `bool` says a comparison ran past `deadline`. Each side is at most `MAX_LINES_SIZE`
/// bytes: the caller reads them so.
pub fn merge_text(base: &[u8], ours: &[u8], theirs: &[u8], deadline: Duration) -> (Text, bool)
```

  `merge_text` is `pub` so that `merge_vs_git.rs` can call it. It keeps nothing: the merged bytes it returns are its caller's, which writes them into the combine's pack set at once and drops them (Task 4), so a combine holds one file's texts at a time, not all of them.

- **The rules** ([decision 167](../decisions/167-merging-text-line-by-line.md)): any side with a zero byte in its first 8,000 bytes is `Binary`; more than 1,000,000 lines on any side is `TooManyLines`. Lines end after each `\n` and are compared byte for byte, endings included. Compare `base` with `ours` and `base` with `theirs` (`line_ops`); each gives changed stretches of `base` (an insertion is an empty stretch at its place). Stretches from the two sides that **overlap or touch** (`a.start <= b.end && b.start <= a.end`) are joined into one region, transitively. A region one side alone changed takes that side's lines; a region both changed is settled when both sides' lines for it are byte-equal; otherwise the file is `Overlap`, with `endings_only` true when `ours` and `theirs` differ only in line endings (`only_line_endings` on `file_lines(ours, theirs, …)`).

- [ ] **Step 1: Write the failing tests** in `text.rs`:

  - `edits_far_apart_are_both_kept`: base `a\nb\nc\nd\ne\n`, ours changes `b`, theirs changes `d`: `Merged` holds both.
  - `edits_that_touch_conflict`: ours changes line 2, theirs line 3: `Overlap { endings_only: false }`.
  - `two_insertions_at_one_place_conflict_unless_equal`: different: `Overlap`; equal: `Merged` with them once.
  - `the_same_change_on_both_sides_is_taken_once`.
  - `line_endings_are_bytes`: ours turns every `\n` into `\r\n`, theirs edits line 1: `Overlap { endings_only: false }`; ours `a\r\nb\r\n`, theirs `a\nb\r\n` from base `a\nb\n`: `Overlap { endings_only: true }`.
  - `a_zero_byte_in_the_first_8000_bytes_is_not_text` (on each side in turn); `a_zero_byte_after_8000_bytes_is_text`.
  - `more_than_a_million_lines_is_not_merged`.
  - `a_last_line_without_a_line_feed_merges`.
  - `a_comparison_past_its_deadline_is_coarser_never_wrong`: `Duration::ZERO` on a 2,000-line file with two far-apart edits: the flag is true, and the answer is the full-time answer or `Overlap`.
  - `text_merge_properties` (proptest, 512 cases; texts of 0 to 30 lines from an alphabet of 4 lines, so that lines repeat on purpose, `\n` and `\r\n` endings mixed, a last line with or without `\n`): `merge(b, o, o) == Merged(o)`; `merge(b, b, t) == Merged(t)`; `merge(b, o, b) == Merged(o)`; `merge(b, o, t)` and `merge(b, t, o)` are both `Merged` with the same bytes or both `Not`.

  In `crates/opal-vcs/tests/merge_vs_git.rs`:

  Each case writes the three to a temporary folder and runs `git merge-file -p ours base theirs`.

  - `text_merge_matches_git_merge_file_on_unique_lines` (proptest, 256 cases): a base of 5 to 40 unique lines (`line 17\n`), each side made by 0 to 4 edits (replace, delete or insert a run of fresh unique lines). In every case, `Merged(bytes)` exactly when Git exits 0, and then `bytes` is Git's stdout. Count both outcomes; assert each occurred at least 25 times.
  - `text_merge_and_git_merge_file_on_repeating_lines` (proptest, 256 cases): the same, with lines drawn from an alphabet of 4 so they repeat. When both are clean, the bytes are equal, in every case. Cases where one is clean and the other isn't are counted and printed with their texts; the count is pinned at the number the implementer reports, with two examples, in the task's report. If "both clean, the same bytes" fails, the implementer stops and reports: that is a ruling for the controller, not something to work around.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs-core --all-features combine -- --test-threads=2`
Expected: does not compile.

- [ ] **Step 3: Implement** `line_ops` (and `file_lines` through it), `merge_text`, `Text` and `NotMerged`.

- [ ] **Step 4: Run the tests and see them pass**, then `--test merge_vs_git`. A difference from Git on unique lines is investigated against [decision 167](../decisions/167-merging-text-line-by-line.md), not silenced: report it. Then break it three ways and report which test caught each: join only stretches that overlap, not those that touch; settle a region both changed by taking `ours`; compare lines with their `\r` taken off.

- [ ] **Step 5: Commit**

```bash
git add crates/opal-vcs-core/src/lib.rs crates/opal-vcs-core/src/combine.rs crates/opal-vcs-core/src/combine crates/opal-vcs-core/src/changes.rs crates/opal-vcs/tests/merge_vs_git.rs
git commit -m "Merge text line by line: changes that overlap or touch conflict, line endings are bytes, and the result is git merge-file's"
```

### Task 4: Merging two versions of the project, entry by entry

**Files:**
- Create: `crates/opal-vcs-core/src/combine/tree.rs`, `crates/opal-vcs-core/src/combine/tests.rs`
- Modify: `crates/opal-vcs-core/src/combine.rs`, `crates/opal-vcs-core/src/diff.rs` (`again` becomes `pub(crate)`), `crates/opal-vcs-core/src/changes.rs` (`stored` and `Side` become `pub(crate)`)

**Interfaces:**
- Consumes: `merge_text`, `NotMerged` (Task 3); `RuleMatcher` (Task 2); `changes::stored(store, kind, id, size)` to read a side's contents (it refuses past 64 MiB before holding them); `listing::{Listing, Entry, EntryKind, SaveLimits}`; `diff::again`; `snapshot::store_listing`; `ingest::store_held` (its error mapped as `snapshot::store_file` maps it); `names::folder_problems`.
- Produces, in `combine.rs`:

```rust
pub enum Side { Ours, Theirs }
/// What happened to one path on the two sides.
pub enum Clash {
    BothChanged(NotMerged),
    ChangedRemoved { removed_on: Side },
    BothAdded,
    FileAndFolder { folder_on: Side },
    MovedApart { ours_to: String, theirs_to: String },
    MovedRemoved { moved_on: Side, to: String },
    MovedOnto { moved_on: Side, to: String },
}
pub struct Conflict { pub path: String, pub clash: Clash }
impl Conflict {
    /// `<path>: <what happened>`, the path and names escaped; `ours` and `theirs` name the sides.
    pub fn describe(&self, ours: &str, theirs: &str) -> String;
}
/// Which of the three versions a merge reads.
pub enum Which { Start, Ours, Theirs }
pub enum CombineError {        // Task 7 adds the command's variants
    NamesClash { folder: String, problem: PortabilityProblem },
    /// A version read is past a save's limits.
    Past { which: Which, detail: String },
    /// Inner combines merged more text than `MAX_HELD_TEXT`.
    HeldTooMuch,
    /// The result is past them.
    TooBig { detail: String },
    Repo(RepoError),
}
```

  and in `combine/tree.rs`:

```rust
/// The most text merged by inner combines that one combine holds in memory.
pub const MAX_HELD_TEXT: u64 = 256 << 20;

/// What a combine has built that a later merge of the same combine may read, before anything
/// is stored. Every object it built is in the combine's `PackSet` too.
pub(crate) struct Made {
    pub listings: HashMap<Id, Listing>,
    /// Texts merged by inner combines only: a made-up version a later merge reads holds them.
    pub texts: HashMap<Id, Vec<u8>>,
    /// Their bytes, and the most allowed (`MAX_HELD_TEXT`; tests lower it).
    pub held: u64,
    pub max_held: u64,
}

pub(crate) struct Merged {
    /// The merged root listing, in `packs` and `made`; meaningless when there are conflicts.
    pub root: Id,
    /// Files merged line by line, in path order.
    pub merged_files: Vec<String>,
    /// The first 100 conflicts in path order (never more are kept), and how many more there were.
    pub conflicts: Vec<Conflict>,
    pub more: u64,
    /// Files whose comparison ran past its deadline.
    pub timed_out: Vec<String>,
}
/// `inner`: this merge makes a made-up starting point, whose merged texts a later merge reads.
pub(crate) fn merge_trees(store: &Store, packs: &mut PackSet, made: &mut Made, inner: bool,
    start: Id, ours: Id, theirs: Id, rules: &RuleMatcher, deadline: Duration)
    -> Result<Merged, CombineError>
```

- **Where merged texts go** (what the code allows; see "How a pack set holds what a combine builds" below). A listing is read from `made.listings` first, else `store.listing`; a file's contents from `made.texts` first, else `stored`. Each listing built goes to `store_listing` (the pack set) and `made.listings`. Each merged text is written into the pack set with `store_held` as soon as it is made, and then dropped, so one file's three versions and its result are the most text the merge holds at a time. Only when `inner` is a merged text also kept in `made.texts`, adding its size to `made.held`; past `made.max_held` the merge stops with `CombineError::HeldTooMuch`.
- **How a pack set holds what a combine builds** (read in `pack.rs`, `ingest.rs`, `repo.rs`): `repo.new_packs()` gives a `PackSet` whose `PackWriter` writes each object into a temporary file in `.opalro/tmp` as it is added (`add_entry` streams it; only an index of about 100 bytes an object stays in memory), and `ingest::store_held` cuts a big file into chunks the same way. Dropping a `PackSet` without `finish` drops its writer, whose `Drop` removes the temporary file; a crash leaves the file in `tmp/`, which `Repository::clear_tmp` deletes under the change lock before the next command. Nothing in `tmp/` is ever read as part of the store. So objects written into the combine's pack set become part of the store only at `store_packs`. Two limits follow: a pack set can't be read back (no reader for the pack being built), which is why the texts of inner combines, which a later merge of the same combine reads, are held in memory, capped at `MAX_HELD_TEXT`; and a pack set that reaches 1,048,576 objects (`WRITER_PACK_OBJECTS`) moves that full pack into `.opalro/packs` at once (`roll_over`), so a combine that builds more objects than that before it is refused leaves that pack behind, named by nothing, for `tidy`, as a refusal after storing does.

- **The rules** ([decision 166](../decisions/166-merging-two-versions-of-the-project.md)), for each name of a folder, with the start's entry, ours and theirs (any may be missing; entries are equal when kind and ID are):
  - ours == theirs: ours. ours == start: theirs. theirs == start: ours. A folder taken this way is taken whole, unread.
  - Both folders: merged one level down, from the start's folder if it is one, else from an empty listing.
  - A folder against a removal, where the start has the folder: the removing side counts as an empty listing; if the merged folder is empty, it is gone. (Two folders that merge to an empty one stay, as an empty folder.)
  - Two files (kinds 1 and 2) with a file at the start: the executable mark and the contents are merged apart, each taking the one side's change. Contents both changed: `NoMarkers` when `rules.conflict_markers(path)` is false; else read all three with `stored` (`TooBig` past 64 MiB) and `merge_text`; `Merged` bytes are stored with `store_held` and the path goes to `merged_files`.
  - Two files merge apart only when the start holds a file there too. Everything else is a conflict: `ChangedRemoved` (a file changed on one side, removed on the other), `BothAdded` (nothing at the start, and the two differ in anything: contents or the executable mark), `FileAndFolder`, and `BothChanged(NotFiles)` for links, project links, conflicts (kind 6) and a file at a path that held something else at the start.
  - **Names one folder can't hold:** after a folder is merged, each pair of its names that `names::folder_problems` reports as `CaseCollision` or `NormalizationCollision` stops the merge with `NamesClash`, unless ours' folder or theirs' folder at that path holds both names.
- **The walk** is iterative (an explicit stack, as `diff::walk`), builds each merged listing as it leaves a folder (`Listing::new`, then `store_listing`), and gives a `SaveLimits` to each of the three versions read: `check_path`, `check_depth`, `add` per listing read unless counted, and `again` for every child folder before going into any. A version past its limits is `Past { which, detail }` with the `SaveLimits` detail; a merged folder past 1,000,000 entries is `TooBig { detail: "a folder in it would hold more than 1,000,000 entries" }`. After the first conflict the walk goes on to name the others, keeping at most 100 records and counting the rest.
- **The words** of `describe` (`<a>` the side that did the first thing, `<b>` the other):

  | Clash | Words |
  |---|---|
  | `BothChanged(Overlap { endings_only: false })` | `changed on both lines, in the same place` |
  | `BothChanged(Overlap { endings_only: true })` | `changed on both lines, in the same place; the two differ only in line endings` |
  | `BothChanged(Binary)` | `changed on both lines, and isn't text, so it can't be merged line by line` |
  | `BothChanged(TooBig)` | `changed on both lines, and is over 64 MiB, too big to merge line by line` |
  | `BothChanged(TooManyLines)` | `changed on both lines, and has over 1,000,000 lines, too many to merge line by line` |
  | `BothChanged(NoMarkers)` | `changed on both lines, and opalro.toml says conflict-markers = false for it` |
  | `BothChanged(NotFiles)` | `changed on both lines, and only files are merged line by line` |
  | `ChangedRemoved` | `changed on <a>, removed on <b>` |
  | `BothAdded` | `added on both lines, differently` |
  | `FileAndFolder` | `a file on <a>, a folder on <b>` |
  | `MovedApart` | `moved to <x> on <ours>, and to <y> on <theirs>` |
  | `MovedRemoved` | `moved to <x> on <a>, removed on <b>` |
  | `MovedOnto` | `moved to <x> on <a>, where <b> has something else` |

- [ ] **Step 1: Write the failing tests** in `combine/tests.rs`, with a helper that stores a tree from a list of `(path, contents, executable)` (and folders, links) through `kept::overlay` from an empty root and `store_packs`:

  - `one_sides_change_is_taken`: a table of cases (unchanged; edited on ours; added on theirs; removed on ours; the same edit on both; removed on both): the merged tree.
  - `a_folder_the_same_on_two_sides_is_not_read`: `big/` with 1,000 folders, the same on all three, theirs edits `a.txt`: `store::counted::listings()` grows by at most 6.
  - `the_executable_mark_and_the_contents_merge_apart`: ours makes `run.sh` executable, theirs edits it: executable, theirs' contents.
  - `a_folder_removed_on_one_side_keeps_only_what_the_other_changed`: start `d/{a,b,c}`; ours removes `d`; theirs edits `d/b`: one conflict, `d/b: changed on side, removed on main` (`ours = "main"`, `theirs = "side"`). Ours removes `d`, theirs adds `d/new`: `d` holds `new` alone. Ours removes `d`, theirs untouched: no `d`.
  - `two_new_files_at_one_path_conflict` (also when they differ only in the executable mark); `two_folders_that_merge_to_nothing_stay_as_an_empty_folder`; `a_file_against_a_folder_conflicts`; `a_link_changed_two_ways_conflicts`; `changed_on_one_side_removed_on_the_other_conflicts`.
  - `text_both_sides_changed_is_merged_line_by_line`: `merged_files == ["src/a.rs"]`, contents hold both edits.
  - `nothing_is_stored_while_merging`: after a merge with conflicts and one merged text, dropping `packs`, the store holds no new pack (`store.pack_count()` unchanged) and `.opalro/tmp` holds no file.
  - `an_outer_merge_keeps_no_text_and_an_inner_one_keeps_its_own`: with `inner == false`, `made.texts` stays empty after a text merge; with `inner == true` it holds the merged text and `made.held` its size; with `max_held` 10 bytes, `HeldTooMuch`.
  - `binary_both_changed_conflicts`; `a_rule_with_conflict_markers_false_is_never_merged` (the Game preset's rules, `Assets/Main.unity` changed far apart on both sides: `NoMarkers`).
  - `names_two_sides_bring_that_clash_stop_the_combine`: ours adds `Readme.md`, theirs `README.md`: `NamesClash` naming both; the same for `é` composed and decomposed.
  - `a_clashing_pair_one_side_already_held_is_kept`: theirs' start and tip hold `a` and `A` (planted), ours edits `b`: merged, both kept.
  - `a_version_past_the_limits_is_refused_naming_it`: theirs nests 4,097 folders: `Past { which: Theirs, detail }` with `nest more than 4096 levels`.
  - `a_deep_tree_merges_without_recursion`: 4,000 levels on every side, both edit different files at the bottom: merged, on a thread with a 256 KiB stack.
  - `conflicts_are_named_up_to_100_and_counted_after`: 150 files changed apart on both: 100 named, `more == 50`.
  - `tree_merge_agrees_with_a_path_by_path_model` (proptest, 256 cases): three trees over the paths `a`, `A`, `b`, `d/a`, `d/c`, `d/e/b`, `D/a`, each entry missing, a one-line file (4 contents, either mark), a link, or a folder; the oracle applies [decision 166](../decisions/166-merging-two-versions-of-the-project.md)'s rules to flat path maps, written separately here. Same conflict paths; without conflicts, the same files.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs-core --all-features combine -- --test-threads=2`
Expected: does not compile.

- [ ] **Step 3: Implement** the types, `describe` and `merge_trees`.

- [ ] **Step 4: Run the tests and see them pass**, then break it three ways and report which test caught each: take theirs when only `ours == start` is checked for folders too, without reading a changed folder; treat a folder against a removal as one conflict; check clashing names only among the names one side added.

- [ ] **Step 5: Commit**

```bash
git add crates/opal-vcs-core/src/combine.rs crates/opal-vcs-core/src/combine crates/opal-vcs-core/src/diff.rs crates/opal-vcs-core/src/changes.rs
git commit -m "Merge two versions of the project entry by entry: folders one level down, a file's mark and contents apart, clashing names refused"
```

### Task 5: `save` notes the moves it is sure of

**Files:**
- Create: `crates/opal-vcs-core/src/moves.rs`
- Modify: `crates/opal-vcs-core/src/lib.rs` (`pub(crate) mod moves;`), `crates/opal-vcs-core/src/snapshot.rs` (`Storing` keeps what it stores; the save record), `docs/storage-format.md` §4 (first, before the code)
- Test: `moves.rs`, `snapshot.rs`'s tests

**Interfaces:**
- Consumes: `diff::for_each_entry_change_in`; `save::{MoveNote, MAX_MOVE_NOTES}`; `names::validate_path`.
- Produces:

```rust
/// Files that left one path and arrived at another between `from_root` and `to_root` with the
/// same non-empty contents, no other file that left and none that arrived holding those
/// contents; sorted by where they went. A path that left holds nothing at `to_root`, and one
/// that arrived held nothing at `from_root`. Either side's listings may be in `unstored`.
pub(crate) fn same_contents(store: &Store, unstored: &HashMap<Id, Listing>, from_root: Id,
    to_root: Id) -> Result<Vec<MoveNote>, StoreError>
```

- "A file" is kind 1 or 2; contents are the entry's ID; non-empty is `size > 0`. A link is never a move.
- `save` works out its notes against its first parent before the save is encoded: `Storing` keeps a copy of each listing it stores (and the rebuilt ones) in a `HashMap<Id, Listing>` for `unstored`; the notes are cut to the first `MAX_MOVE_NOTES`. `save --amend` works them out afresh against the amended save's first parent, not copying them. Safety-copy saves and shelves (`kept::kept_save`) keep `move_notes: vec![]`.
- storage-format.md §4 gains, after the move-note limits: `A save notes a move from A to B, compared with its first parent, when A held a file there and holds nothing now, B holds a file now and held nothing there, the two files have the same non-empty contents, and no other file that left and none that arrived has those contents. Saves made for safety copies and shelves note none.`

- [ ] **Step 1: Write the failing tests:**

  - `moves.rs`: `a_file_moved_alone_is_paired`; `a_file_moved_and_changed_is_not`; `two_identical_files_swapping_pair_nothing`; `a_copy_to_two_places_pairs_nothing`; `an_empty_file_moved_pairs_nothing`; `a_moved_folder_pairs_each_file` (sorted by where they went); `a_file_replaced_by_a_folder_is_not_a_place_left`; `a_link_is_not_moved`.
  - `snapshot.rs`: `a_save_notes_a_file_moved_alone` (the stored save's `move_notes == [a.txt → Characters/a.txt]`); `a_save_notes_nothing_when_unsure`; `amend_works_its_notes_out_afresh` (move, save, move back, `save --amend`: no notes); `a_shelf_notes_no_moves`; `a_save_moving_more_than_100000_files_notes_the_first_100000` (`#[ignore = "nightly: 100,001 files"]`).

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs-core --all-features moves -- --test-threads=2`
Expected: does not compile.

- [ ] **Step 3: Edit storage-format.md §4, then implement** `same_contents` and the notes in `save_folder`.

- [ ] **Step 4: Run the tests and see them pass**, then `snapshot`, `shelf` and the CLI's `amend`, `import_vs_git` test files. Break it three ways and report which test caught each: leave out the uniqueness of arrived files; let empty files pair; take a path that now holds a folder as left.

- [ ] **Step 5: Commit**

```bash
git add crates/opal-vcs-core/src/moves.rs crates/opal-vcs-core/src/lib.rs crates/opal-vcs-core/src/snapshot.rs docs/storage-format.md
git commit -m "A save notes a file moved with nothing else changed, when Opalro is certain"
```

### Task 6: A combine follows moves

**Files:**
- Modify: `crates/opal-vcs-core/src/moves.rs`, `crates/opal-vcs-core/src/combine.rs`
- Test: `crates/opal-vcs-core/src/combine/tests.rs`

**Interfaces:**
- Consumes: `same_contents` (Task 5); `merge_trees`, `Made`, `Conflict`, `Clash` (Task 4); `ancestry::only_in`; `kept::overlay_with` and `kept::Put`.
- Produces:

```rust
// moves.rs
/// One version a merge reads: its root listing, and its save, `None` when it is made up.
pub(crate) struct Version { pub root: Id, pub save: Option<Id> }
/// Where the files of `start` went on `side`: start path to side path. Each move is checked.
/// Listings in `made` are read before the store's.
pub(crate) fn followed(repo: &Repository, made: &Made, start: &Version, side: &Version)
    -> Result<BTreeMap<String, String>, RepoError>

// combine.rs
/// Merges `ours` and `theirs` from `start`, following moves. Everything it builds goes to
/// `packs`, its listings to `made` too, and its texts as well only when `inner`; nothing is
/// stored.
pub(crate) fn merge_three(repo: &Repository, packs: &mut PackSet, made: &mut Made, inner: bool,
    start: &Version, ours: &Version, theirs: &Version, rules: &RuleMatcher)
    -> Result<Merged, CombineError>

// kept.rs: `overlay_with` becomes pub(crate) and reads base folders from `unstored` first
pub(crate) fn overlay_with(store: &Store, unstored: &HashMap<Id, Listing>, base_root: Id,
    changes: &BTreeMap<String, Option<Put>>,
    keep: &mut dyn FnMut(&Listing) -> Result<Id, SaveError>) -> Result<Id, SaveError>
```

- **`followed`** ([decision 168](../decisions/168-moves-in-saves-and-in-combines.md)):
  1. When `start.save` and `side.save` are both saves: walk first parents from the side's save, at most `only_in(repo, side, &[start])? + 1` steps (every save before the start on that path is one the start doesn't have, and `only_in` never counts too few). At the start's save, stop and use the notes of the saves passed, oldest first, chained (A to B, then B to C, is A to C); a combined save on the way is one more step, its notes exact against its first parent. When the walk ends without meeting the start (it was reached through a second parent), use no notes. Notes whose paths fail `validate_path` are skipped.
  2. For the start's files not followed by step 1, and for all of them when the start or the side is made up: `same_contents(start.root, side.root)`, with `made.listings` as `unstored`.
  - A move from A to X counts only when the start holds a file at A and nothing at X, and the side holds a file at X and nothing at A (a side that moved A away and made a new file at A has not moved it). Anything else a note says is ignored.
- **`merge_three`:** for each file of the start that a side moved (A to X):

  | The other side | Result |
  |---|---|
  | holds A unmoved, nothing at X, and only folders (or nothing) above X | merged at X: the start's A and the other side's A are moved to X first |
  | moved it to X too | merged at X |
  | moved it elsewhere | `MovedApart` at A |
  | removed A | `MovedRemoved` at A |
  | holds something at X, or a non-folder above it | `MovedOnto` at A |

  "Moved first": the start's root and each side's root are rewritten with `kept::overlay_with` (A set to nothing, X to that version's entry at A; each listing it builds goes to `store_listing` and `made.listings`) and given to `merge_trees`; the move conflicts and the tree's are named together, in path order, at most 100 kept.

- [ ] **Step 1: Write the failing tests** (saves made with `save_folder`, so notes are real; a helper plants saves without notes for histories from Git):

  - `a_change_to_a_moved_file_lands_where_it_went`: on `main`, `Player.cs` moved to `Characters/Player.cs` and saved alone; on `side`, `Player.cs` edited: the result has the edit at `Characters/Player.cs` and nothing at `Player.cs`.
  - `both_moved_to_the_same_place_are_merged_there` (each also edits a different line).
  - `moved_apart_conflicts`; `moved_and_removed_conflicts`; `moved_onto_something_new_conflicts` (the other side added `Characters/Player.cs`, and in a second case a file named `Characters`).
  - `notes_chain_a_to_b_to_c`.
  - `a_history_without_notes_is_followed_by_contents`.
  - `a_combined_save_on_the_way_is_one_more_step`: the side's move is noted behind a combined save on its first-parent path, and the file is then edited too: followed by the notes, the other side's change lands at the new place.
  - `a_start_reached_through_a_second_parent_is_followed_by_contents`: the start is on the side's history only through a second parent; a move and edit in one save is then not followed, and is `ChangedRemoved`; a pure move is followed by contents.
  - `a_side_that_made_a_new_file_where_one_left_has_not_moved_it`.
  - `a_file_added_in_a_moved_folder_stays_in_the_old_folder`.
  - `a_note_that_doesnt_fit_is_ignored`: planted notes from a path that held nothing, to a path holding a folder, to `../x`, and to a different file: the result is as without them, no error.
  - `a_move_to_a_place_that_held_something_at_the_start_is_not_a_move`.
  - `a_made_up_version_follows_contents_only`: `Version { save: None }` as a side, and in a second case as the start, with a note-bearing history behind it: contents only.
  - `the_walk_for_notes_is_bounded`: a side 1,000 saves past a start reached only through a second parent: `saves_read` grows by at most what `only_in` reads plus 1,002.
  - `following_moves_agrees_with_a_model` (proptest, 128 cases): a start of up to 6 files with distinct contents; each side applies up to 3 moves, each saved alone (so noted), then up to 2 edits or removals saved; the oracle applies the table to flat maps.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs-core --all-features combine -- --test-threads=2`
Expected: does not compile.

- [ ] **Step 3: Implement** `followed` and `merge_three`.

- [ ] **Step 4: Run the tests and see them pass**, then break it three ways and report which test caught each: drop the "nothing at X at the start" check; let a removal win over a move; stop the chain at a combined save.

- [ ] **Step 5: Commit**

```bash
git add crates/opal-vcs-core/src/moves.rs crates/opal-vcs-core/src/combine.rs crates/opal-vcs-core/src/combine crates/opal-vcs-core/src/kept.rs
git commit -m "A combine follows files that moved: a change lands where the file went, and a move against a deletion is a conflict"
```

### Task 7: `combine` in the engine

**Files:**
- Modify: `crates/opal-vcs-core/src/combine.rs`, `crates/opal-vcs-core/src/switch.rs` (`landing`), `crates/opal-vcs-core/src/warning.rs` (`MERGE_TIMED_OUT`)
- Test: `crates/opal-vcs-core/src/combine/tests.rs`

**Interfaces:**
- Consumes: `ancestry::{only_in, common_starts, Starts}` (Task 1); `merge_three`, `Made` (Tasks 4 and 6); `RuleMatcher::new(&repo.rules().rules)` (Task 2); `lines::Target` as `lines::resolve_target` gives it (the CLI resolves; the engine reads the target again under the lock); `moves::same_contents` for the combined save's own notes (Task 5); `switch::{locked, kept_on, Switch, switch, switch_warnings, Earlier}`; `writeout::plan`; `kept::switch_saves`; `changes::deadline()`.
- Produces:

```rust
// switch.rs: the match at the top of `go`, moved out; `go` calls it.
/// Where `target` leads now, read under the lock: the position and the save.
pub(crate) fn landing(repo: &Repository, state: &State, target: &Target)
    -> Result<(Position, Id), SwitchError>

// combine.rs
pub const MAX_INNER: usize = 64;
pub enum Combined {
    Nothing,
    Forward,
    Made { starts: Vec<Id>, merged_files: Vec<String> },
}
pub struct CombineOutcome {
    pub line: String,
    pub result: Combined,
    /// The line's newest save now.
    pub save: Id,
    /// The target's save.
    pub with: Id,
    /// As `goto` gives them; default for `Nothing`, which writes nothing.
    pub applied: Applied,
    pub safety_copies: usize,
    pub carried: usize,
}
/// How the target is named: `raw` goes into the default message as it is; `shown` is the same
/// name made safe to print (an escaped line name, `tags::shown_name` for a tag, an ID as is).
pub struct Named { pub raw: String, pub shown: String }

/// Combines `target` into the line the project is on.
pub fn combine(repo: &mut Repository, target: &Target, named: &Named, message: Option<&str>,
    who: &Person, git_excludes: Option<&Path>, on_wait: &mut dyn FnMut(&str))
    -> Result<CombineOutcome, CombineError>
```

  `CombineError` gains `NotOnALine`, `Unrelated`, `Tangled`, `Conflicts`, `StartsConflict`, `Collisions`, `Switch(SwitchError)` and `hint()`; `Past`, `NamesClash` and the new variants carry the line's name (escaped) and the target's `shown` name for their words, which print them as they are.

- **Behaviour**, inside `switch::locked(repo, "combine", …)`:
  1. At a save: `NotOnALine`. `with` = `landing(...)`'s save; `from` = the line's newest.
  2. `only_in(with, [from]) == 0`: `Nothing`; no step (R3).
  3. `only_in(from, [with]) == 0`: **forward.** `plan(from, with)`; collisions are `Collisions`; the state's line points at `with`; `switch_saves`, `switch` with command `combine`.
  4. Otherwise **a combined save.** One `PackSet` and one `Made` (`max_held: MAX_HELD_TEXT`) serve the whole combine; every object is written into the pack set as it is made, and nothing becomes part of the store until step "store" below, so every refusal before it stores nothing (the pack set is dropped unfinished, which removes its temporary file). Inner combines call `merge_three` with `inner: true`, the last merge with `inner: false`, so its merged texts are never held beyond their own file; past `MAX_HELD_TEXT` of inner texts, `HeldTooMuch`.
     - The start (R2): `common_starts([from], [with])`. `Found([])`: `Unrelated`. `TooMany`: `Tangled`. One: that save. Several: in ID order, `acc` begins as the first; each next starting point `s` is merged with `acc` from the start of `common_starts(acc's tips, [s])`, found by these same rules (a recursion each of whose calls counts one inner combine, so it is at most `MAX_INNER` calls deep; past `MAX_INNER` in all, `Tangled`); any conflict there is `StartsConflict`; `acc` becomes the merged listing with `acc`'s tips plus `s`. A made-up version has `save: None`.
     - `merge_three(start, from, with)`; conflicts are `Conflicts`; clashing names are `NamesClash`.
     - The save: parents `[from, with]`, the merged root, author and saver `who`, a new change ID, the message given or `Combine <named.raw> into <line>`, extra `line`, and its own move notes against `from` like any save's (`same_contents` with `made.listings`, cut to `MAX_MOVE_NOTES`), checked with `Save::decode` before `add_record`.
     - The result's limits: `SaveLimits::default().add_all(root, 0, 0, read)`, with `read` looking in `made.listings` first, then the store (one read of every listing of the result); past them, `TooBig`, and nothing is stored.
     - Store: `store_packs`, then a new `PackSet` for the switch.
     - `plan(from, M)`; collisions are `Collisions`; the line points at `M`; `switch_saves`; `switch` with command `combine`.
  5. Every merged file whose comparison ran past its deadline adds one `merge_timed_out` warning naming it: `Comparing the versions of <path> took over 2 seconds, so Opalro compared them roughly; a closer look might have merged them.`
- **The words:**

  | Error | Message | Hint |
  |---|---|---|
  | `NotOnALine` | `You're at a save, not on a line, so there is no line to combine into.` | `Go to a line first (opalro goto <line>), or start one here: opalro line new <name>` |
  | `Unrelated` | `<line> and <target> have no save in common, so there is no starting point to combine them from, and nothing was changed.` | `Opalro combines only lines that share some history.` |
  | `Tangled` | `The histories of <line> and <target> cross each other too many times for Opalro to combine them, so nothing was changed.` | none: combining an older save first isn't something Opalro can promise works |
  | `Conflicts` | `Combining <target> into <line> would leave 3 files in conflict, which Opalro can't keep yet, so nothing was changed:` then a line `  <describe>` per file (`ours` = the line, `theirs` = the target), then `  … and N more` | `For each of these files, make the two lines agree and save; then run opalro combine again.` |
  | `StartsConflict` | `Combining <target> into <line> needs their common starting points combined first, and those conflict with each other, which Opalro can't keep yet, so nothing was changed.` | none: there is nothing Opalro can honestly recommend |
  | `NamesClash` | `Combining <target> into <line> would put two names in one folder that clash, so nothing was changed:` then `  in <folder>/: <problem>` (`in the top folder:` at the top) | `Rename one of them on its line and save; then run opalro combine again.` |
  | `Past` | `<what> is past Opalro's limits for a save (<detail>), so nothing was changed.`, `<what>` being `The save where <line> and <target> parted`, `Line <line>'s newest save` or `<target>` | |
  | `TooBig` | `The combined save would be past Opalro's limits for a save (<detail>), so nothing was changed.` | |
  | `HeldTooMuch` | `Combining <target> into <line> needs their common starting points combined first, and that merges more than 256 MiB of text, more than Opalro holds in memory for it, so nothing was changed.` | none |
  | `Collisions` | `Your unsaved changes, or files Opalro ignores, are in the way of the combine, so nothing was changed:` and the paths as `goto` lists them | `Save them first (opalro save "describe your change"), shelve them (opalro shelf) or discard them (opalro discard), and move ignored files somewhere else, then run opalro combine again.` |

  `1 file in conflict`; counts with `thousands`.

- [ ] **Step 1: Write the failing tests:**

  - `nothing_to_combine_makes_no_step`: the target's save in the line's history, and the line's own name: `Nothing`, head unchanged.
  - `a_line_that_hasnt_moved_is_moved_forward`: `Forward`; `lines["main"] == with`; no new save; one entry, command `combine`, with a folder change; the folder holds the target's files.
  - `a_combined_save_has_both_parents_and_says_what_it_combined`: parents `[from, with]`, message `Combine side into main`, `line()` is `main`, author and saver `who`, a change ID no other save has; the folder holds both sides' changes; one entry, command `combine`.
  - `a_combined_save_notes_the_moves_it_brought_in`: `side` moved a file alone; the combined save's `move_notes` hold that move.
  - `a_message_given_is_the_saves`.
  - `at_a_save_combine_is_refused`; `unrelated_histories_are_refused`.
  - `a_conflict_stops_and_stores_nothing`: head, lines, folder and `.opalro/state` as before, and `repo.store().pack_count()` unchanged; the message lists `a.txt: changed on both lines, in the same place`. The same store check for `unrelated_histories_are_refused`, `seventeen_starting_points_are_refused` and `starting_points_that_conflict_stop_the_combine`.
  - `unsaved_work_where_the_combine_writes_stops_it` and `unsaved_work_elsewhere_comes_along` (`carried == 1`).
  - `a_criss_cross_keeps_a_revert_that_one_starting_point_would_lose`: `B1` has `F = old`, `B2` has `F = new`; one line takes `B2` then reverts to `old`; built as the notes' example: the result keeps `old`.
  - `starting_points_that_conflict_stop_the_combine`: `StartsConflict`.
  - `seventeen_starting_points_are_refused`: `Tangled`.
  - `the_budget_of_inner_combines_is_kept`: through `combine_with(.., max_inner, max_held)`, the private function `combine` calls with `MAX_INNER` and `MAX_HELD_TEXT`: 3 starting points with a budget of 1: `Tangled`, nothing stored.
  - `inner_text_past_what_is_held_is_refused`: the same function, a criss-cross whose starting points merge a text file of 100 bytes, `max_held` 10: `HeldTooMuch`, store and `.opalro/tmp` unchanged; with the default, the combine is made.
  - `a_refused_combine_leaves_nothing_in_tmp`: after `a_conflict_stops_and_stores_nothing`'s refusal, `.opalro/tmp` holds no file.
  - The memory bound of this task's design is pinned in Task 8 (`a_combine_of_many_large_text_files_stays_within_memory`): the project's memory tests measure the `opalro` process (`support::run_limited`), which needs the command line.
  - `a_result_past_the_limits_is_refused`: through the same function's `SaveLimits` (as `diff::walk_files` takes `max_entries`), two sides of 6 entries each under a limit of 10: `TooBig`; head, lines and folder as before, `repo.store().pack_count()` unchanged and `.opalro/tmp` holding no file.
  - `the_folders_rules_decide`: an unsaved `opalro.toml` saying `*.unity` gets no markers, both lines edit `Main.unity` far apart: `Conflicts` with `NoMarkers`.
  - `a_slow_comparison_is_told` (`fault-injection`, `OPALRO_DIFF_DEADLINE_MS=0`): `merge_timed_out` names the file.
  - `undo_takes_a_combine_back_and_redo_brings_it_again`: lines and folder each time.
  - `a_combine_stopped_at_every_step_is_finished_or_never_happened` (`fault-injection`, as `a_goto_stopped_at_every_step_is_finished_or_never_happened`): afterwards the folder is the line's save whole, and the same combine again is `Nothing` when the first got that far.
  - `a_combine_after_a_stopped_goto_finishes_it_first`: `switch_finished` among the warnings.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs-core --all-features combine -- --test-threads=2`
Expected: does not compile (`combine` not found).

- [ ] **Step 3: Implement** `landing`, `combine`, the errors and `MERGE_TIMED_OUT`.

- [ ] **Step 4: Run the tests and see them pass**, then the engine's `switch`, `undo` and `shelf` tests.

- [ ] **Step 5: Commit**

```bash
git add crates/opal-vcs-core/src/combine.rs crates/opal-vcs-core/src/combine crates/opal-vcs-core/src/switch.rs crates/opal-vcs-core/src/warning.rs
git commit -m "combine: nothing to do, the line moved forward, or a combined save, switched to as goto switches, one step that undo takes back"
```

### Task 8: `opalro combine` on the command line

**Files:**
- Modify: `crates/opal-vcs/src/main.rs` (the `Combine` command, its handler, `From<CombineError> for Failure`), `help.rs` (the overview's row, `examples("combine")`), `git_words.rs`
- Create: `crates/opal-vcs/src/combine_output.rs`
- Modify: `docs/terminal.md` (a new `opalro combine` section after `opalro goto`; `opalro undo`'s example), `docs/json.md` (a new section; `merge_timed_out` in §4), `docs/guide.md` (a new `opalro combine` section in §4, combining in §8, `merge` in §9)
- Test: `crates/opal-vcs/tests/combine.rs` (new, with snapshots), `json_formats.rs`, `cli.rs`, `terminal.rs`, `git_words.rs`, `undo.rs`

**Interfaces:**
- Consumes: `combine::{combine, CombineOutcome, Combined, CombineError, Named}` (Task 7); `lines::resolve_target`; `tags::shown_name`; `repo.tag_list`; `short_or_whole`.
- Produces:

```rust
/// Bring another line's work into the line you're on.
Combine {
    /// What to combine in: a line, line@server, tag, change ID or fingerprint.
    target: String,
    /// The combined save's message; Opalro writes one if you leave it out.
    message: Option<String>,
},
```

- The target is read by `lines::resolve_target` alone (its refusals, `LineOrTag` and `ServerLineOrTag` among them, are `goto`'s). `Named`: for `Target::Line` and `Target::ServerLine`, the text as typed (`shown` escaped); for `Target::Save` when the tag list holds the exact text as a tag's name (nothing in the resolver says a tag was typed, so the list is checked after resolving), the text as typed (`shown` = `tags::shown_name`); otherwise the save's short change ID (`short_or_whole`) for both.
- **What people see** (terminal.md, `opalro combine`); while it works, stderr shows the turning mark `Combining` (§8):

  | State | Outcome line | Details | Next step |
  |---|---|---|---|
  | Combined | `Combined <target> into <line>: save <id> "<message>".` | `Merged 2 files line by line.` (`1 file`) when any; what was written as `goto` says it; `Your unsaved changes in 2 files came along.` when some did | `Changed your mind? opalro undo` |
  | Forward | `Moved <line> forward to <target>: save <id> "<message>".` | `<line> had nothing <target> lacks, so no combined save was needed.`; `No save was made, so your message wasn't used.` when one was given; what was written | `Changed your mind? opalro undo` |
  | Nothing | `Nothing to combine: <line> already has everything <target> has.` | | |
  | A refusal | stdout gets nothing; `error:` with Task 7's words, exit 1 | | |

- **Colour:** `Combined` and `Moved` green and bold, line names blue, IDs magenta and bold, paths in a refusal as `goto` colours them.
- The overview's row (with `line` and `tag`): `combine` — `Bring another line's work into yours`. `examples("combine")`: `opalro combine wall-run` "Bring line wall-run's work into the line you're on", `opalro combine wall-run "Wall running is in"` "The same, with your own message".
- **Git's words:** `merge <target>` runs `combine <target>`. Explained, each in its own words, as `rebase` and `cherry-pick` are:
  - `merge --abort` (`MERGE_ABORT`): `` `opalro combine` never stops half-way, so there is nothing to abort.\n`opalro undo` takes back a combine once it is made. ``
  - `merge --no-ff` (`NO_FF`): `` `opalro combine` makes a combined save only when both lines have work the other lacks; otherwise it moves your line forward.\nOpalro can't make a combined save every time. ``
  - `merge --squash` (`SQUASH`): `` Opalro can't bring a line's work in as one plain save.\n`opalro combine <line>` makes a combined save that keeps both lines' history. ``
  - Any other option goes to `combine` as typed, which refuses it as unknown options are refused on any command (exit 2).
  - `LATER` covers `blame` alone and becomes `` Opalro can't show who changed each line of a file yet.\nUntil then, `opalro history` lists the saves on your line. `` (no version promised).
- **The document** (json.md, `opalro combine --json`):

  | Field | Type | Meaning |
  |---|---|---|
  | `opalro_json` | number | `1` |
  | `result` | string | `"combined"`, `"forward"` or `"nothing"` |
  | `line` | string | The line the work came into. |
  | `save` | save | The line's newest save now. |
  | `with` | string | The target's save: 64 hex digits. |
  | `starts` | list of string | The saves where the two parted, 64 hex digits each, in ID order; `[]` unless `result` is `"combined"`. |
  | `merged_files` | list of string | Files merged line by line; `[]` if none. |
  | `written`, `removed`, `not_written`, `placeholders`, `safety_copies` | | As in `opalro goto --json`; `0` and `[]` for `"nothing"`. |
  | `warnings` | list | §4; `merge_timed_out` among them. |

  A refusal is the §5 error document, the conflicting files in `error.message`.

- [ ] **Step 1: Write the failing tests** in `tests/combine.rs`:

  - `combine_makes_a_combined_save_and_says_so`: two lines with an edit each: stdout starts `Combined side into main: save `; `history --json`'s first save has the default message.
  - `combine_moves_a_line_forward_when_that_is_all_it_takes`: stdout holds `main had nothing side lacks, so no combined save was needed.`; with a message, `No save was made, so your message wasn't used.`
  - `combining_twice_says_nothing_to_combine`: exit 0, `Nothing to combine: main already has everything side has.`
  - `a_conflict_stops_names_the_files_and_changes_nothing`: exit 1; stderr holds `would leave 1 file in conflict` and `a.txt: changed on both lines, in the same place`; `status` and `history` as before.
  - `combine_at_a_save_is_refused`; `combine_takes_a_tag_and_an_id` (the default message names the tag, and the short change ID).
  - `unsaved_work_in_the_way_names_combine_in_the_hint`.
  - `combine_json_matches_its_snapshot` (each of the three results, masked as the others are).
  - `undo_says_what_a_combine_took_back`: `opalro undo` after a combine: exactly `Undid combine: main is back at <id>.` (`step_line` unchanged).
  - `a_combine_of_many_large_text_files_stays_within_memory` (`#[cfg(target_os = "linux")]`, as `shelving_a_big_file_uses_little_memory`): 12 text files of 30 MiB each (lines of 40 bytes, from a seed), each edited far apart on `main` and on `side`, saved; `opalro combine side` under `support::run_limited(cmd, 600)` exits 0 with `merged_files` holding all 12, and `peak_mb < 300`: the merge holds one file's versions and result at a time (about 4 × 30 MiB plus the line lists), never the 360 MiB of all results.
  - `a_hostile_path_in_a_conflict_is_escaped`: an imported file whose name holds `ESC[31m` and U+202E, changed apart: the refusal holds neither raw (`assert_escaped`).
  - `a_name_both_a_line_and_a_tag_is_refused_by_combine_too`: a line `v1` and a tag `v1` at another save: exit 1 with `LineOrTag`'s words.
  - `a_tags_odd_name_is_quoted_in_the_outcome`: an imported tag named `odd name`: `Combined "odd name" into main: …`, and the default message holds the name as it is.
  - In `git_words.rs`: `merge_runs_combine`, `merge_abort_is_explained`, `merge_no_ff_and_squash_are_explained`, `merge_with_another_option_is_refused_as_unknown`, `blame_is_still_explained`.
  - Add the document to `json_formats.rs`'s `every_document`, `combine` to `cli.rs`'s `COMMANDS` (its length grows by one) and to the commands `terminal.rs` runs without a terminal.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs --all-features --test combine -- --test-threads=2`
Expected: fails: clap doesn't know `combine`.

- [ ] **Step 3: Write terminal.md, json.md and the guide** as above (the guide shows `opalro combine wall-run` in a runnable `sh` block, and §8 walks through combining a line and deleting it).

- [ ] **Step 4: Implement** the command, `combine_output.rs`, the help rows, the Git words and `From<CombineError> for Failure`.

- [ ] **Step 5: Run the tests and see them pass**

Run: the `combine`, `json_formats`, `cli`, `terminal`, `git_words`, `undo` and `guide` test files, with `cargo insta` snapshots reviewed and accepted.

- [ ] **Step 6: Commit**

```bash
git add crates/opal-vcs/src crates/opal-vcs/tests docs/terminal.md docs/json.md docs/guide.md
git commit -m "opalro combine: on the command line, in the guide, and in --json; git merge runs it"
```

### Task 9: Finding a save by its ID through every parent

**Files:**
- Modify: `crates/opal-vcs-core/src/history.rs` (`by_id`)
- Modify: `docs/terminal.md` (`opalro show`'s facts: which digits `show` takes back), `docs/json.md` (`opalro goto --json`'s first paragraph)
- Test: `history.rs`'s tests; `crates/opal-vcs/tests/combine.rs`

**Interfaces:**
- Consumes: `repo.tag_list`, `repo.server_lines`, `tip_of`.
- Produces: `by_id` (and so `resolve`, `find`, `line_named`, `tag_named`) looks in two steps ([decision 169](../decisions/169-saves-reached-through-a-combine.md)): first as today; only when that finds nothing, every save that a line, a server line, a tag or the save the project is at reaches through every parent, each read once (a seen-set and a stack, no recursion).
- The first step stays exactly today's: a change ID on the current line, a fingerprint as today. The check that a line's or tag's name isn't also another save's ID (`line_named`, `tag_named`, decision 068) uses both steps, so a line named only with the letters k to z costs the step-2 walk each time it is named; that is accepted, and measured.
- **Measured:** `a_real_repository_matches_git` (the nightly differential job) times a lookup of a change ID no save has, and prints it for benchmarks.md.

- [ ] **Step 1: Write the failing tests:**

  - Engine: `a_save_reached_only_through_a_second_parent_is_found` (by change ID and by fingerprint); `the_line_comes_first_so_a_short_id_history_printed_still_works` (a start matching one save on the line and one elsewhere: the line's); `several_found_elsewhere_are_listed` (`Ambiguous`); `a_tags_or_server_lines_history_counts`; `a_save_only_a_shelf_holds_is_not_found`; `a_miss_reads_each_save_once` (`saves_read` grows by at most the number of saves).
  - `tests/combine.rs`: `show_finds_a_save_a_deleted_line_left_behind_a_combine` (combine `side`, `line delete side`, `show <its short change ID>` exits 0).

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs-core --all-features history -- --test-threads=2`
Expected: the first test gets `NotFound`.

- [ ] **Step 3: Implement** the second step, and edit the two docs.

- [ ] **Step 4: Run the tests and see them pass**, then the `goto`, `tags`, `lines`, `combine` test files.

- [ ] **Step 5: Commit**

```bash
git add crates/opal-vcs-core/src/history.rs crates/opal-vcs/tests/combine.rs crates/opal-vcs/tests/import_vs_git.rs docs/terminal.md docs/json.md
git commit -m "A save is found by its ID through every parent, once the line you're on has none"
```

### Task 10: `history <from>`, the `[combined …]` marks, `show`'s row and `parents`

**Files:**
- Modify: `crates/opal-vcs-core/src/history.rs` (`for_each_from`), `crates/opal-vcs/src/main.rs` (`History { from }`, `history`, `show`), `history_output.rs`, `show_output.rs`, `output.rs` (`save_json`)
- Modify: `docs/terminal.md` (`opalro history`, `opalro show`), `docs/json.md` (§2's save, `opalro history --json`), `docs/guide.md` (`opalro history`, §8)
- Test: `crates/opal-vcs/tests/combine.rs`, `terminal.rs`, `json_formats.rs`

**Interfaces:**
- Consumes: `resolve`/`find` (Task 9).
- Produces:

```rust
/// Calls `each` with every save from `tip` back along first parents, newest first, each with
/// the first 12 letters of its change ID. Nothing is cached.
pub fn for_each_from<E: From<RepoError>>(repo: &Repository, tip: Id,
    each: impl FnMut(&Id, &Save, &str) -> Result<(), E>) -> Result<(), E>
```

- `opalro history <line or save>`: clap `History { /// List from this line, tag or save instead: by line name, line@server, tag, change ID or fingerprint.  from: Option<String> }`. A line's name lists that line as today's `for_each_on_line` would (its own short IDs); anything else uses `for_each_from`, under the heading `Saves back from save <id> "<message>", newest first:` (`<id>` its 12 letters). The place marker goes on the row of the save the project is at, if listed. The `from` text is read by `lines::resolve_target`, as `show` reads its argument.
- A row of a save with more than one parent ends, after its line and tag marks, with `[combined <id>]` per other parent (12 hex digits, blue, as fingerprints).
- `show`: `After` names the first parent alone; each other parent gets a row `Combined     <12 hex> "<message>"` under it (the label on the first only; no message when the save can't be read).
- `save_json` gains `"parents"`: the fingerprints of its parents, in order; every document with a save object has it. `history --json` gains `from`: 64 hex digits, `null` without an argument.

- [ ] **Step 1: Write the failing tests:**

  - `history_marks_a_combined_save`: after a combine, `history`'s first row ends `[combined <12 hex of side's save>]`, and lists none of `side`'s own saves.
  - `history_from_a_line_a_tag_or_a_save_lists_from_there`: `history side`, `history v1.0`, and `history <the 12 hex a mark names>` each list that history with 12-letter IDs.
  - `show_names_the_other_parent_on_a_combined_row`.
  - `every_save_object_has_its_parents` (history, show and goto documents).
  - `terminal.rs`'s `project_text_is_escaped_with_colour_on` gains `history <from>` over a hostile message.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs --all-features --test combine -- --test-threads=2`
Expected: no `[combined` in the row.

- [ ] **Step 3: Implement**, and edit the three docs.

- [ ] **Step 4: Run the tests and see them pass**: the `combine`, `terminal`, `json_formats`, `tags`, `lines`, `cli` and `guide` test files, snapshots of `history` and `show` reviewed.

- [ ] **Step 5: Commit**

```bash
git add crates/opal-vcs-core/src/history.rs crates/opal-vcs/src crates/opal-vcs/tests docs/terminal.md docs/json.md docs/guide.md
git commit -m "history marks a combined save and lists from any line or save; show and --json name every parent"
```

### Task 11: The model test learns to combine

**Files:**
- Modify: `crates/opal-vcs/tests/model.rs`, `docs/testing.md` (the model test's actions)

**Interfaces:**
- Consumes: `combine::combine` (Task 7) and the moves `save` notes (Task 5).
- Produces: `Act::Combine { pick: usize, message: bool }` with `Shelf`'s weight; `Saved` gains `others: Vec<usize>` (its other parents), checked at the end of the run. The model, written apart from the engine: its own common starting points (ancestors from its `Saved` table, brute force), its own folding of several in ID order with the same budget, its own path-by-path merge of [decision 166](../decisions/166-merging-two-versions-of-the-project.md), its own following of moves (each save's moves worked out from its tree and its first parent's, chained, then by contents), and its own text merge (longest common subsequence by dynamic programming). Its generator gives every version of a text file lines that don't repeat within that version, so the lining-up is unique and model and engine must agree exactly; repeated lines are Task 3's. It expects `Nothing`, `Forward`, a combined save with the files it computes, `Collisions` when its `switch` would refuse, or a conflict on the files it finds; any difference fails.

- [ ] **Step 1: Add the action and the model's expectations; run the model test**

Run: `PROPTEST_CASES=64 cargo test -j 2 -p opal-vcs --all-features --test model -- --test-threads=2`
Expected: passes. A failure is a difference between the engine and the model: find which is wrong before changing either.

- [ ] **Step 2: Run it longer once**

Run: `PROPTEST_CASES=512 cargo test -j 2 -p opal-vcs --all-features --release --test model -- --test-threads=2`

- [ ] **Step 3: Update testing.md and commit**

```bash
git add crates/opal-vcs/tests/model.rs docs/testing.md
git commit -m "The model test combines lines, with the merge rules written a second time"
```

### Task 12: Whole combines against `git merge`

**Files:**
- Modify: `crates/opal-vcs/tests/merge_vs_git.rs`, `docs/testing.md` (the differential tests)

**Interfaces:**
- Consumes: the `opalro` binary; `support::vs_git` helpers.
- Produces: `combine_matches_git_merge` (proptest, 64 cases): a Git repository with a first commit of 3 to 12 text files of unique lines (some in folders), then two branches of 1 to 4 commits each: edits of unique lines, files added and removed, and files moved with nothing else changed in that commit. Import it; in one copy run `opalro combine <other>`; in another, `git -c merge.directoryRenames=false -c merge.renames=true merge --no-edit <other>`. When `opalro` exits 0, Git exits 0 and the two folders hold the same files, contents and executable marks. Count the four outcomes (both clean, both conflict, Opalro clean only, Git clean only); "Opalro clean only" must be 0; print the counts; assert at least 16 "both clean".

- [ ] **Step 1: Write the test.** A Git-clean, Opalro-conflict case is allowed (Git pairs moves Opalro doesn't); print each one's reason.

Run: `cargo test -j 2 -p opal-vcs --all-features --test merge_vs_git -- --test-threads=2`
Expected: passes.

- [ ] **Step 2: Update testing.md and commit**

```bash
git add crates/opal-vcs/tests/merge_vs_git.rs docs/testing.md
git commit -m "Whole combines are compared with git merge: clean in Opalro means clean in Git, with the same files"
```

### Task 13: The documents

**Files:**
- Modify: `docs/design.md` (§5.5 and §5.10's "Built so far", the phase line, open and deferred lists: `--no-ff`, guessed moves, merge inside a line), `docs/milestones.md` (milestone 8's steps ticked), `docs/testing.md` (the new test files), `docs/security.md` (below), `docs/guide.md` (§13, what Opalro doesn't do yet), `README.md` if it lists commands, `CLAUDE.md` (the phase line), `docs/benchmarks.md` (the controller supplies the numbers: `common_starts` and a miss of Task 9 on Git's own history)

- security.md gains, in the house form: a planted history can make the starting-point walk long, never wrong, and more than 16 candidates or 64 inner combines are refused; a move note can only choose which file a change lands in, never a path outside the project, and a note that doesn't fit is ignored; every version a combine reads is held to a save's limits on its own, and the result to them too; a refusal names at most 100 files, and paths and tag names in it are escaped or quoted; a combine refused for conflicts, starting points, unrelated histories or a result past the limits stores nothing (past 1,048,576 objects built, a full pack it moved in early stays, named by nothing), and one refused for unsaved work in the way leaves only records nothing names; a combine holds one merged file at a time in memory, and at most 256 MiB of text merged by inner combines.
- design.md and the deferred list gain the follow-ups the rulings name: a warning for a `[[rule]]` pattern holding a line break; `show` and `changes --save` saying "moved"; checking the result's size only when the sides' counts could pass the limit; a cache of every reachable save's change ID if Task 9's numbers ask for it.
- The guide's "doesn't do yet" list loses combining; it keeps keeping conflicts and `who`.

- [ ] **Step 1: Edit the documents; check each with `tr -cd '\r' < file | wc -c`**
- [ ] **Step 2: Run the `guide` test file**
- [ ] **Step 3: Commit**

```bash
git add docs README.md CLAUDE.md
git commit -m "Milestone 8's documents: what is built, how it is tested, and what an attacker could try"
```

### Task 14: Red team (the controller's)

- [ ] Write `redteam/attack-surface-m8.md`: the starting-point walk and its budget, planted histories (crossed, dated backwards, very wide), the tree merge on planted listings (deep, wide, twins, kind 6, sizes that lie), move notes that lie, text at the limits, a combine against unsaved work and files in use, undo and crash across a combine, the ID lookup through every parent.
- [ ] One session from the no-network sandbox (decision 024), attack code in `redteam/attacks17`, report in `docs/redteam/`. Windows and macOS through a `redteam/` CI branch.
- [ ] Every proven finding fixed, with its proof in `crates/opal-vcs/tests/redteam18.rs`, or ruled on in a decision record. A second session on the fixes when a finding changed engine code.

### Task 15: Review, rulings and the merge into `v0.2` (the controller's)

- [ ] A whole-branch review by a fresh reviewer, given the Review Focus; its findings fixed or ruled on.
- [ ] `docs/plans/2026-11-14-milestone-8-rulings.md`: the please-confirm list first, then every ruling as made.
- [ ] The nightly workflow run once on the branch (model test at 1,024 cases, the fuzz targets, Git's own repository through Tasks 1 and 12's checks, the ignored 100,001-file save); benchmarks.md gets the numbers.
- [ ] CI green on Windows, macOS and Linux; `milestone-8` merged into `v0.2`; milestones.md and CLAUDE.md say milestone 8 is done.

---

## Words to review

Every message, hint, line of help and `--json` name this plan introduces, in one place. `<line>` is escaped; `<target>` is `Named::shown`; `<path>`, `<x>`, `<y>` are escaped paths; `<a>`, `<b>` name the two sides (the line and the target).

| Kind | Words | Task |
|---|---|---|
| Conflict row | `<path>: changed on both lines, in the same place` | 4 |
| Conflict row | `<path>: changed on both lines, in the same place; the two differ only in line endings` | 4 |
| Conflict row | `<path>: changed on both lines, and isn't text, so it can't be merged line by line` | 4 |
| Conflict row | `<path>: changed on both lines, and is over 64 MiB, too big to merge line by line` | 4 |
| Conflict row | `<path>: changed on both lines, and has over 1,000,000 lines, too many to merge line by line` | 4 |
| Conflict row | `<path>: changed on both lines, and opalro.toml says conflict-markers = false for it` | 4 |
| Conflict row | `<path>: changed on both lines, and only files are merged line by line` | 4 |
| Conflict row | `<path>: changed on <a>, removed on <b>` | 4 |
| Conflict row | `<path>: added on both lines, differently` | 4 |
| Conflict row | `<path>: a file on <a>, a folder on <b>` | 4 |
| Conflict row | `<path>: moved to <x> on <line>, and to <y> on <target>` | 4 |
| Conflict row | `<path>: moved to <x> on <a>, removed on <b>` | 4 |
| Conflict row | `<path>: moved to <x> on <a>, where <b> has something else` | 4 |
| Conflict list end | `  … and N more` | 4 |
| Error | `You're at a save, not on a line, so there is no line to combine into.` | 7 |
| Hint | `Go to a line first (opalro goto <line>), or start one here: opalro line new <name>` (placeholders, no project text) | 7 |
| Error | `<line> and <target> have no save in common, so there is no starting point to combine them from, and nothing was changed.` | 7 |
| Hint | `Opalro combines only lines that share some history.` | 7 |
| Error | `The histories of <line> and <target> cross each other too many times for Opalro to combine them, so nothing was changed.` (no hint) | 7 |
| Error | `Combining <target> into <line> would leave 3 files in conflict, which Opalro can't keep yet, so nothing was changed:` (`1 file`) and the rows | 7 |
| Hint | `For each of these files, make the two lines agree and save; then run opalro combine again.` | 7 |
| Error | `Combining <target> into <line> needs their common starting points combined first, and those conflict with each other, which Opalro can't keep yet, so nothing was changed.` (no hint) | 7 |
| Error | `Combining <target> into <line> would put two names in one folder that clash, so nothing was changed:` then `  in <folder>/: <problem>` (`in the top folder:`) | 7 |
| Hint | `Rename one of them on its line and save; then run opalro combine again.` | 7 |
| Error | `<what> is past Opalro's limits for a save (<detail>), so nothing was changed.`; `<what>`: `The save where <line> and <target> parted`, `Line <line>'s newest save`, `<target>` | 7 |
| Error | `The combined save would be past Opalro's limits for a save (<detail>), so nothing was changed.` | 7 |
| Error | `Combining <target> into <line> needs their common starting points combined first, and that merges more than 256 MiB of text, more than Opalro holds in memory for it, so nothing was changed.` (no hint) | 7 |
| Detail of a limit | `a folder in it would hold more than 1,000,000 entries` | 4 |
| Error | `Your unsaved changes, or files Opalro ignores, are in the way of the combine, so nothing was changed:` and the paths | 7 |
| Hint | `Save them first (opalro save "describe your change"), shelve them (opalro shelf) or discard them (opalro discard), and move ignored files somewhere else, then run opalro combine again.` | 7 |
| Warning `merge_timed_out` | `Comparing the versions of <path> took over 2 seconds, so Opalro compared them roughly; a closer look might have merged them.` | 7 |
| Default message | `Combine <target> into <line>` (raw names, stored) | 7 |
| Outcome | `Combined <target> into <line>: save <id> "<message>".` | 8 |
| Outcome | `Moved <line> forward to <target>: save <id> "<message>".` | 8 |
| Outcome | `Nothing to combine: <line> already has everything <target> has.` | 8 |
| Detail | `Merged 2 files line by line.` (`1 file`) | 8 |
| Detail | `<line> had nothing <target> lacks, so no combined save was needed.` | 8 |
| Detail | `No save was made, so your message wasn't used.` | 8 |
| Progress mark | `Combining` | 8 |
| Undo | `Undid combine: main is back at <id>.` (existing words, pinned) | 8 |
| Help | `Bring another line's work into the line you're on.`; `target`: `What to combine in: a line, line@server, tag, change ID or fingerprint.`; `message`: `The combined save's message; Opalro writes one if you leave it out.` | 8 |
| Help overview | `combine` — `Bring another line's work into yours` | 8 |
| Examples | `opalro combine wall-run` "Bring line wall-run's work into the line you're on"; `opalro combine wall-run "Wall running is in"` "The same, with your own message" | 8 |
| Git word `merge --abort` | `` `opalro combine` never stops half-way, so there is nothing to abort.\n`opalro undo` takes back a combine once it is made. `` | 8 |
| Git word `merge --no-ff` | `` `opalro combine` makes a combined save only when both lines have work the other lacks; otherwise it moves your line forward.\nOpalro can't make a combined save every time. `` | 8 |
| Git word `merge --squash` | `` Opalro can't bring a line's work in as one plain save.\n`opalro combine <line>` makes a combined save that keeps both lines' history. `` | 8 |
| Git word `blame` | `` Opalro can't show who changed each line of a file yet.\nUntil then, `opalro history` lists the saves on your line. `` | 8 |
| JSON `opalro combine --json` | `result` (`"combined"`, `"forward"`, `"nothing"`), `line`, `save`, `with`, `starts`, `merged_files`, `written`, `removed`, `not_written`, `placeholders`, `safety_copies`, `warnings` | 8 |
| Help | `history`'s `from`: `List from this line, tag or save instead: by line name, line@server, tag, change ID or fingerprint.` | 10 |
| Heading | `Saves back from save <id> "<message>", newest first:` | 10 |
| History mark | `[combined <id>]` (12 hex digits) | 10 |
| Show row | `Combined     <12 hex> "<message>"` | 10 |
| JSON | `parents` on every save object; `from` in `opalro history --json` | 10 |
