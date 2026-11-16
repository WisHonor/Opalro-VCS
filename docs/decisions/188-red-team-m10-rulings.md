# 188 — Red team on milestone 10: two sessions, one Low finding, ruled and left for the owner

- **Status:** Accepted
- **Date:** 2026-11-16
- **Agenda topic:** none (milestone 10, its red-team sessions RT23 and RT24)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 176](176-a-conflict-in-the-folder.md) (what the folder shows for a place in conflict); [decision 184](184-opalro-who-the-save-behind-each-line.md), [185](185-save-amend-and-a-save-something-else-holds.md) and [186](186-speed-before-v0.2.md) (the code the two sessions attacked)

## Context

Milestone 10 was attacked twice from the no-network sandbox ([decision 024](024-red-team-gate.md)).

**The first session** (RT23, [report](../redteam/2026-11-16-milestone-10.md)) attacked `milestone-10` at 23bb160. It was cut in two by a pause and spent most of its time reading. It proved nothing. What it ran held: `opalro who` in a project where a `goto` was stopped at each of its 16 fault points changed nothing under `.opalro` or in the folder; a random model of saves, switches and combines named, for each of 294 lines, a save that holds the line; a named pipe at the path was refused at once; a file of a million lines took 5.3 seconds and 130 MiB. It left three suspicions without proof:

- in a project that cannot be written, `who` said only `error: Permission denied (os error 13)`, with no path and no hint;
- a history planted to keep `who` busy could keep it busy for hours (not built);
- on a drive that ignores letter case, a save made elsewhere can hold two spellings of one name, and `who` then follows one of them (needs Windows or macOS).

Between the sessions the whole-branch review's fix wave changed engine code: `who` holds a moved file's old path once; a refused amend reads each save once to name its holders; what `who` remembers between comparisons is keyed by contents and claimed sizes; the folder walk and the switch ask how the drive compares names through one function. The first suspicion above was fixed in that wave: the error now names the folder and says why a file is needed there.

**The second session** (RT24, [report](../redteam/2026-11-16-milestone-10-session-2.md)) attacked `milestone-10` at 9775dec, almost wholly by experiment. What held:

- **The amend's holders:** in 150 random planted histories (saves with one to three parents, times equal, negative or random; lines, tags and server lines on random saves) `opalro save --amend` was refused exactly when a brute-force search found a holder, and named exactly those holders. A holder whose history has a save the store lacks stopped it, with the damage named.
- **`who` and moved files:** a random model with edits, moves, two files swapping names, folder moves and combines: for each of 938 lines the named save holds the line at the named path, with the usual deadline and with none.
- **The switch and the file cache:** with the cache missing, cut in half, with a broken seal, rewritten, or replaced by an older one, `goto` and `combine` refused to overwrite unsaved work every time.
- **`combine` and `resolve` at the same time:** in 16 rounds one waited for the other, `verify` passed and the fix survived.
- **Three common starting points:** the same result in every order of their IDs.
- **A project whose `.opalro/tmp` cannot be written:** 13 commands each stopped with exit code 1 and changed nothing.
- **The steps of the page for trying 0.2:** every line the page quotes was what the program printed.

**One finding, RT24-1 (Low).** `main` moves the folder `d` away (to `e`) and changes a file of it in the same save; `side` changed `d/x.txt`. `opalro combine side` keeps a conflict at `d/x.txt` (changed on `side`, removed on `main`; nothing is in the folder for it) and says "No file needed changing.", and yet an empty folder `d` is now in the project. `opalro resolve --keep mine d/x.txt` keeps the removal and leaves the empty `d`. `opalro status` does not mention it. Nothing is lost. The proof is `a_combine_that_writes_nothing_makes_no_folder_a_moved_away_conflict_sits_in` in `crates/opal-vcs/tests/redteam24.rs`.

Why it happens: the combined save's listing holds the folder `d`, because the conflict's record sits in it. Read as it stands, that folder holds nothing (the conflict stands as nothing), so the switch writes what it reads: an empty folder.

Neither session reached: a `cache/last-made` note with a good seal copied from another project of the same user (reasoned through, not planted); a history that needs more than 256 remembered comparisons in `who` (not built); the letter-case shapes on Windows and macOS (no platform tests).

## Options

For RT24-1:

- **A. The switch makes a folder only when something is written into it,** as a write-out limited to some paths has done since this milestone. But a save can hold an empty folder on purpose, and a switch must make that one. The rule would have to tell a folder that is empty from a folder that only carries conflicts with nothing in the folder for them. It changes the switch every command uses, after both sessions.
- **B. A `resolve` that keeps a removal removes the folder if it is empty.** It would also remove an empty folder the person made there themselves in the meantime.
- **C. Leave it in v0.2, say so, and put the choice to the owner.** The folder made is what the save's listing holds as it stands; nothing is lost; deleting the folder by hand is harmless.

## Decision

**C.**

**1. RT24-1 is not changed in v0.2.** The proof stays in `redteam24.rs`, ignored, with the finding named in its reason. The known problems of the release say that an empty folder can appear and that deleting it is safe. Whether the switch should make that folder is on the list of things for the owner to decide, with options A and B.

**2. RT23's first suspicion is fixed** (in the fix wave, before the second session, which attacked it): a project whose `.opalro/tmp` cannot be written gets an error that names the folder and a hint that says why Opalro makes a file there.

**3. The other two suspicions and what neither session reached go to the follow-ups list** for the next red team. A history planted to keep `who` busy stays accepted: it writes nothing and can be stopped ([decision 184](184-opalro-who-the-save-behind-each-line.md)).

**4. No third session.** After the second session the only change to code is the `#[ignore]` on its proof.

## Consequences

- With this record every finding of both sessions is fixed or ruled on in writing, which is what [decision 024](024-red-team-gate.md) asks before a milestone is merged.
- v0.2 can leave an empty folder after a combine in the shape above. It is the third time in two milestones that an empty folder is where a person would not expect one (the first two were a `resolve --keep` that made a deleted folder again, fixed, and `status` not showing a saved empty folder that is gone, on the follow-ups list). The three share one cause: Opalro keeps empty folders in saves, and `status` shows files only. That question is the owner's.
- The second session's experiments are kept uncompiled in `redteam/attacks23/`, with the script that runs the try steps of the release's page.
