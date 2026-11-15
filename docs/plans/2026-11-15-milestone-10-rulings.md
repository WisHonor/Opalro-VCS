# Milestone 10 (`who`, and v0.2): rulings made while the owner was away

On 2026-11-13 the owner said v0.1 only waits for them to publish, asked for v0.2 to be started, and said: "I'll be away, so work on your own." Milestone 10 is the last of v0.2's four ([decision 157](../decisions/157-v0.2-in-four-milestones.md)). This file lists what was decided without the owner, so that each item can be confirmed or reversed. Nothing here is merged into `main`.

The decisions themselves are records [184](../decisions/184-opalro-who-the-save-behind-each-line.md) to [187](../decisions/187-v0.2s-package.md). Each says "Decided by Claude, at the owner's request while the owner was away. The owner can reverse any part." The plan is [2026-11-15-milestone-10-who-and-v0.2.md](2026-11-15-milestone-10-who-and-v0.2.md).

## Please confirm

1. **`opalro who <file>` reads the file as it is in your folder** ([184](../decisions/184-opalro-who-the-save-behind-each-line.md)), as `git blame` does, and marks the lines you have not saved yet. `--save <save>` reads a saved version instead. The other choice was the saved version only, as Mercurial does.
2. **A change of line endings alone does not count as changing a line** (184). A save that only turned `\r\n` into `\n` takes no line in `who`. Everything else counts, spaces too; Git counts line endings unless asked not to.
3. **`who` follows a moved file only where a save recorded the move** (184), never by likeness. One thing it cannot follow: a file moved on one line and changed, under its old name, on a line that was later combined in; the lines of those changes are named as the combine's. Nothing a save stores says where a second parent had the file.
4. **In a folder that holds a conflict's marked text, `who` names the save behind each side's lines,** and the marker lines as the combine's (184). It needed no rule of its own: a place in conflict is read as what stands there.
5. **`opalro save --amend` is refused when anything else has the save in its history, through any parent** ([185](../decisions/185-save-amend-and-a-save-something-else-holds.md)): another line that combined yours in, a tag further on, a server line. Until now only first parents were followed, and a tag or server line counted only when it named the save itself. `opalro resolve` uses the same rule to decide whether a fix replaces the combined save.
6. **Another save of the same change no longer stops an amend** (185). The old check also refused when another line held an older copy of the save you are amending (reached by going to it by its ID and starting a line there). Two saves of one change were visible before the amend and still are after it, so the refusal changed nothing.
7. **A small note replaces the cache the amend check kept** (185): the save Opalro made last and the diary entry that recorded it, in the project's cache folder. With it, an amend right after a save reads nothing of the other lines. `cache/other-lines`, which Opalro 0.1 wrote, is no longer read or written.
8. **Speed: nothing that is flushed changes** ([186](../decisions/186-speed-before-v0.2.md)). Going back (`goto`) was to come within twice Git's time in v0.2. The work done twice is taken out; the flushes, which cost more, are yours to rule on, with a table of what each costs. If `goto` is still over twice Git's time, v0.2 is packaged with that said plainly, as v0.1 was.
9. **v0.2 is packaged up to the draft and no further** ([187](../decisions/187-v0.2s-package.md)): the version is 0.2.0 on the branch `v0.2`, the release workflow has run there (every job but the draft), and nothing is tagged, drafted, merged into `main` or opened as a pull request.
10. **`USER-AGREEMENT.md` was not touched** (187). One sentence in it speaks of the version: liability is limited to what you paid, "nothing: Opalro 0.1 is free". Every archive carries the agreement, so that sentence needs your word before v0.2 is published.

## Every ruling, as made

**Before the build** (2026-11-15, from the research of the code in `milestone-9`)

- **R1. `who` works on what stands at a path,** the file itself or, at a place in conflict, what the conflict stands as ([decision 174](../decisions/174-the-conflict-record.md)'s one rule), read through the reader every other command uses. No other handling of conflicts in `who`.
- **R2. Lines are compared without their line ending,** and by nothing else: no option to leave spaces out in this milestone.
- **R3. Every parent is followed, the first one first:** a line both parents have goes to the first parent's side.
- **R4. Versions wait on a heap, the newest save first, and lines that reach one version two ways are taken together;** the answer never depends on the order, so clocks that lie only cost time.
- **R5. No limit on how far back `who` reads.** Each comparison has `changes`' deadline; past it `who` says the answer may name a newer save than the first that had a line. The command writes nothing and can be stopped.
- **R6. The amend check asks the ancestry module** ([decision 160](../decisions/160-whether-a-save-is-in-anothers-history.md)), with one new question (which of these saves holds this one), asked again without each holder found, so that the refusal can name them.
- **R7. What is asked of:** every other line, tag and server line that is new or has moved since the diary entry the note names; all of them when there is no usable note. A line, tag or server line that has not moved since the save was made pointed at a save that already existed, and a save that existed first cannot have a later one in its history.
- **R8. The note is trusted only for which of them to ask about,** under the user's seal, for the save it names, with a diary entry that has the line at that save.
- **R9. The release workflow is started from the branch `v0.2`,** where its last job, the draft, does not run by the workflow's own rule.

**While building**

(kept as the build goes)

## Deferred

(kept as the build goes; each is a row in the [follow-ups](2026-11-01-milestone-1-follow-ups.md), under "Deferred by milestone 10")
