//! Random edits, saves, amends, switches, discards, shelves, lines, tags, combines, conflicts and
//! their fixes, tidies, undos and redos, applied both to a project and to a model of what its
//! folder, saves and diary should hold (testing.md §3).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use opal_core::combine::{self, CombineError, Combined, Held, Keep, Named, NotKept, Stands};
use opal_core::conflict::Term;
use opal_core::diary::{self, Action, FolderChange, Position, Tag, TagList};
use opal_core::diff::ChangeKind;
use opal_core::id::{Id, hash_record};
use opal_core::lines::{self, LineError, NameError, Target};
use opal_core::listing::EntryKind;
use opal_core::names::Given;
use opal_core::pack::ObjectKind;
use opal_core::presets::{self, Found, Preset};
use opal_core::repo::Repository;
use opal_core::resolve::{self, ResolveError, ResolveOutcome, ResolveRequest};
use opal_core::save::Person;
use opal_core::setup::{apply_setup, plan_setup};
use opal_core::shelf::{Shelved, shelf, unshelf};
use opal_core::snapshot::{SaveError, SaveOutcome, SaveRequest, save_folder};
use opal_core::status::{Standing, status};
use opal_core::store::Store;
use opal_core::switch::{SwitchError, discard, goto};
use opal_core::tags::{self, Ask, TagError, Tagged};
use opal_core::undo::{
    Effect, StepInfo, StepRequest, UndoError, UndoOutcome, redo, undo, undo_list, undo_to,
};
use proptest::prelude::*;
use proptest::test_runner::{RngAlgorithm, TestRng, TestRunner};
use serde_json::{Value, json};

mod support;
use support::{Env, noise};

// Names differ in more than letter case, so every drive holds them all. No file name is ever a
// folder's.
const FILES: [&str; 6] = ["a", "b", "d/a", "d/c", "d/e/b", "e/a"];
const FOLDERS: [&str; 3] = ["d", "d/e", "e"];
// Few lengths, so files often keep their size when they change.
const LENGTHS: [usize; 3] = [1, 2, 5];
/// A big file is cut into chunks when saved, from 1 MiB.
const BIG: usize = 1 << 20;
/// The shelf numbers `unshelf` asks for.
const LABELS: [&str; 3] = ["1", "2", "3"];
/// The presets `opalro setup` switches between.
const PRESETS: [Preset; 3] = [Preset::Code, Preset::Game, Preset::Custom];
/// How many of the steps undo can take `opalro undo --list` shows.
const LISTED: usize = 20;
/// Names for lines and tags, few so they often collide: alike but for case, or the other kind's.
/// None can read as the start of a change ID or a fingerprint.
const NAMES: [&str; 8] = [
    "main", "side", "Side", "wall-run", "v1.0", "V1.0", "fix/a", "-x",
];

/// The modified time a write leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum When {
    /// The clock's, often within the file cache's 2 s of its last look.
    Now,
    /// A time long past that no earlier write had.
    Past,
    /// The time the file had before: a change the file cache is allowed to miss.
    Same,
}

