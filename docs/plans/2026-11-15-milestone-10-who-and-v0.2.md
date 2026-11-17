# Milestone 10 ("`who`, and v0.2"): Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task by task. Steps use checkbox (`- [ ]`) syntax for tracking. Every subagent runs on Opus.

**Goal:** `opalro who <file>` names the save behind each line of a file; `opalro save --amend` (and `opalro resolve`, which shares the check) refuses or decides by whether anything else holds the save through any parent, asking only of what moved since the save was made; `goto` and `status` are timed part by part and lose the work they do twice; and v0.2.0 is packaged up to, but not including, the draft release.

**Architecture:** One new engine module, `who.rs`, walks back from the version asked about: each save's version of the file is read *as it stands* (through `Listings::standing` and `writeout::Tree`, so a place in conflict needs no rule of its own), compared with each parent's by `changes::line_ops` on lines without their endings, and the lines a parent shares travel on to it, newest save first on a heap. `ancestry.rs` gains one question, `holder`. A new cache, `cache/last-made` (`lastmade.rs`), notes the save this computer made last and the diary entry that recorded it; `snapshot::shown_elsewhere` is rewritten on both, and `otherlines.rs` goes. The command line gains `who` with its output module and Git's `blame` and `annotate`. The speed work adds no module. The package changes the version, the notes and the documents that name the version.

**Tech Stack:** Rust 1.99.1 and the crates already in the workspace. No new dependency.

**Spec:** four decisions: [decision 184](../decisions/184-opalro-who-the-save-behind-each-line.md) (`who`), [decision 185](../decisions/185-save-amend-and-a-save-something-else-holds.md) (the amend check), [decision 186](../decisions/186-speed-before-v0.2.md) (speed before v0.2), [decision 187](../decisions/187-v0.2s-package.md) (the package). They refine decisions 017, 021, 074, 112, 116, 128, 145, 157, 160, 168, 169, 177 and 179.

**Rulings:** [2026-11-15-milestone-10-rulings.md](2026-11-15-milestone-10-rulings.md) holds the rulings made before the build, and is kept as the build goes. Every ruling's premise is checked in the code before it is made.

**Branches:** tasks are built on `milestone-10`, made from `v0.2` once milestone 9 is merged into it. It is merged into `v0.2` after its red-team session (Task 11) and whole-branch review (Task 12). `main` is never touched; no tag, no draft release, no pull request.

## Global Constraints

