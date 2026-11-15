# 171 — Red team on milestone 8: one finding fixed, one bounded and the rest of it ruled on

- **Status:** Accepted
- **Date:** 2026-11-14
- **Agenda topic:** none (milestone 8, its red-team session, RT19)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 164](164-where-two-lines-parted.md) (when a combine is refused for too many starting points); [decision 168](168-moves-in-saves-and-in-combines.md) (how many move notes a combine checks); [decision 170](170-what-building-milestone-8-refined.md) (the limit of 16 was applied to the walk's candidates); [decision 160](160-whether-a-save-is-in-anothers-history.md) (what a walk over history reads of a save)
- **Refined by:** [decision 172](172-red-team-m8-session-2-rulings.md) (the second session: the fixes held; one finding of its own, about rules that can't be read)

## Context

The red team's session on milestone 8 (RT19, [report](../redteam/2026-11-14-milestone-8.md)) attacked a copy of `milestone-8` at c6f8222 in the no-network sandbox. It found two things, both low, and nothing that loses or mixes what a person saved.

What held: 430 random histories of two lines, with edits, moves, removals, new files and folders, each combined through the real command. 201 combines went through and 229 were refused. In none did an added line go missing, a removed line come back, a file hold another file's lines, a file sit under a name neither line gave it, or the folder differ from the save; `undo` and `redo` gave back exactly what was there; and no refusal changed or stored anything. The table of moves, worked by hand, held too.

- **RT19-1 (Low).** A combine of two lines that parted once could be refused as "crossing each other too many times". The search for common starting points counted its candidates, and gave up past 16, before it dropped the extra candidates that saves dated alike or out of order can leave ([decision 170](170-what-building-milestone-8-refined.md)). With seventeen such extras the combine was refused though the lines parted at one save; with all saves dated alike, the same history was refused or not depending on the saves' IDs. Nothing was lost, but a combine a person needs was refused, by chance.
- **RT19-2 (Low).** A history planted with saves that hold very many move notes makes a combine slow. Fifty saves of 100,000 untrue notes each (2.4 MB of packs) made a combine take 22 seconds in the sandbox's debug build. The report put this down to checking the notes.
- **Noted for the owner, as designed.** A file one line removed can come back under another name, with no question, when the other line moved it onto a path the starting point held: decision 168 says such a move doesn't count, so the moved file reads as the other line's change to the file that was there. `git merge` gives the same files.
- **Not tried, for lack of time:** stopping a combine inside its switch (the switch is `goto`'s, attacked in milestone 3), a save with 1,024 parents in `show`, a rule in `opalro.toml` changed on one of the two lines, and v0.1 reading a project with combined saves.

**RT19-2, measured.** Before ruling, the planted history was timed on the owner's computer with a clock around each reading of a save. In a release build the combine took 5.7 seconds, and checking the notes was a tenth of it:

| Where the time went | Seconds | |
|---|---|---|
| The four walks over history (is one line in the other, where they parted, how far a line's notes reach) | 2.7 | each builds every save's notes in memory and uses none |
| The change-ID index, reading the new saves once | 0.7 | builds the notes, uses none |
| The notes pass, reading the saves | 0.7 | uses the notes |
| The notes pass, checking the notes | 0.6 | the part the report named |
| The rest of the combine | 1.0 | |

A save holding 100,000 notes is a record of 1.6 MB. Reading one costs 3 ms to unpack it and check its fingerprint, and 10 ms to build its notes (two pieces of text each); checking the notes as the format demands without building them costs 2.5 ms. So the cost is mostly in reading large saves, six times each, and that is older than milestone 8: format 1 has allowed a save 100,000 notes since v0.1, and every reader has built them. What milestone 8 added is the notes pass, and a command that walks the history four times.

## Options

**The limit of 16 (RT19-1)**

- **A. Apply it to the starting points that are left after the extra candidates are dropped,** and drop them in a way whose cost doesn't grow with their number.
- **B. Keep counting candidates, with a higher limit.** Still refuses by chance, only more rarely.

**Saves with very many notes (RT19-2)**

- **A. Bound what a combine checks, and stop building notes nobody uses.** A budget on the notes one combine checks; walks and the change-ID index check a save's notes without building them. Small, and measured: it takes away most of what milestone 8 added. Reading a large save still costs in proportion to its size.
- **B. Only the budget,** as the report proposed. It would take half a second off 5.7.
- **C. Hold a save's notes unbuilt for every reader, or keep an index of each save's parents and date on disk** (the index [decision 160](160-whether-a-save-is-in-anothers-history.md) set aside; Git's commit-graph file). The first changes how every part of Opalro holds a save; the second is a new stored format. Either makes large saves cheap to walk; neither is a bound.

## Decision

**A, and A.**

- **RT19-1.** The limit of 16 applies to the common starting points themselves. When the walk leaves more than 16 candidates, the ones that another candidate has in its history are dropped in one pass that reads each save behind them once, not by one walk per candidate; with 16 or fewer, as before. Only then is the count compared with 16. So two lines are refused for too many starting points only when they really have more than 16, whatever their saves' dates say.
- **RT19-2, the notes a combine checks.** One combine checks at most 1,000,000 move notes in all, true or not. Past that, notes are no longer believed for the rest of that combine and saves are no longer read for them: files are followed by their contents alone, within the bound that has, and the combine says so once, with a warning of its own (`too_many_move_notes`). A note is set aside at its first check that fails, before the costlier ones.
- **RT19-2, reading a save for its place in history.** What needs only a save's parents, date or change ID (the walks over history, the change-ID index) reads those and checks the rest of the record exactly as before, notes included, without building the notes. A save that can't be read whole is still refused wherever it is read: there is one check, with or without the notes kept.
- **RT19-2, what stays.** Reading a save costs in proportion to its size, for every reader, and a combine reads each save several times. A history planted with thousands of the largest saves stays slow to walk, as a Git history with thousands of very large commits is slow for Git, which unpacks and reads every commit it walks. Option C is not taken now: it bounds nothing, the finding is low and needs a planted history, and until Opalro shares projects such saves arrive only inside a folder somebody else made. It is on the [follow-ups](../plans/2026-11-01-milestone-1-follow-ups.md), with `history` and `show`, which build notes they don't show.
- **RT19-2's proof, restated.** The red team's proof asserted that the planted combine takes under 5 seconds in a debug build, a bound chosen in the belief that checking notes was the cost. With the fix above it still takes longer than that in a debug build, almost all of it reading. A wall-clock bound that passes on three kinds of computer and fails without the fix can't be set, so the proof now asserts what the fix promises: the planted combine goes through and says the notes were too many. Tests in the engine count the notes a combine checks and show that a walk builds none. The times, before and after, are in [`docs/benchmarks.md`](../benchmarks.md).
- **The designed behaviour stays,** for the owner to weigh ([rulings](../plans/2026-11-14-milestone-8-rulings.md), "Please confirm").
- **A second session** attacks these fixes and tries what this one had no time for.

## Consequences

- A history planted to leave very many candidates costs the search one more read of the saves behind them, once.
- A combine whose lines hold more than a million move notes between them follows moves by contents past that point, and says so. A save holds at most 100,000 notes, so this takes more than ten saves that each moved as many files as a save can note.
- A combine has one more warning, in `--json` too.
- The first proof test (`crates/opal-vcs/tests/redteam19.rs`) runs as the red team wrote it; the second runs as restated here. The probe for drives that ignore letter case (`redteam19_platforms.rs`) runs on Windows and macOS.
- Whether to make large saves cheap to walk (option C) is the owner's to weigh: it is on the rulings' "Please confirm" list.
