# Milestone 8 ("Combine"): rulings

The owner asked for v0.2 to be started while they were away, with Claude working alone. This file collects the calls made while planning and building milestone 8. It has three parts:

- **Please confirm:** the calls that most change what Opalro does. Each can be reversed.
- **Every ruling, as made:** each ruling as it was written when it was made, with its reason and what it costs if it is wrong.
- **Deferred:** what this milestone left for later.

The decisions themselves are records [164](../decisions/164-where-two-lines-parted.md) to [169](../decisions/169-saves-reached-through-a-combine.md). Each says "Decided by Claude, at the owner's request while the owner was away. The owner can reverse any part." The plan is [2026-11-14-milestone-8-combine.md](2026-11-14-milestone-8-combine.md).

**Branch:** `milestone-8`, from `v0.2`. Nothing is merged into `main`, which stays what v0.1 is released from.

## Please confirm

1. **`opalro combine <target>` makes the combined save at once** ([165](../decisions/165-combine-the-command.md)), with a message Opalro writes unless you give one. There is no "combine in progress" state between two commands, as Mercurial and Plastic have: after the command the combine is made, or nothing happened. `opalro undo` takes it back.
2. **When your line hasn't moved since the other left it, the line just moves forward** (165): no combined save is made (Git's default; there is no `--no-ff` yet).
3. **Two lines that share no save can't be combined** ([164](../decisions/164-where-two-lines-parted.md)), and there is no flag to force it.
4. **When two lines crossed each other** (each was combined into the other), **their common starting points are combined with each other first** (164), as Git does, so that a change one line took back stays taken back. There is a budget: more than 16 starting points, 64 inner combines, or 256 MiB of text merged on the way, and the combine is refused.
5. **In milestone 8 a combine that would leave a conflict stops,** names the files (the first 100) and changes nothing (165). Milestone 9 keeps such a combine instead.
6. **When Opalro merges a text file by itself** ([167](../decisions/167-merging-text-line-by-line.md)): only when both lines changed it, it is text, and the two sets of changes neither overlap nor touch. Line endings are compared as the bytes they are. A file whose rule in `opalro.toml` says `conflict-markers = false` (Unity scenes, prefabs and `.meta` files in the Game preset) is never merged automatically.
7. **A rule's patterns in `opalro.toml` ignore the letter case of ASCII letters, on every system** (167). A combine's result is stored, so it must not depend on the computer; and this way `*.unity` also covers `Main.UNITY`.
8. **Moves** ([168](../decisions/168-moves-in-saves-and-in-combines.md)): a save notes that a file moved only when Opalro is certain (the same contents gone from one path and present at exactly one new one). A combine follows those notes, so a change to a moved file lands where the file went. A file moved on one line and removed on the other is a conflict. A file moved and changed in one save isn't followed. Folders aren't moved as such.
9. **`opalro history` stays the line's own story** ([169](../decisions/169-saves-reached-through-a-combine.md)): a combined save is one row that says `[combined <id>]`, and `opalro history <line or save>` lists from somewhere else. A save is found by its ID through every parent when the line itself doesn't have it.
10. **Two names one folder can't hold on Windows or macOS** (`Readme.md` from one line, `README.md` from the other) **stop the combine** ([166](../decisions/166-merging-two-versions-of-the-project.md)).

## Every ruling, as made

**Before the build: what the decisions didn't yet say** (written for the planning agent on 2026-11-14, and since folded into the decisions)

