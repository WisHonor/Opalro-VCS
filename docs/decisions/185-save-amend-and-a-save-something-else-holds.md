# 185 — `save --amend` and a save something else holds, through every parent

- **Status:** Accepted
- **Date:** 2026-11-15
- **Agenda topic:** none (milestone 10 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 074](074-save-amend.md) (when an amend is refused); [decision 112](112-rulings-after-the-second-red-team-session-on-the-speed-work.md) (the check before an amend keeps what it read of the other lines); [decision 157](157-v0.2-in-four-milestones.md) and [decision 169](169-saves-reached-through-a-combine.md) (milestone 10 changes the check); [decision 160](160-whether-a-save-is-in-anothers-history.md) (one module answers every question about ancestry); [decision 177](177-recording-a-fix-save-and-resolve.md) (`resolve` uses the same check to decide whether a fix replaces the combined save)

## Context

**In plain words.** `opalro save --amend` replaces your line's newest save with a corrected one. That is only safe while nobody else is looking at the old one. If another line, a tag or a server line still holds the old save, the amend leaves two versions of the same change in the project: yours, corrected, and theirs, not. So an amend is refused when something else holds the save (decision 074).

Until now "holds" was checked the way v0.1 could have it: another line's history was followed back along first parents only, and a tag or a server line counted only when it named the save itself. v0.1 had no combines, so that was nearly the whole truth. Since milestone 8 it is not:

- line `feature` combines `main` into itself. `main`'s newest save is now in `feature`'s history, as the combine's second parent. Amending it on `main` goes through today, and `feature` keeps the old one;
- a tag two saves further on, on a line that was since deleted, reaches the save without naming it. Amending goes through today.

The exact question exists since milestone 7: whether a save is in another's history, through every parent, whatever the clocks say (decision 160). `line delete` already asks it.

What it costs is the open point. The old check kept a cache of what it had read of the other lines (decision 112), because a project imported from Git with hundreds of old release lines paid 20 seconds on every amend without it. The exact walk reads at least as much in such a project.

## Options

