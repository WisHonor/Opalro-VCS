# 170 — What building milestone 8 refined

- **Status:** Accepted
- **Date:** 2026-11-14
- **Agenda topic:** none (milestone 8 build)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 164](164-where-two-lines-parted.md) (when the walk's candidates are the answer; starting points with no save in common); [decision 165](165-combine-the-command.md) (what a refused combine stores; Git's `merge` with two lines); [decision 166](166-merging-two-versions-of-the-project.md) (names alike in threes); [decision 167](167-merging-text-line-by-line.md) (how close to Git the text merge is; how a rule's patterns match); [decision 168](168-moves-in-saves-and-in-combines.md) (what a save may hold for its notes; when a move is believed; what a file arriving at another's place means); [decision 169](169-saves-reached-through-a-combine.md) (damage met while looking for a save)
- **Refined by:** [decision 171](171-red-team-m8-rulings.md) (the limit of 16 is applied after the extra candidates are dropped, not before); [decision 180](180-a-file-that-arrived-where-the-other-line-changed-the-file.md) (where another file arrived at a place the other line changed: the conflict a combine now keeps there, and its name); [decision 195](195-the-import-records-the-renames-it-is-sure-of.md) (a combine on imported history follows the import's notes); [decision 196](196-empty-folders-in-a-switch-and-in-status.md) (a combined save's folder that only conflicts standing as nothing fill is not made)

## Context