#[derive(Debug, Clone)]
enum Act {
    /// Writes the file whole, of one of the `LENGTHS`; the byte varies what it holds.
    Write(&'static str, u8, usize, When),
    /// Writes other bytes of the same length over a file that is there.
    Rewrite(&'static str, u8, When),
    /// Writes a big file whole, of 1 to 3 MiB that look random, made from the seed.
    Big(&'static str, u64, usize),
    /// Puts three bytes into the big file `pick` chooses, at the place `at` picks, moving the rest
    /// along; or writes them over its own there.
    Edit {
        pick: usize,
        at: usize,
        bytes: [u8; 3],
        insert: bool,
        when: When,
    },
    /// Turns the executable bit on or off (Unix only; elsewhere it does nothing).
    Flip(&'static str),
    Delete(&'static str),
    Rename(&'static str, &'static str),
    MakeFolder(&'static str),
    DeleteFolder(&'static str),
    Save,
    /// Replaces the line's newest save, with a new message or keeping its own.
    Amend {
        new_message: bool,
    },
    /// Goes to the save or line `pick` chooses.
    Goto {
        pick: usize,
        by_line: bool,
        unsaved: Unsaved,
    },
    /// Discards everything, or what is at one path.
    Discard(Option<&'static str>),
    /// Undoes or redoes up to `n` steps; `shelve` sets unsaved work in the way aside.
    Undo {
        n: usize,
        shelve: bool,
    },
    Redo {
        n: usize,
        shelve: bool,
    },
    /// Goes back to the diary entry `pick` chooses.
    UndoTo {
        pick: usize,
        shelve: bool,
    },
    Shelf,
    /// Brings back the newest shelf, or the one with that number.
    Unshelf(Option<&'static str>),
    /// Status reading every file, not trusting the file cache.
    FullStatus,
    /// Switches the rule files to a preset's, as `opalro setup --preset` does.
    Setup(Preset),
    /// Makes a line at the save the project is on.
    LineNew(&'static str),
    /// Renames a line `pick` chooses, the current one, or `old`, which may be no line's.
    LineRename {
        pick: usize,
        old: &'static str,
        new: &'static str,
    },
    /// Deletes a line `pick` chooses, or `name`; `yes` answers the question, if asked.
    LineDelete {
        pick: usize,
        name: &'static str,
        yes: bool,
    },
    /// `save --line`, starting a new line.
    SaveOnNewLine(&'static str),
    /// Tags the save the project is on; `move_it` answers whether a tag elsewhere moves.
    Tag {
        name: &'static str,
        move_it: bool,
    },
    /// Deletes a tag `pick` chooses, or `name`; `yes` answers the question.
    TagDelete {
        pick: usize,
        name: &'static str,
        yes: bool,
    },
    /// Edits a file as text, one line at a time: a new line put in, one taken out, or one
    /// replaced. Every new line is one no file has held, so no version repeats a line.
    Text {
        path: &'static str,
        at: usize,
        edit: u8,
    },
    /// Moves the file `pick` chooses to `to`, makes a new text where it was if `remake` says so,
    /// and saves. The move is noted unless a new text is made where it was, it lands on a file
    /// that was there, or the file was edited since the last save.
    Moved {
        pick: usize,
        to: &'static str,
        remake: bool,
    },
    /// Saves the first of `changes` to a text the save holds (`path`, if it holds none), with any
    /// unsaved work, goes to an older save `pick` chooses in the history of that one, and saves
    /// the second there, which starts a line: work a combine then finds on both sides, often at
    /// one place. With `back`, the first change's save is then combined into that line.
    Fork {
        pick: usize,
        path: &'static str,
        at: usize,
        edit: u8,
        back: bool,
        changes: (Change, Change),
        /// The combine back asks for a clean line.
        clean: bool,
    },
    /// Combines the line or save `pick` chooses into the line the project is on, keeping the
    /// conflicts it meets unless asked for a clean line.
    Combine {
        pick: usize,
        message: bool,
        clean: bool,
    },
    /// Combines the line `pick` chooses into this one and this one's old save into that, then
    /// that line into this one: two lines that took each other's work, with two starting points.
    /// With `fix`, each line keeps its own side of what the first two combines left in conflict,
    /// so the two disagree about what they parted from.
    CrissCross {
        pick: usize,
        fix: bool,
        /// The last combine asks for a clean line.
        clean: bool,
    },
    /// `resolve`, at a place in conflict `pick` chooses, or at every one.
    Resolve {
        pick: usize,
        how: ResolveHow,
    },
    /// Writes a file shown with markers without them, or with one marked place left.
    FixFile {
        pick: usize,
    },
    /// Changes what stands at a place in conflict shown as a version.
    ChangeInPlace {
        pick: usize,
    },
    /// `tidy`, as if two days later, keeping every step.
    Tidy,
    /// `save <path>` at a place in conflict `pick` chooses, or at the folder above it.
    SaveOnly {
        pick: usize,
    },
    /// Renames the file at a place in conflict `pick` chooses to `to`, and saves.
    RenameInConflict {
        pick: usize,
        to: &'static str,
    },
    /// Goes to a line whose save another line's history holds, where that other line's save
    /// holds a conflict, and combines it in asking for a clean line: a forward refused.
    CleanForward {
        pick: usize,
    },
    /// Undoes one step and redoes it, with a `tidy` between them if `tidy` says so.
    UndoRedo {
        tidy: bool,
    },
    /// Makes places in conflict on purpose (two texts in one folder edited in the same place on
    /// two lines, or with `picture` a text one line made not text), then at once does what `then`
    /// picks with them: each form of `resolve`, a refusal, a rename, `save <path>`, an undo.
    Conflicted {
        pick: usize,
        picture: bool,
        then: u8,
    },
}

/// What one line of a `Fork` does to its file, each kind of clash a combine meets drawn on
/// purpose; the numbers pick among the places that were empty when the fork began.
#[derive(Debug, Clone, Copy)]
enum Change {
    /// A line put in, taken out or replaced where `at` and `edit` say.
    Edit,
    /// A new last line.
    EditEnd,
    Remove,
    /// Moved to an empty place, and with `true` a new text made where it was.
    Move(usize, bool),
    /// Bytes that aren't text.
    Binary,
    /// A new text at an empty place, the file left alone.
    Add(usize),
    /// Removed and saved, then another file moved onto its place.
    Arrive,
}

/// Makes the two changes of a fork's lines from two picks and a choice.
type Pair = fn(usize, usize, bool) -> (Change, Change);

/// How `resolve` is asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolveHow {
    /// Every place the folder shows fixed.
    All,
    /// A place, a folder above some, a path not in conflict, or the whole project, as the folder
    /// holds it.
    Paths,
    /// A pattern that matches one place, which takes it only if the folder shows it fixed.
    Pattern,
    KeepMine,
    KeepTheirs,
}

/// What a goto does with unsaved work first.
#[derive(Debug, Clone, Copy)]
enum Unsaved {
    Save,
    Discard,
    /// Leaves it: it comes along, or stops the goto where it collides.
    Keep,
}

/// The acts drawn; with `everywhere_alike`, only those whose effect is the same on every system:
/// no executable bit turned, which only Unix keeps (a status that reads every file stands in).
fn act(everywhere_alike: bool) -> impl Strategy<Value = Act> {
    let file = || prop::sample::select(&FILES[..]);
    let folder = || prop::sample::select(&FOLDERS[..]);
    let when = || prop_oneof![Just(When::Now), Just(When::Past), Just(When::Same)];
    let any_path = prop::sample::select([&FILES[..], &FOLDERS[..]].concat());
    let unsaved = prop_oneof![
        1 => Just(Unsaved::Save),
        1 => Just(Unsaved::Discard),
        2 => Just(Unsaved::Keep)
    ];
    let steps = || (1..=3usize, any::<bool>());
    let label = prop_oneof![
        2 => Just(None),
        1 => prop::sample::select(&LABELS[..]).prop_map(Some)
    ];
    let name = || prop::sample::select(&NAMES[..]);
    // Pairs for the two lines of a fork, each kind of clash among them: a change on both, a
    // change against a removal, two additions, moves apart, against a removal and onto an
    // addition, a file that arrived where the other line changed the one there, and moves
    // followed into a change.
    let change = || {
        let pairs: [Pair; 29] = [
            |_, _, _| (Change::Edit, Change::Edit),
            |_, _, _| (Change::Edit, Change::EditEnd),
            |_, _, _| (Change::EditEnd, Change::Edit),
            |_, _, _| (Change::EditEnd, Change::EditEnd),
            |_, _, _| (Change::Binary, Change::Edit),
            |_, _, _| (Change::Edit, Change::Binary),
            |_, _, _| (Change::Binary, Change::Binary),
            |_, _, _| (Change::Binary, Change::EditEnd),
            |_, _, _| (Change::EditEnd, Change::Binary),
            |_, _, _| (Change::Edit, Change::Remove),
            |_, _, _| (Change::Remove, Change::Edit),
            |a, b, _| (Change::Add(a), Change::Add(b)),
            |a, _, _| {
                (
                    Change::Move(a, false),
                    Change::Move(a.wrapping_add(1), false),
                )
            },
            |a, _, _| {
                (
                    Change::Move(a.wrapping_add(1), false),
                    Change::Move(a, false),
                )
            },
            |a, _, _| {
                (
                    Change::Move(a, false),
                    Change::Move(a.wrapping_add(1), false),
                )
            },
            |a, _, _| (Change::Move(a, false), Change::Remove),
            |a, _, _| (Change::Move(a, false), Change::Remove),
            |a, _, r| (Change::Move(a, r), Change::Remove),
            |_, b, _| (Change::Remove, Change::Move(b, false)),
            |_, b, _| (Change::Remove, Change::Move(b, false)),
            |_, b, r| (Change::Remove, Change::Move(b, r)),
            |a, _, _| (Change::Move(a, false), Change::Add(a)),
            |a, _, _| (Change::Move(a, false), Change::Add(a)),
            |a, _, _| (Change::Add(a), Change::Move(a, false)),
            |a, _, _| (Change::Add(a), Change::Move(a, false)),
            |_, _, _| (Change::Arrive, Change::Edit),
            |_, _, _| (Change::Edit, Change::Arrive),
            |a, _, r| (Change::Move(a, r), Change::Edit),
            |_, b, r| (Change::Edit, Change::Move(b, r)),
        ];
        (
            prop::sample::select(pairs.to_vec()),
            any::<usize>(),
            any::<usize>(),
            any::<bool>(),
        )
            .prop_map(|(pair, a, b, r)| pair(a, b, r))
    };
    let how = prop_oneof![
        2 => Just(ResolveHow::All),
        3 => Just(ResolveHow::Paths),
        2 => Just(ResolveHow::Pattern),
        1 => Just(ResolveHow::KeepMine),
        1 => Just(ResolveHow::KeepTheirs)
    ];
    prop_oneof![
        5 => (file(), any::<u8>(), prop::sample::select(&LENGTHS[..]), when())
            .prop_map(|(p, b, n, w)| Act::Write(p, b, n, w)),
        3 => (file(), any::<u8>(), when()).prop_map(|(p, b, w)| Act::Rewrite(p, b, w)),
        3 => (file(), any::<u64>(), BIG..=3 * BIG).prop_map(|(p, seed, n)| Act::Big(p, seed, n)),
        4 => (any::<usize>(), any::<usize>(), any::<[u8; 3]>(), any::<bool>(), when())
            .prop_map(|(pick, at, bytes, insert, when)| Act::Edit { pick, at, bytes, insert, when }),
        1 => file().prop_map(move |p| if everywhere_alike {
            Act::FullStatus
        } else {
            Act::Flip(p)
        }),
        2 => file().prop_map(Act::Delete),
        2 => (file(), file()).prop_map(|(a, b)| Act::Rename(a, b)),
        1 => folder().prop_map(Act::MakeFolder),
        1 => folder().prop_map(Act::DeleteFolder),
        3 => Just(Act::Save),
        2 => any::<bool>().prop_map(|new_message| Act::Amend { new_message }),
        4 => (any::<usize>(), any::<bool>(), unsaved)
            .prop_map(|(pick, by_line, unsaved)| Act::Goto { pick, by_line, unsaved }),
        1 => Just(Act::Discard(None)),
        1 => any_path.prop_map(|p| Act::Discard(Some(p))),
        4 => steps().prop_map(|(n, shelve)| Act::Undo { n, shelve }),
        3 => steps().prop_map(|(n, shelve)| Act::Redo { n, shelve }),
        2 => (any::<usize>(), any::<bool>())
            .prop_map(|(pick, shelve)| Act::UndoTo { pick, shelve }),
        2 => Just(Act::Shelf),
        3 => label.prop_map(Act::Unshelf),
        1 => Just(Act::FullStatus),
        1 => prop::sample::select(&PRESETS[..]).prop_map(Act::Setup),
        2 => name().prop_map(Act::LineNew),
        2 => (any::<usize>(), name(), name())
            .prop_map(|(pick, old, new)| Act::LineRename { pick, old, new }),
        2 => (any::<usize>(), name(), any::<bool>())
            .prop_map(|(pick, name, yes)| Act::LineDelete { pick, name, yes }),
        2 => name().prop_map(Act::SaveOnNewLine),
        2 => (name(), any::<bool>()).prop_map(|(name, move_it)| Act::Tag { name, move_it }),
        2 => (any::<usize>(), name(), any::<bool>())
            .prop_map(|(pick, name, yes)| Act::TagDelete { pick, name, yes }),
        4 => (file(), any::<usize>(), any::<u8>())
            .prop_map(|(path, at, edit)| Act::Text { path, at, edit }),
        2 => (any::<usize>(), file(), any::<bool>())
            .prop_map(|(pick, to, remake)| Act::Moved { pick, to, remake }),
        10 => (
            (any::<usize>(), file(), any::<usize>(), any::<u8>()),
            (prop::bool::weighted(0.75), prop::bool::weighted(0.2)),
            change(),
        )
            .prop_map(|((pick, path, at, edit), (back, clean), changes)| Act::Fork {
                pick,
                path,
                at,
                edit,
                back,
                changes,
                clean
            }),
        3 => (any::<usize>(), any::<bool>(), prop::bool::weighted(0.25))
            .prop_map(|(pick, message, clean)| Act::Combine { pick, message, clean }),
        3 => (any::<usize>(), prop::bool::weighted(0.75), prop::bool::weighted(0.25))
            .prop_map(|(pick, fix, clean)| Act::CrissCross { pick, fix, clean }),
        4 => (any::<usize>(), how).prop_map(|(pick, how)| Act::Resolve { pick, how }),
        4 => any::<usize>().prop_map(|pick| Act::FixFile { pick }),
        2 => any::<usize>().prop_map(|pick| Act::ChangeInPlace { pick }),
        1 => Just(Act::Tidy),
        2 => any::<usize>().prop_map(|pick| Act::SaveOnly { pick }),
        2 => (any::<usize>(), file()).prop_map(|(pick, to)| Act::RenameInConflict { pick, to }),
        2 => any::<usize>().prop_map(|pick| Act::CleanForward { pick }),
        3 => any::<bool>().prop_map(|tidy| Act::UndoRedo { tidy }),
        8 => (any::<usize>(), prop::bool::weighted(0.35), any::<u8>())
            .prop_map(|(pick, picture, then)| Act::Conflicted { pick, picture, then }),
    ]
}

/// What a file holds, shared by the trees that hold it, as a big one is costly to copy.
#[derive(Clone, PartialEq, Eq)]
struct File {
    bytes: Rc<[u8]>,
    exec: bool,
}

impl std::fmt::Debug for File {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A big one by its size and ID, so a failure doesn't print megabytes.
        if self.bytes.len() >= BIG {
            let id = opal_core::id::hash_content(&self.bytes).short_hex();
            return write!(
                f,
                "File({} bytes, {id}, exec: {})",
                self.bytes.len(),
                self.exec
            );
        }
        f.debug_struct("File")
            .field("bytes", &self.bytes)
            .field("exec", &self.exec)
            .finish()
    }
}

/// Files by path, and every folder, a file's included; in a save as stored, the places in
/// conflict too, with nothing below them. A folder on a drive holds none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Tree {
    files: BTreeMap<String, File>,
    folders: BTreeSet<String>,
    kept: BTreeMap<String, Kept>,
}

/// One version a conflict holds: a file, or a folder with everything in it, its paths its own.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    File(File),
    Folder(Tree),
}

/// A place in conflict as a save holds it: yours, where the lines parted, and theirs; and, when
/// the folder holds a version of its own there, that version with its marker length (0: none).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Kept {
    terms: [Option<Part>; 3],
    own: Option<(Option<Part>, u8)>,
}

impl Kept {
    /// What the folder holds for it: its own version, else yours.
    fn stands(&self) -> Option<&Part> {
        match &self.own {
            Some((own, _)) => own.as_ref(),
            None => self.terms[0].as_ref(),
        }
    }

    /// Its marker length: 0 unless it is shown with markers.
    fn marker(&self) -> u8 {
        self.own.as_ref().map_or(0, |(_, m)| *m)
    }
}

impl Tree {
    fn put(&mut self, path: &str, file: File) {
        self.add_folders_above(path);
        self.kept.remove(path);
        self.files.insert(path.to_owned(), file);
    }

    /// Holds the conflict `k` at `path`, in place of whatever was there.
    fn keep(&mut self, path: &str, k: Kept) {
        self.remove(path);
        self.add_folders_above(path);
        self.kept.insert(path.to_owned(), k);
    }

    /// Everything strictly below the folder `path`, its paths taken from there.
    fn below(&self, path: &str) -> Tree {
        let rest = |p: &str| p.strip_prefix(path)?.strip_prefix('/').map(str::to_owned);
        Tree {
            files: (self.files.iter())
                .filter_map(|(p, f)| Some((rest(p)?, f.clone())))
                .collect(),
            folders: self.folders.iter().filter_map(|p| rest(p)).collect(),
            kept: (self.kept.iter())
                .filter_map(|(p, k)| Some((rest(p)?, k.clone())))
                .collect(),
        }
    }

    /// Makes `path` hold `part`, or nothing.
    fn place(&mut self, path: &str, part: Option<&Part>) {
        self.remove(path);
        match part {
            Some(Part::File(f)) => self.put(path, f.clone()),
            Some(Part::Folder(t)) => {
                self.add_folder(path);
                for (p, f) in &t.files {
                    self.put(&format!("{path}/{p}"), f.clone());
                }
                for p in &t.folders {
                    self.add_folder(&format!("{path}/{p}"));
                }
                for (p, k) in &t.kept {
                    self.keep(&format!("{path}/{p}"), k.clone());
                }
            }
            None => {}
        }
    }

    /// The tree as it stands in a folder: each place in conflict holding what it stands as.
    fn standing(&self) -> Tree {
        let mut t = Tree {
            files: self.files.clone(),
            folders: self.folders.clone(),
            kept: BTreeMap::new(),
        };
        for (p, k) in &self.kept {
            t.place(p, k.stands());
        }
        t
    }

    fn add_folder(&mut self, path: &str) {
        self.add_folders_above(path);
        self.folders.insert(path.to_owned());
    }

    fn add_folders_above(&mut self, path: &str) {
        let mut at = path;
        while let Some((up, _)) = at.rsplit_once('/') {
            self.folders.insert(up.to_owned());
            at = up;
        }
    }

    /// Drops `path` and everything in it.
    fn remove(&mut self, path: &str) {
        self.files.retain(|p, _| !within(p, path));
        self.folders.retain(|p| !within(p, path));
        self.kept.retain(|p, _| !within(p, path));
    }

    /// Makes `path` and everything in it what `other` has there.
    fn take(&mut self, path: &str, other: &Tree) {
        self.remove(path);
        for (p, f) in other.files.iter().filter(|(p, _)| within(p, path)) {
            self.put(p, f.clone());
        }
        for p in other.folders.iter().filter(|p| within(p, path)) {
            self.add_folder(p);
        }
        for (p, k) in other.kept.iter().filter(|(p, _)| within(p, path)) {
            self.keep(p, k.clone());
        }
    }

    /// Makes the entry at `path` what `other` has there: a file, a folder, or nothing.
    fn set(&mut self, path: &str, other: &Tree) {
        if let Some(k) = other.kept.get(path) {
            self.keep(path, k.clone());
        } else if let Some(f) = other.files.get(path) {
            self.put(path, f.clone());
        } else if other.folders.contains(path) {
            self.add_folder(path);
        } else {
            self.remove(path);
        }
    }
}

fn within(p: &str, top: &str) -> bool {
    p == top
        || p.strip_prefix(top)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The files that differ between two trees.
fn changed_files(a: &Tree, b: &Tree) -> BTreeSet<String> {
    a.files
        .keys()
        .chain(b.files.keys())
        .filter(|p| a.files.get(*p) != b.files.get(*p))
        .cloned()
        .collect()
}

/// The paths where two trees differ: a file, or a folder on one side only.
fn differences(a: &Tree, b: &Tree) -> BTreeSet<String> {
    let mut paths = changed_files(a, b);
    paths.extend(a.folders.symmetric_difference(&b.folders).cloned());
    paths
}

/// What a switch of `folder` from the save `from` to `into` leaves, as `goto` makes it, or the
/// paths where unsaved work is in its way.
fn switch(folder: &Tree, from: &Tree, into: &Tree) -> Result<Tree, BTreeSet<String>> {
    let unsaved = changed_files(folder, from);
    let unlike_target: BTreeSet<String> = unsaved
        .intersection(&changed_files(folder, into))
        .cloned()
        .collect();
    // A change unlike the target collides at, above or below a path where the target differs
    // from the save and the folder isn't already as the target is; matching work comes along.
    let in_the_way: BTreeSet<String> = differences(from, into)
        .intersection(&differences(folder, into))
        .filter(|s| unlike_target.iter().any(|u| within(u, s) || within(s, u)))
        .cloned()
        .collect();
    if !in_the_way.is_empty() {
        return Err(in_the_way);
    }
    // The target's own differences, around the unsaved work that comes along.
    let mut after = folder.clone();
    for p in from.folders.difference(&into.folders) {
        after.remove(p);
    }
    // Folders made since the save stay, with the folders above them.
    for p in folder.folders.difference(&from.folders) {
        after.add_folder(p);
    }
    for p in changed_files(from, into) {
        match into.files.get(&p) {
            Some(f) => after.put(&p, f.clone()),
            None => drop(after.files.remove(&p)),
        }
    }
    for p in into.folders.difference(&from.folders) {
        after.add_folder(p);
    }
    Ok(after)
}

/// [`switch`], with the unsaved work in its way set aside first if `shelve` says so: the folder
/// after and the shelf made on `tip`, the save the project is on, or the paths still in the way.
fn switch_or_shelve(
    folder: &Tree,
    from: &Tree,
    into: &Tree,
    tip: &Tree,
    shelve: bool,
) -> Result<(Tree, Option<Tree>), BTreeSet<String>> {
    let paths = match switch(folder, from, into) {
        Ok(after) => return Ok((after, None)),
        Err(paths) if !shelve => return Err(paths),
        Err(paths) => paths,
    };
    let aside = set_aside(folder, tip, &paths);
    // One with nothing set aside at, above or below it is still in the way.
    let left: BTreeSet<String> = paths
        .into_iter()
        .filter(|c| !aside.iter().any(|p| within(p, c) || within(c, p)))
        .collect();
    if !left.is_empty() {
        return Err(left);
    }
    let (mut shelf, mut from) = (tip.clone(), from.clone());
    for p in &aside {
        shelf.set(p, folder);
        from.set(p, folder);
    }
    let after = switch(folder, &from, into)?;
    Ok((after, Some(shelf)))
}

/// What `--shelve` sets aside of the work at and below `paths`, on a shelf on `tip`: each path
/// where a discard of those paths changes the folder, folders above them that `tip` holds included.
fn set_aside(folder: &Tree, tip: &Tree, paths: &BTreeSet<String>) -> Vec<String> {
    let discarded = |p: &String| {
        paths
            .iter()
            .any(|c| within(p, c) || (tip.folders.contains(p) && within(c, p)))
    };
    differences(folder, tip)
        .into_iter()
        .filter(discarded)
        .collect()
}

/// A save the model knows.
#[derive(Debug, Clone)]
struct Saved {
    id: Id,
    /// What it stands as in a folder, which `goto` writes.
    tree: Tree,
    /// As stored, its places in conflict included; a shelf's is what it stands as.
    stored: Tree,
    /// Its first parent.
    parent: Option<usize>,
    /// Its other parents: a combined save's target.
    others: Vec<usize>,
    /// An amend keeps the change of the save it replaces.
    change: usize,
    message: String,
    /// A shelf's save, which no line reaches.
    shelf: bool,
}

/// The lines, the tags, where the project is and the shelves, newest first; saves by index.
#[derive(Debug, Clone, PartialEq, Eq)]
struct State {
    lines: BTreeMap<String, usize>,
    tags: BTreeMap<String, usize>,
    /// The line the project is on, or `None` at the save `on`.
    line: Option<String>,
    on: usize,
    shelves: Vec<(String, usize)>,
}

/// A step: a command's diary entry, with the state and the folder before and after it.
#[derive(Debug, Clone)]
struct Step {
    entry: Id,
    command: &'static str,
    /// `None` for the diary's first entry, which can't be undone.
    before: Option<State>,
    after: State,
    folder: (Tree, Tree),
    /// A combine that made places in conflict, not one that moved the line forward.
    made_conflicts: bool,
}

impl Step {
    fn wrote(&self) -> bool {
        self.folder.0 != self.folder.1
    }

    /// What it changed, as undo's list tells it, `files` being its folder change.
    fn effects(&self, m: &Model, files: Option<FolderChange>) -> Vec<Effect> {
        let Some(before) = &self.before else {
            return Vec::new();
        };
        let (from, to) = (recorded(m, before), recorded(m, &self.after));
        let mut found = Vec::new();
        let names: BTreeSet<&String> = from.lines.keys().chain(to.lines.keys()).collect();
        for name in names {
            let (was, now) = (from.lines.get(name).copied(), to.lines.get(name).copied());
            if was != now {
                let name = name.clone();
                found.push(Effect::Line {
                    name,
                    from: was,
                    to: now,
                });
            }
        }
        if from.position != to.position {
            found.push(Effect::Position {
                from: from.position.clone(),
                to: to.position.clone(),
            });
        }
        let holds = |state: &diary::State, save: &Id| state.shelves.iter().any(|(_, s)| s == save);
        for (label, save) in &to.shelves {
            if !holds(&from, save) {
                let (label, save) = (label.clone(), *save);
                found.push(Effect::ShelfAdded { label, save });
            }
        }
        for (label, save) in &from.shelves {
            if !holds(&to, save) {
                let (label, save) = (label.clone(), *save);
                found.push(Effect::ShelfGone { label, save });
            }
        }
        if from.tags != to.tags {
            found.push(Effect::Tags);
        }
        found.extend(files.map(|FolderChange { from, to }| Effect::Files { from, to }));
        found
    }
}

/// What a new diary entry should record.
#[derive(Debug, Clone, Copy)]
enum Made {
    Command(&'static str),
    Undo(Id),
    Redo(Id),
}

struct Model {
    folder: Tree,
    saves: Vec<Saved>,
    state: State,
    /// The steps undo can take, the diary's first entry first, and those redo can bring back,
    /// the next one last.
    done: Vec<Step>,
    undone: Vec<Step>,
    /// Every diary entry, oldest first, with its state.
    entries: Vec<(Id, State)>,
    /// Files whose modified time an earlier file held (`When::Same`), so the file cache may
    /// take them for that file wherever they go.
    shared_time: BTreeSet<String>,
    past: u64,
    new_lines: usize,
    /// Text lines made so far: the next is new to every file.
    fresh: usize,
    /// The step being made is a combine that made places in conflict.
    made_conflicts: bool,
}

impl Model {
    /// A project just started: an empty save on `main`.
    fn new(repo: &Repository) -> Model {
        let (head, entry) = repo.current().unwrap();
        let first = entry.state.lines["main"];
        let message = repo.store().save(&first, false).unwrap().message;
        let state = State {
            lines: BTreeMap::from([("main".to_owned(), 0)]),
            tags: BTreeMap::new(),
            line: Some("main".to_owned()),
            on: 0,
            shelves: Vec::new(),
        };
        let start = Step {
            entry: head,
            command: "start",
            before: None,
            after: state.clone(),
            folder: Default::default(),
            made_conflicts: false,
        };
        Model {
            folder: Tree::default(),
            saves: vec![Saved {
                id: first,
                tree: Tree::default(),
                stored: Tree::default(),
                parent: None,
                others: Vec::new(),
                change: 0,
                message,
                shelf: false,
            }],
            done: vec![start],
            undone: Vec::new(),
            entries: vec![(head, state.clone())],
            state,
            shared_time: BTreeSet::new(),
            past: 0,
            new_lines: 0,
            fresh: 0,
            made_conflicts: false,
        }
    }

    fn head(&self) -> Id {
        self.entries[self.entries.len() - 1].0
    }

    /// What the save the project is on stands as.
    fn tip(&self) -> &Tree {
        &self.saves[self.state.on].tree
    }

    /// The places in conflict the save the project is on holds.
    fn kept(&self) -> &BTreeMap<String, Kept> {
        &self.saves[self.state.on].stored.kept
    }
}

/// `s` as the diary records it.
fn recorded(m: &Model, s: &State) -> diary::State {
    let id = |i: &usize| m.saves[*i].id;
    diary::State {
        lines: s.lines.iter().map(|(n, i)| (n.clone(), id(i))).collect(),
        tags: tag_list(m, &s.tags),
        position: match &s.line {
            Some(line) => Position::OnLine(line.clone()),
            None => Position::AtSave(id(&s.on)),
        },
        shelves: s.shelves.iter().map(|(l, i)| (l.clone(), id(i))).collect(),
        server_lines: None,
    }
}

/// The record a state's tags are kept in, none if it has none. The model's tags have no message.
fn tag_list(m: &Model, tags: &BTreeMap<String, usize>) -> Option<Id> {
    if tags.is_empty() {
        return None;
    }
    let tag = |save: usize| Tag {
        save: m.saves[save].id,
        message: String::new(),
        git_tags: Vec::new(),
    };
    let list = TagList {
        tags: tags.iter().map(|(n, &s)| (n.clone(), tag(s))).collect(),
    };
    // Built with the engine's own encoder; the encoding has format tests of its own.
    let flavor = ObjectKind::TagList.flavor().unwrap();
    Some(hash_record(flavor, &list.encode()))
}

fn who() -> Person {
    Person {
        name: "Test".into(),
        email: "test@example.com".into(),
        time_ms: 1_700_000_000_000,
        tz_offset_min: 0,
    }
}

fn run(acts: &[Act]) {
    let env = Env::empty();
    let root = env.root();
    let mut repo = Repository::init(root, &who()).unwrap();
    // Without a key, as without the user's own, the file cache is never read or written.
    repo.set_cache_key([7; 32]);
    let mut m = Model::new(&repo);
    // A setup is followed by an undo and a redo of it, or of what stopped them.
    let acts: Vec<Act> = acts
        .iter()
        .flat_map(|a| match a {
            Act::Setup(_) => vec![
                a.clone(),
                Act::Undo {
                    n: 1,
                    shelve: false,
                },
                Act::Redo {
                    n: 1,
                    shelve: false,
                },
            ],
            _ => vec![a.clone()],
        })
        .collect();
    for (n, a) in acts.iter().enumerate() {
        let missable = apply(&mut repo, &mut m, root, a);
        check(
            &repo,
            &m,
            root,
            matches!(a, Act::FullStatus),
            missable.as_deref(),
        )
        .unwrap_or_else(|e| panic!("after action {n}, {a:?}: {e}"));
    }
    for s in &m.saves {
        let save = repo.store().save(&s.id, false).unwrap();
        let parents: Vec<Id> = (s.parent.iter().chain(&s.others))
            .map(|&p| m.saves[p].id)
            .collect();
        assert_eq!(
            (save.message.as_str(), &save.parents),
            (s.message.as_str(), &parents),
            "save {:?}",
            s.id
        );
        let stored = saved_tree(repo.store(), s.id);
        if s.shelf {
            assert_eq!(stored.standing(), s.tree, "shelf {:?}", s.id);
        } else {
            assert_eq!(stored, s.stored, "save {:?}", s.id);
        }
        // A shelf notes nothing; any other save, what moved since its first parent.
        let want = match s.parent {
            Some(p) if !s.shelf => same_contents(&m.saves[p].stored, &s.stored),
            _ => Vec::new(),
        };
        let noted: Vec<(String, String)> = (save.move_notes.iter())
            .map(|n| (n.from.clone(), n.to.clone()))
            .collect();
        assert_eq!(noted, want, "the move notes of save {:?}", s.id);
    }
    // A process for every action would be too slow on Windows: once a case pins the document.
    listed_document(&env, &repo, &m);
}

/// Applies `a` to the folder and the model. Gives the path of a change the file cache may
/// miss, if it made one.
fn apply(repo: &mut Repository, m: &mut Model, root: &Path, a: &Act) -> Option<String> {
    let who = who();
    let before = m.folder.clone();
    let missable = match *a {
        Act::Write(path, b, len, when) => write(m, root, path, b, len, when),
        Act::Rewrite(path, b, when) => {
            let len = m.folder.files.get(path)?.bytes.len();
            write(m, root, path, b, len, when)
        }
        Act::Big(path, seed, len) => put(m, root, path, noise(seed, len), When::Now),
        Act::Edit {
            pick,
            at,
            bytes,
            insert,
            when,
        } => {
            let big: Vec<(&String, &File)> = m
                .folder
                .files
                .iter()
                .filter(|(_, f)| f.bytes.len() >= BIG)
                .collect();
            let (path, old) = big.get(pick % big.len().max(1))?;
            let (path, old) = ((*path).clone(), &old.bytes);
            let edited = if insert {
                let at = at % (old.len() + 1);
                [&old[..at], &bytes, &old[at..]].concat()
            } else {
                let at = at % (old.len() - bytes.len());
                [&old[..at], &bytes, &old[at + bytes.len()..]].concat()
            };
            put(m, root, &path, edited, when)
        }
        Act::Flip(path) => {
            flip(m, root, path);
            None
        }
        Act::Delete(path) => {
            m.folder.files.remove(path)?;
            std::fs::remove_file(root.join(path)).unwrap();
            None
        }
        Act::Rename(from, to) => {
            if from == to {
                return None;
            }
            let file = m.folder.files.remove(from)?;
            let at = root.join(to);
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            std::fs::rename(root.join(from), &at).unwrap();
            m.folder.put(to, file);
            let shared = m.shared_time.remove(from);
            m.shared_time.remove(to);
            if shared {
                m.shared_time.insert(to.to_owned());
            }
            let time = std::fs::metadata(&at).unwrap().modified().unwrap();
            (shared && long_ago(time)).then(|| to.to_owned())
        }
        Act::MakeFolder(path) => {
            std::fs::create_dir_all(root.join(path)).unwrap();
            m.folder.add_folder(path);
            None
        }
        Act::DeleteFolder(path) => {
            if m.folder.folders.contains(path) {
                std::fs::remove_dir_all(root.join(path)).unwrap();
                m.folder.remove(path);
            }
            None
        }
        Act::Save => {
            save(repo, m, None);
            None
        }
        Act::SaveOnNewLine(name) => {
            save(repo, m, Some(name));
            None
        }
        Act::LineNew(name) => {
            make_line(repo, m, name);
            None
        }
        Act::LineRename { pick, old, new } => {
            let n = m.state.lines.len();
            // One slot past the lines is `old`, the next the current line.
            let old = match pick % (n + 2) {
                k if k < n => m.state.lines.keys().nth(k).cloned(),
                k if k == n => Some(old.to_owned()),
                _ => None,
            };
            rename(repo, m, old, new);
            None
        }
        Act::LineDelete { pick, name, yes } => {
            let name = chosen(m.state.lines.keys(), pick, name);
            delete_line(repo, m, &name, yes);
            None
        }
        Act::Tag { name, move_it } => {
            tag_here(repo, m, name, move_it);
            None
        }
        Act::TagDelete { pick, name, yes } => {
            let name = chosen(m.state.tags.keys(), pick, name);
            delete_tag(repo, m, &name, yes);
            None
        }
        Act::Amend { new_message } => {
            amend(repo, m, new_message);
            None
        }
        Act::Goto {
            pick,
            by_line,
            unsaved,
        } => {
            if m.folder != *m.tip() {
                match unsaved {
                    Unsaved::Save => save(repo, m, None),
                    Unsaved::Discard => discard_all(repo, m),
                    Unsaved::Keep => {}
                }
            }
            // Every line may have been deleted.
            let (target, to, line) = if by_line && !m.state.lines.is_empty() {
                let lines = &m.state.lines;
                let (name, &to) = lines.iter().nth(pick % lines.len()).unwrap();
                (Target::Line(name.clone()), to, Some(name.clone()))
            } else {
                let saves: Vec<usize> = (0..m.saves.len()).filter(|&i| !m.saves[i].shelf).collect();
                let to = saves[pick % saves.len()];
                (Target::Save(m.saves[to].id), to, None)
            };
            let done = goto(repo, &target, &who, "goto", None, &mut |_| {});
            match switch(&m.folder, m.tip(), &m.saves[to].tree) {
                Err(_) => {
                    assert!(
                        matches!(done, Err(SwitchError::Collisions { .. })),
                        "{done:?}"
                    );
                    settle(repo, m, Vec::new());
                }
                Ok(folder) => {
                    done.unwrap();
                    let (was, held) = (m.state.clone(), std::mem::replace(&mut m.folder, folder));
                    m.state.on = to;
                    m.state.line = line;
                    stepped(repo, m, "goto", was, held);
                }
            }
            None
        }
        Act::Discard(None) => {
            discard_all(repo, m);
            None
        }
        Act::Discard(Some(path)) => {
            let saved = m.tip().clone();
            let at = |t: &Tree| {
                t.files.keys().any(|p| within(p, path)) || t.folders.iter().any(|p| within(p, path))
            };
            let paths = [path.to_owned()];
            let done = discard(repo, Some(&paths), &who, "discard", None, &mut |_| {});
            if !at(&saved) && !at(&m.folder) {
                assert!(matches!(done, Err(SwitchError::NoMatch { .. })), "{done:?}");
                settle(repo, m, Vec::new());
                return None;
            }
            done.unwrap();
            let (was, held) = (m.state.clone(), m.folder.clone());
            m.folder.take(path, &saved);
            stepped(repo, m, "discard", was, held);
            None
        }
        Act::Undo { n, shelve } | Act::Redo { n, shelve } => {
            let again = matches!(a, Act::Redo { .. });
            let req = StepRequest {
                n,
                shelve,
                who: &who,
                git_excludes: None,
            };
            let done = if again {
                redo(repo, &req, &mut |_| {})
            } else {
                undo(repo, &req, &mut |_| {})
            };
            take_steps(repo, m, (n, None), (again, shelve), done);
            None
        }
        Act::UndoTo { pick, shelve } => {
            go_back_to(repo, m, pick, shelve);
            None
        }
        Act::Shelf => {
            shelve(repo, m);
            None
        }
        Act::Unshelf(label) => {
            bring_back(repo, m, label);
            None
        }
        Act::FullStatus => None,
        Act::Setup(preset) => {
            set_up(repo, m, root, preset);
            None
        }
        Act::Text { path, at, edit } => edit_text(m, root, path, at, edit),
        Act::Moved { pick, to, remake } => {
            let files: Vec<&str> = FILES
                .into_iter()
                .filter(|p| m.folder.files.contains_key(*p))
                .collect();
            let from = *files.get(pick % files.len().max(1))?;
            apply(repo, m, root, &Act::Rename(from, to));
            if remake && from != to {
                edit_text(m, root, from, 0, 0);
            }
            save(repo, m, None);
            None
        }
        Act::Fork {
            pick,
            path,
            at,
            edit,
            back,
            changes,
            clean,
        } => {
            // A text the save holds, saved first if there is none, so both sides change it.
            let texts: Vec<&'static str> = (FILES.into_iter())
                .filter(|p| (m.tip().files.get(*p)).is_some_and(|f| text_lines(&f.bytes).is_some()))
                .collect();
            let path = match texts.get(at % texts.len().max(1)) {
                Some(&text) => text,
                None => {
                    edit_text(m, root, path, at, edit);
                    save(repo, m, None);
                    path
                }
            };
            let empty: Vec<&'static str> = (FILES.into_iter())
                .filter(|p| !m.folder.files.contains_key(*p))
                .collect();
            change_file(repo, m, root, changes.0, path, (at, edit), &empty);
            save(repo, m, None);
            let first = m.state.on;
            // An older save than this one, so the two lines part: mostly its parent, where the
            // first edit is all that differs.
            let mut mine = history(m, &[m.state.on]);
            if mine.len() > 1 {
                mine.remove(&m.state.on);
            }
            let to = match m.saves[m.state.on].parent {
                Some(parent) if pick % 4 != 0 => parent,
                _ => *mine.iter().nth(pick % mine.len()).unwrap(),
            };
            let saves = (0..m.saves.len()).filter(|&i| !m.saves[i].shelf);
            let pick = saves.into_iter().position(|s| s == to).unwrap();
            let unsaved = Unsaved::Save;
            let goto = Act::Goto {
                pick,
                by_line: false,
                unsaved,
            };
            apply(repo, m, root, &goto);
            change_file(repo, m, root, changes.1, path, (at, edit), &empty);
            save(repo, m, None);
            if back {
                combine_into(repo, m, &Aim::Save(first), None, keep_of(clean));
            }
            None
        }
        Act::Combine {
            pick,
            message,
            clean,
        } => {
            let aims = aims(m);
            let message = message.then_some("Combined");
            combine_into(repo, m, &aims[pick % aims.len()], message, keep_of(clean));
            None
        }
        Act::CrissCross { pick, fix, clean } => {
            let mut here = m.state.line.clone()?;
            let others: Vec<String> = (aims(m).into_iter())
                .filter_map(|a| match a {
                    Aim::Line(line) if line != here => Some(line),
                    _ => None,
                })
                .collect();
            let mut there = others.get(pick % others.len().max(1)).cloned();
            if fix {
                // Two lines that each changed one text in the same place, from this one.
                let fork = Act::Fork {
                    pick,
                    path: "a",
                    at: pick,
                    edit: (pick % 3) as u8,
                    back: false,
                    changes: (Change::Edit, Change::Edit),
                    clean: false,
                };
                apply(repo, m, root, &fork);
                there = Some(here);
                here = m.state.line.clone()?;
                if !m.state.lines.contains_key(there.as_ref()?) || Some(&here) == there.as_ref() {
                    return None;
                }
            }
            let there = there?;
            let old = m.state.on;
            // Each line keeps its own side of what its combine left in conflict.
            let own_side = |repo: &mut Repository, m: &mut Model| {
                if fix && !m.kept().is_empty() {
                    let whole = Some(Asked::Path(String::new()));
                    resolve_at(repo, m, whole, ResolveHow::KeepMine, true);
                }
            };
            combine_into(repo, m, &Aim::Line(there.clone()), None, Keep::Conflicts);
            own_side(repo, m);
            let pick = m.state.lines.keys().position(|l| *l == there).unwrap();
            let unsaved = Unsaved::Discard;
            let goto = Act::Goto {
                pick,
                by_line: true,
                unsaved,
            };
            apply(repo, m, root, &goto);
            combine_into(repo, m, &Aim::Save(old), None, Keep::Conflicts);
            own_side(repo, m);
            combine_into(repo, m, &Aim::Line(here), None, keep_of(clean));
            None
        }
        Act::Resolve { pick, how } => {
            let places: Vec<String> = m.kept().keys().cloned().collect();
            // The places, and for a path also the folders above them, a file in no conflict
            // and the whole project; a keep, the places and the whole project.
            let mut targets = places.clone();
            match how {
                ResolveHow::Paths => {
                    let above = (FOLDERS.into_iter())
                        .filter(|f| places.iter().any(|p| within(p, f) && p != f));
                    targets.extend(above.map(str::to_owned));
                    let calm = (FILES.into_iter())
                        .find(|f| !places.iter().any(|p| within(p, f) || within(f, p)));
                    targets.extend(calm.map(str::to_owned));
                    targets.push(String::new());
                }
                ResolveHow::KeepMine | ResolveHow::KeepTheirs => targets.push(String::new()),
                ResolveHow::Pattern => {
                    // A pattern takes only what the folder shows fixed: mostly one that is.
                    let fixed: Vec<String> = (conflicts_listed(m).into_iter())
                        .filter(|(_, s)| *s == Standing::Fixed)
                        .map(|(p, _)| p)
                        .collect();
                    if !fixed.is_empty() && pick % 4 != 0 {
                        targets = fixed;
                    }
                }
                ResolveHow::All => {}
            }
            let target = targets.get(pick % targets.len().max(1)).cloned();
            let asked = match how {
                ResolveHow::All => None,
                ResolveHow::Pattern => Some(Asked::Pattern(target?)),
                _ => Some(Asked::Path(target.unwrap_or_default())),
            };
            resolve_at(repo, m, asked, how, (pick / 7).is_multiple_of(2));
            None
        }
        Act::FixFile { pick } => fix_file(m, root, pick),
        Act::ChangeInPlace { pick } => change_in_place(m, root, pick),
        Act::Tidy => {
            tidy_and_check(repo, m);
            None
        }
        Act::SaveOnly { pick } => {
            // A place in conflict, or the folder above one.
            let mut targets: Vec<String> = m.kept().keys().cloned().collect();
            let above = (targets.iter())
                .filter_map(|p| p.rsplit_once('/').map(|(up, _)| up.to_owned()))
                .collect::<BTreeSet<String>>();
            targets.extend(above);
            let path = targets.get(pick % targets.len().max(1))?.clone();
            save_only(repo, m, &path);
            None
        }
        Act::RenameInConflict { pick, to } => {
            let places: Vec<&String> = (m.kept().keys())
                .filter(|p| m.folder.files.contains_key(*p))
                .collect();
            let from = places.get(pick % places.len().max(1))?.to_string();
            let from = *FILES.iter().find(|f| **f == from)?;
            if from == to || m.folder.files.contains_key(to) {
                return None;
            }
            apply(repo, m, root, &Act::Rename(from, to));
            met("a rename at a place in conflict");
            save(repo, m, None);
            None
        }
        Act::CleanForward { pick } => {
            // A line whose save is in the history of another line's save that holds a conflict.
            let lines = &m.state.lines;
            let pairs: Vec<(String, String)> = (lines.iter())
                .flat_map(|(a, &sa)| {
                    (lines.iter())
                        .filter(move |&(b, &sb)| a != b && sa != sb)
                        .map(move |(b, &sb)| (a.clone(), b.clone(), sa, sb))
                })
                .filter(|(_, _, sa, sb)| {
                    !m.saves[*sb].stored.kept.is_empty() && history(m, &[*sb]).contains(sa)
                })
                .map(|(a, b, _, _)| (a, b))
                .collect();
            let (a, b) = pairs.get(pick % pairs.len().max(1))?.clone();
            let pick = m.state.lines.keys().position(|l| *l == a).unwrap();
            let goto = Act::Goto {
                pick,
                by_line: true,
                unsaved: Unsaved::Discard,
            };
            apply(repo, m, root, &goto);
            if m.state.line.as_deref() == Some(a.as_str()) {
                combine_into(repo, m, &Aim::Line(b), None, Keep::OnlyIfClean);
            }
            None
        }
        Act::Conflicted {
            pick,
            picture,
            then,
        } => {
            in_conflict(repo, m, root, pick, picture);
            then_with_conflicts(repo, m, root, pick, then);
            None
        }
        Act::UndoRedo { tidy } => {
            let back = Act::Undo { n: 1, shelve: true };
            apply(repo, m, root, &back);
            if tidy {
                tidy_and_check(repo, m);
            }
            let again = Act::Redo { n: 1, shelve: true };
            apply(repo, m, root, &again);
            None
        }
    };
    let wrote = matches!(
        a,
        Act::Goto { .. }
            | Act::Combine { .. }
            | Act::CrissCross { .. }
            | Act::Resolve { .. }
            | Act::CleanForward { .. }
            | Act::UndoRedo { .. }
            | Act::Conflicted { .. }
            | Act::Fork { back: true, .. }
            | Act::Discard(_)
            | Act::Undo { .. }
            | Act::Redo { .. }
            | Act::UndoTo { .. }
            | Act::Shelf
            | Act::Unshelf(_)
            | Act::Setup(_)
    );
    if wrote {
        // Files Opalro wrote or removed have times of their own now.
        let after = &m.folder;
        m.shared_time
            .retain(|p| after.files.contains_key(p) && before.files.get(p) == after.files.get(p));
    }
    missable
}

/// The two texts `in_conflict` puts in conflict, in one folder.
const TWO_TEXTS: [&str; 2] = ["d/a", "d/c"];

/// Makes places in conflict on the line the project is on: both texts of `TWO_TEXTS` given a
/// line in the same place on two lines, combined; or with `picture`, a text one line made bytes
/// that aren't text, which stands as yours.
fn in_conflict(repo: &mut Repository, m: &mut Model, root: &Path, pick: usize, picture: bool) {
    if picture {
        let fork = Act::Fork {
            pick: 1,
            path: "a",
            at: pick,
            edit: 2,
            back: true,
            changes: (Change::Binary, Change::Edit),
            clean: false,
        };
        apply(repo, m, root, &fork);
        return;
    }
    for p in TWO_TEXTS {
        if (m.folder.files.get(p)).is_none_or(|f| text_lines(&f.bytes).is_none()) {
            let _ = std::fs::remove_file(root.join(p));
            m.folder.files.remove(p);
            edit_text(m, root, p, 0, 0);
        }
    }
    save(repo, m, None);
    let edit_both = |m: &mut Model| {
        for p in TWO_TEXTS {
            edit_text(m, root, p, pick, 2);
        }
    };
    edit_both(m);
    save(repo, m, None);
    let first = m.state.on;
    let Some(parent) = m.saves[first].parent else {
        return;
    };
    let saves = (0..m.saves.len()).filter(|&i| !m.saves[i].shelf);
    let pick_parent = saves.into_iter().position(|s| s == parent).unwrap();
    let goto = Act::Goto {
        pick: pick_parent,
        by_line: false,
        unsaved: Unsaved::Save,
    };
    apply(repo, m, root, &goto);
    edit_both(m);
    save(repo, m, None);
    combine_into(repo, m, &Aim::Save(first), None, Keep::Conflicts);
}

/// What `Conflicted` does with the places in conflict it made, as `then` picks.
fn then_with_conflicts(repo: &mut Repository, m: &mut Model, root: &Path, pick: usize, then: u8) {
    let places: Vec<String> = m.kept().keys().cloned().collect();
    let Some(place) = places.get(pick % places.len().max(1)).cloned() else {
        return;
    };
    let fix_all = |m: &mut Model| {
        for k in 0..places.len() {
            fix_file(m, root, k * 6 + 1);
        }
    };
    let path = |p: &str| Some(Asked::Path(p.to_owned()));
    let yes = (pick / 7).is_multiple_of(2);
    match then % 14 {
        0 => {
            fix_all(m);
            resolve_at(repo, m, None, ResolveHow::All, true);
        }
        1 | 13 => {
            fix_all(m);
            let asked = Some(Asked::Pattern(place));
            resolve_at(repo, m, asked, ResolveHow::Pattern, true);
        }
        2 => drop(resolve_at(repo, m, path(&place), ResolveHow::Paths, yes)),
        3 => drop(resolve_at(repo, m, path("d"), ResolveHow::Paths, yes)),
        4 => drop(resolve_at(
            repo,
            m,
            path(&place),
            ResolveHow::KeepMine,
            true,
        )),
        5 => drop(resolve_at(
            repo,
            m,
            path(&place),
            ResolveHow::KeepTheirs,
            true,
        )),
        6 => {
            let calm = FILES.into_iter().find(|f| !places.iter().any(|p| p == f));
            if let Some(calm) = calm {
                resolve_at(repo, m, path(calm), ResolveHow::Paths, true);
            }
        }
        7 => {
            let to = FILES.into_iter().find(|f| !m.folder.files.contains_key(*f));
            if let Some(to) = to {
                apply(repo, m, root, &Act::RenameInConflict { pick, to });
            }
        }
        8 => {
            fix_file(m, root, pick);
            change_in_place(m, root, pick);
            save_only(repo, m, &place);
        }
        9 => {
            let asked = Some(Asked::Path(String::new()));
            resolve_at(repo, m, asked, ResolveHow::KeepTheirs, true);
            apply(repo, m, root, &Act::UndoRedo { tidy: false });
        }
        10 => {
            fix_file(m, root, pick);
            save(repo, m, None);
            let tidy = pick.is_multiple_of(2);
            apply(repo, m, root, &Act::UndoRedo { tidy });
        }
        11 => {
            // Without a side, a picture still standing as yours is refused.
            let picture = (m.kept().iter())
                .find(|(_, k)| k.marker() == 0)
                .map(|(p, _)| p.clone());
            let at = picture.unwrap_or(place);
            resolve_at(repo, m, path(&at), ResolveHow::Paths, yes);
        }
        _ => carried_into_a_clean_line(repo, m, root),
    }
}

/// From the line the project is on, whose save holds a conflict: another line made from that
/// save's first parent with a change elsewhere, combined in asking for a clean line, so the
/// conflict would be carried.
fn carried_into_a_clean_line(repo: &mut Repository, m: &mut Model, root: &Path) {
    let (Some(here), Some(parent)) = (m.state.line.clone(), m.saves[m.state.on].parent) else {
        return;
    };
    let saves = (0..m.saves.len()).filter(|&i| !m.saves[i].shelf);
    let pick = saves.into_iter().position(|s| s == parent).unwrap();
    let goto = Act::Goto {
        pick,
        by_line: false,
        unsaved: Unsaved::Save,
    };
    apply(repo, m, root, &goto);
    edit_text(m, root, "b", 0, 2);
    save(repo, m, None);
    let Some(there) = m.state.line.clone().filter(|l| *l != here) else {
        return;
    };
    let Some(pick) = m.state.lines.keys().position(|l| *l == here) else {
        return;
    };
    let goto = Act::Goto {
        pick,
        by_line: true,
        unsaved: Unsaved::Save,
    };
    apply(repo, m, root, &goto);
    if m.state.line.as_deref() == Some(here.as_str()) {
        combine_into(repo, m, &Aim::Line(there), None, Keep::OnlyIfClean);
    }
}

/// Makes `change` to the text at `path` in the folder, for a line of a fork; `empty` are the
/// places that were empty when the fork began.
fn change_file(
    repo: &mut Repository,
    m: &mut Model,
    root: &Path,
    change: Change,
    path: &'static str,
    (at, edit): (usize, u8),
    empty: &[&'static str],
) {
    let there = m.folder.files.contains_key(path);
    let pick = |k: usize| empty.get(k % empty.len().max(1)).copied();
    match change {
        Change::Edit => drop(edit_text(m, root, path, at, edit)),
        Change::EditEnd => {
            // Apart from the other line's edit unless that was at the end too.
            let end = (m.folder.files.get(path)).map_or(0, |f| lines_of(&f.bytes).len());
            drop(edit_text(m, root, path, end, 0));
        }
        Change::Remove if there => drop(apply(repo, m, root, &Act::Delete(path))),
        Change::Move(k, remake) if there => {
            if let Some(to) = pick(k) {
                apply(repo, m, root, &Act::Rename(path, to));
                if remake {
                    edit_text(m, root, path, 0, 0);
                }
            }
        }
        Change::Binary => {
            let mut bytes = vec![0];
            bytes.extend(fresh_line(m));
            drop(put(m, root, path, bytes, When::Now));
        }
        Change::Add(k) => {
            if let Some(to) = pick(k).filter(|to| !m.folder.files.contains_key(*to)) {
                drop(edit_text(m, root, to, 0, 0));
            }
        }
        Change::Arrive if there => {
            // Another file, saved where the file was once that place is empty, so it is noted.
            let others: Vec<&'static str> = (FILES.into_iter())
                .filter(|p| {
                    *p != path && m.folder.files.get(*p).is_some_and(|f| !f.bytes.is_empty())
                })
                .collect();
            if let Some(&from) = others.get(at % others.len().max(1)) {
                apply(repo, m, root, &Act::Delete(path));
                save(repo, m, None);
                apply(repo, m, root, &Act::Rename(from, path));
            }
        }
        Change::Remove | Change::Move(..) | Change::Arrive => {}
    }
}

/// Writes `len` bytes to `path`, leaving the modified time `when` says.
fn write(m: &mut Model, root: &Path, path: &str, b: u8, len: usize, when: When) -> Option<String> {
    let bytes: Vec<u8> = (0..len)
        .map(|i| b.wrapping_add((i as u8).wrapping_mul(37)))
        .collect();
    put(m, root, path, bytes, when)
}

/// Writes `bytes` to `path` whole, leaving the modified time `when` says.
fn put(m: &mut Model, root: &Path, path: &str, mut bytes: Vec<u8>, when: When) -> Option<String> {
    let len = bytes.len();
    let old = m.folder.files.get(path).cloned();
    // A text edit can leave a file empty, which a rewrite then leaves as it is.
    if when != When::Same && !bytes.is_empty() && old.as_ref().is_some_and(|o| *o.bytes == *bytes) {
        bytes[0] ^= 1;
    }
    let at = root.join(path);
    let old_time = old
        .as_ref()
        .map(|_| std::fs::metadata(&at).unwrap().modified().unwrap());
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::fs::write(&at, &bytes).unwrap();
    let time = match when {
        When::Now => None,
        When::Past => {
            m.past += 1;
            Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000 + m.past))
        }
        When::Same => old_time,
    };
    if let Some(t) = time {
        let f = std::fs::OpenOptions::new().write(true).open(&at).unwrap();
        f.set_modified(t).unwrap();
    }
    let exec = old.as_ref().is_some_and(|o| o.exec);
    let kept_time = when == When::Same && old.is_some();
    let missable = kept_time
        && old_time.is_some_and(long_ago)
        && old.is_some_and(|o| o.bytes.len() == len && *o.bytes != *bytes);
    let bytes = bytes.into();
    m.folder.put(path, File { bytes, exec });
    if kept_time {
        m.shared_time.insert(path.to_owned());
    } else {
        m.shared_time.remove(path);
    }
    missable.then(|| path.to_owned())
}

/// Whether a file with this modified time may be trusted by the file cache: only one at least
/// 2 s older than a look that recorded it is, and every look so far came before now.
fn long_ago(time: SystemTime) -> bool {
    // 1.5 s leaves room for the drive's clock running a little apart from this one.
    time + Duration::from_millis(1500) < SystemTime::now()
}

#[cfg(unix)]
fn flip(m: &mut Model, root: &Path, path: &str) {
    use std::os::unix::fs::PermissionsExt;
    let Some(file) = m.folder.files.get_mut(path) else {
        return;
    };
    file.exec = !file.exec;
    let mode = if file.exec { 0o755 } else { 0o644 };
    std::fs::set_permissions(root.join(path), std::fs::Permissions::from_mode(mode)).unwrap();
}

#[cfg(not(unix))]
fn flip(_: &mut Model, _: &Path, _: &str) {}

/// `setup --preset`: the preset's `opalro.toml` and `.opalroignore` block written as one step that
/// moves no line, or nothing when the files already are those. The model's names are no
/// language's or engine's, and only this writes `.opalroignore`, so it only ever holds a block.
fn set_up(repo: &mut Repository, m: &mut Model, root: &Path, preset: Preset) {
    let plan = plan_setup(root, preset).unwrap();
    let done = apply_setup(repo, &plan, &who(), None, &mut |_| {}).unwrap();
    let found = Found::default();
    let mut after = m.folder.clone();
    for (path, text) in [
        ("opalro.toml", presets::opal_toml(preset, &found)),
        (".opalroignore", presets::opalroignore_block(preset, &found)),
    ] {
        match text {
            Some(text) => {
                let exec = m.folder.files.get(path).is_some_and(|f| f.exec);
                let bytes = text.into_bytes().into();
                after.put(path, File { bytes, exec });
            }
            None => drop(after.files.remove(path)),
        }
    }
    assert_eq!(done.written.is_empty(), after == m.folder, "{preset:?}");
    let (was, held) = (m.state.clone(), std::mem::replace(&mut m.folder, after));
    stepped(repo, m, "setup", was, held);
}

/// `save`, or `save --line` with `line`, which starts that line where the project is.
fn save(repo: &mut Repository, m: &mut Model, line: Option<&str>) {
    let who = who();
    // A save made at a save starts a line.
    let new_line = match line {
        Some(name) => Some(name.to_owned()),
        None => m
            .state
            .line
            .is_none()
            .then(|| format!("line-{}", m.new_lines)),
    };
    let req = SaveRequest {
        message: Some("Save"),
        who: &who,
        command: "save",
        accept_crlf: false,
        git_excludes: None,
        new_line: new_line.as_deref(),
        only: None,
        amend: false,
    };
    let done = save_folder(repo, &req, &mut |_| {}, &mut |_| true);
    if let Some(name) = line
        && let Some(want) = line_name_refused(m, name, None)
    {
        let refused = matches!(&done, Err(SaveError::Name(e)) if want.is(e, name));
        assert!(refused, "{done:?}, not {want:?}");
        settle(repo, m, Vec::new());
        return;
    }
    let stored = settled(&m.folder, &m.saves[m.state.on].stored);
    match done.unwrap() {
        SaveOutcome::Saved {
            save_id, conflicts, ..
        } => {
            assert_ne!(
                stored, m.saves[m.state.on].stored,
                "saved with nothing changed"
            );
            assert_eq!(saved_tree(repo.store(), save_id), stored);
            assert_eq!(conflicts, stored.kept.len() as u64, "still in conflict");
            count_settled(m.kept(), &stored);
            let was = m.state.clone();
            m.saves.push(Saved {
                id: save_id,
                tree: stored.standing(),
                stored,
                parent: Some(m.state.on),
                others: Vec::new(),
                change: m.saves.len(),
                message: "Save".to_owned(),
                shelf: false,
            });
            m.state.on = m.saves.len() - 1;
            if let Some(name) = new_line {
                if line.is_none() {
                    m.new_lines += 1;
                }
                m.state.line = Some(name);
            }
            m.state
                .lines
                .insert(m.state.line.clone().unwrap(), m.state.on);
            let held = m.folder.clone();
            stepped(repo, m, "save", was, held);
        }
        SaveOutcome::NothingChanged { .. } => {
            assert_eq!(stored, m.saves[m.state.on].stored, "changes not saved");
            settle(repo, m, Vec::new());
        }
    }
}

/// Whether a version is a file that isn't text, which only a person's word settles.
fn is_picture(term: &Option<Part>) -> bool {
    matches!(term, Some(Part::File(f)) if text_lines(&f.bytes).is_none())
}

/// Counts how a save settled the places in conflict `before` held.
fn count_settled(before: &BTreeMap<String, Kept>, after: &Tree) {
    for (p, k) in before {
        match after.kept.get(p) {
            None => met("save fixed a marked text"),
            Some(now) if now == k => {}
            Some(_) if k.marker() > 0 => met("save kept a half-fixed text"),
            Some(_) if k.terms.iter().any(is_picture) => {
                met("save kept a picture changed in place");
            }
            Some(_) => {}
        }
    }
}

/// `save <path>`: the place at or below `path` settled as a full save settles it, everything
/// else as the save the project is on holds it.
fn save_only(repo: &mut Repository, m: &mut Model, path: &str) {
    let who = who();
    let only = [Given::Path(path.to_owned())];
    let req = SaveRequest {
        message: Some("Save"),
        who: &who,
        command: "save",
        accept_crlf: false,
        git_excludes: None,
        new_line: None,
        only: Some(&only),
        amend: false,
    };
    // At a save, a save starts a line: that is the full save's to draw.
    let Some(line) = m.state.line.clone() else {
        return;
    };
    let done = save_folder(repo, &req, &mut |_| {}, &mut |_| true);
    let tip = m.saves[m.state.on].stored.clone();
    let mut stored = tip.clone();
    stored.take(path, &settled(&m.folder, &tip));
    let named = |p: &String| within(p, path);
    match done.unwrap() {
        SaveOutcome::Saved {
            save_id, conflicts, ..
        } => {
            assert_ne!(stored, tip, "saved with nothing changed");
            assert_eq!(saved_tree(repo.store(), save_id), stored, "save {path}");
            let still = stored.kept.keys().filter(|p| named(p)).count() as u64;
            assert_eq!(conflicts, still, "of the places saved, still in conflict");
            if m.kept().keys().any(named) {
                met("save <path> at a place in conflict");
            }
            count_settled(m.kept(), &stored);
            let was = m.state.clone();
            m.saves.push(Saved {
                id: save_id,
                tree: stored.standing(),
                stored,
                parent: Some(m.state.on),
                others: Vec::new(),
                change: m.saves.len(),
                message: "Save".to_owned(),
                shelf: false,
            });
            m.state.on = m.saves.len() - 1;
            m.state.lines.insert(line, m.state.on);
            let held = m.folder.clone();
            stepped(repo, m, "save", was, held);
        }
        SaveOutcome::NothingChanged { .. } => {
            assert_eq!(stored, tip, "changes not saved");
            settle(repo, m, Vec::new());
        }
    }
}

/// What a save holds on top of the save `tip` (as stored): the folder as it is,
/// with each of the tip's places in conflict settled by what the folder shows there. Untouched,
/// the conflict stays as it is; shown with markers, it is fixed once no marker line is left,
/// and else keeps the file as its own; standing as a version, it keeps whatever the folder holds
/// as its own, for only the person's word decides it.
fn settled(folder: &Tree, tip: &Tree) -> Tree {
    let mut out = folder.clone();
    for (p, k) in &tip.kept {
        let now = part_at(folder, p);
        if now.as_ref() == k.stands() {
            out.keep(p, k.clone());
            continue;
        }
        let marker = k.marker();
        if marker == 0 {
            let own = Some((now, 0));
            let terms = k.terms.clone();
            out.keep(p, Kept { terms, own });
        } else if let Some(Part::File(f)) = &now
            && marker_lines(&f.bytes, marker) != (0, 0)
        {
            let own = Some((now.clone(), marker));
            let terms = k.terms.clone();
            out.keep(p, Kept { terms, own });
        }
    }
    out
}

/// `save --amend`: refused at a save, and where another line or a tag shows the save.
fn amend(repo: &mut Repository, m: &mut Model, new_message: bool) {
    let who = who();
    let message = new_message.then_some("Amended");
    let req = SaveRequest {
        message,
        who: &who,
        command: "save",
        accept_crlf: false,
        git_excludes: None,
        new_line: None,
        only: None,
        amend: true,
    };
    let done = save_folder(repo, &req, &mut |_| {}, &mut |_| true);
    let old = m.state.on;
    let Some(line) = m.state.line.clone() else {
        assert!(matches!(done, Err(SaveError::AmendAtSave)), "{done:?}");
        settle(repo, m, Vec::new());
        return;
    };
    if shown_elsewhere(m, &line, old) {
        assert!(
            matches!(done, Err(SaveError::AmendShared { .. })),
            "{done:?}"
        );
        settle(repo, m, Vec::new());
        return;
    }
    let message = message.unwrap_or(m.saves[old].message.as_str()).to_owned();
    let stored = settled(&m.folder, &m.saves[old].stored);
    let same = stored == m.saves[old].stored && message == m.saves[old].message;
    match done.unwrap() {
        SaveOutcome::NothingChanged { .. } => {
            assert!(same, "nothing amended");
            settle(repo, m, Vec::new());
        }
        SaveOutcome::Saved {
            save_id,
            replaced,
            conflicts,
            ..
        } => {
            assert!(!same, "amended with nothing to amend");
            assert_eq!(replaced, Some(m.saves[old].id));
            assert_eq!(saved_tree(repo.store(), save_id), stored);
            assert_eq!(conflicts, stored.kept.len() as u64, "still in conflict");
            count_settled(m.kept(), &stored);
            let was = m.state.clone();
            let (parent, change) = (m.saves[old].parent, m.saves[old].change);
            // An amend keeps every parent.
            let others = m.saves[old].others.clone();
            m.saves.push(Saved {
                id: save_id,
                tree: stored.standing(),
                stored,
                parent,
                others,
                change,
                message,
                shelf: false,
            });
            m.state.on = m.saves.len() - 1;
            m.state.lines.insert(line, m.state.on);
            let held = m.folder.clone();
            stepped(repo, m, "save", was, held);
        }
    }
}

/// Whether a tag is at `save`, or a line other than `line` holds it, or a save of its change,
/// along first parents.
fn shown_elsewhere(m: &Model, line: &str, save: usize) -> bool {
    let (change, id) = (m.saves[save].change, m.saves[save].id);
    m.state.tags.values().any(|&t| m.saves[t].id == id)
        || m.state
            .lines
            .iter()
            .filter(|(name, _)| *name != line)
            .any(|(_, &tip)| {
                std::iter::successors(Some(tip), |&s| m.saves[s].parent)
                    .any(|s| m.saves[s].change == change)
            })
}

/// The name `pick` chooses among `names`, or `other` in the slot past them.
fn chosen<'a>(
    mut names: impl ExactSizeIterator<Item = &'a String>,
    pick: usize,
    other: &str,
) -> String {
    let n = names.len();
    names
        .nth(pick % (n + 1))
        .map_or_else(|| other.to_owned(), Clone::clone)
}

/// Why a new name is refused: it breaks the rules, it is alike to a name of its own kind (the one
/// given), or it is exactly the other kind's.
#[derive(Debug)]
enum Refused {
    Bad,
    Alike(String),
    OtherKind,
}

impl Refused {
    /// Whether the engine refused `name` as a line's for this reason.
    fn is(&self, e: &NameError, name: &str) -> bool {
        match (self, e) {
            (Refused::Bad, NameError::Bad { name: n, .. })
            | (Refused::OtherKind, NameError::Tag { name: n }) => n == name,
            (Refused::Alike(want), NameError::Taken { name: n, taken }) => {
                n == name && taken == want
            }
            _ => false,
        }
    }

    /// Whether the engine refused `name` as a tag's for this reason.
    fn is_tag(&self, e: &TagError, name: &str) -> bool {
        match (self, e) {
            (Refused::Bad, TagError::BadName { name: n, .. })
            | (Refused::OtherKind, TagError::Line { name: n }) => n == name,
            (Refused::Alike(want), TagError::Alike { name: n, taken }) => {
                n == name && taken == want
            }
            _ => false,
        }
    }
}

/// Why `name` can't be a new line's, `renaming` aside. Every name in `NAMES` keeps the line-name
/// rules but for the leading `-`.
fn line_name_refused(m: &Model, name: &str, renaming: Option<&str>) -> Option<Refused> {
    if name.starts_with('-') {
        return Some(Refused::Bad);
    }
    let alike = (m.state.lines.keys())
        .find(|l| l.to_lowercase() == name.to_lowercase() && Some(l.as_str()) != renaming);
    if let Some(taken) = alike {
        return Some(Refused::Alike(taken.clone()));
    }
    m.state
        .tags
        .contains_key(name)
        .then_some(Refused::OtherKind)
}

/// Why `name` can't be a new tag's.
fn tag_name_refused(m: &Model, name: &str) -> Option<Refused> {
    if name.starts_with('-') {
        return Some(Refused::Bad);
    }
    let alike = (m.state.tags.keys()).find(|t| t.to_lowercase() == name.to_lowercase());
    if let Some(taken) = alike {
        return Some(Refused::Alike(taken.clone()));
    }
    m.state
        .lines
        .contains_key(name)
        .then_some(Refused::OtherKind)
}

/// `line new`: a line at the save the project is on, which the project goes on.
fn make_line(repo: &mut Repository, m: &mut Model, name: &str) {
    let done = lines::new_line(repo, name, &who(), &mut |_| {});
    if let Some(want) = line_name_refused(m, name, None) {
        let refused = matches!(&done, Err(LineError::Name(e)) if want.is(e, name));
        assert!(refused, "{done:?}, not {want:?}");
        settle(repo, m, Vec::new());
        return;
    }
    let made = done.unwrap();
    assert_eq!(
        (made.line.as_str(), made.save),
        (name, m.saves[m.state.on].id)
    );
    let (was, held) = (m.state.clone(), m.folder.clone());
    m.state.lines.insert(name.to_owned(), m.state.on);
    m.state.line = Some(name.to_owned());
    stepped(repo, m, "line new", was, held);
}

/// `line rename`: line `old`, or the current one, takes the name `new`; a project on it stays on
/// it. The name it has already changes nothing.
fn rename(repo: &mut Repository, m: &mut Model, old: Option<String>, new: &str) {
    let done = lines::rename_line(repo, old.as_deref(), new, &who(), &mut |_| {});
    let Some(old) = old.or_else(|| m.state.line.clone()) else {
        assert!(matches!(done, Err(LineError::NotOnALine)), "{done:?}");
        settle(repo, m, Vec::new());
        return;
    };
    let Some(save) = m.state.lines.get(&old).copied() else {
        assert!(
            matches!(&done, Err(LineError::NoSuchLine(n)) if *n == old),
            "{done:?}"
        );
        settle(repo, m, Vec::new());
        return;
    };
    if old != new
        && let Some(want) = line_name_refused(m, new, Some(&old))
    {
        let refused = matches!(&done, Err(LineError::Name(e)) if want.is(e, new));
        assert!(refused, "{done:?}, not {want:?}");
        settle(repo, m, Vec::new());
        return;
    }
    let done = done.unwrap();
    let told = (done.from.as_str(), done.to.as_str(), done.renamed);
    assert_eq!(told, (old.as_str(), new, old != new));
    let (was, held) = (m.state.clone(), m.folder.clone());
    m.state.lines.remove(&old);
    m.state.lines.insert(new.to_owned(), save);
    if m.state.line.as_ref() == Some(&old) {
        m.state.line = Some(new.to_owned());
    }
    stepped(repo, m, "line rename", was, held);
}

/// How many saves only line `name` holds: those in its history, through every parent, that no
/// other line's newest save and no tag's save has in its own.
fn only_here(m: &Model, name: &str) -> u64 {
    let held: HashSet<Id> = (m.state.lines.iter())
        .filter(|(line, _)| *line != name)
        .map(|(_, &tip)| tip)
        .chain(m.state.tags.values().copied())
        .flat_map(|tip| history(m, &[tip]))
        .map(|s| m.saves[s].id)
        .collect();
    let mine: HashSet<Id> = history(m, &[m.state.lines[name]])
        .into_iter()
        .map(|s| m.saves[s].id)
        .filter(|id| !held.contains(id))
        .collect();
    mine.len() as u64
}

/// `line delete`: refused for the line the project is on, and asked first when the line alone
/// holds saves; `yes` is the answer.
fn delete_line(repo: &mut Repository, m: &mut Model, name: &str, yes: bool) {
    let mut asked = Vec::new();
    let done = lines::delete_line(repo, name, &who(), &mut |_| {}, &mut |line, n| {
        asked.push((line.to_owned(), n));
        yes
    });
    let refused = |m: &mut Model, asked: Vec<(String, u64)>| {
        assert!(asked.is_empty(), "asked {asked:?}");
        settle(repo, m, Vec::new());
    };
    let Some(&tip) = m.state.lines.get(name) else {
        assert!(
            matches!(&done, Err(LineError::NoSuchLine(n)) if n == name),
            "{done:?}"
        );
        return refused(m, asked);
    };
    if m.state.line.as_deref() == Some(name) {
        assert!(
            matches!(&done, Err(LineError::OnIt(n)) if n == name),
            "{done:?}"
        );
        return refused(m, asked);
    }
    let only = only_here(m, name);
    let want: Vec<(String, u64)> = (only > 0)
        .then(|| (name.to_owned(), only))
        .into_iter()
        .collect();
    assert_eq!(asked, want, "what line delete asked");
    if only > 0 && !yes {
        let kept = matches!(&done, Err(LineError::Kept { line, only_here })
            if line == name && *only_here == only);
        assert!(kept, "{done:?}");
        settle(repo, m, Vec::new());
        return;
    }
    let done = done.unwrap();
    assert_eq!((done.save, done.only_here), (m.saves[tip].id, only));
    let (was, held) = (m.state.clone(), m.folder.clone());
    m.state.lines.remove(name);
    stepped(repo, m, "line delete", was, held);
}

/// What `tag` or `delete_tag` asked: the tag's name, the save it is at, and where it would move.
fn asked_of(ask: &Ask<'_>) -> (String, Id, Option<Id>) {
    match *ask {
        Ask::Move { name, from, to } => (name.to_owned(), from, Some(to)),
        Ask::Delete { name, save } => (name.to_owned(), save, None),
    }
}

/// `tag`: tags the save the project is on. A tag there already changes nothing; one elsewhere
/// moves only if `move_it` answers yes.
fn tag_here(repo: &mut Repository, m: &mut Model, name: &str, move_it: bool) {
    let mut asked = Vec::new();
    let done = tags::tag(repo, name, None, None, &who(), &mut |_| {}, &mut |a| {
        asked.push(asked_of(a));
        move_it
    });
    let (on, here) = (m.state.on, m.saves[m.state.on].id);
    let at = m.state.tags.get(name).map(|&s| m.saves[s].id);
    let want_asked: Vec<_> = at
        .filter(|&from| from != here)
        .map(|from| (name.to_owned(), from, Some(here)))
        .into_iter()
        .collect();
    assert_eq!(asked, want_asked, "what tag asked");
    let refusal = match at {
        Some(save) if save == here => {
            let already = matches!(&done, Ok((Tagged::Already { save }, _)) if *save == here);
            assert!(already, "{done:?}");
            true
        }
        Some(from) if !move_it => {
            let kept = matches!(&done, Err(TagError::NotMoved { name: n, from: f, to })
                if n == name && *f == from && *to == here);
            assert!(kept, "{done:?}");
            true
        }
        Some(from) => {
            let moved = matches!(&done, Ok((Tagged::Moved { from: f, to }, _))
                if *f == from && *to == here);
            assert!(moved, "{done:?}");
            false
        }
        None => match tag_name_refused(m, name) {
            Some(want) => {
                let refused = matches!(&done, Err(e) if want.is_tag(e, name));
                assert!(refused, "{done:?}, not {want:?}");
                true
            }
            None => {
                let made = matches!(&done, Ok((Tagged::Made { save }, _)) if *save == here);
                assert!(made, "{done:?}");
                false
            }
        },
    };
    if refusal {
        settle(repo, m, Vec::new());
        return;
    }
    let (was, held) = (m.state.clone(), m.folder.clone());
    m.state.tags.insert(name.to_owned(), on);
    stepped(repo, m, "tag", was, held);
}

/// `tag --delete`, always asked first; `yes` is the answer.
fn delete_tag(repo: &mut Repository, m: &mut Model, name: &str, yes: bool) {
    let mut asked = Vec::new();
    let done = tags::delete_tag(repo, name, &who(), &mut |_| {}, &mut |a| {
        asked.push(asked_of(a));
        yes
    });
    let Some(save) = m.state.tags.get(name).map(|&s| m.saves[s].id) else {
        assert!(
            matches!(&done, Err(TagError::NoSuchTag(n)) if n == name),
            "{done:?}"
        );
        assert!(asked.is_empty(), "asked {asked:?}");
        settle(repo, m, Vec::new());
        return;
    };
    assert_eq!(
        asked,
        [(name.to_owned(), save, None)],
        "what tag --delete asked"
    );
    if !yes {
        let kept = matches!(&done, Err(TagError::NotDeleted { name: n }) if n == name);
        assert!(kept, "{done:?}");
        settle(repo, m, Vec::new());
        return;
    }
    assert_eq!(done.unwrap().0, save);
    let (was, held) = (m.state.clone(), m.folder.clone());
    m.state.tags.remove(name);
    stepped(repo, m, "tag", was, held);
}

/// Throws every unsaved change away.
fn discard_all(repo: &mut Repository, m: &mut Model) {
    discard(repo, None, &who(), "discard", None, &mut |_| {}).unwrap();
    let saved = m.tip().clone();
    let (was, held) = (m.state.clone(), std::mem::replace(&mut m.folder, saved));
    stepped(repo, m, "discard", was, held);
}

/// `shelf`: sets every unsaved change aside, then puts the folder back to its save.
fn shelve(repo: &mut Repository, m: &mut Model) {
    let done = shelf(repo, None, &who(), None, &mut |_| {}).unwrap();
    if m.folder == *m.tip() {
        assert!(
            matches!(
                done,
                Shelved::Nothing {
                    changed_again: false,
                    ..
                }
            ),
            "{done:?}"
        );
        settle(repo, m, Vec::new());
        return;
    }
    let Shelved::Made(made) = done else {
        panic!("nothing shelved: {done:?}");
    };
    assert_eq!(made.files, changed_files(m.tip(), &m.folder).len());
    let label = free_label(&m.state.shelves);
    let was = m.state.clone();
    let mut made = std::iter::once((made.label, made.save));
    let (store, tree) = (repo.store(), m.folder.clone());
    let save = new_shelf(store, m, &mut made, &label, tree, "Shelved");
    m.state.shelves.insert(0, (label, save));
    let saved = m.tip().clone();
    let held = std::mem::replace(&mut m.folder, saved);
    stepped(repo, m, "shelf", was, held);
}

/// `unshelf`: brings back the newest shelf, or the one labelled `label`, as a switch from the
/// save it was made on.
fn bring_back(repo: &mut Repository, m: &mut Model, label: Option<&str>) {
    let done = unshelf(repo, label, &who(), None, &mut |_| {});
    let asked = match label {
        Some(label) => m.state.shelves.iter().position(|(l, _)| l == label),
        None => (!m.state.shelves.is_empty()).then_some(0),
    };
    let Some(k) = asked else {
        assert!(matches!(done, Err(SwitchError::NoShelf { .. })), "{done:?}");
        settle(repo, m, Vec::new());
        return;
    };
    let (name, save) = m.state.shelves[k].clone();
    let base = m.saves[save].parent.unwrap();
    match switch(&m.folder, &m.saves[base].tree, &m.saves[save].tree) {
        Err(_) => {
            assert!(
                matches!(done, Err(SwitchError::ShelfInTheWay(_))),
                "{done:?}"
            );
            settle(repo, m, Vec::new());
        }
        Ok(folder) => {
            let done = done.unwrap();
            assert_eq!((done.label, done.save), (name, m.saves[save].id));
            let was = m.state.clone();
            m.state.shelves.remove(k);
            let held = std::mem::replace(&mut m.folder, folder);
            stepped(repo, m, "unshelf", was, held);
        }
    }
}

/// The smallest number from 1 that no shelf has.
fn free_label(shelves: &[(String, usize)]) -> String {
    (1usize..)
        .map(|n| n.to_string())
        .find(|n| !shelves.iter().any(|(l, _)| l == n))
        .unwrap()
}

/// Adds the shelf the engine says it made next, on the save the project is on, once it holds
/// `tree`. Gives its index.
fn new_shelf(
    store: &Store,
    m: &mut Model,
    made: &mut dyn Iterator<Item = (String, Id)>,
    label: &str,
    tree: Tree,
    message: &str,
) -> usize {
    let (got, id) = made.next().expect("a shelf the engine didn't make");
    assert_eq!(got, label, "the new shelf's number");
    assert_eq!(
        saved_tree(store, id).standing(),
        tree,
        "what shelf {label} holds"
    );
    m.saves.push(Saved {
        id,
        stored: tree.clone(),
        tree,
        parent: Some(m.state.on),
        others: Vec::new(),
        change: m.saves.len(),
        message: message.to_owned(),
        shelf: true,
    });
    m.saves.len() - 1
}

/// Takes off the shelves a step added and puts back those it removed, compared by save, for an
/// undo; the other way round for a redo. One put back goes first, keeping its number unless
/// another shelf has it.
fn put_shelves_back(
    shelves: &mut Vec<(String, usize)>,
    before: &[(String, usize)],
    after: &[(String, usize)],
    again: bool,
) {
    let missing = |from: &[(String, usize)], there: &[(String, usize)]| -> Vec<(String, usize)> {
        let gone = |save: &usize| !there.iter().any(|(_, s)| s == save);
        from.iter().filter(|(_, s)| gone(s)).cloned().collect()
    };
    let (added, removed) = (missing(after, before), missing(before, after));
    let (off, back) = if again {
        (removed, added)
    } else {
        (added, removed)
    };
    shelves.retain(|(_, s)| !off.iter().any(|(_, o)| o == s));
    for (at, (label, save)) in back.into_iter().enumerate() {
        let label = if shelves.iter().any(|(l, _)| *l == label) {
            free_label(shelves)
        } else {
            label
        };
        shelves.insert(at, (label, save));
    }
}

/// What the model makes of the next step undo takes, or redo brings back.
enum Next {
    /// Taken: its entry records this.
    Took(Made),
    /// None left.
    Nothing,
    /// Unsaved work in the way stopped it, at these paths.
    Stopped(BTreeSet<String>),
}

/// Takes the next step back, or forward with `again`: the state from before it, or its own,
/// with the shelves it added or removed taken off or put back, and the folder switched back from
/// what it wrote to what it held, or the other way.
fn take(
    store: &Store,
    m: &mut Model,
    (again, shelve): (bool, bool),
    shelved: &mut dyn Iterator<Item = (String, Id)>,
) -> Next {
    let next = if again {
        m.undone.last()
    } else {
        m.done.last().filter(|s| s.before.is_some())
    };
    let Some(step) = next.cloned() else {
        return Next::Nothing;
    };
    let before = step.before.clone().unwrap();
    let source = if again { &step.after } else { &before };
    let mut state = State {
        shelves: m.state.shelves.clone(),
        ..source.clone()
    };
    put_shelves_back(
        &mut state.shelves,
        &before.shelves,
        &step.after.shelves,
        again,
    );
    let (held, wrote) = &step.folder;
    let (from, into) = if again { (held, wrote) } else { (wrote, held) };
    if held != wrote {
        let (folder, shelf) = match switch_or_shelve(&m.folder, from, into, m.tip(), shelve) {
            Ok(done) => done,
            Err(paths) => return Next::Stopped(paths),
        };
        if let Some(tree) = shelf {
            let label = free_label(&state.shelves);
            let message = format!("Set aside by {}", if again { "redo" } else { "undo" });
            let save = new_shelf(store, m, shelved, &label, tree, &message);
            state.shelves.insert(0, (label, save));
        }
        m.folder = folder;
    }
    m.state = state;
    let entry = step.entry;
    let kept = !m.saves[step.after.on].stored.kept.is_empty();
    let made_here = step.made_conflicts;

    match (step.command, kept, again) {
        ("combine", true, false) if made_here => met("undo over a combine that kept conflicts"),
        ("combine", true, true) if made_here => met("redo over a combine that kept conflicts"),
        ("save", true, false) => met("undo over a save that kept a conflict"),
        ("save", true, true) => met("redo over a save that kept a conflict"),
        ("resolve", _, false) => met("undo over a resolve"),
        ("resolve", _, true) => met("redo over a resolve"),
        _ => {}
    }

    if again {
        m.undone.pop();
        m.done.push(step);
        Next::Took(Made::Redo(entry))
    } else {
        m.done.pop();
        m.undone.push(step);
        Next::Took(Made::Undo(entry))
    }
}

/// The steps an undo, a redo or an `undo --to` took, the shelves it made and why it stopped.
fn outcome(
    done: Result<UndoOutcome, UndoError>,
) -> (usize, Vec<(String, Id)>, Option<SwitchError>) {
    match done {
        Ok(done) => {
            assert!(!done.finished && !done.head_lost, "{done:?}");
            (done.taken.len(), done.shelved, None)
        }
        Err(e) => (e.taken.len(), e.shelved, Some(*e.error)),
    }
}

/// Plays up to `n` steps of undo, or of redo with `again`, on the model, and holds `done`, what
/// the engine did, to them; `to` is the entry `undo --to` was given.
fn take_steps(
    repo: &Repository,
    m: &mut Model,
    (n, to): (usize, Option<&str>),
    (again, shelve): (bool, bool),
    done: Result<UndoOutcome, UndoError>,
) {
    let (taken, shelved, error) = outcome(done);
    let mut shelved = shelved.into_iter();
    let (mut made, mut stopped) = (Vec::new(), None);
    for _ in 0..n {
        match take(repo.store(), m, (again, shelve), &mut shelved) {
            Next::Took(entry) => made.push((entry, m.state.clone())),
            Next::Nothing => break,
            Next::Stopped(paths) => {
                stopped = Some(paths);
                break;
            }
        }
    }
    same_stop(stopped, error, (again, shelve), to);
    assert_eq!(taken, made.len(), "steps taken");
    assert!(shelved.next().is_none(), "a shelf the model didn't make");
    settle(repo, m, made);
}

/// Holds the engine's error to the model's stop: none, or unsaved work in the way of a step at
/// exactly the model's paths, naming the entry `undo --to` was given, if any.
fn same_stop(
    stopped: Option<BTreeSet<String>>,
    error: Option<SwitchError>,
    (again, shelve): (bool, bool),
    to: Option<&str>,
) {
    match (stopped, error) {
        (None, None) => {}
        (
            Some(want),
            Some(SwitchError::InTheWayOfStep {
                paths,
                redo,
                shelve: shelved,
                to: given,
            }),
        ) => {
            let want: Vec<String> = want.into_iter().collect();
            let got = (paths, redo, shelved, given.as_deref());
            assert_eq!(got, (want, again, shelve, to));
        }
        (stopped, error) => panic!("the model stops at {stopped:?}, the engine with {error:?}"),
    }
}

/// `undo --to` the entry `pick` chooses: the current one changes nothing, one undo or redo
/// reaches is reached by them, and any other by a jump.
fn go_back_to(repo: &mut Repository, m: &mut Model, pick: usize, shelve: bool) {
    let who = who();
    let (id, target) = m.entries[pick % m.entries.len()].clone();
    let req = StepRequest {
        n: 1,
        shelve,
        who: &who,
        git_excludes: None,
    };
    let text = id.to_hex();
    let done = undo_to(repo, &text, &req, &mut |_| {});
    let to = Some(text.as_str());
    if id == m.head() {
        let (taken, shelved, error) = outcome(done);
        assert!(
            taken == 0 && shelved.is_empty() && error.is_none(),
            "{error:?}"
        );
        settle(repo, m, Vec::new());
    } else if let Some(i) = m.undone.iter().position(|s| s.entry == id) {
        let n = m.undone.len() - i;
        take_steps(repo, m, (n, to), (true, shelve), done);
    } else if let Some(i) = m.done.iter().position(|s| s.entry == id) {
        let n = m.done.len() - 1 - i;
        take_steps(repo, m, (n, to), (false, shelve), done);
    } else {
        jump(repo, m, (target, &text), shelve, done);
    }
}

/// `undo --to` an entry neither undo nor redo reaches, given as `text`: one step to its lines and
/// position, the shelves kept, switching the folder to its save as `goto` does. None if that is
/// where things are.
fn jump(
    repo: &Repository,
    m: &mut Model,
    (target, text): (State, &str),
    shelve: bool,
    done: Result<UndoOutcome, UndoError>,
) {
    let (taken, shelved, error) = outcome(done);
    let mut shelved = shelved.into_iter();
    let was = m.state.clone();
    let mut state = State {
        shelves: was.shelves.clone(),
        ..target
    };
    let there = recorded(m, &state) == recorded(m, &was);
    let into = m.saves[state.on].tree.clone();
    match switch_or_shelve(&m.folder, m.tip(), &into, m.tip(), shelve) {
        _ if there => {
            assert!(taken == 0 && error.is_none(), "{error:?}");
            settle(repo, m, Vec::new());
        }
        Err(paths) => {
            assert_eq!(taken, 0);
            same_stop(Some(paths), error, (false, shelve), Some(text));
            settle(repo, m, Vec::new());
        }
        Ok((folder, shelf)) => {
            assert_eq!(taken, 1);
            same_stop(None, error, (false, shelve), Some(text));
            if let Some(tree) = shelf {
                let label = free_label(&state.shelves);
                let save = new_shelf(
                    repo.store(),
                    m,
                    &mut shelved,
                    &label,
                    tree,
                    "Set aside by undo",
                );
                state.shelves.insert(0, (label, save));
            }
            let held = std::mem::replace(&mut m.folder, folder);
            m.state = state;
            stepped(repo, m, "undo", was, held);
        }
    }
    assert!(shelved.next().is_none(), "a shelf the model didn't make");
}

/// The lines of a text: each ends after a line feed, the last perhaps without one.
fn lines_of(bytes: &[u8]) -> Vec<&[u8]> {
    bytes.split_inclusive(|&c| c == b'\n').collect()
}

/// A line no file has held.
fn fresh_line(m: &mut Model) -> Vec<u8> {
    m.fresh += 1;
    format!("line {}\n", m.fresh).into_bytes()
}

/// Writes `path` edited as text: a new line put in at `at`, the line there taken out, or
/// replaced, as `edit` picks; three new lines when it holds none.
fn edit_text(m: &mut Model, root: &Path, path: &str, at: usize, edit: u8) -> Option<String> {
    let old = m.folder.files.get(path).map(|f| f.bytes.clone());
    let mut lines: Vec<Vec<u8>> = lines_of(old.as_deref().unwrap_or_default())
        .into_iter()
        .map(<[u8]>::to_vec)
        .collect();
    let n = lines.len();
    if n == 0 {
        lines = (0..3).map(|_| fresh_line(m)).collect();
    } else {
        match edit % 3 {
            0 => {
                // A last line without a line feed stays last.
                let open = !lines[n - 1].ends_with(b"\n");
                let at = at % (n + 1 - usize::from(open));
                lines.insert(at, fresh_line(m));
            }
            1 => drop(lines.remove(at % n)),
            _ => lines[at % n] = fresh_line(m),
        }
    }
    put(m, root, path, lines.concat(), When::Now)
}

/// What a combine is given: a line, or a save the model knows.
#[derive(Debug, Clone)]
enum Aim {
    Line(String),
    Save(usize),
}

/// What a combine may be given: the lines and saves outside the history of the save the project
/// is on, which a combine has work to bring from, then every line and save, so that a pick often
/// finds work to combine.
fn aims(m: &Model) -> Vec<Aim> {
    let mine = history(m, &[m.state.on]);
    let lines = m
        .state
        .lines
        .iter()
        .map(|(l, &s)| (Aim::Line(l.clone()), s));
    let saves = (0..m.saves.len())
        .filter(|&i| !m.saves[i].shelf)
        .map(|i| (Aim::Save(i), i));
    let all: Vec<(Aim, usize)> = lines.chain(saves).collect();
    let apart = all.iter().filter(|(_, s)| !mine.contains(s)).cloned();
    apart.chain(all.clone()).map(|(a, _)| a).collect()
}

/// More common starting points than this at one level stop a combine, and so do more inner
/// combines than `MOST_INNER` in one.
const MOST_STARTS: usize = 16;
const MOST_INNER: usize = 64;

/// A conflict a combine made: its place, the row it names it with (its lines called `O` and
/// `T`), and how it stands in the folder, `None` where it isn't kept.
type MadeRow = (String, String, Option<Stands>);

/// Why the model's combine stops before it switches anything.
#[derive(Debug)]
enum Stopped {
    Unrelated,
    Tangled,
    /// Asked for a clean line: the conflicts it would make, and how many the result would hold
    /// already, on a forward the target's.
    Unclean {
        made: Vec<MadeRow>,
        held: u64,
        forward: bool,
    },
    /// Both lines changed `path`, and `on` holds a conflict at `at`, the path or below it.
    Held {
        path: String,
        at: String,
        on: Held,
    },
}

impl Stopped {
    fn holds(&self, done: &Result<combine::CombineOutcome, CombineError>) {
        match (self, done) {
            (Stopped::Unrelated, Err(CombineError::Unrelated { .. }))
            | (Stopped::Tangled, Err(CombineError::Tangled { .. })) => {}
            (
                Stopped::Unclean {
                    made,
                    held,
                    forward,
                },
                Err(CombineError::Conflicts {
                    conflicts,
                    more,
                    why,
                    held: got_held,
                    forward: got_forward,
                    ..
                }),
            ) => {
                let got: Vec<MadeRow> = conflicts.iter().map(row_of).collect();
                assert_eq!(
                    (&got, *more, *why, *got_held, *got_forward),
                    (made, 0, NotKept::OnlyIfClean, *held, *forward),
                    "what a combine asked for a clean line met"
                );
                for (_, row, _) in made {
                    met(clash_kind(row));
                }
                if *forward {
                    met("--only-if-clean: a forward onto a save in conflict");
                } else if !made.is_empty() {
                    met("--only-if-clean: a conflict it would make");
                }
                if *held > 0 && !*forward {
                    met("--only-if-clean: a conflict carried from a line");
                }
            }
            (
                Stopped::Held { path, at, on },
                Err(CombineError::HeldConflict {
                    path: got_path,
                    at: got_at,
                    on: got_on,
                    ..
                }),
            ) => {
                assert_eq!((got_path, got_at, got_on), (path, at, on));
                met("stopped at a conflict a line holds");
            }
            _ => panic!("the model's combine stops with {self:?}, the engine's with {done:?}"),
        }
    }
}

/// A conflict the engine named, as the model names one.
fn row_of(c: &combine::Conflict) -> MadeRow {
    (c.path.clone(), c.describe("O", "T"), c.stands)
}

/// Which kind of clash a row tells of, by its words (terminal.md).
fn clash_kind(row: &str) -> &'static str {
    let what = row.split_once(": ").map_or(row, |(_, w)| w);
    if what.contains(", and to ") {
        "moved apart"
    } else if what.starts_with("moved here from") && what.contains(", removed on ") {
        "moved, removed"
    } else if what.contains("has something else") {
        "moved onto"
    } else if what.ends_with("can't tell what it held where they parted") {
        "start unknown"
    } else if what.contains(", removed on ") {
        "changed, removed"
    } else if what == "added on both lines, differently" {
        "added on both"
    } else if what == "changed on both lines, in the same place" {
        "changed on both, in the same place"
    } else if what.ends_with("differ only in line endings") {
        "changed on both, line endings"
    } else if what.contains("isn't text") {
        "changed on both, not text"
    } else {
        "changed on both, other"
    }
}

/// `combine`: nothing when the target is in the line's history, the line moved forward when the
/// line's save is in the target's, else a combined save holding the conflicts it keeps; the
/// folder switched to the result as `goto` switches it. `keep` says whether conflicts are kept.
fn combine_into(
    repo: &mut Repository,
    m: &mut Model,
    aim: &Aim,
    message: Option<&str>,
    keep: Keep,
) {
    let (target, with, raw) = match aim {
        Aim::Line(name) => (
            Target::Line(name.clone()),
            m.state.lines[name],
            name.clone(),
        ),
        Aim::Save(i) => (Target::Save(m.saves[*i].id), *i, m.saves[*i].id.short_hex()),
    };
    let named = Named {
        raw: raw.clone(),
        shown: raw.clone(),
    };
    let done = combine::combine(
        repo,
        &target,
        &named,
        message,
        &who(),
        None,
        keep,
        &mut |_| {},
    );
    let Some(line) = m.state.line.clone() else {
        assert!(matches!(done, Err(CombineError::NotOnALine)), "{done:?}");
        settle(repo, m, Vec::new());
        return;
    };
    let from = m.state.on;
    if history(m, &[from]).contains(&with) {
        let done = done.unwrap();
        assert_eq!(
            (done.result, done.save),
            (Combined::Nothing, m.saves[from].id)
        );
        settle(repo, m, Vec::new());
        return;
    }
    let (into, made) = if history(m, &[with]).contains(&from) {
        let held = m.saves[with].stored.kept.len() as u64;
        if keep == Keep::OnlyIfClean && held > 0 {
            let made = Vec::new();
            let forward = true;
            Stopped::Unclean {
                made,
                held,
                forward,
            }
            .holds(&done);
            settle(repo, m, Vec::new());
            return;
        }
        (m.saves[with].stored.clone(), None)
    } else {
        let sides = (keep == Keep::Conflicts).then(|| sides(repo, m, &line, from, with, &raw));
        match combined(m, from, with, sides.as_ref()) {
            Ok(c) => (c.tree.clone(), Some(c)),
            Err(stopped) => {
                stopped.holds(&done);
                settle(repo, m, Vec::new());
                return;
            }
        }
    };
    let standing = into.standing();
    let Ok(folder) = switch(&m.folder, m.tip(), &standing) else {
        let stopped = matches!(done, Err(CombineError::Collisions { .. }));
        assert!(stopped, "{done:?}");
        settle(repo, m, Vec::new());
        return;
    };
    let done = done.unwrap();
    let to = match made {
        None => {
            assert_eq!(
                (done.result, done.save),
                (Combined::Forward, m.saves[with].id)
            );
            with
        }
        Some(c) => {
            let Combined::Made {
                starts,
                merged_files,
                conflicts,
                more,
                conflict_count,
            } = &done.result
            else {
                panic!(
                    "the model made a combined save, the engine {:?}",
                    done.result
                );
            };
            let want_starts: Vec<Id> = c.starts.iter().map(|&s| m.saves[s].id).collect();
            assert_eq!((starts, merged_files), (&want_starts, &c.merged));
            let got: Vec<MadeRow> = conflicts.iter().map(row_of).collect();
            assert_eq!(
                (&got, *more),
                (&c.made, 0),
                "the conflicts the combine made"
            );
            assert_eq!(
                *conflict_count,
                into.kept.len() as u64,
                "places in conflict"
            );
            for (_, row, _) in &c.made {
                met(clash_kind(row));
            }
            m.made_conflicts = !c.made.is_empty();
            if c.unsure {
                met("starting points that disagree");
            }
            assert_eq!(
                saved_tree(repo.store(), done.save),
                into,
                "the combined save"
            );
            let message =
                message.map_or_else(|| format!("Combine {raw} into {line}"), str::to_owned);
            m.saves.push(Saved {
                id: done.save,
                tree: standing,
                stored: into,
                parent: Some(from),
                others: vec![with],
                change: m.saves.len(),
                message,
                shelf: false,
            });
            m.saves.len() - 1
        }
    };
    let was = m.state.clone();
    m.state.lines.insert(line, to);
    m.state.on = to;
    let held = std::mem::replace(&mut m.folder, folder);
    stepped(repo, m, "combine", was, held);
}

/// The first 8 letters of a save's change ID, as a label shows it.
fn letters(repo: &Repository, save: Id) -> String {
    let change = repo.store().save(&save, false).unwrap().change_id;
    change.to_letters()[..8].to_owned()
}

/// What the labels of a combine's marked texts are made of: the line and the target as the
/// person named them, each with its save's letters, and the one save where they parted.
struct Sides {
    ours: String,
    theirs: String,
    /// `None` where they parted at several saves.
    parted: Option<String>,
}

/// What a marked text's middle label says of where the lines parted at one place.
#[derive(Clone, Copy)]
enum Middle {
    Parted,
    Nothing,
    Unknown,
}

impl Sides {
    fn labels(&self, middle: Middle) -> [String; 3] {
        let parted = "where the lines parted";
        let base = match (middle, &self.parted) {
            (Middle::Parted, Some(save)) => format!("{parted} ({save})"),
            (Middle::Parted, None) => format!("{parted} (several saves)"),
            (Middle::Nothing, Some(save)) => format!("{parted} ({save}), which held nothing here"),
            (Middle::Nothing, None) => format!("{parted}, which held nothing here"),
            (Middle::Unknown, _) => format!("{parted}, which Opalro can't tell here"),
        };
        [self.ours.clone(), base, self.theirs.clone()]
    }
}

fn sides(repo: &Repository, m: &Model, line: &str, from: usize, with: usize, raw: &str) -> Sides {
    let starts = common_starts(m, &[from], &[with]);
    let parted = match starts[..] {
        [one] => Some(letters(repo, m.saves[one].id)),
        _ => None,
    };
    Sides {
        ours: format!("{line} ({})", letters(repo, m.saves[from].id)),
        theirs: format!("{raw} ({})", letters(repo, m.saves[with].id)),
        parted,
    }
}

/// Every save in the history of `tips`, through every parent, the tips included.
fn history(m: &Model, tips: &[usize]) -> BTreeSet<usize> {
    let mut seen = BTreeSet::new();
    let mut todo = tips.to_vec();
    while let Some(s) = todo.pop() {
        if seen.insert(s) {
            todo.extend(m.saves[s].parent.iter().chain(&m.saves[s].others));
        }
    }
    seen
}

/// The common starting points of two sets of saves, in ID order: the saves both have in their
/// history that aren't in the history of another such save.
fn common_starts(m: &Model, a: &[usize], b: &[usize]) -> Vec<usize> {
    let both: BTreeSet<usize> = history(m, a)
        .intersection(&history(m, b))
        .copied()
        .collect();
    let under: BTreeSet<usize> = (both.iter())
        .flat_map(|&s| history(m, &[s]).into_iter().filter(move |&u| u != s))
        .collect();
    let mut starts: Vec<usize> = both.difference(&under).copied().collect();
    starts.sort_by_key(|&s| m.saves[s].id);
    starts
}

/// One version a merge reads, as stored, and its save, `None` when it is made up; for a made-up
/// one, the places where its starting points disagreed (`""` for everywhere), so nothing is
/// known there or below.
#[derive(Debug, Clone)]
struct Ver {
    tree: Tree,
    save: Option<usize>,
    unsure: BTreeSet<String>,
}

fn ver(m: &Model, save: usize) -> Ver {
    Ver {
        tree: m.saves[save].stored.clone(),
        save: Some(save),
        unsure: BTreeSet::new(),
    }
}

/// Whether `unsure` says nothing is known at `path`: it, or a folder holding it, is listed.
fn unsure_at(unsure: &BTreeSet<String>, path: &str) -> bool {
    unsure.iter().any(|u| u.is_empty() || within(path, u))
}

/// Whether something listed in `unsure` is inside the folder `path`.
fn unsure_inside(unsure: &BTreeSet<String>, path: &str) -> bool {
    unsure.iter().any(|u| u != path && within(u, path))
}

/// A combined save worked out: its tree as stored, the common starting points, the files merged
/// line by line, the conflicts it made, and whether its starting points disagreed anywhere.
struct Combine {
    tree: Tree,
    starts: Vec<usize>,
    merged: Vec<String>,
    made: Vec<MadeRow>,
    unsure: bool,
}

/// The combined save, or why there is none; `sides` only where conflicts are kept.
fn combined(
    m: &Model,
    from: usize,
    with: usize,
    sides: Option<&Sides>,
) -> Result<Combine, Stopped> {
    let starts = common_starts(m, &[from], &[with]);
    if starts.len() > MOST_STARTS {
        return Err(Stopped::Tangled);
    }
    // An amend of the first save makes a second first save, which the line may not share.
    if starts.is_empty() {
        return Err(Stopped::Unrelated);
    }
    let mut left = MOST_INNER;
    let start = start_from(m, &starts, &mut left)?;
    let done = merge(m, &start, &ver(m, from), &ver(m, with), sides, false)?;
    // Asked for a clean line, the conflicts it would make stop it, and so do those it carries.
    let held = done.tree.kept.len() as u64;
    if sides.is_none() && (!done.made.is_empty() || held > 0) {
        let forward = false;
        return Err(Stopped::Unclean {
            made: done.made,
            held,
            forward,
        });
    }
    Ok(Combine {
        tree: done.tree,
        starts,
        merged: done.merged,
        made: done.made,
        unsure: !start.unsure.is_empty(),
    })
}

/// The version to merge from: one starting point is itself; several are combined first, two at a
/// time in ID order, each pair from its own starting points, while `left` inner combines remain.
fn start_from(m: &Model, starts: &[usize], left: &mut usize) -> Result<Ver, Stopped> {
    // Starting points that share no save, after an amend of the first save, merge from a start
    // that knows nothing.
    let Some((&first, rest)) = starts.split_first() else {
        let tree = Tree::default();
        let unsure = BTreeSet::from([String::new()]);
        return Ok(Ver {
            tree,
            save: None,
            unsure,
        });
    };
    let (mut acc, mut tips) = (ver(m, first), vec![first]);
    for &next in rest {
        *left = left.checked_sub(1).ok_or(Stopped::Tangled)?;
        let under = common_starts(m, &tips, &[next]);
        if under.len() > MOST_STARTS {
            return Err(Stopped::Tangled);
        }
        let start = start_from(m, &under, left)?;
        // Where the two clash, the start of the next merge holds nothing and is unsure.
        // What the starting points so far were unsure of stays so, whatever the next one holds.
        let done = merge(m, &start, &acc, &ver(m, next), None, true)?;
        let mut tree = done.tree;
        let mut unsure = acc.unsure;
        unsure.extend(done.unsure);
        for c in &unsure {
            tree.remove(c);
        }
        acc = Ver {
            tree,
            save: None,
            unsure,
        };
        tips.push(next);
    }
    Ok(acc)
}

/// Files that left one path and arrived at another between two trees with the same non-empty
/// contents, no other file that left and none that arrived holding them; sorted by where they
/// went. A place in conflict holds something, and is no file that moves.
fn same_contents(before: &Tree, after: &Tree) -> Vec<(String, String)> {
    let gone = |a: &Tree, b: &Tree| {
        let mut by: BTreeMap<Rc<[u8]>, Vec<String>> = BTreeMap::new();
        for (p, f) in &a.files {
            let held = b.files.contains_key(p) || b.folders.contains(p) || b.kept.contains_key(p);
            if !f.bytes.is_empty() && !held {
                by.entry(f.bytes.clone()).or_default().push(p.clone());
            }
        }
        by
    };
    let (left, came) = (gone(before, after), gone(after, before));
    let mut moves: Vec<(String, String)> = (left.iter())
        .filter_map(|(bytes, from)| match (&from[..], &came.get(bytes)?[..]) {
            ([from], [to]) => Some((from.clone(), to.clone())),
            _ => None,
        })
        .collect();
    moves.sort_by(|a, b| a.1.cmp(&b.1));
    moves
}

/// Where the files of the save `start` went by the time of `tip`, by the notes of the saves
/// along `tip`'s first parents back to it, taken one after another; none when that path doesn't
/// reach it. Each save's notes are what moved since its first parent.
fn noted(m: &Model, start: usize, tip: usize) -> BTreeMap<String, String> {
    let mut path = Vec::new();
    let mut at = tip;
    while at != start {
        path.push(at);
        match m.saves[at].parent {
            Some(p) => at = p,
            None => return BTreeMap::new(),
        }
    }
    // Which start file each place holds: a place a file left holds a new one from then on, and
    // one never left still holds its own, however it was edited, removed or made again.
    let mut from: BTreeMap<String, Option<String>> = BTreeMap::new();
    for &s in path.iter().rev() {
        let before = &m.saves[m.saves[s].parent.unwrap()].stored;
        for (left, came) in same_contents(before, &m.saves[s].stored) {
            let file = from
                .get(&left)
                .cloned()
                .unwrap_or_else(|| Some(left.clone()));
            from.insert(came, file);
            from.insert(left, None);
        }
    }
    from.into_iter()
        .filter_map(|(at, file)| file.filter(|f| *f != at).map(|f| (f, at)))
        .collect()
}

/// Where the start's files went on one side, and the places where the side's notes show a file
/// arriving that a file still holds at the side's end.
struct Went {
    went: BTreeMap<String, String>,
    /// Each with where the file that came was.
    arrived: BTreeMap<String, String>,
}

/// Whether `tree` holds nothing at `path`, and no file or conflict above it.
fn room(tree: &Tree, path: &str) -> bool {
    let mut up = path;
    while let Some((above, _)) = up.rsplit_once('/') {
        if tree.files.contains_key(above) || tree.kept.contains_key(above) {
            return false;
        }
        up = above;
    }
    at(tree, path) == At::Nothing
}

/// Where the files of `start` went on `side`: by its notes first, then by contents for the files
/// the notes don't account for, where the path left empty. A move counts only from a start file
/// to a place the start had room at, where the side holds a file.
fn followed(m: &Model, start: &Ver, side: &Ver) -> Went {
    let noted = match (start.save, side.save) {
        (Some(s), Some(t)) => noted(m, s, t),
        _ => BTreeMap::new(),
    };
    // A place the start, or a made-up side, is unsure of is never where a move began or ended.
    let sure = |p: &str| !unsure_at(&start.unsure, p) && !unsure_at(&side.unsure, p);
    let fits = |from: &str, to: &str| {
        sure(from)
            && sure(to)
            && start.tree.files.contains_key(from)
            && room(&start.tree, to)
            && side.tree.files.contains_key(to)
    };
    let mut went: BTreeMap<String, String> = (noted.iter())
        .filter(|(from, to)| fits(from, to))
        .map(|(from, to)| (from.clone(), to.clone()))
        .collect();
    let taken: BTreeSet<String> = went.values().cloned().collect();
    for (from, to) in same_contents(&start.tree, &side.tree) {
        if !went.contains_key(&from) && !taken.contains(&to) && fits(&from, &to) {
            went.insert(from, to);
        }
    }
    let arrived = (noted.into_iter())
        .filter(|(_, to)| side.tree.files.contains_key(to))
        .map(|(from, to)| (to, from))
        .collect();
    Went { went, arrived }
}

/// Moves the file at `from` to `to`.
fn relocate(tree: &mut Tree, from: &str, to: &str) {
    if let Some(f) = tree.files.remove(from) {
        tree.put(to, f);
    }
}

/// What a merge gave: the merged tree as stored, the files merged line by line, the conflicts it
/// made, and for an inner merge the places it couldn't settle, which it leaves empty.
struct Merge {
    tree: Tree,
    merged: Vec<String>,
    made: Vec<MadeRow>,
    unsure: BTreeSet<String>,
}

/// Merges `ours` and `theirs` from `start`, following moves. `sides` are the labels where
/// conflicts are kept; an `inner` merge, of starting points, keeps none and never stops.
fn merge(
    m: &Model,
    start: &Ver,
    ours: &Ver,
    theirs: &Ver,
    sides: Option<&Sides>,
    inner: bool,
) -> Result<Merge, Stopped> {
    let went = [followed(m, start, ours), followed(m, start, theirs)];
    let was = [&start.tree, &ours.tree, &theirs.tree];
    let mut trees = was.map(Tree::clone);
    let line = |k: usize| if k == 1 { "O" } else { "T" };
    let file_at = |k: usize, p: &str| was[k].files.get(p).cloned().map(Part::File);
    // Where a moved file arrived on something the other line put there: two things added.
    let onto_row = |at: &str, from: &str, k: usize| {
        let (mover, other) = (line(k), line(3 - k));
        format!("{at}: moved here from {from} on {mover}, where {other} has something else")
    };
    // The conflicts a move makes where it went, kept where the result holds a plain file there:
    // the start's file is the middle version.
    let mut landed: Vec<(String, String, [Option<Part>; 3])> = Vec::new();
    let mut onto: BTreeMap<String, String> = BTreeMap::new();
    // Each start file moved on one line or both, by who moved it where and what each line holds
    // at the old and the new places.
    let moved: BTreeSet<&String> = went[0].went.keys().chain(went[1].went.keys()).collect();
    for &from in &moved {
        let start_file = Some(Part::File(was[0].files[from].clone()));
        let to = [went[0].went.get(from), went[1].went.get(from)];
        match to {
            [Some(a), Some(b)] if a == b => relocate(&mut trees[0], from, a),
            [Some(a), Some(b)] => {
                // Two places: each holds what its line moved there, unless the other line holds
                // something there of its own. The merge doesn't see the file where it was.
                trees[0].files.remove(from);
                for (at, k, other_to) in [(a, 1, b), (b, 2, a)] {
                    if room(was[3 - k], at) {
                        let (mover, other) = (line(k), line(3 - k));
                        let row = format!(
                            "{at}: moved here from {from} on {mover}, and to {other_to} on {other}"
                        );
                        let terms = [file_at(1, at), start_file.clone(), file_at(2, at)];
                        landed.push((at.clone(), row, terms));
                    } else {
                        onto.insert(at.clone(), onto_row(at, from, k));
                    }
                }
            }
            [Some(dest), None] | [None, Some(dest)] => {
                let k = if to[0].is_some() { 1 } else { 2 };
                let other = 3 - k;
                // Gone on the other line: nothing there, or a file its notes brought there.
                let removed = !was[other].kept.contains_key(from)
                    && (!was[other].files.contains_key(from)
                        || went[other - 1].arrived.contains_key(from));
                let free = room(was[other], dest);
                if removed {
                    trees[0].files.remove(from);
                    if free {
                        let row = format!(
                            "{dest}: moved here from {from} on {}, removed on {}",
                            line(k),
                            line(other)
                        );
                        let terms = [file_at(1, dest), start_file.clone(), file_at(2, dest)];
                        landed.push((dest.clone(), row, terms));
                    }
                } else if free && was[other].files.contains_key(from) {
                    // Followed: the other line's version goes where the file went.
                    relocate(&mut trees[0], from, dest);
                    relocate(&mut trees[other], from, dest);
                }
                // Anything else is merged where it is: the start's file at its old place
                // against each line's, and what each line holds where it went.
                if !free {
                    onto.insert(dest.clone(), onto_row(dest, from, k));
                }
            }
            [None, None] => unreachable!(),
        }
    }
    // A file a line's notes brought to a start file's place: the start's file is gone on that
    // line. Where the other line changed it, the two clash as one moved onto something else,
    // the start's file in the middle; else the file that came is taken.
    let mut arrived: BTreeMap<String, (String, [Option<Part>; 3])> = BTreeMap::new();
    let places: BTreeSet<&String> = (went[0].arrived.keys())
        .chain(went[1].arrived.keys())
        .collect();
    for at in places {
        let Some(start_file) = was[0].files.get(at) else {
            continue;
        };
        if moved.contains(at) || alike(was[1], was[2], at) {
            continue;
        }
        let came = [went[0].arrived.get(at), went[1].arrived.get(at)];
        let (k, from) = match came {
            [Some(from), None] => (1, from),
            [None, Some(from)] => (2, from),
            _ => {
                trees[0].files.remove(at);
                continue;
            }
        };
        match self::at(was[3 - k], at) {
            At::Nothing => drop(trees[0].files.remove(at)),
            At::File(f) if f != start_file => {
                let terms = [
                    file_at(1, at),
                    Some(Part::File(start_file.clone())),
                    file_at(2, at),
                ];
                arrived.insert(at.clone(), (onto_row(at, from, k), terms));
            }
            _ => {}
        }
    }
    // A folder the moves left empty is no version's own.
    for (tree, before) in trees.iter_mut().zip(was) {
        loop {
            let emptied: Vec<String> = (tree.folders.iter())
                .filter(|p| holds_nothing(tree, p) && !holds_nothing(before, p))
                .cloned()
                .collect();
            if emptied.is_empty() {
                break;
            }
            for p in emptied {
                tree.folders.remove(&p);
            }
        }
    }
    let [s, o, t] = &trees;
    let mut paths = PathMerge {
        three: [s, o, t],
        unsure: start.unsure.clone(),
        ours_unsure: &ours.unsure,
        out: Tree::default(),
        made: BTreeMap::new(),
        clashes: BTreeSet::new(),
        merged: Vec::new(),
        sides,
        inner,
        arrived: &arrived,
        stop: None,
    };
    for c in paths.children("") {
        paths.merge_at(&c);
    }
    if let Some(stop) = paths.stop {
        return Err(stop);
    }
    // Where a moved file met something the other line put there, the two added things clash
    // under the move's row.
    for (at, row) in onto {
        let both_added = |(r, _): &(String, Option<Stands>)| clash_kind(r) == "added on both";
        if paths.made.get(&at).is_some_and(both_added) {
            paths.made.get_mut(&at).unwrap().0 = row;
        }
    }
    for (at, row, terms) in landed {
        // A place below another conflict's is held by it, and a place met twice keeps its first.
        let mut up = at.as_str();
        let mut plain = paths.out.files.contains_key(&at) && !paths.made.contains_key(&at);
        while let Some((above, _)) = up.rsplit_once('/') {
            plain &= paths.out.folders.contains(above) && !paths.out.kept.contains_key(above);
            up = above;
        }
        if !plain {
            continue;
        }
        if inner {
            paths.out.remove(&at);
            paths.clashes.insert(at);
            continue;
        }
        let stands = paths.sides.map(|_| {
            if terms[0].is_some() {
                Stands::Yours
            } else {
                Stands::Nothing
            }
        });
        if paths.sides.is_some() {
            let own = None;
            paths.out.keep(&at, Kept { terms, own });
        } else {
            paths.out.remove(&at);
        }
        paths.made.insert(at, (row, stands));
    }
    paths.merged.sort();
    let made = (paths.made.into_iter())
        .map(|(p, (row, stands))| (p, row, stands))
        .collect();
    Ok(Merge {
        tree: paths.out,
        merged: paths.merged,
        made,
        unsure: paths.clashes,
    })
}

/// What a tree holds at one path.
#[derive(PartialEq)]
enum At<'a> {
    Nothing,
    File(&'a File),
    Folder,
    Kept(&'a Kept),
}

fn at<'a>(tree: &'a Tree, path: &str) -> At<'a> {
    if let Some(k) = tree.kept.get(path) {
        return At::Kept(k);
    }
    match tree.files.get(path) {
        Some(f) => At::File(f),
        None if tree.folders.contains(path) => At::Folder,
        None => At::Nothing,
    }
}

/// What `tree` holds at `path` as one version a conflict can hold.
fn part_at(tree: &Tree, path: &str) -> Option<Part> {
    match at(tree, path) {
        At::File(f) => Some(Part::File(f.clone())),
        At::Folder => Some(Part::Folder(tree.below(path))),
        At::Nothing => None,
        At::Kept(_) => panic!("a conflict at {path} isn't one version"),
    }
}

/// Whether nothing is below the folder `path` in `tree`.
fn holds_nothing(tree: &Tree, path: &str) -> bool {
    let below = |p: &&String| within(p, path) && *p != path;
    !tree.files.keys().any(|p| below(&p))
        && !tree.folders.iter().any(|p| below(&p))
        && !tree.kept.keys().any(|p| below(&p))
}

/// Whether two trees hold the same at `path` and below it, conflicts as stored.
fn alike(a: &Tree, b: &Tree, path: &str) -> bool {
    type Below<'t> = (
        Vec<(&'t String, &'t File)>,
        Vec<&'t String>,
        Vec<(&'t String, &'t Kept)>,
    );
    fn below<'t>(t: &'t Tree, path: &str) -> Below<'t> {
        let files = t.files.iter().filter(|(p, _)| within(p, path)).collect();
        let folders = t.folders.iter().filter(|p| within(p, path)).collect();
        let kept = t.kept.iter().filter(|(p, _)| within(p, path)).collect();
        (files, folders, kept)
    }
    below(a, path) == below(b, path)
}

/// The merge of the start, ours and theirs, path by path, after the moves.
struct PathMerge<'a> {
    three: [&'a Tree; 3],
    /// Where the start knows nothing; a start in conflict that each line made a folder of is
    /// added as it is met.
    unsure: BTreeSet<String>,
    /// Where ours, made up from starting points, is unsure: nothing there is decided.
    ours_unsure: &'a BTreeSet<String>,
    out: Tree,
    made: BTreeMap<String, (String, Option<Stands>)>,
    /// An inner merge's places that clash.
    clashes: BTreeSet<String>,
    merged: Vec<String>,
    sides: Option<&'a Sides>,
    inner: bool,
    /// Places where a file arrived and the other line changed the file that was there.
    arrived: &'a BTreeMap<String, (String, [Option<Part>; 3])>,
    stop: Option<Stopped>,
}

impl PathMerge<'_> {
    /// The paths of everything directly in the folder `path` on any of the three.
    fn children(&self, path: &str) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for t in self.three {
            for p in t.files.keys().chain(&t.folders).chain(t.kept.keys()) {
                let rest = if path.is_empty() {
                    Some(p.as_str())
                } else {
                    p.strip_prefix(path).and_then(|r| r.strip_prefix('/'))
                };
                if let Some(rest) = rest {
                    let name = rest.split('/').next().unwrap();
                    names.insert(if path.is_empty() {
                        name.to_owned()
                    } else {
                        format!("{path}/{name}")
                    });
                }
            }
        }
        names
    }

    fn merge_at(&mut self, path: &str) {
        if self.stop.is_some() {
            return;
        }
        let [s, o, t] = self.three;
        if unsure_at(self.ours_unsure, path) {
            return;
        }
        if let Some((row, terms)) = self.arrived.get(path) {
            return self.arrival(path, row.clone(), terms.clone());
        }
        // A place in conflict where the lines parted, that each line made a folder of: what was
        // there is unknown, and everything below.
        if matches!(at(s, path), At::Kept(_))
            && at(o, path) == At::Folder
            && at(t, path) == At::Folder
        {
            self.unsure.insert(path.to_owned());
        }
        // Where the start is unsure, at the path or inside it, nothing counts as unchanged: the
        // two sides alike are the result; two folders, or a folder against nothing when only
        // something inside is unsure, are merged inside; anything else conflicts.
        let here = unsure_at(&self.unsure, path);
        if here || unsure_inside(&self.unsure, path) || unsure_inside(self.ours_unsure, path) {
            if alike(o, t, path) {
                return self.out.take(path, o);
            }
            let inside = match (at(o, path), at(t, path)) {
                (At::Folder, At::Folder) => true,
                (At::Folder, At::Nothing) | (At::Nothing, At::Folder) => !here,
                _ => false,
            };
            if !inside {
                return self.decide(path, true);
            }
            return self.inside(path);
        }
        // Equal sides are the result; else the side that changed it.
        if alike(o, t, path) || alike(s, t, path) {
            return self.out.take(path, o);
        }
        if alike(s, o, path) {
            return self.out.take(path, t);
        }
        match (at(s, path), at(o, path), at(t, path)) {
            // Two folders, or a folder against its removal, merge inside.
            (_, At::Folder, At::Folder)
            | (At::Folder, At::Folder, At::Nothing)
            | (At::Folder, At::Nothing, At::Folder) => self.inside(path),
            _ => self.decide(path, false),
        }
    }

    /// Merges what is inside the folder `path`: what is left there keeps the folder. Left with
    /// nothing, two folders stay when a side holds its own empty; else the folder is gone.
    fn inside(&mut self, path: &str) {
        let [_, o, t] = self.three;
        for c in self.children(path) {
            self.merge_at(&c);
        }
        let both = at(o, path) == At::Folder && at(t, path) == At::Folder;
        if both && (holds_nothing(o, path) || holds_nothing(t, path)) {
            self.out.add_folder(path);
        }
    }

    /// A place both lines changed, in ways that aren't both folders; `unknown`: where the lines
    /// parted can't tell what it held.
    fn decide(&mut self, path: &str, unknown: bool) {
        let [s, o, t] = self.three.map(|tree| at(tree, path));
        // A conflict is never made inside another: the line holding it fixes it first.
        let on = match (matches!(o, At::Kept(_)), matches!(t, At::Kept(_))) {
            _ if self.inner => None,
            (true, true) => Some(Held::Both),
            (true, false) => Some(Held::Ours),
            (false, true) => Some(Held::Theirs),
            (false, false) => None,
        };
        if let Some(on) = on {
            let (path, at) = (path.to_owned(), path.to_owned());
            self.stop = Some(Stopped::Held { path, at, on });
            return;
        }
        // Starting points' own combine only learns that they clash here.
        if self.inner && [&s, &o, &t].iter().any(|a| matches!(a, At::Kept(_))) {
            self.clashes.insert(path.to_owned());
            return;
        }
        let both = "changed on both lines";
        // Where the lines parted it was in conflict, and each changed it since: unknown too.
        if unknown || matches!(s, At::Kept(_)) {
            if !unknown && self.sides.is_none() && !self.inner {
                met("a start in conflict, asked for a clean line");
            }
            let row =
                format!("{path}: {both}, and Opalro can't tell what it held where they parted");
            return self.no_start(path, row, Middle::Unknown);
        }
        let [ps, po, pt] = self.three.map(|tree| part_at(tree, path));
        let row = match (&s, &o, &t) {
            (_, At::Folder, At::File(_)) => format!("{path}: a file on T, a folder on O"),
            (_, At::File(_), At::Folder) => format!("{path}: a file on O, a folder on T"),
            (_, _, At::Nothing) => format!("{path}: changed on O, removed on T"),
            (_, At::Nothing, _) => format!("{path}: changed on T, removed on O"),
            (At::Nothing, _, _) => {
                let row = format!("{path}: added on both lines, differently");
                return self.no_start(path, row, Middle::Nothing);
            }
            (At::File(s), At::File(o), At::File(t)) => {
                let (s, o, t) = ((*s).clone(), (*o).clone(), (*t).clone());
                return self.file(path, &s, &o, &t);
            }
            _ => format!("{path}: {both}, and only files are merged line by line"),
        };
        self.clashed(path, row, [po, ps, pt]);
    }

    /// A clash kept standing as yours, or nothing; named only where conflicts aren't kept.
    fn clashed(&mut self, path: &str, row: String, terms: [Option<Part>; 3]) {
        if self.inner {
            self.clashes.insert(path.to_owned());
            return;
        }
        let Some(_) = self.sides else {
            self.made.insert(path.to_owned(), (row, None));
            return;
        };
        // A folder kept whole as yours or theirs may hold no conflict of its own.
        for (k, on) in [(0, Held::Ours), (2, Held::Theirs)] {
            if let Some(Part::Folder(f)) = &terms[k]
                && let Some(below) = f.kept.keys().next()
            {
                let (path, at) = (path.to_owned(), format!("{path}/{below}"));
                self.stop = Some(Stopped::Held { path, at, on });
                return;
            }
        }
        let stands = if terms[0].is_some() {
            Stands::Yours
        } else {
            Stands::Nothing
        };
        self.out.keep(path, Kept { terms, own: None });
        self.made.insert(path.to_owned(), (row, Some(stands)));
    }

    /// Kept with `own`, a text marked with `marker`-long markers at `places` places.
    fn marked_at(
        &mut self,
        path: &str,
        row: String,
        terms: [Option<Part>; 3],
        own: (File, u8, u64),
    ) {
        let (file, marker, places) = own;
        let own = Some((Some(Part::File(file)), marker));
        self.out.keep(path, Kept { terms, own });
        let stands = Some(Stands::Markers { places });
        self.made.insert(path.to_owned(), (row, stands));
    }

    /// Two versions with nothing known between them (added on both lines, or unknown): two texts
    /// are marked as one place; anything else stands as yours.
    fn no_start(&mut self, path: &str, row: String, middle: Middle) {
        let [_, o, t] = self.three;
        let terms = [part_at(o, path), None, part_at(t, path)];
        if let (Some(sides), [Some(Part::File(o)), _, Some(Part::File(t))]) = (self.sides, &terms)
            && let Some((bytes, marker, places)) =
                marked(b"", &o.bytes, &t.bytes, &sides.labels(middle), false)
        {
            let file = File {
                bytes: bytes.into(),
                exec: o.exec,
            };
            return self.marked_at(path, row, terms.clone(), (file, marker, places));
        }
        self.clashed(path, row, terms);
    }

    /// A file arrived where the other line changed the file that was there: both are shown whole.
    fn arrival(&mut self, path: &str, row: String, terms: [Option<Part>; 3]) {
        if let (
            Some(sides),
            [
                Some(Part::File(o)),
                Some(Part::File(s)),
                Some(Part::File(t)),
            ],
        ) = (self.sides, &terms)
            && let Some((bytes, marker, places)) = marked(
                &s.bytes,
                &o.bytes,
                &t.bytes,
                &sides.labels(Middle::Parted),
                true,
            )
        {
            let exec = if o.exec == s.exec { t.exec } else { o.exec };
            let file = File {
                bytes: bytes.into(),
                exec,
            };
            return self.marked_at(path, row, terms.clone(), (file, marker, places));
        }
        self.clashed(path, row, terms);
    }

    /// Two files both sides changed: the executable mark and the contents merged apart.
    fn file(&mut self, path: &str, s: &File, o: &File, t: &File) {
        let exec = if o.exec == s.exec { t.exec } else { o.exec };
        let bytes = if o.bytes == s.bytes {
            t.bytes.clone()
        } else if t.bytes == s.bytes || t.bytes == o.bytes {
            o.bytes.clone()
        } else if let Some(merged) = merge_text(&s.bytes, &o.bytes, &t.bytes) {
            self.merged.push(path.to_owned());
            merged.into()
        } else {
            let terms = [o, s, t].map(|f| Some(Part::File(f.clone())));
            let labels = self.sides.map(|sides| sides.labels(Middle::Parted));
            if let Some(labels) = labels
                && !self.inner
                && let Some((bytes, marker, places)) =
                    marked(&s.bytes, &o.bytes, &t.bytes, &labels, false)
            {
                let row = format!("{path}: changed on both lines, in the same place");
                let file = File {
                    bytes: bytes.into(),
                    exec,
                };
                return self.marked_at(path, row, terms, (file, marker, places));
            }
            let row = format!("{path}: {}", not_merged([s, o, t].map(|f| &*f.bytes)));
            return self.clashed(path, row, terms);
        };
        self.out.put(path, File { bytes, exec });
    }
}

