# 175 — What a combine keeps as a conflict, and what still stops it

- **Status:** Accepted
- **Date:** 2026-11-14
- **Agenda topic:** none (milestone 9 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refined by:** [decision 180](180-a-file-that-arrived-where-the-other-line-changed-the-file.md) (a file moved onto a place whose file the other line changed; what the merge sees at a moved file's old place); [decision 181](181-what-building-milestone-9-refined.md) (what `--only-if-clean` promises; when a held conflict stops a combine; three or more starting points; a place both lines hold alike); [decision 182](182-red-team-m9-rulings.md) (`--only-if-clean` with nothing to combine)
- **Refines:** [decision 165](165-combine-the-command.md) (in milestone 8 a combine that would leave a conflict stopped; from milestone 9 it is kept); [decision 166](166-merging-two-versions-of-the-project.md) (what counts as a conflict, entry by entry); [decision 164](164-where-two-lines-parted.md) (starting points that conflict with each other); [decision 007](007-conflicts.md) (a conflicted save is a normal save)

## Context

**In plain words.** Since milestone 8, `opalro combine` joins two lines when nothing clashes, and stops, changing nothing, when something does. That left a person with no way forward but to change one of the lines by hand. From milestone 9 the combine goes through: the combined save holds a conflict at each place that clashes, every version is kept ([decision 174](174-the-conflict-record.md)), and the person fixes them one at a time, when they choose.

This decision says which clashes become kept conflicts, what a conflict's versions are in each case, and which things still stop a combine because they aren't about one file.

Milestone 8 names seven kinds of clash: both lines changed a file; one changed it and the other removed it; both added different things at one place; a file on one line and a folder on the other; a file moved to two different places; moved on one line and removed on the other; moved onto a place where the other line put something new. Three more things stopped a combine in milestone 8 that are not one file's clash: the two lines' common starting points clash with each other; two names one folder can't hold on Windows or macOS; a result past a save's limits.

Two cases are new, because a save can now hold a conflict when the next combine starts: a line that still holds an unfixed conflict, and a starting point that holds one.

## Options

**Which clashes are kept**

- **A. Every clash that is about one place is kept; nothing a line saved is left out of the result without a conflict that holds it.** Moves included: a file moved to two places becomes two conflicts.
- **B. Clashes of contents are kept; clashes of moves and of kinds still stop the combine.** Less to build. The person is still stuck exactly where milestone 8 left them, in the cases that are hardest to see by eye.

Git keeps every one of these in its index and working folder, in several forms. Jujutsu keeps contents and kinds, and has no moves.

**When the lines' common starting points clash with each other** (two lines that were each combined into the other, and the saves where they parted disagree about a file)

- **A. Stop,** as milestone 8 does. There is nothing the person can do: no line holds the starting points.
- **B. Use the clash itself as the starting version** (Git writes the starting points' conflict markers into the file and merges on from that; Jujutsu carries the wider conflict). Merges more by itself, and needs conflicts of more than three versions, or markers inside the version everything is compared with.
- **C. Treat the starting version as unknown at that place.** If the two lines hold the place alike, that is the result. If not, it is a conflict between the two lines' versions, whole. Never guesses, needs nothing new, and gives up merging that one file line by line.

**A line that still holds an unfixed conflict**

- **A. Refuse to combine at all** until every conflict on both lines is fixed.
- **B. Carry conflicts through where only one line touched the place, and ask for a fix first only where both lines changed a place that one of them holds in conflict.**
- **C. Combine the conflicts themselves** into wider ones (decision 007's conflict algebra, planned for later).

## Decision

**A, C and B.**

**1. A clash about one place becomes a conflict there.** Each is a record of three terms: yours (the line you are on), the version where the lines parted, theirs (the line combined in).

| The clash | Yours | Where they parted | Theirs | In your folder ([decision 176](176-a-conflict-in-the-folder.md)) |
|---|---|---|---|---|
| Both lines changed a file, and its text can be shown with markers | your file | the file | their file | the text, with markers at the places that clash |
| Both changed it, and it can't be (not text, too big, its rule says no markers, or only its line endings differ) | your file | the file | their file | your version |
| One line changed it, the other removed it | file or absent | the file | absent or file | your version: nothing, if you removed it |
| Both lines added different things at one place | yours | absent | theirs | markers if both are text, else your version |
| Different kinds at one place (a file and a folder, a link and a file), or two links or project links that differ | yours | what was there, or absent | theirs | your version |
| A file moved to two different places | at your place: your file; at theirs: absent | the file, from its old place | at your place: absent; at theirs: their file | two conflicts: your file at your place, nothing at theirs |
| Moved on one line, removed on the other | the moved file, or absent | the file, from its old place | absent, or the moved file | your version |
| Moved onto a place where the other line put something new | what you hold there | absent | what they hold there | as two added things |

- **Nothing a line saved is left out of the result without a conflict that holds it.** Where a clash over a moved file leaves a line's version at the old place out of the result (the other line changed the file where it was, and the change has no place to land), that version gets a conflict at the old place too.
- A conflict that involves a folder holds the folder whole, as one term; it is not opened file by file. So nothing below such a place gets a conflict of its own: a file that moved into a folder which is itself one side of a conflict is held by that folder, and no record is ever put at a place below another conflict's.
- Every other rule of the merge stays as milestone 8 has it: what only one line changed is taken, text is merged line by line, moves are followed.

**2. What still stops a combine, changing nothing:** two lines with no save in common; more than 16 common starting points, or more than 64 combines among them; two names one folder can't hold on Windows or macOS; a result past a save's limits; unsaved work in the way; a text file both lines changed while either line's `opalro.toml` can't be read whole (decisions [172](172-red-team-m8-session-2-rulings.md) and [173](173-the-rules-of-both-lines-decide-what-a-combine-merges.md): the rules would decide whether it is merged or marked, so fix the rules file, not the file), among the starting points' own combines too; a project that can't keep conflicts yet ([decision 178](178-the-conflicts-feature.md)); and case 4 below.

**3. Where the starting version is unknown,** the two lines' versions decide: alike, that is the result; different, it is a conflict of the two (yours, absent, theirs), whole, with markers when both are text. The starting version is unknown at a place when:

- the lines' common starting points clash with each other there (two starting points with no save in common included: what they hold differently is unknown, not a refusal). Their combine is never written to a drive, so names alike inside it stop nothing either;
- the save where the lines parted itself holds a conflict there, and both lines changed the place since (each fixed it its own way).

"Unknown" is not "absent": a place one line holds and the other doesn't is a conflict here, never an addition. Where both lines hold a folder at such a place, the merge goes into the two folders and decides entry by entry, everything below still unknown.

**4. A conflict a line already holds is carried or asked about, never widened.** In the entry-by-entry rules a conflict is compared as stored: alike on both lines, it is kept; changed on one line only (fixed there, or made there), that line's entry is taken. Where both lines changed a place and one of them holds it in conflict, or holds a conflict somewhere below it (a folder), the combine stops and names the place and the line: fix it there first. So a conflict is never made inside another one.

**5. The command.** `opalro combine <target>` makes the combined save with its conflicts, as one diary step that `opalro undo` takes back whole. It says how many places are in conflict, names the first 100 with how each stands in the folder, and says what to do next. It ends with 0: the combine is done ([decision 017](017-commands-and-output.md)'s three codes stay). `--json` lists the conflicts.

**6. `opalro combine --only-if-clean`** is milestone 8's combine: where a conflict would be kept it stops, names the files and changes nothing, ending with 1. It is for scripts that must not go on with a conflict in the folder.

## Consequences

- A combine never leaves a person stuck on a clash of files: it either goes through or names something they can act on.
- In a criss-cross history a file that the starting points disagree about is not merged line by line; the person gets both lines' versions whole. Git would often merge it. It is the price of never guessing, and of conflicts with three versions only.
- Two lines that each hold the same place in conflict, or one that holds it and one that changed it, can't be combined until that place is fixed on the line that holds it.
- A script that ran `git merge … && …` and now runs `opalro merge … && …` goes on after a combine that kept conflicts, where Git would have stopped it. `--only-if-clean` is the answer, and the words for `merge` in the guide say so.
- Every conflict a combine keeps makes a record, and a marked text when it is shown with markers; they are written as they are made, one file's versions in memory at a time, as merged texts are.
- The seven words milestone 8 uses for the kinds of clash stay, now describing what is kept instead of why the combine stopped.
- The model test and the comparison with `git merge` grow: a combine that Git reports as conflicted must be one Opalro keeps a conflict in, at the same places, wherever both tools follow the same moves.
