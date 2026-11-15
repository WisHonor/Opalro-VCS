//! The merge of two versions of the project against their starting point, entry by entry, one
//! folder at a time.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use super::{
    Clash, CombineError, Conflict, Held, Names, NotMerged, Side, Stands, Text, Which, merge_text,
};
use crate::changes::{self, stored};
use crate::conflict::markers::{Parted, Sides};
use crate::conflict::{Own, Record, Term};
use crate::diff::again;
use crate::id::Id;
use crate::ingest::store_held;
use crate::listing::{
    Entry, EntryKind, Listing, MAX_ENTRIES, MAX_SAVE_DEPTH, MAX_SAVE_ENTRIES, SaveLimits,
};
use crate::names::{PortabilityProblem, folder_problems};
use crate::pack::PackSet;
use crate::rules::{LineMerge, RuleMatcher};
use crate::snapshot::{ingest_failed, store_listing, store_record};
use crate::store::{Store, StoreError};

/// The most text merged by inner combines that one combine holds in memory.
pub const MAX_HELD_TEXT: u64 = 256 << 20;

/// Conflicts named; the rest are only counted.
pub(super) const MAX_NAMED: usize = 100;

/// What a combine has built that a later merge of the same combine may read, before anything
/// is stored. Every object it built is in the combine's `PackSet` too.
#[derive(Debug)]
pub(crate) struct Made {
    pub listings: HashMap<Id, Listing>,
    /// Texts merged by inner combines only: a made-up version a later merge reads holds them.
    pub texts: HashMap<Id, Vec<u8>>,
    /// Their bytes, and the most allowed (`MAX_HELD_TEXT`; tests lower it).
    pub held: u64,
    pub max_held: u64,
    /// The most memory taken to hold the files that left a side and arrived, to follow them by
    /// contents
    /// (`moves::MAX_LEFT_BYTES`; tests lower it).
    pub max_left_bytes: usize,
    /// How long each file's comparison may take (`changes::deadline()`; tests lower it).
    pub deadline: Duration,
    /// How many more move notes this combine may check (`moves::MAX_CHECKED_NOTES` at first;
    /// tests lower it), and whether a save's notes were past it, so no more notes count.
    pub notes_left: usize,
    pub notes_spent: bool,
    /// The records this combine made, by ID, for measuring its result.
    pub records: HashMap<Id, Record>,
    /// Whether conflicts are kept (the project keeps them, and `Keep::Conflicts`), with what
    /// every label of this combine is made from; each file's labels come from it.
    pub keep: Option<Sides>,
    /// Whether the project keeps conflicts: only then is a place a line holds in conflict
    /// refused as one.
    pub keeps: bool,
    /// The most entries the checks for a conflict below a folder may read in each version
    /// (`MAX_SAVE_ENTRIES`; tests lower it).
    pub max_below: u64,
}

