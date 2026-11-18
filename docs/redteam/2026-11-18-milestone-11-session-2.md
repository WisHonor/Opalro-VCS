# Red-team report: milestone 11, second session (RT26, decision 024, 2026-11-18)

Code attacked: a copy of `milestone-11` at `fd0eb95`, in the no-network sandbox (4 GB, 4 CPUs, user 1000), after the fix wave for RT25. Most of the session went to what the fix wave changed: the import's comparison of commits, its cache of trees and its new accounting. Then RT25-2's fix, and a hostile switch journal, which the first session didn't reach. About one hour and forty minutes.

Builds, all in the sandbox, each in its own target folder under `/tmp`:
- `/tmp/t11`: milestone 11 (`fd0eb95`), release, for every timing.
- `/tmp/t02`: v0.2 (`main` at `3f71add`), release, so both could import the same repositories.
- `/tmp/td`: the debug build of the tests, `--all-features`, for the proofs, the explorer and the journal experiments (which need fault injection).

Proof tests:
- `crates/opal-vcs/tests/redteam26.rs`: one test, RT26-1.
- `crates/opal-vcs/tests/redteam26_platforms.rs`: one test, RT26-2. It reads `/proc`, so the file is `#![cfg(target_os = "linux")]`.

Both are `#[ignore]`d. To see them fail (I added `CARGO_TARGET_DIR=/tmp/td`, since building on the copy's mount is slow):
- `bash redteam/sandbox.sh run cargo test -p opal-vcs --all-features --test redteam26 -- --ignored` (about 5 seconds in a debug build)
- `bash redteam/sandbox.sh run cargo test -p opal-vcs --all-features --test redteam26_platforms -- --ignored` (about 8 seconds)

Both passed `cargo clippy -p opal-vcs --all-features --test redteam26 --test redteam26_platforms -- -D warnings` and `rustfmt --check` in the sandbox. They were not linted on Windows; there `redteam26_platforms.rs` compiles away.

Attack code, in `redteam/attacks25/`, all uncompiled text. Copy the scripts without `.txt` into `rt26/` in the sandbox's copy, since they call each other as `/work/rt26/<name>`:
- `op.sh` runs `opalro` as a test user; `OPALRO_BIN` picks the binary (default `/tmp/t11/release/opalro`).
- `measure.sh` times one import and reads its `/proc` counters (bytes read, peak memory), killing it past a limit: `OPALRO_BIN=/tmp/t02/release/opalro sh /work/rt26/measure.sh /tmp/pb1 label 600`.
- `packbuy.py` makes RT26-1's repositories: `python3 /work/rt26/packbuy.py /tmp/pb1 4095 3000 100 5 760 99000 40` (packs, one-file trees to buy with, copies, levels, letters per level name, files, commits). It calls `fillpacks.py` (RT25's, copied unchanged).
- `widedeep.py` and `widenest.py` make RT26-2's: `python3 /work/rt26/widedeep.py /tmp/wd1m 1000000 4085`, `python3 /work/rt26/widenest.py /tmp/wn 250000 3 4075`.
- `notes16.py` (renames back and forth at long paths, suspicion S1) and `cachemem.py` (trees that cost the cache its least, S2).
- `journal1.sh`, `journal2.sh`, `journal3.sh` (a hostile journal) and `swap1.sh` (RT25-2's fix): `bash redteam/sandbox.sh run sh /work/rt26/journal1.sh`. What they printed is in the matching `.out.txt` file.
- `rt26_sanity.rs.txt`: RT26-2's proof with the wide folder at the top, where its paths are short. It passes, holding under the bound, which shows the bound is fair. Copy it to `crates/opal-vcs/tests/rt26_sanity.rs`, run it as above, then remove it.
- `explore.out.txt`: RT25's explorer (`attacks24/rt25_explore.rs.txt`) run again on this code, with `RT25_SEEDS=150 RT25_OPS=14`.
- `timings.txt` holds every time and peak measured.

## Summary

The fix holds against what RT25 did: RT25-1's proof passes, and RT25's padded repository imports as fast as in v0.2. The notes held too: RT25's explorer gave 1,066 notes over 4,500 saves, every one true and exactly what a save's own rule gives, and a second import gave the same saves. RT25-2's fix counts every kind swap I tried, and a hostile journal made every changing command refuse without writing anything.

Two things did not hold. The base now counts 1 KiB for each pack index searched, and a search takes tens of nanoseconds: 4,095 packs of one blob each, beside a history the floor stops in 6 seconds, let it compare 3.4 s for every commit, about 36 minutes by the counts before it stops, and hours with more trees (RT26-1). And the comparison's list of folders still to compare holds each one's whole path: a 4.9 MB repository of two commits made the import hold more than 4 GB and be killed, where v0.2 holds 312 MB (RT26-2). That one is older than the fix wave; RT25 didn't find it.

| | High | Medium | Low |
|---|---|---|---|
| Findings | 0 | 2 | 0 |

## Findings

### RT26-1 (Medium): each pack index searched buys the comparison 32 KiB, though a search takes tens of nanoseconds

Threat: T7 and T59 (security.md says a history "built cheap to convert and costly to compare" stops the import, and that "packing can move where the stop falls").

What happens: the base the comparison's allowance is 32 times is `read_work(bytes, searched)` for each tree the converter reads: its bytes, 1 KiB for the read and 1 KiB (`INDEX_SEARCH_WORK`) for each pack index searched to find it (`read_folder` in `convert.rs`). A search is a binary search of an index already in memory (`Pack::find`): v0.2 made about 12.5 million of them importing the repository below in 0.87 s, so one takes at most about 70 ns. With 4,095 packs that each hold one blob, and the history's objects loose, every tree read searches 4,096 indexes, so it adds 4 MiB to the base and buys 128 MiB of comparing. The comparison then walks its trees from the cache, where a tree counts only its bytes and searches nothing. So the cheap side of the allowance is priced like the dear one.

`packbuy.py` builds the shape: two top trees taken in turns, each reaching 100 copies of a folder of 99,000 files whose contents all differ between the two, at paths of 3,816 bytes, so each comparison walks about 634 MB of trees from the cache, about 3.4 s in release. Beside them, `buy/` holds one-file trees, the same in both tops, converted once. Every object is loose. Release builds, the same repositories:

| Repository | Milestone 11 | v0.2 |
|---|---|---|
| 3,000 one-file trees, no extra packs, 40 commits | stopped, 5.8 s | 0.43 s |
| the same beside 4,095 one-blob packs (8,190 files, 5.2 MB) | imported all 40, 135.7 s | 0.87 s |
| 100 one-file trees beside the packs, 40 commits | stopped after 84.0 s, at "15 GiB" | not run |

The third row stopped where the counts say (32 times 115 trees times 4,097 KiB is about 14.4 GiB, which the stop rounds up to 15, or about 24 comparisons), so the counts can be trusted for the second: its allowance is about 405 GB, and it stops after about 640 commits, 36 minutes. With 30,000 one-file trees (30,000 loose objects of about 50 bytes) it would be about 6,400 commits, 6 hours. Reading those trees took the converter milliseconds: v0.2 imports the whole repository in under a second.

Without the extra packs the small trees stay within the rule: 100,000 of them bought about 3.4 GB of comparing, 16 s, about 2.4 times v0.2's whole import (`timings.txt`, pb4). Only the searches are priced far past their cost.

How it is proved: `redteam26.rs`, `a_history_the_floor_stops_still_stops_beside_packs_of_one_blob`. It writes, all loose, two top trees taken in turns over 20 commits, each holding 760 copies of a folder of 1,000 files with 765-letter names, the copies in one top differing from those in the other by one file, so each comparison walks 1.2 GB from the cache; and 256 one-file trees under `buy/`. Imported alone, the history stops at its first comparison, which the test checks first. With 4,095 packs of one blob each beside it, it should stop too. Today it imports all 20 commits: `beside 4,095 packs of one blob each, the import of a history the 1 GiB floor stops at its first comparison went on, comparing about 1.2 GB for each of 19 commits: Imported 20 commits from Git: 1 line, 0 tags.`

What it costs a person: an import of a small repository from a stranger runs for hours with no stop and no hint why, where the same history without the filler packs stops in seconds. Nothing is lost: stopping it by hand commits nothing. Git itself merges packs once there are 50 (`gc.autoPackLimit`), so no honest repository looks like this, but a hostile one costs nothing to make so.

What I would do: take the searches out of the base, as the fixer's report offers ("drop `INDEX_SEARCH_WORK` from `read_folder`'s `read_work`, one line"), and keep charging them where the comparison reads a tree again, where they are its own cost. Then the base doesn't depend on how Git packed the repository, as the owner's ruling sentence has it, and the proof passes: the history stops at its first comparison with the packs or without. Counting a search at a few bytes, or at most a few searches a read, would do too.

### RT26-2 (Medium): the comparison's list of folders still to compare holds each one's whole path

Threat: T7 (size bombs) and T59 ("comparing its commits costs without end (deep trees ...)").

What happens: `compare` in `convert.rs` keeps the folders it has still to compare in `todo`, a stack of `(path, old, new)`. Going through a folder, it pushes every subfolder that differs, each with its whole path, a fresh `String`, before it compares any of them. A path may be 4,096 bytes and a folder may hold 1,000,000 entries, so one folder deep down puts up to 4 GB of paths on the stack at once. The allowance doesn't see memory, and the trees involved count only a few hundred bytes each.

`widedeep.py` builds the shape: a folder 4,085 bytes down (five levels of 765-letter names and one of 255) holds `n` folders, each holding the file `x`, as the blob A in the first commit and B in the second. All `n` differ, so all `n` paths go on the stack. Release builds:

| Repository (two commits) | Milestone 11 | v0.2 |
|---|---|---|
| 250,000 folders, 1.4 MB | 1.95 s, peak 1,081 MB | 0.43 s, peak 91 MB |
| 1,000,000 folders, 4.9 MB | killed (exit 137) after 18.2 s, at 4,153 MB, the sandbox's limit | 1.96 s, peak 312 MB |
| 250,000 folders three times nested under `z` (`widenest.py`), 3.8 MB | 5.44 s, peak 3,147 MB | 1.29 s, peak 179 MB |

Nesting multiplies it: the stack goes into `z`, sorted last, while the folders before it still wait. A save's 10,000,000 entries allow five levels of 1,000,000 such folders, about 20 GB of paths from a repository of about 25 MB. RT25's padded repositories and deep chain didn't meet this, since their folders hold one entry each. The fix wave's T6-4 change builds no path for an entry alike on both sides, but a folder that differs still pushes a whole one.

How it is proved: `redteam26_platforms.rs`, `comparing_a_wide_folder_deep_down_holds_no_path_for_each_entry`. It writes the two commits with 300,000 folders 4,085 bytes down, runs `opalro import` and watches its peak memory (`VmHWM` in `/proc/<pid>/status`), killing it past 512 MiB. Today it fails: `the import of two commits of 300000 one-file folders had held 513 MiB, past the 512 MiB it should need, and was still going`. The same 300,000 folders at the top of the tree, where their paths are short, import within the bound in a debug build (`rt26_sanity.rs.txt`, 13 s).

What it costs a person: importing a stranger's repository of a few megabytes takes gigabytes of memory, enough to kill the import on a small machine or push a laptop into swapping; nested, more than most computers have. Nothing is lost: the import commits nothing until its end, and a killed one leaves only the `.opalro` folder it began.

What I would do: don't keep a path for each folder waiting. Keep each folder's path once (an index into a list of the folders gone into, or an `Rc<str>` of the parent's path beside the child's name), and build the whole path only where it is used: `each`, `entry`, and messages. Or walk depth first with a stack of per-folder positions, as the converter's own snapshot walk does, so the stack holds one frame per level. The proof's bound then holds, as its sanity run shows.

## Suspicions without proof (not counted)

- **S1: renames back and forth make every imported save as big as a save's notes may be.** `notes16.py` renames 8,000 files with contents of their own at paths of 3,835 bytes, in every commit. Each save keeps 16 MiB of note paths (`MAX_NOTED_BYTES`), and pairing, sorting and storing them costs about 64 ns for each byte the comparison counts, about ten times what a walk does. 50 commits took 4.03 s against v0.2's 0.65 s, and `opalro history` then took 0.80 s over them. At the floor that is about 1,000 commits and a minute, and every later reader pays about 16 ms a save. That is T44's known cost of big saves, reached here from a Git repository rather than a planted store, and bounded by the save's own limits, so I count it only as what RT26-1 makes long.
- **S2: the cache's memory is up to about four times its count.** A tree with no entry the import keeps counts 64, but takes about 250 bytes: its shared list, its place in the map by ID and its place in the order of use. 300,000 such trees (`cachemem.py`, each holding only `.git`) made the import peak at 305 MB against v0.2's 202 MB. A full cache of them would take about 260 MB. It is bounded, and v0.2's own map of converted trees grows the same way, so it is not a finding; `cache_cost` says "about what it takes in memory".
- RT25's S2 (the comparison's reads again pushing a repository past the delta allowance) wasn't built. With the cache, only trees too big for it are read again: a folder of about 900,000 entries with short names, fewer with long ones.