**R1. The common starting points, precisely.** `ancestry::common_starts(repo: &Repository, ours: &[Id], theirs: &[Id]) -> Result<Vec<Id>, RepoError>` in `crates/opal-vcs-core/src/ancestry.rs`. Each side is a set of tips, so that a made-up starting point (R2) can be one side.
- Marks per save: OURS, THEIRS, and BELOW ("under a save both sides have"). Every tip of `ours` starts with OURS, every tip of `theirs` with THEIRS.
- A save waits when its marks differ from the marks it last spread. Take the newest waiting save (by saver time, then ID, as `only_in` does), record its marks as spread, and give each parent those marks; if the save holds both OURS and THEIRS, each parent also gets BELOW. BELOW travels on like a mark.
- Go on while some waiting save lacks BELOW.
- The candidates are the saves that end with OURS and THEIRS and without BELOW (whether or not they were ever taken from the queue).
- More than 16 candidates: refuse (R2's error).
- Two or more candidates: drop each one that is in another candidate's history, tested exactly with `only_in(repo, candidate, &[other]) == 0`.
- The promise: the result is exactly the saves both sides have in their history that no other such save has in its history, **whatever the saves' clocks say**. Why it holds: a mark only ever lands on a save that really is in that side's history; BELOW only ever lands on a save under a common save; every path from a tip to a true starting point runs through saves that only one side has, which wait without BELOW and so keep the walk going until the mark arrives. With honest clocks the candidates are already exact; with clocks that run backwards there can be extra candidates, which the exact drop removes.
- The plan's task for it must demand: a property test against a brute-force answer on random histories (up to 3 parents) under four clocks (rising, ties among a few values, random, and the ends of 64 bits), with single tips and with sets of tips; tests that pin how many saves were read on a long history with one starting point (it must not read to the first save); a comparison with `git merge-base --all` in `crates/opal-vcs/tests/import_vs_git.rs` (see how `assert_ancestry_matches_git` in `tests/support/vs_git.rs` does it for `only_in`), with the count of pairs pinned; and that the implementer breaks the algorithm on purpose in three ways and reports that a test caught each.

**R2. Several starting points, and the budget.** As decision 164 says: they are combined with each other first, in ID order, each pair from its own starting points. Add:
- One side of an inner combine can be a made-up listing with several tips behind it; that is why R1 takes sets.
- **A budget per `opalro combine`:** at most 16 starting points at any level, and at most 64 inner combines in all. Past either, the combine is refused, changing nothing, with words that say the two lines' histories cross each other too many times for Opalro to combine. No call depth may grow with the input beyond that budget.
- An inner combine follows the same file-by-file and line-by-line rules. For moves, a made-up side has no chain of saves, so only the second rule of decision 168 (same contents gone from one path and present at exactly one new one) applies to it.
- In milestone 8 a conflict inside an inner combine stops the whole combine, with words that say the lines' common starting points themselves conflict.

**R3. The cases before any merge,** from the existing exact walk: nothing to combine when `only_in(repo, target, &[line_tip])? == 0`; the line only moves forward when `only_in(repo, line_tip, &[target])? == 0`; otherwise a combined save.

**R4. No "combine in progress" state.** The combined save is stored, then the folder is switched to it exactly as `goto` switches. One diary step, `combine`. A crash is finished by the next command like any switch.

**R5. Everything read from a save is untrusted.** The tree walk is iterative and applies a save's limits per side; move notes that name a path that doesn't fit are ignored; counts and sizes are bounded before memory is taken. Every new refusal names what it is about.

**R6. What milestone 8 does not do:** keep conflicts (milestone 9), `who` (milestone 10), guessed moves, moving a folder as such, a drawn graph, `--no-ff`, merge programs, combining unrelated histories.

**Before the build: answers to the planning agent's questions**

A planning agent turned the six decisions into the plan and listed where the decisions, the code and the rulings above disagreed or were silent. These are the answers, numbered as its questions were. Each is in the plan, and in the decisions where it changes what they say.

1. **More than 16 starting points.** `pub enum Starts { Found(Vec<Id>), TooMany(usize) }` and `common_starts(...) -> Result<Starts, RepoError>`. The walk's candidates are counted first: more than 16 candidates is `TooMany(count)` without the exact drop (the drop's cost is bounded that way); otherwise the exact drop runs and `Found` holds the result. With clocks running backwards this can refuse a combine that had 16 or fewer true starting points: safe, never wrong, and said in the decision. The property test asserts `Found` equals the brute-force answer whenever the candidates are 16 or fewer, and that `TooMany` only happens when the brute-force candidates-by-walk exceed 16 (so the generator needs a case with more than 16 true starting points).

