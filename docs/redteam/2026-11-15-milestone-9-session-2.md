# Red-team report: milestone 9, conflicts, second session (RT22, decision 024, 2026-11-15)

Code attacked: a copy of `milestone-9` at 76ba7bb (the fixes of decision 182). Built with `--all-features` in the no-network sandbox (4 GB, 4 CPUs), debug build throughout. About ninety minutes of work.

**The first session's memory proof, run on Linux for the first time:** `status_and_save_hold_little_for_many_wide_conflict_records` (`crates/opal-vcs/tests/redteam21.rs`) **passes** in the container: `bash redteam/sandbox.sh run cargo test -p opal-vcs --all-features --test redteam21 -- --nocapture` gives `4 passed; 0 failed`. The test prints nothing, so it says only that both commands stayed under its 120 MB bound for 20,000 planted records. The same shape measured with the harness (20,000 records of 255 versions, one folder): `status` 84 MB, `save` 85 MB, `resolve` 84 MB, `goto` 83 MB. At 250,000 records `status` now peaks at 196 MB where it aborted at 3,075 MB before the fix (table below).

Proof tests: `crates/opal-vcs/tests/redteam22.rs`, 1 test, asserting what should be true; it fails today (`0 passed; 1 failed`). The file passes `cargo fmt --check` and `cargo clippy -p opal-vcs --all-targets --all-features -- -D warnings` in the container. It uses nothing gated on a platform.

Platform tests: none. No `redteam22_platforms.rs` was written.

Attack code: `redteam/attacks21/`. The `.rs.txt` files are uncompiled tests; to run one, copy it into `crates/opal-vcs/tests/` as `<name>.rs`, run `bash redteam/sandbox.sh run cargo test -p opal-vcs --all-features --test <name> -- --nocapture`, then remove it. The `.txt` files beside them are their outputs.
- `rt22_memory.rs.txt`: the first session's harness with `status`, `goto`, `show`, `changes --save`, `resolve`, `tidy`, `verify` and `save`; `RT22_N` sets the number of records (`env RT22_N=250000 cargo test …`). Output: `memory-250000.txt`.
- `rt22_slow.rs.txt`: time of `status`, `goto`, `resolve`, `save` for 10,000, 20,000 and 40,000 conflicts in one folder, each its own record and all one record (`RT22_SIZES`). Output: `slow.txt`.
- `rt22_crash.rs.txt`: `resolve` stopped at each of its writes (`OPALRO_FAULT_AT=1…`), four ways. Output: `crash.txt`.
- `rt22_tidy.rs.txt`: `tidy` past the undo window, called in-process with a clock 40 days ahead. Output: `tidy.txt`, `tidy-control.txt`.
- `rt22_keep.rs.txt`: the exploration behind RT22-1. Output: `keep.txt`.
- `rt22_starts.rs.txt`: three common starting points that disagree. Output: `starts.txt`.
- `rt22_paths.rs.txt`: a folder's path given to `resolve` in seventeen spellings. Output: `paths.txt`.

## Summary

The four fixes held against everything I tried. `--only-if-clean` read through: the "nothing" case now looks through the line's tip with the same measure as a forward. A folder's path in every spelling I tried (trailing `/`, `./`, `/.`, `..`, `//`, from a subfolder, with `--yes`, `--json`, with a pattern or a second path) never recorded the picture changed in place. Memory at 250,000 wide records is now 82 to 336 MB in every command, against 3 GB before. `resolve` stopped at each of its writes always left unsaved work alone, finished when run again, and `undo` gave the folder back exactly. `tidy` past the window kept every version of every kept conflict, including one only a shelf reached.

One finding, Low, on a planted save only.

| | High | Medium | Low |
|---|---|---|---|
| Findings | 0 | 0 | 1 |

## Findings