## What was tried and held

**The comparison's accounting** (`timings.txt`)
- RT25's proofs pass on this code: RT25-1's in 3.96 s (it failed at 48.7 s), RT25-2's in 0.25 s. RT25's padded repository (`padpack.py`, 20 commits, fixed blocks) imported in 29.8 s in release, as v0.2 does (261.1 s before the fix): the comparison takes both trees from the cache, and what is left is the converter's own inflating (RT25's S1).
- Small trees buy within the rule: pb4's 100,000 bought 16 s of comparing (above).
- By the code: padding is counted for every stream a read inflates whole, the bases and deltas behind it included; a loose stream's trailing bytes are refused, a pack entry's never read; a loose header is at most 32 bytes. Rebuilding through deltas stays bounded by its own allowance, unchanged.
- By the code, the work the walk does for each entry is in proportion to what it counts, but for the paths of RT26-2 and the notes of S1. A deep chain's paths, built only where entries differ, cost by my estimate about what a wide folder's entries do for each counted byte; not timed.

**The notes after the walk's changes** (`explore.out.txt`)
- RT25's explorer, 150 random histories of up to 14 changes a commit: 4,500 saves and 1,066 notes, every note true of its save and first parent and exactly what a save's own rule gives, and a second import identical; its Git LFS case too.
- By the code: building a path only where an entry differs skips exactly the entries alike on both sides, as before. The cache hands out `Rc<[Item]>`, never changed after it is made; `read_folder` and `kept` judge names alike and sort them alike; the cache is keyed by Git ID. What it holds depends only on the order of asks, so two imports of one repository take the same path through it; only packing moves the stop (RT26-1 is that, taken to its end).

