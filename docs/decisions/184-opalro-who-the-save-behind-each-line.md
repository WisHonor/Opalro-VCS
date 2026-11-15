# 184 — `opalro who`: the save behind each line of a file

- **Status:** Accepted
- **Date:** 2026-11-15
- **Agenda topic:** none (milestone 10 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 017](017-commands-and-output.md) (`who <file>` is one of the history commands); [decision 021](021-v0.1-command-scope.md) and [decision 157](157-v0.2-in-four-milestones.md) (`who` comes in v0.2, in milestone 10); [decision 116](116-gits-words-as-shortcuts.md) (`blame` was explained until `who` arrived); [decision 168](168-moves-in-saves-and-in-combines.md) (`who` follows move notes back through a file's history); [decision 179](179-every-command-and-a-conflict.md) (what `who` does with a conflict was left to this decision)

## Context

**In plain words.** You are looking at a line of a file and want to know where it came from: which save put it there, who made that save, when, and why. `opalro history` lists saves; it can't answer for one line. `opalro who <file>` does: beside every line of the file it names the save that line is from.

It works backwards, like tracing a sentence through the drafts of a letter. Take the file as it is now and the draft before it. Every line that is the same in both was not written by the newest save, so the question moves to the older draft; a line that is not in the older draft was written by the newer save. Repeat with the draft before that, until every line has a save.

Git calls this `blame` (and `annotate`), Mercurial `annotate`, Jujutsu `file annotate`, Perforce `annotate`.

Four things have to be decided: which version of the file is read; what happens where two lines were combined; how far a moved file is followed; and what is shown.

## Options

**Which version of the file**

- **A. The file as it is in your folder.** Lines you have not saved yet are marked as such. The line numbers are the ones your editor shows. Git does this.
- **B. The file as the save you are on has it.** Simpler, and unsaved edits shift the line numbers against what you see. Mercurial does this.
- **C. A, with a way to ask about a save instead.**

**Where two lines were combined**

- **A. Follow the first parent only** (the line the combine was made on). Everything that came in from the other line is named as the combine's. Simple, and wrong for the question: the combine did not write those lines.
- **B. Follow every parent, the first one first.** A line the combined save shares with its first parent goes there; of the rest, a line it shares with the next parent goes there; what is left is the combine's own (a conflict's fix, for instance). Git, Mercurial and Jujutsu do this.

**A file that was moved**

- **A. Not followed:** the save that moved the file gets every line.
- **B. Followed where a save recorded the move** (decision 168's move notes, each checked against the two saves before it is believed).
- **C. Guessed by likeness,** as Git does. Decision 157 left guessed moves out of v0.2.

## Decision

**C, B and B.**

**1. `opalro who <file>`** reads the file as it is in your folder and names, for every line, the save it is from. A line that is in no save yet is marked as not saved yet. **`opalro who <file> --save <save>`** reads the file as that save has it instead; `<save>` is anything `opalro show` takes (an ID, a line, a tag). It takes one file, by its path, never a pattern.

**2. How a line finds its save.** Starting from the version asked about, each save's version is compared, line by line, with the version each of its parents has, the first parent first. A line the two share belongs to the parent's side and the question goes on there; a line no parent has is that save's. So:

- through a combine, a line goes to the line it came from: the first parent's where both have it;
- what a combine itself wrote (the lines of a fix, the markers of a conflict it kept) is named as the combine's;
- a parent that does not have the file there, or has something there that is not text, takes no lines.

Two lines are the same when their bytes are, **leaving out the line ending**: a save that only changed a file's line endings (`\r\n` against `\n`, or a missing last one) did not write its lines. Everything else counts, spaces too.

**3. A moved file is followed where the move is recorded.** Where a save's move notes say the file came from another path, and the note is true of the save and its first parent (the same check every reader of notes makes: notes are not trusted), the question goes on at the old path. Notes are kept against the first parent only, so:

- a file moved on the line you are on is followed through the move;
- **known limit:** a file that was moved on one line and changed, under its old name, on a line that was later combined in keeps the lines of the changes made under the old name as the combine's. Nothing a save stores says where a second parent had the file. It goes on the follow-ups list.

**4. A place in conflict is read as what stands there** ([decision 174](174-the-conflict-record.md)'s one rule): the text with markers where the conflict is shown with markers, otherwise your version. So in the folder, during a conflict, `who` names the save behind each line of either side, which is what a person fixing the conflict wants to know; the marker lines, and lines only the middle part has, are named as the combine's, which wrote them there. After the fix is saved, the fixed file's lines are followed through the conflict to the two lines' saves in the same way. No rule of its own is needed.

**5. What it refuses, saying why:** a path that is no file there (a folder, a link, a project link, nothing at all), a file that is not text or is too big to compare line by line (the limits `opalro changes` and the text merge already have: a zero byte in the first 8,000, more than 64 MiB, more than 1,000,000 lines), a path outside the project. An older version that is not text or is too big ends the trail there: its lines are named as the save that made the next version.

**6. What it shows.** One row per line of the file: the save's short ID, who made it, the day, the line's number, the line. Rows of one save that follow each other repeat the save (so the output can be searched and cut), dimmed after the first on a terminal. A line not saved yet says so in place of a save. File contents and names are escaped as everywhere. With `--json`: the runs of lines (first line, how many, the save, the path the file had there if it was another) and each save once (its IDs, who, when, its message); the lines' text is not repeated.

**7. Time.** Each comparison has the deadline `opalro changes` has (2 seconds); past it the comparison is coarser, never wrong about a line it does match, and may leave a line with a newer save than the one that first had it. `who` says when that happened. There is no limit on how far back it reads: a file with a long history takes as long as its history is; the command changes nothing and can be stopped.

**8. Git's words.** `opalro blame <file>` and `opalro annotate <file>` run `who`, with the note every shortcut gets (decision 116). The explanation `blame` had until now goes.

## Consequences

- The person at the keyboard sees their own file, with their own line numbers, and can tell at a glance what they have not saved.
- `who` reads only what it needs: for each save on the way, the folder listings along the file's path and the file's two versions. Saves in which the file did not change cost one look each. How long that takes on a long history is measured on Git's own (the nightly run), against `git blame`.
- The comparison is the one `changes` and the text merge use, so a line that moved inside a file is new where it lands, as in a diff. Git can do better with `-M`; that is a later step.
- A re-indented block is named as the save that re-indented it. An option to leave spaces out can come later; line endings are left out now because one save that normalises them would otherwise take every line of every file it touched.
- Against `git blame` the two agree wherever the history has no moved file that was also changed in the same save (Git guesses those; Opalro follows recorded moves) and no line repeated in a file (where two right answers exist). A comparison test on generated histories pins that.
- The model test gains nothing: `who` changes no state. Its tests are its own, the comparison with Git, and a property test (every line of the file gets exactly one save, and that save's version holds the line).
- Decision 179's open row is closed: `who` and a conflict.
- New rows in the threat list: a planted history made to keep `who` busy (bounded per comparison, not in all; read-only); planted move notes (checked before use); what a line or a name can do to a terminal (escaped).
