# 193 — The next version starts with what v0.2 left: milestone 11

- **Status:** Accepted
- **Date:** 2026-11-17
- **Agenda topic:** none (planning the version after v0.2)
- **Decided by:** Claude, at the owner's request while the owner was away ("continue working on opalro ... work autonomously"). The owner can reverse any part.
- **Refines:** [decision 189](189-the-owners-review-of-v0.2.md) point 5 and 6 (what comes first in the next version); [decision 157](157-v0.2-in-four-milestones.md) (where a version's work lives while the one before it isn't published)

## Context

**In plain words.** v0.2 is finished and sits in `main`, waiting for the owner to try it and publish it. The owner's review of v0.2 put six things first in the next version: four small changes to what Opalro does, one set of crash tests, and one measurement. The roadmap gives the next version, v0.3, its headline: working together through a server, with file locks ([decision 001](001-first-audience-and-v0.1.md), [decision 015](015-sync.md)). That headline needs a design pass of its own first, and some of its questions only the owner can answer: where a server runs and who pays for it, and what Opalro may send over the network, which today is nothing ([decision 142](142-bolt-sends-nothing-and-each-build-proves-it.md)) and which the user agreement would have to say.

The six things from the review (the follow-ups list, "First in the next version"):

1. **Short names:** text counts as the start of a change ID only from four letters on.
2. **The import and renames:** `opalro import` records a rename it is sure of, so that `opalro who` follows it.
3. **Empty folders:** a switch stops making a folder that holds only a conflict with nothing in it, and `opalro status` shows a saved empty folder that is gone.
4. **The flush after the journal is removed:** put to the crash tests before anything about it is proposed.
5. **`who` at a combine:** measure what Git's rule would save in time.
6. **What the program says about older versions:** two messages name "Opalro 0.1".

## Options

- **A. Milestone 11 takes the six, as the first milestone of v0.3, on a branch of its own.** The rest of v0.3 is planned once its design pass is done.
- **B. A fix release, 0.2.1, from `main`.** Most of the six change what Opalro does or what an import writes; they aren't fixes, and the owner said "the next version". It would also change `main` under a v0.2 the owner hasn't tried yet.
- **C. Start with the server.** The headline comes sooner, but the owner's questions stand in its way, and the review asked for these six first.

## Decision

**A.**

1. **Milestone 11, "Loose ends", is the six items above.** Items 1 to 3 and 6 change what Opalro does, each by a decision of its own where the follow-ups list asks for one ([194](194-a-change-ids-start-counts-from-four-letters.md), [195](195-the-import-records-the-renames-it-is-sure-of.md), [196](196-empty-folders-in-a-switch-and-in-status.md)). Items 4 and 5 change nothing Opalro does: they produce tests and numbers, written up for the owner. **No flush changes** in this milestone, whatever the crash tests show ([decision 189](189-the-owners-review-of-v0.2.md) point 3).
2. **The words about older versions** (item 6): each feature a project can turn on is named with the version it came with. Today there is one, `conflicts`, which came with 0.2, so the two messages say "Opalro before 0.2" where they said "Opalro 0.1". Only a project made by a build from before 0.2 ever shows them.
3. **Where the work lives.** On the branch `v0.3`, made from `main` as it is now (v0.2). Milestone 11 is built on `milestone-11` and merged into `v0.3` after its red-team session and its whole-branch review. `main` stays v0.2, untouched, until the owner publishes it, so that what the owner tries is what gets published, and a fix to v0.2 can still be made there and merged into `v0.3`. `v0.3` reaches `main` only after v0.2 is published, through a pull request whose checks pass.
4. **Format 1 stays frozen as v0.2 left it.** Nothing in milestone 11 changes how a record is written: a save's record has had room for move notes since milestone 1 and saves have written them since v0.2, so an import that writes them changes what it writes, not how. `opalro_json` stays `1`: fields are added, none is changed.
5. **The version stays 0.2.0 on `v0.3`** until v0.3 is packaged, as `v0.2` kept 0.1.0 until milestone 10.
6. **The rest of v0.3 waits for its design pass.** Sync, locks, on-demand big files and `reverse` (decisions 001, 015, 017 and 021) are planned after it. The owner's questions in it come first.

## Consequences

- The owner comes back to decisions 193 to 196 and milestone 11's rulings file, whose "please confirm" list starts with the few items that carry a real choice.
- [milestones.md](../milestones.md) gains a v0.3 section with milestone 11's steps; [design.md](../design.md)'s roadmap names milestone 11; the follow-ups list marks the six as taken.
- Two Dependabot pull requests (#39 and #40) were left out of v0.2 ([decision 192](192-the-owner-has-claude-merge-v0.2-into-main.md)). They are taken on `v0.3`, each library on its own with the tests, outside milestone 11's tasks.