- **A. Ask the exact question on every amend.** Right, and as slow as the red team found the first-parent walk to be (decision 112's numbers: 300 lines of 2,000 saves each, 19 to 21 s an amend).
- **B. Keep the first-parent check with its cache, and add the exact question beside it.** Two checks with two answers to explain, and the slow one still runs every time.
- **C. Ask the exact question, but only of what could have changed the answer.** A save can only be in the history of saves made after it. When Opalro makes a save, nothing else can hold it yet. So if it notes, at that moment, what every other line, tag and server line pointed at, a later amend needs to ask only about those that have moved since. In the usual case (save, then amend) that is none, and the amend reads nothing.
- **D. A table of every save each line reaches,** kept up to date. Fast always; a new stored structure to specify, keep right and defend. Git has one (the commit-graph) and Jujutsu an index; both are the kind of thing decision 160 put off until measurements ask for it.

Git never checks: an amend of a commit others have is the user's risk. Jujutsu refuses to rewrite what its "immutable heads" reach (the main branch, tags, untracked remote branches), through every parent, using its index. Mercurial keeps a phase on every commit.

## Decision

**C.**

**1. The rule.** `opalro save --amend` is refused, changing nothing, when another line, a tag or a server line names the save **or has it in its history, through any parent**. The refusal names what holds it (the first few, when there are many) and says to save a new change instead. `opalro resolve` uses the same rule, by the same code, to decide whether a fix replaces the combined save or becomes a save of its own (decision 177).

**2. The question is the ancestry module's** (decision 160): exact whatever the clocks say. The walk along first parents by change ID, and the cache it kept (`cache/other-lines`), are no longer used.

**3. What is asked of.** When a command makes a save (`save`, `save --amend`, a combine that makes a save, a `resolve` that makes or replaces one), it leaves a note in the cache folder: the save it made and the diary entry it wrote. An amend of that save then compares the other lines, tags and server lines as that entry has them with what they are now, and asks the exact question only of those that are new or have moved. Those that have not moved pointed, when the save was made, at saves that already existed, and a save that existed before another cannot have it in its history.

- **Without a usable note** (the save was made by an import, by Opalro 0.1, on another computer; the note is for another save; the cache is gone or its seal is wrong; the diary entry it names has been tidied away) the question is asked of every other line, tag and server line. That is the slow case, paid once: the amend it allows leaves a note.
- **The note is a cache like the others** ([decision 100](100-caches-are-written-without-a-flush-and-only-when-they-changed.md)): sealed with the user's key, written without a flush or the lock, skipped if the write fails. `docs/storage-format.md` §10 gets its layout and a test vector before code writes it, and says that `cache/other-lines` is no longer written or read (a file Opalro 0.1 left is ignored).
- **What it is trusted for:** which lines, tags and server lines need asking about. A wrong note under a good seal can make the check miss one: the amend goes ahead, that line keeps the old save, and `opalro undo` takes the amend back. It cannot change a stored save. This is the trust decision 112 gave the cache it replaces.

**4. Another save with the same change ID no longer counts.** The old check matched by change ID, so it also refused when another line held a different save of the same change (an older copy of an amended save, reached by going to it by its ID and starting a line there). The new one asks about the save itself. In that case two saves of one change were already visible before the amend and still are after it; refusing changed nothing about that. `opalro history` shows both, as it did.

**5. Measured** in milestone 10, and written into `docs/benchmarks.md`: an amend right after a save in a project with 300 old lines of 2,000 saves each (the red team's shape: it must read no save of another line); the slow case in the same project; the slow case on Git's own history with its tags and server lines.

## Consequences

- An amend can no longer leave another line, a tag or a server line holding a save that its own line has replaced, however that line came to hold it.
- Save, then amend, reads nothing of the other lines, from the first amend on. Before, the first amend in a project read every other line once.
- The slow case is slower than the old check's second run was: after working on another line, an amend asks about that line exactly, which reads back to where it parted. In a project with hundreds of lines and no note at all, it is the 20 seconds the red team measured, once. If the measurements say this bites, option D is the next step, and the follow-ups list says so.
- One cache goes and a much smaller one comes: a save's ID and a diary entry's ID, instead of runs of saves up to 64 MiB.
- The model test's rule for "shown elsewhere" changes with the engine's: through every parent, tags and server lines by reach, saves by ID. It is the model that says when a `resolve` replaces the combined save, so its counts move.
- The red team's proof for decision 112 (a second amend reads no other line's saves again) is rewritten for the new promise: an amend of a save Opalro just made reads none at all.
- Threat list: T29's row (two saves of one change after an amend through a second parent) is closed by the rule; the note is one more cache a planted project can carry, with a seal that a copied project does not have.

## Correction (2026-11-15, while it was built)

Building the check showed three things this record had wrong or had not said.

- **Point 4's case cannot be reached through Opalro's commands.** The record said another line could hold an older copy of an amended save, "reached by going to it by its ID and starting a line there". It can't: a save an amend replaced is not found by its ID afterwards (only what a line, a tag, a server line or an undone step still holds is found, [decision 169](169-saves-reached-through-a-combine.md)), so no line can be started on it. Two lines can hold different saves of one change only in a project made by other means. The rule stands as decided (the check asks about the save itself); for a project made with Opalro's commands it changes nothing.
- **One note for the whole project was not enough: the cache keeps one for each line's newest save.** As first built, the note named one save, the last this computer made, on whichever line. A save on another line then replaced it, and an amend of this line found no usable note and asked about every other line, tag and server line. That is the slow case, paid each time someone worked on another line and came back: the very flow decision 112's cache had been built for. The task's review caught it. So the cache holds a note for each save that is some line's newest and that this computer made; an amend finds the note of the save it is amending and asks only about what has moved since that save was made, as the consequences above say. Each note stands by itself: whatever has not moved since its diary entry cannot hold its save, however old the note is. Point 3's "a note" is to be read this way, and `docs/storage-format.md` §10 has the layout.
- **A line, tag or server line that names a save the store does not hold stops the check, with the damage named,** when it has to be asked about, as `opalro line delete` already stops. Opalro cannot tell whether a history it cannot read holds the save. Saving as a new save still works.
