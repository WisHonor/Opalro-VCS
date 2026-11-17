# 186 — Speed before v0.2: measure, take what costs no safety, write the rest up for the owner

- **Status:** Accepted
- **Date:** 2026-11-15
- **Agenda topic:** none (milestone 10 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 128](128-v0.1-ships-without-goto-within-twice-gits-time.md) (`goto` within twice Git's time is a goal for v0.2); [decision 157](157-v0.2-in-four-milestones.md) (going back is measured against twice Git's time in milestone 10); [decision 161](161-what-building-milestone-7-refined.md) and the milestone 7 rulings (`status` needs room under its limit on Linux: a speed pass before v0.2 is released)

## Context

**In plain words.** Two of Opalro's speed promises are tight. Going back to an older save (`opalro goto`) was to come within twice Git's time in v0.2; in the last nightly run it took 2.7 times as long (17.7 ms against 6.5 ms). And `opalro status` with nothing changed sits right at its limit of twice Git's time on GitHub's Linux machines (1.84 to 2.08 over the last runs), with code that did not get slower: Git got quicker when the machines' image changed.

The benchmarks say where the time goes (`docs/benchmarks.md` §7, §8). For `goto`: about ten flushes to the disk (2.6 ms of 25 on a quiet machine) and seven renames, the look at the whole folder that `status` also makes, and some work done twice. For `status`: reading the last save's listings from packs where Git reads one index, opening every folder, handing work between threads, and one small file flushed to read the drive's clock.

The ideas on record are of two kinds. Some change only what is read: the same listing read four or five times in one `goto`, the file cache loaded twice, packs opened in name order before the newest diary entry is found. They are worth about a millisecond together. The others change what makes Opalro safe or how its core works: flushing less, and walking the folder on several threads.

The owner reviews all code that touches the disk, and how much is flushed is the heart of "never lose data". The owner is away.

## Options

- **A. Measure and report, change nothing.** Keeps the promise as decision 157 words it ("is measured"). Leaves a known millisecond on the table.
- **B. Measure, take the changes that alter no write, no flush and no check, measure again, and write up the rest with its measured gain for the owner to rule on.**
- **C. Flush less and thread the walk now.** The only way the numbers on record reach twice Git's time. Changes crash safety and the module that decides what a save holds, unreviewed.

## Decision

**B.**

**1. First, timers.** Throwaway timers around the parts of `goto` and of `status` on the gate's data set, in a release build, on GitHub's Linux machine and on the laptop: each flush, each rename, the scan, each read of a listing, the cache loads, the diary lookup, the drive clock's file. The timers are not kept in the code. Nothing is changed before the numbers exist, and no part is called the cause of the time before it is timed.

**2. Then only what changes no write, no flush and no check:** work that is done twice is done once (a listing already read in the command is not read again; the file cache a scan loaded is handed on rather than loaded again), and a lookup that opens files it does not need opens fewer. Each such change is kept only if the timers show it gains something, and is covered by the tests that already pin what `goto` and `status` do.

**3. Not touched:** what is flushed and when; the order of the journal, the files and `head`; the check of the drive's clock; the walk's threads. For each, the write-up says what it costs today (measured), what changing it would gain, and what it would put at risk. That is the owner's to rule on.

**4. The gate stays as it is:** `status` and the small save at twice Git's time, `goto` failing past 3.5 and saying when it is over the goal of 2 (`bench/run.sh`). If `goto` is still over twice Git's time after step 2, **v0.2 is packaged with that said plainly** in the release notes' known limits and in the list of things to confirm, as v0.1 was (decision 128). Whether v0.2 ships that way, or waits for the flushes to be looked at, is the owner's call.

**5. Written down:** `docs/benchmarks.md` gets the timers' table and the gate before and after; the follow-ups list gets the row the speed pass never had.

## Consequences

- The promise "measured in milestone 10" is kept, and anything gained is gained without touching what keeps data safe.
- On the numbers on record, `goto` will most likely still be over twice Git's time on Linux after this: the flushes alone are more than the work done twice. The owner gets a table that says which flush buys what.
- `status` gains little from step 2 (it reads each listing once already); its room under the limit depends on the drive clock's flush and the threads, which wait.
- Nothing in this decision can lose data: no write changes.
