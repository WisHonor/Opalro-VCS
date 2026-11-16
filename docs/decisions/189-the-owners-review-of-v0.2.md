# 189 — The owner's review of v0.2: the 71 things to confirm

- **Status:** Accepted
- **Date:** 2026-11-16
- **Agenda topic:** none (the owner's review of what was decided while they were away, milestones 7 to 10)
- **Decided by:** the owner. They confirmed the first item in person, and for the other 70 said to go with Claude's recommendations. The recommendations are Claude's and are written out here, so that each can still be reversed.
- **Refines:** [decision 156](156-the-user-agreements-blanks-filled-by-the-owner.md) (one sentence of the agreement); [157](157-v0.2-in-four-milestones.md); [186](186-speed-before-v0.2.md) points 3 and 4; [187](187-v0.2s-package.md) points 4 and 6; [188](188-red-team-m10-rulings.md)

## Context

Milestones 7 to 10, which are v0.2, were built while the owner was away. Every choice the owner had not made themselves was written down for them: each milestone's rulings file has a "Please confirm" list ([7](../plans/2026-11-14-milestone-7-rulings.md#please-confirm): 12 items; [8](../plans/2026-11-14-milestone-8-rulings.md#please-confirm): 21; [9](../plans/2026-11-14-milestone-9-rulings.md#please-confirm): 21; [10](../plans/2026-11-15-milestone-10-rulings.md#please-confirm): 17), 71 in all.

On 2026-11-16 the owner came back and began to go through them one by one. The first, milestone 7's item 1 (what v0.2 is, in four milestones on a branch of its own, and what it leaves out), they confirmed as decided. For the rest they asked that Claude's recommendation be taken.

## Options

For each item: keep what was decided; change it now, before v0.2 is published; or keep it for v0.2 and change it first in the next version.

## Decision

**1. Every item is kept as its decision record has it,** except where the points below say more. That is also so where an item named another choice and asked whether it was wanted: `opalro resolve` puts a fix into the combined save when it can (milestone 9's item 5); `opalro setup --upgrade` stays a step of its own, since Opalro 0.1 cannot open the project afterwards and a door that only opens one way should be opened on purpose (item 8); `status` reads the files that still have markers each time (item 14); `resolve --copy` is not guarded against a second route to the same disk, which would need new operating-system calls (item 18); guessing moves by likeness stays deferred (milestone 8's item 13).

**2. The user agreement's sentence names 0.2** (milestone 10's item 10). "(nothing: Opalro 0.1 is free)" becomes "(nothing: Opalro 0.2 is free)". It is the owner's own sentence from decision 156 with only the number moved: it says what is true of the version the agreement comes with, and promises nothing about a later one. It moves again with each version that is free.

**3. All ten flushes stay** (milestone 10's item 8; decision 186 points 3 and 4). Nothing about what is flushed, or when, changes. v0.2 is published with `goto` over twice Git's time on Linux, said plainly in its notes, as v0.1 was. The flushes are what makes a finished command survive a crash, they are about a tenth of a `goto` on Linux, and the one that looked as if it could go (the flush after the journal is removed, a quarter of a millisecond) rests on a reading of the code that no test has checked.

**4. The limit for `status` stays at twice Git's time and the speed check stays as it is** (milestone 7's item 11, milestone 8's item 16). It is not judged over several runs and not tied to one version of Git. The nightly run can fail on the processor where Git is quickest.

**5. Kept for v0.2, and changed first in the next version:**

- **Short names** (milestone 7's item 9). Text counts as the start of a change ID only from four letters on, the shortest Opalro ever prints, so that a line or tag called `x`, `try` or `tmp` always means the line or tag. It refines [decision 068](068-a-goto-target-that-names-both.md) when it is built.
- **The import records the renames Git's history shows** (milestone 10's item 15), where a commit takes a file away from one path and puts the same contents at exactly one new path: the certainty a save asks for ([decision 168](168-moves-in-saves-and-in-combines.md)). Then `opalro who` follows a renamed file in imported history. It changes what an import writes, so it gets a decision of its own and its comparison with Git.
- **Empty folders** (milestone 10's item 17, [decision 188](188-red-team-m10-rulings.md)): a switch stops making a folder that holds only conflicts with nothing in the folder for them, and `opalro status` shows a saved empty folder that is gone. One decision for both, since they share a cause.
- **The flush after the journal is removed** is put to the crash tests before anything about it is proposed.

**6. Kept, with the other choice written down and not taken:** `opalro who` asks the first parent first also where a combine took the other line's version of a file whole (milestone 10's item 16); what Git's rule would save in time is measured in the next version. `opalro resolve` stops where a line, tag or server line names a save the store does not hold.

**7. The two Low findings that v0.1's draft also has are not carried into v0.1** (milestone 7's item 12). They are fixed in v0.2.

## Consequences

- `USER-AGREEMENT.md` changes by one number. The release workflow is run again on `v0.2`, so that the archives the owner tries carry the agreement as it now reads.
- Each of the four lists says at its top that it was reviewed, and what the review came to.
- In `docs/releases/v0.2.0-before-you-publish.md`, parts 1, 3 and 4 are done. Left for the owner: trying it (part 2); whether v0.1 is published first or v0.2 replaces it (part 5), where Claude's recommendation is to publish 0.2 and never publish 0.1's draft, rather than rebuild 0.1 for two Low findings; publishing (parts 6 to 9); and the one question that is in no list, whether v0.2 needs papers about its code as v0.1 has.
- The follow-ups list gains "First in the next version".
- Not part of this review: the older lists of milestones 5 and 6, which belong to v0.1.
