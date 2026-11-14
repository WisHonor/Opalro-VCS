# 165 — `combine`: what it takes, what it makes, and when the line only moves forward

- **Status:** Accepted
- **Date:** 2026-11-14
- **Agenda topic:** none (milestone 8 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 012](012-lines.md) (fast-forward behaviour and the combine commands are designed before v0.2); [decision 006](006-save-object.md) (a combined save's first parent is the line you were on); [decision 003](003-working-copy-model.md) (`combine` never overwrites unsaved work without a safety copy); [decision 017](017-commands-and-output.md) (`combine <line>`); [decision 116](116-gits-words-as-shortcuts.md) (Git's word `merge`)

## Context

**In plain words.** `opalro combine wall-run`, typed on `main`, brings everything `wall-run` has into `main`. Three things can be true beforehand:

1. `main` already has everything `wall-run` has. There is nothing to do.
2. `main` hasn't moved since `wall-run` left it. Then `wall-run`'s newest save already *is* "`main` plus `wall-run`'s work".
3. Both have saves the other lacks. Then a new save is needed that holds both.

Decision 012 left the second case (Git calls it a fast-forward) and the command's shape to be designed before v0.2.

## Options

**When the line you are on hasn't moved** (case 2)

- **A. Move the line forward, making no new save** (Git's default; Mercurial with bookmarks). History stays as simple as what happened: the other line's saves are now this line's too. Each save still says which line it was made on (decision 012), so nothing about where the work happened is lost.
- **B. Always make a combined save** (Git's `--no-ff`; Plastic and Perforce always record the merge). History shows a clear "combined here" point, at the cost of a save that changes nothing.
- **C. Do nothing, and have the user move the line** (Jujutsu moves bookmarks by hand). One more thing to learn.

**When a new save is needed** (case 3)

- **A. Make it at once, with a message Opalro writes** unless one is given (Git makes the merge commit at once when nothing conflicts, with "Merge branch …").
- **B. Leave the combined files unsaved** for the user to look at and save (Mercurial's `hg merge`, then `hg commit`; Plastic's merge, then check-in). Safer-feeling, but it needs a "combine in progress" state between the two commands, which every other command must then know about.

## Decision

**A, and A.**

- **`opalro combine <target> ["message"]`.** The target is anything `goto` takes: a line, a server line, a tag, or a save by its ID. The project must be on a line, which is the line that receives the work; at a save the command stops and says to go to a line or start one.
- **Nothing to combine:** when the target is the line's newest save or already in its history. It says so and makes no step.
- **The line only moves forward:** when the line's newest save is in the target's history. The line then points at the target's save, the folder is switched to it as `goto` would switch it, and no save is made. A message given is not used, and Opalro says so.
- **A combined save:** otherwise. Opalro finds where the two parted ([decision 164](164-where-two-lines-parted.md)), merges the two versions of the project ([decision 166](166-merging-two-versions-of-the-project.md)), and makes a save:
  - its parents are the line's newest save, then the target's;
  - the message is the one given, or `Combine <target> into <line>`, naming the target as it was typed, or by its short change ID when an ID was typed;
  - the author and the saver are the user, now; the change ID is new; it is made on the line.
- **The folder** is switched from the line's newest save to the result exactly as `goto` switches: unsaved work comes along where nothing collides, and where the combine changes a path that has unsaved work, it stops before changing anything and says to save, shelve or discard that work (decision 050). What a switch never touches, it doesn't touch here.
- **One diary step,** `combine`, which `undo` takes back like a `goto`: the line returns to where it was and the files with it.
- **In milestone 8, a combine that would leave a conflict stops,** names each file with what happened to it on the two lines (the first 100, then how many more), and changes nothing. Milestone 9 keeps such a combine instead.
- **A combine that stops stores nothing,** whatever the reason, except when unsaved work is in the way: then the combined save was already stored for the comparison with the folder, named by nothing, and `opalro tidy` frees it, as after a `shelf` or a `goto` that stops. (A combine that had built more than a million new records before it stopped also leaves one full pack that nothing names.)
- **Git's words:** `merge <target>` runs `combine <target>`. `merge --abort` is explained (`opalro undo` takes a combine back), and so are `merge --no-ff` and `merge --squash`, which Opalro can't do. Git's other options for `merge` are refused as unknown options are on any command.

## Consequences

- No "combine in progress" state exists: after the command, the combine is either made, or nothing happened. `goto`, `shelf` and `undo` need no new rules.
- A project that wants a visible "combined here" save every time (Git's `--no-ff`) can't have it yet; it is on the deferred list (decision 157).
- A crash in the middle is finished by the next command, as for any switch (decision 075). Running the same combine again then finds nothing to combine.
- `history` follows first parents, so after a combined save it lists that one save, not the other line's saves one by one. What `history` and `show` say about a combined save, and how the saves it brought in are found, is [decision 169](169-saves-reached-through-a-combine.md).
- The `--json` document says which of the three cases happened.
