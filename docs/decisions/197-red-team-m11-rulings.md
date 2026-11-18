# 197 — Red team on milestone 11: two sessions, four findings, all fixed, and one estimate left open

- **Status:** Accepted
- **Date:** 2026-11-18
- **Agenda topic:** none (milestone 11, its red-team sessions RT25 and RT26)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 195](195-the-import-records-the-renames-it-is-sure-of.md) (what comparing commits for their renames may cost, and how it is counted); [decision 196](196-empty-folders-in-a-switch-and-in-status.md) (what a switch counts when a folder and a file trade places)
- **Refined by:** none yet

## Context

Milestone 11 was attacked twice from the no-network sandbox ([decision 024](024-red-team-gate.md)).

**In plain words.** To note renames, the import compares each Git commit with its first parent. Both sessions went mostly after what that comparison costs. The first found a way to make it run for hours: its limit counted what Git's folders hold, not what reading them takes. The fix kept the folders it reads in a cache and counted what reading costs. The second session found two holes in that fix: filler files that bought the comparison time for nothing, and a list of folders still to compare that could take gigabytes. All four findings are fixed. A review of the fixes then estimated one more way to make the comparison slow, which nobody has built; it is left open, in writing. Nothing could be lost in any of them: the import writes nothing until its end.

**The first session** (RT25, [report](../redteam/2026-11-17-milestone-11.md)) attacked `milestone-11` at `141ebaf`. What held: in 270 random Git histories every rename noted was true of its save and its first parent and exactly what a save's own rule gives, and a second import gave the same saves byte for byte; short names, empty folders in `status`, and `resolve --keep` held too. Two findings:

- **RT25-1 (Medium).** The comparison's allowance (at most the larger of 1 GiB and 32 times the bytes of the trees converted) counted what a tree holds, not what reading it takes. A tree padded with empty deflate blocks costs thousands of times its bytes to inflate. An 8 MB repository made each commit cost 12 seconds of reading in a release build while the allowance counted 58 bytes, so the import never stopped. Proof: `an_import_reads_no_more_than_its_allowance_past_the_repository` (`redteam25_platforms.rs`, Linux).
- **RT25-2 (Low).** A switch that puts an empty folder where a file was removed the file and said `No file needed changing.`, with `"removed": 0`; the other way round it left out the folder it removed. Proof: `a_switch_that_puts_a_folder_in_a_files_place_counts_the_file_it_removes` (`redteam25.rs`).

And two suspicions: Opalro's inflater is very slow on empty fixed-Huffman blocks, about 400 times slower than Git's zlib, which is what makes RT25-1 costly (S1); and the comparison's reading again might push a repository past the import's allowance for rebuilding through deltas (S2).

Between the sessions the whole-branch review asked for a sturdier fix of RT25-1 than first ruled: compressed bytes must never buy allowance, since padding them is what costs. The fix wave built it:

- **What converting buys.** The base is what reading the converted trees cost: their bytes and 1 KiB for each read, never their compressed bytes.
- **What comparing spends.** Every tree the comparison walks counts by its bytes. A tree it must read again also counts 1 KiB, 1 KiB for each pack index searched, and 1,000 for each compressed byte its stream takes past what zlib could need for what it gives.
- **A cache** of the trees the comparison walks, fed by the converter as it reads them, at most 64 MiB by a count that is the same on every computer, the least recently used dropped first.

It fixed RT25-2 both ways round, and the review's small findings.

**The second session** (RT26, [report](../redteam/2026-11-18-milestone-11-session-2.md)) attacked `milestone-11` at `fd0eb95`. What held: both of RT25's proofs pass, and RT25's padded repository imports as fast as v0.2 imports it; RT25's explorer, 150 more histories with up to 14 changes a commit, found all 1,066 notes true and exact; every kind swap of RT25-2's fix counted right; and a hostile switch journal (empty, or written back after a later command) made every changing command refuse without writing anything. Two findings:

- **RT26-1 (Medium).** The fix wave's base also counted 1 KiB for each pack index the converter searched, and a search takes tens of nanoseconds. 4,095 packs of one blob each, beside a history the 1 GiB floor stops in 6 seconds, let it compare for about 36 minutes before stopping, and hours with more trees. Proof: `a_history_the_floor_stops_still_stops_beside_packs_of_one_blob` (`redteam26.rs`).
- **RT26-2 (Medium).** The comparison kept each folder it had still to compare with its whole path, and pushed every differing subfolder of a folder before going into any. A 4.9 MB repository of two commits, one wide folder 4,085 bytes down, made the import pass 4 GB and be killed, where v0.2 peaks at 312 MB. It was older than the fix wave. Proof: `comparing_a_wide_folder_deep_down_holds_no_path_for_each_entry` (`redteam26_platforms.rs`, Linux).

And two suspicions: renames back and forth at long paths make each imported save as big as a save's notes may be, which costs every later reader of that history (S1, the known cost of big saves, T44); and the cache's memory is up to about four times its count (S2).

**The review of the fixes.** A scoped re-review read both rounds of fixes and found every finding above addressed, and the new walk handing the pairing of moves the same paths as before. It raised one new concern, an estimate it did not build (N-1): fixed-Huffman blocks that each carry one short match cost the inflater about 2 µs a block, as empty ones do, yet take no more compressed bytes than they give, so a tree written that way and read again is charged only as plain bytes. By its estimate a million one-file trees could buy about two hours of comparing, where v0.2 imports the same repository in about a minute and a half. And two small ones, fixed: the cache's feed copied a tree too big for the cache before finding it couldn't hold it, and a test's doc comment had landed on the wrong function.

