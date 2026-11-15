# 181 — What building milestone 9 refined

- **Status:** Accepted
- **Date:** 2026-11-15
- **Agenda topic:** none (milestone 9 build)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 174](174-the-conflict-record.md) (how often a record is read, and how many are remembered); [decision 175](175-what-a-combine-keeps-as-a-conflict.md) (what `--only-if-clean` promises; when a held conflict stops a combine; starting points that disagree; the check below a folder); [decision 176](176-a-conflict-in-the-folder.md) (a label's long name; what `status` reads); [decision 177](177-recording-a-fix-save-and-resolve.md) (renames, names and `save <paths>`; what `resolve` takes, records and refuses; where `--copy` may write); [decision 178](178-the-conflicts-feature.md) (what `setup --upgrade` offers); [decision 179](179-every-command-and-a-conflict.md) (what `status`, `goto`, `show` and `combine` say); [decision 004](004-project-contents.md) (new names are judged against a conflict's name too); [decision 116](116-gits-words-as-shortcuts.md) (how long an explanation of a Git word is); [decision 117](117-what-json-covers-at-v0.1.md) (when a `--json` field may take a new value); [decisions 172](172-red-team-m8-session-2-rulings.md) and [173](173-the-rules-of-both-lines-decide-what-a-combine-merges.md) (which files the rules are asked about)

## Context

