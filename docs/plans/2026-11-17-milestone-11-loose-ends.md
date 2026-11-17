# Milestone 11 ("Loose ends"): Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task by task. Steps use checkbox (`- [ ]`) syntax for tracking. Every subagent runs on Opus.

**Goal:** the six things the owner's review of v0.2 put first in the next version: a change ID's start counts from four letters; the program names the version a feature came with; a switch stops making folders that only conflicts standing as nothing fill, and `status` shows empty folders; `opalro import` notes the renames it is sure of; the flush after the journal is removed is put to the crash tests; and Git's rule for `who` at a combine is measured.

**Architecture:** No new module. `history.rs` gets the four-letter rule and a hint. `writeout.rs`'s planner drops a folder it would make only for conflicts standing as nothing (today's `made_on_the_way`, run for every plan), and `kept.rs`'s overlay takes out a folder `resolve --keep` empties when it is gone from disk. `status.rs` hands out empty folders as a difference of their own. `moves.rs`'s pairing takes its changes from any walk, so that `import/convert.rs` can feed it from a comparison of two Git trees. The crash tests and the `who` measurement add tests and a throwaway branch, no code that ships.

**Tech Stack:** Rust 1.99.1 and the crates already in the workspace. No new dependency.

**Spec:** [decision 193](../decisions/193-the-next-version-starts-with-what-v0.2-left.md) (the milestone, the words about older versions, where the work lives), [194](../decisions/194-a-change-ids-start-counts-from-four-letters.md) (short names), [195](../decisions/195-the-import-records-the-renames-it-is-sure-of.md) (renames in an import), [196](../decisions/196-empty-folders-in-a-switch-and-in-status.md) (empty folders). The research behind them is in `.superpowers/sdd/2026-11-17-milestone-11-loose-ends/research-A.md` and `research-B.md` (local only), with `file:line` for everything this plan names.

**Rulings:** [2026-11-17-milestone-11-rulings.md](2026-11-17-milestone-11-rulings.md), kept as the build goes; its "Please confirm" list starts with the few items that carry a real choice.

**Branches:** tasks are built on `milestone-11`, made from `v0.3` (which is `main` at 3f71add, v0.2). It is merged into `v0.3` after its red-team session and whole-branch review (Task 10). `main` is never touched: no pull request into it, no tag, no draft release. Task 8's measuring code lives only on `gate/m11-who-rule`, never merged.

## Global Constraints

- Everything in CLAUDE.md applies.
  - `#![forbid(unsafe_code)]` in the engine and the command-line crate. **No task adds `unsafe`.**
  - No `unwrap`, `expect` or `panic` outside tests.
  - **One place per rule:**
    - reading typed text as a save only in `history::find` (and `is_change_start`), with `MIN_SHORT` the one number for printing and reading a change ID's start;
    - which folders a switch makes only in `writeout::planned` and the function that drops folders made on the way;
    - what counts as a move only in `moves.rs` (`file`, `seen`, the uniqueness rule, `validate_path`, the limits), whatever walks the two sides;
    - how the import turns a Git tree entry into an Opalro entry only in `import/convert.rs`'s existing name judgment and conversion, which the import's comparison calls rather than repeats;
    - Git's words only in `git_words.rs`; colour, symbols and widths only in `style.rs`.
