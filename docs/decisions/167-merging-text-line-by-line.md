# 167 — Merging text line by line

- **Status:** Accepted
- **Date:** 2026-11-14
- **Agenda topic:** none (milestone 8 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 012](012-lines.md) (text merged line by line; binary files never merged automatically); [decision 004](004-project-contents.md) (bytes in, bytes out: no line-ending conversion); [decision 007](007-conflicts.md) (presets decide which files get conflict markers; the Unity preset treats scenes, prefabs and `.meta` files like binaries); [decision 058](058-changes-line-by-line.md) (how `changes` compares lines, and what it takes for text); [decision 114](114-bolt-toml-keys-and-how-bolt-reads-it.md) (`conflict-markers`)

## Context

**In plain words.** When both lines changed the same file, Opalro can often still combine them by itself: if you edited the top of a script and they edited the bottom, the result has both edits. This record says exactly when Opalro does that, and when it gives up and calls it a conflict.

Decision 012 said text is merged line by line and binary files never. Left open: what counts as text, what happens when two edits sit right next to each other, line endings, a file too slow to compare, and which copy of the project's rules decides.

## Options

**How close two edits may be**

- **A. Edits that overlap or touch are a conflict** (Git, Mercurial and the classic `diff3` all do this). Two people editing neighbouring lines usually means they were working on the same thing.
- **B. Only edits to the very same lines are a conflict.** Fewer conflicts, and more results that neither person wrote or would have written.

**Line endings**

- **A. Lines are compared as the bytes they are,** endings included (Git's default). A file one side converted from CRLF to LF and the other side edited is a conflict over every line.
- **B. Endings are ignored when comparing** (Git's `merge.renormalize`). Friendlier in that one case, but the result's endings must then be invented, which breaks "bytes in, bytes out" (decision 004).

**A file whose rule says "no conflict markers"** (Unity scenes, prefabs and `.meta` files in the Game preset)

- **A. Never merged automatically.** They are text, but a merge that looks clean line by line can still break the references inside them, which is why Unity ships a merge tool of its own.
- **B. Merged when it comes out clean,** and only spared the markers when it doesn't.

## Decision

**A, A, and A.**

- **What is merged as text:** a file both sides changed since the starting point, where all three versions are files, none has a zero byte in its first 8,000 bytes, each is at most 64 MiB and 1,000,000 lines (the limits `opalro changes` uses, kept in one place), and the path's `conflict-markers` rule isn't `false`. Everything else both sides changed is a conflict.
- **Lines** end after each line feed, and are compared byte for byte, endings included. Opalro never changes a line ending to make a merge work.
- **How.** Opalro compares the starting point with your version, and with theirs, line by line, with the same comparison `opalro changes` uses. Each comparison marks stretches of the starting point as changed. Stretches from the two sides that overlap or touch are taken together as one region. A region is settled when only one side changed it (that side's lines are taken) or when both wrote the same lines (taken once). A file whose every region is settled is merged. One region that isn't makes the file a conflict.
- **Time.** Each comparison gets `changes`' two seconds. A comparison that runs out gives a coarser answer, never a wrong one: the merge can then only be more careful, calling a conflict where a longer look would have settled it. A warning (`merge_timed_out`) names such a file.
- **The rules that decide** are the ones in the folder's `opalro.toml` when the combine starts, as for every command (decision 114), even if the combine changes that file. A path's `conflict-markers` is what the last rule matching it says, and `true` when none does (storage-format.md §12). Patterns match as `.opalroignore` patterns do, by the same code, with one difference: the letter case of ASCII letters is ignored on every system. A combine's result is stored, so it must not depend on the computer it ran on; and of the two fixed choices this is the safe one, since `*.unity` then also covers `Main.UNITY`, and a file is at worst left for a person to settle, never merged against a rule its author thought covered it.
- **The result** is stored like any file's contents. Its executable mark comes from decision 166.

## Consequences

- The aim is that Opalro merges a file exactly when Git does on the same three versions, apart from two things: where the two programs line the texts up differently (only where lines repeat), and files Git's guesses pair up after a move and Opalro's rules don't (decision 168). The comparison with `git merge-file` checks this: on texts whose lines don't repeat the two must agree every time, and on texts whose lines repeat they must give the same bytes whenever both merge.
- The merge is checked three ways: properties that must hold for any three texts (changing one side only gives that side; the same change twice gives it once; swapping the sides gives the same result), comparison with `git merge-file` on generated texts, and the model test.
- A project that changed line endings on one line will see conflicts in every file both lines touched. The message for a conflict says when the two versions differ only by line endings, as `opalro changes` does.
- Merging inside a line, and merging by a file's grammar, stay on the deferred list.