impl Default for Made {
    fn default() -> Made {
        Made {
            listings: HashMap::new(),
            texts: HashMap::new(),
            held: 0,
            max_held: MAX_HELD_TEXT,
            max_left_bytes: crate::moves::MAX_LEFT_BYTES,
            deadline: changes::deadline(),
            notes_left: crate::moves::MAX_CHECKED_NOTES,
            notes_spent: false,
            records: HashMap::new(),
            keep: None,
            keeps: false,
            max_below: MAX_SAVE_ENTRIES,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Merged {
    /// The merged root listing, in `packs` and `made`; meaningless when a conflict isn't kept.
    pub root: Id,
    /// Files merged line by line, in path order.
    pub merged_files: Vec<String>,
    /// The first 100 conflicts in path order (never more are kept), and how many more there were.
    pub conflicts: Vec<Conflict>,
    pub more: u64,
    /// Whose `opalro.toml`, if any, wasn't read whole and left a file, named or counted, in
    /// conflict only for that.
    pub rules_not_whole: Option<Side>,
    /// Where conflicts are kept, the places those rules would decide, which stop the combine:
    /// the first 100 in path order, and how many more.
    pub undecided: Vec<Conflict>,
    pub undecided_more: u64,
    /// Files whose comparison ran past its deadline.
    pub timed_out: Vec<String>,
    /// Sides where too many files left for moves to be followed by their contents.
    pub unfollowed: Vec<Side>,
    /// Whether, by the end of this merge, the combine's move notes were past what it checks.
    pub notes_unbelieved: bool,
    /// Ours' and theirs' checks for a conflict below a folder: their limits, held across the
    /// merge, and the folders walked already, each walked once however many places name it.
    below: [(SaveLimits, HashSet<Id>); 2],
}

/// The three versions' entries, or listings, in the order of `Which`.
type Three<T> = [T; 3];
pub(super) const WHICH: Three<Which> = [Which::Start, Which::Ours, Which::Theirs];

/// Merges `ours` and `theirs` against `start`, all root listings. `inner`: this merge makes a
/// made-up starting point, whose merged texts a later merge reads. `moved` gives a followed
/// file's name at the start, by where it is now.
#[allow(clippy::too_many_arguments)]
pub(crate) fn merge_trees(
    store: &Store,
    packs: &mut PackSet,
    made: &mut Made,
    inner: bool,
    start: Id,
    ours: Id,
    theirs: Id,
    rules: &RuleMatcher,
    moved: &BTreeMap<String, String>,
    deadline: Duration,
) -> Result<Merged, CombineError> {
    let mut merged = Merged {
        root: ours,
        merged_files: Vec::new(),
        conflicts: Vec::new(),
        more: 0,
        rules_not_whole: None,
        undecided: Vec::new(),
        undecided_more: 0,
        timed_out: Vec::new(),
        unfollowed: Vec::new(),
        notes_unbelieved: false,
        below: [(); 2].map(|()| {
            (
                SaveLimits::new(MAX_SAVE_DEPTH, made.max_below),
                HashSet::new(),
            )
        }),
    };
    if let Some(root) = one_sides([start, ours, theirs].map(Some)) {
        merged.root = root.unwrap_or(ours);
        return Ok(merged);
    }
    let mut walk = Walk {
        store,
        packs,
        made,
        inner,
        rules,
        moved,
        deadline,
        limits: [(); 3].map(|()| SaveLimits::default()),
        merged,
    };
    // What is left to do waits here, not in a call per folder level: a save may nest deeper than
    // a stack. Each folder's listing is built as the walk leaves it.
    let root = GoIn {
        ids: [Some(start), Some(ours), Some(theirs)],
        counted: [false; 3],
    };
    let mut folders = vec![walk.enter(String::new(), String::new(), 0, root)?];
    loop {
        let Some(here) = folders.last_mut() else {
            return Ok(walk.merged);
        };
        if let Some(next) = here.todo.pop() {
            let path = join(&here.path, &next.name);
            match next.into {
                Some(into) => {
                    let depth = here.depth + 1;
                    let folder = walk.enter(path, next.name, depth, into)?;
                    folders.push(folder);
                }
                None => {
                    if let Some(e) = walk.decide(&path, next.name, next.entries)? {
                        here.entries.push(e);
                    }
                }
            }
            continue;
        }
        let Some(done) = folders.pop() else {
            return Ok(walk.merged);
        };
        let (name, held_empty) = (done.name.clone(), done.held_empty);
        let (id, empty) = walk.leave(done)?;
        match folders.last_mut() {
            None => {
                walk.merged.root = id;
                return Ok(walk.merged);
            }
            // An empty folder only the merge made is no side's own.
            Some(up) if held_empty || !empty => up.entries.push(Entry {
                name,
                kind: EntryKind::Folder,
                id,
                size: 0,
            }),
            Some(_) => {}
        }
    }
}

/// The result when at most one side changed an entry: `Some(None)` for nothing there.
fn one_sides<T: Copy + PartialEq>([s, o, t]: Three<Option<T>>) -> Option<Option<T>> {
    if o == t || o == s {
        Some(t)
    } else if t == s {
        Some(o)
    } else {
        None
    }
}

/// What makes two entries the same version.
fn version(e: Option<&Entry>) -> Option<(EntryKind, Id)> {
    e.map(|e| (e.kind, e.id))
}

fn is_folder(e: Option<&Entry>) -> bool {
    e.is_some_and(|e| e.kind == EntryKind::Folder)
}

fn is_file(e: &Entry) -> bool {
    matches!(e.kind, EntryKind::File | EntryKind::Executable)
}

fn folder_id(e: Option<&Entry>) -> Option<Id> {
    e.filter(|e| e.kind == EntryKind::Folder).map(|e| e.id)
}

fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_owned()
    } else {
        format!("{folder}/{name}")
    }
}

/// A folder the walk goes into.
struct GoIn {
    /// Each version's listing there; `None` reads as empty.
    ids: Three<Option<Id>>,
    /// Each version counted whole already.
    counted: Three<bool>,
}

/// A name in a folder that the walk has still to work out.
struct Todo {
    name: String,
    entries: Three<Option<Entry>>,
    into: Option<GoIn>,
}

impl Todo {
    /// The bytes that place its paths among its folder's: a folder's are all `name/…`.
    fn key(&self) -> impl Iterator<Item = u8> + '_ {
        let slash = self.into.is_some().then_some(b'/');
        self.name.bytes().chain(slash)
    }
}

