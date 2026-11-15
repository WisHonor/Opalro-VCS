# 183 — Red team on milestone 9, second session: a folder kept by `resolve --keep` is told by its path

- **Status:** Accepted
- **Date:** 2026-11-15
- **Agenda topic:** none (milestone 9, its second red-team session, RT22)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 177](177-recording-a-fix-save-and-resolve.md) (how `--keep` puts a folder version into the save); [decision 182](182-red-team-m9-rulings.md) (the first session's rulings, attacked here; the memory a command holds, now measured after the fix)

## Context

The second red-team session on milestone 9 (RT22, [report](../redteam/2026-11-15-milestone-9-session-2.md)) attacked a copy of `milestone-9` at 76ba7bb in the no-network sandbox: the four fixes decision 182 ruled, and what the first session had no time for.

**What held.**

- **The four fixes.** `--only-if-clean` with nothing to combine: no way was found to end with 0 on a line whose save holds a conflict (by reading the code). A folder's path given to `resolve`: 17 spellings of it were run (a trailing `/`, `./`, `/.`, `..`, `//`, from a folder below, with `--yes`, with `--json`, mixed with patterns), and none recorded the picture changed in place. `--copy before` says its new sentence in both cases.
- **The memory a command holds, measured for the first time on Linux after the fix** (peak, debug build, the red team's planted records of 255 versions each):

  | | `status` | `save` | `resolve` | `goto` | `tidy` | `verify` |
  |---|---|---|---|---|---|---|
  | 20,000 records | 84 MB | 85 MB | 84 MB | 83 MB | | |
  | 250,000 records (43 MB of store) | 196 MB | 182 MB | 196 MB | 95 MB | 111 MB | 336 MB |
  | 250,000, before the fix | stopped at 3,075 MB | 3,107 MB | | 95 MB | | |

  The first session's proof (20,000 records under 120 MB) passes, in the sandbox and on GitHub's Linux machine. Time grows in a straight line with the number of conflicts; no command reads a record so often that it slows down sharply.
- **`resolve` stopped at each of its writes,** four ways, 8 or 15 points each: unsaved work was never touched, running the command again finished it, `undo` gave the folder back byte for byte, and nothing was left in `.opalro/tmp`.
- **`tidy` past the undo window:** every version of the kept conflicts was still there after the other line was deleted; so was a text half fixed and saved twice, and a conflict only a shelf reaches.
- **Three common starting points that disagree:** no version lost.
- **Git's words:** each explains; none acts on a conflict.

**RT22-1 (Low): `resolve --keep` can turn a link nobody named into a folder.** In plain words: to take a side that is a whole folder, `resolve --keep` carries that folder through one step of its work dressed as another kind of entry, and afterwards turns back every such entry that names the same contents. Opalro never makes two entries like that, so in a project made by Opalro nothing else is touched. But in a planted project, a link beside the conflict that was made to name the same contents becomes a folder too, with the other side's files in it, at a place the person never named. Nothing is lost and nothing leaves the project. The whole-branch review, asked by decision 182 to read this code, found the same.

**Two things noticed without proof.**

- `goto` to a save of 250,000 planted conflicts took 70 seconds, where the same save of plain files takes 5; `status` takes 27. It grows in a straight line and looks like several reads of each record where one would do. The parts were not timed.
- Where the other line holds no file at a place (it moved it there), the combine's row still says the file was "changed on both lines". The first session noted the same looseness for moves.

**Not reached:** `combine` and `resolve` at the same time; a conflict below a folder the other line moved; three starting points in every order of their IDs.

## Options

- **A. Tell the dressed-up entry by its path.** `resolve --keep` knows the places it is keeping; only those are turned back.
- **B. Leave it:** only a planted save reaches it, and what it gets is a damaged link turned into a healthy folder. But "every project is potentially hostile" is a rule here, and a command that changes a place nobody named is the kind of thing that rule is for.

## Decision

**A.**

**1. A folder version that `resolve --keep` puts into the save is told by the path it is kept at, never by what it names.** No entry at another path changes. The red team's proof (`keeping_a_folder_side_changes_no_other_entry`) joins the tests.

**2. The measure in decision 182 is made exact** (its "Corrections and additions"): a command's own memory for a record is small and fixed; the store's memory of whole records keeps its budget; the numbers above are what that gives.

**3. No third session,** as in milestones 7 and 8: the fix is a few lines, is built with the whole-branch review's fixes and re-reviewed with them. What neither session reached goes onto the follow-ups list and into the brief for milestone 10's red team.

**4. `goto` on a project of very many conflicts** is timed part by part before anything is said about why it is slow or changed; until then it is a row on the follow-ups list with the two numbers.

## Consequences

- `resolve --keep` on a folder changes exactly the places it was asked to.
- Two red-team sessions on conflicts found five things, none High: a promise with an exception, memory without a bound, a folder's path that decided too much, a sentence that was wrong in one case, and this. Each is fixed.
- The planted project that stopped `status` at 3 GB now takes 196 MB, where the same save of 250,000 plain files takes 86 MB (the first session's table). The difference is bounded by the store's budget for whole records.