Decisions 174 to 179 were written before any of milestone 9 was built. Building and reviewing it, task by task, showed sentences in them that were wrong, too narrow, or silent. Each was ruled on as it came up, with its reason and its cost ([the milestone's rulings](../plans/2026-11-14-milestone-9-rulings.md), "While building"). This record collects what changed, so that the decisions and the code agree again. Where this record and one of them differ, this record is right. One case had a record of its own first: a file that arrived where the other line changed the file that was there ([decision 180](180-a-file-that-arrived-where-the-other-line-changed-the-file.md)).

**In plain words.** A conflict is a place in the project where two lines disagree and nobody has decided yet. The whole milestone rests on two promises: no version anyone saved is ever lost, and Opalro never decides a conflict for you. Most of what follows are corners where one of those promises would have been broken, each found by a reviewer working cases through by hand, by a test that compares one way of doing something with another, or by the tests running on Linux and macOS. None reached a person.

The most serious, so the reader knows where this code has been wrong:

- **`save <paths>` could drop a conflict nobody had settled**, twice: a picture in conflict inside a folder that had been deleted, and a conflict standing as a folder that was named directly. Both were closed by one test that compares a save of some paths with a full save at every place.
- **With three or more common starting points, a removal could be lost**, depending on the order of the saves' IDs: what the first two starting points could not agree on was forgotten when the third was combined in.
- **`opalro combine --only-if-clean` could succeed on a save that held conflicts**, when the conflicts came from the lines and not from this combine, or when the line only moved forward.
- **`opalro resolve` could record a file it had not looked at**: it decided from one read of the folder and recorded from a second.
- **Renaming a picture in conflict by letter case alone** made a save that held two names one drive cannot tell apart, after which a change to the file was never saved.

Two smaller faults were found last, by the model of the whole command line once it kept conflicts by its own rules: a file moved to two places kept a conflict even where both lines held the very same file, and under `--only-if-clean` one kind of place was named wrongly.

## Decision

**The record (decision 174)**

- **A command reads each record once**, whichever of its steps meets it: the project's store remembers the records a command has met. It remembers them only while the versions they name stay within 1,048,576 in all; past that a record is read again when it is met again. An honest project never gets there.

**What a combine keeps (decision 175)**

- **`--only-if-clean` promises that the save the line ends on holds no conflict.** It stops, changing nothing, for a conflict this combine would make, one either line already holds and the combine would carry, and, when the line would only move forward, one the target's save holds. Decision 175 said it stops "where a conflict would be kept". For a forward, Opalro looks through the target's save once, only with this flag and only in a project that keeps conflicts: the one place where it looks through a save for conflicts, because the person asked. A target that cannot be looked through (past a save's limits) is refused with the flag, even if it holds no conflict.
- **A project that can't keep conflicts is always told so**, with or without the flag: the flag's hint ("run it again without it") would be untrue there.
- **A place that a line already holds in conflict stops a combine only in a project that keeps conflicts.** In a project made by Opalro 0.1 and not upgraded, such an entry (which only a project made outside Opalro can hold) is treated as milestone 8 treated it.
- **Starting points that disagree: what they could not tell stays unknown through every later combine of them**, and a place unknown on the side made so far is never read as that side having removed it. Decision 175 described two starting points; with three or more, this is what keeps the answer the same whatever the order of the saves' IDs.
- **Where the starting version holds a conflict and both lines hold a folder there, everything below is unknown**: a file only one line holds below is a conflict, never an addition.
- **Before a folder becomes one side of a conflict, it is checked for a conflict below, once per combine**, and all such checks of one line together are held to that line's limits.
- **A place both lines hold alike is never a conflict, at the places a moved file went to as anywhere else.** A file moved to two places, where both lines hold the very same file at one of them, keeps a conflict only at the other. At a moved file's old place the move rules still come before "what both lines hold alike is the result", as milestone 8 decided on purpose.
- **Where ours holds something of its own at the place theirs moved a file to, the conflict there is the moved-onto one**: yours, nothing, theirs, since the place held nothing where the lines parted.
- **A place whose starting version nobody can tell is named so under `--only-if-clean` too**: the row is about the place, not about whether this combine keeps conflicts.

**A conflict in the folder (decision 176)**

- **In a marker's label a long name is cut, never the save's letters**: the name is cut between characters, the cut is shown with `…`, and ` (<8 letters>)` is always whole, within 200 bytes.
- **`status` reads each file that still has markers every time it runs**, to count the places left, and takes the file's ID and its markers from one read. The promise that a second `status` opens no file holds for conflicts that stand as a version (a picture, a scene).

**`save` and `resolve` (decision 177)**

- **A conflict does not follow a rename.** A rename is a removal and an addition: a file with markers that is renamed is fixed by its removal, and the new name is an ordinary file that still has the marker lines; a picture in conflict that is renamed stays in conflict at its old name, with nothing in the folder there.
- **A conflict that a save keeps where the folder holds nothing still holds its name**, and new names are judged against it as against every other name in the folder ([decision 004](004-project-contents.md)): a new name that differs from it only in upper and lower case, or is the same name written another way, is refused, on every system.
- **`save <paths>` settles every place it names exactly as a full save would**, and leaves every other place as it was.
- **A conflict kept inside a folder you deleted, where a file now has that folder's name, stops the save** in words of its own.
- **`resolve` takes what `save` takes: paths and patterns.** A pattern stands for every place in conflict it matches. What was typed is a path first: where it is, letter for letter, a place in conflict or a folder with places below it, it means that path and nothing else; only otherwise is it read as a pattern. (A file can really be named `a[1].png`.)
- **`resolve` records only what it looked at.** A file that changes between its two reads of the folder stops it, with `save`'s own words for a file that changed while it was read.
- **The engine itself refuses to keep a side without places**, and "where the lines parted" is not a side that can be kept.
- **`--copy` never writes into `.opalro`, told by which folder the destination is, not by how its path is spelled**, as far as safe code can tell: on Unix by the folder's identity, on Windows by the system's own identity for a folder. Not caught: a second route to the same disk that the system reports as another folder (a share from another machine, a network mount of the project). The destination's folder must be readable.
- **A save that `--keep` makes is held to a save's limits** like any other.
- **The default message is `Fixed conflicts in N places`**, not "N files": a place can be a folder.
- **Under `resolve`, a refusal that comes from `save`'s own checks speaks of `resolve`**: "Nothing was recorded", and "run opalro resolve again".

**The feature (decision 178)**

- **`opalro setup --upgrade` never offers `chunks`**: that feature is added by itself by any command that changes the project (decisions 083 and 121), so a question whose "no" changes nothing is not a question.

**Every command and a conflict (decision 179)**

- **`status` ends with one `Next:` line**, the first that applies: record what you fixed; else fix the marked places; else pick a version.
- **`goto` says the save it went to holds a conflict whenever it met one there**, whatever the conflict stands as; a combine that only moved the line forward says the same.
- **A combine that made no conflict but carries some says so**: the outcome gives the count, and a detail says the two lines held them already.
- **In `show` and `changes --save`, a folder that became a conflict's place, or a conflict's place that became a folder again, is one row** with nothing below it: the conflict holds the folder whole.

**Rules in `opalro.toml` (decisions 172 and 173)**

- **Two files that both lines added with the same contents, differing only in the executable mark, are not a question for the rules**: there is nothing to merge line by line. A rules file that can't be read whole no longer stops a combine for such a pair.

**Git's words (decision 116)**

- **An explanation of a Git word is one to three lines**, not "two or three". The one-time note for `merge` has a second line: a combine keeps conflicts and still ends with 0; `--only-if-clean` stops instead.

**`--json` (decision 117)**

- **A field whose values are a fixed list of codes takes a value an older Opalro never gave only in a project whose format has a feature that older Opalro did not know.** New fields and new kinds of warning may appear in any project, as before. A field that holds a name (a command's name in the diary, a path, a message) is not such a list: a newer Opalro's commands appear there by name.

## Consequences

- The six planning decisions keep their text; each has a "Refined by" line pointing here.
- `docs/terminal.md`, `docs/json.md`, `docs/security.md` (T43, T46, T47, T52 to T54) and the guide say what this record says; the rulings file gives each ruling with its reason and its cost.
- Things the owner may want to weigh are in the rulings file's "Please confirm" list: that a conflict does not follow a rename; what `--only-if-clean` promises; that `resolve` takes patterns; what `--copy` does not catch on a second route to the same disk; that `status` reads marked files on every run.
- One thing is known and not settled by this record: each command keeps its own map of the records it has read, without a limit of its own, so a planted save of very many conflicts, each naming a different record of the largest size, could make one command hold far more than an ordinary save of that many entries. It is handed to the red team to measure.