- **RT22-1 Low (`resolve --keep` turns into a folder a link entry nobody named, when the link's ID is the kept folder's listing ID; T47, planted records; `resolve.rs` `Built::keeping`).**
  - **What happens.** To keep a folder version, `keeping` puts it into the overlay as a `Symlink` entry with the folder's listing ID, and then, in every listing the overlay rebuilds, turns back into a `Folder` **every** `Symlink` entry whose ID is in the set of kept folder IDs, matched by ID, not by path. I planted a save on `main` whose root holds a conflict `d` (yours: a folder with `y.txt`; theirs: a folder with `x.txt`) and, beside it, a link entry `l` whose ID is theirs's listing ID. `opalro resolve d --keep theirs` ends with 0 and says `Took theirs for 1 place.` The new save's root holds `d Folder 9b4b…` and `l Folder 9b4b…`: the link `l`, which nobody named, is now a folder holding theirs's `x.txt`. Opalro never makes such a link (a link's target and a listing are hashed as different kinds), so only a planted or damaged save reaches this. But a command that was asked about `d` changed the kind of another entry, and the result is a save that reads cleanly where the planted one could not (a link whose object is a listing).
  - **How to see it.** `keeping_a_folder_side_changes_no_other_entry`.
  - **What I would do.** Turn the leaf back into a folder by its path, not by its ID: keep the set of places (`path` → listing ID) and, when the overlay stores a listing, convert only the entry at a kept place. Or give the overlay a real "folder, whole" kind of change, so nothing is carried as another kind.

## Suspicions without proof

None counted.
- **`goto` reads each record several times.** At 250,000 wide records `goto main` took 70 s against 5.3 s for the same save of files, while `status` took 27 s; with all entries naming one record, `goto` took 0.8 s for 40,000. So `goto`'s cost is per distinct record read, about four reads' worth per place to `status`'s one. The growth is linear (3.3 s, 7.4 s, 15.4 s for 10,000, 20,000, 40,000), so it is not a trap, and I did not time the parts to name where the reads are.
- **The wording of a whole conflict with an absent side.** After three starting points that disagree, `combine` says `f.txt: changed on both lines, and Opalro can't tell what it held where they parted` where the other line holds no `f.txt` (it was moved there). Nothing is lost (`--copy mine` gives the version, `--copy theirs` says the other line holds no file). The first session noted the same looseness of "changed" for moves.

## What was tried and held

- **`--only-if-clean` with nothing to combine** (fix 1), by reading `combine_locked` and `held_in`: the line's tip is measured with `add_all_standing`, which counts every kind-6 entry, those inside a folder a conflict stands as included, and a save past the limits is refused with the flag. A line back at a combined save after `undo` of a `resolve` goes through the same check. A project without the feature skips it, as decision 182 rules.
- **A folder's path to `resolve`** (fix 3), run: `art/`, `./art`, `art/.`, `art/../art`, `art//`, `art --yes`, `art --json`, `art a.txt`, `art/*`, `*`, `**`, `art/p*`, `art/pic.png/..`, and from inside `art`: `.`, `..`, `../art`, `*`. In none was `art/pic.png` (changed in place) recorded; each recorded only the fixed texts below it. By reading `choose`: a pattern that is literally a place goes through the same filter; `--keep` alone widens it, as ruled.
- **`--copy before` with no middle version** (fix 4): the new sentence is shown for both absent cases met here (the three-starts case and RT21-4's).
- **`resolve` stopped at each of its writes** (`rt22_crash.rs.txt`): `resolve a.txt pic.png` into the combined save and as a save of its own (8 fault points each), and `resolve pic.png a.txt --keep theirs` both ways (15 each), with an unsaved `u.txt` and a picture changed in place. After every stop: `u.txt` untouched, `status` ran, `.opalro/tmp` empty after it, running the command again finished it (or said there was nothing left), `a.txt` was no longer in conflict, and `undo` gave back the folder byte for byte, the picture's unsaved bytes included.
- **`tidy` past the undo window** (`rt22_tidy.rs.txt`, `tidy` in-process with the clock 40 days ahead and a 1-day window, `verify` in-process at the same clock): kept conflicts after the other line was deleted; a text half fixed and saved twice; a safety copy made by `--keep` over unsaved work; a conflict only a shelf reaches, after both lines that reached it were deleted. `verify` found nothing missing; `--copy mine|theirs|before` worked for each kept conflict, and `--keep theirs` gave theirs's bytes. (The shelf could not be brought back onto the other line, with the documented refusal.) Note for whoever repeats this: `verify` run at the real clock after a `tidy` at a clock ahead reports one or three objects missing, with or without conflicts (the control test shows it). That is the faked clock, not a bug: at the same clock it reports nothing.
- **Three common starting points that disagree** (`rt22_starts.rs.txt`): lines `ax` and `by` changed `f.txt` differently, `cz` moved it to `h.txt` and removed `g.txt`; `l1` combined `by` and `cz` keeping its own, `l2` combined `ax` and `cz` keeping theirs, then `l2` into `l1`. Every version either line held is in a file of the result or a side of a conflict.
- **Many conflicts in one folder** (`rt22_slow.rs.txt`): linear in time and memory up to 40,000 distinct wide records; one record named from 40,000 places costs under a second in every command. No command reads a record so often that it is slow beyond the per-record read.
- **What the narrowing could have broken** (fix 2), by reading: `Summary` is keyed by record ID, and a record `save` rewrites gets a new ID, so no summary can go stale against it within a command. `terms` is the decoded width, which `--keep` and `--copy` use to refuse more than three versions; the kind comes from the stored term.
- **Git's words**, by reading `git_words.rs`: `checkout --ours|--theirs` and `restore --ours|--theirs` explain whatever else is typed; `merge --abort`, `--continue`, `mergetool`, `add`, `rebase`, `cherry-pick` explain; `merge -X ours side` and `merge -s ours side` count two non-options and explain (`MERGE_MANY`); `merge --strategy=ours side` goes to `combine`, which refuses an option it doesn't know.

### Memory and time after the fix

250,000 planted conflict entries in 250 folders of 1,000, each naming its own record of 255 versions (43 MB of store), against the same save of ordinary files (4 KB), debug build, peak resident memory from `/proc`, cap 3.5 GB (`memory-250000.txt`):

| Command | Files | Wide records | Before the fix (RT21) |
|---|---|---|---|
| `status` | 86 MB, 1.9 s | 196 MB, 26.8 s | aborted at 3,075 MB, 44 s |
| `goto main` | 80 MB, 5.3 s | 95 MB, 70.4 s | 95 MB |
| `show` | 14 MB, 2.0 s | 14 MB, 2.6 s | not measured |
| `changes --save main` | 14 MB, 6.5 s | 13 MB, 2.0 s | not measured |
| `resolve` | 54 MB, 0.8 s | 196 MB, 27.5 s | not measured |
| `tidy` | 13 MB, 0.2 s | 111 MB, 27.4 s | not measured |
| `verify` | 11 MB, 0.2 s | 336 MB, 95.5 s (exit 1: the planted versions are not in the store, as planted) | not measured |
| `save` | 16 MB, 0.8 s | 182 MB, 27.7 s | 3,107 MB |

Many conflicts in one folder (`slow.txt`), each its own record of 255 versions:

| Entries | `status` | `goto main` | `resolve` | `save` |
|---|---|---|---|---|
| 10,000 | 72 MB, 1.2 s | 73 MB, 3.3 s | 71 MB, 1.2 s | 73 MB, 2.5 s |
| 20,000 | 84 MB, 2.0 s | 83 MB, 7.4 s | 84 MB, 2.0 s | 85 MB, 4.6 s |
| 40,000 | 101 MB, 3.7 s | 101 MB, 15.4 s | 101 MB, 3.6 s | 106 MB, 9.0 s |
| 40,000, one shared record | 36 MB, 0.4 s | 40 MB, 0.8 s | 36 MB, 0.3 s | 39 MB, 0.7 s |

## Not covered

- `combine` racing `resolve` (two commands at once).
- A conflict below a folder moved or renamed on the other line: not run this session.
- Three starting points in every order of their IDs (I could not choose the IDs from the command line).
