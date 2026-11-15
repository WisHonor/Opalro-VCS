# 180 — A file that arrived where the other line changed the file that was there

- **Status:** Accepted
- **Date:** 2026-11-15
- **Agenda topic:** none (milestone 9 build, Task 10)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 170](170-what-building-milestone-8-refined.md) ("where another file arrived, the file that was there is gone": what the combine now keeps there, and how it names it); [decision 175](175-what-a-combine-keeps-as-a-conflict.md) (its table gains this clash; what the merge sees at a moved file's old place)

## Context

**In plain words.** A place in the project held a file when two lines parted, say `notes.txt`. On one line you edited it where it was. On the other line someone removed it and moved a different file, `draft.txt`, to that same place, so their `notes.txt` is really the old `draft.txt`. When the lines are combined, one place has two files that have nothing to do with each other.

Milestone 8 stopped such a combine. Decision 170 said how it was named: the file that was there counts as removed on the line where another arrived, so your edit is "changed here, removed there", and it is never merged into the file that arrived. That was written for a combine that stops. From milestone 9 a combine keeps the conflict ([decision 175](175-what-a-combine-keeps-as-a-conflict.md)), and its table has no row for this case: it has "moved onto a place where the other line put something new", where nothing was there when the lines parted.

Building Task 10 showed what the first attempt kept: a record with nothing in the middle, shown with markers whose middle label said the place "held nothing" where the lines parted, under the row "changed on one line, removed on the other". The label was false (the place held `notes.txt`), and the row said nothing about the file that arrived, which is what half of the markers show.

The same task met a second question at a moved file's **old** place. When a file is moved to two places, or moved on one line and removed on the other, and the line that moved it then made a new file under the old name, the merge compared that new file with the file that had left, and called it "changed on one line, removed on the other".

## Options

1. **Keep decision 170's words for it.** The record `[yours, nothing, theirs]`, the row "changed, removed". True of the file that left, silent about the one that arrived, and the middle label is wrong.
2. **Name it as what it is for the place: something moved here onto something else.** The record holds the file the place had where the lines parted as its middle term; the row is the moved-onto one, which names where the arriving file came from.
3. **Two conflicts at one place.** Not possible: a place holds one entry.

## Decision

**Option 2.**

1. **The arrival case is a moved-onto conflict that knows what was there.** Where one line changed a file in place, and the other line removed it and moved another file to that place:
   - the record is yours, **the file the place held where the lines parted**, theirs;
   - its row is `<path>: moved here from <x> on <a>, where <b> has something else`;
   - when both files are text and both lines' rules allow markers under every name either file had, the file is written with one marked place holding **your file whole, the start's file whole in the middle, and their file whole**. The two are never merged line by line: they are different files, and lines they happen to share mean nothing. Otherwise the place stands as your version.
   - Decision 170's substance stays: the file that was there is gone on the line where another arrived, and the other line's change is never merged into the file that arrived.

2. **At a moved file's old place, the merge does not see the file that left.** For a file moved to two places, or moved on one line and removed on the other, the starting version's entry at the old place is left out of what the merge compares there. Each line's own entry there is never touched. So a new file that a line made under the old name is that line's addition (taken, or a conflict with what the other line put there), not a change to the file that moved away. The starting version of the moved file is held as the middle term of the conflicts kept where it went.

## Consequences

- Decision 175's table reads, for this clash: yours: what you hold there; where they parted: the file that was there; theirs: what they hold there; in your folder: one marked place with both files whole, or your version.
- `opalro resolve <path> --copy before --to <file>` gives the file the place held where the lines parted, which is what a person needs to see what the other line changed before deciding.
- A file moved onto a place where the other line holds the very same file is no clash: both lines hold the same thing there, it is taken once, and the old place is empty as the line that moved it left it. Milestone 8 named a conflict there. (The model of the command line met this case in CI and disagreed with the engine until it learned the rule.)
- A combine that milestone 8 stopped with a "changed, removed" row at a moved file's old place, because the mover had made a new file under the old name, now goes through there with the new file taken. Nothing a line saved is left out: the property test of Task 10 checks every version each line made against the result.
- The model of the command line (`crates/opal-vcs/tests/model.rs`) and the engine's own model of moves follow the same two rules, each written apart from the engine.
- The code for this was written under a ruling made during the build (the ledger's Task 10 entries) before this record; the record makes it a decision the owner can reverse.
