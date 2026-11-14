# 169 — Saves reached through a combine: finding them, and what `history` shows

- **Status:** Accepted
- **Date:** 2026-11-14
- **Agenda topic:** none (milestone 8 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 055](055-lines-in-v0.1.md) (what `goto` takes); [decision 105](105-the-change-id-cache-keeps-each-lines-saves-in-order.md) (a save is found by its ID along the line's first parents); [decision 165](165-combine-the-command.md) (`history` lists a combined save as one save)

## Context

**In plain words.** After `opalro combine wall-run`, the line `main` has one new save that holds all of `wall-run`'s work. The saves made on `wall-run` are still there, behind that combined save, as its second parent's history. If `wall-run` is then deleted, that is the only way left to reach them.

In v0.1 nothing looks there. `history` follows first parents only, and so does finding a save by its ID: `opalro show <id>` says "no save matches" for a save reached only through a second parent. Only a Git import made such saves, so that was a known limit. Now `combine` makes them every day.

## Options

**What `history` lists after a combine**

- **A. The line's own saves,** with the combined save as one row that says what it brought in, and a way to list those saves when wanted (Git's `log --first-parent`; Fossil's and Plastic's default views are per-branch too).
- **B. Every save reachable,** both lines' saves mixed by date (Git's `log` default). Complete, and hard to read: saves from two lines of work alternate with no sign of which is which.
- **C. A drawn graph** (Git's `log --graph`, Jujutsu's default). The best view, and a much larger piece of work.

## Decision

**A.**

- **Finding a save by the start of its ID** (a change ID or a fingerprint, wherever a save is asked for) looks in two steps:
  1. on the line the project is on, along first parents, as today. One save found there is the answer. This keeps every short ID that `history` prints working, since they are shortened to be unique on the line.
  2. only when the line has none: among every save that any line, server line or tag reaches, through every parent, and the save the project is at. One is the answer; several are listed, as today.
- **`opalro history`** lists the line's saves along first parents, as today. A save with more than one parent says so at the end of its row, `[combined <id>]` for each other parent, by the first 12 hex digits of its fingerprint.
- **`opalro history <line or save>`** lists from another place: a line, a tag, a server line, or a save by its ID, such as the one a `[combined …]` mark names. There the saves' change IDs are shown 12 letters long, since shortening them to be unique would mean reading the whole list first.
- **`opalro show`** on a combined save names each other parent on a `Combined` row under `After`, and compares with the first parent, as today.
- **`--json`:** every save object gains `parents`, the fingerprints of its parents in order.

## Consequences

- The second step reads every save the project's lines and tags reach, once, and only for an ID the line doesn't have: a mistyped ID in a very large project costs that walk. It is measured on Git's own history; a faster way (the packs' own indexes already find a fingerprint by its start) is a later step if the numbers ask for it.
- The check that a line's or tag's name isn't also the start of another save's ID (decision 068) uses both steps, or a line could pose as a save a combine brought in. So a line named only with the letters k to z (`work`, `story`) costs the second step's walk every time it is named. That is the price of the check; a table of every reachable save's change ID would remove it, if the measurements ask for one.
- A save that only a shelf or an undone step still holds isn't found by its ID, as today: those are reached through `opalro shelf --list` and `opalro undo --list`.
- A drawn graph of lines stays for later, with the interactive viewer (design.md §7).
- The amend check still follows first parents; milestone 10 changes it (decision 157).
