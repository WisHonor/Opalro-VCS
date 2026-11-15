# 176 — A conflict in the folder: markers, or your version

- **Status:** Accepted
- **Date:** 2026-11-14
- **Agenda topic:** none (milestone 9 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 007](007-conflicts.md) (text files get markers with the ancestor section, labelled with line names and change IDs; other files keep the version of the person's own line; a rule decides which files get markers); [decision 167](167-merging-text-line-by-line.md) (which files are text, and the rule `conflict-markers`); [decision 004](004-project-contents.md) (line endings are bytes)
- **Refined by:** [decision 181](181-what-building-milestone-9-refined.md) (a label's long name is cut, never the save's letters; what `status` reads)

## Context

**In plain words.** A save can hold a conflict at a file's place, but a folder on a drive can only hold files. So for every conflict Opalro must put *something* in the folder: something a person can work with, and something their game engine or compiler doesn't choke on. Decision 007 set the two forms: a text file gets the well-known markers around the places that clash; anything else keeps the version of your own line, and the other versions stay inside `.opalro`.

This decision settles the details: exactly what the marked text looks like, when a file gets markers and when it doesn't, and how Opalro tells that a person has fixed it.

A marked place looks like this, as Git's `diff3` style and most editors show it:

```text
<<<<<<< main (kxqmzzyx)
the lines as your line has them
||||||| where the lines parted (lmnopqrs)
the lines as both started with them
=======
the lines as the other line has them
>>>>>>> side (vwxyzklm)
```

## Options

**What the folder holds when your own line has no file at the place** (you removed it, the other line changed it)

- **A. Nothing: your version stays, and your version is "no file".** One rule for every conflict that isn't shown with markers. The other line's version is brought in by a command.
- **B. The other line's version,** as Git leaves it in the working folder for a file "deleted by us". The person sees the file they must decide about. But a file appears that their own line had removed, and in a game project that can be half an asset (the file without its `.meta`).

**What counts as "markers left"**

- **A. An opening or closing marker line** (`<<<<<<<` or `>>>>>>>`, of the conflict's own length).
- **B. Any of the four marker lines.** Stricter, but a line of exactly seven `=` is ordinary in a text (a heading's underline in Markdown or reStructuredText), so a fixed file could read as unfixed.

## Decision

**A, and A.**

**1. One rule says what a conflict stands as in the folder:** the record's version of its own if it has one, otherwise its first term, your line's version ([decision 174](174-the-conflict-record.md)). Every command that writes the folder from a save, or compares the folder with a save, treats a conflict's place as holding that entry: a file, an executable file, a link, a folder with everything in it, or nothing.

**2. A conflict is shown with markers when** both sides are files, the version where the lines parted is a file or absent, the rules of both lines allow it (the folder's `opalro.toml` and the one in the save being combined in, each read whole, asked under every name the file had: decisions [167](167-merging-text-line-by-line.md), [172](172-red-team-m8-session-2-rulings.md) and [173](173-the-rules-of-both-lines-decide-what-a-combine-merges.md)), every version is text within the limits text has everywhere (no zero byte in its first 8,000 bytes, at most 64 MiB, at most 1,000,000 lines), the versions don't differ only in line endings, and the marked text is itself at most 64 MiB. **Every other conflict stands as your version.** One case is neither: while either line's `opalro.toml` can't be read whole, a place where the rules would have to decide (both sides are text files) stops the combine, as decision 172 says, because Opalro can't know whether that file may be merged or marked.

**3. The marked text** is the text the line-by-line merge gives, with each place where the lines clash written as above:

- an opening line of `<` and a label, your lines, a line of `|` and a label, the lines both started with, a line of `=`, their lines, a closing line of `>` and a label. The part between `|` and `=` is always there; when nothing was there where the lines parted, or what was there is unknown ([decision 175](175-what-a-combine-keeps-as-a-conflict.md)), it is empty and its label says which.
- **Labels:** the name the person combined by (the line they were on; the line, tag or ID they named) and the short change ID of that save; for the middle part, "where the lines parted" and that save's short change ID, or "several saves" when there is more than one. A label is one line of plain text, with anything odd in a name shown as Opalro shows it elsewhere, and at most 200 bytes.
- **Marker length:** seven characters, or one more than the longest run of `<`, `|`, `=` or `>` that starts a line in any of the three versions, so that the file's own text is never taken for a marker. (Past 255 the conflict stands as your version.) The record stores the length.
- **Line endings:** each marker line ends as the first line of your version ends (`\r\n` or `\n`; if your version has no line break, as theirs, then as the starting version's, else `\n`). Where a part's last line has no line break (the end of a file), one is added before the next marker.
- The comparison's time limit stays: past it the places come out larger, never wrong, and the combine warns as it does today.
- The marked text is a file like any other in the store, and gets the executable mark the merge gives the file.

**4. Reading the folder back.** At a conflict's place, the folder is compared with what the conflict stands as:

- **The same:** not touched since the save.
- **Different, and the conflict is shown with markers:** if the folder holds a file with an opening or closing marker line of the conflict's length, it is **being fixed** (the places left are its opening lines); otherwise it is **fixed**: by decision 007, markers gone means fixed, and so does the file being gone.
- **Different, and the conflict stands as a version:** **changed in place**, and still a conflict. Only the person's word fixes such a conflict ([decision 177](177-recording-a-fix-save-and-resolve.md)): game engines rewrite files just by opening a project, and a file that changed by itself must not settle which line's version is kept.

A marker line is a line that starts with exactly that many `<` or `>` followed by a space or the line's end.

**5. Comparing two saves is different.** When a save is compared with another save (in `show`, in `changes --save`, in a combine), a conflict is compared as stored: it equals only the same conflict. What it stands as matters only between a save and the folder.

## Consequences

- Editors and merge tools that know Git's markers work on a file in conflict as they do with Git.
- A project keeps opening in its game engine after a combine with conflicts: scenes, prefabs and binary assets are your line's versions, whole.
- When you removed a file and the other line changed it, nothing appears in your folder; `opalro status` lists the place, and `opalro resolve` brings their version if you want it.
- A text that is itself about conflict markers gets longer ones, and is read back right.
- A person who leaves a stray `=======` line behind has saved a file with a stray line; Opalro doesn't hold the file back for it.
- What a conflict's place holds is decided by one function, used by write-out, the folder walk, the check for unsaved work, safety copies, `discard` and the shelf.
