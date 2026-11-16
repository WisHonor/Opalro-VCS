# 179 — Every command and a conflict

- **Status:** Accepted
- **Date:** 2026-11-14
- **Agenda topic:** none (milestone 9 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 007](007-conflicts.md) (a conflicted save is a normal save: it can be checked out, built on and combined, and its conflicts stay listed until fixed); [decision 017](017-commands-and-output.md) (what commands print)
- **Refined by:** [decision 181](181-what-building-milestone-9-refined.md) (one `Next:` line; `goto` and a forward tell of a conflict; one row for a folder that became a conflict's place); [decision 184](184-opalro-who-the-save-behind-each-line.md) (`who` and a conflict: a place in conflict is read as what stands there)

## Context

**In plain words.** Decision 007 promises that a save with conflicts in it is a normal save. Until now no command had to keep that promise, because no command could make such a save. From milestone 9 one can, and then every command that reads a save, writes the folder or compares the two meets conflicts. Today each of them does something that was only meant as a safe stand-in: write-out leaves a conflict's place alone, `save` drops the conflict and saves whatever file is there, `status` reports the place as changed, `tidy` stops, `verify` calls the listing damaged.

This decision goes through the commands one by one. Most of it follows from one rule, set in [decision 176](176-a-conflict-in-the-folder.md): between a save and the folder, a conflict's place holds the entry the conflict stands as; between two saves, a conflict equals only itself.

Two things need a choice of their own.

**What finding a conflict may cost.** Every project made by v0.2 can hold conflicts, and almost none will hold one at any given moment. `opalro status` is measured against Git and is already at its limit. So a project with no conflict in it must not pay for the ability to have one.

**How `opalro history` could tell that a save holds conflicts.** It reads one small record for each row it prints, never a save's folders.

## Options

**How a command finds the conflicts in a save**

- **A. Read the save's folders and look,** once in each command that meets the folder. Simple to build on: everything after that sees ordinary files. It reads every folder of the save whether or not there is a conflict.
- **B. Meet a conflict where its entry is met.** A command that compares a folder of the save with the drive, or with another save, sees the conflict's entry there and treats it as what it stands as. Nothing is read that isn't read today, until there is a conflict.
- **C. Keep a note or a cache that says which saves hold conflicts.** Fast, but a note inside a save can be planted, and a cache needs a format and a seal of its own.

**A mark in `history` for a save that holds conflicts**

- **A. The save says so itself,** in a field written by whatever makes the save. It can't be kept exact without reading the save's folders each time one is made (a save of only some paths doesn't look at the rest), and it is text a planted project controls.
- **B. No mark for now.** `status` lists the conflicts of the save you are on, `show` says which places a save put in conflict or fixed, and `combine` says how many its save holds.

Jujutsu marks conflicted commits in its log and keeps the fact in an index of its own. Git has no such saves.

## Decision

**B, and B.**

- **No command reads a save's folders to look for conflicts.** A conflict is met where its entry is met, and its record is read then. A project that holds none reads nothing for them: `status`, `save` and `goto` cost what they cost without the feature, and a test holds that.
- **`history` has no mark** in this milestone. It is on the follow-ups, with what it needs (a save that says it holds conflicts, or a cache).

The commands:

| Command | With a conflict |
|---|---|
| `status` | Lists the places in conflict first, each with how it stands: marked places left to fix (and how many), fixed and not yet recorded, your version in place, changed in place, or not in your folder. Then what to do next. A conflict's place isn't listed again among the changes. `--json` carries the list. |
| `changes` | At a conflict's place, shows what changed in the file since the save (the folder against what the conflict stands as), under a line saying the file is in conflict. |
| `save` | [Decision 177](177-recording-a-fix-save-and-resolve.md). A conflict's place is never a file that moved: move notes pass it over. |
| `resolve` | Decision 177. |
| `combine` | [Decision 175](175-what-a-combine-keeps-as-a-conflict.md). It counts the places its save holds in conflict while it measures the result against a save's limits, which reads every folder of the result already. Unsaved work in the way stops it as before; a half-fixed file not yet saved is unsaved work. |
| `show`, `changes --save` | A place that became a conflict in that save reads "in conflict", one that stayed one with another version "still in conflict", one that stopped being one "conflict fixed"; no lines are compared for any of them. `show` counts them. |
| `history` | No mark (above). |
| `goto`, `undo`, `redo` | Write, compare and protect the folder by the one rule: going to a conflicted save writes its marked texts and your versions; leaving one removes them. A conflict's file that wasn't touched is not unsaved work. One that was (half-fixed, or changed in place) is, like any changed file: it stops a `goto` it is in the way of, and `undo` handles it as it handles any unsaved change. `goto` says when it met a place in conflict in the save it went to. |
| `discard` | Gives back what the conflict stands as in the save (the marked text as saved), keeping a safety copy of what was there. |
| `shelf`, `unshelf` | A changed file at a conflict's place is shelved and put back like any changed file; the conflict stays in the save. |
| `tidy` | Keeps everything a kept conflict names ([decision 174](174-the-conflict-record.md)); no longer stops at a conflict record in a project with the feature. |
| `verify` | Decodes every conflict record and checks what it names. |
| `line`, `tag` | Nothing changes. A tag may name a save that holds conflicts. |
| A save's limits | What a conflict stands as counts as the entry there; a folder it stands as is counted with everything in it. |
| `import` | Makes no conflicts: Git has none to bring. |
| `who` | Decided with `who`, in milestone 10. |

**Git's words.** `merge` runs `combine`, and the guide says that it ends with 0 when conflicts are kept, where Git ends with 1 (`--only-if-clean` for scripts). Explained, not run: `merge --abort` (`opalro undo` takes the combine back, conflicts and all); `add <file>` after a merge and `merge --continue` (`opalro resolve`); `checkout --ours <file>` and `checkout --theirs <file>` (`opalro resolve <file> --keep mine` or `--keep theirs`); `mergetool` (Opalro starts no other program: fix the file in your editor, or copy a version out with `opalro resolve <file> --copy`).

## Consequences

- Decision 007's promise is kept by one rule in one place, not by a special case in each command: what the folder holds for a conflict.
- Keeping conflicts costs a project nothing until it has one. Then it costs a record read for each conflict a command meets, and a look into the folders that hold one.
- A save that holds conflicts looks like any other in `history`'s list until that mark is built. `show` on it, and `status` after going to it, say so.
- A person can go away from a half-finished combine and come back: `goto` elsewhere (after saving or shelving half-fixed files), and `goto` back writes the conflicts out again as they were saved.
- Every one of these gets its words in `docs/terminal.md` and its fields in `docs/json.md` before the code.
- Opalro can't yet write a conflicted save into a Git repository; exporting is not in v0.2 (decision 007 notes that it needs a rule).