/// Why three versions of a file both lines changed aren't merged, as a row says it.
fn not_merged(texts: [&[u8]; 3]) -> String {
    let both = "changed on both lines";
    if texts.iter().any(|t| t.len() > 64 << 20) {
        format!("{both}, and is over 64 MiB, too big to merge line by line")
    } else if texts.iter().any(|t| t[..t.len().min(8000)].contains(&0)) {
        format!("{both}, and isn't text, so it can't be merged line by line")
    } else if texts.iter().any(|t| lines_of(t).len() > 1_000_000) {
        format!("{both}, and has over 1,000,000 lines, too many to merge line by line")
    } else if endings_only(texts[1], texts[2]) {
        format!("{both}, in the same place; the two differ only in line endings")
    } else {
        format!("{both}, in the same place")
    }
}

/// The lines of a version merged as text: no zero byte in its first 8,000 bytes, at most 64 MiB
/// and 1,000,000 lines. No rule's pattern matches a name the model uses, so every file may get
/// conflict markers.
fn text_lines(bytes: &[u8]) -> Option<Vec<&[u8]>> {
    let binary = bytes[..bytes.len().min(8000)].contains(&0);
    if binary || bytes.len() > 64 << 20 {
        return None;
    }
    Some(lines_of(bytes)).filter(|l| l.len() <= 1_000_000)
}

