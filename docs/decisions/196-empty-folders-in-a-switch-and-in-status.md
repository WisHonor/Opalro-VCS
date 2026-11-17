# 196 — Empty folders in a switch and in `status`

- **Status:** Accepted
- **Date:** 2026-11-17
- **Agenda topic:** none (milestone 11; the owner put it first in the next version, [decision 189](189-the-owners-review-of-v0.2.md) point 5: "one decision for both, since they share a cause")
- **Decided by:** the owner, that both change; Claude, while the owner was away, how. The owner can reverse any part.
- **Refines:** [decision 004](004-project-contents.md) (empty folders are part of a save), [decision 170](170-what-building-milestone-8-refined.md) (a combine's empty folders), [decision 177](177-recording-a-fix-save-and-resolve.md) (`resolve --keep`), [decision 179](179-every-command-and-a-conflict.md) (`status` and conflicts), [decision 188](188-red-team-m10-rulings.md) (RT24-1)

## Context

**In plain words.** Opalro keeps empty folders: a save holds them, `goto` makes them, `discard` throws a new one away and puts a deleted one back. Two things don't fit:

- **`status` doesn't see them.** Delete a saved empty folder, or make a new one, and `status` says nothing changed. Yet `opalro save` then makes a save ("0 files") that takes the folder out or puts it in, and `discard` says it threw away changes "to 1 folder". `status` is the one command that is blind to them (the research: `diff::file_change` gives a folder no change, and `status` and `changes` keep only changes).
- **A switch can make a folder that should not be there** (the red team's RT24-1). Your line moved or deleted a folder, the other line changed a file in it, and you combine: the combined save holds that folder with one place in conflict, and the place stands as nothing, since your line has nothing there. The switch reads the folder as it stands, finds it empty, and makes it. You get an empty folder neither line has, and `status` (once it sees empty folders) would show it as a change you never made.

They share a cause: a folder that *stands* empty is treated like a folder *saved* empty. The two can be told apart: a folder saved empty is stored as the empty listing, the same in every project; a folder that holds only places in conflict standing as nothing is stored as a listing with those places in it.

A third piece: `opalro resolve <path> --keep mine`, where your version is nothing, leaves the folder in the new save, empty, though it isn't on disk (`overlay_whole` with `drop_emptied: false`); `resolve` without `--keep`, and a save of some paths, take out a folder they empty that is gone from disk (design.md §5.1). Fix only the switch, and `status` would then show RT24-1's folder as removed.

## Options

- **A. Make a folder only when something is written into it.** Simple, but it stops making folders saved empty on purpose, and folders that hold only project links or names this computer can't write, which are made today.
- **B. Tell the two apart.** A switch makes a folder the target holds when it is saved empty, or when anything below it is written or kept as it is; a folder whose contents are only places in conflict standing as nothing is not made. `status` shows empty folders saved or new. `resolve --keep` follows the save's rule for a folder it empties.
- **C. Leave the switch, and have `status` show only folders saved empty.** RT24-1's folder stays, unexplained.

## Decision

**B.**

1. **The switch** (every command that writes the folder: `goto`, `combine`, `undo`, `redo`, `shelf`, `unshelf`, `discard`, `resolve --keep`, `setup --preset`) makes a folder that the target save holds and the disk lacks only when one of these is true:
   - the save holds it as an empty folder (the empty listing, as stored);
   - something below it is written, or kept as it is (a project link, a name this computer can't write);
   - it stands where the disk has a file, so that the file goes.

   A folder whose contents, all the way down, are places in conflict that stand as nothing is not made. Nothing else about what a switch writes, keeps or removes changes.
2. **`resolve --keep`** takes out of the new save a folder it empties that is gone from disk, as `resolve` and a save of some paths do. A folder it empties that is on disk stays, as an empty folder.
3. **`status` shows empty folders** that differ between the folder and the save it is on: a folder the save holds empty that the disk doesn't have (`removed`), and an empty folder on disk that the save doesn't have (`added`). Only the innermost is named: a new `a/b/` with `b` empty is one row, `a/b`. A folder that holds only files the ignore rules leave out is an empty folder to a save, so it is shown too. A folder replaced by a file of the same name, or the other way round, shows the file as today.
   - **Its words:** a row as a change's, `added` or `removed`, the path, and `empty folder` where a file's size goes. The rows are among the changes, in path order. The count before them counts files, and folders only when no file changed (`Changed since then (1 folder):`), as `discard` and `undo` count. `Nothing changed since then.` needs no folder changed either.
   - **Its `--json`:** a new field, `empty_folders`, a list of `{"path", "change"}` with `change` `"added"` or `"removed"`, `[]` when none. A field is added and none changed, so `opalro_json` stays `1` ([json.md](../json.md) §1); `changes` keeps listing files only.
4. **`opalro changes`, `show` and `changes --save` stay as they are:** they show what changed inside files, and an empty folder holds none.

## Consequences

- RT24-1's proof (`redteam24.rs`) runs again and passes, and the two tests that pinned the old switch (`resolve.rs`, `in_a_deleted_folder` and the test after it) say what happens now.
- The model test (`model.rs`) learns both rules: its switch leaves out folders that hold only conflicts standing as nothing, and its `status` lists empty folders.
- [terminal.md](../terminal.md) (`status`), [json.md](../json.md) (`status`), [guide.md](../guide.md), [design.md](../design.md) §5.1, §5.5 and §5.9, and the release page's known problem ("An empty folder can appear after a combine") follow.
- The guide and terminal.md say a file that became a folder shows as `kind changed`, `file → folder`; the code never gives a folder a change (the research, not yet run). The milestone checks it, and the pages are made to say what the code does.