2. **Naming the files in conflict.** The first 100 in path order, then `… and N more`. The tree merge keeps at most 100 conflict records and a counter, never an unbounded list. No new field in the `--json` failure document: the stop is milestone 8's only (milestone 9 keeps the conflicts and reports them in the success document), so a structured list there would live for one milestone.

3. **The model test's text merge.** The model's generator gives every version of a file lines that don't repeat within that version. Then the lining-up is unique, model and engine must agree exactly, and there is no "the engine's answer stands" escape. Repeated lines are covered in Task 3 instead: its invariants hold for any texts (generate texts with repeats there on purpose), and its comparison with `git merge-file` runs two generators: unique lines (status and bytes must be equal to Git's, every case) and repeating lines (when both are clean the bytes must be equal; cases where one is clean and the other isn't are counted, printed and bounded by a pinned number the implementer reports with examples). If "both clean, same bytes" fails for repeating lines, the implementer stops and reports: that is a ruling for Claude, not something to paper over.

4. **Two files merge apart only when the starting point holds a file there.** Yes. Two new files that differ in anything (contents or the executable mark) are a conflict.

5. **Two folders that merge to an empty one stay, as an empty folder.** Yes: neither side removed the folder. Only "a folder against a removal" drops a folder that ends up empty.

6. **ID lookup.** Step 1 exactly as today (change IDs on the current line; fingerprints as today); step 2 only on a miss. The check that a line's or tag's name isn't also another save's ID (decision 068) uses both steps, so a line named only with the letters k to z costs the step-2 walk every time it is named. That is correct and accepted for now. Task 9 measures it on Git's own history (the nightly differential job prints the time of a miss); a cache of every reachable save's change ID is a follow-up if the number asks for it. Say this in decision 169's consequences.

7. **Letter case when a rule's patterns are matched.** Neither the drive's rule nor exact: **ASCII letter case is ignored, on every system.** A stored result must not depend on the computer, and of the two fixed choices this is the safe one: `*.unity` then also covers `Main.UNITY`, so a file is at worst not merged automatically (a conflict a person settles), never merged against a rule its author thought covered it. It goes into storage-format.md §12 and decision 167 ("patterns match as `.opalroignore` patterns do, by the same code, with ASCII letter case ignored on every system").

8. **A pattern holding a line break matches nothing.** Yes. Follow-up: warn about such a rule when `opalro.toml` is read.

9. **A file below a matched folder pattern matches.** Yes, `.opalroignore`'s meaning.

10. **A combined save notes moves against its first parent, like any save, and the chain goes through it.** The notes are exact against the first parent, and the chain follows first parents, so a combined save on the way is one more step, not a break. The chain fails only when the starting point isn't on the side's first-parent path (it was reached through a second parent) or when it is made up; then the contents rule applies. decision 168 is reworded: "(a history from Git has none, and the starting point isn't always on the side's first-parent path)".

11. **A made-up starting point:** the contents rule for both sides. Yes.

12. **A side that moved A to X and then made a new file at A:** the move doesn't count. Yes (the conservative reading).

13. **The default message** names a tag by its name, a line or server line by its name, and a save typed by ID by its short change ID. Use whatever `goto` already uses to know that a tag was typed; if nothing does, check the tag list for the exact text after resolving.

14. **Words:** Claude reviews every drafted message in the plan before it is committed. For `StartsConflict` there is no hint, because there is nothing a person can do that Opalro can honestly recommend (milestone 9 replaces this stop before v0.2 is released). The message alone: `Combining <target> into <line> needs their common starting points combined first, and those conflict with each other, which Opalro can't keep yet, so nothing was changed.` Never promise a milestone or a version in a message.

