# 195 — The import records the renames it is sure of

- **Status:** Accepted
- **Date:** 2026-11-17
- **Agenda topic:** none (milestone 11; the owner put it first in the next version, [decision 189](189-the-owners-review-of-v0.2.md) point 5: "it changes what an import writes, so it gets a decision of its own and its comparison with Git")
- **Decided by:** the owner, the rule (a commit that takes a file away from one path and puts the same contents at exactly one new path); Claude, while the owner was away, how it is built and what follows from it. The owner can reverse any part.
- **Refines:** [decision 032](032-exact-repeatable-commit-import.md) (commits become saves exactly and repeatably), [decision 040](040-import-once-catch-up-later.md) (catching up later), [decision 168](168-moves-in-saves-and-in-combines.md) (moves in saves and combines), [decision 170](170-what-building-milestone-8-refined.md), [decision 184](184-opalro-who-the-save-behind-each-line.md) (what `who` follows)

## Context

**In plain words.** Since v0.2, when you move a file and save, Opalro notes the move in the save, so `opalro who` can follow the file's lines back past the move and `opalro combine` can bring changes across it. It notes only moves it is sure of: a file gone from one place and the very same contents arriving at exactly one new place. History brought in from Git has no such notes, because Git doesn't record moves: it guesses them later by how alike two files are. So in an imported project, `who` stops at every rename.

What the research found (`research-B.md`):

- **Move notes are part of a save's fingerprint.** A save's record has had room for them since milestone 1, and `save` has written them since v0.2. Notes in an imported save change its fingerprint and that of every save after it; the change IDs (the k-to-z letters screens show), the `git-commit` each save records and every file stay the same.
- **The import can't reuse the save's comparison as it is.** The folder listings an import makes can't be read back until the import commits, so it needs its own comparison of each commit's Git tree with its first parent's, going down only where Git's tree IDs differ, with every entry taken as the import takes it (names it leaves out, Git LFS files by what they hold, kinds).
- **Git's own `README` doesn't benefit.** It became `README.md` in a commit that also changed 7 of its lines (4ad21f5d, 2016), so it isn't the same contents; Git follows it because Git follows renames with edits up to half the file. Following that is guessing by likeness, which stays deferred (decision 189 point 1).
- **Combines change too.** A combine follows the move notes along each line, so on imported history a combine now carries changes across a rename that Git also carries across: the merge test that pinned the old way (`merge_vs_git.rs`, a file moved on one line and removed on the other) will now stop where Git stops.

## Options

- **A. Exact renames, against the first parent, by the save's own rule,** fed from a comparison of Git trees. One rule for every note Opalro writes.
- **B. Also renames with edits,** by likeness, as Git does. It would follow `README.md`, but it is guessing, which Opalro has kept out (decisions 006 and 157).
- **C. A separate list of renames beside the save,** so that fingerprints don't change. A second place for one fact, and a new thing on disk after format 1 is frozen.

## Decision

**A.**

1. **What is noted.** An imported save notes a move where its commit, compared with its first parent, has a file gone from one path and a file with the same contents arriving at exactly one new path, by the rule a save uses (decision 168, `moves.rs`): files with contents only, never a link, a project link or an empty file; contents that leave or arrive at two places are no move; an executable flag may change with the move; both paths must pass the name rules. A folder moved whole gives one note for each file in it. The first commit has no parent and notes nothing. A merge commit notes against its first parent, as a combined save does, so it records the moves the other line brought in.
2. **Compared as the import sees it.** Contents are compared as Opalro stores them, so a Git LFS file counts by what it holds. A name the import leaves out (decision 035), and everything under it, is in no note. A rename that changes only letter case or Unicode form is a move, as in a save.
3. **One rule in one place.** The pairing in `moves.rs` takes its changes from either source: the save's comparison of two folder listings, or the import's comparison of two Git trees. Its limits stay (100,000 notes, 16 MiB of paths), and every limit is counted the same way on every computer, so that importing a repository anywhere gives the same saves.
4. **What it means for fingerprints.** Each version of Opalro imports a repository the same way every time, anywhere; this one gives different fingerprints than 0.2 from the first commit that renames a file without changing it. A project already imported keeps its saves: nothing rewrites history, so its `who` follows renames only after the repository is imported again. Catching up with Git later (decision 040) matches commits by the `git-commit` each save records, never by working out the saves again.
5. **Proved against Git.** The tests compare every note with Git's exact renames (`git diff -M100%` between a commit and its first parent), leaving out contents that more than one file holds on either side, where Git picks one pairing and Opalro notes none. The merges compared with Git (`merge_vs_git.rs`) and the lines compared with `git blame` (`who_vs_git.rs`) now include renames in imported histories. The time an import of Git's own repository takes is measured before and after and written into [benchmarks.md](../benchmarks.md).
6. **Nothing new in `--json` or on the screen:** an import doesn't count the renames it noted; `history` and `show` say nothing new.

## Consequences

- `who` follows a file through renames in imported history that kept its contents; where Git followed a rename with edits, `who` still stops there, and the guide says so.
- A combine on imported history follows those renames too, as it already did on history Opalro made.
- The deferred "What `who` doesn't follow" in [design.md](../design.md) §7 keeps only the renames with edits; §5.16 says what an import notes; [storage-format.md](../storage-format.md) §4's "gives the same saves" says "by the same version"; [testing.md](../testing.md) and [guide.md](../guide.md) follow. The release notes of v0.2 stay as they are: they are true of 0.2.
- Decisions 168, 170 and 184 link here.