/// A folder being merged.
struct Folder {
    path: String,
    name: String,
    depth: usize,
    /// Both sides have the folder and one holds it empty, so it stays though nothing is left.
    held_empty: bool,
    /// Ours' and theirs' listings here, for the names check.
    ours: Listing,
    theirs: Listing,
    /// The merged entries so far.
    entries: Vec<Entry>,
    /// Names still to work out, the next in path order last.
    todo: Vec<Todo>,
}

struct Walk<'a> {
    store: &'a Store,
    packs: &'a mut PackSet,
    made: &'a mut Made,
    inner: bool,
    rules: &'a RuleMatcher,
    moved: &'a BTreeMap<String, String>,
    deadline: Duration,
    /// Each version is held to a save's limits on its own.
    limits: Three<SaveLimits>,
    merged: Merged,
}

impl Walk<'_> {
    /// Reads the three listings of the folder at `path` and works out what it can at once.
    fn enter(
        &mut self,
        path: String,
        name: String,
        depth: usize,
        into: GoIn,
    ) -> Result<Folder, CombineError> {
        let mut listings = [Listing::empty(), Listing::empty(), Listing::empty()];
        for (v, here) in listings.iter_mut().enumerate() {
            let Some(id) = into.ids[v] else { continue };
            self.limits[v].check_depth(depth).map_err(past(v))?;
            *here = listing(self.store, self.made, &id)?;
            if !into.counted[v] {
                self.limits[v].add(here.entries().len()).map_err(past(v))?;
            }
        }
        let [s, o, t] = &listings;
        let both = into.ids[1].is_some() && into.ids[2].is_some();
        let held_empty = both && (o.entries().is_empty() || t.entries().is_empty());
        let mut entries = Vec::new();
        let mut todo = Vec::new();
        for (name, three) in names(s, o, t) {
            if let Some(taken) = one_sides(three.map(version)) {
                if taken.is_some() {
                    let side = if version(three[2]) == taken { 2 } else { 1 };
                    entries.extend(three[side].cloned());
                }
                continue;
            }
            todo.push(Todo {
                name: name.to_owned(),
                into: going_into(three),
                entries: three.map(|e| e.cloned()),
            });
        }
        let at = if path.is_empty() { 0 } else { path.len() + 1 };
        // Each version meets its folders before going into any, so one met again is counted
        // whole before its paths are walked.
        let read = |id: &Id| listing(self.store, self.made, id);
        for t in &mut todo {
            let here = at + t.name.len();
            for (v, e) in t.entries.iter().enumerate() {
                if e.is_some() {
                    self.limits[v].check_path(here).map_err(past(v))?;
                }
            }
            let Some(next) = &mut t.into else { continue };
            for v in 0..3 {
                next.counted[v] = into.counted[v]
                    || again(&mut self.limits[v], next.ids[v], depth + 1, here, read)
                        .map_err(|e| limit(v, e))?;
            }
        }
        todo.sort_unstable_by(|a, b| b.key().cmp(a.key()));
        let [_, ours, theirs] = listings;
        Ok(Folder {
            path,
            name,
            depth,
            held_empty,
            ours,
            theirs,
            entries,
            todo,
        })
    }

    /// What labels are made from when this merge keeps conflicts; an inner merge never does.
    fn keeping(&self) -> Option<&Sides> {
        if self.inner {
            None
        } else {
            self.made.keep.as_ref()
        }
    }

    /// The merged entry for a name both sides changed: `None` for nothing there, or for a clash
    /// that isn't kept.
    fn decide(
        &mut self,
        path: &str,
        name: String,
        [s, o, t]: Three<Option<Entry>>,
    ) -> Result<Option<Entry>, CombineError> {
        let held = |e: &Option<Entry>| e.as_ref().is_some_and(|e| e.kind == EntryKind::Conflict);
        // A conflict is never made inside another one: the line holding it fixes it first.
        // Without the feature such an entry is no conflict to Opalro, as before.
        let on = match (held(&o), held(&t)) {
            _ if self.inner || !self.made.keeps => None,
            (true, true) => Some(Held::Both),
            (true, false) => Some(Held::Ours),
            (false, true) => Some(Held::Theirs),
            (false, false) => None,
        };
        if let Some(on) = on {
            return Err(CombineError::HeldConflict {
                names: Names::default(),
                path: path.to_owned(),
                at: path.to_owned(),
                on,
            });
        }
        if held(&s) && self.keeping().is_some() {
            // Each line changed it its own way since: what it held where they parted is unknown.
            return self.no_start(
                path,
                name,
                Clash::StartUnknown,
                [o, None, t],
                Parted::Unknown,
            );
        }
        let clash = match (&s, &o, &t) {
            (_, Some(o), Some(t))
                if (o.kind == EntryKind::Folder) != (t.kind == EntryKind::Folder) =>
            {
                let folder_on = if o.kind == EntryKind::Folder {
                    Side::Ours
                } else {
                    Side::Theirs
                };
                Clash::FileAndFolder { folder_on }
            }
            (_, Some(_), None) => Clash::ChangedRemoved {
                removed_on: Side::Theirs,
            },
            (_, None, Some(_)) => Clash::ChangedRemoved {
                removed_on: Side::Ours,
            },
            (None, Some(_), Some(_)) => Clash::BothAdded,
            (Some(s), Some(o), Some(t)) if is_file(s) && is_file(o) && is_file(t) => {
                return self.files(path, name, s, o, t);
            }
            _ => Clash::BothChanged(NotMerged::NotFiles),
        };
        if let (Clash::BothAdded, Some(sides)) = (&clash, self.keeping()) {
            let here = Parted::Nothing(match sides.parted {
                Parted::One(save) => Some(save),
                _ => None,
            });
            return self.no_start(path, name, clash, [o, None, t], here);
        }
        self.clashed(path, name, clash, [o, s, t])
    }

    /// A place with no starting version to merge from (added on both lines, or unknown): two
    /// text files are marked; anything else stands as yours.
    fn no_start(
        &mut self,
        path: &str,
        name: String,
        clash: Clash,
        terms: Three<Option<Entry>>,
        here: Parted,
    ) -> Result<Option<Entry>, CombineError> {
        let (o, t) = match &terms {
            // The same contents (the executable mark aside) are nothing for the rules to decide.
            [Some(o), _, Some(t)] if is_file(o) && is_file(t) && o.id != t.id => {
                (o.clone(), t.clone())
            }
            _ => return self.clashed(path, name, clash, terms),
        };
        match self.contents(path, None, &o, &t, here)? {
            Contents::Marked(bytes, marker, places) => {
                let own = self.store_text(path, &name, o.kind, &bytes)?;
                let own = Some((own, marker, places));
                self.record_at(path, name, clash, terms, own).map(Some)
            }
            Contents::Not(why @ NotMerged::RulesNotWhole { .. }) => {
                self.clashed(path, name, Clash::BothChanged(why), terms)
            }
            // Alike but for the executable mark, or not text: no markers.
            _ => self.clashed(path, name, clash, terms),
        }
    }

    /// Two files both sides changed, with a file at the start: the executable mark and the
    /// contents are merged apart.
    fn files(
        &mut self,
        path: &str,
        name: String,
        s: &Entry,
        o: &Entry,
        t: &Entry,
    ) -> Result<Option<Entry>, CombineError> {
        let kind = match one_sides([s, o, t].map(|e| Some(e.kind))) {
            Some(Some(kind)) => kind,
            _ => o.kind,
        };
        let terms = || [Some(o.clone()), Some(s.clone()), Some(t.clone())];
        let (id, size) = match one_sides([s, o, t].map(|e| Some(e.id))) {
            Some(Some(id)) if id == o.id => (o.id, o.size),
            Some(_) => (t.id, t.size),
            None => {
                let here = self.keeping().map_or(Parted::Several, |sides| sides.parted);
                match self.contents(path, Some(s), o, t, here)? {
                    Contents::Merged(bytes) => self.store_merged(path, bytes)?,
                    Contents::Marked(bytes, marker, places) => {
                        let own = self.store_text(path, &name, kind, &bytes)?;
                        let clash = Clash::BothChanged(NotMerged::Overlap {
                            endings_only: false,
                        });
                        let own = Some((own, marker, places));
                        return self.record_at(path, name, clash, terms(), own).map(Some);
                    }
                    Contents::Not(why) => {
                        return self.clashed(path, name, Clash::BothChanged(why), terms());
                    }
                }
            }
        };
        Ok(Some(Entry {
            name,
            kind,
            id,
            size,
        }))
    }

    /// Contents both sides changed (or added, with no `base`), merged line by line if the rules
    /// allow it; where conflicts are kept and the rules allow markers, a clash is marked.
    fn contents(
        &mut self,
        path: &str,
        base: Option<&Entry>,
        o: &Entry,
        t: &Entry,
        here: Parted,
    ) -> Result<Contents, CombineError> {
        let rule = self
            .rules
            .line_merge(path, self.moved.get(path).map(String::as_str));
        if let LineMerge::RuleSaysNo { theirs } = rule {
            return Ok(Contents::Not(NotMerged::NoMarkers { theirs }));
        }
        let labels = match rule {
            LineMerge::Yes => self.keeping().map(|sides| sides.labels(here)),
            _ => None,
        };
        let (text, late) = {
            let read = |e| bytes(self.store, &self.made.texts, e);
            let base = match base {
                Some(b) => read(b)?,
                None => Some(Cow::Borrowed(&b""[..])),
            };
            match (base, read(o)?, read(t)?) {
                (Some(s), Some(o), Some(t)) => {
                    merge_text(&s, &o, &t, self.deadline, labels.as_ref())
                }
                _ => (Text::Not(NotMerged::TooBig), false),
            }
        };
        if late {
            self.merged.timed_out.push(path.to_owned());
        }
        // Rules that can't be read whole stop a text they would decide: one that would merge,
        // and where conflicts are kept one that overlaps too. Any other keeps its own reason.
        let decided = match &text {
            Text::Merged(_) => true,
            Text::Not(NotMerged::Overlap { .. }) => self.keeping().is_some(),
            _ => false,
        };
        if let (LineMerge::RulesNotWhole { theirs }, true) = (rule, decided) {
            // One matcher serves the whole combine, so every such file names the same side.
            self.merged.rules_not_whole = Some(if theirs { Side::Theirs } else { Side::Ours });
            return Ok(Contents::Not(NotMerged::RulesNotWhole { theirs }));
        }
        Ok(match text {
            Text::Merged(bytes) => Contents::Merged(bytes),
            Text::Marked {
                bytes,
                marker,
                places,
            } => Contents::Marked(bytes, marker, places),
            Text::Not(why) => Contents::Not(why),
        })
    }

    /// Stores a text merged line by line, held for later merges when this one is inner.
    fn store_merged(&mut self, path: &str, bytes: Vec<u8>) -> Result<(Id, u64), CombineError> {
        let id = store_held(self.packs, self.store, &bytes).map_err(|e| ingest_failed(e, path))?;
        let size = bytes.len() as u64;
        self.merged.merged_files.push(path.to_owned());
        if self.inner && !self.made.texts.contains_key(&id) {
            let held = self.made.held.saturating_add(size);
            if held > self.made.max_held {
                return Err(CombineError::HeldTooMuch {
                    names: Names::default(),
                });
            }
            self.made.held = held;
            self.made.texts.insert(id, bytes);
        }
        Ok((id, size))
    }

    /// Stores a marked text as it came out of the merge: its size was checked where it was made.
    fn store_text(
        &mut self,
        path: &str,
        name: &str,
        kind: EntryKind,
        bytes: &[u8],
    ) -> Result<Entry, CombineError> {
        let id = store_held(self.packs, self.store, bytes).map_err(|e| ingest_failed(e, path))?;
        Ok(Entry {
            name: name.to_owned(),
            kind,
            id,
            size: bytes.len() as u64,
        })
    }

    /// A clash with no version of its own: kept as a conflict standing as yours (or nothing)
    /// where conflicts are kept, unless rules that can't be read whole would decide it; else
    /// named only.
    fn clashed(
        &mut self,
        path: &str,
        name: String,
        clash: Clash,
        terms: Three<Option<Entry>>,
    ) -> Result<Option<Entry>, CombineError> {
        if self.keeping().is_none() {
            self.conflict(path, clash, None);
            return Ok(None);
        }
        if matches!(clash, Clash::BothChanged(NotMerged::RulesNotWhole { .. })) {
            let undecided = Conflict {
                path: path.to_owned(),
                clash,
                stands: None,
            };
            let merged = &mut self.merged;
            name_in(&mut merged.undecided, &mut merged.undecided_more, undecided);
            return Ok(None);
        }
        self.record_at(path, name, clash, terms, None).map(Some)
    }

    /// [`record`] at `path`, as the entry `name` its folder holds.
    fn record_at(
        &mut self,
        path: &str,
        name: String,
        clash: Clash,
        terms: Three<Option<Entry>>,
        own: Option<(Entry, u8, u64)>,
    ) -> Result<Entry, CombineError> {
        let (store, made, merged) = (self.store, &mut *self.made, &mut self.merged);
        let id = record(store, self.packs, made, merged, path, clash, terms, own)?;
        Ok(Entry {
            name,
            kind: EntryKind::Conflict,
            id,
            size: 0,
        })
    }

    fn conflict(&mut self, path: &str, clash: Clash, stands: Option<Stands>) {
        named(&mut self.merged, path, clash, stands);
    }

    /// Builds and stores a merged folder's listing once all of it is worked out: its ID, and
    /// whether it is empty.
    fn leave(&mut self, folder: Folder) -> Result<(Id, bool), CombineError> {
        if folder.entries.len() as u64 > MAX_ENTRIES {
            return Err(CombineError::TooBig {
                detail: "a folder in it would hold more than 1,000,000 entries".to_owned(),
            });
        }
        if let Some(problem) = unheld_clash(&folder) {
            return Err(CombineError::NamesClash {
                names: Names::default(),
                folder: folder.path,
                problem: Box::new(problem),
            });
        }
        let listing = Listing::new(folder.entries).map_err(StoreError::from)?;
        let id = store_listing(self.store, self.packs, &listing)?;
        let empty = listing.entries().is_empty();
        self.made.listings.insert(id, listing);
        Ok((id, empty))
    }
}