**RT25-2's fix** (`swap1.sh`, `swap1.out.txt`)
- `goto`, its `--json`, `undo` and `redo`, for a file and a full folder, a link and an empty folder both ways, a file and a folder three levels deep both ways, and a link to a folder giving way to a file: every count matched the disk. `discard` of a folder put where a saved file was and a file where a saved folder was, then its `undo` (`2 files back, 3 removed.`), and `shelf` and `unshelf` of a file turned into an empty folder, counted right too.
- A file and a link swapping places stays `1 file written.`, the fixer's choice 7: a link counts as a file there.

**A hostile journal** (`journal1.sh` to `journal3.sh`, and their `.out.txt`)
- A journal of 0 bytes, and one written back after a later command moved `head`, put to the commands `recovery.rs` doesn't run: `start` and `import` refused as in any project; `discard a.txt`, `goto <fingerprint>` and `resolve --keep mine a.txt` refused in the journal's words with the folder, `head`, the store and the journal unchanged; `resolve --copy` refused as there was no conflict; `setup` and `setup --upgrade` changed nothing; `status`, `changes`, `who`, `show` and `verify` warned and exited 0.
- A journal back after an `undo`, or an `undo --to` the switch's own starting entry: each step makes a new diary entry, so `head` is never the journal's `from` again, and the journal reads as made for another save. No command I found puts `head` back on an entry it already had: even the pick-up after a lost `head` makes a new one, and drops the journal (`recovery.rs`).

## What was not reached

- A journal naming another target with `head` on its `from`: crafting one needs its BLAKE3 checksum, which the sandbox's Python lacks. By the code it is carried out as any stopped switch, each change checked against what the plan found, so it can do what a planted store can and no more.
- An honest-looking repository that 0.2 imports and milestone 11 stops: only the shapes above were built, none of them honest. The likeliest is a monorepo whose folders are too big for the cache and stored as long delta chains.
- Windows and macOS: kind swaps with names this computer can't write, and links written as placeholders.
- RT26-1 combined with S1, which would make each of its commits slower still.
