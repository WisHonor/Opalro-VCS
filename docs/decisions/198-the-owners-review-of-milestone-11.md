# 198 — The owner's review of milestone 11: the seven questions

- **Status:** Accepted
- **Date:** 2026-11-18
- **Agenda topic:** none (the owner's review of what was decided while they were away, milestone 11)
- **Decided by:** the owner. Claude's report on milestone 11 put five questions first, and the owner said to "go with your recommendations on all five". Asked about two more that carry a recommendation, they chose to go with those too. The recommendations are Claude's and are written out here, so that each can still be reversed.
- **Refines:** [decision 189](189-the-owners-review-of-v0.2.md) points 3, 5 and 6 (the flush after the journal is removed; Git's rule for `who` at a combine)
- **Refined by:** none yet

## Context

Milestone 11 was built while the owner was away ([decisions 193](193-the-next-version-starts-with-what-v0.2-left.md) to [197](197-red-team-m11-rulings.md)). Its [rulings file](../plans/2026-11-17-milestone-11-rulings.md#please-confirm) lists ten things to confirm, four of them with a real choice. The follow-ups' "For the owner to decide" sets out the questions behind items 4, 8 and 9, and one more: `similar` 3, the library behind Opalro's line diffs, which Dependabot asks for.

On 2026-11-18 Claude's report put the four real choices and `similar` first. The owner said to go with Claude's recommendations on all five. Asked whether the two other questions with a recommendation (items 8 and 9) should go the same way, the owner said yes.

## Options

For each question: keep what was built, or what Claude recommended; or take one of the other ways written beside it.

## Decision

**1. An import's fingerprints change, as built** (item 1; [decision 195](195-the-import-records-the-renames-it-is-sure-of.md)). A rename the import notes is part of the save and of its fingerprint. From the first commit that renames a file without changing it, every save gets another fingerprint than 0.2's import gave it; the change IDs people see stay. Each version imports the same repository the same way every time, and projects already imported keep their saves. Not taken: a list of renames kept beside the saves, a second place for one fact.

**2. Git's own `README.md` stays unfollowed** (item 2). It was renamed with 7 lines changed in the same commit. Following it means guessing by likeness, which stays deferred ([decision 189](189-the-owners-review-of-v0.2.md) point 1).

**3. `opalro status` lists empty folders, added and removed, as rows of their own** (item 3; [decision 196](196-empty-folders-in-a-switch-and-in-status.md)). Not taken: showing only saved empty folders that are gone, or showing none.

**4. Past the comparison's allowance an import stops, committing nothing** (item 4; decisions 195 and [197](197-red-team-m11-rulings.md)). The allowance is the larger of 1 GiB of work and 32 times what converting the Git trees cost; Git's own history uses about a sixteenth of it. Not taken: going on without noting moves, which would make the saves depend on how Git packed the repository; or two budgets, which is more rule, worth it only if honest histories that stop turn up.

**5. `similar` stays at 2.7.0.** It moves when there is a reason, such as a fix Opalro needs or `insta`, the snapshot tests' library, moving to 3. Opalro then calls `myers::diff_deadline_raw`, which kept 2.7.0's results on everything tried. Not taken now: 3.2 with that call, which isn't proven beyond what was tried, may end a diff near its 2-second deadline differently, and would leave 2.7.0 in the lock beside it for `insta`; or 3.2's new default, which changes what `changes`, `combine` and `who` find and would need a decision of its own.

**6. The flush after the journal is removed stays** (item 8; decision 189 point 5). Milestone 11's crash tests found no fault: every command but `setup --upgrade` handles the state the flush guards against. Dropping it could add one state, on a drive that reorders writes in a folder, in which every changing command but `setup --upgrade` refuses until a file is deleted by hand. It would save a quarter of a millisecond per switch on Linux, and nothing on Windows. So all ten flushes of a `goto` stay, as decision 189 point 3 kept them.

**7. Git's rule for `who` at a combine is taken in a later version, through a decision of its own** (item 9; decision 189 point 6; [decision 184](184-opalro-who-the-save-behind-each-line.md), its correction). Where a combine's file is byte for byte a later parent's, `git blame` names every line of it through that parent. On five files of Git's own history, Git's rule named another save for 49 of 13,385 lines and was 4 to 13 % faster ([benchmarks.md](../benchmarks.md) §16). It matches `git blame`, which people compare `who` against, but it changes which save some lines name, so it gets a decision of its own. With it, `who_vs_git.rs` draws a combine that takes one side whole.

**8. The rest stands as decided.** Items 5, 6, 7 and 10 were not put to the owner one by one. They stand until the owner reverses one with a new record, as decisions 193 to 197 and the rulings made while building do.

## Consequences

- No code changes.
- The rulings file says at its top that it was reviewed, and what the review came to.
- In the follow-ups, the four questions under "For the owner to decide" say how they were ruled, `similar`'s row waits, and "Deferred by milestone 11" gains a row for Git's rule.
- In design.md, the open item on library updates and the deferred items on Git's rule and on going back within twice Git's time say what was decided; decisions 197 and 198 join its list of decisions.
- benchmarks.md §15 and §16 name the rulings.
- Dependabot's pull requests #39 and #40 stay open for the owner. As they are, they fail to build: they take `sha2` 0.11, which waits for a stable `sha1-checked` 0.11, and `similar` 3.