15. **Undo's words for a combine:** `Undid combine: main is back at <id>.` Pinned as is.

16. **What a refused combine may leave.** A combine that stops for conflicts, for too many starting points or for unrelated histories stores nothing (the unwritten packs are dropped). One that stops later (the result's limits, names one folder can't hold if only known after storing, unsaved work in the way) may leave unnamed records, which `tidy` frees, as `shelf` and `goto` can. "Changes nothing" in every message means the lines, the tags, where the project is, and the folder.

17. **Recursion:** bounded by the budget (at most 64 calls deep) is fine.

On the decisions:

- decision 164: R1, R2 and ruling 1 are folded in before committing (done).
- decision 166: the result's whole tree is measured once against the limits on every combine; the cost (one read of every listing of the result) is said in its consequences, and measured on Git's own history. Checking only when the sides' counts could pass the limit is a follow-up.
- decision 167: the consequence about Git is worded as an aim that Task 3's comparison checks, not as a fact.
- decision 165: Git's `merge --abort` is explained (`opalro undo`), and so are `merge --no-ff` and `merge --squash` (not in v0.2), each in its own words as `rebase` and `cherry-pick` are; Git's other options for `merge` are refused as unknown options are on any command.
- `show` and `changes --save` saying "moved": not in milestone 8; a follow-up.

Also binding, from milestone 7's red team (decision 162), since milestone 8 prints names and reads names:

- The target of `combine` is read by the one resolver (`lines::resolve_target`): a line, a server line, a tag, a change ID, a fingerprint; a name that is both a line (or server line) and a tag at two saves is refused there already.
- A stored tag's name written as text goes through the one function that quotes odd names (see `crates/opal-vcs-core/src/text.rs` or wherever milestone 7 put it; check the code). No hint puts a stored tag's name, or any other text from the project, inside a command.
- An error about something stored names it (which save, which file, which tag).

**On the six points the plan still left open**

1. **Merged texts are not held in memory until the end.** They are written into the combine's own unstored pack set as each is made (check in the code that a pack set streams to temporary files and that dropping it leaves nothing behind; `save` of a 10 GiB file peaks at 44 MB, so something streams). Only listings are kept in memory until the combine is known to go ahead; the pack set is dropped on any refusal. If the code shows a pack set can not do this, then cap the merged texts held in memory at 256 MiB in all and refuse past it, naming the cap; say which in the plan.
2. The second oracle (R1's marking re-run plainly in the test) is fine: it pins the threshold; the brute-force answer pins the result.
3. `Named { raw, shown }` is accepted.
4. The cap on the notes walk (`only_in(side, [start]) + 1` steps) is accepted: `only_in` never counts too low, so the cap is safe whatever the clocks say.
5. The three older Git-word messages that name a version (`SYNC`, `REVERT`, `MOVE_SAVES`) stay as they are: they were decided before and are not milestone 8's. The rule binds new words.
6. The controller reads the `Tangled` hint with the other words in the "Words to review" table.

**From reading the finished plan**

Read in full: the header, Global Constraints, Review Focus, Tasks 1 to 7 and the words table. Tasks 8 to 13 skimmed by their headings and words only.

1. **`Tangled` gets no hint.** "Try combining an older save of it first" is not something Opalro can promise will work, so the message stands alone, as `StartsConflict` does.
2. **The result's limits are checked before `store_packs`,** with a reader that looks in `made.listings` first and then in the store, so a result past the limits stores nothing either. Only a refusal for unsaved work in the way (which needs the save stored for the plan) can leave unnamed records.
3. Everything else read is accepted as written.

## Deferred

Kept as the build goes. So far, from the rulings above: a warning when a rule's pattern holds a line break; checking the result's limits only when the two sides' counts could pass them; `show` and `changes --save` saying "moved" for a noted move; a table of every reachable save's change ID, if the walk for a missed ID is too slow.