- **Spec before code:** `docs/terminal.md` and `docs/json.md` for every word and field a task prints, in the same commit as the code; `docs/storage-format.md` §4 for what an import notes (Task 6), before the code that writes it.
- **Format 1 is frozen.** No task changes how any record is written. An import writes move notes, which a save's record has held since milestone 1.
- **`opalro_json` stays `1`:** fields are added (`status`'s `empty_folders`), none is changed or given a new code.
- **No flush, write or check changes** in any task. Task 7 adds tests only; if a test shows a fault in the code, it stops and reports.
- **Untrusted text in hints:** no hint puts typed text, a stored name, a path or any other text from the project inside a command a person would paste.
- **No message promises a milestone or a version** (naming the version a feature came with, Task 2, promises nothing).
- **`--json` never asks and never carries colour, symbols, progress or "Next:" lines.**
- **Exit codes:** 0 done, 1 failed or refused, 2 a command line Opalro couldn't use.
- **Tests that can't lie:**
  - a property test's model shares neither the engine's code nor its rule's wording;
  - random tests have fixed seeds and assert that each kind of case they name occurred at least 3 times;
  - a comparison with Git fails when it compared nothing;
  - no assertion on a clock unless the gap is at least tenfold;
  - each rule is broken on purpose once, in the ways the task names; the implementer runs the tests after each break, reports which test caught it, and puts the code back.
- **Test names that read as IDs:** no test resolves a line or tag name of 4 or more letters made only of `k` to `z`, or of 8 or more hex digits, unless the test is about that clash. Names of 1 to 3 letters are safe from Task 1 on, and only Task 1's tests rely on that.
- **One writer at a time:** tasks run strictly one after another. `git add` names paths; never `git add -A` (the untracked `.codex/` and `AGENTS.md` are not ours).
- Code comments are short and human: the why in a line or two, no process words, no task, ruling or decision numbers in code.
- **Values:**

  | What | Value |
  |---|---|
  | Shortest change-ID start read | 4 letters (`MIN_SHORT`) |
  | Shortest fingerprint start read | 8 hex digits (unchanged) |
  | The hint for 1 to 3 letters k to z naming nothing | `A save's change ID is typed with at least 4 letters; see your saves with: opalro history` |
  | The words about older versions | `Opalro before 0.2` where they said `Opalro 0.1` |
  | A status row for an empty folder | `added` or `removed`, the path, `empty folder` where a file's size goes |
  | `status --json` | `empty_folders`: list of `{"path": string, "change": "added" \| "removed"}`, `[]` when none |
  | Move-note limits | 100,000 notes, 16 MiB of paths, 64 MiB to pair (unchanged); `HELD_PER_PLACE` the same number on every computer (203, what it is on a 64-bit computer today) |
  | The `who` measurement's switch | `OPALRO_WHO_RULE=git`, on `gate/m11-who-rule` only |

- **Local runs on this machine:** `export PATH="$HOME/.cargo/bin:/c/Program Files/GitHub CLI:$PATH"` and `export OPALRO_CONFIG_DIR="$(mktemp -d)"` before any test. The machine is short of memory: one cargo command at a time, `-j 1`, `-- --test-threads=2`, one test file per command. A run that memory stops is not started again: say so.
- **A task is done only when its tests pass on Windows, macOS and Linux:** the controller pushes and reads CI on the task's commit before the task is called complete. Tests that make links or are `cfg(unix)` prove nothing on this Windows machine.
- When output changes, run `--test cli` and `--test json_formats` too.

## Review Focus

The inputs the decisions imply and a person is most likely to meet, each pinned by a test in the task that owns the code:

1. **Two files with the same contents in a Git commit that renames one of them** (license files, empty-ish `__init__.py`, generated stubs): no note, and above all no wrong note (Task 6).
2. **A Git commit that moves a whole folder of thousands of files:** one note per file, inside the limits, and the import's time measured (Task 6).
3. **A file edited by the person after a crash that left a finished switch's journal behind:** the next command removes the journal and never touches the edit (Task 7).
4. **Someone used to Jujutsu typing the one to three letters it highlights** (`opalro goto kyp`): the error says a change ID takes at least 4 letters (Task 1).
5. **A folder holding only ignored files, and a folder whose files were all deleted:** what `status` lists is what the next `save` records, folders included (Task 4).

---

### Task 1: A change ID's start counts from four letters

**Files:**
- Modify: `crates/opal-vcs-core/src/history.rs` (`is_change_start`, `ResolveError::hint`, the doc comment of `find`), `crates/opal-vcs-core/src/lines.rs` (the doc comment of `resolve_target`)
- Test: `crates/opal-vcs-core/src/history.rs` (its tests), `crates/opal-vcs/tests/goto.rs`
- Docs: `docs/terminal.md` (the "not found" error's hint), `docs/json.md` (where `goto`, `show`, `changes --save`, `combine`, `tag --save`, `history <from>` and `who --save` read a target: "the start of a change ID, at least 4 letters"), `docs/guide.md` (where it says how a save is typed), `docs/security.md` T21 and T50

**Interfaces:**
- Produces: `is_change_start(text)` true only for at least `MIN_SHORT` letters, all `k` to `z`. `ResolveError::NotFound(text).hint()` gives the four-letter hint when `text` is 1 to 3 letters all `k` to `z`, else today's.

- [ ] **Step 1: Write the failing tests** (in `history.rs`'s tests, with the helpers its neighbouring tests use)
  - `a_start_of_three_letters_names_no_save`: a line of saves; take the first 3 letters of a save's change ID: `resolve` gives `NotFound`; the first 4 still find it (or are `Ambiguous`, as today).
  - `a_line_of_one_letter_is_the_line_whatever_ids_start_with_it`: a save whose change ID starts with letter `L`; a line named `L` at another save: the line's save is found, no `LineOrSave`. The same for a tag named with the save's first 2 letters (no `TagOrSave`).
  - `a_name_of_four_letters_still_clashes`: a line named with a save's first 4 letters at another save is still refused with `LineOrSave` (decision 068 unchanged from 4 letters).
  - `the_hint_for_a_short_start_says_four_letters`: `NotFound("kyp")` and `NotFound("x")` give the four-letter hint; `NotFound("xyz1")`, `NotFound("kyps")` (4 letters) and `NotFound("main")` give `See your saves with: opalro history`.
  - In `goto.rs`: `goto_a_line_named_with_a_save_ids_first_letter`: two saves, the second's change ID read from `history --json`; `opalro line new <its first letter>` at the first; `opalro goto <that letter>` lands on the line (exit 0, `You're on line …`).
- [ ] **Step 2: Run them; they fail** (`cargo test -j 1 -p opal-vcs-core --all-features history -- --test-threads=2`).
- [ ] **Step 3: Implement.** `is_change_start` requires `text.len() >= MIN_SHORT`; the hint is chosen by the same test on the text. The doc comments of `find` and `resolve_target` say "at least 4 letters".
- [ ] **Step 4: Run the tests; they pass**, then `--test goto`, `--test cli`, `--test json_formats`.
- [ ] **Step 5: Break on purpose:** set the minimum to 3 (the three-letter test must fail); give the four-letter hint to every `NotFound` (the hint test must fail). Report which test caught each; put the code back.
- [ ] **Step 6: Docs** as listed; the design.md and decision-record edits are Task 9's.
- [ ] **Step 7: Commit** — `A change ID's start counts from four letters, so a short line or tag name is always the line or tag`.

### Task 2: The words about older versions

**Files:**
- Modify: `crates/opal-vcs/src/setup_output.rs` (`upgrade_offer`, two strings), `crates/opal-vcs-core/src/combine.rs` (the hint of `CombineError::Conflicts { why: NotKept::NoFeature }`)
- Test: `crates/opal-vcs/tests/setup.rs` (around line 883), `crates/opal-vcs/tests/combine.rs` (around line 287)
- Docs: `docs/terminal.md` (lines ~179 and ~461), `docs/json.md` (line ~80), `docs/guide.md` if it quotes either

- [ ] **Step 1:** change the two tests' expected words to `Opalro before 0.2` (`Once it is on, Opalro before 0.2 can't open this project, and opalro undo can't turn it off.`; `Once they are on, Opalro before 0.2 can't open this project, and opalro undo can't turn them off.`; `opalro setup --upgrade lets this project keep conflicts (Opalro before 0.2 can't open it afterwards). Or make the two lines agree on these files, save, and run opalro combine again.`); run them; they fail.
- [ ] **Step 2:** change the strings; run `--test setup`, `--test combine`, `--test cli`; they pass.
- [ ] **Step 3:** the docs, word for word; `grep -rn "Opalro 0.1" crates docs/terminal.md docs/json.md docs/guide.md` finds none of these left (pages about v0.1's own history keep theirs).
- [ ] **Step 4: Commit** — `The program names the version the conflicts feature came with`.

### Task 3: A switch makes no folder for conflicts standing as nothing; `resolve --keep` drops a gone folder it empties

**Files:**
- Modify: `crates/opal-vcs-core/src/writeout.rs` (`planned`; `made_on_the_way`), `crates/opal-vcs-core/src/kept.rs` (`overlay_whole`'s `drop_emptied`), `crates/opal-vcs-core/src/resolve.rs` (the `--keep` call)
- Test: `crates/opal-vcs-core/src/writeout/tests.rs`, `crates/opal-vcs/tests/resolve.rs` (`in_a_deleted_folder` and the two tests after it, ~960-1015), `crates/opal-vcs/tests/redteam24.rs` (RT24-1, un-ignored), `crates/opal-vcs/tests/model.rs` (its `switch`), `crates/opal-vcs-core/src/resolve/tests.rs` (`goto_writes` / `assert_folder_is_goto_of` if they model folders)
- Docs: `docs/releases/known-problems.md` ("An empty folder can appear after a combine": removed), `docs/guide.md` if it mentions the empty folder

**Interfaces:**
- Produces: a `MakeFolder` action with `expected: None` survives `planned` only when the scope names it, or the target holds it as the empty listing as stored (`Listing::empty().id()`), or an action kept below it, or an entry the switch passes over below it (a project link, a name this computer can't write, a refused name). `overlay_whole` takes the folders it may drop when the changes leave them empty as a predicate on the path (`drop_emptied: &dyn Fn(&str) -> bool`) instead of a `bool`; every caller passes what it passes today (`|_| true` or `|_| false`) except `resolve --keep`, which passes "this folder is not on disk".

- [ ] **Step 1: Write the failing tests**
  - In `writeout/tests.rs`: `a_folder_only_conflicts_standing_as_nothing_fill_is_not_made` (a combined save whose folder `d` holds one place in conflict standing as nothing; the plan from a save without `d` has no `MakeFolder d`); `a_folder_saved_empty_is_still_made`; `a_folder_holding_only_a_project_link_is_still_made`; `a_folder_with_a_conflict_standing_as_a_version_is_made` (the file is written into it); `nested_folders_only_conflicts_fill_are_not_made` (`d/e/x.txt` the only place, standing as nothing: neither `d` nor `d/e`).
  - In `resolve.rs`: `in_a_deleted_folder` asserts `d` is **not** made by the combine; `keeping_nothing_below_a_deleted_folder_leaves_the_folder_deleted` asserts the new save holds no `d` and `status` lists nothing for it; `keeping_a_file_below_a_deleted_folder_makes_the_folder_for_it` unchanged. New: `keeping_nothing_where_the_folder_is_on_disk_keeps_it_empty` (the person made `d` by hand before `resolve --keep mine`: the save holds `d` empty).
  - `redteam24.rs`: remove the `#[ignore]` of RT24-1's test and make it assert the fixed behaviour.
  - `model.rs`: the model's switch leaves out a folder whose contents, all the way down, are places in conflict standing as nothing (written from the decision's words, not from the engine's code).
- [ ] **Step 2: Run them; they fail.**
- [ ] **Step 3: Implement** in `planned`: note each `MakeFolder` path the target holds as the empty listing and each path passed over (the `refused` set and the project-link skip); run the generalised `made_on_the_way(actions, only: Option<&Scope>, held_empty, passed_over)` for every plan, deepest first, before `take_over_renamed`. In `resolve --keep`, the predicate asks the folder as the command read it.
- [ ] **Step 4: Run** the `writeout` and `resolve` engine tests, then `--test resolve`, `--test redteam24`, `--test goto`, `--test combine`, `--test conflicts`, `--test undo`, `--test shelf`, `--test discard`, `--test model` (one at a time).
- [ ] **Step 5: Break on purpose:** keep every `MakeFolder` (the first writeout test and RT24-1 must fail); drop every `MakeFolder` with nothing written below (the saved-empty and project-link tests must fail); pass `|_| false` for `--keep` (the resolve test must fail). Report which caught each; put the code back.
- [ ] **Step 6: Commit** — `A switch makes no folder that only conflicts standing as nothing fill, and resolve --keep takes out a gone folder it empties`.

### Task 4: `status` shows empty folders

**Files:**
- Modify: `crates/opal-vcs-core/src/status.rs` (`Difference`, `each_difference`, `Status`, `status`), `crates/opal-vcs-core/src/changes.rs` (ignores the new difference), `crates/opal-vcs/src/main.rs` (`status_command`: rows, count, "Nothing changed", JSON), `crates/opal-vcs/src/output.rs` (the row's right column)
- Test: `crates/opal-vcs-core/src/status.rs` (its tests), `crates/opal-vcs/tests/status.rs`, `crates/opal-vcs/tests/model.rs` (its `changes`/`check`), the status JSON snapshot
- Docs: `docs/terminal.md` (`status`), `docs/json.md` (`status`), `docs/guide.md` (`status`)

**Interfaces:**
- Produces:
  ```rust
  /// An empty folder the save holds and the folder doesn't (`Removed`), or the other way round (`Added`).
  pub struct EmptyFolder { pub path: String, pub change: ChangeKind }
  // Status gains:
  pub empty_folders: Vec<EmptyFolder>, // in path order
  // status.rs's Difference gains:
  EmptyFolder { path: &'a str, change: ChangeKind },
  ```
  A difference is an empty folder where `change_at` gives nothing, one side is a folder entry whose ID is `Listing::empty().id()` and the other side has nothing at that path.

- [ ] **Step 1: Write the failing tests**
  - Engine: `a_saved_empty_folder_gone_from_disk_is_removed`, `a_new_empty_folder_is_added`, `only_the_innermost_new_empty_folder_is_named` (`a/b/` new, `b` empty: one `a/b`), `a_folder_holding_only_ignored_files_is_an_added_empty_folder`, `a_folder_emptied_of_its_files_shows_the_files_not_the_folder`, `a_folder_only_conflicts_fill_is_never_an_empty_folder`.
  - CLI (`status.rs`): `status_shows_an_empty_folder_added_and_removed` (exact rows; `Changed since then (1 folder):` when only folders changed; with one file and one folder `Changed since then (1 file):` and both rows; `Next: save them with: …`); `status_json_lists_empty_folders` (`empty_folders` exact, `changes` unchanged); `status_says_nothing_changed_only_without_folders_either`.
  - Review Focus 5, in `status.rs` (CLI): `what_status_lists_is_what_save_records` (a folder of only ignored files, a folder emptied of its files, a saved empty folder deleted, a new empty folder: after `opalro save`, `opalro show --json` and the save's folders account for exactly the rows `status` printed before it, and `status` then says nothing changed).
  - Model test: its `status` lists empty folders, from the decision's words.
- [ ] **Step 2: Run them; they fail.**
- [ ] **Step 3: Implement.** `changes` ignores `Difference::EmptyFolder` with a one-line comment (it shows what changed inside files). The row's right column is `empty folder`; the count follows `discard`'s rule (folders count only where no file does).
- [ ] **Step 4: Check the pages' claim about a file that became a folder** (terminal.md's `file → folder`, guide.md's "kind changed when a file became a folder"): make one, run `opalro status`, and make the pages say what it prints.
- [ ] **Step 5: Run** the engine `status` tests, `--test status`, `--test cli`, `--test json_formats`, `--test model`, `--test changes`.
- [ ] **Step 6: Break on purpose:** report every folder on one side only (the "emptied of its files" and conflict tests must fail); leave folders out of "Nothing changed" (its test must fail). Report which caught each; put the code back.
- [ ] **Step 7: Commit** — `status shows empty folders, added and removed, as save records them`.

### Task 5: The pairing of moves takes its changes from any walk

**Files:**
- Modify: `crates/opal-vcs-core/src/moves.rs` (`same_contents`, `count_and_moves`, `paired`, `HELD_PER_PLACE`)
- Test: `crates/opal-vcs-core/src/moves.rs` (its tests)

**Interfaces:**
- Produces:
  ```rust
  /// `same_contents`' moves between two sides that `walk` gives: each call of `walk` calls its
  /// argument with every path whose entry differs and the entry on each side, the same paths each
  /// time, in any order. None when holding the files would take more than `most_left_bytes`.
  pub(crate) fn moves_by<E: From<StoreError>>(
      walk: impl FnMut(&mut dyn FnMut(&str, Option<&Entry>, Option<&Entry>) -> Result<(), E>) -> Result<(), E>,
      most_left_bytes: usize,
  ) -> Result<Option<Vec<MoveNote>>, E>
  ```
  `same_contents` and `count_and_moves` call it (or share its body) with `for_each_entry_change_in`; nothing they return changes. `HELD_PER_PLACE` becomes the literal `203` with its comment saying it is what the formula gives on a 64-bit computer, so the bound counts the same everywhere.

- [ ] **Step 1: Write the failing test** `moves_by_a_walk_in_another_order_gives_the_same_notes`: on the module's random pairs of listings (fixed seed), `moves_by` over the listings' differences in reversed path order equals `same_contents`; require at least 3 cases each of: a note, contents that left twice (no note), contents that arrived twice (no note), a link or empty file passed over.
- [ ] **Step 2: Run it; it fails to compile.**
- [ ] **Step 3: Implement** with no change to what any existing test sees.
- [ ] **Step 4: Run** the `moves` and `combine` engine tests, then `--test combine`, `--test merge_vs_git`, `--test who`.
- [ ] **Step 5: Break on purpose:** make `moves_by` keep the first of two files with one contents (the random test must fail). Put it back.
- [ ] **Step 6: Commit** — `The pairing of moves takes its changes from any walk`.

### Task 6: The import notes the renames it is sure of

**Files:**
- Modify: `crates/opal-vcs-core/src/import/convert.rs` (`Converter::commit`, a commit → Git tree map, the comparison of two Git trees), `docs/storage-format.md` §4 (what an import notes; "gives the same saves" said of one version)
- Test: `crates/opal-vcs-core/src/import/convert.rs` (its tests), `crates/opal-vcs/tests/import_vs_git.rs`, `crates/opal-vcs/tests/merge_vs_git.rs` (~1183 and the counts at ~928-945), `crates/opal-vcs/tests/who_vs_git.rs` (imported histories may now draw moves)
- Docs: `docs/design.md` §5.16 is Task 9's; `docs/guide.md` (`who` and imported history), `docs/testing.md` (the tests' rows)

**Interfaces:**
- Consumes: `moves::moves_by` (Task 5) with `MAX_LEFT_BYTES`, then `moves::first_that_fit` with the save's limits.
- Produces: `Converter::commit` sets `move_notes` from the first parent's Git tree and this commit's, compared where their Git IDs differ, each entry made as the converter makes it (the same name judgment as `read_folder`, the same kinds, contents and sizes from `blobs`, so a Git LFS file by what it holds); a name the import leaves out, and what is below it, is in no note.

- [ ] **Step 1: Write the failing tests** (`convert.rs`)
  - `a_rename_with_the_same_contents_is_noted`; `a_rename_with_an_edit_is_not`; `two_files_with_one_contents_give_no_note` (Review Focus 1: one renamed, the other left in place, and the case where both are renamed); `a_folder_moved_whole_notes_each_file` (Review Focus 2: 2,000 files, 2,000 notes, sorted by `to`); `a_link_an_empty_file_and_a_project_link_are_never_noted`; `an_lfs_file_renamed_is_noted_by_what_it_holds`; `a_name_the_import_leaves_out_is_in_no_note`; `a_case_only_rename_is_noted`; `a_merge_notes_against_its_first_parent`; `the_first_commit_notes_nothing`; `importing_twice_gives_identical_saves_with_notes` (today's `converting_twice_gives_identical_saves`, with a history that has a note).
- [ ] **Step 2: Run them; they fail.**
- [ ] **Step 3: Implement** (spec first: storage-format.md §4's lines).
- [ ] **Step 4: Against Git** (`import_vs_git.rs`): `notes_match_gits_exact_renames`: on the generated histories, for each imported save, its notes equal `git diff --name-status -M100% <first parent> <commit>`'s `R100` pairs, leaving out contents more than one file holds on either side (Git picks one pairing there, Opalro notes none); the test fails when it compared no rename, and requires at least 3 histories with a folder moved whole, a rename of an executable, and contents held twice. Run Git first on one hand-made case to confirm what `-M100%` prints, and say so in the report.
- [ ] **Step 5: The tests this changes:** `merge_vs_git.rs`'s `a_file_moved_and_changed_and_removed_on_the_other_line_is_kept_where_git_asks` now expects the stop Git makes (as its Opalro-made twin does); `combine_matches_git_merge` is run and its comment's counts and `docs/testing.md`'s are made to say what it now measures; `who_vs_git.rs`'s imported histories may draw moves, and its doc comment says so.
- [ ] **Step 6: Run** the `import` engine tests, `--test import`, `--test import_vs_git`, `--test merge_vs_git`, `--test who_vs_git`, `--test who`, `--test import_lfs`.
- [ ] **Step 7: Break on purpose:** note against the last parent instead of the first (the merge test must fail); compare Git blob IDs instead of the converter's contents (the LFS test must fail); skip the name judgment (the left-out name test must fail). Report; put it back.
- [ ] **Step 8: Commit** — `An import notes the renames it is sure of, so who and combine follow them in history from Git`.
- [ ] **Step 9 (the controller):** the `Bench` workflow's `import-git` job on `main`'s build against this one (`bench/run.sh import`), three runs each; the times and peak memory go into `docs/benchmarks.md` with Task 9.

### Task 7: The flush after the journal is removed, put to the crash tests

**Files:**
- Test: `crates/opal-vcs/tests/recovery.rs` (and `crates/opal-vcs-core/src/writeout/tests.rs` where in-process is simpler)
- Docs: `docs/testing.md` (the crash tests' section, including line ~143's claim that "a loop kills real Opalro processes at random moments", which nothing in the repository does: the section is made to say what the tests do), `docs/benchmarks.md` (the "For the owner's ruling" table's row 10 and the paragraph after it)

**Interfaces:**
- Consumes: the fault hooks in `fsio::fault` (`OPALRO_FAULT_FAIL=journal:removing`, `fault::act_at("journal:removed", …)`); no new hook.

- [ ] **Step 1: State A** (the journal back with `head` on its target, as a lost removal leaves it): made with `OPALRO_FAULT_FAIL=journal:removing` on a `goto`, then a file the person edits by hand. For every changing command not yet covered by `recovery.rs` (`combine`, `resolve`, `line new`, `line rename`, `line delete`, `tag`, `tag --delete`, `redo`, `undo --to`, `shelf`, `unshelf`, `tidy`, `setup --preset`; `save`, `goto`, `discard` and `undo` are covered): the journal is gone afterwards, the edited file is untouched, no `switch_finished` warning, and the command did its own work (or, for `undo`, `redo` and `unshelf`, said that it finished the stopped switch and took no step, as today). For every reading command (`status`, `changes`, `history`, `show`, `who`, `line`, `tag`, `shelf --list`, `undo --list`, `verify`): exit 0 and the "just after it was done" warning.
- [ ] **Step 2: State B2** (the journal back after a later command moved `head`): a `goto` stopped as above, the journal's bytes kept, the next command run (removing it), a `save`, then the bytes written back. Pin what the code does: every changing command refuses with `JournalUnusable`'s words and its hint, writes nothing, and the folder is unchanged; reading commands warn and exit 0.
- [ ] **Step 3: Run** `--test recovery` (and the writeout engine tests if touched). Every test passes on today's code: these tests pin behaviour; if one shows a fault, stop and report it.
- [ ] **Step 4: The write-up.** In `docs/benchmarks.md`'s table, row 10 says what the tests showed for State A, and a sentence after the table says what State B2 is, that it can only come from a drive that puts a later rename on the disk before an earlier removal in the same folder, which no test here can make, and what a person then meets (every changing command refuses until the journal is deleted, as its hint says; nothing is lost). No recommendation in that page; the follow-ups gain it (Task 9).
- [ ] **Step 5: Commit** — `The journal left after a finished switch is put to every command, and the crash tests' page says what they do`.

### Task 8: Git's rule for `who` at a combine, measured (throwaway)

**Branch:** `gate/m11-who-rule`, from `milestone-11` once Task 6 is in. Never merged; `gate/**` pushes skip CI.

**Files (on that branch only):** `crates/opal-vcs-core/src/who.rs`, `.github/workflows/` (one job, taken from commit `5df6ba2`'s `who-against-blame` job), a short script beside it.

- [ ] **Step 1:** in `who.rs`, only when `OPALRO_WHO_RULE=git` is set: at a save, look up every parent's entry first (following the first parent's note as today); if one parent's contents equal this version's, it takes every line and no parent is compared. Count comparisons, versions read and how often that took a later parent, printed to stderr at the end under the same variable.
- [ ] **Step 2:** the job: build release, import Git's own repository at `7fb6055e678f800320d8ab8cd8f201d9493d1ba0` with `git commit-graph write` first, then for each of `Makefile`, `diff.c`, `git-compat-util.h`, `README.md`, `t/t8002-blame.sh`, 9 runs of `who --json` with the variable off and on, in turns, on the same runner; print the medians, the counters, and how many lines name a different save under the two rules.
- [ ] **Step 3 (the controller):** push the branch, start the job, read it; the table goes into `docs/benchmarks.md` (a section after §15) with Task 9, and the follow-ups' row "Git's rule for `who` at a combine" says what it showed.

### Task 9: The documents

**Files:** `docs/design.md` (§5.1, §5.5, §5.9, §5.10, §5.16, the phase line, the roadmap's v0.3 line, §7: the deferred short-names item closed, "What `who` doesn't follow" narrowed to renames with edits, the flush and `who` rows), `docs/milestones.md` (a v0.3 section with milestone 11's steps and marks), `docs/benchmarks.md` (the import's time with notes; Task 8's table), `docs/security.md` (rows for what the import's notes and the switch's folder rule open to a hostile repository), `docs/testing.md` (the new tests' rows), the follow-ups list ("First in the next version" marked taken; "For the owner to decide": the flush after the journal is removed, with the tests' result and Claude's recommendation; Git's rule for `who`, with the numbers), decisions 055, 068, 159, 161, 168, 170, 184 and 188 (a "Refined by" line each), `CLAUDE.md` (the phase line only).

- [ ] **Step 1: Edit; every statement from a decision, `docs/terminal.md`, `docs/json.md` or the code.** Check each file with `tr -cd '\r' < file | wc -c` (0).
- [ ] **Step 2: Run the `guide` test file.**
- [ ] **Step 3: Commit** — `Milestone 11's documents`.

### Task 10: Red team, whole-branch review, merge into `v0.3`

- [ ] **Step 1: Red team** (decision 024), session RT25, from the no-network sandbox: attack code in `redteam/attacks24/`, proofs in `crates/opal-vcs/tests/redteam25.rs`, the report `docs/redteam/2026-11-17-milestone-11.md`. Where to press: a Git repository crafted so the import notes a move that isn't true, or so the comparison of trees costs without bound (deep trees, many renames, a tree ID repeated); notes that send `who` or `combine` somewhere wrong in an imported history; a short name that reaches a save it shouldn't; a folder made or not made by a switch so that a person's file is lost or `status` lies; `resolve --keep` taking out a folder that holds the person's work; the journal states of Task 7 with a hostile journal. Every proven finding is fixed or ruled on in a decision record; a second session if engine code changed.
- [ ] **Step 2:** the whole-branch review on the range from `v0.3`, pointed at the ledger's deferred minors; one fix wave; one scoped re-review.
- [ ] **Step 3:** the closing pages (the rulings file, the phase lines, the milestones' marks); merge `milestone-11` into `v0.3`; push `v0.3`; read CI on it.