/// What merging a file's contents gave.
enum Contents {
    Merged(Vec<u8>),
    /// The text with markers, their length, and the places marked.
    Marked(Vec<u8>, u8, u64),
    Not(NotMerged),
}

/// Makes the record of a conflict at `path`: `terms` in the record's order (yours, where the
/// lines parted, theirs), its own version if any (with its marker length and marked places),
/// into the pack set and `made.records`; checks first that no folder among yours and theirs
/// holds a conflict below it (`HeldConflict`); names the conflict with how it stands. Gives the
/// record's ID.
#[allow(clippy::too_many_arguments)]
pub(super) fn record(
    store: &Store,
    packs: &mut PackSet,
    made: &mut Made,
    merged: &mut Merged,
    path: &str,
    clash: Clash,
    terms: Three<Option<Entry>>,
    own: Option<(Entry, u8, u64)>,
) -> Result<Id, CombineError> {
    for (term, v, on) in [(&terms[0], 1, Held::Ours), (&terms[2], 2, Held::Theirs)] {
        let Some(folder) = folder_id(term.as_ref()) else {
            continue;
        };
        let below = &mut merged.below[v - 1];
        if let Some(at) = conflict_below(store, made, below, path, folder, v)? {
            return Err(CombineError::HeldConflict {
                names: Names::default(),
                path: path.to_owned(),
                at,
                on,
            });
        }
    }
    let term = |e: &Entry| Term {
        kind: e.kind,
        id: e.id,
        size: e.size,
    };
    let record = Record {
        terms: terms.iter().map(|e| e.as_ref().map(term)).collect(),
        own: own.as_ref().map(|(e, marker, _)| Own {
            version: Some(term(e)),
            marker: *marker,
        }),
    };
    let stands = match (&own, &terms[0]) {
        (Some((.., places)), _) => Stands::Markers { places: *places },
        (None, Some(_)) => Stands::Yours,
        (None, None) => Stands::Nothing,
    };
    let id = store_record(store, packs, &record)?;
    made.records.insert(id, record);
    named(merged, path, clash, Some(stands));
    Ok(id)
}

