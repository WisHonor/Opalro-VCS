# 177 — Recording a fix: `save` and `resolve`

- **Status:** Accepted
- **Date:** 2026-11-14
- **Agenda topic:** none (milestone 9 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 007](007-conflicts.md) ("fixed" means the markers are gone, or the person marks the file as fixed; a command keeps any version); [decision 003](003-working-copy-model.md) (a save can hold conflicts, so people can keep working and fix them later); [decision 142](142-bolt-sends-nothing-and-each-build-proves-it.md) (Opalro starts no other program)
- **Refined by:** [decision 181](181-what-building-milestone-9-refined.md) (renames; names; `save <paths>`; what `resolve` takes, records and refuses; where `--copy` may write; "N places"); [decision 182](182-red-team-m9-rulings.md) (a folder's path given to `resolve` is read as a pattern is; what `--copy before` says with nothing to copy); [decision 183](183-red-team-m9-session-2-rulings.md) (a folder version kept by `--keep` is told by its path); [decision 185](185-save-amend-and-a-save-something-else-holds.md) (`resolve` decides by the same rule whether a fix replaces the combined save); [decision 196](196-empty-folders-in-a-switch-and-in-status.md) (`resolve --keep` takes out a folder it empties that is gone from disk)

## Context

**In plain words.** After a combine with conflicts, a person edits the marked files, or decides for each picture or scene which line's version to keep. Then Opalro has to *record* that: the place stops being a conflict and becomes an ordinary file again. This decision says which commands record a fix, and where the fix goes in the history.

Three things pull in different directions:

- **Nothing may be lost by saving.** Decision 003 lets a person save in the middle of fixing. So `opalro save` must work on a folder with conflicts, half-fixed files included.
- **A file that changed by itself must not decide anything.** For a conflict shown with markers, decision 007 says "markers gone" means fixed. A conflict that stands as your version has no markers to remove, and game engines rewrite such files just by opening the project. If "the file differs from your version" counted as a fix, a combine's other side could be dropped without anyone choosing.
- **Where the fix lands.** People who know Git expect the combined save itself to hold the result. Opalro's saves aren't edited in place, with one exception: `save --amend` replaces the line's newest save when nothing else shows it.

## Options

**Where a fix is recorded**

- **A. Always a new save on top.** Simple: history only grows. Every combine with conflicts leaves two or more saves, the first of them conflicted for good.
- **B. Into the combined save,** replacing it as `save --amend` does, when that is allowed; a new save otherwise. History ends with one combined save that holds the result, as a Git merge commit does (and as Jujutsu users get by squashing their fix into the merge).
- **C. No recording step: a plain `save` both records and replaces.** Fewest commands, but `save` would sometimes replace a save and sometimes add one, and a person couldn't tell which beforehand.

**What `save` does with a conflict that stands as a version, when the file in the folder changed**

- **A. Counts it as fixed** with what the folder holds.
- **B. Keeps it a conflict, and keeps the changed file** as the conflict's own version. Only `resolve` fixes it.
- **C. Refuses to save** until it is resolved (Git refuses to commit with unmerged paths).

## Decision

**B, and B.**

**1. `opalro save` on a folder with conflicts** saves everything as always and, at each conflict's place, follows what the folder shows ([decision 176](176-a-conflict-in-the-folder.md)):

| The folder, at the conflict's place | What the save holds there |
|---|---|
| Not touched | the conflict, as it is |
| Shown with markers, and markers are left | the conflict, with the file as it now is as its own version: half-fixed work is saved |
| Shown with markers, and none are left (or the file is gone, or something else is there) | an ordinary entry: fixed |
| Stands as a version, and the folder holds something else | the conflict, with what the folder holds as its own version: saved, still to decide |

`save` says how many of the places it looked at are still in conflict. It never asks about them and never refuses because of them.

**2. `opalro resolve` records fixes.**

- **`opalro resolve`** records every conflict the folder shows as fixed, and lists what is left with what each needs.
- **`opalro resolve <path>…`** records those places as they stand in the folder (a folder's path means every conflict below it). A file that still holds marked places is asked about first (`--yes` answers; without a terminal it is refused). A conflict that stands as your line's version, as the combine left it, is refused: the person must say which version, with `--keep`. One that already holds a version of its own, saved earlier, is recorded as that version. A path that isn't in conflict is an error. If any path is refused, nothing is recorded.
- **`--keep mine` or `--keep theirs`** puts that side's version in the folder, whole, and records it. `mine` is the line the combine was made on (the record's first term), `theirs` the line that was combined in (its last). If the folder held something at that place that no save holds, a safety copy is kept first, as `discard` keeps them. Paths are required: `opalro resolve . --keep theirs` is the way to take one side everywhere.
- **`--copy mine|theirs|before --to <file>`** writes one version of one file out to a place the person names, so they can look at it or open it in their own program, and records nothing. `<file>` must not exist and may not be inside `.opalro`. It is how a person sees the other line's picture before choosing: Opalro starts no other program (decision 142).
- A conflict of more than three versions (none can be made by 0.2) is recorded as it stands in the folder; `--keep` and `--copy` say they can't choose a side of it.

**3. Where `resolve` records.** When the line's newest save is a combined save that `save --amend` may replace (no other line reaches it, no tag or server line names it), the fix goes into it: the save is replaced by one with the same parents, message, author and change ID, whose folder differs only at the places recorded. Otherwise `resolve` makes a new save on top, with the message `Fixed conflicts in <path>` or `Fixed conflicts in N files`. Either way it is one diary step named `resolve`. `opalro undo` takes it back: the line returns to the save it was at and the folder to what it held before the step, so a fix made by hand is still in the folder.

**4. Everything else is `save`'s.** `resolve` saves only conflicts' places: other unsaved changes stay unsaved. It meets every check `save` makes on the files it records. It needs a line to record on: at a save that no line is at, it says so, and how to make a line there.

## Consequences

- The usual path is short: `opalro combine side`, fix the marked files, `opalro resolve`. History then holds one combined save with the result in it.
- A person who prefers to keep working can: `save` carries conflicts along, half-fixed files and all, and `resolve` records them whenever they are done. When other saves came in between, the fix is a save of its own and says so.
- A picture or a scene in conflict is never settled by accident. It stays listed until someone runs `resolve` with `--keep`, or names the path once they have put the version they want in the folder.
- Until a fix is recorded, the combined save is a conflicted save: `show` says so, and combining it with a line that changed the same place is refused ([decision 175](175-what-a-combine-keeps-as-a-conflict.md)).
- `resolve` replacing a save is the second way a save is replaced, after `save --amend`. It uses the same check and the same code, and it never replaces a save anything else shows.
- Git's `add` (to mark a file resolved), `checkout --ours`, `checkout --theirs`, `mergetool` and `merge --continue` are explained with `resolve`'s forms ([decision 179](179-every-command-and-a-conflict.md)).
