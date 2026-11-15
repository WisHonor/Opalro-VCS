# 172 — Red team on milestone 8, second session: rules that can't be read stop automatic merging

- **Status:** Accepted
- **Date:** 2026-11-14
- **Agenda topic:** none (milestone 8, its second red-team session, RT20)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 114](114-bolt-toml-keys-and-how-bolt-reads-it.md) (an `opalro.toml` Opalro can't use is passed over with a warning and Opalro's own values apply); [decision 167](167-merging-text-line-by-line.md) (a file's rule decides whether it is merged line by line; the rules are the folder's own when the combine starts); [decision 171](171-red-team-m8-rulings.md) (the first session's rulings, attacked here)

## Context

The second red-team session on milestone 8 (RT20, [report](../redteam/2026-11-14-milestone-8-session-2.md)) attacked a copy of `milestone-8` at 987a2e9 in the no-network sandbox: the fixes decision 171 ruled, and what the first session had no time for.

**What held.**

- **The fixes.** The search for common starting points gave a brute-force answer on 6,000 queries over 1,500 planted histories, under five kinds of clock; a history with 1,024 true starting points was refused with that number, in 27 ms. The budget of move notes is charged before a save's notes are read, a save that doesn't fit drops that line's notes whole, and the warning is given whenever notes went unbelieved. The read of a save without its notes runs the same checks as the full read.
- **A combine stopped at every write** (16 points for a combined save, 14 for a line that only moves forward): unsaved work was never touched, running the combine again finished it with the same folder, `undo` gave back exactly what was there, and nothing was left in `.opalro/tmp`.
- **`tidy` after a combine,** with the other line deleted and the undo window passed: every save the combined save reaches through its second parent was kept.
- **A save with 1,024 parents** in `show`, `history`, `status` and `combine`.
- **v0.1 on a project this build made, and back:** v0.1 read a combined save and a save with move notes, went to both lines, undid, redid, tidied and saved; this build then verified and went on.

**RT20-1 (Medium).** `opalro.toml` can say that a kind of file is never merged line by line (`conflict-markers = false`: Unity scenes, prefabs). When the file can't be used at all (it isn't valid TOML, is too large, is a link), decision 114 has Opalro pass it over and use its own values, with a warning. A combine did that too, but its own value for every file is "merge it", and it gave no warning. So a line that changed nothing but `opalro.toml`, into a file that can't be read, combined in cleanly and silently; and the next combine merged a scene line by line against its rule, silently again. A scene merged that way can load broken or not at all.

The same hole is there in small: when Opalro can use the file but not one part of it (a rule with a mistake in it), that part is passed over, and a combine merged the files it was meant to protect.

**Two things noticed without proof.**

- A file's rule is looked up under the name it has in the result. A scene renamed on one line to a name no rule covers, and edited on both, would be merged line by line. Git looks its attributes up the same way.
- Move notes are read along a line's first parents only. A note on a save that came in through a combine is not used; the combine falls back to following by contents, as [decision 168](168-moves-in-saves-and-in-combines.md) says.

**Not tried, for lack of time:** two commands at once; a reshuffle past the bound on following moves; a comparison that runs out of its two seconds; text files at the size and line limits; a rules file changed on both lines; true notes moving 100,000 files back and forth over many saves.

## Options

**When the folder's rules can't be read whole (RT20-1)**

- **A. Warn.** The combine gives the warning `save` gives. The merged result is stored and written before anyone reads it.
- **B. Merge no file line by line.** While `opalro.toml` was passed over, in whole or in part, a file both lines changed is not merged by Opalro, as if its rule said so; the combine says why. And warn.
- **C. Refuse every combine** until the file is fixed. Also refuses the combines that need no merge, the one that would bring the fixed file in among them.

## Decision

**B.**

- **A combine merges a file line by line only when the folder's rules were read whole.** If `opalro.toml` was passed over, or any part of it was, a file both lines changed is not merged: it is a clash like one whose rule says `conflict-markers = false`, with words of its own (`opalro.toml can't be read whole, so no file is merged line by line`). In milestone 8 a clash stops the combine, changing nothing; the hint points at `opalro setup`, which says what is wrong with the file. A combine that needs no such merge goes through.
- **A combine gives the rules' warning** (`rules_file_skipped` or `rules_ignored`) when the rules it started with weren't read whole, and when the `opalro.toml` it leaves in the folder can't be: the combine that brings a broken file in says so at once.
- **The two warnings say what follows:** until the file is fixed, a combine merges no file line by line.
- **A moved file is merged line by line only when the rules allow it under its old name and under its new one.** The first thing noticed without proof is closed without waiting for one: a test first, then the check.
- **Notes along first parents only stays as decided** (decision 168).
- **`docs/security.md` is corrected** where it said a hostile `opalro.toml` could at most change chunking, the numbers a save asks at, or which files are saved.
- **No third session.** The fix is small and is reviewed, and the whole-branch review reads it. What neither session reached goes into the brief for milestone 9's red team, which attacks the same code with conflicts kept.

## Consequences

- An `opalro.toml` with a mistake in it stops Opalro from merging files by itself until it is fixed: any mistake, not only one in a rule about merging, because Opalro can't tell what a part it couldn't read was for. The warning names each part.
- An `opalro.toml` written by a newer Opalro, with keys this one doesn't know, has the same effect on this one; the warning says to update.
- Decision 114's promise stands for the file itself: nothing in it stops a command, and a file that can't be used never makes Opalro do more. From here on it can make a combine do less.
- A person who breaks the rules file on one line learns it from the combine that brings it in, not from the scene that broke later.
- Once conflicts are kept (milestone 9), such a file is still a reason to stop and fix `opalro.toml`, not a conflict to keep.
- The proof test (`crates/opal-vcs/tests/redteam20.rs`) runs on every push once the fix is in.
