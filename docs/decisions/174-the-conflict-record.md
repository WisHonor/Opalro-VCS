# 174 — The conflict record, final

- **Status:** Accepted
- **Date:** 2026-11-14
- **Agenda topic:** none (milestone 9 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 007](007-conflicts.md) (a conflict keeps every version, as a list of terms, with the person's half-fixed version); [decision 148](148-what-format-1s-spec-says-before-it-freezes.md) (the conflict record's layout was left a draft, to arrive with the `conflicts` feature)

## Context

**In plain words.** When two lines changed the same file in ways Opalro can't put together, decision 007 says Opalro keeps every version: yours, theirs, and the one both started from. The combined save then holds, at that file's place, not a file but a small record that points at those versions. This decision fixes what that record holds, byte for byte, because from v0.2 on it is part of what Opalro stores and every later version must read it.

**Where things stand.** `docs/storage-format.md` §8 has held a draft since v0.1: a version byte, an odd number of terms (side, ancestor, side, …), each either absent or a kind, an ID and a size, and an optional "in-progress version" (a file ID and a size). What froze with format 1 is only the reservation: listing entry kind 6, pack object kind 19 and the fingerprint's context string. The layout itself was left open until conflicts were built.

Two questions decide the layout:

1. **How does Opalro know what the file in the folder should look like, and whether the person has touched it?** A text file in conflict is written out with markers around the places that clash. The plain way to tell "untouched" from "edited" is to write the markers again and compare. That fails here: the line-by-line comparison has a time limit ([decision 167](167-merging-text-line-by-line.md)), and past it the places come out larger. Written twice, the same conflict can give two different texts.
2. **How does Opalro know what "no markers left" means?** For a text file written with markers, markers gone means fixed (decision 007). For a file written without markers (a picture, a Unity scene), the file never had markers, so their absence means nothing.

## Options

- **A. Store only the versions; work the folder's file out each time.** Smallest record (the draft as it was). Needs a comparison that gives the same text every time, which the time limit rules out, or a parser that reads marked text back into its three versions and compares those. The parser is the hard part: marker lines inside the file's own text, a last line without a line break, labels.
- **B. Store the versions and the file as it stands in the folder.** The record holds, beside the terms, the version the folder holds for it: the marked text when it is first made, the person's half-fixed text once they save in the middle of fixing. Writing a conflict out is then writing a stored file, and "has the person touched it" is the comparison Opalro makes for every file: same ID or not. One file's worth of extra storage for each conflict in text.
- **C. As B, with the labels, the mode and the marker length each stored as fields of their own, and the marked text worked out again when needed.** Keeps the stored text out, but brings back A's problem.

Jujutsu stores only the terms and parses marked files back (A). Git stores nothing: its markers live only in the working folder, and a file with markers that is committed is just a file.

## Decision

**B.** The record holds the terms, and optionally the version that stands in the folder.

```text
u8      version = 1
varint  term count (odd, from 3 to 255)
terms, in the order side, ancestor, side, …, side:
  bool    present (false = absent: removed there, or never there)
  if present:
    u8      kind (1 to 5, as in §3; never 6)
    id      id
    varint  size (as §3 says for that kind)
bool    the folder holds a version of its own
if true:
  bool    present (false = the person removed it)
  if present:
    u8      kind (1 to 5)
    id      id
    varint  size
  u8      marker length (0, or from 7 to 255)
```

- **The terms** are decision 007's list. The first term is the side of the line the combine was made on ("mine"), the last is the side that was combined in ("theirs"), and between two sides stands the version both started from. A term may be absent, and terms may be of different kinds (a file against a folder).
- **What a conflict stands as in the folder** is one rule, used everywhere a save meets the folder: the version of its own if the record has one, otherwise the first term (your version stays, [decision 176](176-a-conflict-in-the-folder.md)). Either may be absent, and then the folder holds nothing there.
- **Marker length** says whether the conflict is shown with markers. 0: it is not (the folder holds a version as it is). 7 or more: the version of its own is a text that was written with conflict markers of that many characters. Stored, because a text that already holds a line of seven `<` gets longer markers, and "are any markers left" must look for the right ones.
- **A record never points at another conflict,** and Opalro 0.2 never writes a conflict one of whose sides is a folder that holds a conflict somewhere below it ([decision 175](175-what-a-combine-keeps-as-a-conflict.md) stops the combine first). Meeting, in the folder, a conflict below a folder that another conflict stands as is refused as damage; `tidy` and `verify` still follow everything such a record names.
- **Opalro 0.2 writes records of three terms only,** and reads any record this layout allows. Decision 007 chose the general list so that more terms can come later (after combines of several lines, or when saves are rebuilt on top of each other) without a new format.
- **A reader refuses** a record with another version byte, an even term count or one outside 3 to 255, a kind outside 1 to 5, a size other than 0 for a folder or a project link, a marker length from 1 to 6, a marker length other than 0 on a version that is absent or not a file (kinds 1 and 2), a bool that is neither 0 nor 1, or bytes left over.
- **The listing entry** stays as format 1 reserved it: kind 6, the record's ID, size 0. The record is pack object kind 19, fingerprinted with the context string reserved for it.
- **`opalro tidy` keeps** what a kept record names: every present term and the version of its own, each followed as a listing entry of that kind is (a folder's listing and everything below it, a big file's chunk list). **`opalro verify` decodes** every record and checks that what it names is there, of the kind and size it says.
- **Test vectors** for the record go into `docs/storage-format.md` §13 and the vectors test, and its decoder gets a fuzz target, before any code writes one.

## Consequences

- Writing a save's conflict into the folder is writing a stored version, the same on every computer and every time. Nothing parses markers to compare versions.
- A text conflict costs one more stored text (the marked one); a half-fixed save costs another. `tidy` frees them with the saves that hold them.
- The labels on marker lines (the lines' names and saves) are part of the marked text, written when the conflict is made, and stored nowhere else. Renaming a line later doesn't change a conflict.
- Two combines of the same versions under different line names make two different records (their marked texts differ), and a later combine takes them for two different conflicts ([decision 175](175-what-a-combine-keeps-as-a-conflict.md)).
- A record with more than three terms can't be made by 0.2, but one from a later version or a planted one is read: it stands in the folder by the same rule, `tidy` and `verify` follow all of it, and the commands that pick a side say they can't for such a record ([decision 177](177-recording-a-fix-save-and-resolve.md)).
- `docs/storage-format.md` §8 is rewritten from "reserved" to this, before the code.
