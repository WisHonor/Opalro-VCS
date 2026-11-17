# 194 — A change ID's start counts from four letters

- **Status:** Accepted
- **Date:** 2026-11-17
- **Agenda topic:** none (milestone 11; the owner put it first in the next version, [decision 189](189-the-owners-review-of-v0.2.md) point 5)
- **Decided by:** the owner, that it is done ("text counts as the start of a change ID only from four letters on"); Claude, while the owner was away, how: the words for a text too short, and what else it changes. The owner can reverse any part.
- **Refines:** [decision 055](055-lines-in-v0.1.md) (how a target is read), [decision 068](068-a-goto-target-that-names-both.md) (a target that names both a line and another save), [decision 159](159-tags.md) (a tag's name that is also the start of another save's ID), [decision 161](161-what-building-milestone-7-refined.md) (its first open item)

## Context

**In plain words.** Every save has a change ID made of the letters k to z, and you can type the start of one to reach the save: `opalro goto wxyn`. Lines and tags have names you choose. When a name you typed could be both, Opalro refuses and asks you to say more (decision 068). Today one letter counts as the start of a change ID, so a line called `x`, `try` or `tmp` can clash with a save whose ID happens to start with those letters, about one save in sixteen for a one-letter name. Then `opalro goto x` stops, though you meant your line. Opalro never shows a change ID shorter than four letters, so nobody needs a shorter start.

What the code does today (the research, `history.rs`): one function, `history::find`, reads every typed target for `goto`, `show`, `changes --save`, `who --save`, `tag --save`, `history <from>` and `combine`; `is_change_start` takes any text of one or more letters k to z; what `history` prints is never shorter than `MIN_SHORT`, 4 letters, and `--json`'s `change_id_short` neither. No test types a shorter start.

## Options

- **A. Four letters, the shortest Opalro prints,** with today's words when nothing matches. One line of code.
- **B. Four letters, and words that say why** when a text of one to three letters k to z names nothing: someone used to Jujutsu, which highlights the one or two letters that are enough there, will type them here.
- **C. Leave it,** and keep refusing such names as meaning both. The owner has ruled it changes.

## Decision

**B.**

1. **Text counts as the start of a change ID only when it is at least 4 letters long,** all of them k to z, as before. Shorter text is a line's, a server line's or a tag's name, or nothing. One rule, `MIN_SHORT`, serves what Opalro prints and what it reads.
2. **So a line, server line or tag named with 1 to 3 letters is always that line or tag.** It is never refused as also meaning a save (decisions 068 and 159), and naming it no longer walks the history to look for a save it might mean. A name of 4 letters or more made only of k to z (`work`, `tops`) is read as before: it can still meet a save's ID, and is refused then.
3. **When a text of 1 to 3 letters, all k to z, names nothing,** the error is today's, `No save, line or ID matches "<text>".`, and the hint says why: `A save's change ID is typed with at least 4 letters; see your saves with: opalro history`. For every other text that names nothing, the hint stays `See your saves with: opalro history`.
4. **The start of a fingerprint stays at least 8 hex digits,** and `undo --to` keeps its own rule for diary entries.

## Consequences

- An ambiguous start can no longer list a sixteenth of a long line's saves: `goto k` named every save whose ID starts with `k`, with no limit on the list. From four letters, a match is rarely more than a few saves.
- `--json` changes only in the `hint`'s words, which [json.md](../json.md) §1 leaves out of the freeze.
- [terminal.md](../terminal.md), [json.md](../json.md) (where a target is read), [guide.md](../guide.md), [design.md](../design.md) §5.10 and §7 (the deferred item closes), [security.md](../security.md) T21 and T50, and [benchmarks.md](../benchmarks.md) §12 (the walk a short k-to-z name paid) say so.
- Decision 161's first open item is closed by this one; decision 068's consequence about the walk's cost now holds only for names of 4 letters or more.