/// Names the conflict at `path` among the merge's.
pub(super) fn named(merged: &mut Merged, path: &str, clash: Clash, stands: Option<Stands>) {
    let conflict = Conflict {
        path: path.to_owned(),
        clash,
        stands,
    };
    name_in(&mut merged.conflicts, &mut merged.more, conflict);
}

/// The first conflict found in the folder listing `id` at `path` of version `v`, or below it
/// (`v` 1 for ours, 2 for theirs): only folders not walked before are read, all of them together
/// held to that version's limits.
fn conflict_below(
    store: &Store,
    made: &Made,
    (limits, seen): &mut (SaveLimits, HashSet<Id>),
    path: &str,
    id: Id,
    v: usize,
) -> Result<Option<String>, CombineError> {
    let mut todo = vec![(path.to_owned(), id, path.split('/').count())];
    while let Some((at, id, depth)) = todo.pop() {
        // A listing met again holds what it held the first time; a conflict found in one stops
        // the combine, so every folder in `seen` holds none.
        if !seen.insert(id) {
            continue;
        }
        limits.check_depth(depth).map_err(past(v))?;
        let here = listing(store, made, &id)?;
        limits.add(here.entries().len()).map_err(past(v))?;
        if let Some(e) = here
            .entries()
            .iter()
            .find(|e| e.kind == EntryKind::Conflict)
        {
            return Ok(Some(join(&at, &e.name)));
        }
        let folders = here.entries().iter().rev();
        todo.extend(
            folders
                .filter(|e| e.kind == EntryKind::Folder)
                .map(|e| (join(&at, &e.name), e.id, depth + 1)),
        );
    }
    Ok(None)
}

