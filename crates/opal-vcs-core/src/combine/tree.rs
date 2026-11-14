//! The merge of two versions of the project against their starting point, entry by entry, one
//! folder at a time.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::time::Duration;

use super::{Clash, CombineError, Conflict, NotMerged, Side, Text, Which, merge_text};
use crate::changes::{self, stored};
use crate::diff::again;
use crate::id::Id;
use crate::ingest::store_held;
use crate::listing::{Entry, EntryKind, Listing, MAX_ENTRIES, SaveLimits};
use crate::names::{PortabilityProblem, folder_problems};
use crate::pack::PackSet;
use crate::rules::RuleMatcher;
use crate::snapshot::{ingest_failed, store_listing};
use crate::store::{Store, StoreError};

/// The most text merged by inner combines that one combine holds in memory.
pub const MAX_HELD_TEXT: u64 = 256 << 20;

/// Conflicts named; the rest are only counted.
const MAX_NAMED: usize = 100;

/// What a combine has built that a later merge of the same combine may read, before anything
/// is stored. Every object it built is in the combine's `PackSet` too.
#[derive(Debug)]
pub struct Made {
    pub listings: HashMap<Id, Listing>,
    /// Texts merged by inner combines only: a made-up version a later merge reads holds them.
    pub texts: HashMap<Id, Vec<u8>>,
    /// Their bytes, and the most allowed (`MAX_HELD_TEXT`; tests lower it).
    pub held: u64,
    pub max_held: u64,
}

impl Default for Made {
    fn default() -> Made {
        Made {
            listings: HashMap::new(),
            texts: HashMap::new(),
            held: 0,
            max_held: MAX_HELD_TEXT,
        }
    }
}

#[derive(Debug)]
pub struct Merged {
    /// The merged root listing, in `packs` and `made`; meaningless when there are conflicts.
    pub root: Id,
    /// Files merged line by line, in path order.
    pub merged_files: Vec<String>,
    /// The first 100 conflicts in path order (never more are kept), and how many more there were.
    pub conflicts: Vec<Conflict>,
    pub more: u64,
    /// Files whose comparison ran past its deadline.
    pub timed_out: Vec<String>,
}

/// The three versions' entries, or listings, in the order of `Which`.
type Three<T> = [T; 3];
const WHICH: Three<Which> = [Which::Start, Which::Ours, Which::Theirs];

/// Merges `ours` and `theirs` against `start`, all root listings. `inner`: this merge makes a
/// made-up starting point, whose merged texts a later merge reads.
#[allow(clippy::too_many_arguments)]
pub fn merge_trees(
    store: &Store,
    packs: &mut PackSet,
    made: &mut Made,
    inner: bool,
    start: Id,
    ours: Id,
    theirs: Id,
    rules: &RuleMatcher,
    deadline: Duration,
) -> Result<Merged, CombineError> {
    let mut merged = Merged {
        root: ours,
        merged_files: Vec::new(),
        conflicts: Vec::new(),
        more: 0,
        timed_out: Vec::new(),
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
        deadline,
        limits: [(); 3].map(|()| SaveLimits::default()),
        merged,
    };
    // What is left to do waits here, not in a call per folder level: a save may nest deeper than
    // a stack. Each folder's listing is built as the walk leaves it.
    let root = GoIn {
        ids: [Some(start), Some(ours), Some(theirs)],
        counted: [false; 3],
        keep_empty: true,
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
        let (keep_empty, name) = (done.keep_empty, done.name.clone());
        let (id, empty) = walk.leave(done)?;
        match folders.last_mut() {
            None => {
                walk.merged.root = id;
                return Ok(walk.merged);
            }
            Some(up) if keep_empty || !empty => up.entries.push(Entry {
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
    /// Both sides have the folder, so it stays though nothing is left in it.
    keep_empty: bool,
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
    keep_empty: bool,
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
            keep_empty: into.keep_empty,
            ours,
            theirs,
            entries,
            todo,
        })
    }

    /// The merged entry for a name both sides changed, unless it is a conflict.
    fn decide(
        &mut self,
        path: &str,
        name: String,
        [s, o, t]: Three<Option<Entry>>,
    ) -> Result<Option<Entry>, CombineError> {
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
        self.conflict(path, clash);
        Ok(None)
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
        let (id, size) = match one_sides([s, o, t].map(|e| Some(e.id))) {
            Some(Some(id)) if id == o.id => (o.id, o.size),
            Some(_) => (t.id, t.size),
            None => match self.contents(path, s, o, t)? {
                Some(merged) => merged,
                None => return Ok(None),
            },
        };
        Ok(Some(Entry {
            name,
            kind,
            id,
            size,
        }))
    }

    /// Contents both sides changed, merged line by line if they can be.
    fn contents(
        &mut self,
        path: &str,
        s: &Entry,
        o: &Entry,
        t: &Entry,
    ) -> Result<Option<(Id, u64)>, CombineError> {
        if !self.rules.conflict_markers(path) {
            self.conflict(path, Clash::BothChanged(NotMerged::NoMarkers));
            return Ok(None);
        }
        let (text, late) = {
            let read = |e| bytes(self.store, &self.made.texts, e);
            match (read(s)?, read(o)?, read(t)?) {
                (Some(s), Some(o), Some(t)) => merge_text(&s, &o, &t, self.deadline),
                _ => (Text::Not(NotMerged::TooBig), false),
            }
        };
        if late {
            self.merged.timed_out.push(path.to_owned());
        }
        let bytes = match text {
            Text::Merged(bytes) => bytes,
            Text::Not(why) => {
                self.conflict(path, Clash::BothChanged(why));
                return Ok(None);
            }
        };
        let id = store_held(self.packs, self.store, &bytes).map_err(|e| ingest_failed(e, path))?;
        let size = bytes.len() as u64;
        self.merged.merged_files.push(path.to_owned());
        if self.inner && !self.made.texts.contains_key(&id) {
            let held = self.made.held.saturating_add(size);
            if held > self.made.max_held {
                return Err(CombineError::HeldTooMuch);
            }
            self.made.held = held;
            self.made.texts.insert(id, bytes);
        }
        Ok(Some((id, size)))
    }

    fn conflict(&mut self, path: &str, clash: Clash) {
        if self.merged.conflicts.len() < MAX_NAMED {
            self.merged.conflicts.push(Conflict {
                path: path.to_owned(),
                clash,
            });
        } else {
            self.merged.more += 1;
        }
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
                folder: folder.path,
                problem,
            });
        }
        let listing = Listing::new(folder.entries).map_err(StoreError::from)?;
        let id = store_listing(self.store, self.packs, &listing)?;
        let empty = listing.entries().is_empty();
        self.made.listings.insert(id, listing);
        Ok((id, empty))
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
    let ids = [folder_id(s), folder_id(o), folder_id(t)];
    let keep_empty = if is_folder(o) && is_folder(t) {
        true
    } else if is_folder(s) && ((is_folder(o) && t.is_none()) || (o.is_none() && is_folder(t))) {
        false
    } else {
        return None;
    };
    Some(GoIn {
        // The start's folder only if it is one: else the folders merge from nothing.
        ids,
        counted: [false; 3],
        keep_empty,
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
