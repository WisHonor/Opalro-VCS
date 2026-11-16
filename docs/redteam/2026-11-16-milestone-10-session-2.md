# Red-team report: milestone 10, second session (RT24, decision 024, 2026-11-16)

Code attacked: a copy of `milestone-10` (version 0.2.0) after the fix wave. Built with `--all-features` in the no-network sandbox (4 GB, 4 CPUs), debug build throughout. About ninety minutes, almost all of it on experiments.

Proof tests: `crates/opal-vcs/tests/redteam24.rs`, one test. No platform tests were written, so there is no `redteam24_platforms.rs`.

Attack code: `redteam/attacks23/rt24_explore.rs.txt`, an exploring test file that is not compiled. To run it, copy it to `crates/opal-vcs/tests/rt24_explore.rs`, then run `bash redteam/sandbox.sh run cargo test -p opal-vcs --all-features --test rt24_explore -- --nocapture --test-threads 1`, then remove it. `redteam/attacks23/try_pages.sh` runs the release page's steps (`bash redteam/sandbox.sh run bash redteam/attacks23/try_pages.sh`). What it printed is in `try_pages.out.txt`. `run3.txt` and `run4.txt` hold two runs' output.

## Summary

The amend's holder check, `who`'s old paths, the switch's file cache, `combine` and `resolve` run together, three common starting points, and an unwritable `.opalro/tmp` all held under the experiments below. One Low finding: a combine that says no file needed changing makes an empty folder for a place in conflict that has nothing in the folder, and keeping the removal leaves that folder behind.

| | High | Medium | Low |
|---|---|---|---|
| Findings | 0 | 0 | 1 |

## Findings

### RT24-1 (Low): a combine makes an empty folder for a conflict whose folder the other line moved away

Threat: new. Near §4's "a command that says it changes nothing changes something", and the same kind of fault as milestone 9's `made_on_the_way` fix.

What happens: `main` moves the folder `d` to `e` (or removes it) and saves. `side` changed `d/x.txt`. `opalro combine side` reports `d/x.txt: changed on side, removed on main; nothing is in your folder` and `No file needed changing.`, but the folder now holds an empty `d`. `opalro resolve --keep mine d/x.txt` keeps the removal and leaves the empty `d` in place. `opalro status` never mentions it. No data is lost; the folder just holds an empty folder that no save has and that the user never made.

How to see it: `redteam24.rs`, `a_combine_that_writes_nothing_makes_no_folder_a_moved_away_conflict_sits_in`. It fails today with `an empty folder d: after the combine true, after resolve --keep mine true`.

What I would do: have the combine's write-out make a folder only when something is written into it (the rule `made_on_the_way` already applies to limited write-outs), or have a resolve that keeps a removal remove the folder the combine made, if it is empty. Or the owner may rule that a folder holding a place in conflict is made on purpose. If so, the test can take an `#[ignore]` naming that ruling.

## Suspicions without proof (not counted)

- `opalro undo` after a resolve that replaced the combined save prints `Undid resolve: main is back at nnuw`, the same four letters it was at, because the replacement keeps the change ID. It is what the page says. Someone reading only the terminal may think nothing moved.

## What was tried and held

**The amend's holders (`shown_elsewhere`, `holder_reading` sharing what it read across turns)**
- 150 random planted histories (6 to 35 saves; 1 to 3 parents each; times all equal, negative, or random in a wide range, so parents are often dated after their children), with up to 5 lines, 2 tags and 2 server lines on random saves and `main` on a random save. Each time, `opalro save --amend` was refused exactly when a brute-force search found a holder, and it named exactly the holders found. Where it said "and N more", the count matched. No miss and no wrong name (`rt24_amend_holders_random`, 1 to 150).
- A line whose history names a save the store lacks, beside a line that really holds the save, in both name orders: the amend was refused, saying the project looks damaged and naming the broken line and the missing object (`rt24_amend_missing_save`).

**`who` and old paths**
- A model on the command line: 12 random histories of 30 steps on two lines, with edits, moves to a free name, two files swapping names, moves into and out of a folder, and combines with conflicts kept as `mine`. For every line `who --json` named, in the folder and at three saves, with the normal deadline and with `OPALRO_DIFF_DEADLINE_MS=0`, the named save holds that line at the named path (`was`, else the path asked about). 938 lines were checked (`rt24_who_moves_random`).
- By the code: lines wait by (save, path), and the path is set only when a move note is true of a save and its first parent. A swap gives two keys, so a line can't take the other file's path. Remembered comparisons are keyed by both versions' content IDs and claimed sizes, and a size counts only when it is over the 64 MiB limit, so a lying size changes whether a side is read, never which lines match.

**The switch and the file cache**
- With unsaved work in a file the other line changed, and a trusted cache written before the edit, I put `.opalro/cache/files` in seven states: as it was, missing, cut in half, last byte flipped, a byte in the seal flipped, rewritten by a `status` after the edit, and the old cache from before the edit put back. `goto side` and `combine side` refused every time, and the work stayed in the file (`rt24_cache_states_switch`). By the code: the switch checks each file's details against the cache right before it acts (`writeout.rs`, the file's size, times, file number). A file changed after the scan changes its times, so the body handed on from the scan can't vouch for it.

**`combine` and `resolve` at the same time**
- 16 rounds, each order: a fixed conflict on `main`, then `resolve` and `combine third` started together. In every round one waited for the other ("Waiting for another Opalro command to finish..."), both ended 0, `verify` passed, and the fix was in the save or the folder (`rt24_combine_and_resolve_together`).

**Other shapes not reached before**
- Three common starting points (main and side each combine three lines, in other orders, then side into main), 12 rounds, so the starts' IDs came in many orders. Every round gave the same file and the same single expected conflict (in the rounds where `g.txt` was added on all three lines), and `verify` passed (`rt24_three_starts`).
- A conflict below a folder the other line moved, both ways round, kept each way: no content was lost, `who` answered for both files, and `verify` passed. This is where RT24-1 was found (`rt24_conflict_under_moved_folder`).

**`.opalro/tmp` that can't be written** (0555, as user 1000)
- `status`, `changes`, `changes a.txt`, `who a.txt`, `save S`, `save S a.txt`, `save --amend`, `goto side`, `combine side`, `discard a.txt`, `undo`, `line new`, `tag`: each ended with exit 1 and changed nothing under the folder or `.opalro`, compared byte for byte. The commands that test the drive say `Opalro couldn't make a file in <path>/.opalro/tmp: Permission denied`. Those that take the change lock first say they couldn't write `state/lock-info`. `verify` and `history` work (`rt24_tmp_unwritable`).

**The release page's steps** (`docs/releases/v0.2.0-before-you-publish.md` §2, with `printf '%s\r\n'` in place of `Set-Content`)
- Every printed line the page quotes came out as quoted: the combine's conflict, `status`, the marked text, `resolve`'s two lines, `history`, `who`, the tag, the refused amend naming `line side and tag v1.0`, the five undos, the sixth undo's refusal, `undo --shelve`, `notes.txt` as `added 10 B`, and `redo`. `unshelf` then brought the fix back. The 0.1-project part was not run, since it needs a 0.1 project.

## What was not reached

The `cache/last-made` note copied from another project of the same user was only reasoned through, not planted. A note names a diary entry, which another project only has if it is a copy, and a copy's note then describes that copy. The planted `who` histories with more than 256 comparisons to remember were not built. The Windows and macOS letter-case shapes of the hoisted lookups had no platform tests.