/// Names `c` in `list`, the first `MAX_NAMED`, else counts it in `more`.
fn name_in(list: &mut Vec<Conflict>, more: &mut u64, c: Conflict) {
    if list.len() < MAX_NAMED {
        list.push(c);
    } else {
        *more += 1;
    }
}

fn is_clash(p: &PortabilityProblem) -> bool {
    matches!(
        p,
        PortabilityProblem::CaseCollision { .. }
            | PortabilityProblem::NormalizationCollision { .. }
    )
}

/// A pair of alike names in a merged folder that neither ours' folder nor theirs' holds both of;
/// a pair one side already held, as an imported history can, is left as it is. Every merged name
/// is in ours' folder or theirs', so a group of alike names has such a pair exactly when one name
/// is only in ours' and another only in theirs'.
fn unheld_clash(folder: &Folder) -> Option<PortabilityProblem> {
    let names: Vec<&str> = folder.entries.iter().map(|e| e.name.as_str()).collect();
    // Each name alike another, with its group's first name. A pair names an earlier name first,
    // so that one's group is known by then.
    let mut group: HashMap<String, String> = HashMap::new();
    for p in folder_problems(&names) {
        let (PortabilityProblem::CaseCollision { a, b }
        | PortabilityProblem::NormalizationCollision { a, b }) = p
        else {
            continue;
        };
        let first = group.get(&a).cloned().unwrap_or_else(|| a.clone());
        group.insert(a, first.clone());
        group.insert(b, first);
    }
    // Per group, the first name only ours' folder holds and the first only theirs' does.
    let mut only: HashMap<&str, (Option<&str>, Option<&str>)> = HashMap::new();
    for &name in &names {
        let Some(first) = group.get(name) else {
            continue;
        };
        let (o, t) = (
            folder.ours.get(name).is_some(),
            folder.theirs.get(name).is_some(),
        );
        let seen = only.entry(first.as_str()).or_default();
        if o && !t {
            seen.0.get_or_insert(name);
        }
        if t && !o {
            seen.1.get_or_insert(name);
        }
        if let (Some(a), Some(b)) = *seen {
            return folder_problems(&[a, b]).into_iter().find(is_clash);
        }
    }
    None
}