Decisions 164 to 169 were written before any of milestone 8 was built. Building and reviewing it, task by task, showed sentences in them that were wrong, too strong, or silent. Each was ruled on the same day, with its reason and its cost ([the milestone's rulings](../plans/2026-11-14-milestone-8-rulings.md)). This record collects what changed, so that the six records and the code agree again. Where this record and one of the six differ, this record is right.

The most serious were three cases in which following a moved file, as decision 168 described it, would have put one file's contents under another file's name without a conflict. Each was found by a builder or a reviewer working cases through by hand, before any of it reached a person.

## Decision

**Where two lines parted (decision 164)**

- **The walk's candidates are the answer only when every save is dated later than its parents.** When two saves share a time (common in a history that came from Git, where times are whole seconds) or a clock ran backwards, there can be extra candidates. Each candidate that another has in its history is dropped, exactly, so the answer is right in every case. Decision 164 said "when no save is dated before its parent"; saves dated alike break that.
- **Two starting points that share no save at all** (possible only in a project with more than one first save) are combined from an empty folder: what only one holds is kept, what both hold alike is taken once, and anything they hold differently is the starting points' own conflict. Refusing the combine as "no save in common" would have been false: the two lines do share saves.
- **A refusal met while the starting points are combined speaks of them.** A pair of names one folder can't hold, brought together by two starting points, is their conflict, with no hint: renaming a file on either line changes no starting point. A starting point past a save's limits is named as the save where the lines parted.

**Merging text (decision 167)**

- **Where lines repeat, Opalro and Git can differ, in three ways.** A text with repeated lines can be lined up in more than one way of the same cost, and the two programs pick differently. Then one may merge where the other calls a conflict, or both may merge to different bytes. On generated texts drawn from four lines, when first measured, 12 cases in 256 differed: 4 where only Opalro merged, 7 where only Git did, 1 where both merged differently. Where lines don't repeat, the two agree in every case, mixed line endings and a missing last line feed included. Decision 167 said the bytes must be equal whenever both merge; that sentence is withdrawn. The merge rules and the one line comparison Opalro uses everywhere are unchanged, and Git's way of breaking ties is not imitated.
- **A rule's patterns.** In a rule's `files`, every entry is a pattern: a leading `#` is an ordinary character, not a comment, and a byte-order mark is not dropped. Letter case is ignored in the pattern too, not only in the name: letters outside brackets, escaped letters and single letters in brackets match either case (the ends of a range and the name of a class stay as written, since changing them would change what they hold). Each of these, left as an ignore file reads it, made a pattern match nothing, and a file no rule matches is merged automatically.

**Moves (decision 168)**

- **A save never fails because of its notes, and never holds much for them.** If the comparison with the parent can't be made, the save is written without notes. The notes kept are the first, in order of where the file went, up to 100,000 and up to 16 MiB of path bytes. A first save compares nothing. What a save holds to work out its notes (the files that left, the files that arrived, the listings it stored) is bounded at 64 MiB of memory, past which it notes nothing. The notes and the count of changes a save reports come from one pass over the folders that changed.
- **A move that also flips the executable mark is a move:** the mark isn't contents.
- **A side's notes decide whether it moved a file.** When the notes along a side say the starting point's file at A went to Y, the side moved it, whatever sits at A on that side now; what the side holds at A is a new file. Decision 168 said such a side "has not moved it"; that sentence is withdrawn. With it, a line that moved a file and then made a new one under the old name could have had that new file take the moved file's place on the other line.
- **A note is believed only when it is true of its own save:** compared with that save's first parent, the file left one place and arrived at the other with the same contents. A note that isn't true is ignored, and so are two notes in one save that send one file to two places. A planted note can't send another person's change into an unrelated file.
- **Where another file arrived, the file that was there is gone.** When a side's true notes show another file arriving at a path the starting point held, and that file is still there at the end, the starting point's file counts as removed on that side, for the whole merge. The other side's change to it is "changed here, removed there", never a merge into the file that arrived; and if the other side moved it, that is a move against a removal.
- **What both sides hold alike is the result, whatever the notes say.** This comes before the rule just above (two lines that ended with the same file in the same place have nothing to settle), and after the table of moves (on purpose: there, taking it would bring the wrong-name case back).
- **Without a note, a file replaced in place is a changed file,** and its change follows the file wherever the other side moved it. Nothing is guessed from contents at a path that never emptied. Git does the same. Likewise a file removed and made again in later saves, without a note, counts as the same file along a side: comparing two ends can't tell that from an edit.
- **Following moves by their contents is bounded as a save's notes are.** Past the bound no move is followed by contents for that side, and the combine says so with a warning, `moves_not_followed`.

**Merging the project (decision 166)**

- **Names alike in threes.** Every pair of names alike that the two sides bring between them is refused, also when each of them pairs with a third name that one side already held.
- **A clash of names comes before conflicts:** a combine that would both conflict and bring such a pair says the pair.

**The command (decision 165)**

- **A combine that is refused stores nothing, the result's limits included:** the whole result is measured before anything is stored. Only a combine stopped by unsaved work in the way has stored its save already.
- **`opalro merge` with two lines is explained, not run.** Git means two branches there; running it as one line with a message would do something the person didn't ask for.

**Finding saves (decision 169)**

- **Damage met while looking.** When a save is looked for through every parent and none is found, and a save on the way couldn't be read, the error is the damage, not "no save matches": the save asked for may lie behind it.
- **`opalro history <line>`** keeps short change IDs only for the line the project is on. Any other line, like any other place, is listed with 12-letter IDs.

**Added the same day, from comparing whole combines with `git merge`**

- **A file moved and changed at once, and removed on the other line, is kept without a question.** Opalro follows a move only when it is certain: a move saved by itself is noted, and a history from Git has no notes, so there only identical contents count. A file moved and changed in one save is therefore, to Opalro, an old file removed and a new file added. When the other line removed the old file too, both lines agree that it is gone, one line has a new file, and the combine goes through with the new file kept. Git pairs the two files by their likeness and stops to ask. Opalro's way loses nothing, since the file a person kept and changed is kept; but the other line's removal doesn't reach it, and nobody is asked. Saved as two steps (the move, then the change), the move is noted and the combine stops with a conflict, as in Git. Guessing moves by likeness, in order to ask in such cases, stays deferred ([decision 157](157-v0.2-in-four-milestones.md)). The comparison with `git merge` allows this one difference, counts it, and holds everything else: where both programs combine cleanly, they give the same files.

- **An empty folder is in a combine's result only when a line's own save holds that folder empty.** Opalro saves empty folders, so the question is real. A folder that only the combine would leave empty is not kept: when each line took a different file out of it (by removing it or by moving it away), and when the combine itself emptied it by putting a moved file at its new place. Decision 166 said two folders that merge to nothing stay, "since neither side removed the folder"; that sentence is withdrawn. Neither side holds the folder empty either, the first form of the rule kept it after two removals and dropped it after two moves, and a combine once left behind on disk a folder no line had (found by the comparison with `git merge`, and fixed). A folder a person emptied by hand, and saved so, comes through the ordinary rules like any other entry.

## Consequences

- Opalro's claim about Git is narrower and exact: on text without repeated lines a combine merges a file exactly when Git does, to the same bytes; with repeated lines it can differ either way.
- Moves are followed in fewer cases than decision 168 allowed, and never on a guess. A combine can stop with a conflict where a person might have expected two files to be merged (a file edited on one line and replaced by a moved file on the other).
- A very large reshuffle (more than about 64 MiB of bookkeeping) is saved without move notes and combined without following moves by contents; the combine says so.
- A history from Git, where many saves share a second, runs the exact drop more often; the answer is the same.
- [security.md](../security.md), [terminal.md](../terminal.md), [json.md](../json.md), [storage-format.md](../storage-format.md) §4 and §12, and the [guide](../guide.md) describe the result.
