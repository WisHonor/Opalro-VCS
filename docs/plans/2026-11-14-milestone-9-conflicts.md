# Milestone 9 ("Conflicts"): Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task by task. Steps use checkbox (`- [ ]`) syntax for tracking. Every subagent runs on Opus.

**Goal:** A combine that has conflicts is kept. The combined save holds a conflict record at each place that clashes, keeping every version; a text file is written into the folder with markers around the places that clash, any other place keeps your line's version; `opalro save` carries conflicts along, half-fixed files and all; `opalro resolve` records fixes, into the combined save when it may be replaced; and every command that reads a save, writes the folder or compares the two knows what a conflict is. A project made by v0.1 gains the ability through `opalro setup --upgrade`; without it, nothing changes.

**Architecture:** One new module, `conflict.rs`, holds the record (its codec, the one rule of what a conflict stands as, and what the folder shows at a conflict's place), `conflict/standing.rs` the one reader that gives a stored listing *as it stands in the folder* (each kind-6 entry replaced by what its record stands as, or left out; the listing keeps its stored ID; records read only for kind-6 entries of listings being read anyway, remembered by ID within a command), and `conflict/markers.rs` the marker lines (writing them, labels, their length, and finding them again). **No command reads a save's listings to look for conflicts:** a conflict is met where a listing's entry is met. The folder walk under `status`, `changes`, `save` and `resolve` reads the base save as stored and settles a kind-6 base entry where it settles that name (in `build_with`, as it keeps a project link today): compared with what the conflict stands as, it is kept, kept with a new record whose own version is what the disk holds, or becomes an ordinary entry; a conflict that stands as a folder is walked with that folder's listing as its base. `status` lists a conflict where its comparison of the save with the folder reaches the entry (always, where the folder is scanned: a listing built from disk never equals a stored one that holds a conflict) and, for the parts the walk carries unread (names this computer can't hold), from what the walk already reads there. Write-out's plan, the check for unsaved work, safety copies, `discard`, the shelf and `setup` read both trees through the standing reader, so a conflict's place is planned as the entry it stands as. A save holds no count of its conflicts. A combine never makes a conflict inside a conflict, and readers refuse one as damage. The merge (`combine/tree.rs`) returns a conflict entry where milestone 8 returned nothing, writing the record and the marked text into the combine's pack set as it goes; move clashes are put back after the tree merge; a starting point that can't be told is a value of its own (`Unknown`) inside the merge. `resolve.rs` is the new command; it shares `save`'s walk and the code that composes and records a save, and writes the folder for `--keep` through write-out's planned actions under one diary step. The `conflicts` feature is one check, `Repository::keeps_conflicts`.

**Tech Stack:** Rust 1.99.1 and the crates already in the workspace. No new dependency.

**Spec:** six decisions: [decision 174](../decisions/174-the-conflict-record.md) (the conflict record, final), [decision 175](../decisions/175-what-a-combine-keeps-as-a-conflict.md) (what a combine keeps as a conflict, and what still stops it), [decision 176](../decisions/176-a-conflict-in-the-folder.md) (a conflict in the folder: markers, or your version), [decision 177](../decisions/177-recording-a-fix-save-and-resolve.md) (recording a fix: `save` and `resolve`), [decision 178](../decisions/178-the-conflicts-feature.md) (the `conflicts` feature), [decision 179](../decisions/179-every-command-and-a-conflict.md) (every command and a conflict); with the rulings made before the build ([the milestone's rulings](2026-11-14-milestone-9-rulings.md): R1 to R8, the rulings A to C, and the answers to the planning questions in four rounds), quoted below where they bind; decisions 003, 006, 007, 008, 017, 121, 142, 148 and 164 to 173 for background; [storage-format.md](../storage-format.md) §3, §8, §10, §13; [terminal.md](../terminal.md) and [json.md](../json.md). Each task that adds words a person sees writes them into terminal.md, json.md and the guide in the same commit.

**Rulings:** [2026-11-14-milestone-9-rulings.md](2026-11-14-milestone-9-rulings.md) holds the rulings made before the build, and is kept as the build goes, verbatim. Every ruling's premise is checked in the code before it is made.

**Branches:** tasks are built on `milestone-9`, made from `v0.2` once milestone 8 is merged into it. It is merged into `v0.2` after the red-team session (Task 18). `main` is never touched.

## Global Constraints

- Everything in CLAUDE.md applies.
  - `#![forbid(unsafe_code)]` in the engine and the command-line crate. **No task adds `unsafe`.**
  - No `unwrap`, `expect` or `panic` outside tests.
  - **One place per rule:**
    - the conflict record's layout, its refusals, and *what a conflict stands as* (`conflict::stands_as`) only in `conflict.rs`; what the folder shows at a place in conflict (`conflict::seen`) there too;
    - a stored listing as it stands in the folder only in `conflict/standing.rs` (`Listings`); how the save walk settles a kind-6 base entry only in `build_with` (snapshot.rs) calling `conflict::seen`; nothing else replaces a kind-6 entry or reads a record's fields to decide what the folder holds;
    - marker lines (their length, their labels, writing them, and finding them in a file) only in `conflict/markers.rs`;
    - whether the project keeps conflicts only in `Repository::keeps_conflicts` (R2); no other code reads `.opalro/format`'s feature list;
    - what counts as text, its limits and the line comparison only in `changes.rs`; the merge rules only in `combine/`; the starting points only in `ancestry.rs`; moves only in `moves.rs`; Git's words only in `git_words.rs`; colour, symbols and widths only in `style.rs`;
    - composing a new save (amend or on top, its extra fields, its change ID) only in `snapshot.rs`, shared by `save` and `resolve`.
- **Spec before code, in the same task or an earlier one:** storage-format.md §8 (final) and §13 (vectors) in Task 1; §10 (the feature) in Task 2; §3 (what a conflict stands as counts toward a save's limits) in Task 3; terminal.md and json.md for every word and field in the task that prints them.
- **Format 1 is frozen.** Nothing here changes how any existing record is written: listings, saves, diary entries, packs and caches keep their bytes. What is new: the layout of pack object kind 19 (reserved since v0.1, written only in a project that lists `feature conflicts`) and the `feature conflicts` line in `.opalro/format`. A save holds no count of its conflicts (ruling B): no new extra field. A task that finds it needs more stops and says so.
- **Without the feature, nothing changes (R2).** In a project whose `format` doesn't list `conflicts`, a kind-6 entry and a kind-19 object are treated exactly as today (storage-format.md §8's frozen paragraph): never decoded, write-out leaves the place alone, `tidy` stops, `verify` reports the listing. The tests that pin this today keep their assertions; they only gain a project made without the feature (`support::without_conflicts_feature`).
- **Everything in a record is untrusted (R3).** A term's kind is checked against the object it names when that object is read. A folder among the terms can be as large as a save: commands that don't need it never read it; `tidy` and `verify` walk it as they walk any folder, with no recursion; `resolve --keep` writes it under write-out's own limits and name rules. A marked text is at most 64 MiB. A label is plain text of at most 200 bytes with nothing that breaks a line. `combine`'s and `resolve`'s lists of conflicts are capped at 100 with a count of the rest; `status` lists conflicts as it lists changes, all of them; counts are `u64`. A conflict record is at most 16 KiB (`conflict::MAX_RECORD_BYTES`): a larger one is refused from its index entry's size before it is unpacked.
- **The record's writer is the combine only** (three terms), plus `save` (and `resolve` through it), which rewrites a record only to change its own version (R4). Nothing writes a record in a project without the feature, not even into a pack set that is later dropped (a pack set past 1,048,576 objects moves a full pack into the store early).
- **No command reads a save's listings to look for conflicts (ruling A).** A project that holds none pays nothing for the feature: `status`, `save` and `goto` read no listing and no record they don't read today. A record is read only for a kind-6 entry of a listing the command reads anyway, at most once per command (remembered by ID). Pinned by Task 4's `a_project_without_conflicts_reads_nothing_more` and Task 6's for `goto`.
- **A listing read as it stands keeps its stored ID and is never stored** under that ID; code that stores a listing stores it under the ID of its content (`store_listing` does). Two folders with the same stored ID stand alike, so skipping equal IDs stays right; different IDs that stand alike are gone into and found equal, which costs only the folders that hold a conflict and those above them.
- **No conflict inside a conflict (ruling C).** A combine stops where both lines changed a place and one of them holds it in conflict or holds a conflict below it. A reader that reads, for the folder, a listing below a folder a conflict stands as refuses a kind-6 entry there as damage, naming the place; `tidy` and `verify` follow every record whatever it holds, and `verify` reports the nesting.
- **No record below another conflict's place (D1).** Putting records back after the tree merge looks at each folder above the place and puts nothing when one is a conflict's entry; it never replaces a conflict's entry with a plain folder (Task 10).
- **Both lines' rules decide, and rules that can't be read whole stop (decision 173, D2).** A file is merged line by line, and shown with markers, only when the folder's `opalro.toml` and the one in the save combined in both allow it, under every name it had (`RuleMatcher::line_merge`, `rules.rs:122`). While either can't be read whole, a place the rules would decide (both sides text within the limits, merging or overlapping) stops the combine with decision 172's words and hints, `rules_not_whole` kept on the error, inside the starting points' own combines too; every other clash is kept as usual. Tasks 9, 11 and 12.
- **A record that can't be read is damage, named:** the command stops with `RepoError::Damaged` saying the place's path when the reader knows it (else the entry's name and its folder listing's ID) and the record's ID (Words: "A conflict record that can't be read"); `verify` reports it as a finding; `tidy` stops as for any record it must keep.
- **Untrusted text is escaped exactly as today** (`opal_core::text::escape`, per piece, at the edge): paths, line names, messages. A stored tag's name through `tags::shown_name`. **No hint puts a stored name, a path or any other text from the project inside a command a person would paste**; a command in a hint holds only placeholders (`<path>`, `<file>`) or text `diary::is_plain` accepts.
- **No message promises a milestone or a version.**
- **`--json` never asks and never carries colour, symbols, progress or "Next:" lines.** `opalro_json` stays `1`: documents and fields are added, none is changed (decision 117).
- **Exit codes:** 0 done (also a combine that kept conflicts, and a `resolve` that leaves some), 1 failed or stopped (`--only-if-clean` meeting a conflict too), 2 a command line Opalro couldn't use (decision 017's three codes).
- **Tests that can't lie:**
  - a property test's model shares neither the engine's code nor its rule's wording;
  - random tests have fixed seeds and assert that each kind of case they name occurred at least 3 times;
  - a comparison with Git fails when it compared nothing;
  - after any command that writes the folder, the folder is compared, folders included, with what `opalro goto <that save>` writes into an empty folder (`support::assert_folder_is_goto_of`, added in Task 6);
  - no assertion on a clock unless the gap is at least tenfold;
  - each rule is broken on purpose once, in the ways the task names; the implementer runs the tests after each break, reports which test caught it, and puts the code back.
- **Crash safety:** every new write (the feature upgrade, `resolve`'s save and its folder change, `save`'s re-recorded records) gets fault-point tests like `goto`'s and `save`'s (`fsio::fault_point` labels named in each task; the tests run with `--features fault-injection`).
- **Test names that read as IDs:** no test resolves a line or tag name made only of the letters `k` to `z`, or of 8 or more hex digits. Use `side`, `wall-run`, `v1.0`.
- **One writer at a time:** tasks run strictly one after another, and no two agents write in the working tree at once. `git add` names paths; never `git add -A` while anyone else is in the folder.
- Code comments are short and human: the why in a line or two, no process words, no task, ruling or decision numbers in code.
- **Values:**

  | What | Value |
  |---|---|
  | Terms in a record | odd, 3 to 255 read; Opalro 0.2 writes 3: yours, where the lines parted, theirs |
  | A record's kinds | terms and its own version: 1 to 5 (§3); never 6 |
  | Marker length | 0 (stands as a version), or 7 to 255; 7, or one more than the longest run of `<`, `\|`, `=` or `>` that starts a line in any of the three versions; past 255 the conflict stands as your version |
  | A marker line | exactly that many `<` (or `>`) at the start of a line, then a space or the line's end |
  | A label | one line of plain text, at most 200 bytes (cut at a character boundary), no control character, no line break |
  | A label's save | the first 8 letters of the save's change ID |
  | A marked text | at most 64 MiB (`changes::MAX_LINES_SIZE`), else the conflict stands as your version |
  | Conflicts named in `combine`'s and `resolve`'s lists | the first 100 in path order, then `… and N more`; `status` names them all |
  | A conflict record's size | at most 16 KiB (`conflict::MAX_RECORD_BYTES`; three terms take under 200 bytes, 255 about 11.3 KiB) |
  | A default `resolve` message | `Fixed conflicts in <path>` (one place) or `Fixed conflicts in N places` |

- **Local runs on this machine:** `export PATH="$HOME/.cargo/bin:/c/Program Files/GitHub CLI:$PATH"` and `export OPALRO_CONFIG_DIR="$(mktemp -d)"` before any test. One cargo command at a time. Build with `-j 2`, test with `-- --test-threads=2`, one test file per command (`cargo test -j 2 -p opal-vcs --all-features --test conflicts -- --test-threads=2`; engine tests: `cargo test -j 2 -p opal-vcs-core --all-features <filter> -- --test-threads=2`). Run `cargo fmt` after every edit and `cargo clippy -j 2 --workspace --all-targets --all-features -- -D warnings` before every commit. Unix-only code can't be checked here: watch CI. Test files live in a temporary folder, never in the repository.
- Docs are edited with the Edit tool or `sed`, never Python in text mode; every edited doc is checked with `tr -cd '\r' < file | wc -c`, which must print 0.
- The untracked `.codex/` and `AGENTS.md` are not ours: never staged, edited or deleted.
- One commit per task plus fix commits, with no Claude attribution, on `milestone-9`. The controller pushes at task boundaries; CI runs on Windows, macOS and Linux, and a task is done only once they pass. `v0.2` comes in by merge, never by rebase.
- Each task ends by naming the documents it changed (`docs/testing.md` for new test files, `docs/security.md` rows for new threats), not leaving them for Task 17.

## Review Focus

1. **A version freed by `tidy` while a conflict still names it.** A combined save whose conflicts name your file, theirs (a big file stored in chunks), a folder of theirs with files inside, and a half-fixed text saved as its own version; the lines then deleted so only the combined save keeps them. Expected: after `tidy`, every term and every own version reads back, chunk lists and everything below the folder included; once no kept save holds the record, all of it is freed. Tests: Task 3 `tidy_keeps_every_version_a_kept_conflict_names`, `tidy_frees_a_record_and_its_versions_once_no_save_holds_it`; Task 16's model rule `no_version_a_kept_conflict_names_is_freed`.
2. **A fix recorded for a file nobody chose.** A Unity scene in conflict standing as your version; the engine rewrites it on opening. Expected: `save` keeps the conflict with the rewritten file as its own version; `resolve` with no path records nothing for it and lists it with what it needs; `resolve Main.unity` without `--keep` refuses until the file is changed or `--keep` given. Tests: Task 7 `a_changed_picture_stays_a_conflict_with_the_changed_file_as_its_own`; Task 13 `resolve_without_paths_never_records_a_version_conflict`, `resolve_refuses_an_untouched_version_conflict_without_keep`.
3. **A half-fixed file overwritten without a safety copy.** A marked text edited but not saved, then `goto` elsewhere, `discard`, `resolve --keep theirs`, `undo`. Expected: `goto` stops (unsaved work in the way); `discard` and `resolve --keep` keep a safety copy first; the copy holds the half-fixed bytes. Tests: Task 6 `a_half_fixed_file_stops_a_goto_it_is_in_the_way_of`, `discard_gives_back_the_marked_text_and_keeps_a_safety_copy`; Task 13 `resolve_keep_theirs_keeps_a_safety_copy_of_a_half_fix`.
4. **A marked text that reads back as fixed.** Texts whose own lines look like markers (seven `<`, `=======` underlines, runs of 300 `>`), CRLF, no last line break. Expected: the marker length always passes the texts' own runs, so the untouched text is found to have exactly as many opening marker lines as places; a save between reading a file and checking its markers stops rather than record a different file. Tests: Task 8 `every_marked_text_reads_back_with_its_places`; Task 7 `a_file_changed_between_the_walk_and_the_marker_check_stops_the_save`.
5. **A conflict that hides a change.** A file moved on one line and changed in place on the other, while the mover put something new where it went; starting points that disagree; a line that already holds an unfixed conflict. Expected: every version a line saved is in the result or in a record there; a place where the start can't be told is never read as an addition or a removal; a place both lines changed that one holds in conflict, or holds a conflict below, stops the combine. Tests: Task 10 `nothing_a_line_saved_is_left_out` (property); Task 11 `unknown_is_not_absent`; Task 9 `both_lines_changed_a_place_one_holds_in_conflict_stops_naming_it`, `a_folder_holding_a_conflict_below_stops_the_combine_too`; Task 10 `a_move_clash_below_a_kept_conflict_is_held_by_that_conflict`; and `status`'s rule that a conflict's place is never listed again among changes is paired with its row saying "changed in place" (Task 4 `a_change_at_a_conflicts_place_shows_in_its_row`).

---

## File structure

| File | Responsibility |
|---|---|
| `crates/opal-vcs-core/src/conflict.rs` (new) | The record (`Record`, `Term`, `Own`), its codec, `stands_as`, `seen`, `Settled` |
| `crates/opal-vcs-core/src/conflict/standing.rs` (new) | `Listings`: a stored listing as stored, or as it stands in the folder; records remembered by ID; the nesting refusal |
| `crates/opal-vcs-core/src/conflict/markers.rs` (new) | Marker lines: length, labels, writing, scanning |
| `crates/opal-vcs-core/src/resolve.rs` (new) | `opalro resolve` and `--copy` |
| `crates/opal-vcs-core/src/repo.rs` | The `format` file from the features a project has; `keeps_conflicts`; turning features on |
| `crates/opal-vcs-core/src/setup.rs` | `setup --upgrade` |
| `crates/opal-vcs-core/src/store.rs` | `Store::conflict` |
| `crates/opal-vcs-core/src/listing.rs` | `SaveLimits::add_all_standing`, which also counts kind-6 entries |
| `crates/opal-vcs-core/src/diff.rs`, `writeout.rs` (`Tree`), `kept.rs` (`overlay_with`) | Read listings through `Listings` |
| `crates/opal-vcs-core/src/tidy.rs`, `verify.rs` | Following and checking records; `verify` reports nesting |
| `crates/opal-vcs-core/src/snapshot.rs`, `snapshot/walk.rs` | The walk settles a kind-6 base entry where it settles that name; a conflict standing as a folder walked with that folder as base; the save's composition shared with `resolve` |
| `crates/opal-vcs-core/src/status.rs`, `changes.rs` | Conflicts in `status` (where the comparison reaches them, and in the parts carried unread); `changes` at a conflict's place; `show`'s new change kinds and counts |
| `crates/opal-vcs-core/src/writeout.rs`, `kept.rs`, `shelf.rs`, `switch.rs`, `setup.rs` | Write-out, safety copies, discard, the shelf, reading both trees as they stand |
| `crates/opal-vcs-core/src/combine.rs`, `combine/tree.rs`, `combine/text.rs` | Keeping conflicts, move clashes, the unknown start, marked texts |
| `crates/opal-vcs-core/src/moves.rs` | Move notes pass a conflict's place over |
| `crates/opal-vcs/src/main.rs`, `resolve_output.rs` (new), `combine_output.rs`, `setup_output.rs`, `goto_output.rs`, `history_output.rs`, `show_output.rs`, `output.rs`, `help.rs`, `git_words.rs` | The command line and its words |
| `crates/opal-vcs/tests/conflicts.rs` (new), `resolve.rs` (new), `corpus_conflicts.rs` (new), `combine.rs`, `setup.rs`, `merge_vs_git.rs`, `model.rs`, `json_formats.rs`, `cli.rs`, `terminal.rs`, `git_words.rs`, `undo.rs`, `support/mod.rs` | Tests |
| `crates/opal-vcs-core/tests/vectors/conflict.json` (new), `format.json`, `tests/vectors.rs`; `fuzz/fuzz_targets/decode_conflict.rs` (new), `fuzz/Cargo.toml` | Vectors and fuzzing |

---

### Task 1: The conflict record

**Files:**
- Modify: `docs/storage-format.md` §8 (first, before the code), §13 (a "Milestone 9" entry), §14 (strike "Conflict record details")
- Create: `crates/opal-vcs-core/src/conflict.rs`; `crates/opal-vcs-core/tests/vectors/conflict.json`; `fuzz/fuzz_targets/decode_conflict.rs`
- Modify: `crates/opal-vcs-core/src/lib.rs` (`pub mod conflict;`), `store.rs` (`Store::conflict`), `crates/opal-vcs-core/tests/vectors.rs`, `fuzz/Cargo.toml` (a `[[bin]]` as `parse_rules` has), `crates/opal-vcs/tests/support/mod.rs` (`add_conflict`)

**Interfaces:**
- Consumes: `encoding::{Reader, Writer}` (`bool`, `u8`, `varint`, `id`, `finish`); `listing::EntryKind`; `id::{hash_record, Flavor::Conflict}`; `Store::read_record(&id, ObjectKind::Conflict)`.
- Produces:

```rust
/// One version a conflict names: an entry's kind, ID and size, without a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Term { pub kind: EntryKind, pub id: Id, pub size: u64 }

/// The version the folder holds for a conflict, when it is its own: `version` is `None` where the
/// person removed it; `marker` is 0, or 7 to 255 when `version` is a text written with markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Own { pub version: Option<Term>, pub marker: u8 }

/// A conflict record (storage-format.md section 8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Side, where they parted, side, …: yours first, theirs last. Odd, 3 to 255.
    pub terms: Vec<Option<Term>>,
    pub own: Option<Own>,
}

pub const MAX_TERMS: u64 = 255;
pub const MIN_MARKER: u8 = 7;
/// The most bytes a conflict record may take; refused before it is unpacked.
pub const MAX_RECORD_BYTES: u64 = 16 << 10;

impl Record {
    pub fn encode(&self) -> Vec<u8>;
    /// Refuses everything section 8 lists; never fixes anything.
    pub fn decode(bytes: &[u8]) -> Result<Record, DecodeError>;
    pub fn id(&self) -> Id; // hash_record(Flavor::Conflict, &self.encode())
    /// Yours: the first term.
    pub fn mine(&self) -> Option<Term>;
    /// Theirs: the last term.
    pub fn theirs(&self) -> Option<Term>;
    /// Where the lines parted, for a record of three terms; `None` for more.
    pub fn before(&self) -> Option<Option<Term>>;
    /// 0 unless its own version was written with markers.
    pub fn marker(&self) -> u8;
}

/// What a conflict stands as between a save and the folder: its own version if it has one, else
/// its first term. `None`: the folder holds nothing there.
pub fn stands_as(r: &Record) -> Option<Term>;

// store.rs
/// The conflict record `id`, checked against its fingerprint and decoded; a copy whose index
/// says it is over `MAX_RECORD_BYTES` is refused before it is unpacked.
pub fn conflict(&self, id: &Id) -> Result<Record, StoreError>;
```

- **The size check needs no new plumbing:** `Store::read_record` is `self.first_good(id, kind, false, |pack, entry| pack.read_record(entry))` (`store.rs:655-657`), and the closure gets the copy's `IndexEntry`, whose `uncompressed_size` the pack's index already checked with `sizes_allowed` (`pack.rs:104`, applied at `pack.rs:404` and `:1479`). `Store::conflict` passes its own closure that refuses `entry.uncompressed_size > MAX_RECORD_BYTES` (`PackError::TooLarge`, so the next copy is tried as for any bad copy) before `pack.read_record(entry)`. `sizes_allowed` itself is not changed: it knows no project, and a project without the feature must keep reading such an object as today. Under `#[cfg(test)]`, `Store::conflict` counts its reads in `store::counted` (`store.rs:145`) as `Store::listing` does: a new `pub(super) fn conflict()` and a `pub(crate) fn conflicts() -> usize` beside `listings()`, so Task 4's and Task 6's tests in other modules read it as `combine/tests.rs:356` reads `counted::listings()`; and `counted` gains `pub(crate) fn unpacked() -> usize`, counted where `PackReader::read_record` unpacks a record past `read_small` (`pack.rs:882-892`), for this task's size test. The refusal is a `StoreError` naming the record's ID; the words with the place's path are Task 4's (its reader knows the path).

- **§8, final** (rewritten from "reserved" to [decision 174](../decisions/174-the-conflict-record.md)'s layout, word for word as the draft's code block), then in prose: the terms' order and meaning; "what a conflict stands as" (one sentence, as `stands_as`); the marker length's meaning; a record never names another conflict, and "Opalro 0.2 never writes a conflict whose first term or own version is a folder holding a conflict below it, and refuses one when it meets it in the folder" (ruling C); a record is at most 16 KiB; Opalro 0.2 writes three terms and reads any count the layout allows; **a reader refuses:** another version byte; a term count that is even or outside 3 to 255; a kind outside 1 to 5; a size other than 0 for a folder or a project link; a marker length from 1 to 6; a marker length other than 0 when the own version is absent or not a file (kinds 1 and 2); a `bool` that is neither 0 nor 1; bytes left over. The listing entry stays kind 6 with the record's ID and size 0; the record is pack object kind 19, fingerprinted with `opal-vcs 2026-10-31 conflict v1`. **`tidy` keeps** every present term and the own version, each followed as a listing entry of that kind is; **`verify` decodes** every record and checks what it names. The frozen paragraph stays, headed "Without the `conflicts` feature", unchanged in substance.
- **Vectors** (`conflict.json`, with a printer like the others): valid, each with hex and ID: `markers` (three file terms, own version a file, marker 7); `yours` (a file, absent, an executable; no own version); `kinds` (a folder against a file, a link with its size, a project link); `half-fixed` (own version an executable file, marker 9); `removed-own` (own present and absent, marker 0); `five-terms` (five terms, which a reader takes). Invalid, each breaking only the rule it names: version 2; term count 2; term count 4; term count 1; term count 257; kind 0; kind 6; kind 7; a folder of size 1; a project link of size 1; marker 6; marker 7 on an absent own version; marker 7 on a folder own version; marker 7 on a link own version; a present flag of 2; an own flag of 2; a trailing byte; truncated in the middle of an ID; a non-minimal varint for the count.
- **The fuzz target** `decode_conflict`: decode any bytes; whatever decodes re-encodes to exactly those bytes.
- **`support::add_conflict(pack: &mut PackSet, record: &Record) -> Id`**: adds the record to a pack set, for the tests that plant conflicted saves before the combine can make them.

- [ ] **Step 1: Write the failing tests**
  - `conflict.rs` `tests`: `a_record_round_trips` (proptest, 256 cases, fixed seed: 3 to 9 terms, each absent or of kinds 1 to 5 with sizes as §3 allows, with and without an own version; `decode(encode(r)) == r`; asserts at least 3 cases each of: an absent term, a folder term, an own version present, an own version absent, a marker of 7 or more); `every_refusal_section_8_lists_is_made` (one case per refusal, each asserting the `DecodeError`); `stands_as_takes_the_own_version_then_the_first_term` (own present, own present-but-absent → `None`, no own → first term, first term absent → `None`); `what_decodes_encodes_back_to_the_same_bytes` (proptest, 1,024 cases, fixed seed: half random bytes, half a valid record's bytes with one byte changed; when decode succeeds, encode gives the input; asserts at least 3 cases that decoded and 3 refused).
  - `store.rs` `tests`: `a_conflict_record_is_read_by_its_id`; `a_listing_is_not_read_as_a_conflict_record` (the kind check of `read_record`); `a_conflict_record_over_16_kib_is_refused_before_it_is_unpacked` (a planted kind-19 object of 16 KiB + 1 whose bytes would decode: refused, and `counted::unpacked()` doesn't move: it was never unpacked).
  - `tests/vectors.rs`: `conflict_vectors` (each valid vector's bytes both ways and its ID; each invalid one refused).
- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs-core --all-features conflict -- --test-threads=2`
Expected: does not compile (`conflict` not found).

- [ ] **Step 3: Rewrite §8, add §13's entry, then implement** `conflict.rs`, `Store::conflict`, the vectors (printed by the code, then reviewed by eye against §8), the fuzz target and the support helper.
- [ ] **Step 4: Run the tests and see them pass**, then `cargo test -j 2 -p opal-vcs-core --all-features --test vectors -- --test-threads=2`, and build the fuzz target as the others are built (docs/testing.md §3). Break it on purpose twice and report which test caught it: accept marker 6; accept a marker on an absent own version.
- [ ] **Step 5: Commit** (testing.md: the vectors and the fuzz target are listed where the others are)

```bash
git add docs/storage-format.md docs/testing.md crates/opal-vcs-core/src/conflict.rs crates/opal-vcs-core/src/lib.rs crates/opal-vcs-core/src/store.rs crates/opal-vcs-core/tests/vectors.rs crates/opal-vcs-core/tests/vectors/conflict.json fuzz/Cargo.toml fuzz/fuzz_targets/decode_conflict.rs crates/opal-vcs/tests/support/mod.rs
git commit -m "The conflict record is final: its layout, what every reader refuses, its test vectors, and a fuzz target for its decoder"
```

### Task 2: The `conflicts` feature

**Files:**
- Modify: `docs/storage-format.md` §10 (first), §13 (`format.json`'s second vector); `docs/terminal.md` (`opalro setup`), `docs/json.md` (`opalro setup --json`, a new `opalro setup --upgrade --json`), `docs/guide.md` (`opalro setup`)
- Modify: `crates/opal-vcs-core/src/repo.rs`, `setup.rs`, `import.rs` (its test `start_and_import_write_feature_chunks`), `crates/opal-vcs/src/main.rs` (`Setup { upgrade }`), `setup_output.rs`, `help.rs`
- Modify: `crates/opal-vcs-core/tests/vectors/format.json`, `tests/vectors.rs`; `crates/opal-vcs-core/src/tidy/tests.rs` (`tidy_still_stops_at_a_conflict_record` runs in a project without the feature); `crates/opal-vcs/tests/support/mod.rs` (`without_conflicts_feature`)
- Test: `crates/opal-vcs/tests/setup.rs`, `json_formats.rs`, `repo.rs`'s tests

**Interfaces:**
- Consumes: `write_atomic` (as `write_format` uses it), `parse_format`, `lock_for_change`.
- Produces:

```rust
// repo.rs
/// Every feature this version of Opalro knows, in the order `format` lists them.
pub const FEATURES: [&str; 2] = ["chunks", "conflicts"];
pub const CONFLICTS: &str = "conflicts";
/// `.opalro/format` for a project with `features`: the two first lines, then one `feature` line
/// for each, in `FEATURES`' order.
pub(crate) fn format_text(features: &BTreeSet<String>) -> String;

impl Repository {
    /// Whether this project keeps conflicts: its `format` lists `feature conflicts`. The only
    /// place that asks.
    pub fn keeps_conflicts(&self) -> bool;
    /// The features this version knows that the project doesn't list, in `FEATURES`' order.
    pub fn features_to_turn_on(&self) -> Vec<&'static str>;
    /// Writes `format` with those features added, under `lock`, by the crash-safe write every
    /// other file gets, and reads it back. Gives what it turned on.
    pub(crate) fn turn_on_features(&mut self, lock: &ChangeLock) -> Result<Vec<&'static str>, RepoError>;
}

// setup.rs
/// What `opalro setup --upgrade` did: the features turned on, none when there was nothing to do.
pub struct Upgraded { pub turned_on: Vec<&'static str> }
/// Turns on every feature this version knows that the project lacks, once `confirm` says yes;
/// changes nothing if it says no, and says so (`SetupError::UpgradeDeclined`).
pub fn upgrade(repo: &mut Repository, on_wait: &mut dyn FnMut(&str),
    confirm: &mut dyn FnMut(&[&'static str]) -> bool) -> Result<Upgraded, SetupError>;
```

- **`parse_format`** accepts `conflicts` as it accepts `chunks`; any other feature is still `NewerFormat`.
- **`write_format`** writes `format_text` of the features the project has, never the constant. The step that adds `chunks` to a project from before milestone 5 (`lock_for_change`) writes the project's features plus `chunks`; it never adds `conflicts`. `FORMAT_TEXT` becomes `format_text` of both features: `begin` (and so `start` and `import`) writes both.
- **`opalro setup --upgrade [--yes]`** (`#[arg(long, conflicts_with = "preset")]`; `yes` today is `#[arg(long, requires = "preset")]`, `main.rs:92`, which would refuse `--upgrade --yes` with exit 2: `preset` and `upgrade` join a clap `ArgGroup` named `change` (`multiple = false`) and `yes` becomes `#[arg(long, requires = "change")]`, so `setup --yes` alone still ends with 2, pinned by a test): says what each feature allows and what it costs, asks (Words; `--yes` answers; with `--json` or without a terminal and without `--yes` it changes nothing, exit 1, hint naming `--yes`), writes under the lock, reads back. Not a diary step: `undo` doesn't take it back, and the words say so before asking. With nothing to turn on, it says so, exit 0.
- **`opalro setup`** without options, in a project with something to turn on, adds the upgrade row (Words); `setup --json` gains `upgrades` (list of strings, `[]` when none).
- §10 gains: `feature conflicts` says the store may hold conflict records (§8) and listings may hold kind-6 entries; `opalro start` and `opalro import` write it; a project made before gains it only through `opalro setup --upgrade`; `format` is always written from the features the project has, and a feature is never taken away.
- `format.json` gains a vector `chunks-and-conflicts`, which `start` writes byte for byte; the milestone 5 vector stays, as one a reader must accept (a project made by v0.1).
- **`support::without_conflicts_feature(root: &Path)`** rewrites `.opalro/format` to the milestone 5 vector's text: a project as v0.1 made it.

- [ ] **Step 1: Write the failing tests**
  - `import.rs` tests (`import.rs:1448`): `start_and_import_write_both_features` (renames and extends `start_and_import_write_feature_chunks`).
  - `repo.rs` tests: `format_file` (`repo.rs:1780`) and `a_changing_command_adds_feature_chunks_before_storing_anything` (`:2031`) compare with `FORMAT_TEXT`, which now lists both features: the first asserts the new text and that the milestone 5 text (chunks only) still parses; the second keeps comparing with `FORMAT_TEXT` for a project `begin` made, and gains a case where a project of the milestone 5 text gains `chunks` only; `a_project_listing_conflicts_opens_and_keeps_them`; `another_feature_is_still_newer` (`feature deltas` → `NewerFormat`); `a_v01_project_never_gains_conflicts_by_itself` (a chunks-only project after a `save`, a `combine`, a `tidy`: `format` byte for byte as before); `the_chunks_step_keeps_conflicts` (a planted `format` with `conflicts` and without `chunks`: after a `save`, both listed, `conflicts` kept); `turning_on_features_writes_and_reads_back`; `keeps_conflicts_is_the_format_line` (with and without).
  - `crates/opal-vcs/tests/setup.rs`: `setup_upgrade_turns_on_conflicts_after_asking` (fake terminal answers `y`); `setup_upgrade_answered_no_changes_nothing`; `setup_upgrade_without_a_terminal_changes_nothing` (exit 1, hint names `--yes`, `format` unchanged); `setup_upgrade_yes_turns_it_on`; `setup_upgrade_with_nothing_to_turn_on_says_so` (a v0.2 project, exit 0); `setup_shows_an_upgrade_when_there_is_one` (and not when there isn't); `setup_upgrade_and_preset_together_are_refused` (exit 2); `setup_yes_alone_is_still_refused` (exit 2, as today); `undo_does_not_take_back_an_upgrade` (undo after it undoes the step before; `format` still lists `conflicts`); with `fault-injection`: `an_upgrade_stopped_before_the_rename_leaves_the_project_as_v01_made_it` (at `write:before-rename`: `format` unchanged, and the project opens).
  - `json_formats.rs`: `setup --upgrade --json` and `setup --json`'s `upgrades` in `every_document`.
  - `tests/vectors.rs`: `start_writes_the_format_vector` checks `chunks-and-conflicts`; `a_v01_format_still_opens`.
- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs --all-features --test setup -- --test-threads=2`
Expected: fails: clap doesn't know `--upgrade`.

- [ ] **Step 3: Edit §10, §13, terminal.md, json.md and the guide; then implement.** Put `tidy_still_stops_at_a_conflict_record` in a project without the feature (its assertions unchanged).
- [ ] **Step 4: Run the tests and see them pass**: `setup`, `json_formats`, `cli` test files; engine `repo` and `tidy` filters; `--test vectors`. Break it once: make `write_format` write the constant again; report which test caught it.
- [ ] **Step 5: Commit**

```bash
git add docs crates/opal-vcs-core/src/repo.rs crates/opal-vcs-core/src/setup.rs crates/opal-vcs-core/src/import.rs crates/opal-vcs-core/src/tidy/tests.rs crates/opal-vcs-core/tests crates/opal-vcs/src crates/opal-vcs/tests
git commit -m "Projects keep conflicts once their format says so: start and import write the feature, and opalro setup --upgrade turns it on after saying what it costs"
```

### Task 3: A combine's limits, `tidy` and `verify` follow a record

**Files:**
- Modify: `docs/storage-format.md` §3 (limits: what a conflict stands as counts) first
- Modify: `crates/opal-vcs-core/src/listing.rs`, `tidy.rs`, `verify.rs`
- Modify: `docs/terminal.md` and `docs/json.md` (`opalro verify`'s new finding), `docs/security.md` (a row, below)
- Test: `listing.rs`, `tidy/tests.rs`, `verify/tests.rs`

**Interfaces:**
- Consumes: `conflict::{Record, Term, stands_as}`; `Store::conflict`; `Repository::keeps_conflicts`.
- Produces:

```rust
// listing.rs
impl SaveLimits {
    /// `add_all`, with each conflict entry counted as what it stands as (`stand` reads that from
    /// the record): a folder it stands as is counted with everything in it. Gives the number of
    /// kind-6 entries met, a listing again for each path it is reached by.
    pub(crate) fn add_all_standing<E>(&mut self, id: Id, depth: usize, at: usize,
        read: impl FnMut(&Id) -> Result<Listing, E>,
        stand: impl FnMut(&Id) -> Result<Option<Term>, E>) -> Result<Result<u64, String>, E>;
}
// `Measure` gains `conflicts: u64`, multiplied by path as `entries` is.

// verify.rs
pub enum Finding { /* as today, and: */
    /// A conflict whose first term or own version is a folder holding a conflict below it.
    NestedConflict { record: Id },
}
```

- **Who measures this way:** only the combine, for its result (Task 9), which reads every listing of the result there already; and `verify`'s per-save size check. `add_all` stays as it is for every other walk. The rule ([decision 179](../decisions/179-every-command-and-a-conflict.md), "a save's limits"): a kind-6 entry counts as one entry, as today; when it stands as a folder, that folder is counted below it as if it were the entry's own (depth and paths included); standing as a file, a link or nothing adds nothing more. One `stand` call per record (the caller remembers it).
- **`tidy`** (only when `repo.keeps_conflicts()`; the source struct `tidy::needs` takes gains `conflicts: bool`, set as `signed` is): the survey no longer stops at a kind-19 object (`tidy.rs:407`); the mark keeps a queue of records beside its queue of listings; a kind-6 entry (`tidy.rs:1023`) pushes its record; each record is read once (`self.read(&id, ObjectKind::Conflict, "conflict record")`), decoded (a refusal is damage, as a listing's is: `Unread::Damaged` under `noted`, else the error), and each present term and the own version is followed exactly as a listing entry of that kind (`tidy.rs:1011-1022`): content kept by fingerprint and its chunk lists queued; a folder's listing queued; a project link read. It follows every record whatever its folders hold (ruling C). Without the feature, everything as today.
- **`verify`** (with the feature): `decodes` (`verify.rs:438`) decodes kind 19 with `Record::decode` and `what` (`:451`) names it "a conflict record"; the needs-walk notes each term's size against what is stored, as it notes a listing entry's (`noted.sizes`), and a term whose ID the store holds as another kind as it notes an entry's (`Unread::OtherKind`); the per-save size check (`Finding::TooBig`, `verify.rs:885-905`) counts what a conflict stands as, from the noted shapes and records. **Nesting:** each noted listing shape gains `holds_conflict` (a kind-6 entry in it or in any folder below, worked out over the shapes the walk noted, each once); a record whose first term or own version is a folder whose shape `holds_conflict` is `Finding::NestedConflict` (Words). No count of a save's conflicts is checked (ruling B). Without the feature, everything as today.
- §3 gains the sentence on what a conflict stands as counting toward a save's limits.
- security.md, a new row (T52): a planted record against `tidy`, `verify` and the limits (a term naming a folder as large as a save, nested records, a record of another kind under a term, a record over 16 KiB) and the defences above.

- [ ] **Step 1: Write the failing tests**
  - `listing.rs`: `a_conflict_counts_as_what_it_stands_as` (standing as a folder of 3 entries: 1 + 3, depth and longest path from below it; as a file: 1; as nothing: 1); `kind_6_entries_are_counted_by_path` (a listing holding one conflict, reached by two paths: 2); `a_folder_a_conflict_stands_as_is_held_to_the_depth_limit`; `theirs_folder_term_is_not_counted`.
  - `tidy/tests.rs`: `tidy_keeps_every_version_a_kept_conflict_names` (terms a file, a big file in chunks, a folder of theirs with files, a project link, and a half-fixed own version: every one readable after `tidy`, each chunk too); `tidy_frees_a_record_and_its_versions_once_no_save_holds_it`; `a_record_that_cant_be_read_stops_tidy_as_damage_naming_it`; `tidy_follows_a_planted_nested_record_whole`; `a_folder_term_3000_deep_is_walked_on_a_small_stack` (on a thread of 256 KiB, as the deep-tree tests run).
  - `verify/tests.rs`: `verify_decodes_every_conflict_record` (a planted record of term count 4: one finding, "It doesn't read as a conflict record: …"); `verify_finds_a_term_that_is_missing`; `verify_finds_a_term_of_the_wrong_size`; `verify_finds_a_term_of_another_kind` (a file term whose ID is a listing); `verify_reports_a_conflict_inside_a_conflict` (first term a folder holding a kind-6 entry two levels down: one `NestedConflict`; theirs' folder holding one: none, it never stands in the folder); `verify_counts_what_a_conflict_stands_as_toward_the_limits`; `without_the_feature_verify_reports_as_v01_does` (the listing damaged, the record by its fingerprint only).
- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs-core --all-features tidy -- --test-threads=2`
Expected: `tidy_keeps_every_version_a_kept_conflict_names` fails with `CantTidy`.

- [ ] **Step 3: Edit §3, terminal.md, json.md; implement.**
- [ ] **Step 4: Run the tests and see them pass** (`listing`, `tidy`, `verify` filters). Break it on purpose three ways and report which test caught each: `tidy` skips a record's own version; `tidy` skips a folder term's listing; the limits count a conflict inside theirs' folder term.
- [ ] **Step 5: Commit** (security.md's row)

```bash
git add docs crates/opal-vcs-core/src/listing.rs crates/opal-vcs-core/src/tidy.rs crates/opal-vcs-core/src/tidy/tests.rs crates/opal-vcs-core/src/verify.rs crates/opal-vcs-core/src/verify/tests.rs
git commit -m "tidy keeps everything a kept conflict names, verify decodes every record and reports a conflict inside one, and a combine's limits count what a conflict stands as"
```

### Task 4: A conflict met where its entry is met: the reader, the walk, `status` and `changes` in the engine

**Files:**
- Create: `crates/opal-vcs-core/src/conflict/standing.rs`, `crates/opal-vcs-core/src/conflict/markers.rs` (the reading half)
- Modify: `crates/opal-vcs-core/src/conflict.rs` (`pub(crate) mod standing; pub(crate) mod markers;` and `pub(crate) use standing::Listings;`, so Task 6 reaches `conflict::Listings` from `diff.rs`, `writeout.rs`, `kept.rs`, `shelf.rs` and `setup.rs`; `seen`, a `#[cfg(test)]` planting helper), `snapshot.rs` (`Last`, `build_reading`, `build_with`, `saved_file`'s caller, `read_folder`, `FolderNow`), `snapshot/walk.rs` (`note_folder`, `note_untouched`, `count_kept_folder`, `rebuild_gone`), `status.rs` (`PathChange`, `status.rs:36`), `changes.rs`, `store.rs` (`counted`); `docs/terminal.md` §6 and `docs/json.md` §5 (this task's damage errors: a record that can't be read, one over 16 KiB, a conflict inside a conflict)
- Test: those modules' tests

**Interfaces:**
- Consumes: `Store::conflict`, `stands_as`, `Repository::keeps_conflicts`, `writeout::Folders` (to read a file at a path without following links, as `changes::on_disk` does).
- Produces:

```rust
// conflict/standing.rs
/// How a walk reads a listing: as stored (one save against another), or as it stands in the
/// folder. Records are read only for kind-6 entries of listings read, each once per command.
pub(crate) struct Listings<'a> {
    store: &'a Store,
    unstored: &'a HashMap<Id, Listing>,
    /// `None`: as stored. Else the records read so far, and the folders a conflict stands as
    /// (and those below them), where a kind-6 entry is refused as nesting.
    standing: Option<RefCell<Stood>>,
}
impl<'a> Listings<'a> {
    pub(crate) fn stored(store: &'a Store, unstored: &'a HashMap<Id, Listing>) -> Listings<'a>;
    /// As it stands, in a project that keeps conflicts; as stored in one that doesn't (R2).
    pub(crate) fn standing(repo: &'a Repository, unstored: &'a HashMap<Id, Listing>) -> Listings<'a>;
    /// The listing `id`: from `unstored`, else the store; as it stands, each kind-6 entry
    /// replaced by what its record stands as (its name, the term's kind, ID and size) or left
    /// out when that is nothing. The listing keeps the ID it was asked by. A kind-6 entry in a
    /// listing below a folder a conflict stands as is damage ("a conflict inside a conflict").
    pub(crate) fn read(&self, id: &Id) -> Result<Listing, StoreError>;
    /// The record `id`, read once per command.
    pub(crate) fn record(&self, id: &Id) -> Result<Record, StoreError>;
    /// Whether any entry this reader replaced was written as a file or link: for `goto`'s words.
    pub(crate) fn stood_in(&self) -> bool;
}

// conflict/markers.rs (reading half; Task 8 adds the writing half)
/// Whether `line` (with its ending, if any) is a marker line of `len` `c`s: exactly that many at
/// the start, then a space, `\r\n`, `\n` or the end.
pub(crate) fn is_marker(line: &[u8], c: u8, len: u8) -> bool;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Left { pub opening: u64, pub closing: u64 }
/// The opening and closing marker lines of length `len` in what `read` gives, holding at most
/// `len + 2` bytes of any line.
pub(crate) fn scan(read: &mut dyn io::Read, len: u8) -> io::Result<Left>;

// conflict.rs
/// What the folder shows at a place in conflict ([decision 176](../decisions/176-a-conflict-in-the-folder.md) point 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Seen { Untouched, MarkersLeft(Left), Fixed, ChangedInPlace }
/// `now` is the folder's entry there as the walk built it; `markers` scans the file when asked.
pub(crate) fn seen(c: &Record, now: Option<(EntryKind, Id)>,
    markers: impl FnOnce(u8) -> io::Result<Left>) -> io::Result<Seen>;

// status.rs
pub struct Status { /* as today, and: */
    /// Every place in conflict the comparison met, in path order.
    pub conflicts: Vec<ConflictAt>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictAt { pub path: String, pub record: Id, pub standing: Standing, pub terms: usize,
    /// What the folder shows there, and whether the record has a version of its own: what
    /// `resolve` decides by (Task 13), which `standing` alone can't tell apart.
    pub(crate) seen: Seen, pub(crate) has_own: bool }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// Shown with markers, and markers are left: `left` opening lines (0 when only closing ones are).
    Marked { left: u64 },
    /// Shown with markers, none left: fixed, not yet recorded.
    Fixed,
    /// Your line's version, as the combine left it.
    Yours,
    /// Stands as a version, and the folder holds something else, or a saved change of the person's.
    ChangedInPlace,
    /// Your line has nothing there, and neither has the folder.
    NotInFolder,
    /// In a part of the save this computer can't hold, kept as it is.
    NotHere,
}

// status.rs (`PathChange`, `status.rs:36`, which `unsaved_changes` hands out too) gains
    /// The path is a place in conflict, or below one standing as a folder.
    pub in_conflict: bool,
```

- **`Listings` is the one reader for the folder side** (Tasks 6 and 13 put write-out on it); the walk under `status`, `save` and `resolve` reads its base **as stored** (`Base` unchanged) and settles kind-6 entries itself, below. Folders keep their stored IDs as their identity: `Listings::read` never recomputes one.
- **`seen`** follows [decision 176](../decisions/176-a-conflict-in-the-folder.md) point 4: `now == stands_as` by kind and ID → `Untouched`; else, marker length 7 or more: `now` a file or executable file → `scan` it: any opening or closing line → `MarkersLeft`, none → `Fixed`; anything else there (nothing, a folder, a link) → `Fixed`; else (marker 0) → `ChangedInPlace`.
- **The walk meets a conflict where it settles that name** (ruling A), in a project that keeps conflicts:
  - `WalkState::note_folder` (`walk.rs:839`), which reads the base listing of a folder the walk keeps from its base entry (`:843`, `Some(e) if e.kind == EntryKind::Folder`), also takes a kind-6 base entry whose record stands as a folder: that folder's listing is the base below (its ID into `old_ids` as a folder's is). The record is read there, once (remembered in `WalkState` (`walk.rs:725`) by ID), because the walk is reading that listing anyway. Every listing read below such a folder is checked for kind-6 entries, which are damage ("a conflict inside a conflict", naming the path); `WalkState` keeps the set of folders below a conflict for that.
  - `WalkState::note_untouched` (`walk.rs:871`) reads the record of each kind-6 entry of a listing it notes (it holds that listing already), and keeps them in a `HashMap<Id, Record>` handed to the build as `Last::records`.
  - **Every place the build asks the base's entry for a name** gets it as it stands when it is kind 6: `build_reading`'s `contents.vouched(c, …)` (`snapshot.rs:1570`), so the file cache vouches against the standing version's ID and size; `build_with`'s `was` (`:1697`) for the executable mark and links Git wrote as files; `saved_file`'s caller (`:931`) for the line-ending checks. One helper, `Last::as_base(rel, name) -> Option<Entry>`.
  - **`build_with`** (`snapshot.rs:1681`): where `was` is kind 6, the child's built entry is compared with what the record stands as through `seen`, and `Contents` gains `fn conflict(&mut self, rel: &str, c: &Child, record: Id, conflict: &Record, built: (EntryKind, Id, u64), seen: Seen) -> Result<Option<(EntryKind, Id, u64)>, SaveError>`, which says what goes into the listing. For `Reading` (`status`, `changes`, `plan`) it is the built entry as it is: the listing then differs from the stored one, so the comparison after the walk reaches the place. (`Storing` is Task 7's.) A base kind-6 entry with no child (nothing on disk there) is met after the children, with `built` none, the same way.
  - **Folders gone from disk** (`rebuild_gone`, `walk.rs:2040`): reads every gone listing already; a kind-6 entry there is left to the comparison for `status` (it reaches gone folders through the old side) and to Task 7 for `save`.
  - **What the walk carries unread** (ruling A's question): entries this computer can't hold, and folders kept as they were, are carried as `Kind::Kept` (`note_untouched`) and never compared, so `status` wouldn't reach a conflict there. For an entry kept as it was that is itself kind 6, `note_untouched` already has it: it goes into `FolderNow.kept_conflicts` with its path. For a folder kept as it was, `count_kept_folder` (`walk.rs:2151`) reads every listing below it already (`limits.add_all(… |id| store.listing(id))`, `:2159`); its read closure notes whether any of them holds a kind-6 entry, and only then does `read_folder` walk that folder again (iteratively, held to a save's limits) to name the places, each `Standing::NotHere`. Saved names that are ignored: a file or link is kept unasked and so read (`kept_unasked`, `walk.rs:188`, whose `saved.kind != Folder` takes kind 6 in). **A conflict standing as a folder that an ignore rule now matches** (second-round answer 1): before writing any code, the implementer finds out what the walk does today with a saved folder an ignore rule now matches (`kept_unasked` refuses folders; the rules then judge it) and writes that down in the task's report; a folder a conflict stands as gets exactly the same, by giving `kept_unasked` and the rules' judgement the entry as it stands (`Last::as_base`). Where that is "gone from the folder", the conflict is met with nothing on disk (a version conflict stays, own version absent, Task 7), and `status` reaches it through the comparison. Test: `an_ignored_folder_a_conflict_stands_as_is_treated_as_an_ignored_saved_folder` (the same project once with a plain saved folder and once with a conflict standing as it: `status` and, in Task 7, `save` treat the two alike). `save <paths>` outside the paths carries base entries untouched (`only::narrow`'s `base_child`, `only.rs:154`): right for `save`; `status` never narrows. A project with none of these pays nothing.
- **`read_folder`** keeps its signature; `FolderNow` gains `records: HashMap<Id, Record>` (what the walk read) and `kept_conflicts: Vec<(String, Id)>`.
- **`status`**: the comparison is as today (`for_each_change_in(store, &now.listings, Some(save.root), now.root, …)`, as stored). A change whose old entry is kind 6 is a place: its row in `conflicts` (state from `seen` on the new entry, then: `Untouched` → `Marked` with `left` from scanning the folder's file when the marker length is 7 or more, `Yours` when the record has no own version and stands as something, `NotInFolder` when it stands as nothing and has no own version, `ChangedInPlace` when it has one; `MarkersLeft(l)` → `Marked { left: l.opening }`; `Fixed` → `Fixed`; `ChangedInPlace` → `ChangedInPlace`), never a row in `changes`. A changed path below a place standing as a folder can't occur: the old side holds the kind-6 entry there, and the comparison hands it out whole without going in. `kept_conflicts` and the kept folders' places are added as `NotHere`. Every place is listed; only marked ones are scanned.
- **`unsaved_changes`** (`changes.rs:204`): at a change whose old entry is kind 6, the old side is what the record stands as (read from `now.records`, else `Store::conflict`), `in_conflict` set on the `PathChange` it hands out; a conflict standing as a folder is compared folder against folder, each file below `in_conflict`. (`changes --json` builds each file's document from `main.rs`'s `file_change(c: &PathChange)`, `main.rs:1131`, used at `:1344`; Task 5 adds `in_conflict` there from the `PathChange`; `output::change_json` is unchanged.)

- [ ] **Step 1: Write the failing tests** (plant conflicted saves with `conflict::tests::plant_conflicted(repo, files, conflicts) -> Id`, a `#[cfg(test)] pub(crate)` helper written here)
  - `standing.rs`: `a_listing_as_it_stands_replaces_each_conflict_and_keeps_its_id`; `a_conflict_standing_as_nothing_leaves_no_name`; `a_conflict_standing_as_a_folder_reads_as_that_folder`; `as_stored_reads_kind_6_as_it_is`; `without_the_feature_as_it_stands_is_as_stored`; `each_record_is_read_once_per_reader`; `a_conflict_inside_a_folder_a_conflict_stands_as_is_damage`; `a_record_that_cant_be_read_is_damage_naming_it` (an ESC in the name: escaped).
  - `markers.rs`: `a_marker_line_is_exactly_that_many_then_a_space_or_the_end` (`<<<<<<< a`, `<<<<<<<`, `<<<<<<<\r\n`, `<<<<<<<\n` are; `<<<<<<<<` is not for 7, `<<<<<<<x`, ` <<<<<<<`, `=======`, `|||||||` are not; the same for `>`); `scan_counts_opening_and_closing_lines`; `scan_holds_a_bounded_amount_of_a_long_line` (a 64 MiB line with no break, from a reader that makes it: the scanner's buffer never holds more than `len + 2` bytes).
  - `conflict.rs`: `seen_follows_point_4_of_the_folder_decision` (untouched; markers left; no markers left; removed; a folder there; marker 0 and changed; marker 0 and removed; only the executable mark changed, markers left).
  - `status.rs`: `status_lists_conflicts_with_how_each_stands` (one place of each `Standing` but `NotHere`); `a_conflicts_place_is_not_listed_among_the_changes`; `a_change_at_a_conflicts_place_shows_in_its_row` (a picture changed in place: no change row, its row `ChangedInPlace`); `status_lists_every_conflict` (150 places, all listed); `status_lists_a_conflict_in_a_folder_gone_from_disk`; `status_lists_a_conflict_kept_as_it_was` (an entry, and a folder, this computer can't hold: `NotHere`; on Windows a name like `aux.txt`, on macOS and Windows a pair of names alike but for letter case; on Linux, which can hold both, the test says why it doesn't apply and returns, as the platform tests do, and CI's Windows and macOS runs carry it); `an_ignored_folder_a_conflict_stands_as_is_treated_as_an_ignored_saved_folder`; `an_untouched_conflict_file_is_vouched_for_once_read` (as `snapshot/endings.rs`'s `a_file_the_cache_vouches_for_is_not_read`: a second `status` opens no file); `a_conflict_standing_as_a_folder_is_compared_with_that_folder`.
  - **`a_project_without_conflicts_reads_nothing_more`** (`status.rs` tests; `save` and `goto` in Tasks 7 and 6): two projects of the same files, folders and saves, one with the feature and one without: the listings `Store::listing` reads and the records `Store::conflict` reads during `status` (the `counted` counters) are the same, and no record is read. A third with one conflict deep in one folder: the extra listings read are only the record's folder, if it stands as one.
  - `changes.rs`: `changes_at_a_conflicts_place_compare_with_what_it_stands_as` (`in_conflict` set; the old side is the marked text).
- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs-core --all-features standing -- --test-threads=2`
Expected: does not compile.

- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run the tests and see them pass** (`conflict`, `status`, `changes`, `snapshot` filters). Break it on purpose three ways and report which test caught each: let `Listings::read` keep the kind-6 entry when the record stands as nothing; let `seen` call a version conflict fixed when it changed; read every listing of the save in `status` to find conflicts.
- [ ] **Step 5: Commit**

```bash
git add docs/terminal.md docs/json.md crates/opal-vcs-core/src/conflict.rs crates/opal-vcs-core/src/conflict crates/opal-vcs-core/src/snapshot.rs crates/opal-vcs-core/src/snapshot crates/opal-vcs-core/src/status.rs crates/opal-vcs-core/src/changes.rs crates/opal-vcs-core/src/store.rs
git commit -m "A conflict is met where its entry is met: the folder walk compares it with what it stands as, status lists it with how it stands, and a project without one reads nothing more"
```

### Task 5: `status` and `changes` on the command line

**Files:**
- Modify: `docs/terminal.md` (`opalro status`, `opalro changes`, §6's error for a record that can't be read and for a conflict inside a conflict), `docs/json.md` (`opalro status --json`, `opalro changes --json`), `docs/guide.md` (`opalro status`)
- Modify: `crates/opal-vcs/src/main.rs` (the `status` and `changes` handlers), `output.rs` (status text and JSON, `change_json`)
- Create: `crates/opal-vcs/tests/conflicts.rs`
- Test: `conflicts.rs`, `json_formats.rs`, `terminal.rs`

**Interfaces:**
- Consumes: `Status::conflicts`, `ConflictAt`, `Standing`; `PathChange::in_conflict`.
- Produces: the words below and these fields:
  - `opalro status --json` gains `conflicts`: a list of `{path, standing, places_left, terms}`, every place, in path order, as `changes` lists changes (no cap); `standing` one of `"markers"`, `"fixed"`, `"yours"`, `"changed"`, `"nothing"`, `"not_here"`; `places_left` a number for `"markers"`, else `null`; `terms` a number.
  - Each file in `opalro changes --json` gains `in_conflict` (boolean).
- **`status`'s layout:** after the outcome line, an empty line, then `In conflict (3 places):` and a row a place, before the changes; a row is the path, then what it needs, right-aligned as change rows are; every place is listed, as every change is (rows cut by §7 on a terminal); then the changes as today; the "Next:" line names what applies (Words). With no conflict, nothing changes.
- **`changes`' layout:** before the header of a file `in_conflict`, the line `In conflict: <path> (compared with what your folder held for it after the last save)`; the rest as today.
- **Colour:** the heading as other headings; a place's path blue; `marked places left to fix` yellow, `fixed, not yet recorded` green, `your version` dim, `changed in place` yellow, `not in your folder` dim, `can't be written on this computer` dim.

- [ ] **Step 1: Write the failing tests** in `tests/conflicts.rs` (planting with `support::add_conflict` and `plant_save`, which moves `main` to the planted save without writing the folder; the test then writes into the folder, itself, what each place should show: the marked text, your version, a half-fixed text, a changed picture, nothing. `goto` writes a conflict's place only from Task 6, so no test here goes through it): `status_lists_the_places_in_conflict_first` (exact text of one of each state); `status_says_what_to_do_next_for_what_is_left`; `status_json_lists_conflicts` (snapshot); `status_lists_150_conflicts_all` (text and JSON); `changes_marks_a_file_in_conflict`; `a_hostile_path_in_conflict_is_escaped_in_status` (ESC and U+202E, `assert_escaped`); `a_record_that_cant_be_read_stops_status_naming_the_place`; `a_planted_conflict_inside_a_conflict_stops_status_as_damage`; `without_the_feature_status_shows_a_planted_conflict_as_v01_does` (a v0.1 project: `kind changed`, no conflict heading). Add both documents to `json_formats.rs`'s `every_document`, and `status` over a hostile conflict path to `terminal.rs`'s `project_text_is_escaped_with_colour_on`.
- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs --all-features --test conflicts -- --test-threads=2`
Expected: no `In conflict` heading.

- [ ] **Step 3: Write terminal.md, json.md and the guide; then implement.**
- [ ] **Step 4: Run the tests and see them pass** (`conflicts`, `json_formats`, `terminal`, `guide` test files; snapshots reviewed and accepted). Break it on purpose twice and report which test caught it: list a conflict's place among the changes too; drop `in_conflict` from `changes --json`.
- [ ] **Step 5: Commit** (testing.md: the new test file)

```bash
git add docs crates/opal-vcs/src crates/opal-vcs/tests
git commit -m "status lists the places in conflict first, with what each needs, and changes marks a file in conflict"
```

### Task 6: Writing the folder: write-out, unsaved work, safety copies, `discard`, the shelf, `goto`, `undo`, `redo`

**Files:**
- Modify: `crates/opal-vcs-core/src/diff.rs` (the walk reads through `Listings`), `writeout.rs` (`planned`, `spellings`, `keep_safety_copies`, `Tree`), `kept.rs` (`switch_saves`, `set_aside`, `links_to_leave`, `overlay_with`), `shelf.rs` (`shelf --list`'s comparison), `setup.rs` (the plan of its write), `switch.rs` (`GotoOutcome` gains `holds_conflicts`); the other callers of `Tree::new` and `overlay_with`, which pass `Listings::stored`: `moves.rs:225, 226, 288, 311`, `combine.rs` (`merge_three`'s `Tree`s and `overlay_with`), `shelf.rs:297, 303`, `setup/tests.rs:111`, `writeout/tests.rs:913, 949`
- Modify: `crates/opal-vcs/src/goto_output.rs`, `main.rs` (`goto --json` is built there, beside `"carried"`, `main.rs:2574`), `docs/terminal.md` (`opalro goto`), `docs/json.md` (`opalro goto --json`'s `holds_conflicts`)
- Modify: `crates/opal-vcs/tests/support/mod.rs` (`assert_folder_is_goto_of`)
- Test: `writeout/tests.rs`, `crates/opal-vcs/tests/conflicts.rs`, `undo.rs`, `shelf.rs`, `discard.rs`

**Interfaces:**
- Consumes: `conflict::Listings` (Task 4).
- Produces:
  - **The readers that take `(store, unstored)` read through `&Listings` instead:** `diff`'s `walk` (its one read site, `listing_of`, `diff.rs:324`, and `again`'s `read` closure, `:199`), `writeout::Tree` (`writeout.rs:1950`, its reads in `folder`, `:1982`) and `kept::overlay_with` (`kept.rs:282`). The public `for_each_change`, `for_each_change_in`, `for_each_entry_change_in` and `count_changes` keep their signatures and read **as stored** (`Listings::stored`), so every comparison of two saves (show, history, the combine, verify) is unchanged; each gains a `_with(&Listings, …)` form for the folder side. `Tree::new` and `overlay_with` take the reader in place of `store, unstored`; the combine's calls pass `Listings::stored`.
  - **The folder side reads as it stands** (`Listings::standing(repo, &now.listings)`, one per command step, so each record is read once): `writeout::planned` (`writeout.rs:121`: the comparisons at `:136`, `:144`, `:155`, `:175`, and its two `Tree`s at `:165-166`), `spellings` (`:334`), `keep_safety_copies` (`:379`); `kept::switch_saves` (`kept.rs:75`: its `Tree`s, its two `for_each_entry_change_in`, its `overlay`s), `set_aside` (`:190`), `links_to_leave` (`:386`); `shelf.rs:300-306` (the shelf against the save it was made on); `setup.rs:238`. Their `store.save(&…)?.root` reads stay: a save's root ID is what it is.
  - **Nothing read as it stands is stored under the ID it was read by.** `overlay_with` rebuilds the folders along the paths it changes, from listings read as they stand, and stores each under the ID of its content (`store_listing`); folders it doesn't change keep their stored IDs. So a kept save is a picture of the folder (a conflict's place holds what it stood as), which is right: kept saves picture the folder, and they are read back through the same reader.
  - **Kind 6 in `not_ours`** (`writeout.rs:1576-1578`) stays, for a project without the feature (R2): with the feature, no kind-6 entry reaches the plan.
  - `GotoOutcome.holds_conflicts: bool`: a listing the plan read for the **target** side held a conflict's entry (`Listings::stood_in` of the reader the target is read through; what is read for the side being left never counts: give the two sides a reader each, or have the reader say which side a listing was read for). It is about the save gone to, not about what was written, and may be missing when the folder already held that conflict (second-round ruling 4). Its detail line; `goto --json` gains `holds_conflicts` (boolean). No count (ruling B).
  - **`support::assert_folder_is_goto_of(env: &Env, save: Id)`**: copies the project's `.opalro` into an empty folder, runs `opalro goto <save>` there, and compares the two folders entry by entry, folders and executable marks included, `.opalro` left out.
- What this gives, with no other change: going to a conflicted save writes its marked texts and your versions; leaving one removes them; a file at a place that is what the conflict stands as is not unsaved work, one that differs is; `discard` gives back what the conflict stands as and keeps a safety copy of what was there; a shelf holds the changed file as it holds any, and the conflict stays in the save; `undo` and `redo` write and protect the folder by the same reader, since their folder changes are kept saves and saves read through it.

- [ ] **Step 1: Write the failing tests**
  - `tests/conflicts.rs`: `goto_a_conflicted_save_writes_marked_texts_and_your_versions` (then `assert_folder_is_goto_of`); `goto_away_removes_them_and_goto_back_writes_them_again`; `an_untouched_conflict_file_is_not_unsaved_work` (`goto` elsewhere: done, no safety copy); `a_half_fixed_file_stops_a_goto_it_is_in_the_way_of` (exit 1, the path named, folder and line unchanged); `a_changed_version_conflict_is_unsaved_work_too`; `discard_gives_back_the_marked_text_and_keeps_a_safety_copy` (the copy holds the half-fixed bytes; `assert_folder_is_goto_of`); `discard_below_a_conflict_that_stands_as_a_folder_finds_the_path`; `shelf_and_unshelf_carry_a_half_fixed_file_and_the_conflict_stays_in_the_save`; `undo_and_redo_over_a_goto_to_a_conflicted_save`; `goto_says_when_the_save_gone_to_holds_a_conflict` (and says nothing when leaving a conflicted save for one without); `without_the_feature_write_out_leaves_a_conflicts_place_alone` (a v0.1 project: today's behaviour); with `fault-injection`: `a_goto_to_a_conflicted_save_stopped_at_every_step_is_finished_or_never_happened` (each of `goto`'s fault points, as `goto.rs` runs them).
  - `writeout/tests.rs`: `goto_in_a_project_without_conflicts_reads_nothing_more` (as Task 4's test, for a `goto` between two saves: the same listings read with and without the feature, no record); `a_kept_save_names_only_stored_listings` (after `discard` below a conflict standing as a folder: every listing the kept saves name is in the store, under the ID of its content); `two_saves_compared_as_stored_still_see_kind_6` (`for_each_change` between a save and its conflicted child hands out the kind-6 entry); `a_folder_term_is_written_under_write_outs_name_rules` (a planted record standing as a folder holding a `.opalro` spelling: refused as `OpalName`, as any save's would be); `a_link_term_leading_out_is_written_as_a_placeholder`.
  - `crates/opal-vcs/tests/corpus_conflicts.rs` (new, with `support/corpus.rs`'s helpers): `goto` onto a planted record whose terms name a missing object, a listing under a file term, a 3,000-deep folder term, a record over 16 KiB, and a conflict inside a folder a conflict stands as: each refused as damage naming its place, or written within limits, and nothing written outside the project.
- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs --all-features --test conflicts -- --test-threads=2`
Expected: the marked text isn't written.

- [ ] **Step 3: Implement**, and edit terminal.md and json.md.
- [ ] **Step 4: Run the tests and see them pass** (`conflicts`, `corpus_conflicts`, `goto`, `undo`, `shelf`, `discard`, `setup`, `json_formats`, `combine` test files; `writeout`, `kept`, `diff` filters). Break it on purpose twice and report which test caught it: `switch_saves` reading as stored; `overlay_with` storing a listing under the ID it was read by.
- [ ] **Step 5: Commit** (testing.md: `corpus_conflicts.rs`; security.md: T53, a planted conflict against write-out, T1 to T5's defences holding for terms)

```bash
git add docs crates/opal-vcs-core/src/diff.rs crates/opal-vcs-core/src/writeout.rs crates/opal-vcs-core/src/kept.rs crates/opal-vcs-core/src/shelf.rs crates/opal-vcs-core/src/setup.rs crates/opal-vcs-core/src/switch.rs crates/opal-vcs-core/src/combine.rs crates/opal-vcs-core/src/writeout/tests.rs crates/opal-vcs-core/src/moves.rs crates/opal-vcs-core/src/setup/tests.rs crates/opal-vcs/src/goto_output.rs crates/opal-vcs/src/main.rs crates/opal-vcs/tests
git commit -m "goto, discard, the shelf, undo and redo read both saves as they stand, so a conflict's place is written and protected as what it stands as"
```

### Task 7: `save` on a folder with conflicts

**Files:**
- Modify: `crates/opal-vcs-core/src/snapshot.rs` (`Storing`'s `Contents::conflict`; `save_folder` and the inner `save_recording` it now wraps), `snapshot/walk.rs` (`rebuild_gone`), `snapshot/only.rs` (`narrow`'s base reads below a conflict standing as a folder), `conflict.rs` (`Recording`, `Settled`), `moves.rs` (`count_and_moves` passes places over)
- Modify: `docs/terminal.md` (`opalro save`), `docs/json.md` (`opalro save --json`'s `conflicts`)
- Modify: `crates/opal-vcs/src/main.rs` (the save outcome's detail), `output.rs`
- Test: `snapshot.rs`'s tests, `crates/opal-vcs/tests/conflicts.rs`, `amend.rs`

**Interfaces:**
- Consumes: Task 4's walk (`Last::records`, `Contents::conflict`, `seen`), `markers::scan`.
- Produces:

```rust
// conflict.rs
/// How a save records the places it meets: none fixed for `save`; for `resolve`, the places to
/// record as the folder shows them.
#[derive(Debug, Default)]
pub(crate) struct Recording { pub fixed: BTreeSet<String> }

/// What a save puts at a place in conflict ([decision 177](../decisions/177-recording-a-fix-save-and-resolve.md) point 1's table).
pub(crate) enum Settled {
    /// The conflict as it is: its kind-6 entry, the same record.
    Kept,
    /// The conflict with a new record: the same terms, its own version what the folder holds.
    Again(Record),
    /// What the folder holds: an ordinary entry, or nothing.
    Fixed,
}
/// `built` is the entry the walk built at the place (kind, ID, size), or `None` for nothing there;
/// a new record keeps `c`'s terms, takes `built` as its own version, and `c`'s marker length when
/// `seen` is `MarkersLeft` (0 for `ChangedInPlace`).
pub(crate) fn settled(c: &Record, seen: Seen, built: Option<(EntryKind, Id, u64)>, fixed: bool) -> Settled;

// snapshot.rs: `Built` (build_with's, `snapshot.rs:1607`) gains
    /// The places it met that are still in conflict.
    met_conflicts: u64,
// save_folder (`snapshot.rs:451`) becomes a wrapper, its signature unchanged, so none of the
// 22 files that build a `SaveRequest` changes:
pub fn save_folder(repo, req, on_wait, confirm_large) -> Result<SaveOutcome, SaveError> {
    save_recording(repo, req, &Recording::default(), on_wait, confirm_large)
}
/// `save_folder`, settling the places `recording` names as fixed; `resolve`'s way in (Task 13).
pub(crate) fn save_recording(repo: &mut Repository, req: &SaveRequest, recording: &Recording,
    on_wait: &mut dyn FnMut(&str), confirm_large: &mut dyn FnMut(&LargeSave) -> bool)
    -> Result<SaveOutcome, SaveError>;
// SaveOutcome::Saved gains
    /// The places in conflict the save met, still in conflict in it.
    conflicts: u64,
```

- **The table** ([decision 177](../decisions/177-recording-a-fix-save-and-resolve.md) point 1), in `Storing`'s `Contents::conflict`, where the walk meets the place (Task 4): `Untouched` → `Kept`; `MarkersLeft` → `Again` (own version the file as the folder holds it, the same marker length); `Fixed` → `Fixed`; `ChangedInPlace` → `Again` (own version what the folder holds, absent when nothing; marker 0). A place in `recording.fixed` is `Fixed` whatever it shows (only `resolve` fills it). `Kept` gives the stored entry back, so an untouched folder's listing equals the stored one and keeps its ID (`build_with`'s `is_listing`, `snapshot.rs:1753`); `Again` adds the new record to the pack set (`ObjectKind::Conflict`) and gives its entry. Each `Kept` or `Again` adds one to `met_conflicts`. No list of places, no second pass, nothing put back afterwards (ruling A).
- **A conflict standing as a folder** is walked with that folder as base (Task 4), so its files are settled as files below, and the folder's built entry is compared with the folder it stands as: the same ID → `Kept`; else `Again`, own version the built folder.
- **Gone from disk:** a base kind-6 entry with nothing on disk is met after the children (Task 4); in a folder gone from disk, `rebuild_gone` (`walk.rs:2040`) keeps a kind-6 entry whose record stands as a version (marker 0), as it keeps an entry it can't hold, with `Again` and an absent own version unless it already stands as nothing (then `Kept`), and the folders above it come back (question 2's answer: the folder stays, standing empty); a markers place there is fixed by removal.
- **The search of folders gone from disk, once spent** (`Base::gone_listing`, `walk.rs:480`: past `limits` "the whole search ends there… and the save goes ahead"; second-round answer 2): a kind-6 entry below where it stopped is never met, and the save goes ahead without it, as it does today for entries kept as they were. Only a base save already past a save's limits gets there. Test: `a_spent_search_of_gone_folders_goes_ahead_as_today` (a base save planted past the limits with `SaveLimits::new` lowered as `walk.rs`'s own tests lower it: the save is made, without the conflict below, and the save before still holds it).
- **`save <path>` inside a conflict standing as a folder** (second-round answer 3): `only::narrow` (`snapshot/only.rs:50`) reads the base below a named path with `store.listing(&e.id)` only for `e.kind == EntryKind::Folder` (`only.rs:131`, `:138`, `:140`); it takes a kind-6 entry whose record stands as a folder the same way, reading that folder's listing as the base below (the record from `Last::records`, or read once there), with the nesting check of Task 4, so the result at the place goes through `Contents::conflict` as a full save's does. Test: `saving_a_path_inside_a_conflict_standing_as_a_folder_goes_as_a_full_save_does` (`ui/` stands as your folder; `save ui/a.txt` after changing it: the save holds a conflict at `ui` whose own version is the folder with `a.txt` changed and the rest as it stood; `save ui/b.txt` untouched: nothing to save). If that needs more than this shared rule, the implementer stops and says so; the answer is then to refuse a path below a conflict's place ("save the place whole, or resolve it").
- **What the walk carries unread** (`Kind::Kept`, `Kind::Left` outside `save <paths>`, `Kind::Back`'s held entries) carries kind-6 entries as they are, as it carries project links: right for `save`, and not counted as met.
- **`Recording`** reaches `Storing` through `save_recording`'s parameter, not through `SaveRequest`: `save_folder` passes `Recording::default()`. Test here: `a_place_recorded_as_fixed_is_an_ordinary_entry` (calling `save_recording` with one place in `fixed`: that place is the folder's entry, the others settled by the table).
- **The marker check reads the stored bytes:** for a markers place whose entry differs, the file is opened again through the walk's folders and scanned while its content ID is worked out; if the ID differs from the walk's, the save stops with `SaveError::ChangedDuringSave { path }` (today's words, `snapshot.rs:140`). Fault point `save:settling` before it.
- **`save_folder`** (`snapshot.rs:451`): as today; `count_and_moves` (`moves.rs:84`) takes `passed_over: &dyn Fn(&str) -> bool`, which `save` sets to "a kind-6 entry in the base or in the new save at this path or above it" (read from the listings `count_and_moves` holds already): a conflict's place is never a file that left or arrived. The outcome carries `met_conflicts` as `conflicts`; the detail and `save --json`'s `conflicts` (Words) say how many places the save met that are still in conflict; a save narrowed to paths says nothing of the rest.

- [ ] **Step 1: Write the failing tests**
  - `snapshot.rs` tests: `saving_an_untouched_conflicted_folder_saves_nothing_new` (`NothingChanged`, and the folders' listings not rebuilt: `LISTINGS_BUILT` empty); `a_half_fixed_file_is_saved_as_the_conflicts_own_version` (its marker length kept; `status` then shows it marked with the places left); `a_file_with_no_markers_left_is_saved_as_fixed`; `a_removed_marked_file_is_saved_as_fixed`; `a_changed_picture_stays_a_conflict_with_the_changed_file_as_its_own`; `a_removed_picture_stays_a_conflict_with_nothing_as_its_own`; `a_picture_in_a_deleted_folder_stays_a_conflict_and_the_folder_stays`; `a_changed_file_inside_a_conflict_standing_as_a_folder_makes_it_changed_in_place`; `save_counts_the_conflicts_it_met` (and `--amend`; `save a.txt` with a conflict in `b/`: says 0, and `b/`'s conflict is still in the save, carried); `a_conflict_this_computer_cant_hold_is_carried` (where the drive can't hold it, as Task 4's `status_lists_a_conflict_kept_as_it_was` chooses the name; on Linux it says why it doesn't apply); `a_place_recorded_as_fixed_is_an_ordinary_entry`; `a_spent_search_of_gone_folders_goes_ahead_as_today`; `saving_a_path_inside_a_conflict_standing_as_a_folder_goes_as_a_full_save_does`; `an_ignored_folder_a_conflict_stands_as_is_saved_as_an_ignored_saved_folder` (Task 4's case, for `save`); `a_conflicts_place_is_never_noted_as_a_move` (a fixed file whose contents left another path); `save_in_a_project_without_conflicts_reads_nothing_more` (as Task 4's test, for `save`); `without_the_feature_save_replaces_a_planted_conflict_as_v01_does`; with `fault-injection`: `a_file_changed_between_the_walk_and_the_marker_check_stops_the_save` (the act point writes markers into a file the walk read without them: `ChangedDuringSave`, nothing stored), `a_save_stopped_at_every_step_stores_nothing_named`.
  - `tests/conflicts.rs`: `save_says_how_many_conflicts_it_met`; `save_json_counts_them`; after each save in this file, `assert_folder_is_goto_of(env, tip)`.
- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs-core --all-features snapshot -- --test-threads=2`
Expected: the half-fixed file is saved as a plain file.

- [ ] **Step 3: Implement**, and edit terminal.md and json.md.
- [ ] **Step 4: Run the tests and see them pass** (`snapshot`, `moves` filters; `conflicts`, `amend`, `json_formats` test files). Break it on purpose three ways and report which test caught each: `ChangedInPlace` saved as fixed; `Kept` giving a re-encoded record instead of the stored entry (the folder's ID changes); `rebuild_gone` dropping a picture's conflict.
- [ ] **Step 5: Commit**

```bash
git add docs crates/opal-vcs-core/src/snapshot.rs crates/opal-vcs-core/src/snapshot crates/opal-vcs-core/src/conflict.rs crates/opal-vcs-core/src/moves.rs crates/opal-vcs/src crates/opal-vcs/tests
git commit -m "save carries conflicts along where it meets them: a half-fixed file is kept as the conflict's own version, a picture changed in place stays to decide"
```

### Task 8: The marked text

**Files:**
- Modify: `crates/opal-vcs-core/src/conflict/markers.rs` (the writing half), `combine/text.rs`, and every caller of `merge_text`, which gains a `None`: `combine/tree.rs` (`contents`, `tree.rs:400`), `combine/tests.rs:1349`, `crates/opal-vcs/tests/merge_vs_git.rs:162, 227` (`model.rs:2548` calls the model's own `merge_text`, not the engine's: unchanged)
- Test: `markers.rs`, `text.rs`

**Interfaces:**
- Consumes: `changes::{MAX_LINES_SIZE, lines_of}`; `text::escape`; `id::ChangeId::to_letters`.
- Produces:

```rust
// conflict/markers.rs
/// The three labels of a marked text, each one line of plain text of at most 200 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Labels { pub ours: String, pub base: String, pub theirs: String }
/// What the middle label says of where the lines parted, at one place: it can differ from place
/// to place in one combine (a place with nothing at the start, or one the start can't tell).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parted { One(ChangeId), Several, Nothing(Option<ChangeId>), Unknown }
/// What every label of one combine is made from: the two names and saves, and where the lines
/// parted. Held once for the combine; the labels are made from it for each file merged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sides {
    /// The names as the person combined by, shown already (escaped, or `tags::shown_name`).
    pub ours: String, pub ours_save: ChangeId,
    pub theirs: String, pub theirs_save: ChangeId,
    /// Where the lines parted: one save, or several.
    pub parted: Parted,
}
impl Sides {
    /// The labels for one file: `here` is what the middle says at that place (`self.parted`
    /// where the start holds a file, `Nothing(…)` where it holds nothing, `Unknown`).
    pub fn labels(&self, here: Parted) -> Labels;
}
/// 7, or one more than the longest run of `<`, `|`, `=` or `>` starting a line in any of
/// `texts`; `None` past 255.
pub fn marker_len(texts: [&[u8]; 3]) -> Option<u8>;

// combine/text.rs
pub enum Text {
    Merged(Vec<u8>),
    /// The merge with each clashing place written with markers of `marker` characters.
    Marked { bytes: Vec<u8>, marker: u8, places: u64 },
    Not(NotMerged),
}
/// As in milestone 8 when `labels` is `None`; with labels, a clash is `Marked` unless the two
/// differ only in line endings, the marker would pass 255, or the marked text 64 MiB.
pub fn merge_text(base: &[u8], ours: &[u8], theirs: &[u8], deadline: Duration,
    labels: Option<&Labels>) -> (Text, bool);
```

- **The marked text** ([decision 176](../decisions/176-a-conflict-in-the-folder.md) point 3): `settle` (in `text.rs`) goes on past a clashing region instead of returning `None`, writing `<`×L, a space, the ours label, the line's end; ours' lines for the region; `|`×L, space, base label, end; the base's lines (`base.get(region)`); `=`×L, end; theirs' lines; `>`×L, space, theirs label, end. The middle part is always there. Each marker line ends as ours' first line ends (`\r\n` or `\n`; ours with no line break: theirs'; neither: the base's; none: `\n`); where a part's last line has no line break, the same ending is added before the next marker. `places` counts the regions so written.
- **Labels:** `<line name> (<8 letters>)`; the middle: `where the lines parted (<8 letters>)`, `where the lines parted (several saves)`, `where the lines parted (<8 letters>), which held nothing here`, `where the lines parted, which held nothing here` (a made-up start), `where the lines parted, which Opalro can't tell here`. Each passes through `escape(…, false)` piece by piece and is cut to 200 bytes at a character boundary. `Sides` is made once per combine; `Sides::labels(here)` once per file merged, with the middle for that place (ruling round 3: labels are given for each file, not held once). Test: `the_middle_label_differs_from_place_to_place` (one combine, three places: the start holds the file; holds nothing; can't tell: three middle labels).
- The marked text is held as the merged text is: the caller writes it to its pack set at once and drops it.

- [ ] **Step 1: Write the failing tests**
  - `text.rs`: `a_clash_is_written_with_markers_and_labels` (exact bytes); `the_middle_part_is_always_there_and_says_when_nothing_was`; `marker_length_grows_past_the_files_own_runs` (a line of nine `<` in theirs: 10; a `=======` underline in ours: 8); `past_255_the_clash_is_not_marked`; `marker_lines_end_as_your_first_line_ends` (CRLF ours; ours without a break, theirs CRLF; neither, base CRLF; none); `a_part_without_a_last_line_break_gets_one_before_the_next_marker`; `endings_only_is_not_marked`; `a_marked_text_past_the_limit_is_not_marked` (the limit is a parameter of an inner function the test lowers); `without_labels_nothing_changes` (milestone 8's tests, unchanged, through `labels: None`).
  - `markers.rs`: `labels_are_plain_one_line_and_at_most_200_bytes` (a name with ESC, `\n`, U+202E and 500 bytes of `é`: escaped, one line, 200 bytes or fewer, cut between characters).
  - **Property tests** (`text.rs`, 256 cases each, fixed seeds; the generator is milestone 8's for `merge_text` plus lines that look like markers: runs of 1 to 300 of each marker character, alone or followed by text, with CRLF, LF and no last break; each kind of line asserted to occur at least 3 times):
    - `every_marked_text_reads_back_with_its_places`: `markers::scan(bytes, marker)` gives `opening == closing == places`, and none of the three versions has a marker line of that length.
    - `taking_one_side_in_every_marked_place_gives_that_side` (the generator makes every change of theirs overlap one of ours, so every region both changed is a clash): a parser written in the test (splitting on lines of exactly `marker` of each character, sharing no code with the engine) taking ours' part everywhere gives `ours` byte for byte, theirs' gives `theirs`, the middle gives `base` (marker lines' added breaks removed where the part had none).
    - `taking_ours_everywhere_agrees_with_a_model` (any changes): ours' parts everywhere give what a three-way merge written in the test (longest common subsequence by dynamic programming, as `model.rs`'s, over texts of lines that don't repeat) gives with every clash settled by ours.
- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs-core --all-features combine::text -- --test-threads=2`
Expected: does not compile.

- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run the tests and see them pass**, then `--test merge_vs_git`. Break it on purpose twice and report which test caught it: marker length 7 always; the middle part left out when empty.
- [ ] **Step 5: Commit**

```bash
git add crates/opal-vcs-core/src/conflict/markers.rs crates/opal-vcs-core/src/combine/text.rs crates/opal-vcs-core/src/combine/tree.rs crates/opal-vcs-core/src/combine/tests.rs crates/opal-vcs/tests/merge_vs_git.rs
git commit -m "The text merge writes a clash with markers: labels, a middle part always, markers longer than any line that looks like one, and the file's own line endings"
```

### Task 9: The tree merge keeps conflicts

**Files:**
- Modify: `crates/opal-vcs-core/src/combine.rs` (`Keep`, `Stands`, `Conflict.stands`, `CombineError::HeldConflict`, `Combined::Made`'s new fields, `make_save`), `combine/tree.rs` (`decide`, `files`, `contents`, `conflict`, `record_at`, `Made`, `Merged`), `combine/tests.rs`, `rules.rs` (`read_saved`'s name lookup)
- Modify: `crates/opal-vcs/src/main.rs` (passes `Keep::OnlyIfClean` until Task 12), `crates/opal-vcs/tests/model.rs` (its engine call `combine::combine(…)` at `model.rs:2079` gains `Keep::OnlyIfClean`, and its `Combined::Made { … }` at `:2124` the three new fields, empty: the model learns to keep conflicts in Task 16)
- Modify: `docs/terminal.md` and `docs/json.md` (this task's words: `HeldConflict`'s error and the `start_unknown` row; the 172 stop's words where conflicts are kept), `docs/security.md` (T46's row: a kept conflict's memory)

**Interfaces:**
- Consumes: `Record`, `Sides`, `Parted`, `merge_text(…, Some(&labels))`, `SaveLimits::add_all_standing`, `Repository::keeps_conflicts`; the matcher as milestone 8 left it (`rules.rs:90-145`): `RuleMatcher::new(ours: &RulesRead, theirs: &RulesRead)`, `line_merge(path, was) -> LineMerge { Yes, RuleSaysNo { theirs }, RulesNotWhole { theirs } }` (decision 173: both lines' rules, under both names of a moved file); `NotMerged::{NoMarkers { theirs }, RulesNotWhole { theirs }}` (`combine.rs:84-103`); `Merged.rules_not_whole: Option<Side>` (`tree.rs:75`); `CombineError::Conflicts { names, conflicts, more, rules_not_whole }` (`combine.rs:254-259`) and its hints (`combine.rs:326-375`), all kept.
- Produces:

```rust
// combine.rs
/// Whether a combine keeps the conflicts it meets, or stops as milestone 8's did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep { Conflicts, OnlyIfClean }
/// How a kept conflict stands in the folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stands { Markers { places: u64 }, Yours, Nothing }
pub struct Conflict { pub path: String, pub clash: Clash, /** `None` when not kept. */ pub stands: Option<Stands> }
pub enum Clash { /* milestone 8's, and: */ StartUnknown }
pub enum CombineError { /* as today, and: */
    /// Both lines changed `path`, and `on` holds it in conflict (`at == path`) or holds the
    /// conflict `at` below it.
    HeldConflict { names: Names, path: String, at: String, on: Held },
}
pub enum Held { Ours, Theirs, Both }
pub enum Combined { /* … */ Made { starts: Vec<Id>, merged_files: Vec<String>,
    /// The places this combine made, the first 100 in path order, and how many more.
    conflicts: Vec<Conflict>, more: u64,
    /// Every kind-6 entry the combined save holds, counted where its result is measured.
    conflict_count: u64 } }
pub fn combine(repo: &mut Repository, target: &Target, named: &Named, message: Option<&str>,
    who: &Person, git_excludes: Option<&Path>, keep: Keep, on_wait: &mut dyn FnMut(&str))
    -> Result<CombineOutcome, CombineError>;

// combine/tree.rs: Made gains
    /// The records this combine made, by ID, for measuring its result.
    pub records: HashMap<Id, Record>,
    /// Whether conflicts are kept (the project keeps them, and `Keep::Conflicts`), with what
    /// every label of this combine is made from; each file's labels come from it.
    pub keep: Option<Sides>,
// Walk gains, for Task 10's move clashes and Task 11's unknown places too:
/// Makes the record of a conflict at `path`: `terms` in the record's order (yours, where the
/// lines parted, theirs), its own version if any, into the pack set and `made.records`; checks
/// first that no folder among yours and theirs holds a conflict below it (`HeldConflict`);
/// names the conflict (`self.conflict`) with how it stands. Gives the kind-6 entry.
pub(super) fn record_at(&mut self, path: &str, name: String, clash: Clash,
    terms: [Option<Entry>; 3], own: Option<(Entry, u8)>) -> Result<Entry, CombineError>;
```

- **Where the merge keeps** (`!inner` and `made.keep` is `Some`): where milestone 8 called `self.conflict(path, clash)` and returned `None`, the walk calls `record_at` and returns its entry (`Three`'s order is `[start, ours, theirs]`, `tree.rs:86`; the record's is yours, start, theirs: `record_at` takes the record's order); a folder term is the folder's entry, whole. Own version, asking `self.rules.line_merge(path, self.moved.get(path))` as `contents` does today (`tree.rs:407-409`):
  - both changed, both files, start a file: `Yes` → `merge_text` with `sides.labels(here)` (`here`: the start's change ID); `Marked` → its bytes stored (`store_held`), own = that file with the kind `files()` gives, marker as given, `Stands::Markers`; otherwise no own version, `Stands::Yours`. `RuleSaysNo` → no markers, `Stands::Yours` (decision 173: either line's rule says no);
  - both added, both files: the same, with `merge_text(b"", o, t, …, Some(&sides.labels(Parted::Nothing(…))))`; the marked text's kind is ours';
  - anything else (not text, too big, kinds differ, a side absent): no own version; `Stands::Yours`, or `Stands::Nothing` when ours has nothing there.
  The text limits and an endings-only difference give no own version.
- **Rules that can't be read whole (D2, decision 172 kept):** while `line_merge` says `RulesNotWhole` for a place where the rules would decide (both sides text files within the limits: their changes would merge **or** overlap; both added text files too), the combine **stops**, changing nothing, even where other places could be kept: `contents` (`tree.rs:400-451`) names such a place `Clash::BothChanged(NotMerged::RulesNotWhole { theirs })` and sets `merged.rules_not_whole` before or after the text merge alike (today it does so only for a file that would have merged, `tree.rs:432-436`; an overlapping one keeps its own reason, which is right only when nothing is kept); with `Keep::Conflicts`, `make_save` then returns `CombineError::Conflicts { rules_not_whole: Some(side), … }` naming only those places, with decision 172's hints (`combine.rs:332-353`) and Task 12's words. Every other clash in such a combine (not text, too big, kinds differ, a side absent) would be kept as a conflict standing as yours, as with any rules, but the stop comes first. Under `Keep::OnlyIfClean` and in a project without the feature, everything stays as milestone 8 left it, word for word.
- **The other side's rules file, by name** (re-review of milestone 8): `rules::read_saved` (`rules.rs:501`) finds the other side's file as the entry named `opalro.toml`; with none, the one entry whose name differs from it only in the case of ASCII letters, on every system; with several such, the rules count as not read whole (`skipped(…)` with the reason, so no file is merged line by line and none is shown with markers). A missing or damaged object for that file is the store's error, never "no rules" (as today: `read_capped`'s `?`).
- **A conflict a line already holds** ([decision 175](../decisions/175-what-a-combine-keeps-as-a-conflict.md) point 4, widened by ruling C): before any rule in `decide` (`tree.rs:336`), if ours or theirs is a kind-6 entry, the combine stops with `HeldConflict` (nothing stored, as any refusal). And `record_at`, before making a record one of whose terms (ours' or theirs') is a folder, walks that folder alone (iteratively, its listings through the walk's `listing`, `tree.rs:577`, held to that version's limits) for a kind-6 entry anywhere below; one found stops the combine the same way, naming the place and the conflict below (`at`). So a combine never makes a conflict inside a conflict; `save` makes none, and a combine can't make one below a place that is one already, since that place's entry differs on the two lines. Carried and fixed conflicts never reach `decide`: `one_sides` (`tree.rs:179`) already takes what only one line changed, a kind-6 entry compared as stored.
- **The start holding a conflict** both lines changed (`s` kind 6, `o` and `t` differ from it and from each other, neither kind 6): the place is a conflict of `[o, absent, t]` with `Clash::StartUnknown` and `Parted::Unknown` (Task 11 gives `Unknown` its full meaning; here only this case).
- **`make_save`** (`combine.rs:878`): `made.keep = (keep == Keep::Conflicts && repo.keeps_conflicts()).then(|| sides)`, `sides` from `Names` and the line's and target's change IDs, `parted` from the starting points (one: its change ID; several or a made-up one: `Several`). Conflicts with `keep` off stop as today (`CombineError::Conflicts`, `combine.rs:922-927`). With it on and no 172 stop, the result is measured with `add_all_standing` where `make_save` measures it today (`combine.rs:934`, which reads every listing of the result already; the stand read from `made.records`, else `Store::conflict`), and the kind-6 entries it meets are the count: carried ones included, no extra walk. Nothing goes into the save's extra fields (ruling B). `Combined::Made` carries the made conflicts, `more` and the count.

- [ ] **Step 1: Write the failing tests** (`combine/tests.rs`; milestone 8's conflict tests keep their assertions by combining with `Keep::OnlyIfClean`, and say so in one line each: `a_conflict_stops_and_stores_nothing` (`:3639`), `while_the_rules_are_not_read_whole_no_file_is_merged_line_by_line` (`:4065`), `a_rule_that_can_match_no_file_stops_line_merging_until_it_is_fixed` (`:4107`), `a_file_that_clashes_for_a_reason_of_its_own_keeps_it_while_the_rules_are_not_read_whole` (`:4129`), `the_hint_names_the_rules_file_when_only_a_counted_conflict_is_there_for_it` (`:4176`))
  - `both_changed_text_is_kept_with_markers` (the record's terms `[ours, start, theirs]`, its own version the marked text with marker 7, the entry kind 6, `Stands::Markers { places: 1 }`, the marked text stored); `both_changed_binary_stands_as_yours` (and `too_big`, `too_many_lines`, endings only: no own version); `a_rule_on_either_line_saying_no_markers_keeps_it_as_yours` (the folder's rule, then only the target's: no own version, decision 173); `a_moved_file_is_marked_only_when_both_its_names_allow_it`; `changed_and_removed_stands_as_your_version_or_nothing`; `both_added_text_gets_markers_with_an_empty_middle`; `both_added_binary_stands_as_yours`; `a_file_against_a_folder_holds_the_folder_whole`; `two_links_that_differ_are_kept_standing_as_yours`; `the_marked_text_has_the_executable_mark_the_merge_gives`.
  - **D2:** `while_the_rules_are_not_read_whole_a_text_both_changed_stops_the_combine` (under `Keep::Conflicts`: a text whose changes would merge, and one whose changes overlap, each stops it: `Conflicts { rules_not_whole: Some(_) }`, nothing stored, the hint 172's; the folder's file and the target's, each); `while_the_rules_are_not_read_whole_a_picture_is_still_kept_as_yours` (no text place: the combine goes through, the picture kept, standing as yours); `the_stop_names_only_the_places_the_rules_would_decide`.
  - **The other side's file by name** (`rules.rs` tests): `the_other_sides_rules_file_is_found_by_its_exact_name`; `failing_that_by_a_name_differing_only_in_letter_case` (`Opalro.toml`, on every system); `several_names_alike_but_for_case_count_as_not_read_whole`; `a_missing_or_damaged_rules_file_object_is_the_stores_error`.
  - `a_conflict_both_lines_hold_alike_is_carried`; `a_conflict_one_line_fixed_is_taken_fixed`; `a_conflict_made_on_one_line_only_is_carried`; `both_lines_changed_a_place_one_holds_in_conflict_stops_naming_it` (ours, theirs and both; nothing stored, as `a_conflict_stops_and_stores_nothing` checks); `a_folder_holding_a_conflict_below_stops_the_combine_too` (ours holds `ui/a.txt` in conflict, theirs made `ui` a file: refused, naming `ui` and `ui/a.txt`); `a_folder_with_no_conflict_below_is_kept_whole` (the check walks only that folder: the listings read are that folder's).
  - `the_start_holding_a_conflict_both_lines_changed_is_unknown` (differently: `[o, absent, t]`, the middle label "can't tell"; alike: taken).
  - `the_count_is_every_kind_6_entry_in_the_result` (made and carried; and no extra field in the save).
  - `labels_name_the_line_and_target_and_their_saves`.
  - `without_the_feature_a_conflict_still_stops_the_combine_and_stores_no_record` (a v0.1 project: refusal; no kind-19 object in any pack, `.opalro/tmp` empty).
  - `every_kept_record_reads_back_and_names_stored_versions` (after `store_packs`: each record decodes, each term and own version is in the store).
  - **Property:** extend `tree_merge_agrees_with_a_path_by_path_model` (`combine/tests.rs:1426`) with a kept-conflict mode (its model, written apart from the engine, also says at each path where it sees a clash which three entries the record must hold and whether it stands as yours or nothing): every path the model calls a clash holds a kind-6 entry whose record has exactly those terms; every other path holds the model's merge; 256 cases; each clash kind asserted at least 3 times.
- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs-core --all-features combine -- --test-threads=2`
Expected: does not compile (`Keep`).

- [ ] **Step 3: Write terminal.md's and json.md's rows for this task's words; implement.**
- [ ] **Step 4: Run the tests and see them pass** (`combine`, `rules` filters), then the `combine`, `merge_vs_git` and `model` test files (unchanged in what they expect: the command line and the model still pass `OnlyIfClean`). Break it on purpose four ways and report which test caught each: the record's terms in `Three`'s order; a carried conflict on both lines changed treated as `NotFiles`; a record written with `keep` off; a text keeping its own reason under rules not read whole (D2).
- [ ] **Step 5: Commit**

```bash
git add docs crates/opal-vcs-core/src/combine.rs crates/opal-vcs-core/src/combine crates/opal-vcs-core/src/rules.rs crates/opal-vcs/src/main.rs crates/opal-vcs/tests/model.rs
git commit -m "The merge keeps each clash about one place as a conflict, marked where both lines' rules allow, and stops where both lines changed a place one of them holds a conflict at or below, or where rules that can't be read whole would decide"
```

### Task 10: Moves keep conflicts, and nothing a line saved is left out

**Files:**
- Modify: `crates/opal-vcs-core/src/combine.rs` (`merge_three`, `combine.rs:449`; `clash_at`, `:615`; `Clash`'s move variants and their words, `:114-116`), `combine/tree.rs` (`record_at` callable from `merge_three`), `combine/tests.rs`
- Modify: `docs/terminal.md` (`opalro combine`'s rows for moves)

**Interfaces:**
- Consumes: Task 9's `record_at` (`tree.rs`, `pub(super)`); here it is called from `merge_three` too, so its body is a free function `pub(super) fn record(store: &Store, packs: &mut PackSet, made: &mut Made, merged: &mut Merged, path: &str, clash: Clash, terms: [Option<Entry>; 3], own: Option<(Entry, u8)>) -> Result<Id, CombineError>` that `Walk::record_at` wraps; `kept::overlay_with` (with `Listings::stored`, Task 6); the `moved` map `merge_three` builds (`combine.rs:470`) and passes to `merge_trees` (`tree.rs:101`), by which a moved file's rule is asked under its old name too (`line_merge(path, was)`, decision 172).
- Produces: the three move clashes name the **place**, not the old path, so a row reads as the record it describes:

```rust
pub enum Clash { /* … */
    /// At one of the two places it went: `from` where it was, and where the other line put it.
    MovedApart { from: String, moved_on: Side, other_to: String },
    /// At the place it went: moved there from `from` on `moved_on`, removed on the other.
    MovedRemoved { moved_on: Side, from: String },
    /// At the place it went, where the other line put something new.
    MovedOnto { moved_on: Side, from: String },
}
```

- **The rules** (the start's file at `from`; `B` where ours put it, `C` where theirs did):

  | Clash | Places | Terms at each place | Stands | The old place `from` |
  |---|---|---|---|---|
  | Moved apart | `B` and `C`, each unless it is below a conflict the tree merge kept (D1) | `[ours@P, start@from, theirs@P]` at each, each side's own entry there | as yours: your file at `B`, nothing at `C` (unless ours holds something there) | left to the tree merge |
  | Moved on one line, removed on the other | `B` (where it went), unless below a kept conflict | `[ours@B, start@from, theirs@B]` | as yours | left to the tree merge |
  | Moved onto something new | `B`, by the tree merge (two added things; the rule asked under the moved file's old name too, through `moved`) | as the tree merge makes them | markers if both are text and both lines' rules allow under both names | **left to the tree merge**, which keeps a conflict there if the other line changed the file in place: nothing a line saved is left out |

- **Nothing is cleared** (second-round answer 6, and D1): `clash_at`'s clearing of every version's entry at `from` (`combine.rs:615-632`) goes; the tree merge sees all three trees as the lines have them, so `B` and `C` are one-sided additions there (or two added things, which it keeps as a conflict). After `merge_trees`, each place of a moved-apart or moved-and-removed clash gets its record with `record` and is put in with `overlay_with` on the merged root, **only after looking at each folder above the place in the merged result: when one of them is a conflict's entry (kind 6), the place gets no record**, since that conflict's folder term already holds everything a line has below it, the moved file that landed there included (D1). So putting records back never replaces a conflict's entry with a plain folder, and no record is ever put below another conflict's place. For moved onto, the conflict the tree merge keeps at `B` is named with `MovedOnto`'s words instead of "added on both lines". The `moved` map keeps being filled for every followed move, as today; a move clash's places are added to it with their start name, so the rule is asked under both names there too.
- Without `keep`, the same places are named and nothing is recorded, so `--only-if-clean` and a project without the feature name the same rows.

- [ ] **Step 1: Write the failing tests** (`combine/tests.rs`; `moved_apart_conflicts` (`:1741`), `moved_and_removed_conflicts` (`:1753`) and `moved_onto_something_new_conflicts` (`:1769`) run with `OnlyIfClean` and assert the new rows)
  - `moved_apart_keeps_two_conflicts`; `moved_and_removed_keeps_the_moved_file_where_it_went` (moved on ours: stands as the file; moved on theirs: stands as nothing); `moved_onto_something_new_keeps_two_added_things_with_markers`; `moved_onto_while_the_other_line_changed_it_in_place_keeps_that_change_at_the_old_place`; `a_move_clash_place_in_a_new_folder_makes_the_folder`; `the_count_includes_move_clash_places`.
  - **D1, the scan's case:** `a_move_clash_below_a_kept_conflict_is_held_by_that_conflict` (start `a.txt`; ours moves it to `ui/a.txt` and adds `ui/b.txt`; theirs moves it to `c.txt` and adds a file `ui`: the result holds a conflict at `ui`, your folder with `a.txt` and `b.txt` in it against their file, and one at `c.txt`; no record below `ui`; every version of every line is in an entry or a record).
  - `a_scene_moved_onto_a_text_is_kept_as_yours_when_its_old_name_says_no_markers` (a `.unity` file moved onto `notes.txt`'s place where the other line added a text, the rule saying `conflict-markers = false` for `*.unity` only: kept, standing as yours, no markers), as `rules.rs`'s `a_moved_file_is_merged_only_when_both_its_names_allow_it` does for merging.
  - **Property:** `nothing_a_line_saved_is_left_out` (128 cases, fixed seed, the generator of `following_moves_agrees_with_a_model` (`:2948`) with honest notes, and drawing on purpose cases where a move clash's place is below a place the tree merge keeps in conflict (a file against a folder, a folder changed against removed); each move clash, and that nested case, asserted at least 3 times): for each side, every entry (kind and ID) that side holds and the start doesn't hold at the same path is, in the result, either an ordinary entry somewhere, a term of a record somewhere, or below a folder that is a term of one; no record is below another conflict's place; and `following_moves_agrees_with_a_model` still passes with its expectations for kept conflicts (its model, apart from the engine, places the records by the table above).
- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs-core --all-features combine -- --test-threads=2`
Expected: `moved_apart_keeps_two_conflicts` fails.

- [ ] **Step 3: Implement**, and edit terminal.md's move rows.
- [ ] **Step 4: Run the tests and see them pass.** Break it on purpose three ways and report which test caught each: clear `from` for moved onto again; put the record at `B` only, for moved apart; put a record back without looking above it (D1).
- [ ] **Step 5: Commit**

```bash
git add docs/terminal.md crates/opal-vcs-core/src/combine.rs crates/opal-vcs-core/src/combine
git commit -m "A combine keeps a conflict wherever a moved file landed in two ways, never below another conflict, and nothing a line saved is left out of the result"
```

### Task 11: The starting version unknown (R7)

**Files:**
- Modify: `crates/opal-vcs-core/src/combine.rs` (`Inner::start_from`, `combine.rs:1040`; `of_starts`, `:989`; `CombineError::StartsConflict`, `:268`, kept only for the rules (D2); `merge_three`'s moves), `combine/tree.rs` (`enter`, `decide`, `contents`, `leave`, `merge_trees`), `combine/tests.rs`
- Modify: `crates/opal-vcs/tests/model.rs` (its `Stopped::StartsConflict`, `model.rs:2033`, `:2043`, and its starting-points merge, `:2234`: the model learns the unknown start, written apart from the engine), `crates/opal-vcs/tests/combine.rs` (`starting_points_that_cant_be_combined_while_opalro_toml_cant_be_read_whole_say_so`, `tests/combine.rs:787`: the new words)
- Modify: `docs/security.md` (T43), `docs/terminal.md` (the starting points' refusal: only the rules' one stays, reworded; the `start_unknown` row), `crates/opal-vcs/src/main.rs` and `combine_output.rs` (the changed variant)

**Interfaces:**
- Consumes: Task 9's `record_at`, `Parted::Unknown`, `LineMerge::RulesNotWhole`.
- Produces:

```rust
// combine/tree.rs
/// Paths where a made-up starting point can't tell what the lines parted from: at and below each.
pub(crate) type Unknown = BTreeSet<String>;
// Made gains
    /// For each made-up starting point, by its root, where it is unknown.
    pub unknown: HashMap<Id, Unknown>,
// Merged gains
    /// For an inner merge: the paths it couldn't settle, which are unknown in what it made.
    pub unknown: Unknown,
// merge_trees (`tree.rs:92`) gains `start_unknown: &Unknown` after `moved` (`:101`), before `deadline`.

// combine.rs: the one stop left for the starting points' own combines (D2)
    /// A file in the starting points' own combine that the rules would decide, while `side`'s
    /// `opalro.toml` can't be read whole.
    StartsConflict { names: Names, rules_not_whole: Side },
```

- **Inner merges** (`inner`): a clash never makes a record and never stops anything: its path goes into `merged.unknown` and nothing goes into the made-up listing there; the names check in `leave` (`unheld_clash`, `tree.rs:499`) is not made (its combine is never written to a drive). **Except (D2):** a place where `line_merge` says `RulesNotWhole` and the rules would decide (both text files within the limits) is not unknown: `merged.rules_not_whole` is set as in Task 9 and `Inner::start_from` returns `StartsConflict { rules_not_whole: side }` with decision 172's hints (`combine.rs:336-353`, kept), changing nothing. `Inner::start_from` records each made-up start's `unknown` in `made.unknown` under its root, and the next inner merge that reads it treats it as below. `of_starts`'s `NamesClash` arm (`combine.rs:991-994`) goes away; `Past` and `TooBig` stay as they are. The `StartsConflict` words for clashing starting points go; its words now say the rules' reason only (Words).
- **Starting points that share no save** (`under` empty): the made-up start is an empty listing unknown at `""`; the inner merge of the two starting points then keeps what they hold alike and is unknown wherever they differ, a place one holds and the other doesn't included.
- **In any merge, at a path the start is unknown at** (the path or one above it is in the set): `one_sides` is not used; ours and theirs alike (kind and ID) → taken; two folders → gone into, everything below unknown; otherwise a conflict of `[ours, absent, theirs]` through `record_at`, `Clash::StartUnknown`, with markers when both are text and both lines' rules allow (`merge_text(b"", …, Some(&sides.labels(Parted::Unknown)))`); a text there under rules not read whole stops the combine (D2). **A start entry that has an unknown path below it** is never "unchanged" for `one_sides` either (at the root too): the walk goes in.
- **Moves:** a note or a pairing by contents whose `from` or `to` is unknown in the start is not followed.
- **No conflict inside a conflict here either:** a conflict this task makes whose ours or theirs term is a folder goes through `record_at`'s check below that folder, as any other.

- [ ] **Step 1: Write the failing tests** (`combine/tests.rs`)
  - `starting_points_that_conflict_are_unknown_where_they_clash` (replaces `starting_points_that_conflict_stop_the_combine`, `:3797`: the lines alike there → taken; differently → `[o, absent, t]` with the "can't tell" label);
  - `starting_points_that_cant_be_combined_while_the_rules_are_not_read_whole_say_so` (`:3843`, kept, its words the new ones: a text both starting points changed, with the folder's `opalro.toml` broken, still stops the combine with 172's hint, and isn't unknown);
  - `unknown_is_not_absent` (a place one line holds and the other doesn't: a conflict, never an addition or a removal);
  - `names_alike_inside_a_made_up_start_stop_nothing` (replaces `names_that_clash_between_starting_points_are_their_conflict`, `:4872`);
  - `starting_points_with_no_save_in_common_are_unknown_where_they_differ` (replaces `starting_points_with_no_save_in_common_are_combined_from_nothing`, `:4834`);
  - `a_made_up_folder_equal_to_one_side_is_not_taken_as_unchanged`;
  - `unknown_inside_two_folders_is_decided_entry_by_entry`;
  - `a_move_into_or_out_of_an_unknown_place_is_not_followed`;
  - `a_criss_cross_keeps_a_revert_that_one_starting_point_would_lose` and the budget tests unchanged.
  - `crates/opal-vcs/tests/combine.rs`: `starting_points_that_cant_be_combined_while_opalro_toml_cant_be_read_whole_say_so` (`:787`) asserts the new words and the same hint.
- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs-core --all-features combine -- --test-threads=2`
Expected: `StartsConflict` for clashing starting points.

- [ ] **Step 3: Implement**, and edit security.md and terminal.md.
- [ ] **Step 4: Run the tests and see them pass**, then the `combine` and `model` test files (`PROPTEST_CASES=64` for the model). Break it on purpose three ways and report which test caught each: unknown read as absent; `one_sides` applied to a start entry with an unknown path below; a rules-not-whole text inside the starting points read as unknown.
- [ ] **Step 5: Commit**

```bash
git add docs crates/opal-vcs-core/src/combine.rs crates/opal-vcs-core/src/combine crates/opal-vcs/src crates/opal-vcs/tests/model.rs crates/opal-vcs/tests/combine.rs
git commit -m "Where the starting points disagree, the start is unknown: what both lines hold alike is kept, anything else is a whole conflict, never a guess; rules that can't be read whole still stop it"
```

### Task 12: `opalro combine` keeps conflicts on the command line

**Files:**
- Modify: `crates/opal-vcs/src/main.rs` (`Combine { only_if_clean }`, `From<CombineError> for Failure`), `combine_output.rs`, `help.rs`; `crates/opal-vcs-core/src/combine.rs` (`CombineError::Conflicts` gains `why: NotKept` beside `rules_not_whole`, which stays, and `conflicts_words`, `combine.rs:393`, which no longer says "which Opalro can't keep yet")
- Modify: `docs/terminal.md` (`opalro combine`), `docs/json.md` (`opalro combine --json`), `docs/guide.md` (§4's `opalro combine`, §8)
- Test: `crates/opal-vcs/tests/combine.rs`, `json_formats.rs`, `terminal.rs`, `undo.rs`; `merge_vs_git.rs` runs `opalro combine --only-if-clean` until Task 16; `model.rs` calls the engine with `Keep::OnlyIfClean` since Task 9 and is unchanged here

**Interfaces:**
- Consumes: `combine(…, keep, …)`; `Combined::Made { conflicts, more, conflict_count }`.
- Produces:

```rust
Combine {
    target: String,
    message: Option<String>,
    /// Stop and change nothing if the combine would leave a conflict.
    #[arg(long)]
    only_if_clean: bool,
},
// combine.rs
/// Why conflicts weren't kept: asked not to, the project can't keep them, or rules that can't
/// be read whole would decide a place (decision 172; then `rules_not_whole` says whose).
pub enum NotKept { OnlyIfClean, NoFeature, RulesNotWhole }
CombineError::Conflicts { names, conflicts, more, rules_not_whole: Option<Side>, why: NotKept }
```

- **What people see** (Words): the outcome line with the count; a row a made conflict, its clash and how it stands; `… and N more`; when the lines held conflicts already, the detail saying how many more the save holds; "Next:" and the undo line. The outcome's count is `conflict_count`, every place the save holds; the rows are the places this combine made (the first 100, then `… and N more`); the detail gives the rest, `conflict_count` less the made ones, as held by the lines already. The refusals: `--only-if-clean`, a project without the feature, and decision 172's stop (`why: RulesNotWhole`, with today's hints by `rules_not_whole`, `combine.rs:332-353`), each with its own first line; none says "which Opalro can't keep yet"; `HeldConflict`. Among `combine`'s warnings, `rules_file_skipped` and `rules_ignored` stay as milestone 8 gives them (`combine.rs:768`, `:811`), for the `opalro.toml` the combine leaves.
- **`combine --json`** gains `conflicts` (list of `{path, clash, why, stands, places}`: `clash` one of `"changed_on_both"`, `"changed_and_removed"`, `"added_on_both"`, `"file_and_folder"`, `"moved_apart"`, `"moved_and_removed"`, `"moved_onto"`, `"start_unknown"`; `why` for `"changed_on_both"`: `"overlap"`, `"endings_only"`, `"binary"`, `"too_big"`, `"too_many_lines"`, `"no_markers"`, `"rules_not_whole"`, `"not_files"`, else `null`; `stands`: `"markers"`, `"yours"`, `"nothing"`; `places`: a number with markers, else `null`), `more_conflicts`, `conflict_count`. A refusal is the §5 error document as today.

- [ ] **Step 1: Write the failing tests** in `tests/combine.rs`: `combine_keeps_conflicts_and_says_where` (exit 0; exact rows; `status` lists them; `assert_folder_is_goto_of`); `a_combine_with_conflicts_is_one_step_undo_takes_back` (`undo`: the line, the folder and `status` as before); `only_if_clean_stops_and_changes_nothing` (exit 1; the milestone 8 test `a_conflict_stops_names_the_files_and_changes_nothing` becomes this, with `--only-if-clean`); `without_the_feature_the_combine_stops_and_names_setup_upgrade` (a v0.1 project; exit 1; nothing stored; `format` unchanged); `combining_a_line_that_holds_a_conflict_both_changed_is_refused`; `combine_json_lists_conflicts` (snapshot); `a_hostile_path_in_a_kept_conflict_is_escaped`; `while_opalro_toml_cant_be_read_whole_a_combine_merges_no_file_line_by_line` (`tests/combine.rs:607`) and `a_combine_that_brings_in_a_broken_opalro_toml_says_so` (`:662`) run with the default (keep) and still see the stop, with the new first line and today's hints, nothing changed; `the_count_is_every_place_and_the_rows_the_made_ones` (a line holding a conflict elsewhere: the count includes it, the rows don't, the detail names how many); `a_combine_with_conflicts_keeps_unsaved_work_in_the_way_as_before`; `a_combine_of_many_large_conflicted_texts_stays_within_memory` (`#[cfg(target_os = "linux")]`, as milestone 8's: twelve 30 MiB texts changed in the same places on both lines, `peak_mb < 300`, all twelve kept with markers). `combine --help` in `cli.rs`.
- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs --all-features --test combine -- --test-threads=2`
Expected: clap doesn't know `--only-if-clean`.

- [ ] **Step 3: Write terminal.md, json.md and the guide; then implement**, passing `Keep::Conflicts` unless `--only-if-clean`.
- [ ] **Step 4: Run the tests and see them pass** (`combine`, `json_formats`, `terminal`, `undo`, `cli`, `guide`, `merge_vs_git`, `model` test files; snapshots reviewed). Break it on purpose twice and report which test caught it: pass `Keep::Conflicts` with `--only-if-clean`; drop `rules_not_whole` from the refusal (the hint falls back to the plain one).
- [ ] **Step 5: Commit**

```bash
git add docs crates/opal-vcs-core/src/combine.rs crates/opal-vcs/src crates/opal-vcs/tests
git commit -m "opalro combine keeps conflicts and says where they are; --only-if-clean stops as before, and a project made by v0.1 is told how to keep them"
```

### Task 13: `resolve` in the engine

**Files:**
- Create: `crates/opal-vcs-core/src/resolve.rs` (and its `tests` module)
- Modify: `crates/opal-vcs-core/src/lib.rs`, `snapshot.rs` (`save_recording` split: `read_and_build`, `compose`; `SaveRequest` unchanged), `conflict.rs` (selecting places), `fsio.rs` (a create-new write for `--copy`), `docs/security.md` (T54), `docs/terminal.md` and `docs/json.md` (this task's refusals and their hints, which the engine makes here; Task 14 adds the outcomes)

**Interfaces:**
- Consumes: `status`'s engine (Task 4: the places the comparison meets, with their records and states), `seen`, `Recording`, `Listings`, `shown_elsewhere`, `switch::{locked, Planned, make_switch}`, `writeout::plan`, `Repository::store_packs`, `Store::copy_content`.
- Produces:

```rust
// snapshot.rs
// `SaveRequest` is unchanged: 22 files build it field by field and it has no `Default`
// (`main.rs`, `kept.rs:564`, `history.rs`, `merge.rs`, `rules.rs`, eleven test modules,
// `model.rs`, `support/mod.rs`, …), so what `resolve` adds goes in parameters.
/// What a walk of the folder built, before it becomes a save.
pub(crate) struct Walked { pub root: Id, pub packs: PackSet, pub met_conflicts: u64, /* the file cache, warnings, move notes, the count of files, as save_folder has them at snapshot.rs:596-643 */ }
/// Walks the folder on top of `parent` as `save` does (`snapshot.rs:492-607`), with every check
/// `save` makes, settling the conflicts it meets with `recording` (Task 7's). `check_named`:
/// whether what `req.only` names is checked to match something (`only::check_matched`, called
/// today without condition at `snapshot.rs:542`); `resolve` passes `false`, since its places come
/// from the save itself and one standing as nothing matches nothing in the folder.
pub(crate) fn read_and_build(repo: &Repository, req: &SaveRequest, parent: &Save,
    recording: &Recording, check_named: bool,
    confirm_large: &mut dyn FnMut(&LargeSave) -> bool) -> Result<Walked, SaveError>;
// Task 7's `save_recording` becomes: lock, finish a stopped switch, current, the amend checks,
// `read_and_build(…, recording, true, …)`, the nothing-changed check, `compose`, commit.
/// A save of `root` on `line`: in place of `parent_id` when `amend`, else on top of it. Its
/// extra fields, change ID and short change ID, worked out as `save` does (`snapshot.rs:646-702`).
pub(crate) fn compose(repo: &Repository, line: &str, parent_id: Id, parent: &Save, root: Id,
    message: &str, who: &Person, amend: bool, move_notes: Vec<MoveNote>,
    packs: &mut PackSet, cache: &mut ChangeIds, warnings: &mut Vec<Warning>) -> Result<(Id, Save, String), SaveError>;

// resolve.rs
pub enum Pick { Mine, Theirs, Before }
pub struct ResolveRequest<'a> {
    /// Places named, a folder standing for every place below it; `None` records every fixed one.
    pub paths: Option<&'a [String]>,
    pub keep: Option<Pick>,          // never `Before`
    pub who: &'a Person,
    pub command: &'a str,
    pub accept_crlf: bool,
    pub git_excludes: Option<&'a Path>,
}
pub enum Ask<'a> { MarkersLeft { path: &'a str, left: u64 }, Large(&'a LargeSave) }
pub enum ResolveOutcome {
    Recorded { save_id: Id, save: Save, short_change_id: String, replaced: Option<Id>,
        recorded: Vec<String>,
        /// The places it met still in conflict: the first 100, and how many in all.
        left: Vec<ConflictAt>, left_count: u64,
        applied: Applied, safety_copies: usize, warnings: Vec<Warning> },
    /// Nothing fixed to record; what is left, with what each needs.
    Nothing { left: Vec<ConflictAt>, left_count: u64 },
    /// The save the project is on holds no conflict.
    NoConflicts { save_id: Id },
}
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    NotOnALine,
    NotInConflict { path: String },
    StillYours { paths: Vec<String> },
    MarkersLeft { path: String, left: u64 },
    CantPickSide { path: String },
    Save(SaveError), Switch(SwitchError), Repo(RepoError),
}
pub fn resolve(repo: &mut Repository, req: &ResolveRequest, on_wait: &mut dyn FnMut(&str),
    ask: &mut dyn FnMut(&Ask) -> bool) -> Result<ResolveOutcome, ResolveError>;
/// Writes `pick`'s version of the file in conflict at `path` to `to`, which must not exist and
/// must not be inside `.opalro`; records nothing.
pub fn copy_version(repo: &Repository, path: &str, pick: Pick, to: &Path) -> Result<Copied, ResolveError>;
pub struct Copied { pub size: u64 }
```

- **The steps**, under `switch::locked(repo, "resolve", …)` (so a stopped switch is finished first):
  1. On a line, else `NotOnALine` (Words; the hint names `opalro line new <name>`, which at a save makes a line there and puts the project on it, `lines.rs:154-175`). **Finding the places** is `status`'s pass (Task 4): the folder walked against the line's save and compared with it, each place met with its record and state, and those the walk carries unread (`NotHere`); no other read of the save (ruling A). None → `NoConflicts`.
  2. **Which places:** with paths, every place at or below each (a path with none: `NotInConflict`); without, every place whose `ConflictAt.seen` is `Fixed`. Each decision below reads `ConflictAt.seen` and `has_own` (Task 4), never `standing` alone. A `NotHere` place can't be recorded from the folder (refused as `save` refuses a name this computer can't hold); `--keep` records the term, the folder isn't written there, and write-out's own "not written here" warning (`writeout::not_written_warning`, `writeout.rs:2060`) says so (second-round answer 6). If any is refused nothing is recorded: without `--keep`, a place `Untouched` whose marker length is 0 and which has no own version (it still stands as your line's version, as the combine left it) → `StillYours` (all such named together); one that is `Untouched` but stands as an own version the person saved is recorded as that version (the ruling on question 7); `MarkersLeft` (untouched marked texts too) → `ask(MarkersLeft)`, a no → `MarkersLeft`; with `--keep`, a record of more than three terms → `CantPickSide`; without `--keep` such a record is recorded as it stands. Nothing to record → `Nothing`.
  3. **Where it records** ([decision 177](../decisions/177-recording-a-fix-save-and-resolve.md) point 3): `amend` when the line's save has more than one parent and `shown_elsewhere` is empty; else a new save, message `Fixed conflicts in <path>` (one place, the path as stored) or `Fixed conflicts in N places`.
  4. **Without `--keep`:** `read_and_build` with `only` the chosen places (each as `Given::Path`), `check_named: false`, and `recording.fixed` those places, so `save`'s checks apply to exactly those files: the stop for changed line endings (`--accept-crlf` answers it, as for `save`), names this computer can't hold (refused as `save` refuses), the question before an unusually large save (`ask(Large)`). Then `compose`, then `repo.commit` with a diary entry whose command is `resolve` and `folder_change: None`.
  5. **With `--keep`:** no second walk. The new root is the line's save with each chosen place replaced by the picked term (`overlay_with` reading as stored, `Listings::stored`; a folder term whole: it holds no conflict below, by ruling C, and a planted one is refused when the folder is next read for the folder), composed, its packs stored (`repo.store_packs`, as a combine stores its save before switching); then `writeout::plan(repo, from, new_save, Some(places), true, drive, …)` and `make_switch` with a state whose line is at the new save and command `resolve`: what the folder held that no save holds is kept as a safety copy first, the term is written under write-out's limits and name rules, one diary step with its folder change.
  6. **What it says is left:** the places step 1 met, less those recorded: exact for what it met, which is every place `status` lists. Nothing is written into the save's extra fields (ruling B); an amend's cloned `extra` is as `save --amend` leaves it.
  Fault points: `resolve:built` (before the commit or the switch), and the switch's own.
- **`--copy`**: finds the place as step 1 does, only along the path given (`writeout::Tree` over `Listings::stored`, reading the listings on that path and the record there, nothing else); the place's record; `Mine`, `Theirs` or `Before` (`CantPickSide` for more than three terms; no file in that version, a folder or a link: refused, Words); `to` checked not inside `.opalro` (compared as the drive compares names) and written with create-new, never over anything (`fsio`'s one write-out of a file outside the project), streamed with `copy_content`, flushed. No lock, nothing recorded.

- [ ] **Step 1: Write the failing tests** (`resolve.rs` tests; folders compared with what `goto` writes after each step that writes, as Task 6's helper does in the engine's own terms)
  - `resolve_records_every_fixed_conflict_into_the_combined_save` (same change ID, parents, message and author; the folder unchanged; the rest left);
  - `resolve_makes_a_new_save_when_the_combined_save_is_shown_elsewhere` (a tag on it; another line; a server line);
  - `resolve_makes_a_new_save_when_the_newest_save_is_not_combined`;
  - `resolve_a_path_records_only_that_place_and_leaves_other_unsaved_changes_unsaved`;
  - `resolve_a_folder_means_every_conflict_below_it`;
  - `resolve_refuses_a_path_not_in_conflict_and_records_nothing` (with a good path beside it);
  - `resolve_refuses_an_untouched_version_conflict_without_keep`;
  - `resolve_without_paths_never_records_a_version_conflict` (changed in place: listed as left);
  - `resolve_asks_about_markers_left_and_records_on_yes`; `a_no_records_nothing`;
  - `resolve_keep_theirs_writes_theirs_whole`; `resolve_keep_theirs_keeps_a_safety_copy_of_a_half_fix`; `resolve_keep_mine_where_mine_is_nothing_removes_the_file`; `resolve_keep_a_folder_term_writes_the_folder_under_write_outs_rules`;
  - `resolve_keep_on_a_record_of_five_terms_is_refused`; `a_five_term_record_is_recorded_as_it_stands`;
  - `undo_takes_a_resolve_back_and_a_hand_fix_stays_in_the_folder`; `undo_takes_back_a_resolve_keep_and_its_folder_change`; `redo_makes_it_again`;
  - `resolve_at_a_save_is_refused`;
  - `resolve_meets_saves_checks` (an imported project with CRLF rewritten: stopped, `accept_crlf` saves it; a large fix asked about; a name this computer can't hold refused);
  - `resolve_says_what_is_left_and_writes_no_extra_field`;
  - `copy_writes_one_version_and_records_nothing`; `copy_never_overwrites_and_never_writes_into_opalro` (an existing file, a dangling link, `.opalro/x`, `.OPALRO/x` on a drive that ignores case); `copy_before_with_nothing_there_is_refused`; `copy_of_a_folder_term_is_refused`;
  - with `fault-injection`: `a_resolve_stopped_at_every_step_is_finished_or_never_happened` (with and without `--keep`: after the next command, either the step is in the diary and the folder is what `goto` writes for it, or neither line nor folder changed, the safety copy kept).
- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs-core --all-features resolve -- --test-threads=2`
Expected: does not compile.

- [ ] **Step 3: Implement**: split `save_folder` first and run the `snapshot` filter and the `amend`, `conflicts` test files before writing `resolve.rs`.
- [ ] **Step 4: Run the tests and see them pass.** Break it on purpose three ways and report which test caught each: record a version conflict without `--keep`; amend when a tag shows the save; skip the safety copy for `--keep`.
- [ ] **Step 5: Commit** (security.md: T54, `resolve` against planted records and against the folder: the copy's destination, the term written)

```bash
git add docs/security.md docs/terminal.md docs/json.md crates/opal-vcs-core/src/resolve.rs crates/opal-vcs-core/src/lib.rs crates/opal-vcs-core/src/snapshot.rs crates/opal-vcs-core/src/conflict.rs crates/opal-vcs-core/src/fsio.rs
git commit -m "resolve records fixes: into the combined save when it may be replaced, else a save of its own, with save's checks and one undoable step"
```

### Task 14: `opalro resolve` on the command line

**Files:**
- Modify: `crates/opal-vcs/src/main.rs` (`Resolve`, its handler, `From<ResolveError> for Failure`), `help.rs` (the overview's row, `examples("resolve")`)
- Create: `crates/opal-vcs/src/resolve_output.rs`, `crates/opal-vcs/tests/resolve.rs`
- Modify: `docs/terminal.md` (a new `opalro resolve` section after `opalro combine`; §10's questions), `docs/json.md` (`opalro resolve --json`, `opalro resolve --copy --json`), `docs/guide.md` (§4's `opalro resolve`, §8: fixing a combine)
- Test: `resolve.rs`, `json_formats.rs`, `cli.rs`, `terminal.rs`, `undo.rs`

**Interfaces:**
- Consumes: `resolve::{resolve, copy_version, …}`.
- Produces:

```rust
/// Record fixed conflicts: all the folder shows fixed, or these paths.
Resolve {
    /// Files and folders in conflict; a folder means every conflict below it.
    paths: Vec<String>,
    /// Take this side's version, whole, at each path.
    #[arg(long, value_enum, requires = "paths", conflicts_with = "copy")]
    keep: Option<KeepArg>,          // mine | theirs
    /// Write one version of one file somewhere else, to look at; records nothing.
    #[arg(long, value_enum, requires = "to")]
    copy: Option<CopyArg>,          // mine | theirs | before
    /// Where --copy writes; it must not exist.
    #[arg(long, value_name = "FILE")]
    to: Option<PathBuf>,
    /// Record files that still hold marked places, or a large fix, without asking.
    #[arg(long)]
    yes: bool,
    /// Record files Git rewrote with CRLF as they are.
    #[arg(long)]
    accept_crlf: bool,
},
```

- `--copy` takes exactly one path (else exit 2 with Words); `--to` is read relative to the current folder, as other paths are. Paths are read as `save`'s are (`project_given`).
- **Questions** (§10's table gains two rows): markers left, `Record it as fixed anyway? [y/N]`, `--yes`, without a terminal: stops, recording nothing; a large fix: `save`'s question and words.
- **`resolve --json`**: `result` (`"recorded"`, `"nothing"`, `"no_conflicts"`), `save` (save object, or `null`), `replaced` (64 hex digits or `null`), `recorded` (list of paths), `conflicts`, `more_conflicts`, `conflict_count` (what is left, as `status --json` has them), `written`, `removed`, `safety_copies`, `warnings`. **`resolve --copy --json`**: `result` `"copied"`, `path`, `version` (`"mine"`, `"theirs"`, `"before"`), `to`, `size`.
- The overview's row: `resolve` — `Record fixed conflicts`. `examples("resolve")`: `opalro resolve` "Record every conflict you fixed"; `opalro resolve Main.unity --keep theirs` "Take the other line's version of a file"; `opalro resolve Hero.png --copy theirs --to ~/Desktop/Hero-theirs.png` "Look at the other line's version first".

- [ ] **Step 1: Write the failing tests** in `tests/resolve.rs`: the outcome of each `ResolveOutcome` and each refusal, exact text; `resolve_asks_on_a_terminal_and_stops_without_one`; `resolve_keep_requires_paths` (exit 2); `copy_takes_one_path_and_a_to` (exit 2 otherwise); `resolve_json_matches_its_snapshot` (each result); `undo_says_what_a_resolve_took_back` (`Undid resolve: main is back at <id>.`); `a_hostile_path_is_escaped_in_resolves_rows` and in the default message's outcome; `the_usual_path_is_short` (combine, fix in the "editor" by writing the file, `resolve`: one combined save, `history` shows one row, `status` clean, `assert_folder_is_goto_of`). Add the documents to `json_formats.rs`, `resolve` to `cli.rs`'s `COMMANDS` (its length grows by one) and to the commands `terminal.rs` runs without a terminal and over hostile text.
- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs --all-features --test resolve -- --test-threads=2`
Expected: clap doesn't know `resolve`.

- [ ] **Step 3: Write terminal.md, json.md and the guide; then implement.**
- [ ] **Step 4: Run the tests and see them pass** (`resolve`, `json_formats`, `cli`, `terminal`, `undo`, `guide` test files; snapshots reviewed). Break it on purpose twice and report which test caught it: let `--keep` run without paths; ask the markers question with `--json`.
- [ ] **Step 5: Commit** (testing.md: the new file)

```bash
git add docs crates/opal-vcs/src crates/opal-vcs/tests
git commit -m "opalro resolve on the command line: record fixes, keep a side whole, or copy one version out to look at it"
```

### Task 15: `show`, `changes --save`, and Git's words

**Files:**
- Modify: `crates/opal-vcs-core/src/diff.rs` (`ChangeKind`'s three new kinds), `changes.rs` (the save's changes: the new kinds, with the feature; no lines for them; their counts), `crates/opal-vcs/src/show_output.rs`, `output.rs` (`change_json`'s new values), `git_words.rs`
- Modify: `docs/terminal.md` (`opalro show`, `opalro changes`, Git's words), `docs/json.md` (§2's change, `opalro show --json`), `docs/guide.md` (§9, Git's words; `merge` ends with 0)
- Test: `crates/opal-vcs/tests/conflicts.rs`, `git_words.rs`, `json_formats.rs`, `terminal.rs`

**Interfaces:**
- Consumes: the comparison of a save with its first parent that `show` and `changes --save` make today (`changes.rs:262-284`, as stored).
- Produces:
  - `ChangeKind::{InConflict, StillInConflict, ConflictFixed}`: between two saves, a path whose new entry is kind 6 and old isn't, both kind 6 with different records, old kind 6 and new not. Only in a project with the feature (the mapping is made in `changes.rs`, where the repository is known); without it, today's kinds. No lines are compared for them ([decision 176](../decisions/176-a-conflict-in-the-folder.md) point 5). The comparison meets these entries anyway: no extra read (ruling A).
  - **`show`** counts them from the same comparison and says so under its changes (Words: "3 places became conflicts in this save", and the same for "stayed in conflict, changed" and "were fixed"); `show --json` gains `conflicts_made`, `conflicts_changed`, `conflicts_fixed` (numbers). `history` gets no mark and a save's JSON no count (ruling B; the mark goes on the follow-ups, Task 17). `change` gains `"in_conflict"`, `"still_in_conflict"`, `"conflict_fixed"`.
  - **Git's words** (Words): `merge` still runs `combine`; `merge --abort` reworded; explained, each in its own words: `merge --continue`, `checkout --ours <file>`, `checkout --theirs <file>` (and `restore --ours`, `--theirs`), `mergetool`; `add`'s words gain the line for after a combine.

- [ ] **Step 1: Write the failing tests**: `show_counts_the_places_a_save_made_kept_and_fixed`; `show_and_changes_save_name_a_conflict_made_still_there_and_fixed` (no hunks for them); `history_is_unchanged_by_conflicts` (a conflicted save's row as any other's); `show_json_counts_them`; in `git_words.rs`: `merge_abort_names_undo`, `merge_continue_names_resolve`, `checkout_ours_and_theirs_name_resolve_keep`, `mergetool_is_explained`, `add_names_resolve_after_a_combine`, `no_git_word_puts_the_typed_path_in_a_command`; `without_the_feature_show_names_a_planted_conflict_as_v01_does`.
- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -j 2 -p opal-vcs --all-features --test conflicts -- --test-threads=2`
Expected: no `in conflict` row.

- [ ] **Step 3: Write the docs; then implement.**
- [ ] **Step 4: Run the tests and see them pass** (`conflicts`, `git_words`, `json_formats`, `terminal`, `combine`, `tags`, `guide` test files; snapshots of `show` reviewed). Break it on purpose twice and report which test caught it: map a kind-6 entry to `KindChanged` as before; count a still-in-conflict place as made.
- [ ] **Step 5: Commit**

```bash
git add docs crates/opal-vcs-core/src/diff.rs crates/opal-vcs-core/src/changes.rs crates/opal-vcs/src crates/opal-vcs/tests
git commit -m "show and changes --save tell the conflicts a save made, kept and fixed, and Git's merge words point at resolve and undo"
```

### Task 16: The model test and the comparison with `git merge`

**Files:**
- Modify: `crates/opal-vcs/tests/model.rs`, `crates/opal-vcs/tests/merge_vs_git.rs`, `crates/opal-vcs/tests/support/vs_git.rs` (if a helper is shared), `docs/testing.md`

**Interfaces:**
- Consumes: the engine's `combine` (`Keep::Conflicts`), `resolve`, `save`, `goto`, `undo`, `redo`, `tidy`.
- Produces:
  - **`model.rs`:** `Act::Combine` keeps conflicts (`--only-if-clean` one time in four); new acts `Resolve { pick: usize, how: ResolveHow }` (`All`, `Paths`, `KeepMine`, `KeepTheirs`), `FixFile { pick }` (write a marked file without its markers, or with one marked place left), `ChangeInPlace { pick }`. The model holds, for each save, its places in conflict with their three versions and what stands in the folder, all worked out by its own rules (written apart from the engine: [decision 175](../decisions/175-what-a-combine-keeps-as-a-conflict.md)'s table, [decision 176](../decisions/176-a-conflict-in-the-folder.md)'s "what it stands as", [decision 177](../decisions/177-recording-a-fix-save-and-resolve.md)'s table), and expects after every act: the folder as it computes it; `status`'s list of places and their states; what `combine`, `save` and `resolve` say they met. It checks after every `tidy` the rule **`no_version_a_kept_conflict_names_is_freed`**: every term and own version of every record a kept save holds reads back. `undo` and `redo` over each act. The generator's text versions keep milestone 8's unique lines.
  - **`merge_vs_git.rs`:** `combine_matches_git_merge` grows: the four outcomes are now (both clean, both conflicted, Opalro clean only, Git clean only); "Opalro clean only" must be 0; where both conflict, the paths Git reports unmerged (`git diff --name-only --diff-filter=U`) equal Opalro's places, whenever the case holds no move both tools follow differently (the case is skipped, counted and printed otherwise); then in each pair of copies, `opalro resolve . --keep mine` against `git checkout --ours -- . && git add -A` gives the same files, contents and executable marks, and `--keep theirs` against `--theirs`. At least 16 "both conflicted" cases compared, else the test fails; the counts are printed.

- [ ] **Step 1: Write the tests.**

Run: `PROPTEST_CASES=64 cargo test -j 2 -p opal-vcs --all-features --test model -- --test-threads=2`
Then: `cargo test -j 2 -p opal-vcs --all-features --test merge_vs_git -- --test-threads=2`
Expected: pass. A failure is a difference between the engine and the model, or Git: find which is wrong before changing either.

- [ ] **Step 2: Run the model longer once**

Run: `PROPTEST_CASES=512 cargo test -j 2 -p opal-vcs --all-features --release --test model -- --test-threads=2`

- [ ] **Step 3: Update testing.md and commit**

```bash
git add crates/opal-vcs/tests/model.rs crates/opal-vcs/tests/merge_vs_git.rs crates/opal-vcs/tests/support docs/testing.md
git commit -m "The model test keeps and fixes conflicts with its own rules, and where Git reports a conflict Opalro keeps one at the same places"
```

### Task 17: The documents

**Files:**
- Modify: `docs/design.md` (§5.5 Conflicts and §5.10's "Built so far": the marked text's form, what stands in the folder, the feature; the phase line; open and deferred lists), `docs/milestones.md` (milestone 9's steps ticked), `docs/testing.md` (the new test files, if any task missed one), `docs/security.md` (the rows T52 to T54 checked against the code as built; T43, T46, T47 updated), `docs/guide.md` (§13, what Opalro doesn't do yet), `README.md` if it lists commands, `CLAUDE.md` (the phase line), `docs/benchmarks.md` (the controller supplies the numbers: `status` and `goto` on a 100,000-file project with the feature and no conflict, against without the feature, which must be the same, and with a few conflicts deep in the tree), the follow-ups file
- The deferred list gains what R8 leaves out: conflicts of more than three terms written by Opalro; combining two conflicts into a wider one; `who`; writing a conflicted save into a Git repository; a general command that takes one file out of any save; writing a conflict out afresh from its terms; merge programs. The follow-ups gain: a mark in `history` for a save that holds conflicts ("needs a save to say it holds conflicts, or a cache"; ruling B); and the follow-ups this plan's questions turn into.

- [ ] **Step 1: Edit the documents; check each with `tr -cd '\r' < file | wc -c`**
- [ ] **Step 2: Run the `guide` test file**
- [ ] **Step 3: Commit**

```bash
git add docs README.md CLAUDE.md
git commit -m "Milestone 9's documents: what is built, how it is tested, and what an attacker could try"
```

### Task 18: Red team (the controller's)

- [ ] Write `redteam/attack-surface-m9.md`: planted records (every refusal of §8, terms that lie about kind and size, a folder term as large as a save, nested records, a record of five terms, a record over 16 KiB, a conflict inside a conflict), marked texts at the limits (marker runs of 255 and 256, 64 MiB, CRLF and no last break, lines that look like markers), what the feature costs a wide project with no conflict and one with a few deep ones (ruling A's promise), `save` and `resolve` against files changed during them, `--copy` destinations (links, `.opalro` spellings, existing files), `resolve --keep` against files in use and links in the folder, undo and crash across `resolve`, `setup --upgrade` stopped halfway, a v0.1 project with planted kind-6 entries.
- [ ] One session from the no-network sandbox (decision 024), attack code in the next free `redteam/attacks*` folder, report in `docs/redteam/`. Windows and macOS through a `redteam/` CI branch.
- [ ] Every proven finding fixed, with its proof in the next free `crates/opal-vcs/tests/redteam*.rs`, or ruled on in a decision record. A second session on the fixes when a finding changed engine code.

### Task 19: Review, rulings and the merge into `v0.2` (the controller's)

- [ ] A whole-branch review by a fresh reviewer, given the Review Focus, and asked to look for any place, besides those the plan names, that stores a listing read as it stands, or stores one under the ID it was read by (second-round answer 7); its findings fixed or ruled on.
- [ ] `docs/plans/…-milestone-9-rulings.md`: the please-confirm list first (resolve replaces the combined save; a combine with conflicts ends with 0; the upgrade step; pictures and scenes fixed only by `resolve`; starting points that clash give whole conflicts; `--copy`; and this plan's questions as answered), then every ruling as made.
- [ ] The nightly workflow run once on the branch (model test at 1,024 cases, the fuzz targets with `decode_conflict`, Git's own repository through Task 16's comparison); benchmarks.md gets the numbers.
- [ ] CI green on Windows, macOS and Linux; `milestone-9` merged into `v0.2`; milestones.md and CLAUDE.md say milestone 9 is done.

---

## Words to review

Every message, row, question, hint, line of help and `--json` name this plan introduces, in one place. `<path>`, `<x>` are escaped paths in prose; inside a command a person would paste (a hint, a "Next:" line, a Git word), `<path>`, `<file>`, `<name>`, `<line>` and `<somewhere>` are the literal placeholders, never filled with text from the project (Global Constraints); `<line>` is escaped; `<target>` is `Named::shown`; `<id>` a save's short change ID; `<a>`, `<b>` name the two lines; `N` a count written with `text::thousands`. Plurals: `1 place`/`2 places`, `1 file`/`2 files`, `1 marked place`/`2 marked places`.

| Kind | Words | Task |
|---|---|---|
| Decode refusal (in `verify`'s "It doesn't read as a conflict record: …"; made by Task 1's decoder, documented in Task 3 with `verify`'s finding) | `unknown conflict-record version`; `a conflict names an odd number of versions, from 3 to 255`; `a version in a conflict is of an unknown kind`; `a folder or project link in a conflict has a size`; `a conflict's marker length is from 1 to 6`; `a conflict has markers on a version that isn't a file` (the words Task 1's decoder has; they replace this table's first draft); existing words for a bad bool, a short record and bytes left over | 3 |
| Upgrade, before asking | `This version of Opalro can turn on 1 feature in this project:` then `  conflicts: a combine can keep conflicts, and you fix them when you choose.` then `Once it is on, Opalro 0.1 can't open this project, and opalro undo can't turn it off.` | 2 |
| Question | `Turn it on? [y/N]` (`Turn them on? [y/N]`) | 2 |
| Outcome | `Turned on conflicts: this project can keep conflicts now.` | 2 |
| Outcome | `Nothing to turn on: this project has every feature this version of Opalro knows.` | 2 |
| Error | `Opalro asks before turning on a feature, and can't ask here, so nothing was changed.` | 2 |
| Hint | `Run it again with --yes to turn it on.` | 2 |
| `opalro setup` row | `Upgrade available: conflicts (opalro setup --upgrade)` | 2 |
| Help | `--upgrade`: `Turn on the features this version of Opalro knows that this project doesn't use yet.` | 2 |
| JSON | `opalro setup --json`: `upgrades`; `opalro setup --upgrade --json`: `result` (`"upgraded"`, `"nothing"`), `features` | 2 |
| `verify` finding | `Conflict record <12 hex> stands as a folder that holds another conflict, which Opalro never writes.`; JSON kind `nested_conflict` with `record` | 3 |
| Error, a record that can't be read | `The conflict at <path> can't be read (record <12 hex>): <detail>.` (`The conflict <name> in folder listing <12 hex> …` where no path is known) with the store's damage hint | 4 |
| Error, a conflict inside a conflict | `<path> is a conflict inside a conflict, which Opalro never writes, so this project may be damaged.` with the store's damage hint (`opalro verify`) | 4 |
| Error, a record too large | `The conflict at <path> names a record over 16 KiB, which Opalro never writes, so this project may be damaged.` with the store's damage hint | 4 |
| `status` heading | `In conflict (N places):` | 5 |
| `status` row | `<path>  2 marked places left to fix` (`1 marked place`; `marker lines left to fix` when only closing ones are) | 5 |
| `status` row | `<path>  fixed, not yet recorded` | 5 |
| `status` row | `<path>  your version, still to decide` | 5 |
| `status` row | `<path>  changed in place, still to decide` | 5 |
| `status` row | `<path>  not in your folder, still to decide` | 5 |
| `status` row | `<path>  can't be written on this computer, kept as it is` | 5 |
| Next (one line: the first that applies; else the step `status` gives without conflicts) | `Next: record what you fixed with: opalro resolve` (any place fixed and not yet recorded); else `Next: fix the marked places in your editor, then run: opalro resolve` (any place with marked places left); else `Next: pick a version with: opalro resolve <path> --keep mine (or theirs); to look at theirs first: opalro resolve <path> --copy theirs --to <file>` (any place standing as a version) | 5 |
| `changes` line | `In conflict: <path> (compared with what your folder held for it after the last save)` | 5 |
| JSON | `status --json`: `conflicts`, every place, uncapped as `changes` is (`path`, `standing`: `"markers"`, `"fixed"`, `"yours"`, `"changed"`, `"nothing"`, `"not_here"`; `places_left`; `terms`); `changes --json`: `in_conflict` | 5 |
| `goto` detail | `The save you went to holds places in conflict: opalro status lists them.` (no number, ruling B; second-round ruling 4) | 6 |
| JSON | `goto --json`: `holds_conflicts` (boolean) | 6 |
| `save` detail | a save of everything: `N places are still in conflict: opalro status lists them.` (`1 place is still in conflict: opalro status lists it.`); a save of some paths: `N of the places you saved are still in conflict: opalro status lists every place in conflict.` (`1 of the places you saved is still in conflict: …`); nothing when 0 | 7 |
| JSON | `save --json`: `conflicts` (the places it met still in conflict) | 7 |
| Marker label, yours and theirs | `<name> (<8 letters>)` | 8 |
| Marker label, the middle | `where the lines parted (<8 letters>)`; `where the lines parted (several saves)`; `where the lines parted (<8 letters>), which held nothing here`; `where the lines parted, which held nothing here`; `where the lines parted, which Opalro can't tell here` | 8 |
| Error | `<path> is in conflict on <line>, and <target> changed it too, so nothing was changed.` (`on <target>, and <line> changed it too`; `on both <line> and <target>`) | 9 |
| Error (a conflict below) | `<x>, inside <path>, is in conflict on <line>, and <target> changed <path> too, so nothing was changed.` (the lines the other way round where it happened so) | 9 |
| Hint | `Fix it on the line that holds it first (opalro resolve there), then run opalro combine again.` | 9 |
| Conflict row | `<path>: changed on both lines, and Opalro can't tell what it held where they parted` | 9 |
| Conflict row (moves, reworded) | `<path>: moved here from <x> on <a>, and to <y> on <b>` | 10 |
| Conflict row | `<path>: moved here from <x> on <a>, removed on <b>` | 10 |
| Conflict row | `<path>: moved here from <x> on <a>, where <b> has something else` | 10 |
| Outcome | `Combined <target> into <line>: save <id> "<message>", with N places in conflict:` | 12 |
| Row | `  <path>: <what happened>; <how it stands>` with `<how it stands>`: `marked in the file (2 places)`, `your version is in the folder`, `nothing is in your folder` | 12 |
| Detail | `The two lines held N more places in conflict already; the save keeps them.` | 12 |
| Next | `Next: fix the marked files and pick a version for the rest, then run opalro resolve. opalro status lists what is left.` | 12 |
| Undo line | `Changed your mind? opalro undo` (existing) | 12 |
| Error (`--only-if-clean`) | `Combining <target> into <line> would leave N places in conflict, so nothing was changed (--only-if-clean):` and the rows | 12 |
| Hint | `Run it again without --only-if-clean to keep the conflicts and fix them one at a time.` | 12 |
| Error (no feature) | `Combining <target> into <line> would leave N places in conflict, which this project can't keep yet, so nothing was changed:` and the rows | 12 |
| Hint | `opalro setup --upgrade lets this project keep conflicts (Opalro 0.1 can't open it afterwards). Or make the two lines agree on these files, save, and run opalro combine again.` | 12 |
| Error (rules not read whole, decision 172, where conflicts are kept) | `Combining <target> into <line> would have to merge N places that opalro.toml decides, and it can't be read whole, so nothing was changed:` (`and the opalro.toml on <target> can't be read whole` when it is the other side's) and the rows of those places only | 9 |
| Hint | decision 172's two hints, unchanged (`combine.rs:332-353`) | 9 |
| Error (starting points, rules not read whole) | `Combining <target> into <line> needs their common starting points combined first, and opalro.toml would have to decide a file there but can't be read whole, so nothing was changed.` (`the opalro.toml on <target>`), with decision 172's hints | 11 |
| Help | `--only-if-clean`: `Stop and change nothing if the combine would leave a conflict.` | 12 |
| JSON | `combine --json`: `conflicts` (`path`, `clash`, `why`, `stands`, `places`), `more_conflicts`, `conflict_count`; the `clash`, `why` and `stands` values of Task 12 | 12 |
| Removed | `StartsConflict`'s words for clashing starting points (`… and those conflict with each other, which Opalro can't keep yet …`); `conflicts_words`' "which Opalro can't keep yet" | 11, 12 |
| Outcome (into the combined save) | `Recorded N fixes in save <id> "<message>" on <line>.` (`1 fix`) | 14 |
| Detail | `The combined save now holds them; its ID stays the same.` | 14 |
| Outcome (a new save) | `Recorded N fixes in a new save <id> "<message>" on <line>.` | 14 |
| Detail | `The combined save couldn't take them: <who> shows it too.` (`<who>` as `AmendShared` names them); or `Your newest save isn't a combined save, so the fixes are a save of their own.` | 14 |
| Detail (`--keep`) | `Took theirs for N places.` (`Took yours`) | 14 |
| Detail | `A safety copy of what your folder held was kept.` (existing words for safety copies) | 14 |
| Detail | `N places it met are still in conflict:` then `status`'s rows, the first 100, then `  … and N more` | 14 |
| Outcome | `Nothing to record: no place in conflict is fixed in your folder.` then the rows of what is left | 14 |
| Outcome | `Nothing to resolve: save <id> "<message>" holds no conflict.` | 14 |
| Default message | `Fixed conflicts in <path>` (raw, stored); `Fixed conflicts in N places` | 13 |
| Error | `You're at a save, not on a line, so there is no line to record fixes on.` | 13 |
| Hint | `Make a line here first: opalro line new <name>` (`line new` at a save makes the line there and puts the project on it) | 13 |
| Error | `<path> isn't in conflict, so nothing was recorded.` | 13 |
| Hint | `opalro status lists the places in conflict.` | 13 |
| Error | `<path> still holds your version, and Opalro won't guess which one you want, so nothing was recorded.` (several: `These still hold your version, … :` and the paths) | 13 |
| Hint | `Say which to keep: opalro resolve <path> --keep mine, or --keep theirs. Or put the version you want in the folder and run opalro resolve <path>.` | 13 |
| Question | `<path> still has 2 marked places.` (`still has marker lines`, when only closing ones are left) then `Record it as fixed anyway? [y/N]` | 14 |
| Error | `<path> still has 2 marked places, so nothing was recorded.` (`still has marker lines, so …`) | 13 |
| Hint | `Fix them first, or run it again with --yes to record the file as it is.` | 13 |
| Error | `<path> is in conflict between more than two lines, so Opalro can't pick one side of it.` | 13 |
| Hint (`resolve`) | `Put the version you want in the folder and run opalro resolve <path>.` | 13 |
| Hint (`--copy`) | `It names more than two versions; opalro status lists it.` | 13 |
| `--copy` outcome | `Wrote the other line's version of <path> to <file>.` (`your version`; `the version where the lines parted`) | 14 |
| `--copy` errors | `<file> is already there, so nothing was written.`; `Opalro doesn't write into .opalro, so nothing was written.`; `The other line holds no file at <path>, so there is nothing to copy.` (`Your line holds no file at <path>, …`; `There was no file at <path> where the lines parted, …`); `<path> is a folder on the other line; --copy writes one file.` (`on your line`; `where the lines parted`); `<path> is a link on the other line; --copy writes one file.` | 13 |
| Usage | `--copy takes one path.` (exit 2) | 14 |
| Help | `Record fixed conflicts: all the folder shows fixed, or these paths.`; `paths`: `Files and folders in conflict; a folder means every conflict below it.`; `--keep`: `Take this side's version, whole, at each path.`; `--copy`: `Write one version of one file somewhere else, to look at; records nothing.`; `--to`: `Where --copy writes; it must not exist.`; `--yes`: `Record files that still hold marked places, or a large fix, without asking.`; `--accept-crlf`: `Record files Git rewrote with CRLF as they are.` | 14 |
| Help overview | `resolve` — `Record fixed conflicts` | 14 |
| Examples | `opalro resolve` "Record every conflict you fixed"; `opalro resolve Main.unity --keep theirs` "Take the other line's version of a file"; `opalro resolve Hero.png --copy theirs --to ~/Desktop/Hero-theirs.png` "Look at the other line's version first" | 14 |
| Undo | `Undid resolve: main is back at <id>.` (existing `step_line`, pinned) | 14 |
| JSON | `resolve --json`: `result` (`"recorded"`, `"nothing"`, `"no_conflicts"`), `save`, `replaced`, `recorded`, `conflicts`, `more_conflicts`, `conflict_count`, `written`, `removed`, `safety_copies`, `warnings`; `--copy`: `result` `"copied"`, `path`, `version`, `to`, `size` | 14 |
| `show` / `changes --save` kinds | `in conflict`, `still in conflict`, `conflict fixed` | 15 |
| `show` detail, under the changes | `N places became conflicts in this save.` (`1 place became a conflict`); `N places stayed in conflict, with another version.`; `N conflicts were fixed in this save.` (each only when not 0) | 15 |
| JSON | `show --json`: `conflicts_made`, `conflicts_changed`, `conflicts_fixed`; `change`: `"in_conflict"`, `"still_in_conflict"`, `"conflict_fixed"` | 15 |
| Removed (ruling B) | `history`'s `[N in conflict]` mark, `show`'s `Conflicts` row, every save object's `conflicts`, `goto`'s count, `verify`'s note finding | 3, 6, 15 |
| Git word `merge --abort` | `` `opalro undo` takes back the last combine, conflicts and all. `` | 15 |
| Git word `merge --continue` | `` `opalro combine` is done as soon as it runs.\n`opalro resolve` records the conflicts you fixed. `` | 15 |
| Git word `checkout --ours <file>` (and `restore --ours`) | `` `opalro resolve <file> --keep mine` takes your version of a file in conflict. `` | 15 |
| Git word `checkout --theirs <file>` (and `restore --theirs`) | `` `opalro resolve <file> --keep theirs` takes the other line's version of a file in conflict. `` | 15 |
| Git word `mergetool` | `` Opalro starts no other program: fix the file in your editor, or copy a version out to look at with `opalro resolve <file> --copy theirs --to <somewhere>`. `` | 15 |
| Git word `add` (a line added) | `` After a combine, `opalro resolve <file>` records a file you fixed. `` | 15 |