- Everything in CLAUDE.md applies.
  - `#![forbid(unsafe_code)]` in the engine and the command-line crate. **No task adds `unsafe`.**
  - No `unwrap`, `expect` or `panic` outside tests.
  - **One place per rule:**
    - every question about ancestry only in `ancestry.rs` (decision 160); nothing else walks parents to decide who holds a save;
    - "every other line, tag and server line with its save" only in `lines.rs` (`lines::others`, today's private `holders` made `pub(crate)`), used by `line delete` and by the amend check;
    - what counts as text, its limits, the line comparison and reading one file of the folder only in `changes.rs`; a listing as it stands only in `conflict/standing.rs`; one path of a save only through `writeout::Tree`; whether a move note is true only in `moves.rs` (today's private `true_of`, made `pub(crate)`); finding the stored spelling of a typed path, a name at a time as the drive takes names, in one function that `resolve` and `who` both call;
    - Git's words only in `git_words.rs`; colour, symbols and widths only in `style.rs`; dates only in `output.rs`.
- **Spec before code:** `docs/storage-format.md` §10 for `cache/last-made` and for `cache/other-lines` no longer written (Task 2, before any code reads or writes the note); `docs/terminal.md` and `docs/json.md` for every word and field in the task that prints them.
- **Format 1 is frozen.** Nothing here changes how any record is written. Caches are not part of the format (§10, "not normative"): one is added, one is no longer written or read.
- **`who` writes nothing**: no pack, no diary entry, no cache, no file in the folder. It takes no lock that a writer waits for longer than a read does today.
- **Everything `who` reads is untrusted:** a save's move notes are believed only when true of the save and its first parent (`moves::true_of`) and when both paths pass `validate_path`; a version past the limits or not text ends the trail; a history is as long as it is, but each comparison stops at `changes::deadline()` and no step recurses; text from the project (a line, a name, a message, a path) is escaped at the edge exactly as today (`opal_core::text::escape`).
- **The note is a hint, never a truth:** it is used only when its seal is the user's, its save is the one being amended, and the diary entry it names can be read and has this line at this save. Anything else means: ask of every other line, tag and server line. No failure to read or write it is an error.
- **Speed work changes no write, no flush and no check** (decision 186): a task that finds it would need to stops and reports.
- **Untrusted text in hints:** no hint puts a stored name, a path or any other text from the project inside a command a person would paste.
- **No message promises a milestone or a version.**
- **`--json` never asks and never carries colour, symbols, progress or "Next:" lines.** `opalro_json` stays `1`: documents and fields are added, none is changed.
- **Exit codes:** 0 done, 1 failed or refused, 2 a command line Opalro couldn't use.
- **Tests that can't lie:**
  - a property test's model shares neither the engine's code nor its rule's wording;
  - random tests have fixed seeds and assert that each kind of case they name occurred at least 3 times;
  - a comparison with Git fails when it compared nothing;
  - no assertion on a clock unless the gap is at least tenfold; what must be fast is asserted by counting what is read (`counted::…`), not by timing;
  - each rule is broken on purpose once, in the ways the task names; the implementer runs the tests after each break, reports which test caught it, and puts the code back.
- **Test names that read as IDs:** no test resolves a line or tag name made only of the letters `k` to `z`, or of 8 or more hex digits. Use `side`, `wall-run`, `v1.0`.
- **One writer at a time:** tasks run strictly one after another. `git add` names paths; never `git add -A` (the untracked `.codex/` and `AGENTS.md` are not ours).
- Code comments are short and human: the why in a line or two, no process words, no task, ruling or decision numbers in code.
- **Values:**

  | What | Value |
  |---|---|
  | A line, for comparing | its bytes without a final `\n`, then without a final `\r` |
  | A version `who` reads | at most 64 MiB (`changes::MAX_LINES_SIZE`), at most 1,000,000 lines (`changes::MAX_LINES`), no zero byte in its first 8,000 (`changes::is_binary`) |
  | One comparison | at most `changes::deadline()` (2 s); past it coarser, and `who` says so |
  | Holders named in a refusal | the first 5 (`snapshot::NAMED`), then `N more` |
  | `cache/last-made` | `OPALMADE`, version 1, a save's ID, a diary entry's ID, the seal: 105 bytes |
  | The version | `0.2.0` |

- **Local runs on this machine:** `export PATH="$HOME/.cargo/bin:/c/Program Files/GitHub CLI:$PATH"` and `export OPALRO_CONFIG_DIR="$(mktemp -d)"` before any test. The machine is short of memory: one cargo command at a time, `-j 1`, `-- --test-threads=2`, one test file per command. A run that memory stops is not started again: say so.
- **A task is done only when its tests pass on Windows, macOS and Linux:** the controller pushes and reads CI on the task's commit before the task is called complete. Tests that make links or are `cfg(unix)` prove nothing on this Windows machine.
- When output changes, run `--test cli` and `--test json_formats` too.

## Review Focus

The inputs the decisions imply and a person is most likely to meet, each pinned by a test in the task that owns the code:

1. **A file whose line endings one save changed** (`\r\n` to `\n` by an editor or by `.gitattributes` after an import): no line may go to that save (Task 5).
2. **A path typed in another letter case than the save has it,** on Windows and macOS: `who Readme.md` for `README.md` must find the saved file, not call every line unsaved (Tasks 5 and 6).
3. **A file both lines changed in different places, then combined cleanly:** lines go to saves on both lines, and the combine itself gets none (Task 5).
4. **An amend right after an `undo` or a `redo`, and after working on another line:** the note is for another save or other lines moved; the answer must still be exact (Task 3).
5. **`who` in a folder that holds a conflict's marked text, half fixed:** side lines to their saves, marker lines to the combine, the person's new lines unsaved (Task 5).

---

### Task 1: `ancestry::holder`, which of these holds this save

**Files:**
- Modify: `crates/opal-vcs-core/src/ancestry.rs`
- Modify: `docs/testing.md` (the ancestry tests' line)

**Interfaces:**
- Consumes: the private `Walk` of `ancestry.rs` and its marks, as `only_in` uses them.
- Produces:
  ```rust
  /// The first found of `others` that is `save` or has it in its history through any parent,
  /// by its index; `None` when none does. Exact whatever the saves' clocks say.
  pub fn holder(repo: &Repository, save: Id, others: &[Id]) -> Result<Option<usize>, RepoError>
  ```
  It ends as soon as one holder is known. When there is none it reads what `only_in(repo, save, others)` reads.

- [ ] **Step 1: Write the failing tests** (in `ancestry.rs`'s tests, with its helpers for planted histories)

  Named tests, each asserting the exact `Ok(Some(i))` or `Ok(None)`:
  - `a_save_is_held_by_one_that_names_it`: `others` contains `save` itself.
  - `a_save_is_held_along_first_parents`, `a_save_is_held_through_a_second_parent`, `a_save_is_held_through_a_second_parent_of_a_second_parent`.
  - `a_save_beside_another_is_not_held`: two lines from one start; neither tip holds the other.
  - `an_older_save_does_not_hold_a_newer_one`: `others` are ancestors of `save`.
  - `a_holder_is_found_whatever_the_clocks_say`: the holder is dated before `save`, and a parent after its child.
  - `each_holder_is_found_once_it_is_the_only_one_left`: two holders; `holder` gives one; asked again without it, the other; asked again without both, `None`.
  - `no_others_hold_nothing`: `others` empty.
  - `a_save_that_cant_be_read_stops_the_question`: the error `only_in` gives.
  - Property: on the module's random histories (fixed seed), for every pair of a save and a set of others, `holder` is `Some(i)` exactly when a search through every parent from `others[i]` meets `save`, and `None` exactly when none does. Count and require at least 3 of each: held by naming, held along first parents, held only through another parent, not held.

- [ ] **Step 2: Run them; they fail to compile** (`holder` not defined).
- [ ] **Step 3: Implement `holder`.** Mark each of `others` and remember, for each save the others' side reaches, the index of one of `others` it was reached from; the answer is that index once `save` is reached. The walk is `only_in`'s: same order, same stop.
- [ ] **Step 4: Run the tests; they pass.** `cargo test -j 1 -p opal-vcs-core --all-features ancestry -- --test-threads=2`.
- [ ] **Step 5: Break on purpose, twice:** hand the index on along first parents only (the second-parent tests must fail); stop the walk at the first save older than `save` (the clocks test must fail). Report which test caught each; put the code back.
- [ ] **Step 6: Commit** — `ancestry answers which of a set of saves holds another, through every parent`.

### Task 2: The note of the save made last: spec, layout, vector

**Files:**
- Modify: `docs/storage-format.md` §10 (the cache's layout and what it is trusted for; `cache/other-lines` marked as written by Opalro 0.1 only) and §13 (the vector's row; the `other_lines.json` row says the vector went with the code)
- Create: `crates/opal-vcs-core/src/lastmade.rs`; `crates/opal-vcs-core/tests/vectors/last_made.json`; `fuzz/fuzz_targets/last_made.rs`
- Modify: `crates/opal-vcs-core/src/lib.rs` (the module), `crates/opal-vcs-core/tests/vectors.rs`, `fuzz/Cargo.toml`, `.github/workflows/nightly.yml` (the fuzz list), `docs/testing.md`

**Interfaces:**
- Consumes: `keyed::seal_in_place`, `keyed::open_sealed`, `fsio::write_cache(repo.tmp_folder(), repo.cache_folder(), FILE, &buf)`, `safe_fs::read_capped_in`, `Repository::cache_key`, as `otherlines.rs` uses them.
- Produces:
  ```rust
  /// The save this computer made last on a line, and the diary entry that recorded it.
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub struct LastMade { pub save: Id, pub entry: Id }
  impl LastMade {
      pub fn encode(&self) -> Vec<u8>;                                   // the body, without its seal
      pub fn decode(body: &[u8]) -> Result<LastMade, DecodeError>;
      pub(crate) fn load(repo: &Repository) -> Option<LastMade>;         // None for no key, no file, a wrong seal, a body that doesn't decode
      pub(crate) fn store(repo: &Repository, save: Id, entry: Id);       // a failure is ignored
  }
  ```
  Layout, exactly: `bytes[8] "OPALMADE"`, `u8 version = 1`, `id` the save, `id` the diary entry, then the 32-byte seal. A body of another length, magic or version is refused. File: `cache/last-made`.

- [ ] **Step 1: Write the spec.** §10 gains `cache/last-made` (not normative), in the form of the entry for `cache/other-lines`: the layout above; what it is for (decision 185 point 3); that it is used only for the save it names, with the diary entry it names; that a wrong one under a good seal can make the check before an amend miss a line, which `opalro undo` takes back, and can change no stored save; written like `cache/files`. The entry for `cache/other-lines` keeps its layout and says: written by Opalro 0.1; Opalro 0.2 neither reads nor writes it, and a file left there is ignored.
- [ ] **Step 2: Write the failing vector test** `last_made_matches_its_vector` in `vectors.rs` (the form of `other_lines_match_their_vector`): body built from the layout with save `[0x11; 32]`, entry `[0x22; 32]`, key `[0x42; 32]`; decode then encode round-trips; `seal(&key, &body) == sealed`; `open_sealed` with `[0; 32]` gives `None`. Unit tests in `lastmade.rs`: every refusal (a short body, a long body, another magic, version 2), and `load` giving `None` for each of: no key, no file, a seal by another key, a torn file.
- [ ] **Step 3: Run; fails** (no module).
- [ ] **Step 4: Implement `lastmade.rs`; generate the vector** with an `#[ignore]`d printer as the other caches have; add the fuzz target (decode never panics; what decodes encodes to the same bytes) and its line in the nightly's list.
- [ ] **Step 5: Run the tests; they pass.** `cargo test -j 1 -p opal-vcs-core --all-features --test vectors -- --test-threads=2` and the module's unit tests.
- [ ] **Step 6: Commit** — `The note of the save made last has its layout in the storage format, a codec, a vector and a fuzz target`.

### Task 3: The check before an amend asks the exact question, of what moved

**Files:**
- Modify: `crates/opal-vcs-core/src/snapshot.rs` (`shown_elsewhere`, its call in the save path, `Made::commit`), `crates/opal-vcs-core/src/resolve.rs` (its call; the note after a `--keep` that makes a save), `crates/opal-vcs-core/src/combine.rs` (the note after a combine that makes a save), `crates/opal-vcs-core/src/lines.rs` (`holders` becomes `pub(crate) fn others`)
- Delete: `crates/opal-vcs-core/src/otherlines.rs`, `crates/opal-vcs-core/tests/vectors/other_lines.json`, `fuzz/fuzz_targets/other_lines.rs` (and their lines in `lib.rs`, `vectors.rs`, `fuzz/Cargo.toml`, `nightly.yml`); the cache's unit tests in `history.rs`
- Modify: `crates/opal-vcs/tests/model.rs` (the model's `shown_elsewhere`), `crates/opal-vcs/tests/redteam13.rs` (the proof of decision 112, rewritten for the new promise), `crates/opal-vcs/tests/` (the amend and resolve tests named below)
- Modify: `docs/terminal.md` (when an amend is refused; when `resolve` replaces the combined save), `docs/testing.md`, `docs/security.md` (T29)

**Interfaces:**
- Consumes: `ancestry::holder` (Task 1); `LastMade` (Task 2); `lines::others(state, line, &tags, &servers) -> Vec<(Holder<'_>, Id)>` (every other line, every tag, every server line, in that order); `Repository::current() -> Result<(Id, DiaryEntry), RepoError>`, `Repository::entry(&Id)`, `Repository::tag_list(&State)`, `Repository::server_lines(&State)`; `Repository::commit(..) -> Result<Id, RepoError>` (the new entry's ID, which `Made::commit` drops today).
- Produces:
  ```rust
  /// Who else holds `id`, the newest save of `line`: each line, tag or server line that names it
  /// or has it in its history through any parent, as `AmendShared` names them.
  pub(crate) fn shown_elsewhere(repo: &Repository, state: &State, line: &str, id: Id) -> Result<Vec<String>, SaveError>
  ```
  The strings and their order are today's (`line x`, `server line y@origin`, `tag v1`, escaped as today; lines by name, then server lines, then tags). `SaveError::AmendShared`, `shown_too`, `who_shows` and the hint keep their words.

**Rules:**
1. **Asked of:** with a usable note, every holder of `lines::others` that the note's entry does not have at the same save (new since, or moved); without one, all of them. A note is usable when `LastMade::load` gives one, its `save` is `id`, `repo.entry(&note.entry)` reads, and that entry's state has `line` at `id`. Tags and server lines are compared through the two states' lists (equal list IDs mean nothing moved).
2. **Found by** asking `ancestry::holder(repo, id, &saves)` again and again, each time without the holder just found, until it answers `None`.
3. **The note is left** by every step that makes a save and puts a line on it: `Made::commit` (with the ID `repo.commit` returns), a `resolve --keep` that makes a save, a combine that makes a save. For the two that go through `make_switch`, the entry is `repo.current()?.0` after the step. A combine that only moves the line forward leaves none.
4. `ChangeIds` is no longer handed to the check; the save path and `resolve` keep loading it for `Made::commit`.

- [ ] **Step 1: Write the failing tests.** Engine (`snapshot.rs` and `resolve.rs` tests) and command line (`crates/opal-vcs/tests/`), each a named test:
  - refused: `an_amend_is_refused_when_another_line_combined_this_one_in` (the save is the other line's second parent); `…when_a_tag_sits_past_it_on_a_deleted_line`; `…when_a_server_line_reaches_it`; the three v0.1 cases (another line, a tag, a server line that names it) still refused, with the same words;
  - allowed: `an_amend_goes_through_beside_an_older_copy_of_the_same_change` (go to the replaced save by its ID, start a line there, go back, amend: allowed; both saves stay visible);
  - counted, with the store's read counters, never a clock: `an_amend_right_after_a_save_reads_no_save_of_another_line` (300 other lines of 20 saves each are enough: the count is 0); `after_a_save_on_one_other_line_only_that_line_is_asked_about` (the count stays under that line's own saves plus the saves back to where it parted, and no save of a third line is read); `without_a_note_every_other_line_is_asked_about_once` (cache folder emptied);
  - the note distrusted: a note for another save; a note whose diary entry is gone; a note sealed by another key; a note under the right seal whose entry has the line at another save: each ends as "without a note", and an amend that must be refused is still refused;
  - after `undo` of an amend, and after `redo`: the answers are exact (refused when another line was started on the save in between);
  - `resolve`: `a_fix_replaces_the_combined_save_when_nothing_else_holds_it` (as today) and `a_fix_is_a_save_of_its_own_when_another_line_combined_the_combined_save_in`;
  - `crates/opal-vcs/tests/redteam13.rs`: the proof of decision 112 becomes `an_amend_of_a_save_just_made_reads_no_other_lines_saves`, on that test's own shape.
  - An `#[ignore]`d timing test beside it, `amend_in_a_project_of_300_old_lines_timed`, that prints (asserts nothing on a clock): an amend right after a save; an amend with no note; in a release build, for `docs/benchmarks.md`.
- [ ] **Step 2: Run; they fail** (the second-parent case is allowed today; the counters are not zero).
- [ ] **Step 3: Implement rules 1 to 4; delete the walk along first parents and its cache.** Update the model's `shown_elsewhere` in `tests/model.rs` to the same rule in the model's own words: a line, tag or server line holds a save when a search through every parent from it meets that save.
- [ ] **Step 4: Run** the engine's tests for `snapshot`, `resolve`, `lines`, `history`; the command line's amend, resolve, `redteam13`, `model` (its fixed pass's counts will move: report the block; every floor must still hold) and `cli`, `json_formats`.
- [ ] **Step 5: Break on purpose, three times:** ask along first parents only; skip the check that the note's entry has the line at the save; ask of every holder though the note is usable. Report which test caught each; put the code back.
- [ ] **Step 6: Documents:** `terminal.md` (an amend is refused when anything else names the save or has it in its history; the same for when `resolve` makes a save of its own), `security.md` T29 (closed by the rule; the note as one more cache with a seal), `testing.md`.
- [ ] **Step 7: Commit** — `An amend is refused when anything else holds the save through any parent, and asks only of what moved since the save was made`.

### Task 4: One file of the folder, and the stored spelling of a typed path

**Files:**
- Modify: `crates/opal-vcs-core/src/changes.rs` (reading one file of the folder), `crates/opal-vcs-core/src/resolve.rs` and `crates/opal-vcs-core/src/writeout.rs` (the spelling), `crates/opal-vcs-core/src/moves.rs` (`true_of` made `pub(crate)`)
- Modify: `docs/testing.md`

**Interfaces:**
- Consumes: `writeout::Folders` (`open`, `enter`, `here`), `changes::on_disk`'s read (today private and tied to a `PathChange`); `resolve::conflict_on`'s way of matching a name as the drive takes it.
- Produces:
  ```rust
  // changes.rs: the folder's file at `path`, never through a link: its bytes, `TooBig` past
  // MAX_LINES_SIZE, or `None` when no regular file is there.
  pub(crate) fn in_folder(disk: &mut Folders, path: &str) -> Result<Option<Side>, SaveError>
  // writeout.rs, on Tree: the entry at a path as it was typed, each name matched exactly or,
  // failing that, as `drive` takes names when exactly one entry matches; with the path as stored.
  pub(crate) fn entry_as_typed(&mut self, typed: &str, drive: Drive) -> Result<Option<(String, Entry)>, StoreError>
  ```
  `on_disk` calls `in_folder` for a file; `resolve::conflict_on` calls the same name matching (its own answer unchanged).

- [ ] **Step 1: Write the failing tests:** `in_folder` for a file, a missing file, a folder, a link (never followed: `None`, tested on CI where links can be made), a file past the limit; `entry_as_typed` for an exact path, a path in another letter case on a drive that ignores case (found, with the stored spelling), the same on an exact drive (`None`), two entries alike but for case (the exact one wins; with no exact one, `None`), a path through a file (`None`). The tests that pin `changes` and `resolve` today keep passing.
- [ ] **Step 2: Run; fail. Step 3: Implement. Step 4: Run** the engine's `changes`, `resolve`, `writeout`, `moves` tests and the command line's `changes` and `resolve` files.
- [ ] **Step 5: Commit** — `One file of the folder can be read by itself, and a typed path is matched to the stored one in one place`.

### Task 5: `who`, the engine

**Files:**
- Create: `crates/opal-vcs-core/src/who.rs`
- Modify: `crates/opal-vcs-core/src/lib.rs`, `docs/testing.md`

**Interfaces:**
- Consumes: `Listings::standing(repo, &unstored)`, `writeout::Tree::new(&listings, root, kept::EXACT)`, `Tree::entry`, `Tree::entry_as_typed` (Task 4); `changes::{stored, in_folder, Side, is_binary, line_count, lines_of, line_ops, deadline, MAX_LINES}`; `Store::save(&id, signatures_allowed) -> Save` (`root`, `parents`, `author`, `saver`, `message`, `change_id`, `move_notes`); `moves::true_of(&mut before, &mut after, &note)`; `names::validate_path`.
- Produces:
  ```rust
  pub enum Asked { Folder, Save(Id) }
  pub struct WhoRequest<'a> { pub path: &'a str, pub asked: Asked }   // a project path, as typed
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub enum By {
      /// The save the line is from, and the path the file had there when it was another.
      Save { id: Id, was: Option<String> },
      /// In the folder only.
      Unsaved,
  }
  pub struct Run { pub first: usize, pub lines: usize, pub by: By }   // `first` counts from 1
  pub struct Whose {
      pub path: String,          // as the save spells it; as typed when no save has it
      pub at: Id,                // the save asked about, or the one the project is at
      pub text: Vec<u8>,         // the version asked about
      pub runs: Vec<Run>,        // every line once, in order
      pub coarse: bool,          // some comparison ran past its deadline
  }
  pub enum WhoError { /* NoFile, NotAFile (with the kind), NotText, TooBig, TooManyLines, each with the path; Repo; Save */ }
  pub fn who(repo: &Repository, req: &WhoRequest<'_>) -> Result<Whose, WhoError>
  ```

**Rules** (decision 184):
1. **The version asked about.** `Asked::Folder`: the folder's file at the path (`in_folder`; none there is `NoFile`); it is first compared with what the save the project is at holds there, as it stands (found by `entry_as_typed` with the project's drive): lines both have travel on from that save, the rest are `Unsaved`; where that save has no file there, every line is `Unsaved`. `Asked::Save`: what that save holds at the path as it stands; nothing there is `NoFile`, a folder, link or project link `NotAFile`.
2. **Limits,** on the version asked about (an error) and on every older version (the trail ends: no line travels to it): `TooBig`, not text, more than `MAX_LINES` lines.
3. **A line, for comparing,** is its bytes without a final `\n`, then without a final `\r`.
4. **From a save's version to its parents', the first parent first.** The path at the first parent is the note's `from` where the save has a note `from → path` that is true (`moves::true_of`) and whose paths pass `validate_path`; otherwise, and at every other parent, the same path. A parent with nothing there, or not a file, takes nothing. A parent whose content ID is the version's takes every line still travelling, unread. Otherwise the two are compared (`line_ops`, parent as old): each line the comparison calls equal travels to that parent, at its line there. Lines no parent took are that save's, with `was` set when the path there is not the path asked about.
5. **Order:** versions wait on a heap, the newest save first; lines that reach one version by two ways are taken together when it is their turn. The answer never depends on the order. Nothing recurses.
6. **Memory:** the lines' places (one pair per line of the version asked about) and the texts of the two versions being compared; older texts are not kept.
7. `coarse` is set when any comparison ran past `changes::deadline()`.

- [ ] **Step 1: Write the failing tests** (unit tests in `who.rs`, histories built with the engine's test helpers; each asserts the whole `runs`):
  - `a_line_is_the_save_that_added_it`; `a_line_taken_out_and_put_back_is_the_save_that_put_it_back`; `a_file_without_a_last_line_ending`;
  - `a_save_that_only_changed_line_endings_takes_no_line` (Review Focus 1), both directions;
  - `through_a_combine_each_line_goes_to_the_line_it_came_from` (Review Focus 3: both lines changed different places; the combine gets none; a line both have goes to the first parent's side);
  - `what_a_combine_wrote_is_the_combines` (a fix recorded into the combined save);
  - `a_fix_saved_after_a_kept_conflict_goes_through_it_to_both_lines`;
  - `in_the_folder_a_marked_text_names_each_side_and_the_combine` (Review Focus 5: side lines, marker lines, one line typed since: `Unsaved`);
  - `a_file_moved_by_a_save_of_its_own_is_followed`, with `was`; `a_move_saved_with_an_edit_is_not_followed`; `a_move_note_that_is_not_true_is_not_followed` (planted);
  - `a_file_moved_on_the_other_line_keeps_the_combine_for_what_was_changed_under_the_old_name` (the known limit, pinned as it is);
  - `the_folders_new_lines_are_unsaved`; `a_file_no_save_has_is_all_unsaved`; `a_path_typed_in_another_case_finds_the_saved_file` (Review Focus 2, on a drive that ignores case);
  - refusals: nothing there, a folder, a link, not text, too big, too many lines; `an_older_version_that_is_not_text_ends_the_trail`;
  - `a_comparison_past_its_deadline_is_said` (with `OPALRO_DIFF_DEADLINE_MS`, under `fault-injection`);
  - `a_save_with_many_parents` (64 parents, each adding a line: every line to its save);
  - `a_save_that_cant_be_read_is_named`.
  - Property (fixed seed): random histories of saves, combines and moves on small files of lines drawn from a small alphabet (so lines repeat) and, in a second pass, of unique lines. For every tip and file: every line has exactly one `By`; a `By::Save`'s version holds that line; with unique lines, no parent of that save holds it at the path it had there. Count and require at least 3 of each: a line through a combine's second parent, a followed move, a line that went to the first parent though the second had it too.
- [ ] **Step 2: Run; they fail** (no module). **Step 3: Implement.** **Step 4: Run** `cargo test -j 1 -p opal-vcs-core --all-features who -- --test-threads=2`.
- [ ] **Step 5: Break on purpose, four times:** follow first parents only; believe a note without checking it; compare lines with their endings; give a line both parents have to the last parent. Report which test caught each; put the code back.
- [ ] **Step 6: Commit** — `who names the save behind each line of a file, through every parent, recorded moves and kept conflicts`.

### Task 6: `opalro who` on the command line

**Files:**
- Create: `crates/opal-vcs/src/who_output.rs`, `crates/opal-vcs/tests/who.rs` and its snapshots
- Modify: `crates/opal-vcs/src/main.rs` (`Command::Who`, `who_command`), `help.rs`, `git_words.rs`, `output.rs` if a date helper must be shared; `crates/opal-vcs/tests/cli.rs` (`COMMANDS`), `git_words.rs`, `json_formats.rs`, `terminal.rs`
- Modify: `docs/terminal.md` (a section `opalro who`), `docs/json.md` (a section `opalro who --json`), `docs/guide.md` (where history is taught; the table of Git's words; what Opalro doesn't do yet), `docs/design.md` (the command table), `docs/testing.md`

**Interfaces:**
- Consumes: `opal_core::who::{who, WhoRequest, Asked, Whose, Run, By, WhoError}`; `names::project_path(repo.root(), &here, given, repo.drive()?)` as `changes_command` takes one path; the resolver `show` uses for `<save>`; `output::save_json`; the short ID `show` prints (`short_or_whole`); `style` for dim text, cutting and the terminal's width; `opal_core::text::escape`.
- Produces: `opalro who <file> [--save <save>] [--json]`; `opalro blame` and `opalro annotate` as shortcuts for it.

**Words** (each goes into `docs/terminal.md` word for word, before the code):

| State | Outcome line | Then |
|---|---|---|
| The folder's file | `Each line of <path> and the save it is from:` | an empty line, the rows; then, if some lines are unsaved, an empty line and `N lines aren't saved yet.` (`1 line isn't saved yet.`) |
| `--save` | `Each line of <path> as save <id> has it, and the save it is from:` | an empty line, the rows |
| No save has the file | `<path> isn't saved yet: none of its N lines is from a save.` (`its 1 line isn't from a save`) | nothing more |
| An empty file | `<path> is empty.` | nothing more |
| A comparison ran out of time | (after the rows) `warning: Comparing some versions took too long, so a line may be named for a newer save than the one that first had it.` | |
| Nothing there | `error: <path> isn't a file in your folder.` | `hint: To read it as a save has it: opalro who <file> --save <save>` |
| `--save`, nothing there | `error: Save <id> has no file at <path>.` | |
| A folder, a link, a project link | `error: <path> is a folder, and opalro who reads one file.` (`is a link`, `is a project link`) | |
| Not text | `error: <path> isn't text, so it has no lines to name a save for.` | |
| Too big | `error: <path> is larger than 64 MiB, the most Opalro compares line by line.` (`has more than 1,000,000 lines, …`) | |

A row: the save's short ID, the author's name, the day (`YYYY-MM-DD`, in the save's own time zone), the line's number, `│`, the line. Columns are as wide as their widest value; on a terminal a name longer than 20 characters is cut at its end, and off a terminal nothing is cut. Rows of one save that follow each other repeat its three columns, dim after the first on a terminal. An unsaved line has `not saved yet` across the three columns. The line is shown without its line ending, escaped as file contents are in `opalro changes`; a tab stays a tab.

`--json`, one document: `path`, `from` (`"folder"` or `"save"`), `save` (the save asked about or the one the project is at, as `output::save_json` writes one), `lines`, `runs` (each `first`, `lines`, `save`: the save's full ID or `null` for unsaved lines, `was`: the path the file had there or `null`), `saves` (each save named in `runs`, once, as `save_json` writes one), `coarse`, `warnings`.

Git's words: `blame` and `annotate` run `who` with what follows them, as `log` runs `history`; their hints: `in Opalro, blame is who: opalro who <file>` (and the same for `annotate`). `SHORTCUTS` gains both. `LATER` and the tests that pin `blame` as explained go.

- [ ] **Step 1: Documents first:** the `terminal.md` section (the table above, the row, the colours) and the `json.md` section with its table of fields.
- [ ] **Step 2: Write the failing tests** in `tests/who.rs`: a snapshot of the text and of `--json` for a file of three saves with an unsaved line; `--save`; each error with its exit code (1) and hint; a path given from a folder below the root; a path outside the project (exit 1, `project_path`'s words); a pattern is taken as a path; a name and a line holding control characters and escape sequences come out escaped (`tests/terminal.rs`); off a terminal nothing is cut or dimmed; `opalro blame <file>` gives the same rows and its hint once; `opalro who` with no path is a usage error (exit 2). `COMMANDS` gains `who`; `json_formats` finds the new section.
- [ ] **Step 3: Run; fail. Step 4: Implement.**
- [ ] **Step 5: Run** `--test who`, `--test cli`, `--test json_formats`, `--test git_words`, `--test terminal`, `--test guide`.
- [ ] **Step 6: The guide:** a short part where `history` and `show` are taught (one example, its output word for word); the table of Git's words (`git blame` → `opalro who`); "what Opalro doesn't do yet" loses the line about who changed each line and gains what `who` doesn't follow (decision 184 point 3) in one sentence.
- [ ] **Step 7: Commit** — `opalro who shows the save behind each line, and Git's blame and annotate run it`.

### Task 7: `who` against `git blame`

**Files:**
- Create: `crates/opal-vcs/tests/who_vs_git.rs`
- Modify: the nightly's differential test on Git's own repository (the test file the Nightly workflow's differential job runs), `docs/testing.md`

**Interfaces:**
- Consumes: the helpers `merge_vs_git.rs` uses to build one history in Git and import it; `opalro who --json`; `git blame --line-porcelain`.

- [ ] **Step 1: Write the test** `who_matches_git_blame_on_unique_lines` (fixed seed, 64 histories): files whose lines are unique in the whole history; saves that add, change and take out lines; two branches joined by merges that conflict nowhere; files moved by a commit of their own. The history is made in Git and imported; for every file at every tip, each line's save (found by its message, unique per commit) is the commit `git blame` names. If an imported history carries no move notes, the moved files are made with Opalro's own commands and mirrored in Git, and the test says which it did. It fails when it compared no line, and requires at least 3 histories with each of: a line named through a merge's second parent, a moved file. Where Git follows a move Opalro does not (a move with an edit), the generator does not draw it; say so in the test's doc comment and in `docs/testing.md`.
- [ ] **Step 2: Run it:** `cargo test -j 1 -p opal-vcs --all-features --test who_vs_git -- --test-threads=1`. Any difference is a finding: report the smallest history; do not weaken the comparison.
- [ ] **Step 3: The nightly:** in the differential test on Git's own repository, run `opalro who` and `git blame` on five files fixed by name (the test names them), print each one's two times and the share of lines on which they name the same commit, and assert only that `who` named a save for every line. No assertion on a clock.
- [ ] **Step 4: Commit** — `who is compared with git blame on generated histories, and timed against it on Git's own`.

### Task 8: Timers, then the work done twice

**Files:** decided by the timers; none that writes, flushes or checks (decision 186).

- [ ] **Step 1 (the controller, before any change):** throwaway timers around the parts of `goto` and `status` named in decision 186 point 1, on the gate's data set, in a release build on this laptop; the same on GitHub's Linux machine through a throwaway `bench/` branch. The timers are never committed to `milestone-10`. The table goes to the implementer and into `docs/benchmarks.md`.
  - **First question for the timers:** did `status` get slower in milestone 9? Its last nightly measured 14.1 ms on a Xeon 6975P-C where earlier runs on that processor type gave 12.7 to 13.9 ms (`docs/benchmarks.md` §14): one run, not explained. Build `v0.2` before milestone 9 (c3da2fc) and after it (33000cf) with the same timers and run both in one job on one runner, several times in turns, so that the runner's processor cannot be the difference. If a part of `status` did get slower, name the part and the commit range before anything is changed; a fix that alters what `status` reads or checks is a ruling of its own, not part of this task's "work done twice".
- [ ] **Step 2:** for each part the table shows done twice and worth at least half a millisecond together on either machine, do it once: hand on what was read instead of reading it again. Each change keeps every test of `goto`, `status` and crash safety passing unchanged; none adds a test of a clock. If nothing is worth it, the task ends here with a ruling saying so.
- [ ] **Step 3:** the gate again, before and after, by the controller (`bench/run.sh gate` on the runners, through the Nightly workflow).
- [ ] **Step 4: Commit** (if anything changed) — a sentence saying what is no longer done twice.

### Task 9: The package

**Files:**
- Modify: `Cargo.toml` (the workspace's version), `Cargo.lock`, `crates/opal-vcs/tests/cli.rs` (the version it expects)
- Create: `docs/releases/v0.2.0.md`, `docs/releases/v0.2.0-before-you-publish.md`
- Modify: `README.md`, `docs/guide.md`, `docs/releases/download-page.md`, `docs/releases/welcome.md`, `docs/releases/known-problems.md`, `docs/releases/reporting-a-problem.md`
- **Not touched:** `USER-AGREEMENT.md`; every fixture, vector and test project that holds `0.1.0` as stored bytes; `docs/releases/v0.1.0*`.

- [ ] **Step 1: The version and its notes in one commit** (decision 145): `0.2.0`; `docs/releases/v0.2.0.md` in the form of `v0.1.0.md`, saying what decision 187 point 2 lists, with every number taken from `docs/benchmarks.md` as it is then.
- [ ] **Step 2: The sweep.** Every place the research lists as naming v0.1 in a way v0.2 changes (the README's first lines; the guide's archive names, what the program prints, "v0.1 isn't signed", the heading "What v0.1 doesn't do yet" and its anchor; the download page; the welcome page; the known problems; the example in reporting a problem). The guide's anchor that v0.1's notes link to keeps working (an explicit anchor, or the old heading's ID kept): say how in the report. `.github/release/first-run.sh` checks the guide names `opalro-$version-…` word for word: run its check by reading it against the guide.
- [ ] **Step 3: `v0.2.0-before-you-publish.md`:** what is left for the owner, in the order of decision 187 point 6, with the agreement's sentence first.
- [ ] **Step 4: Run** `--test cli`, `--test guide`, `--test json_formats`, and `cargo build -j 1 --release -p opal-vcs` once (the program says `opalro 0.2.0`).
- [ ] **Step 5: Commit** — `Opalro 0.2.0: the version, its notes, and every document a release carries`.

### Task 10: The documents

**Files:** `docs/design.md` (§5 where history and saves are described: `who`, the amend rule; the phase line; open and deferred lists), `docs/milestones.md` (milestone 10's section and its marks), `docs/testing.md`, `docs/security.md` (rows for `who`: a planted history that keeps it busy, planted move notes, what a line or a name can do to a terminal; T29), `docs/benchmarks.md` (the amend check; `who` against `git blame`; the timers; the gate), the follow-ups file (what `who` does not follow; an option to leave spaces out; line ranges; a table of the saves each line reaches, if the amend's slow case bites; the speed pass's row; the flushes and threads written up for the owner), `CLAUDE.md` (the phase line only).

- [ ] **Step 1: Edit; every statement from a decision, `docs/terminal.md`, `docs/json.md` or the code.** Check each file with `tr -cd '\r' < file | wc -c` (0).
- [ ] **Step 2: Run the `guide` test file.**
- [ ] **Step 3: Commit** — `Milestone 10's documents: who, the amend check, the speed write-up and the package`.

### Task 11: Red team

A fresh agent attacks the milestone from the no-network sandbox (decision 024), session RT23: attack code in `redteam/attacks22/`, proofs in `crates/opal-vcs/tests/redteam23.rs` (and `redteam23_platforms.rs`), the report `docs/redteam/2026-11-15-milestone-10.md`. Where to press: a version lost or changed by `who` (it must write nothing); a line named for a save that never held it; a planted history, note or record that makes `who` hang, crash or use memory without bound; what a line, a name or a path can do to the terminal; an amend allowed while something holds the save (a planted note, two commands at once, `undo` and `redo` between); the note written where it must not be; the package naming the wrong version anywhere. And what milestone 9's two sessions left ([decision 183](../decisions/183-red-team-m9-session-2-rulings.md)): `combine` and `resolve` run at the same time; a conflict below a folder the other line moved; three common starting points in every order of their IDs. Every proven finding is fixed or ruled on in a decision record; a second session if engine code changed.

### Task 12: Whole-branch review, closing, and the package's run

- [ ] **Step 1:** the whole-branch review on the range from `v0.2`, pointed at the ledger's deferred minors; one fix wave; one scoped re-review.
- [ ] **Step 2:** the closing documents (the rulings file, the phase lines, the milestones' marks); the Nightly workflow on the last commit; its numbers into `docs/benchmarks.md`.
- [ ] **Step 3:** merge `milestone-10` into `v0.2`.
- [ ] **Step 4:** start the release workflow by hand on `v0.2`'s last commit; every job it runs must pass; the draft job does not run there. Write the run into `docs/releases/v0.2.0-before-you-publish.md`. Nothing is tagged, drafted, merged into `main` or opened as a pull request.
