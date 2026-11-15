# 173 — The rules of both lines decide what a combine merges by itself

- **Status:** Accepted
- **Date:** 2026-11-14
- **Agenda topic:** none (milestone 8, its whole-branch review)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 167](167-merging-text-line-by-line.md) (the rules that decide whether a file is merged line by line were the folder's own when the combine starts); [decision 172](172-red-team-m8-session-2-rulings.md) (a rules file that can't be read whole stops automatic merging; a moved file's rule is asked under both its names)
- **Refined by:** [decision 176](176-a-conflict-in-the-folder.md) (markers only when both lines' rules allow it)

## Context

**In plain words.** A rule in `opalro.toml` can say that a kind of file is never merged line by line: Unity scenes and prefabs, for which a merge that looks clean can give a file that no longer loads. Decision 167 had a combine take those rules from the folder, that is, from the line you are on.

The whole-branch review of milestone 8 showed where that falls short. Line `art` adds the rule for `*.unity` (or switches to the Game preset) and edits `Main.unity`. Line `main` doesn't have the rule yet and edits the same scene somewhere else. Combining `art` into `main` reads the folder's rules, finds none for scenes, and merges the scene line by line, without a word. The combined save then holds the very rule that should have stopped it. Combining the other way round (`main` into `art`), the folder has the rule and the scene is not merged. The same two lines give two results.

It is the harm decision 172 closed for a rules file that can't be read (a protected scene merged silently), by another route: there a line could switch protection off for the next combine; here a line's new protection is not heard by the first one.

## Options

- **A. The folder's rules only,** as decided. Git does the same: it reads `.gitattributes` from the working folder, which is the branch being merged into. Simple, and the result depends on which line you stand on.
- **B. Both lines' rules.** A file is merged line by line only when the folder's `opalro.toml` and the one in the save being combined in both allow it.
- **C. Those two, and the rules where the lines parted.** A rule both lines have since dropped would still count. It adds little and keeps old rules alive.

## Decision

**B.**

- **A combine reads two rules files:** the folder's, as before (the copy in the folder, saved or not), and the `opalro.toml` in the top folder of the save being combined in. That one is read under the same terms as the folder's: a regular file of at most 1 MiB, parsed by the same code; a link, a folder or a larger file there is passed over whole; no file means no rules.
- **A file is merged line by line only when both allow it,** under each of its names (decision 172).
- **If either wasn't read whole, no file is merged line by line** (decision 172's rule, now for both). The clash and the hint say which of the two it is: the folder's own, which `opalro setup` explains, or the other side's, which is fixed there.
- **The words name the other side when the reason is only there:** `opalro.toml on <target> says conflict-markers = false for it`, and `opalro.toml on <target> can't be read whole, so no file is merged line by line`.
- **The starting points' own combines** use the same two files.
- **The warning after a combine** stays about the file the combine leaves in the folder.
- **Once conflicts are kept** (milestone 9), a file in conflict is shown with markers only when both allow it too.

## Consequences

- Combining `art` into `main` and `main` into `art` treat a file alike.
- A protection added on one line holds from the first combine that meets it.
- A protection taken away on one line keeps holding against lines that still have it, until they have the change too. Opalro then merges less than it could, never more.
- A rules file with a mistake in it on the other side stops automatic merging as one in the folder does, and the person has to go to that line to fix it.
- Opalro differs from Git here on purpose.
- It is built with the whole-branch review's other fixes, before milestone 8 is merged.