/// Whether two texts differ, and only in some lines ending `\r\n` on one and `\n` on the other.
fn endings_only(a: &[u8], b: &[u8]) -> bool {
    let plain = |l: &[u8]| -> Vec<u8> {
        match l.strip_suffix(b"\r\n") {
            Some(rest) => [rest, b"\n"].concat(),
            None => l.to_vec(),
        }
    };
    let (a, b) = (lines_of(a), lines_of(b));
    a != b && a.len() == b.len() && a.iter().zip(&b).all(|(x, y)| plain(x) == plain(y))
}

/// A stretch of the start, `lo..hi` in lines, that one side changed to `lines`.
struct Hunk<'a> {
    lo: usize,
    hi: usize,
    lines: Vec<&'a [u8]>,
    side: usize,
}

/// The stretches of `a` that `b` changed, by a longest common subsequence of their lines.
fn hunks<'a>(a: &[&[u8]], b: &[&'a [u8]], side: usize) -> Vec<Hunk<'a>> {
    let (n, k) = (a.len(), b.len());
    // How long the longest common subsequence of a[i..] and b[j..] is.
    let mut l = vec![vec![0usize; k + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..k).rev() {
            l[i][j] = if a[i] == b[j] {
                l[i + 1][j + 1] + 1
            } else {
                l[i + 1][j].max(l[i][j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let mut close = |(lo, from): (usize, usize), (hi, to): (usize, usize)| {
        if (lo, from) != (hi, to) {
            let lines = b[from..to].to_vec();
            out.push(Hunk {
                lo,
                hi,
                lines,
                side,
            });
        }
    };
    let (mut i, mut j, mut gap) = (0, 0, (0, 0));
    while i < n && j < k {
        if a[i] == b[j] {
            close(gap, (i, j));
            (i, j) = (i + 1, j + 1);
            gap = (i, j);
        } else if l[i + 1][j] >= l[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    close(gap, (n, k));
    out
}

/// A stretch of the start, `lo..hi` in lines, that a side or both changed: what each side made
/// of it, `None` for a side that left it alone.
struct Region<'a> {
    lo: usize,
    hi: usize,
    made: [Option<Vec<&'a [u8]>>; 2],
}

/// The text merge's regions: stretches the two sides changed that overlap or touch are one.
fn regions<'a>(s: &[&'a [u8]], o: &[&'a [u8]], t: &[&'a [u8]]) -> Vec<Region<'a>> {
    let mut all = hunks(s, o, 0);
    all.extend(hunks(s, t, 1));
    all.sort_by_key(|h| (h.lo, h.hi));
    let mut out = Vec::new();
    let mut k = 0;
    while k < all.len() {
        let (lo, mut hi, mut end) = (all[k].lo, all[k].hi, k + 1);
        while end < all.len() && all[end].lo <= hi {
            hi = hi.max(all[end].hi);
            end += 1;
        }
        let region = &all[k..end];
        // What a side made of the region, if it changed anything there.
        let made = |side: usize| {
            let mut at = lo;
            let mut lines: Vec<&[u8]> = Vec::new();
            for h in region.iter().filter(|h| h.side == side) {
                lines.extend(&s[at..h.lo]);
                lines.extend(&h.lines);
                at = h.hi;
            }
            lines.extend(&s[at..hi]);
            region.iter().any(|h| h.side == side).then_some(lines)
        };
        out.push(Region {
            lo,
            hi,
            made: [made(0), made(1)],
        });
        k = end;
    }
    out
}

/// The text merge: settled when in each region only one side changed it or both wrote the same
/// lines; `None` if any isn't.
fn merge_text(start: &[u8], ours: &[u8], theirs: &[u8]) -> Option<Vec<u8>> {
    let s = text_lines(start)?;
    let (o, t) = (text_lines(ours)?, text_lines(theirs)?);
    let mut out: Vec<&[u8]> = Vec::new();
    let mut done = 0;
    for r in regions(&s, &o, &t) {
        let lines = match r.made {
            [Some(a), Some(b)] if a != b => return None,
            [Some(a), _] | [None, Some(a)] => a,
            [None, None] => unreachable!(),
        };
        out.extend(&s[done..r.lo]);
        out.extend(lines);
        done = r.hi;
    }
    out.extend(&s[done..]);
    Some(out.concat())
}

/// How a line ends where a text's first line break is, if it has one.
fn first_ending(text: &[u8]) -> Option<&'static [u8]> {
    let at = text.iter().position(|&b| b == b'\n')?;
    Some(if at > 0 && text[at - 1] == b'\r' {
        b"\r\n"
    } else {
        b"\n"
    })
}

/// The marker length for three versions: 7, or one more than the longest run of `<`, `|`, `=`
/// or `>` that starts a line in any of them; `None` past 255.
fn marker_length(texts: [&[u8]; 3]) -> Option<u8> {
    let run = |line: &[u8]| {
        let c = *line.first()?;
        b"<|=>"
            .contains(&c)
            .then(|| line.iter().take_while(|&&b| b == c).count())
    };
    let longest = (texts.iter())
        .flat_map(|t| t.split(|&b| b == b'\n'))
        .filter_map(run)
        .max()
        .unwrap_or(0);
    u8::try_from((longest + 1).max(7)).ok()
}

/// The text the person sees for a conflict shown with markers: each region the two sides changed
/// differently (with `whole`, all of each side, as one) written between marker lines labelled
/// `labels`. `None` when it isn't shown with markers: a version isn't text, the two differ only
/// in line endings, the markers would pass 255, the text 64 MiB, or nothing clashes.
fn marked(
    base: &[u8],
    ours: &[u8],
    theirs: &[u8],
    labels: &[String; 3],
    whole: bool,
) -> Option<(Vec<u8>, u8, u64)> {
    let (s, o, t) = (text_lines(base)?, text_lines(ours)?, text_lines(theirs)?);
    if endings_only(ours, theirs) {
        return None;
    }
    let len = usize::from(marker_length([base, ours, theirs])?);
    let end = (first_ending(ours).or_else(|| first_ending(theirs)))
        .or_else(|| first_ending(base))
        .unwrap_or(b"\n");
    let all = if whole {
        let made = [Some(o.clone()), Some(t.clone())];
        vec![Region {
            lo: 0,
            hi: s.len(),
            made,
        }]
    } else {
        regions(&s, &o, &t)
    };
    let mut out: Vec<u8> = Vec::new();
    let line = |out: &mut Vec<u8>, c: u8, label: Option<&str>| {
        out.extend(std::iter::repeat_n(c, len));
        if let Some(label) = label {
            out.push(b' ');
            out.extend(label.as_bytes());
        }
        out.extend(end);
    };
    let part = |out: &mut Vec<u8>, lines: &[&[u8]]| {
        out.extend(lines.concat());
        if lines.last().is_some_and(|l| !l.ends_with(b"\n")) {
            out.extend(end);
        }
    };
    let (mut done, mut places) = (0, 0);
    for r in all {
        out.extend(s[done..r.lo].concat());
        match r.made {
            [Some(a), Some(b)] if a != b => {
                line(&mut out, b'<', Some(&labels[0]));
                part(&mut out, &a);
                line(&mut out, b'|', Some(&labels[1]));
                part(&mut out, &s[r.lo..r.hi]);
                line(&mut out, b'=', None);
                part(&mut out, &b);
                line(&mut out, b'>', Some(&labels[2]));
                places += 1;
            }
            [Some(a), _] | [None, Some(a)] => out.extend(a.concat()),
            [None, None] => unreachable!(),
        }
        done = r.hi;
    }
    out.extend(s[done..].concat());
    (places > 0 && out.len() <= 64 << 20).then_some((out, len as u8, places))
}

/// Whether `line`, with its ending, is a marker line of `len` `c`s: exactly that many at its
/// start, then a space or the line's end.
fn is_marker(line: &[u8], c: u8, len: u8) -> bool {
    let len = usize::from(len);
    line.len() >= len
        && line[..len].iter().all(|&b| b == c)
        && matches!(&line[len..], [] | [b' ', ..] | b"\n" | b"\r\n")
}

/// How many opening and closing marker lines of `len` a text holds.
fn marker_lines(bytes: &[u8], len: u8) -> (u64, u64) {
    let lines = lines_of(bytes);
    let count = |c| lines.iter().filter(|l| is_marker(l, c, len)).count() as u64;
    (count(b'<'), count(b'>'))
}

/// How a place in conflict stands, the folder holding `now` there: as it was left, with markers
/// left or none, as yours, or changed in place.
fn standing_of(k: &Kept, now: Option<&Part>) -> Standing {
    let marker = k.marker();
    let left = |now: Option<&Part>| match now {
        Some(Part::File(f)) => match marker_lines(&f.bytes, marker) {
            (0, 0) => None,
            (opening, _) => Some(opening),
        },
        _ => None,
    };
    if now == k.stands() {
        return match (marker, &k.own, &k.terms[0]) {
            (1.., _, _) => left(now).map_or(Standing::Fixed, |left| Standing::Marked { left }),
            (0, Some(_), _) => Standing::ChangedInPlace,
            (0, None, None) => Standing::NotInFolder,
            (0, None, Some(_)) => Standing::Yours,
        };
    }
    if marker == 0 {
        return Standing::ChangedInPlace;
    }
    left(now).map_or(Standing::Fixed, |left| Standing::Marked { left })
}

/// What `status` lists as in conflict: every place the save the project is on holds in
/// conflict, with how it stands in the folder.
fn conflicts_listed(m: &Model) -> Vec<(String, Standing)> {
    (m.kept().iter())
        .map(|(p, k)| (p.clone(), standing_of(k, part_at(&m.folder, p).as_ref())))
        .collect()
}

/// What `resolve` is given: a path (`""` for the whole project), or a pattern matching exactly
/// the one place it holds.
enum Asked {
    Path(String),
    Pattern(String),
}

/// Whether `keep` asks for a clean line.
fn keep_of(clean: bool) -> Keep {
    if clean {
        Keep::OnlyIfClean
    } else {
        Keep::Conflicts
    }
}

/// `resolve` given `asked`, or nothing for every place the folder shows fixed; `yes` answers
/// whether to record a file with markers left. Gives what came of it: `recorded`, or why not.
fn resolve_at(
    repo: &mut Repository,
    m: &mut Model,
    asked_for: Option<Asked>,
    how: ResolveHow,
    yes: bool,
) -> &'static str {
    let who = who();
    let given = |a: &Asked| match a {
        Asked::Path(p) => Given::Path(p.clone()),
        Asked::Pattern(place) => {
            // The place's last letter in brackets: a pattern that matches it and nothing else.
            let (head, last) = place.split_at(place.len() - 1);
            let glob = format!("{head}[{last}]");
            let below = String::new();
            Given::Pattern { below, glob }
        }
    };
    let paths: Option<Vec<Given>> = asked_for.as_ref().map(|a| vec![given(a)]);
    let keep = match how {
        ResolveHow::KeepMine => Some(resolve::Side::Mine),
        ResolveHow::KeepTheirs => Some(resolve::Side::Theirs),
        _ => None,
    };
    let req = ResolveRequest {
        paths: paths.as_deref(),
        keep,
        who: &who,
        command: "resolve",
        accept_crlf: false,
        git_excludes: None,
    };
    let mut asked = Vec::new();
    let done = resolve::resolve(repo, &req, &mut |_| {}, &mut |ask| match ask {
        resolve::Ask::MarkersLeft { path, left } => {
            asked.push((path.to_string(), *left));
            yes
        }
        resolve::Ask::Large(_) => true,
    });
    let refused = |done: &Result<ResolveOutcome, ResolveError>, want: &str| {
        let got = match done {
            Ok(_) => "recorded",
            Err(ResolveError::NotOnALine) => "not on a line",
            Err(ResolveError::NotInConflict { .. }) => "not in conflict",
            Err(ResolveError::StillYours { .. }) => "still yours",
            Err(ResolveError::MarkersLeft { .. }) => "markers left",
            Err(_) => "another error",
        };
        assert_eq!(got, want, "{done:?}");
    };
    let Some(line) = m.state.line.clone() else {
        refused(&done, "not on a line");
        settle(repo, m, Vec::new());
        return "not on a line";
    };
    let on = m.state.on;
    if m.kept().is_empty() {
        let done = done.unwrap();
        assert!(
            matches!(done, ResolveOutcome::NoConflicts { save_id } if save_id == m.saves[on].id),
            "{done:?}"
        );
        settle(repo, m, Vec::new());
        return "no conflicts";
    }
    let listed = conflicts_listed(m);
    let several = matches!(&asked_for, Some(Asked::Path(top)) if !top.is_empty()
        && listed.iter().filter(|(p, _)| within(p, top)).count() > 1);
    let fixed = |s: &Standing| *s == Standing::Fixed;
    // The places named, or those the folder shows fixed; a pattern takes only those too.
    let chosen: Vec<&(String, Standing)> = match &asked_for {
        None => listed.iter().filter(|(_, s)| fixed(s)).collect(),
        Some(Asked::Path(top)) => listed
            .iter()
            .filter(|(p, _)| top.is_empty() || within(p, top))
            .collect(),
        Some(Asked::Pattern(place)) => listed
            .iter()
            .filter(|(p, s)| p == place && (keep.is_some() || fixed(s)))
            .collect(),
    };
    if matches!(asked_for, Some(Asked::Path(_))) && chosen.is_empty() {
        refused(&done, "not in conflict");
        met("resolve refused: not in conflict");
        settle(repo, m, Vec::new());
        return "not in conflict";
    }
    if keep.is_none() {
        // Without a side named, only what the folder shows decides: a version standing as the
        // combine left it is refused, and markers left are asked about, one place at a time.
        let still = |s: &Standing| matches!(s, Standing::Yours | Standing::NotInFolder);
        if chosen.iter().any(|(_, s)| still(s)) {
            refused(&done, "still yours");
            met("resolve refused: still yours");
            settle(repo, m, Vec::new());
            return "still yours";
        }
        let marked: Vec<(String, u64)> = (chosen.iter())
            .filter_map(|(p, s)| match s {
                Standing::Marked { left } => Some((p.clone(), *left)),
                _ => None,
            })
            .collect();
        let want_asked = if yes {
            &marked[..]
        } else {
            &marked[..marked.len().min(1)]
        };
        assert_eq!(asked, want_asked, "what resolve asked");
        if !yes && !marked.is_empty() {
            refused(&done, "markers left");
            met("resolve refused: markers left, answered no");
            settle(repo, m, Vec::new());
            return "markers left";
        }
    }
    let places: Vec<String> = chosen.iter().map(|(p, _)| p.clone()).collect();
    let left_count = (listed.len() - places.len()) as u64;
    let done = done.unwrap();
    if places.is_empty() {
        let ResolveOutcome::Nothing {
            left_count: got, ..
        } = done
        else {
            panic!("the model records nothing, the engine {done:?}");
        };
        assert_eq!(got, left_count);
        settle(repo, m, Vec::new());
        return "nothing";
    }
    let ResolveOutcome::Recorded {
        save_id,
        replaced,
        recorded,
        left_count: got_left,
        ..
    } = done
    else {
        panic!("the model records {places:?}, the engine {done:?}");
    };
    assert_eq!((&recorded, got_left), (&places, left_count));
    // Into the combined save where nothing else shows it, else a save of its own on top.
    let amend = !m.saves[on].others.is_empty() && !shown_elsewhere(m, &line, on);
    assert_eq!(
        replaced,
        amend.then_some(m.saves[on].id),
        "what resolve replaced"
    );
    let mut stored = m.saves[on].stored.clone();
    let mut folder = m.folder.clone();
    for p in &places {
        let k = &m.kept()[p];
        match keep {
            Some(side) => {
                let t = if side == resolve::Side::Mine { 0 } else { 2 };
                stored.place(p, k.terms[t].as_ref());
                folder.place(p, k.terms[t].as_ref());
            }
            None => stored.place(p, part_at(&m.folder, p).as_ref()),
        }
    }
    assert_eq!(
        saved_tree(repo.store(), save_id),
        stored,
        "what resolve saved"
    );
    let (parent, others, change, message) = if amend {
        let s = &m.saves[on];
        (s.parent, s.others.clone(), s.change, s.message.clone())
    } else {
        let message = match &places[..] {
            [one] => format!("Fixed conflicts in {one}"),
            many => format!("Fixed conflicts in {} places", many.len()),
        };
        (Some(on), Vec::new(), m.saves.len(), message)
    };
    met(if amend {
        "resolve into the combined save"
    } else {
        "resolve on a save of its own"
    });
    let was = m.state.clone();
    m.saves.push(Saved {
        id: save_id,
        tree: stored.standing(),
        stored,
        parent,
        others,
        change,
        message,
        shelf: false,
    });
    m.state.on = m.saves.len() - 1;
    m.state.lines.insert(line, m.state.on);
    let held = std::mem::replace(&mut m.folder, folder);
    stepped(repo, m, "resolve", was, held);
    if several {
        met("resolve at a folder above several places");
    }
    met(match how {
        ResolveHow::All => "resolve with no paths",
        ResolveHow::Paths => "resolve with a path",
        ResolveHow::Pattern => "resolve with a pattern",
        ResolveHow::KeepMine => "resolve --keep mine",
        ResolveHow::KeepTheirs => "resolve --keep theirs",
    });
    "recorded"
}

/// Writes a file shown with markers, that still has them, with each marked place made into
/// your lines then theirs; one place is left as it is when `pick` says so.
fn fix_file(m: &mut Model, root: &Path, pick: usize) -> Option<String> {
    let marked: Vec<(String, u8)> = (m.kept().iter())
        .filter(|(_, k)| k.marker() > 0)
        .filter_map(|(p, k)| match m.folder.files.get(p) {
            Some(f) if marker_lines(&f.bytes, k.marker()).0 > 0 => Some((p.clone(), k.marker())),
            _ => None,
        })
        .collect();
    let (path, len) = marked.get(pick % marked.len().max(1))?.clone();
    let was = m.folder.files[&path].bytes.clone();
    let mut leave = usize::from((pick / 3).is_multiple_of(2) && marker_lines(&was, len).0 > 1);
    let mut out: Vec<u8> = Vec::new();
    // 0 outside a marked place; 1 in yours, 2 where they parted, 3 in theirs.
    let (mut part, mut kept): (u8, Vec<u8>) = (0, Vec::new());
    let mut held: Vec<u8> = Vec::new();
    for l in lines_of(&was) {
        let is = |c| is_marker(l, c, len);
        match part {
            0 if is(b'<') => {
                part = 1;
                held = l.to_vec();
            }
            0 => out.extend(l),
            _ => {
                held.extend(l);
                if part == 1 && is(b'|') {
                    part = 2;
                } else if part == 2 && is(b'=') {
                    part = 3;
                } else if is(b'>') {
                    part = 0;
                    if leave > 0 {
                        leave -= 1;
                        out.extend(&held);
                    } else {
                        out.extend(&kept);
                    }
                    kept.clear();
                } else if part != 2 {
                    kept.extend(l);
                }
            }
        }
    }
    // An unclosed place keeps its lines.
    if part != 0 {
        out.extend(&held);
    }
    if *out == *was {
        return None;
    }
    met("a marked file fixed by hand");
    put(m, root, &path, out, When::Now)
}

/// Changes what the folder holds at a place in conflict that stands as a version: a file's text
/// edited, or a new text where nothing stands.
fn change_in_place(m: &mut Model, root: &Path, pick: usize) -> Option<String> {
    let places: Vec<String> = (m.kept().iter())
        .filter(|(p, k)| {
            let above_free = !m.folder.files.keys().any(|f| within(p, f) && f != *p);
            k.marker() == 0 && above_free && !m.folder.folders.contains(*p)
        })
        .map(|(p, _)| p.clone())
        .collect();
    let path = places.get(pick % places.len().max(1))?.clone();
    edit_text(m, root, &path, pick, (pick % 3) as u8)
}

/// `tidy`, two days on so that every pack counts as old, keeping every step: every version a
/// kept save's conflicts name must still read back, and nothing else changes.
fn tidy_and_check(repo: &mut Repository, m: &mut Model) {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let later = now + 2 * 86_400_000;
    opal_core::tidy::tidy(repo, 36_500, later, &mut |_| {}, &mut |_| {}).unwrap();
    no_version_a_kept_conflict_names_is_freed(repo, m);
    // A save in conflict that no line, tag or shelf reaches, only a step undo or redo can take.
    let held: Vec<usize> = (m.state.lines.values())
        .chain(m.state.tags.values())
        .chain(m.state.shelves.iter().map(|(_, s)| s))
        .copied()
        .collect();
    let reached = history(m, &held);
    let only_by_steps = (0..m.saves.len())
        .any(|i| !m.saves[i].shelf && !m.saves[i].stored.kept.is_empty() && !reached.contains(&i));
    if only_by_steps {
        met("tidy with a conflict only undo reaches");
    }
    settle(repo, m, Vec::new());
}

/// Every save the model knows is kept (every step is), and every version its conflicts name, as
/// a term or as its own, reads back whole.
fn no_version_a_kept_conflict_names_is_freed(repo: &Repository, m: &Model) {
    for s in &m.saves {
        let stored = saved_tree(repo.store(), s.id);
        if s.shelf {
            assert_eq!(stored.standing(), s.tree, "shelf {:?} after tidy", s.id);
        } else {
            assert_eq!(stored, s.stored, "save {:?} after tidy", s.id);
        }
    }
}

/// Settles what a command did: a step if it changed the state or the folder, else no entry.
/// The step clears what redo could bring back.
fn stepped(repo: &Repository, m: &mut Model, command: &'static str, was: State, held: Tree) {
    let made_conflicts = std::mem::take(&mut m.made_conflicts);
    if recorded(m, &m.state) == recorded(m, &was) && m.folder == held {
        settle(repo, m, Vec::new());
        return;
    }
    let made = vec![(Made::Command(command), m.state.clone())];
    let entry = settle(repo, m, made)[0];
    let change = repo.entry(&entry).unwrap().folder_change;
    assert_eq!(
        change.is_some(),
        m.folder != held,
        "{command} records a folder change"
    );
    if let Some(FolderChange { from, to }) = change {
        // Its two saves differ exactly where the step changed the folder.
        let stands = |save| saved_tree(repo.store(), save).standing();
        let (from, to) = (stands(from), stands(to));
        let want = differences(&held, &m.folder);
        assert_eq!(differences(&from, &to), want, "{command} records");
    }
    m.undone.clear();
    m.done.push(Step {
        entry,
        command,
        before: Some(was),
        after: m.state.clone(),
        folder: (held, m.folder.clone()),
        made_conflicts,
    });
}

/// Holds the entries the diary gained since the model's last one to `made`: what each records,
/// and the state the model gives it. Adds them to the model's, and gives their IDs.
fn settle(repo: &Repository, m: &mut Model, made: Vec<(Made, State)>) -> Vec<Id> {
    let mut new = Vec::new();
    let mut at = repo.head().unwrap();
    while at != m.head() {
        assert!(
            new.len() < made.len(),
            "the diary gained more than the {} entries the model made",
            made.len()
        );
        new.push(at);
        at = repo
            .entry(&at)
            .unwrap()
            .parent
            .expect("no entry the model has");
    }
    assert_eq!(new.len(), made.len(), "new diary entries");
    new.reverse();
    for (id, (kind, state)) in new.iter().zip(made) {
        let entry = repo.entry(id).unwrap();
        let (action, command) = match kind {
            Made::Command(command) => (Action::Command, command),
            Made::Undo(step) => (Action::Undo(step), "undo"),
            Made::Redo(step) => (Action::Redo(step), "redo"),
        };
        let told = (&entry.action, entry.command.as_str());
        assert_eq!(told, (&action, command), "entry {id:?}");
        if let Made::Undo(step) | Made::Redo(step) = kind {
            // The step's own folder change, backwards for an undo.
            let change = repo.entry(&step).unwrap().folder_change;
            let change = change.map(|c| match kind {
                Made::Undo(_) => FolderChange {
                    from: c.to,
                    to: c.from,
                },
                _ => c,
            });
            assert_eq!(entry.folder_change, change, "the folder change of {kind:?}");
        }
        assert_eq!(entry.state, recorded(m, &state), "entry {id:?}");
        m.entries.push((*id, state));
    }
    new
}

/// The changes `status` should list, by path.
fn changes(folder: &Tree, saved: &Tree) -> Vec<(String, ChangeKind)> {
    changed_files(folder, saved)
        .into_iter()
        .map(|p| {
            let kind = match (saved.files.get(&p), folder.files.get(&p)) {
                (None, _) => ChangeKind::Added,
                (_, None) => ChangeKind::Removed,
                (Some(a), Some(b)) if a.exec != b.exec => ChangeKind::KindChanged,
                _ => ChangeKind::Modified,
            };
            (p, kind)
        })
        .collect()
}

/// What the folder, `status`, the state and undo's lists say, against the model. `missable` is
/// a change the file cache may miss: status that trusts it may say either, and a full one must
/// see it.
fn check(
    repo: &Repository,
    m: &Model,
    root: &Path,
    full: bool,
    missable: Option<&str>,
) -> Result<(), String> {
    let on_disk = folder_tree(root);
    if on_disk != m.folder {
        return Err(format!("the folder holds {on_disk:?}, not {:?}", m.folder));
    }
    // A place in conflict is listed as one, not again among the changes.
    let in_conflict = |p: &str| m.kept().keys().any(|k| within(p, k));
    let mut want = changes(&m.folder, m.tip());
    want.retain(|(p, _)| !in_conflict(p));
    let conflicts = conflicts_listed(m);
    for (_, standing) in &conflicts {
        met(match standing {
            Standing::Marked { .. } => "status: markers left",
            Standing::Fixed => "status: fixed",
            Standing::Yours => "status: yours",
            Standing::ChangedInPlace => "status: changed in place",
            Standing::NotInFolder => "status: not in the folder",
            Standing::NotHere => "status: not here",
        });
    }
    let listed = |full: bool| -> Result<Vec<(String, ChangeKind)>, String> {
        let st = status(repo, full, None).map_err(|e| e.to_string())?;
        if st.save_id != m.saves[m.state.on].id {
            return Err("status compared with the wrong save".into());
        }
        let got: Vec<(String, Standing)> = (st.conflicts.iter())
            .map(|c| (c.path.clone(), c.standing))
            .collect();
        // A change the file cache may miss may leave a place's standing as it was.
        let missed = |(p, _): &&(String, Standing)| Some(p.as_str()) != missable || full;
        let (got, wanted): (Vec<_>, Vec<_>) = (
            got.iter().filter(missed).collect(),
            conflicts.iter().filter(missed).collect(),
        );
        if got != wanted {
            return Err(format!("status lists in conflict {got:?}, not {wanted:?}"));
        }
        Ok(st
            .changes
            .iter()
            .map(|c| (c.path.clone(), c.change))
            .collect())
    };
    let got = listed(full)?;
    let other = |l: &[(String, ChangeKind)]| -> Vec<(String, ChangeKind)> {
        l.iter()
            .filter(|(p, _)| Some(p.as_str()) != missable)
            .cloned()
            .collect()
    };
    if other(&got) != other(&want) {
        return Err(format!("status says {got:?}, not {want:?}"));
    }
    if missable.is_some() {
        let got = listed(true)?;
        if got != want {
            return Err(format!(
                "status reading every file says {got:?}, not {want:?}"
            ));
        }
    } else if got != want {
        return Err(format!("status says {got:?}, not {want:?}"));
    }
    let (head, entry) = repo.current().map_err(|e| e.to_string())?;
    if head != m.head() {
        return Err(format!("the diary is at {head:?}, not {:?}", m.head()));
    }
    let state = recorded(m, &m.state);
    if entry.state != state {
        return Err(format!("the state is {:?}, not {state:?}", entry.state));
    }
    let to_redo: Vec<&Step> = m.undone.iter().rev().collect();
    // A short list too, since a case rarely makes more steps than the list shows.
    for limit in [LISTED, 3] {
        let list = undo_list(repo, limit).map_err(|e| e.to_string())?;
        let to_undo: Vec<&Step> = m.done.iter().rev().take(limit).collect();
        steps_listed(m, "redo", &to_redo, &list.redo)?;
        steps_listed(m, "undo", &to_undo, &list.undo)?;
        // The diary's first entry, which can't be undone, is never one more.
        if list.more != (m.done.len() > limit + 1) {
            return Err(format!("undo's list of {limit} says more is {}", list.more));
        }
    }
    Ok(())
}

/// Whether one of undo's lists holds exactly `steps`, each with what it changed.
fn steps_listed(m: &Model, which: &str, steps: &[&Step], got: &[StepInfo]) -> Result<(), String> {
    let found: Vec<Id> = got.iter().map(|s| s.entry).collect();
    let want: Vec<Id> = steps.iter().map(|s| s.entry).collect();
    if found != want {
        return Err(format!("the {which} list holds {found:?}, not {want:?}"));
    }
    for (s, g) in steps.iter().zip(got) {
        let files = g.diary.folder_change;
        let effects = s.effects(m, files);
        let told = (
            &g.diary.action,
            g.diary.command.as_str(),
            files.is_some(),
            &g.effects,
        );
        if told != (&Action::Command, s.command, s.wrote(), &effects) {
            return Err(format!(
                "the {which} list tells {:?} as {told:?}, not {} {effects:?}",
                g.entry, s.command
            ));
        }
    }
    Ok(())
}

/// `opalro undo --list --json`, against the model.
fn listed_document(env: &Env, repo: &Repository, m: &Model) {
    let out = env
        .opalro()
        .args(["undo", "--list", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    let entries =
        |steps: Vec<&Step>| -> Value { steps.into_iter().map(|s| step_json(repo, m, s)).collect() };
    let want = json!({
        "opalro_json": 1,
        "redo": entries(m.undone.iter().rev().collect()),
        "undo": entries(m.done.iter().rev().take(LISTED).collect()),
        "more": m.done.len() > LISTED + 1,
    });
    let got = json!({
        "opalro_json": doc["opalro_json"],
        "redo": doc["redo"],
        "undo": doc["undo"],
        "more": doc["more"],
    });
    assert_eq!(got, want);
}

/// A step as `undo --list --json` gives it (json.md).
fn step_json(repo: &Repository, m: &Model, s: &Step) -> Value {
    let files = repo.entry(&s.entry).unwrap().folder_change;
    let effects: Vec<Value> = s.effects(m, files).iter().map(effect_json).collect();
    json!({
        "entry": s.entry.to_hex(),
        "action": "command",
        "target": null,
        "command": s.command,
        "time_ms": who().time_ms,
        "effects": effects,
        "first": s.before.is_none(),
    })
}

/// An effect as json.md gives it.
fn effect_json(e: &Effect) -> Value {
    let hex = |id: &Option<Id>| id.map(|id| id.to_hex());
    let place = |p: &Position| match p {
        Position::OnLine(line) => json!({ "line": line }),
        Position::AtSave(save) => json!({ "save": save.to_hex() }),
    };
    match e {
        Effect::Line { name, from, to } => {
            json!({"kind": "line", "name": name, "from": hex(from), "to": hex(to)})
        }
        Effect::Position { from, to } => {
            json!({"kind": "position", "from": place(from), "to": place(to)})
        }
        Effect::ShelfAdded { label, save } => {
            json!({"kind": "shelf_added", "label": label, "save": save.to_hex()})
        }
        Effect::ShelfGone { label, save } => {
            json!({"kind": "shelf_gone", "label": label, "save": save.to_hex()})
        }
        Effect::Files { from, to } => {
            json!({"kind": "files", "from": from.to_hex(), "to": to.to_hex()})
        }
        Effect::Tags => json!({"kind": "tags"}),
        Effect::ServerLines => panic!("the model has no server lines"),
    }
}

#[cfg(unix)]
fn executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(_: &std::fs::Metadata) -> bool {
    false
}

/// What the project folder holds, `.opalro` left out.
fn folder_tree(root: &Path) -> Tree {
    let mut tree = Tree::default();
    let mut todo = vec![(root.to_owned(), String::new())];
    while let Some((dir, prefix)) = todo.pop() {
        for e in std::fs::read_dir(&dir).unwrap() {
            let e = e.unwrap();
            let name = e.file_name().into_string().unwrap();
            if prefix.is_empty() && name == ".opalro" {
                continue;
            }
            let path = format!("{prefix}{name}");
            let meta = e.metadata().unwrap();
            if meta.is_dir() {
                tree.folders.insert(path.clone());
                todo.push((e.path(), format!("{path}/")));
            } else {
                let bytes = std::fs::read(e.path()).unwrap().into();
                let exec = executable(&meta);
                tree.files.insert(path, File { bytes, exec });
            }
        }
    }
    tree
}

/// What a save holds as stored, read from the store, every version of its conflicts read whole.
fn saved_tree(store: &Store, save: Id) -> Tree {
    listed_tree(store, store.save(&save, false).unwrap().root)
}

/// What the folder listing `root` holds, as stored.
fn listed_tree(store: &Store, root: Id) -> Tree {
    let mut tree = Tree::default();
    let mut todo = vec![(root, String::new())];
    while let Some((id, prefix)) = todo.pop() {
        for e in store.listing(&id).unwrap().entries() {
            let path = format!("{prefix}{}", e.name);
            match e.kind {
                EntryKind::Folder => {
                    tree.folders.insert(path.clone());
                    todo.push((e.id, format!("{path}/")));
                }
                EntryKind::Conflict => {
                    let record = store.conflict(&e.id).unwrap();
                    let part = |t: &Option<Term>| t.map(|t| read_part(store, t));
                    let [mine, before, theirs] = &record.terms[..] else {
                        panic!("{path} holds a conflict of {} versions", record.terms.len());
                    };
                    let terms = [part(mine), part(before), part(theirs)];
                    let own = record.own.map(|own| (part(&own.version), own.marker));
                    tree.kept.insert(path, Kept { terms, own });
                }
                kind => {
                    let term = Term {
                        kind,
                        id: e.id,
                        size: e.size,
                    };
                    let Part::File(f) = read_part(store, term) else {
                        unreachable!()
                    };
                    tree.files.insert(path, f);
                }
            }
        }
    }
    tree
}

/// One version, read whole.
fn read_part(store: &Store, t: Term) -> Part {
    let exec = match t.kind {
        EntryKind::Folder => return Part::Folder(listed_tree(store, t.id)),
        EntryKind::File => false,
        EntryKind::Executable => true,
        other => panic!("a version saved as {other:?}"),
    };
    let mut bytes = Vec::new();
    store.copy_content(&t.id, &mut bytes).unwrap();
    let bytes = bytes.into();
    Part::File(File { bytes, exec })
}

thread_local! {
    /// How many cases of each kind this thread's runs compared.
    static MET: RefCell<BTreeMap<&'static str, usize>> = const { RefCell::new(BTreeMap::new()) };
}

fn met(kind: &'static str) {
    MET.with(|m| *m.borrow_mut().entry(kind).or_default() += 1);
}

/// 64 cases on every push; nightly sets `PROPTEST_CASES` higher.
fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(64)
}

/// The kinds of case the fixed pass must meet at least 3 times each: every clash a combine keeps
/// that the model's files can make (two kinds at one place, a link, a rule against markers and
/// files past the text limits they can't), the starting points that disagree, a line's conflict
/// that stops a combine, and each way the other commands meet a conflict.
const MUST_MEET: [&str; 37] = [
    "changed on both, in the same place",
    "changed on both, not text",
    "changed, removed",
    "added on both",
    "moved apart",
    "moved, removed",
    "moved onto",
    "start unknown",
    "starting points that disagree",
    "stopped at a conflict a line holds",
    "--only-if-clean: a conflict it would make",
    "--only-if-clean: a conflict carried from a line",
    "--only-if-clean: a forward onto a save in conflict",
    "resolve with no paths",
    "resolve with a path",
    "resolve with a pattern",
    "resolve --keep mine",
    "resolve --keep theirs",
    "resolve into the combined save",
    "resolve on a save of its own",
    "resolve refused: not in conflict",
    "resolve refused: still yours",
    "resolve refused: markers left, answered no",
    "resolve at a folder above several places",
    "a marked file fixed by hand",
    "save fixed a marked text",
    "save kept a half-fixed text",
    "save kept a picture changed in place",
    "a rename at a place in conflict",
    "save <path> at a place in conflict",
    "undo over a combine that kept conflicts",
    "redo over a combine that kept conflicts",
    "undo over a save that kept a conflict",
    "redo over a save that kept a conflict",
    "undo over a resolve",
    "redo over a resolve",
    "tidy with a conflict only undo reaches",
];

/// How many cases the fixed pass runs.
const FIXED_CASES: u32 = 128;

/// Runs `run` on `config.cases` cases drawn by `runner`, then gives what they met.
fn pass(mut runner: TestRunner, everywhere_alike: bool) -> BTreeMap<&'static str, usize> {
    let acts = prop::collection::vec(act(everywhere_alike), 1..=30);
    let done = runner.run(&acts, |acts| {
        run(&acts);
        Ok(())
    });
    if let Err(e) = done {
        panic!("{e}");
    }
    MET.with(|m| m.borrow().clone())
}

/// Cases drawn anew on every run, to find what nobody thought of: 64 on every push, more nightly.
/// It asserts no count, and prints them.
#[test]
fn edits_saves_switches_shelves_and_undos_match_the_model() {
    let config = ProptestConfig {
        cases: cases(),
        source_file: Some(file!()),
        ..ProptestConfig::default()
    };
    let met = pass(TestRunner::new(config), false);
    println!("met: {met:#?}");
}

/// The same on cases from a fixed seed, the same on every run, on which every kind the model is
/// there to exercise must be met at least 3 times. It draws no act whose effect differs between
/// systems, so its counts are the same everywhere and on every run, but where three or more
/// starting points are combined in an order random change IDs set: that moves only "starting
/// points that disagree" and "start unknown", both far above their floors.
#[test]
fn a_fixed_pass_meets_every_kind_of_case() {
    let config = ProptestConfig {
        cases: FIXED_CASES,
        failure_persistence: None,
        ..ProptestConfig::default()
    };
    let rng = TestRng::deterministic_rng(RngAlgorithm::ChaCha);
    let met = pass(TestRunner::new_with_rng(config, rng), true);
    println!("met on the fixed pass: {met:#?}");
    let short: Vec<(&str, usize)> = (MUST_MEET.iter())
        .map(|k| (*k, met.get(k).copied().unwrap_or(0)))
        .filter(|(_, n)| *n < 3)
        .collect();
    assert!(short.is_empty(), "met fewer than 3 times: {short:?}");
}

/// A combine whose lines parted at a save holding a conflict that each line fixed its own way:
/// one line kept theirs, the other fixed the markers by hand. `clean` asks for a clean line.
fn parted_in_conflict(clean: bool) -> Vec<Act> {
    let fork = Act::Fork {
        clean: false,
        pick: 1,
        path: "a",
        at: 0,
        edit: 2,
        back: true,
        changes: (Change::Edit, Change::Edit),
    };
    let to_line_0 = Act::Goto {
        pick: 0,
        by_line: true,
        unsaved: Unsaved::Save,
    };
    let keep_theirs = Act::Resolve {
        pick: 0,
        how: ResolveHow::KeepTheirs,
    };
    let combine_side = Act::Combine {
        pick: 0,
        message: false,
        clean,
    };
    vec![
        fork,
        Act::LineNew("side"),
        Act::FixFile { pick: 1 },
        Act::Save,
        to_line_0,
        keep_theirs,
        combine_side,
    ]
}

#[test]
fn a_start_in_conflict_that_both_lines_fixed_is_unknown() {
    run(&parted_in_conflict(false));
    let met = MET.with(|m| m.borrow().get("start unknown").copied());
    assert_eq!(met, Some(1));
}

#[test]
fn a_start_in_conflict_that_both_lines_fixed_is_unknown_when_asked_for_a_clean_line() {
    run(&parted_in_conflict(true));
}

/// The seed's history: a file moved to two places where both lines hold it alike at one of them.
#[test]
fn a_file_moved_to_two_places_held_alike_at_one_is_no_conflict_there() {
    run(&[
        Act::Big("d/a", 0, 1_048_576),
        Act::Fork {
            clean: false,
            pick: 4_915_421_869_871_524_073,
            path: "a",
            at: 0,
            edit: 3,
            back: true,
            changes: (Change::EditEnd, Change::Edit),
        },
        Act::Goto {
            pick: 13_901_229_420_598_900_031,
            by_line: false,
            unsaved: Unsaved::Save,
        },
        Act::Moved {
            pick: 7_925_481_868_113_665_007,
            to: "e/a",
            remake: true,
        },
        Act::Fork {
            clean: false,
            pick: 13_567_937_977_859_821_551,
            path: "a",
            at: 14_330_216_744_520_979_105,
            edit: 39,
            back: true,
            changes: (
                Change::Add(10_411_058_395_405_241_676),
                Change::Move(10_411_058_395_405_241_676, false),
            ),
        },
        Act::UndoTo {
            pick: 567_419_809_342_993_382,
            shelve: true,
        },
        Act::CrissCross {
            clean: false,
            pick: 14_366_835_740_253_199_054,
            fix: true,
        },
        Act::Undo { n: 1, shelve: true },
        Act::Combine {
            pick: 774_662_330_187_787_330,
            message: true,
            clean: true,
        },
    ]);
}
