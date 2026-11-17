# 192 — The owner has Claude merge v0.2 into `main`

- **Status:** Accepted
- **Date:** 2026-11-16
- **Agenda topic:** none (v0.2's road to being published)
- **Decided by:** the owner ("merge everything yourself now"). What "everything" takes in is Claude's reading, set out below; the owner can change any of it.
- **Supersedes in part:** [decision 157](157-v0.2-in-four-milestones.md) and [decision 187](187-v0.2s-package.md) ("`v0.2` goes into `main` only by the owner's hand") and [decision 190](190-v0.2-is-the-first-published-version.md) ("when the owner merges `v0.2` into it"): the owner hands this merge to Claude

## Context

**In plain words.** v0.2 was built on a branch of its own while the owner was away, and every record kept one step for the owner: putting it into `main`, the branch releases are made from. On 2026-11-16 the owner asked whether all the branches were merged into `main`. Twelve were not. Shown the list, the owner answered: "merge everything yourself now".

Two things stand between those words and `main`:

- **`main` has rules of its own.** Only a pull request gets in, and only once the tests have passed on Windows, macOS and Linux and `deny` has passed. Nobody is let past them, the owner included.
- **Not all twelve belong there.** One is `v0.2` itself, and five are already inside it. Of the other six, some say things that aren't true, some hold code made to be thrown away, and two don't pass their tests.

## Options

- **A. Merge all twelve,** as the words say. `main` would say that v0.1 and v0.2 are published, carry timing code nobody is meant to get, and take library updates whose tests fail, which the rules refuse anyway.
- **B. Merge `v0.2`,** which carries the five work branches inside it, and leave out the rest, saying why for each.
- **C. Keep the merge as the owner's step** and explain it again. The owner has just said who does it.

## Decision

**B.**

1. **`v0.2` goes into `main`** through a pull request that Claude opens and merges, with a merge commit, once every required check has passed on its last commit. The rules on `main` are neither changed nor got around.
2. **Already inside `v0.2`,** so with nothing of their own to merge: `milestone-7`, `milestone-8`, `milestone-9`, `milestone-10` and `owner-review-v0.2`.
3. **Left out:**
   - **`v0.2-published`** says that v0.2.0 is published and that format 1 is frozen. It goes in once that is true, as the owner's page has it (part 11).
   - **`v0.1-published`** says that v0.1 is published. It never was ([decision 190](190-v0.2-is-the-first-published-version.md)), so this branch is never merged.
   - **`gate/m10-timers` and `gate/m10-who-diff`** hold measuring code, written to be thrown away ([decision 186](186-speed-before-v0.2.md)).
   - **The two pull requests that update libraries** (#39 and #40, opened by GitHub's Dependabot on 2026-11-16). Each updates five libraries, two of them to a version that breaks compatibility: `sha2`, which reads Git's fingerprints on an import, and `similar`, which compares text. Their tests fail on all three systems. They are work of their own, after v0.2.
4. **What stays the owner's:** trying v0.2, starting the release workflow from `main`, publishing the draft, the website going live, and making the repository private. A merge publishes nothing: no tag and no draft release come of it.

## Consequences

- `main` goes from v0.1's code straight to v0.2's, as decision 190 said it would. The next release workflow started from `main` builds 0.2.0.
- The owner's page put trying the program (part 2) before the merge. Merging first loses nothing: what is found while trying is fixed on a branch and reaches `main` the same way, before anything is published.
- v0.2 changes no file of the platform crate, the only place `unsafe` code may be, so there is none for the owner to read line by line before this merge. The owner's review of v0.2 is [decision 189](189-the-owners-review-of-v0.2.md)'s.
- The pages that said the merge waits for the owner say who did it: `CLAUDE.md`, [design.md](../design.md), [milestones.md](../milestones.md) and the owner's page (part 7).
- Dependabot's two pull requests stay open. Closing them, or taking the updates one library at a time with the tests, is for after v0.2.