/// The folder a name both sides changed is merged as, if it is one: two folders, or a folder
/// against a removal where the start has the folder.
fn going_into([s, o, t]: Three<Option<&Entry>>) -> Option<GoIn> {
    let both = is_folder(o) && is_folder(t);
    let against_removal =
        is_folder(s) && ((is_folder(o) && t.is_none()) || (o.is_none() && is_folder(t)));
    if !both && !against_removal {
        return None;
    }
    Some(GoIn {
        // The start's folder only if it is one: else the folders merge from nothing.
        ids: [folder_id(s), folder_id(o), folder_id(t)],
        counted: [false; 3],
    })
}

/// Every name of three listings, with each one's entry, in name order.
fn names<'a>(
    s: &'a Listing,
    o: &'a Listing,
    t: &'a Listing,
) -> impl Iterator<Item = (&'a str, Three<Option<&'a Entry>>)> {
    let mut at = [s.entries(), o.entries(), t.entries()].map(|e| e.iter().peekable());
    std::iter::from_fn(move || {
        let name = at
            .iter_mut()
            .filter_map(|it| it.peek().copied().map(|e| e.name.as_str()))
            .min_by(|a, b| a.as_bytes().cmp(b.as_bytes()))?;
        let three = at
            .each_mut()
            .map(|it| it.next_if(|e| e.name.as_bytes().cmp(name.as_bytes()) == Ordering::Equal));
        Some((name, three))
    })
}

