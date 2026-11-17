# Milestone 11 ("Loose ends"): rulings made while the owner was away

On 2026-11-17 the owner wrote: "continue working on the opalro and continue whats missing, features, issues or whatever it is. Im gonna be away so work autonomously". Milestone 11 is the first of v0.3 ([decision 193](../decisions/193-the-next-version-starts-with-what-v0.2-left.md)): the six things the owner's review of v0.2 put first in the next version ([decision 189](../decisions/189-the-owners-review-of-v0.2.md) point 5). This file lists what was decided without the owner, so that each item can be confirmed or reversed. Nothing here is merged into `main`, which stays v0.2 until the owner publishes it.

The decisions are records [193](../decisions/193-the-next-version-starts-with-what-v0.2-left.md) to [196](../decisions/196-empty-folders-in-a-switch-and-in-status.md). The plan is [2026-11-17-milestone-11-loose-ends.md](2026-11-17-milestone-11-loose-ends.md).

## Please confirm

The first three carry a real choice; the rest keep what the owner already asked for, in the way set out.

1. **An import now gives different fingerprints than 0.2 did** ([195](../decisions/195-the-import-records-the-renames-it-is-sure-of.md)). A rename noted in a save is part of the save's fingerprint, so every save from the first commit that renames a file without changing it gets a new one; the change IDs people see stay. Each version imports the same repository the same way every time; projects already imported keep their saves. The other choice was a separate list of renames beside the saves, a second place for one fact and a new thing on disk after format 1 freezes.
2. **It doesn't follow Git's own `README.md`.** The example that started this (23 of 75 lines where `who` and `git blame` disagree) is a rename with 7 lines changed in the same commit, so it isn't the same contents and isn't noted. Only guessing by likeness would follow it, and that stays deferred.
3. **`opalro status` now lists empty folders** ([196](../decisions/196-empty-folders-in-a-switch-and-in-status.md)), added and removed, as rows of their own (`empty folder` where a file's size goes), and `--json` gains `empty_folders`. A folder that holds only ignored files shows as an added empty folder, because that is what a save records. The other choices: show only saved empty folders that are gone, or say nothing and let `save` keep acting silently.
4. **Short names** ([194](../decisions/194-a-change-ids-start-counts-from-four-letters.md)): text counts as a change ID's start from 4 letters, and 1 to 3 letters of k to z that name nothing get a hint saying so (for Jujutsu's habit of typing the one or two letters it highlights).
5. **A switch makes no folder that only conflicts standing as nothing fill, and `resolve --keep` takes out a folder it empties that is gone from disk** (196), as `resolve` and a save of some paths already do. Folders saved empty, and folders that hold only project links or names this computer can't write, are made as before.
6. **The program says "Opalro before 0.2"** where it said "Opalro 0.1" ([193](../decisions/193-the-next-version-starts-with-what-v0.2-left.md) point 2).
7. **The flush after the journal is removed is only tested, not changed** (193 point 1). What the tests show, and a recommendation, are in the follow-ups' "For the owner to decide".
8. **Git's rule for `who` at a combine is only measured,** on a throwaway branch (193 point 1); the numbers are in benchmarks.md.
9. **The work lives on `v0.3`** (193 point 3), made from `main`; `main` stays v0.2 untouched until it is published, and the version stays 0.2.0 on `v0.3` until v0.3 is packaged.

## Every ruling, as made

(Kept as the build goes, with the evidence for each.)
