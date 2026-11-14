# 164 — Where two lines parted: the common starting points

- **Status:** Accepted
- **Date:** 2026-11-14
- **Agenda topic:** none (milestone 8 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 012](012-lines.md) (combining is a three-way merge from the common starting point, and several common starting points are recorded through decision 007's record); [decision 160](160-whether-a-save-is-in-anothers-history.md) (the walk)

## Context

**In plain words.** To combine two lines, Opalro compares each with the save where they parted: what one line changed since then is kept unless the other changed the same thing. So the first job of a combine is finding that save, the common starting point.

Usually there is exactly one. There can be none (two histories that never met, such as a Git repository's `gh-pages` branch). And there can be several: when two lines were each combined into the other at about the same time, both of those combines' parents are "the last save both sides have", and neither is older than the other. Git calls this a criss-cross.

## Options

For several starting points:

- **A. Pick one** (Mercurial picks one and says which). Simple, but it can lose work without a word. Say a file was reverted on one line after both lines had taken the change: with the wrong starting point, the revert looks like "no change" and the other line's copy of the change wins.
- **B. Combine the starting points first,** and use the result as the starting point (Git's recursive and ort strategies; Jujutsu does the same). Right in the case above. It needs the combine to be able to run on the starting points themselves, which have starting points of their own.
- **C. Refuse.** Safe, but two people who each took the other's work can then never combine again.

## Decision

**B.**

- **A common starting point** of two saves is a save both have in their history that isn't in the history of another such save.
- **How they are found.** A walk like decision 160's goes back from both saves at once, newest first. It marks each save as reached from one side, from the other, or from both, and marks everything under a save that both sides reach. The candidates are the saves reached from both sides that aren't under another such save.
  - When no save is dated before its parent, the candidates are the answer.
  - When clocks ran backwards there can be extra candidates. Each candidate that another candidate has in its history is dropped, by decision 160's test, which is exact whatever the clocks say. So the answer is exact too.
  - The walk stops soon after the histories meet; like decision 160's, it reads further back only when one side has a long line of its own that parted early.
- **One:** it is the starting point.
- **None:** the combine is refused. Two lines that share no save have nothing to be combined from, and doing it anyway is nearly always a mistake. There is no flag to force it in v0.2.
- **Several:** they are combined with each other first, two at a time, in the order of their IDs so the result never depends on chance, each pair from its own common starting points. The result is a folder listing that exists only for this combine. It is stored with the combined save, named by no save, and `opalro tidy` frees it later.
  - One side of such an inner combine can be a made-up listing with several saves behind it, so the walk takes a set of saves on each side.
  - **A budget.** More than 16 candidates at any level, or more than 64 inner combines for one `opalro combine`, is refused: no honest history has them, and each costs a combine. With clocks that ran backwards, more than 16 candidates can be refused though fewer are true starting points; that is safe, never wrong.
  - What inner combines merge line by line is held in memory until the combine ends, up to 256 MiB; past that the combine is refused.
  - In milestone 8, starting points that conflict with each other stop the combine, which says so. Milestone 9 lets such a conflict pass into the result through the conflict record's list of sides, as decision 012 planned.
- **A combine that is refused stores nothing.** Whatever it built is thrown away.

## Consequences

- A combine reads the saves the two lines don't share, and a little more: the walk stops where the histories meet.
- The made-up starting listing is never shown and never a parent of anything. Only its effect is visible: fewer false conflicts, and no change lost.
- The differential tests compare Opalro's starting points with `git merge-base --all` on every imported repository.