**Measured** on Git's own repository (86,322 commits), on two runners: the import took 12.8 % and 10.6 % longer than v0.2's, where the first build took 38 to 40 % longer; the comparison itself costs about 5 seconds, where it cost 50; the peak memory is about 150 MB higher, the cache's ([benchmarks.md](../benchmarks.md) §16).

## Options

**The comparison's cost (RT25-1):**
- **A. Count what reading costs, and keep a cache** of the trees the comparison reads, so honest imports read each tree about once.
- **B. Stop noting renames past a budget,** instead of stopping the import. Safe only if where the cut falls depends on the history alone; what reading costs depends on how Git packed it, so two clones of one history would get different fingerprints, which decision 195 point 3 forbids.
- **C. Keep the count in what trees hold,** and accept that a crafted repository keeps an import busy for hours.

**What the base counts (RT26-1):**
- **A. Each converted tree's bytes and 1 KiB for its read,** with index searches charged only where the comparison reads a tree again.
- **B. Price a search at a few bytes.** It keeps a charge that buys almost nothing and still moves with packing.

**The comparison's list of folders (RT26-2):**
- **A. Walk depth first with one frame per level,** each holding its folder's path once, as the converter's own walk does.
- **B. Keep the parent's path once,** shared, beside each waiting folder's name.

**The re-review's estimate (N-1):**
- **A. Count deflate blocks where a tree is read again,** and charge each. The inflater (miniz_oxide) reports block boundaries only through its core interface, not the streaming one Opalro's Git reader uses, so this reworks how the reader inflates.
- **B. An inflater that keeps the fixed tables,** as zlib does, so a block costs next to nothing to start. It would also end S1 in the converter, but [decision 031](031-own-git-reader.md) chose this inflater because it forbids `unsafe`.
- **C. A known limit for now,** written in security.md, on the follow-ups list, and first in the next red team's brief, to be proved or cleared.

**A third session on the last fixes:**
- **A. None,** as in milestones 7, 8 and 9.
- **B. A third session now.**

## Decision

**A for RT25-1, A for RT26-1, A for RT26-2, C for N-1, and A: no third session.**

**1. RT25-1 is fixed as option A,** in the fix wave, then corrected by RT26-1: the comparison may do at most the larger of 1 GiB and 32 times what reading the converted trees cost (their bytes and 1 KiB a read). Every tree it walks counts by its bytes; one it reads again also counts its read, its index searches and 1,000 for each byte of padding. The cache holds at most 64 MiB by its count. Past the allowance the import stops in its own words, with nothing committed, and says this is a limit of its rename notes, not damage in the repository. Packing can move where the stop falls, through what reading again costs, but never the saves. Whether to stop the import, or to stop noting renames past one budget and stop the import past another, is put to the owner ([rulings](../plans/2026-11-17-milestone-11-rulings.md), "Please confirm" 4).

**2. RT25-2 is fixed:** a switch counts an entry it removes to put one of another kind in its place, a file giving way to a folder and a folder to a file or link, in its words and in `--json`'s `removed`. A file and a link trading places stays one write, which `written` shows.

**3. RT26-1 is fixed:** the base counts no index searches, so it doesn't depend on how Git packed the repository.

**4. RT26-2 is fixed:** the comparison walks depth first, one frame per level, and builds a path only where it uses one. It hands the pairing of moves the same paths as before, in another order, which the pairing doesn't depend on.

**5. N-1 is a known limit, option C.** It is an estimate, and decision 024 counts a finding only with a proof. Its cost is a constant factor of the repository's own size (about 80 times v0.2's import of that shape, by the estimate), the kind T7 already accepts for rebuilding through deltas (35 to 1,000 times), not a tiny repository that costs hours, as RT25-1 and RT26-1 were. Options A and B both reach into the Git reader, and B into decision 031. security.md T59 says it; the follow-ups have it as "Should do, red team first", with both ways; milestone 12's red team gets it first. The re-review's two small findings are fixed.

**6. The suspicions.** RT25's S1 is counted wherever the comparison reads a tree again; in the converter's single read of each object it stays as in v0.2, bounded by the repository's size, on the [follow-ups](../plans/2026-11-01-milestone-1-follow-ups.md). RT25's S2: with the cache the comparison adds 2.15 % to the rebuilding work on Git's own history, against 2.8 % before; on the follow-ups. RT26's S1 is T44's known cost of big saves, reached from Git and bounded by a save's limits; on the follow-ups. RT26's S2 is bounded by the cache's count; the pages give its real memory, and a leaner cache is on the follow-ups.

**7. No third session.** RT26-1's fix is one line; RT26-2's changes how the walk keeps its folders, not what it compares. Each has its proof, CI runs both on all three systems, and the scoped re-review read both with the fix wave. What neither session reached goes to the follow-ups and to milestone 12's red team: a journal naming another target with `head` on its `from` (crafting one needs its checksum), an honest repository that v0.2 imports and this one stops, and a switch's counts on Windows and macOS.

## Consequences

- With this record every finding of both sessions is fixed, which is what [decision 024](024-red-team-gate.md) asks before a milestone is merged. One estimated cost is open, in writing.
- An import of a hostile repository costs, in comparing, at most 32 times what converting its trees cost by the count, plus the floor, but for N-1; and memory in proportion to its folders' width, as converting does.
- An honest import of Git's own history takes 11 to 13 % longer than v0.2's and about 150 MB more memory; the first build of this milestone took 35 to 40 % longer.
- The proofs of all four findings pass and stay in the test suite: `redteam25.rs`, `redteam25_platforms.rs`, `redteam26.rs` and `redteam26_platforms.rs`.
