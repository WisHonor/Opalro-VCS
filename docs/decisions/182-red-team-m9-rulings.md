# 182 — Red team on milestone 9: `--only-if-clean` with nothing to combine, what a command remembers of a conflict, and what a folder's path records

- **Status:** Accepted
- **Date:** 2026-11-15
- **Agenda topic:** none (milestone 9, its red-team session, RT21)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 174](174-the-conflict-record.md) (what a command remembers of the records it reads); [decision 175](175-what-a-combine-keeps-as-a-conflict.md) and [decision 181](181-what-building-milestone-9-refined.md) (what `--only-if-clean` promises, now also when there is nothing to combine); [decision 177](177-recording-a-fix-save-and-resolve.md) (what `opalro resolve` records when it is given a folder's path; what `--copy before` says when it has nothing to copy)

## Context

The red-team session on milestone 9 (RT21, [report](../redteam/2026-11-15-milestone-9.md)) attacked a copy of `milestone-9` at bdbfaf9 in the no-network sandbox, for about ninety minutes.

**What held.** No version was lost and no conflict was settled by a plain `save` or by an ignore rule.

- Moves against a folder that the other line made a file, in both directions, and a file moved to two places with one of them inside such a clash: every version either line saved was in an entry or in a side of a conflict.
- A conflict the two lines' common start held: taken cleanly where one line fixed it, kept as a new conflict with both fixes where both did.
- `--only-if-clean` where either line holds a conflict the combine would carry: refused, in both directions.
- `resolve --keep` over a folder with unsaved work kept a safety copy, and `undo` brought the work back.
- By reading the code: `resolve` between its two looks at the folder; where `--copy` may write (a link as the last name, a path through a link into `.opalro`, a folder swapped after the check); what `tidy` keeps of a record; how marker lines are read.

**Four findings, none High.**

**RT21-1 (Medium): `--only-if-clean` can succeed on a line that holds a conflict.** In plain words: the flag is for scripts, and its one promise (decision 181) is that when it succeeds, the save the line is on holds no conflict. But when the line already has everything the target has, the combine says "Nothing to combine" and ends with 0 before the flag is looked at. So `opalro combine side` keeps a conflict, a script then runs `opalro combine side --only-if-clean && build`, and the build runs on a file full of markers.

**RT21-2 (Medium): `status` and `save` keep every conflict record they read, whole.** In plain words: a conflict record may list up to 255 versions (Opalro writes 3, and reads more so that later versions can). Each command kept every record it met in full, about 12 to 15 KB of memory each for a wide one, with no limit. The store's own memory of records has a budget since the build; the commands' own maps did not, and decision 181 named that as known and not settled. Measured: a planted project of 250,000 such records is 43 MB on disk and made `status` stop at 3 GB of memory; with 20,000 it took 306 MB where the same project with ordinary files took 21 MB. Nothing is written before the stop, so nothing is lost; but a project someone hands you could make `status` and `save` unusable.

**RT21-3 (Low, a judgement): `opalro resolve .` records a picture nobody picked.** In plain words: a picture in conflict is never settled by what happens to be in the folder, because game engines rewrite files just by opening a project. To settle one, a person names it, or says which side with `--keep` (decision 177). A pattern (`"*"`) takes only the places the folder shows fixed. But a folder's path took every place below it as it stands in the folder. So a person fixes a text file by hand, the engine re-exports a picture meanwhile, and `opalro resolve .`, typed the way one types `git add .`, records the engine's picture as the result of the combine.

**RT21-4 (Low): `resolve --copy before` says a file was not there when it was.** Where the saves two lines started from disagree about a file, the middle version of the conflict is unknown, and the record stores it as absent (ruling R7 keeps "unknown" out of the record). `--copy before` then said "There was no file at `<path>` where the lines parted", while the marked file's own label says Opalro can't tell what was there.

**Two things noticed without proof.**

- A line that only moved a folder is told it "changed" a file inside it that the other line holds in conflict. Stopping is right; the word is loose. A conflict that follows a renamed file is already on the follow-ups list.
- `resolve --keep` carries a folder version through one step as an entry of another kind and turns it back by its ID. Only a planted save could hold a second entry with that ID. The whole-branch review is asked to read that code.

**Not reached in the session:** `tidy` past the undo window with shelves and safety copies, `resolve` stopped at each of its writes, histories with three or more common starting points, Git's words.

## Options

**RT21-1**

- **A. With the flag, "nothing to combine" looks through the line's own save, as a forward looks through the target's,** and refuses when it holds a conflict. The flag keeps its one-line promise.
- **B. Say that "nothing" promises nothing.** No code; a script author has to know the exception.

**RT21-2**

- **A. A command keeps of each record only what it uses** (what the conflict stands as, its marker length, how many versions it has, whether it has a version of its own), and reads the record again where it needs all of it. A fixed, small amount per record, so a project of conflicts costs about what a project of files costs.
- **B. Hold the commands' maps to a budget, as the store's is,** and read again past it. Bounded, but the budget is one more number, and under it a wide record still costs 70 times its size.
- **C. Refuse a save that holds more than so many conflicts or versions.** Every limit on what a save may hold is a way to make an honest project unreadable later.

**RT21-3**

- **A. A folder's path is read as a pattern is:** it stands for the places in conflict below it, and of those `resolve` records what the folder shows fixed and lists the rest with what each needs. A place is recorded as it stands in the folder only when it is named by its own path.
- **B. Keep the rule and say in the output which places were taken as changed in place.** The person sees it afterwards, when the save is made.
- **C. Ask before recording a place changed in place below a folder.** One more question, in a command scripts run.

**RT21-4**

- **A. Word the refusal so it is true in both cases.**
- **B. Find out which case it is** by looking for a save where the lines parted. A walk to choose between two sentences.

## Decision

**A, A, A and A.**

**1. `opalro combine <target> --only-if-clean` with nothing to combine** looks through the save the line is on, once, in a project that keeps conflicts, and stops, changing nothing, when it holds a conflict: with the words the flag already has for a conflict a line holds, and exit code 1. A save that cannot be looked through (past a save's limits) is refused with the flag, as a forward's target is. Without the flag, and in a project that does not keep conflicts, "nothing to combine" is what it was. So the promise has no exception: **when `--only-if-clean` ends with 0, the save the line is on holds no conflict.**

**2. A command keeps of a conflict record what it uses, not the record.** What `status`, `save` and every other command that walks the folder remember per record is small and of a fixed size, whatever the record's width. Where all of a record is needed (to write it again with a new version of its own, to take a side), it is read again then, one at a time; the store's own memory of records keeps its budget. The measure: a project of conflict entries takes about the memory the same project of files takes. The red team's proof (20,000 wide records under 120 MB) joins the tests, and the larger run (250,000) is written into the security notes with its numbers.

**3. A folder's path, given to `opalro resolve`, stands for the places in conflict below it, as a pattern stands for those it matches.** Of those places, `resolve` records what the folder shows fixed (exactly what `opalro resolve` without paths records there) and lists the rest with what each needs. **A place is recorded as it stands in the folder only when its own path is given:** then a picture changed in place is recorded as the folder holds it, a file with markers left is asked about, and a place still as the combine left it is refused, as decision 177 says. `--keep mine` and `--keep theirs` are unchanged: a folder's path means every place below it, because the person said which side. `opalro resolve .` is then `opalro resolve` for the whole project, and never decides more.

**4. `resolve --copy before`, where the record has no middle version,** says that Opalro has no version of the file from where the lines parted, because nothing was there or because Opalro can't tell what was, and that there is nothing to copy. One sentence, true in both cases.

**5. A second session** attacks these four fixes and what the first did not reach, before the milestone is merged (decision 024: engine code changed).

## Consequences

- A script can rely on `--only-if-clean` without knowing how a line came to hold a conflict.
- `--only-if-clean` with nothing to combine now reads the line's save where it read nothing. Only with the flag, only in a project that keeps conflicts.
- The known and unsettled point of decision 181 is settled: a planted project of conflicts can no longer take a machine's memory through `status` or `save`. Reading a record a second time, where all of it is needed, costs one read from the store per such place.
- `opalro resolve art/` no longer records a picture that was changed in place below `art/`; the person names it, or says a side. This is the one place where the fix takes something away that worked: someone who put the wanted picture in the folder and named its folder must now name the picture. The output says which places need it. It is on the list of things for the owner to confirm.
- Decision 177's "a folder's path means every conflict below it" holds for `--keep`, and for which places are looked at; it no longer means every place below it is recorded as it stands.
- The model of the command line follows: in it, a folder's path takes what a pattern takes.
- The rulings file gains these four, and the list of things to confirm gains the third.
