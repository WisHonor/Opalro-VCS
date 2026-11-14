# 168 — Moves: what a save notes, and how a combine follows them

- **Status:** Accepted
- **Date:** 2026-11-14
- **Agenda topic:** none (milestone 8 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 006](006-save-object.md) (a save records a move only when Opalro is certain; merges use move notes when present); [decision 012](012-lines.md) (combining follows move notes); [decision 166](166-merging-two-versions-of-the-project.md) (the file-by-file merge)

## Context

**In plain words.** You move `Player.cs` into a `Characters/` folder on your line. Meanwhile, on another line, someone fixes a bug in `Player.cs` where it used to be. When the two lines are combined, the fix belongs in `Characters/Player.cs`. Without knowing about the move, a tool sees something else: "they changed a file that you removed", plus a new file that has nothing to do with it.

Decision 006 gave saves room for move notes and said Opalro records a move only when it is certain. No save of v0.1 records one. This record settles when a save notes a move, and what a combine does with moves.

## Options

**Knowing that a file moved**

- **A. Guess afterwards, from how alike two files are** (Git compares removed and added files and calls a pair that is at least half alike a rename). Works on any history; guesses wrong sometimes, in both directions.
- **B. Note it when saving, and only when certain** (decision 006's choice: the same contents gone from one path and present at another). Never a guess. It misses a file that was moved and changed in the same save.
- **C. Only moves made through a command** (Perforce's `p4 move`). Exact, but a move made in Explorer or in a game engine counts for nothing.

**A file moved on one side and removed on the other**

- **A. A conflict** (Git: "rename/delete"). One person kept the file under a new name, the other threw it away; Opalro can't know who is right.
- **B. The removal wins when the file was only moved, not changed.** Fewer conflicts, but a file one person deliberately kept disappears without a word.

## Decision

**B for knowing, A for the conflict.**

**What a save notes.** Compared with its first parent, a save notes a move from path A to path B when:

- A held a file in the parent and holds nothing in the save;
- B holds a file in the save and held nothing in the parent;
- the two files have the same contents, which aren't empty;
- and no other file that left, and no other file that arrived, has those contents.

So two identical files swapping places, or one file copied to two places while the original goes, note nothing: Opalro isn't certain. Empty files never count: they are all alike. Notes are kept in the save sorted by where the file went (storage-format.md §4), at most 100,000 of them: a save that moves more notes the first 100,000 in that order. Saves made only for safety copies and shelves note nothing. `save --amend` works its notes out afresh. A combined save notes moves against its first parent in the same way, so it records the moves the other line brought in.

**What a combine follows.** For each side, Opalro works out where the starting point's files went:

1. From the notes of the side's saves, back along first parents to the starting point, taken one after another (A to B, then B to C, is A to C). A combined save on the way is one more step: it notes moves against its first parent like any save.
2. For files the notes don't account for (a history from Git has none; the starting point isn't always on the side's first-parent path; and a starting point made up from several has no saves of its own), by the same test between the starting point and the side's newest save: the same non-empty contents gone from one path and present at exactly one new path.

A move counts only when the place the file went to held nothing at the starting point, and the place it left holds nothing on that side now: a side that moved a file away and then made a new file where it was has not moved it, as far as a combine is concerned.

**Then, for a file of the starting point:**

| What happened | Result |
|---|---|
| One side moved it; the other still has it where it was, and nothing where it went | The file is merged at its new place by the ordinary rules, so the other side's changes land there; the old place is empty |
| Both moved it to the same place | Merged there |
| Both moved it, to different places | A conflict |
| One side moved it, the other removed it | A conflict |
| One side moved it to a place where the other side made something new | A conflict |

Folders aren't moved as such: each file in a moved folder is followed by itself, and a file the other side adds in the old folder stays in the old folder.

## Consequences

- A file moved and changed in one save isn't followed, and a change to it on the other line becomes "changed here, removed there". Guessing by likeness would catch it; that stays on the deferred list (decision 157). Saving a move by itself, before editing the file, makes it certain.
- Unity's `.meta` files moving with their asset (decision 006's other signal) need a rule in `opalro.toml` that v0.1 left out; it comes later, with the Game preset.
- A wrong note is possible only through a coincidence: one small file removed, and an unrelated one with the very same contents added, in the same save. The combine's result always shows where each change landed (`opalro show`), and `undo` takes it back.
- Notes in a save are untrusted input, like the rest of it. They can only choose which file a change lands in, never write outside the project, and a note naming a path that doesn't fit is ignored.
- `opalro show` and `opalro changes --save` can say "moved" for a noted move. Whether they do is settled when they are touched for it.
- `opalro who` (milestone 10) follows notes back through a file's history.
