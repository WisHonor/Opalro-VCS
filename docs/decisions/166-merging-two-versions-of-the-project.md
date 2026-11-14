# 166 — Merging two versions of the project, file by file

- **Status:** Accepted
- **Date:** 2026-11-14
- **Agenda topic:** none (milestone 8 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 012](012-lines.md) (per-file three-way merge from the common starting point; binary files never merged automatically); [decision 004](004-project-contents.md) (what a project can contain, and the names two systems can't tell apart); [decision 007](007-conflicts.md) (what counts as a conflict)

## Context

**In plain words.** A combine has three versions of the project in hand: the starting point, yours (the line you are on) and theirs (the line being combined in). For every path it asks the same question: who changed this since the starting point? If only one side did, that side's version is taken. If both did the same thing, that is taken once. Only when both changed it differently is there something to work out.

Decision 012 fixed that shape. This record settles the cases it left open: a file's executable mark, folders against deletions, and names that can't live in one folder.

## Options

**The executable mark and the contents of one file**

- **A. Two separate facts** (Git merges a file's mode and its contents apart). One side marks a script executable, the other edits it: the result is the edited script, executable.
- **B. One fact.** The same case would be a conflict, though nothing really disagrees.

**A folder deleted on one side and changed on the other**

- **A. Look inside:** each file of the folder is judged by itself (Git does this, having no folders of its own). Files the other side never touched go; only the files it changed are conflicts.
- **B. The whole folder is one conflict.** Simple, and a large one: deleting a folder of 500 files while someone edits one of them would put all 500 in question.

**Two names one folder can't hold on Windows or macOS** (`Readme.md` from one side, `README.md` from the other)

- **A. Refuse the combine,** naming both. A save made from the folder refuses such a pair already (decision 004).
- **B. Take both,** as an import does for a Git history that holds them. Then the folder can't show the save it was just given.

## Decision

**A, A, and A.**

For each path, with the starting point's entry, yours and theirs (any of which may be missing):

- **Yours and theirs are equal:** that is the result, whether both changed it alike or neither did.
- **Only one side differs from the starting point:** that side's entry is the result. A removal is a change like any other, and so is a new file.
- **Both differ from the starting point and from each other:**
  - **Two folders:** their entries are merged the same way, one level down, from the starting point's folder when it has one, else from an empty one. Two folders that merge to nothing stay, as an empty folder: neither side removed the folder.
  - **A folder against a removal,** where the starting point has the folder: the side that removed it counts as an empty folder, so each file inside is judged by itself. If nothing is left, the folder is gone.
  - **Two files, where the starting point has a file too:** the executable mark and the contents are merged apart. Each takes the one side's change when only one side changed it. Contents both sides changed are merged as text when they are text ([decision 167](167-merging-text-line-by-line.md)); otherwise they conflict.
  - **Anything else is a conflict:** a file changed on one side and removed on the other; two new files at one path that differ in anything, the executable mark included; a file against a folder; a link or a project link changed two ways. Links and project links are never merged, only compared.
- **What isn't looked at.** A folder whose listing has the same ID on two sides is taken whole, unread. A combine reads only the folders that differ.
- **Names one folder can't hold.** After a folder is merged, a pair of names the save-time rules refuse (alike but for letter case, or for how an accented letter is written), where each side brought one of the two, stops the combine, naming both paths. A pair that one side already held (an imported history can) is left as it is.
- **The result must be a save Opalro would make:** within the limits of storage-format.md §3. Two honest sides can exceed only the count of entries, and then the combine stops and says so. Checking this reads every folder listing of the result once, on every combine.

## Consequences

- The result is built from stored listings alone; the project folder is read only afterwards, to switch to it (decision 165).
- In milestone 8 every conflict stops the combine, and its message says what happened on each side ("changed on main, removed on wall-run"). Milestone 9 turns each into a conflict kept in the save.
- A file moved on one side is first paired with where it went ([decision 168](168-moves-in-saves-and-in-combines.md)), so that a change to it on the other side lands on the moved file instead of becoming "changed here, removed there".
- The model test grows a combine of its own, with these rules written a second time, independently.