fn listing(store: &Store, made: &Made, id: &Id) -> Result<Listing, StoreError> {
    match made.listings.get(id) {
        Some(l) => Ok(l.clone()),
        None => store.listing(id),
    }
}

/// A file's contents, from what this combine made or from the store; `None` past the size
/// merged line by line.
fn bytes<'a>(
    store: &Store,
    texts: &'a HashMap<Id, Vec<u8>>,
    e: &Entry,
) -> Result<Option<Cow<'a, [u8]>>, CombineError> {
    if let Some(text) = texts.get(&e.id) {
        return Ok(Some(Cow::Borrowed(text)));
    }
    Ok(
        match stored(store, Some(e.kind), Some(e.id), Some(e.size))? {
            changes::Side::Bytes(b) => Some(Cow::Owned(b)),
            changes::Side::TooBig => None,
        },
    )
}

fn past(v: usize) -> impl Fn(String) -> CombineError {
    move |detail| CombineError::Past {
        names: Names::default(),
        which: WHICH[v],
        detail,
    }
}

/// A version's limit met while counting a folder whole, or the store's error reading it.
fn limit(v: usize, e: StoreError) -> CombineError {
    match e {
        StoreError::SaveTooLarge { detail } => past(v)(detail),
        e => e.into(),
    }
}
