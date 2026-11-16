//! Line history, short change IDs, and finding a save from what the user typed. A line can be
//! any length and a message 1 MiB, so history is read one save at a time and never held.

use std::collections::HashSet;

use crate::changeids::ChangeIds;
use crate::diary::{DiaryEntry, Position, TagList};
use crate::id::{ChangeId, Id, is_hex_start};
use crate::lines::Target;
use crate::repo::{RepoError, Repository};
use crate::save::Save;
use crate::store::is_unreadable_record;
use crate::text::escape;

/// A short change ID has at least this many letters.
const MIN_SHORT: usize = 4;
/// The letters in a whole change ID.
const LETTERS: usize = 2 * size_of::<ChangeId>();

/// What the user typed comes back in the message, escaped. The candidates are IDs Opalro made.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("No save, line or ID matches \"{}\".", escape(.0, false))]
    NotFound(String),
    #[error(
        "\"{}\" matches more than one save: {}.",
        escape(text, false),
        candidates.join(", ")
    )]
    Ambiguous {
        text: String,
        candidates: Vec<String>,
    },
    #[error(
        "\"{}\" is the name of a line and also the start of another save's ID ({}), so Opalro \
         didn't pick either.",
        escape(text, false),
        saves.join(", ")
    )]
    LineOrSave { text: String, saves: Vec<String> },
    #[error(
        "\"{}\" is the name of a tag and also the start of another save's ID ({}), so Opalro \
         didn't pick either.",
        escape(text, false),
        saves.join(", ")
    )]
    TagOrSave { text: String, saves: Vec<String> },
    /// `line` and `tag` are the two saves' first 12 hex digits.
    #[error(
        "\"{}\" is the name of a line and also of a tag at another save (line {line}, tag {tag}), \
         so Opalro didn't pick either.",
        escape(text, false)
    )]
    LineOrTag {
        text: String,
        line: String,
        tag: String,
    },
    #[error(
        "\"{}\" is the name of a server line and also of a tag at another save (server line \
         {line}, tag {tag}), so Opalro didn't pick either.",
        escape(text, false)
    )]
    ServerLineOrTag {
        text: String,
        line: String,
        tag: String,
    },
    #[error(transparent)]
    Repo(#[from] RepoError),
}

impl ResolveError {
    /// What the user can do next, if there is something to say.
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        match self {
            ResolveError::NotFound(_) => Some("See your saves with: opalro history".to_owned()),
            ResolveError::Ambiguous { .. } => Some("Type more of the ID to pick one.".to_owned()),
            ResolveError::LineOrSave { .. } => Some(
                "To reach the save, type more of its ID; opalro history --json gives the whole \
                 change ID and fingerprint. To reach the line by another name: opalro line \
                 rename <old> <new>"
                    .to_owned(),
            ),
            ResolveError::TagOrSave { .. } => Some(
                "To reach the save, type more of its ID; opalro history --json gives the whole \
                 change ID and fingerprint. To reach the tag's save, take its ID from: opalro tag"
                    .to_owned(),
            ),
            ResolveError::LineOrTag { .. } => Some(
                "To reach one of them, type its ID as shown here. To end the double meaning, \
                 rename the line (opalro line rename <old> <new>) or delete the tag (opalro tag \
                 --delete <name>)."
                    .to_owned(),
            ),
            ResolveError::ServerLineOrTag { .. } => Some(
                "To reach one of them, type its ID as shown here. To end the double meaning, \
                 delete the tag: opalro tag --delete <name>"
                    .to_owned(),
            ),
            ResolveError::Repo(r) => r.hint(),
        }
    }
}

impl From<crate::store::StoreError> for ResolveError {
    fn from(e: crate::store::StoreError) -> Self {
        RepoError::from(e).into()
    }
}

/// The newest save of the current line, or the checked-out save.
pub(crate) fn tip_of(entry: &DiaryEntry) -> Result<Id, RepoError> {
    match &entry.state.position {
        Position::OnLine(name) => {
            entry
                .state
                .lines
                .get(name)
                .copied()
                .ok_or_else(|| RepoError::Damaged {
                    detail: format!("the current line {name} does not exist"),
                })
        }
        Position::AtSave(id) => Ok(*id),
    }
}

/// The current line, or `None` at a save.
fn line_of(entry: &DiaryEntry) -> Option<&str> {
    match &entry.state.position {
        Position::OnLine(name) => Some(name),
        Position::AtSave(_) => None,
    }
}

/// The save the user is on: the current line's newest save, or the checked-out save.
pub fn current_save(repo: &Repository) -> Result<Id, RepoError> {
    tip_of(&repo.current()?.1)
}

/// How many leading letters two change IDs share.
fn common_letters(a: &ChangeId, b: &ChangeId) -> usize {
    let bytes = a.0.iter().zip(&b.0).take_while(|(x, y)| x == y).count();
    let next_letter = a.0.get(bytes).zip(b.0.get(bytes));
    2 * bytes + usize::from(next_letter.is_some_and(|(x, y)| x >> 4 == y >> 4))
}

/// The letters of a change ID that tell it from every other, when the closest one shares `common`.
fn short_len(common: usize) -> usize {
    (common + 1).clamp(MIN_SHORT, LETTERS)
}

/// For each change ID, how many of its letters tell it from all the others.
fn short_lengths(ids: &[ChangeId]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..ids.len()).collect();
    order.sort_by_key(|&i| ids[i]);
    // Whoever shares the longest start with an ID is next to it in the sorted order.
    let mut common = vec![0; ids.len()];
    for (&a, &b) in order.iter().zip(order.iter().skip(1)) {
        let shared = common_letters(&ids[a], &ids[b]);
        common[a] = common[a].max(shared);
        common[b] = common[b].max(shared);
    }
    common.into_iter().map(short_len).collect()
}

/// The shortest form of `change_id` among the sorted `ids`, leaving one copy of it out when
/// `itself` (it is the change ID of one of their saves).
fn short_among(ids: &[ChangeId], change_id: &ChangeId, itself: bool) -> String {
    let at = ids.partition_point(|c| c < change_id);
    let (below, mut above) = ids.split_at(at);
    if itself && above.first() == Some(change_id) {
        above = &above[1..];
    }
    // The IDs sharing the longest start with it are its neighbours in the sorted order.
    let common = [below.last(), above.first()]
        .into_iter()
        .flatten()
        .map(|other| common_letters(change_id, other))
        .max()
        .unwrap_or(0);
    change_id.to_letters()[..short_len(common)].to_owned()
}

/// The shortest form of `change_id`, the change ID of the save `id`, that no other of `saves`
/// starts with.
pub(crate) fn short_in(saves: &[(Id, ChangeId)], id: &Id, change_id: &ChangeId) -> String {
    let common = saves
        .iter()
        .filter(|(other, _)| other != id)
        .map(|(_, other)| common_letters(change_id, other))
        .max()
        .unwrap_or(0);
    change_id.to_letters()[..short_len(common)].to_owned()
}

/// The shortest form of `change_id` (at least 4 letters) that no other save on the current line
/// starts with. `id` is the save it belongs to, on the line or not.
pub fn short_change_id(
    repo: &Repository,
    id: &Id,
    change_id: &ChangeId,
) -> Result<String, RepoError> {
    let (_, entry) = repo.current()?;
    let mut cache = ChangeIds::load(repo);
    let short = short_in(
        cache.chain(repo, line_of(&entry), tip_of(&entry)?)?,
        id,
        change_id,
    );
    cache.store(repo, &entry.state);
    Ok(short)
}

/// The current line's saves and their change IDs, read once and kept as IDs alone, to shorten
/// any number of change IDs as [`short_change_id`] shortens one.
pub struct LineIds {
    /// Sorted.
    change_ids: Vec<ChangeId>,
    saves: HashSet<Id>,
}

impl LineIds {
    /// Reads the current line, or the saves back from the one the project is at.
    pub fn read(repo: &Repository) -> Result<LineIds, RepoError> {
        let (_, entry) = repo.current()?;
        let mut cache = ChangeIds::load(repo);
        let line = cache.chain(repo, line_of(&entry), tip_of(&entry)?)?;
        let mut change_ids: Vec<ChangeId> = line.iter().map(|(_, c)| *c).collect();
        let saves = line.iter().map(|(id, _)| *id).collect();
        cache.store(repo, &entry.state);
        change_ids.sort_unstable();
        Ok(LineIds { change_ids, saves })
    }

    /// The shortest form of `change_id`, the change ID of the save `id`, that no other save on
    /// the line starts with.
    #[must_use]
    pub fn short(&self, id: &Id, change_id: &ChangeId) -> String {
        short_among(&self.change_ids, change_id, self.saves.contains(id))
    }

    /// Whether another save on the line has `change_id`, the change ID of the save `id`, so that
    /// no form of it tells them apart: an amended save does, once the amend is undone.
    #[must_use]
    pub fn shared(&self, id: &Id, change_id: &ChangeId) -> bool {
        let at = self.change_ids.partition_point(|c| c < change_id);
        let copies = self.change_ids[at..]
            .iter()
            .take_while(|c| *c == change_id)
            .count();
        copies > usize::from(self.saves.contains(id))
    }
}

/// Whether the first `count` saves `saves` lists are what the list says, read from the store:
/// each with the list's change ID, and as its first parent the next save listed, or none after
/// the last. `each` is handed each save that passes. An error for a save the store can't give.
fn links_hold(
    repo: &Repository,
    saves: &[(Id, ChangeId)],
    count: usize,
    mut each: impl FnMut(Save),
) -> Result<bool, crate::store::StoreError> {
    for (at, (id, change_id)) in saves.iter().enumerate().take(count) {
        let save = repo.store().save(id, repo.signatures_allowed())?;
        let next = saves.get(at + 1).map(|(next, _)| next);
        if save.change_id != *change_id || save.parents.first() != next {
            return Ok(false);
        }
        each(save);
    }
    Ok(true)
}

/// Saves from `start` back along first parents, newest first, each read when asked for. A cycle
/// can't be built (a save's fingerprint covers its parents), but a store missing a parent fails
/// with an error, which ends the walk.
fn chain(repo: &Repository, start: Id) -> impl Iterator<Item = Result<(Id, Save), RepoError>> + '_ {
    let mut next = Some(start);
    std::iter::from_fn(move || {
        let id = next.take()?;
        Some(match repo.store().save(&id, repo.signatures_allowed()) {
            Ok(save) => {
                next = save.parents.first().copied();
                Ok((id, save))
            }
            Err(e) => Err(e.into()),
        })
    })
}

/// How many letters of each change ID of `saves` tell it from the others, in their order.
fn lengths_of(saves: &[(Id, ChangeId)]) -> Vec<usize> {
    short_lengths(&saves.iter().map(|(_, c)| *c).collect::<Vec<_>>())
}

/// Calls `each` with every save on the current line, newest first, and the shortest form of its
/// change ID. The saves come from their own first parents, read one at a time, so that only the
/// save being handed out is held; the short forms come from the cache's list of the line, or from
/// the line itself when that list is wrong.
pub fn for_each_on_line<E: From<RepoError>>(
    repo: &Repository,
    mut each: impl FnMut(&Id, &Save, &str) -> Result<(), E>,
) -> Result<(), E> {
    let (_, entry) = repo.current()?;
    let (line, tip) = (line_of(&entry), tip_of(&entry)?);
    let mut cache = ChangeIds::load(repo);
    cache.chain(repo, line, tip)?;
    // Kept before the saves are handed out, however that ends; asked again, it reads nothing.
    cache.store(repo, &entry.state);
    let listed = cache.chain(repo, line, tip)?;
    let lengths = lengths_of(listed);
    let short = |lengths: &[usize], at: usize, save: &Save| {
        let len = lengths.get(at).copied().unwrap_or(LETTERS);
        save.change_id.to_letters()[..len].to_owned()
    };
    let mut walk = chain(repo, tip).enumerate();
    let mut wrong = None;
    let mut count = 0;
    for (at, step) in walk.by_ref() {
        let (id, save) = step?;
        count = at + 1;
        if listed.get(at) != Some(&(id, save.change_id)) {
            wrong = Some((at, id, save));
            break;
        }
        each(&id, &save, &short(&lengths, at, &save))?;
    }
    if wrong.is_none() && count == listed.len() {
        return Ok(());
    }
    // The list isn't the line: it is dropped, the line read from the store, and the listing goes
    // on with the line's own short forms.
    let mut read = ChangeIds::default();
    let lengths = lengths_of(read.chain(repo, line, tip)?);
    read.store(repo, &entry.state);
    if let Some((at, id, save)) = wrong {
        each(&id, &save, &short(&lengths, at, &save))?;
        for (at, step) in walk {
            let (id, save) = step?;
            each(&id, &save, &short(&lengths, at, &save))?;
        }
    }
    Ok(())
}

/// How many letters of a change ID a listing from another place shows: shortening them to be
/// unique would mean reading the whole list first.
const FROM_LETTERS: usize = 12;

/// Calls `each` with every save from `tip` back along first parents, newest first, each with
/// the first 12 letters of its change ID. Nothing is cached.
pub fn for_each_from<E: From<RepoError>>(
    repo: &Repository,
    tip: Id,
    mut each: impl FnMut(&Id, &Save, &str) -> Result<(), E>,
) -> Result<(), E> {
    for step in chain(repo, tip) {
        let (id, save) = step?;
        each(&id, &save, &save.change_id.to_letters()[..FROM_LETTERS])?;
    }
    Ok(())
}

fn one_of(text: &str, mut found: Vec<(Id, String)>) -> Result<Id, ResolveError> {
    found.sort_by(|a, b| a.1.cmp(&b.1));
    found.dedup();
    match found.as_slice() {
        [] => Err(ResolveError::NotFound(text.to_owned())),
        [(id, _)] => Ok(*id),
        _ => Err(ResolveError::Ambiguous {
            text: text.to_owned(),
            candidates: found.into_iter().map(|(_, c)| c).collect(),
        }),
    }
}

/// The saves of `saves` whose change ID starts with `text`, each with its change ID.
fn starting_with(saves: &[(Id, ChangeId)], text: &str) -> Vec<(Id, String)> {
    saves
        .iter()
        .filter_map(|(id, change_id)| {
            let letters = change_id.to_letters();
            letters.starts_with(text).then_some((*id, letters))
        })
        .collect()
}

/// The saves on the current line whose change ID starts with `text`, each with its change ID.
fn by_change_prefix(
    repo: &Repository,
    entry: &DiaryEntry,
    text: &str,
) -> Result<Vec<(Id, String)>, ResolveError> {
    let (line, tip) = (line_of(entry), tip_of(entry)?);
    let mut cache = ChangeIds::load(repo);
    let saves = cache.chain(repo, line, tip)?;
    let listed = starting_with(saves, text);
    // The one save the list names is shown to be on the line, link by link from its newest save;
    // no save or several, from the store. That the list hides no second save of the line with
    // this start is trusted, as the short IDs Opalro prints trust it: showing it reads the line.
    let checked = match listed.as_slice() {
        [(id, _)] => Some(
            saves
                .iter()
                .position(|(s, _)| s == id)
                .is_some_and(|at| matches!(links_hold(repo, saves, at + 1, |_| {}), Ok(true))),
        ),
        _ => None,
    };
    if checked == Some(true) {
        cache.store(repo, &entry.state);
        return Ok(listed);
    }
    let mut read = ChangeIds::default();
    let found = starting_with(read.chain(repo, line, tip)?, text);
    // A list the check found wrong is dropped even where it gave the right answer.
    if checked == Some(false) || found != listed {
        cache = read;
    }
    cache.store(repo, &entry.state);
    Ok(found)
}

/// Calls `visit` once for each save reachable from `tips` along first parents and not in `seen`,
/// adding them to it. A chain an earlier tip walked is not walked again, so the cost doesn't grow
/// with the number of tips.
fn each_once<E>(
    seen: &mut HashSet<Id>,
    tips: impl IntoIterator<Item = Id>,
    mut first_parent: impl FnMut(&Id) -> Result<Option<Id>, E>,
    mut visit: impl FnMut(&Id),
) -> Result<(), E> {
    for tip in tips {
        let mut next = Some(tip);
        while let Some(id) = next {
            if !seen.insert(id) {
                break;
            }
            visit(&id);
            next = first_parent(&id)?;
        }
    }
    Ok(())
}

/// The saves, among those any line, server line or tag reaches by first parents, whose
/// fingerprint starts with `text`, each with its fingerprint. `tags` are the project's.
fn by_fingerprint_prefix(
    repo: &Repository,
    entry: &DiaryEntry,
    tags: &TagList,
    text: &str,
) -> Result<Vec<(Id, String)>, ResolveError> {
    let text = text.to_ascii_lowercase();
    if let Some(id) = Id::from_hex(&text) {
        // A line the cache knows at its newest save may hold it; the current one first. It is
        // shown to be on the line, link by link from its newest save, before it is believed.
        let mut cache = ChangeIds::load(repo);
        if let Some(line) = line_of(entry) {
            cache.chain(repo, Some(line), tip_of(entry)?)?;
        }
        let checked = cache.reaches(&entry.state, &id).map(|(saves, at)| {
            matches!(
                links_hold(repo, saves, at.saturating_add(1), |_| {}),
                Ok(true)
            )
        });
        if checked == Some(false) {
            // The lists are dropped; on a line, its own, read from the store as a handle with
            // no cache reads it, is kept. At a save nothing more is read.
            cache = ChangeIds::dropped();
            if let Some(line) = line_of(entry) {
                cache.chain(repo, Some(line), tip_of(entry)?)?;
            }
        }
        cache.store(repo, &entry.state);
        if checked == Some(true) {
            return Ok(vec![(id, text)]);
        }
    }
    let mut found = Vec::new();
    let mut visit = |id: &Id| {
        let hex = id.to_hex();
        if hex.starts_with(&text) {
            found.push((*id, hex));
        }
    };
    let parent = |id: &Id| {
        let save = repo.store().save(id, repo.signatures_allowed());
        save.map(|save| save.parents.first().copied())
    };
    let mut seen = HashSet::new();
    each_once(
        &mut seen,
        entry.state.lines.values().copied(),
        |id| Ok::<_, RepoError>(parent(id)?),
        &mut visit,
    )?;
    // A server line's or tag's save that can't be read ends its chain there: it can still be
    // named, and it never stops a search that the lines alone would answer.
    let servers = repo.server_lines(&entry.state)?;
    let more = (servers.servers.values())
        .flat_map(|lines| lines.values().copied())
        .chain(tags.tags.values().map(|tag| tag.save));
    let damage_ends = |id: &Id| match parent(id) {
        Err(e) if is_unreadable_record(&e) => Ok(None),
        read => Ok::<_, RepoError>(read?),
    };
    each_once(&mut seen, more, damage_ends, &mut visit)?;
    Ok(found)
}

/// The save `id` as a hint's command names it: its short change ID when that reaches it on the
/// current line, otherwise the first 12 hex digits of its fingerprint when those reach it; `None`
/// when neither does.
#[must_use]
pub fn id_for_command(repo: &Repository, id: &Id) -> Option<String> {
    let reaches = |text: &String| matches!(resolve(repo, text), Ok(found) if found == *id);
    // A save off the line is named by its fingerprint: its short change ID is made unique only
    // among the line's saves.
    let on_line = |text: &String| {
        let found = repo
            .current()
            .ok()
            .map(|(_, entry)| by_change_prefix(repo, &entry, text));
        matches!(found, Some(Ok(found)) if matches!(found.as_slice(), [(one, _)] if one == id))
    };
    let save = repo.store().save(id, repo.signatures_allowed()).ok();
    let short = save.and_then(|save| short_change_id(repo, id, &save.change_id).ok());
    short
        .filter(on_line)
        .into_iter()
        .chain([id.short_hex()])
        .find(reaches)
}

/// Finds a save from text, as [`find`] reads it.
pub fn resolve(repo: &Repository, text: &str) -> Result<Id, ResolveError> {
    find(repo, text).map(|(_, id)| id)
}

/// Reads `text` in the one order a save is named by: a line's name, a server line
/// `line@server`, a tag's name, the start of a change ID (lowercase `k`-`z`, unique on the
/// current line), the start of a fingerprint (at least 8 hex digits). Gives where that leads,
/// and the save. A line or server line whose name a tag at another save has too is refused, as
/// is a line or tag name that is also the start of another save's ID.
pub(crate) fn find(repo: &Repository, text: &str) -> Result<(Target, Id), ResolveError> {
    let (_, entry) = repo.current()?;
    // Read once: it can be large.
    let tags = repo.tag_list(&entry.state)?;
    if let Some(line) = line_named(repo, &entry, &tags, text)? {
        if let Some(tag) = tag_elsewhere(&tags, text, line) {
            return Err(ResolveError::LineOrTag {
                text: text.to_owned(),
                line: shown_save(&line),
                tag: shown_save(&tag),
            });
        }
        return Ok((Target::Line(text.to_owned()), line));
    }
    // A line name never holds `@`, so a server line can't be taken for one.
    if let Some((line, server)) = text.split_once('@') {
        let servers = repo.server_lines(&entry.state)?;
        if let Some(&save) = servers.servers.get(server).and_then(|l| l.get(line)) {
            if let Some(tag) = tag_elsewhere(&tags, text, save) {
                return Err(ResolveError::ServerLineOrTag {
                    text: text.to_owned(),
                    line: shown_save(&save),
                    tag: shown_save(&tag),
                });
            }
            let target = Target::ServerLine {
                server: server.to_owned(),
                line: line.to_owned(),
                save,
            };
            return Ok((target, save));
        }
    }
    let id = match tag_named(repo, &entry, &tags, text)? {
        Some(id) => id,
        None => match by_id(repo, &entry, &tags, text)? {
            (found, Some(damage)) if found.is_empty() => return Err(damage.into()),
            (found, _) => one_of(text, found)?,
        },
    };
    Ok((Target::Save(id), id))
}

/// The save of the tag named `text`, if it isn't `save`.
fn tag_elsewhere(tags: &TagList, text: &str, save: Id) -> Option<Id> {
    tags.tags.get(text).map(|t| t.save).filter(|t| *t != save)
}

/// A save as a refusal names it: its fingerprint's first 12 hex digits.
fn shown_save(id: &Id) -> String {
    id.to_hex()[..12].to_owned()
}

/// The save of the line named `text`, if there is one. A name that is also the start of
/// another save's change ID or fingerprint is refused, so that neither passes for the other.
fn line_named(
    repo: &Repository,
    entry: &DiaryEntry,
    tags: &TagList,
    text: &str,
) -> Result<Option<Id>, ResolveError> {
    let Some(&line) = entry.state.lines.get(text) else {
        return Ok(None);
    };
    let saves = others_by_id(repo, entry, tags, text, line)?;
    if saves.is_empty() {
        return Ok(Some(line));
    }
    Err(ResolveError::LineOrSave {
        text: text.to_owned(),
        saves,
    })
}

/// The save of the tag named `text`, if there is one. A name that is also the start of
/// another save's change ID or fingerprint is refused, as a line's is.
fn tag_named(
    repo: &Repository,
    entry: &DiaryEntry,
    tags: &TagList,
    text: &str,
) -> Result<Option<Id>, ResolveError> {
    let Some(tag) = tags.tags.get(text) else {
        return Ok(None);
    };
    let saves = others_by_id(repo, entry, tags, text, tag.save)?;
    if saves.is_empty() {
        return Ok(Some(tag.save));
    }
    Err(ResolveError::TagOrSave {
        text: text.to_owned(),
        saves,
    })
}

/// The saves other than `own` that `text` names by ID, as shown, sorted and once each.
fn others_by_id(
    repo: &Repository,
    entry: &DiaryEntry,
    tags: &TagList,
    text: &str,
    own: Id,
) -> Result<Vec<String>, ResolveError> {
    // A save the wider walk couldn't read doesn't refuse the name: it can't be gone to either.
    // Any other failure still does.
    let mut saves: Vec<String> = (by_id(repo, entry, tags, text)?.0)
        .into_iter()
        .filter(|(id, _)| *id != own)
        .map(|(_, shown)| shown)
        .collect();
    saves.sort_unstable();
    saves.dedup();
    Ok(saves)
}

/// The saves found, each as it matched, and damage passed over on the way.
type Found = (Vec<(Id, String)>, Option<RepoError>);

fn is_change_start(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| (b'k'..=b'z').contains(&b))
}

/// The saves `text` names as the start of a change ID or of a fingerprint: on the current line
/// (a fingerprint: along first parents from any line, server line or tag) first; only when none
/// is there, through every parent. With them, damage the wider walk passed over.
fn by_id(
    repo: &Repository,
    entry: &DiaryEntry,
    tags: &TagList,
    text: &str,
) -> Result<Found, ResolveError> {
    let first = if is_change_start(text) {
        by_change_prefix(repo, entry, text)?
    } else if is_hex_start(text) {
        by_fingerprint_prefix(repo, entry, tags, text)?
    } else {
        return Ok((Vec::new(), None));
    };
    if !first.is_empty() {
        return Ok((first, None));
    }
    through_every_parent(repo, entry, tags, text)
}

/// The saves whose change ID or fingerprint starts with `text` among every save that a line, a
/// server line, a tag or the save the project is at reaches through every parent, each read
/// once. A save that can't be read ends the walk there but not the search (its fingerprint
/// still counts); the first such damage comes back with what was found.
fn through_every_parent(
    repo: &Repository,
    entry: &DiaryEntry,
    tags: &TagList,
    text: &str,
) -> Result<Found, ResolveError> {
    let by_change = is_change_start(text);
    let hex = text.to_ascii_lowercase();
    let servers = repo.server_lines(&entry.state)?;
    let at = match entry.state.position {
        Position::AtSave(id) => Some(id),
        Position::OnLine(_) => None,
    };
    let mut stack: Vec<Id> = (entry.state.lines.values().copied())
        .chain(servers.servers.values().flat_map(|l| l.values().copied()))
        .chain(tags.tags.values().map(|tag| tag.save))
        .chain(at)
        .collect();
    let mut seen = HashSet::new();
    let (mut found, mut damage) = (Vec::new(), None);
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        if !by_change && id.to_hex().starts_with(&hex) {
            found.push((id, id.to_hex()));
        }
        match repo.store().save(&id, repo.signatures_allowed()) {
            Ok(save) => {
                let letters = save.change_id.to_letters();
                if by_change && letters.starts_with(text) {
                    found.push((id, letters));
                }
                stack.extend(save.parents.into_iter().filter(|p| !seen.contains(p)));
            }
            Err(e) if is_unreadable_record(&e) => {
                damage.get_or_insert(RepoError::from(e));
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok((found, damage))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::diary::Action;
    use crate::pack::ObjectKind;
    use crate::save::{EXTRA_LINE, Person};
    use proptest::prelude::*;
    use std::collections::BTreeMap;

    fn who() -> Person {
        Person {
            name: "Ada".into(),
            email: "a@example.com".into(),
            time_ms: 0,
            tz_offset_min: 0,
        }
    }

    /// 32 letters starting with `start`.
    fn letters(start: &str) -> String {
        format!("{start}{}", "k".repeat(LETTERS - start.len()))
    }

    fn change(start: &str) -> ChangeId {
        ChangeId::from_letters(&letters(start)).unwrap()
    }

    /// Commits a save with the given parents and change ID, and points `line` at it if given.
    fn plant(repo: &mut Repository, parents: &[Id], start: &str, line: Option<&str>) -> Id {
        let save = save_of(repo, parents, start, "m");
        plant_save(repo, &save, line)
    }

    /// A save with the given parents, change ID and message, holding main's folder.
    fn save_of(repo: &Repository, parents: &[Id], start: &str, message: &str) -> Save {
        let (_, entry) = repo.current().unwrap();
        let root = repo
            .store()
            .save(&entry.state.lines["main"], false)
            .unwrap()
            .root;
        Save {
            root,
            parents: parents.to_vec(),
            author: who(),
            saver: who(),
            message: message.into(),
            change_id: change(start),
            move_notes: vec![],
            signature: vec![],
            extra: BTreeMap::from([(EXTRA_LINE.to_owned(), b"main".to_vec())]),
        }
    }

    /// Commits `save`, and points `line` at it if given.
    fn plant_save(repo: &mut Repository, save: &Save, line: Option<&str>) -> Id {
        let lock = repo.lock_for_change("save", &mut |_| {}).unwrap();
        let (head, entry) = repo.current().unwrap();
        let mut packs = repo.new_packs().unwrap();
        let id = packs.add_record(ObjectKind::Save, &save.encode()).unwrap();
        let mut state = entry.state.clone();
        if let Some(line) = line {
            state.lines.insert(line.to_owned(), id);
        }
        let diary = DiaryEntry {
            parent: Some(head),
            action: Action::Command,
            command: "save".into(),
            who: who(),
            opal_version: "0.1.0".into(),
            state,
            folder_change: None,
        };
        repo.commit(&lock, packs, &diary).unwrap();
        id
    }

    /// `short_change_id`, by reading the line from `tip`.
    fn walk_short(repo: &Repository, tip: Id, id: &Id, change_id: &ChangeId) -> String {
        let saves: Vec<(Id, ChangeId)> = chain(repo, tip)
            .map(|s| s.map(|(id, save)| (id, save.change_id)).unwrap())
            .collect();
        short_in(&saves, id, change_id)
    }

    /// The user's own cache key, as the command line hands it over.
    const KEY: [u8; crate::keyed::KEY_LEN] = [7; crate::keyed::KEY_LEN];

    /// A project whose line `main` ends with a save for each start given, the first one oldest.
    /// Also returns the saves' IDs, oldest first, and the first save's.
    fn project_with(starts: &[&str]) -> (tempfile::TempDir, Repository, Vec<Id>) {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(dir.path(), &who()).unwrap();
        repo.set_cache_key(KEY);
        let mut tip = repo.current().unwrap().1.state.lines["main"];
        let mut ids = vec![tip];
        for start in starts {
            tip = plant(&mut repo, &[tip], start, Some("main"));
            ids.push(tip);
        }
        (dir, repo, ids)
    }

    /// Each ID's shortest unique form by the plain definition: try longer starts until no other
    /// ID has it, and settle for all the letters if another ID is just the same.
    fn plainly(ids: &[ChangeId]) -> Vec<usize> {
        let all: Vec<String> = ids.iter().map(ChangeId::to_letters).collect();
        (0..ids.len())
            .map(|k| {
                (MIN_SHORT..=LETTERS)
                    .find(|&len| {
                        (0..ids.len()).all(|j| j == k || !all[j].starts_with(&all[k][..len]))
                    })
                    .unwrap_or(LETTERS)
            })
            .collect()
    }

    #[test]
    fn short_ids_grow_to_stay_unique() {
        let ids = [
            change("klmnop"),
            change("klmnopq"),
            change("zzzz"),
            change("zzzz"),
        ];
        assert_eq!(short_lengths(&ids), [7, 7, LETTERS, LETTERS]);
        assert_eq!(short_lengths(&ids[..3]), [7, 7, MIN_SHORT]);
        assert_eq!(short_lengths(&ids[..1]), [MIN_SHORT]);
        assert!(short_lengths(&[]).is_empty());
    }

    proptest! {
        // The first three bytes vary over a few values and the rest are alike, so that start
        // letters are shared at every length, and some IDs are the same.
        #[test]
        fn short_ids_are_the_shortest_that_tell_a_save_from_the_rest(
            heads in prop::collection::vec(
                prop::array::uniform3(prop::sample::select(vec![0x00u8, 0x01, 0x10, 0x11])),
                0..40,
            ),
        ) {
            let ids: Vec<ChangeId> = heads
                .iter()
                .map(|h| {
                    let mut bytes = [0; 16];
                    bytes[..3].copy_from_slice(h);
                    ChangeId(bytes)
                })
                .collect();
            let want = plainly(&ids);
            prop_assert_eq!(short_lengths(&ids), want.clone());
            // Taking the closest of the others one by one gives the same lengths.
            let one_by_one: Vec<usize> = (0..ids.len())
                .map(|k| {
                    let closest = (0..ids.len())
                        .filter(|&j| j != k)
                        .map(|j| common_letters(&ids[k], &ids[j]))
                        .max()
                        .unwrap_or(0);
                    short_len(closest)
                })
                .collect();
            prop_assert_eq!(one_by_one, want);
        }
    }

    #[test]
    fn what_the_user_typed_is_shown_escaped() {
        use crate::test_support::{HOSTILE, assert_hostile_text_escaped};
        let typed = ResolveError::NotFound(HOSTILE.into());
        assert_hostile_text_escaped(&typed.to_string());
        let ambiguous = ResolveError::Ambiguous {
            text: HOSTILE.into(),
            candidates: vec!["klmn".into(), "klmo".into()],
        };
        assert_hostile_text_escaped(&ambiguous.to_string());
    }

    #[test]
    fn a_change_prefix_names_one_save_or_lists_the_candidates() {
        let (_dir, repo, ids) = project_with(&["klmnopk", "klmnopl", "mmmmmm"]);
        // Long prefixes, so the random first save is not likely to share them.
        assert_eq!(resolve(&repo, "klmnopk").unwrap(), ids[1]);
        assert_eq!(resolve(&repo, "klmnopl").unwrap(), ids[2]);
        assert_eq!(resolve(&repo, "mmmmmm").unwrap(), ids[3]);
        match resolve(&repo, "klmnop") {
            Err(ResolveError::Ambiguous { candidates, .. }) => {
                assert_eq!(candidates, [letters("klmnopk"), letters("klmnopl")]);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            resolve(&repo, "zzzzzz"),
            Err(ResolveError::NotFound(_))
        ));
    }

    /// Gives the save `id` the line `name` too.
    fn name_line(repo: &mut Repository, name: &str, id: Id) {
        let lock = repo.lock_for_change("line", &mut |_| {}).unwrap();
        let (head, entry) = repo.current().unwrap();
        let mut state = entry.state.clone();
        state.lines.insert(name.to_owned(), id);
        let diary = DiaryEntry {
            parent: Some(head),
            action: Action::Command,
            command: "line".into(),
            who: who(),
            opal_version: "0.1.0".into(),
            state,
            folder_change: None,
        };
        let packs = repo.new_packs().unwrap();
        repo.commit(&lock, packs, &diary).unwrap();
    }

    #[test]
    fn a_line_named_like_another_saves_id_is_refused_and_one_named_like_its_own_is_not() {
        let (_dir, mut repo, ids) = project_with(&["mnopqr", "nopqrs"]);
        let elsewhere = plant(&mut repo, &[ids[0]], "kkkkkk", Some("other"));
        let refused = |repo: &Repository, text: &str| match resolve(repo, text) {
            Err(ResolveError::LineOrSave { saves, .. }) => saves,
            other => panic!("{other:?}"),
        };
        // `mnop` starts the change ID of a save on main, and names a line elsewhere.
        name_line(&mut repo, "mnop", elsewhere);
        assert_eq!(refused(&repo, "mnop"), [letters("mnopqr")]);
        let target = crate::lines::resolve_target(&repo, "mnop");
        assert!(matches!(target, Err(ResolveError::LineOrSave { .. })));
        // A line named like the start of another save's fingerprint is refused the same way.
        let hex = elsewhere.to_hex()[..8].to_owned();
        name_line(&mut repo, &hex, ids[1]);
        assert_eq!(refused(&repo, &hex), [elsewhere.to_hex()]);
        // A line named like the start of its own save's IDs is just the line.
        name_line(&mut repo, "nopq", ids[2]);
        assert_eq!(resolve(&repo, "nopq").unwrap(), ids[2]);
        let own = ids[2].to_hex()[..8].to_owned();
        name_line(&mut repo, &own, ids[2]);
        assert_eq!(
            crate::lines::resolve_target(&repo, &own).unwrap(),
            crate::lines::Target::Line(own.clone())
        );
    }

    #[test]
    fn a_line_or_save_refusal_shows_the_name_escaped() {
        use crate::test_support::{HOSTILE, assert_hostile_text_escaped};
        let refused = ResolveError::LineOrSave {
            text: HOSTILE.into(),
            saves: vec!["klmn".into()],
        };
        assert_hostile_text_escaped(&refused.to_string());
        let refused = ResolveError::TagOrSave {
            text: HOSTILE.into(),
            saves: vec!["klmn".into()],
        };
        assert_hostile_text_escaped(&refused.to_string());
        let (line, tag) = ("3f9a1c2b4d5e".to_owned(), "9b8c7d6e5f4a".to_owned());
        let refused = ResolveError::LineOrTag {
            text: HOSTILE.into(),
            line: line.clone(),
            tag: tag.clone(),
        };
        assert_hostile_text_escaped(&refused.to_string());
        let refused = ResolveError::ServerLineOrTag {
            text: HOSTILE.into(),
            line,
            tag,
        };
        assert_hostile_text_escaped(&refused.to_string());
    }

    /// Gives the save `id` the tag `name` too, as an import may: any name.
    fn name_tag(repo: &mut Repository, name: &str, id: Id) {
        use crate::diary::Tag;
        let lock = repo.lock_for_change("tag", &mut |_| {}).unwrap();
        let (head, entry) = repo.current().unwrap();
        let mut list = repo.tag_list(&entry.state).unwrap();
        let tag = Tag {
            save: id,
            message: String::new(),
            git_tags: vec![],
        };
        list.tags.insert(name.to_owned(), tag);
        let mut packs = repo.new_packs().unwrap();
        let mut state = entry.state.clone();
        state.tags = Some(
            packs
                .add_record(ObjectKind::TagList, &list.encode())
                .unwrap(),
        );
        let diary = DiaryEntry {
            parent: Some(head),
            action: Action::Command,
            command: "tag".into(),
            who: who(),
            opal_version: "0.1.0".into(),
            state,
            folder_change: None,
        };
        repo.commit(&lock, packs, &diary).unwrap();
    }

    #[test]
    fn a_line_wins_over_a_tag_of_the_same_name_at_its_save() {
        use crate::lines::{Target, resolve_target};
        let (_dir, mut repo, ids) = project_with(&["lmnopq"]);
        name_line(&mut repo, "v1.0", ids[0]);
        name_tag(&mut repo, "v1.0", ids[0]);
        assert_eq!(resolve(&repo, "v1.0").unwrap(), ids[0]);
        assert_eq!(
            resolve_target(&repo, "v1.0").unwrap(),
            Target::Line("v1.0".into())
        );
        // A tag alone is its save, and not a line.
        name_tag(&mut repo, "release/1", ids[0]);
        assert_eq!(resolve(&repo, "release/1").unwrap(), ids[0]);
        assert_eq!(
            resolve_target(&repo, "release/1").unwrap(),
            Target::Save(ids[0])
        );
    }

    #[test]
    fn a_tag_named_like_the_start_of_another_saves_id_is_refused() {
        let (_dir, mut repo, ids) = project_with(&["lmnopq"]);
        let hex = ids[0].to_hex()[..8].to_owned();
        name_tag(&mut repo, &hex, ids[1]);
        let refused = resolve(&repo, &hex).unwrap_err();
        match &refused {
            ResolveError::TagOrSave { saves, .. } => assert_eq!(saves, &[ids[0].to_hex()]),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            refused.to_string(),
            format!(
                "\"{hex}\" is the name of a tag and also the start of another save's ID ({}), so \
                 Opalro didn't pick either.",
                ids[0].to_hex()
            )
        );
        assert_eq!(
            refused.hint().unwrap(),
            "To reach the save, type more of its ID; opalro history --json gives the whole change \
             ID and fingerprint. To reach the tag's save, take its ID from: opalro tag"
        );
        let target = crate::lines::resolve_target(&repo, &hex);
        assert!(matches!(target, Err(ResolveError::TagOrSave { .. })));
        // A tag named like the start of its own save's IDs is just the tag.
        let own = ids[1].to_hex()[..8].to_owned();
        name_tag(&mut repo, &own, ids[1]);
        assert_eq!(resolve(&repo, &own).unwrap(), ids[1]);
    }

    #[test]
    fn a_tag_holding_an_at_sign_is_found_when_no_server_line_matches() {
        use crate::lines::{Target, resolve_target};
        let (_dir, mut repo, ids) = project_with(&["lmnopq"]);
        name_tag(&mut repo, "v1@home", ids[0]);
        assert_eq!(
            resolve_target(&repo, "v1@home").unwrap(),
            Target::Save(ids[0])
        );
        assert!(matches!(
            resolve_target(&repo, "v2@home"),
            Err(ResolveError::NotFound(_))
        ));
    }

    #[test]
    fn a_line_or_save_refusal_hints_at_renaming_the_line() {
        let refused = ResolveError::LineOrSave {
            text: "mnop".into(),
            saves: vec!["mnopqr".into()],
        };
        assert!(
            refused
                .hint()
                .unwrap()
                .ends_with(" To reach the line by another name: opalro line rename <old> <new>")
        );
    }

    #[test]
    fn a_fingerprint_prefix_finds_saves_on_any_line_and_through_a_second_parent() {
        let (_dir, mut repo, ids) = project_with(&["kkkk"]);
        let (first, tip) = (ids[0], ids[1]);
        let side = plant(&mut repo, &[first], "lmno", Some("side"));
        // `lone` is on no line; the only way to it is the merge's second parent.
        let lone = plant(&mut repo, &[first], "mnop", None);
        let merge = plant(&mut repo, &[tip, lone], "nopq", Some("main"));
        let prefix = |id: &Id| id.to_hex()[..8].to_uppercase();
        for found in [first, tip, side, merge, lone] {
            assert_eq!(resolve(&repo, &prefix(&found)).unwrap(), found);
        }
        assert!(matches!(
            resolve(&repo, "0123456"),
            Err(ResolveError::NotFound(_))
        ));
    }

    /// Gives the save `id` the server line `line@server`.
    fn name_server_line(repo: &mut Repository, server: &str, line: &str, id: Id) {
        use crate::diary::ServerLines;
        let lock = repo.lock_for_change("fetch", &mut |_| {}).unwrap();
        let (head, entry) = repo.current().unwrap();
        let servers = ServerLines {
            servers: [(server.to_owned(), [(line.to_owned(), id)].into())].into(),
        };
        let mut packs = repo.new_packs().unwrap();
        let mut state = entry.state.clone();
        state.server_lines = Some(
            packs
                .add_record(ObjectKind::ServerLines, &servers.encode())
                .unwrap(),
        );
        let diary = DiaryEntry {
            parent: Some(head),
            action: Action::Command,
            command: "fetch".into(),
            who: who(),
            opal_version: "0.1.0".into(),
            state,
            folder_change: None,
        };
        repo.commit(&lock, packs, &diary).unwrap();
    }

    /// Puts the save `id` on the shelf, as `shelf` would, without touching the folder.
    fn shelve(repo: &mut Repository, id: Id) {
        let lock = repo.lock_for_change("shelf", &mut |_| {}).unwrap();
        let (head, entry) = repo.current().unwrap();
        let mut state = entry.state.clone();
        state.shelves.insert(0, ("put away".to_owned(), id));
        let diary = DiaryEntry {
            parent: Some(head),
            action: Action::Command,
            command: "shelf".into(),
            who: who(),
            opal_version: "0.1.0".into(),
            state,
            folder_change: None,
        };
        let packs = repo.new_packs().unwrap();
        repo.commit(&lock, packs, &diary).unwrap();
    }

    #[test]
    fn a_save_reached_only_through_a_second_parent_is_found() {
        let (_dir, mut repo, ids) = project_with(&["kkkk"]);
        let lone = plant(&mut repo, &[ids[0]], "mnopqr", None);
        let merge = plant(&mut repo, &[ids[1], lone], "nopqrs", Some("main"));
        assert_eq!(resolve(&repo, "mnopqr").unwrap(), lone);
        assert_eq!(resolve(&repo, &lone.to_hex()[..8]).unwrap(), lone);
        // The save the project is at counts too, when no line has it.
        let behind = plant(&mut repo, &[ids[0]], "opqrst", None);
        let at = plant(&mut repo, &[merge, behind], "pqrstu", None);
        set_position(&mut repo, Position::AtSave(at));
        assert_eq!(resolve(&repo, "opqrst").unwrap(), behind);
        assert_eq!(resolve(&repo, &behind.to_hex()[..8]).unwrap(), behind);
        assert_eq!(resolve(&repo, &at.to_hex()[..8]).unwrap(), at);
    }

    #[test]
    fn a_line_named_like_a_save_reached_only_through_a_second_parent_is_refused() {
        let (_dir, mut repo, ids) = project_with(&["kkkk"]);
        let lone = plant(&mut repo, &[ids[0]], "mnopqr", None);
        plant(&mut repo, &[ids[1], lone], "nopqrs", Some("main"));
        // No line has `lone` on its first parents; a line elsewhere takes the start of its ID.
        let name = repo
            .store()
            .save(&lone, false)
            .unwrap()
            .change_id
            .to_letters()[..12]
            .to_owned();
        name_line(&mut repo, &name, ids[1]);
        match resolve(&repo, &name) {
            Err(ResolveError::LineOrSave { saves, .. }) => assert_eq!(saves, [letters("mnopqr")]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_line_comes_first_so_a_short_id_history_printed_still_works() {
        let (_dir, mut repo, ids) = project_with(&["lmnopl"]);
        let lone = plant(&mut repo, &[ids[0]], "lmnopk", None);
        plant(&mut repo, &[ids[1], lone], "nopqrs", Some("main"));
        assert_eq!(resolve(&repo, "lmnop").unwrap(), ids[1]);
        let short = short_change_id(&repo, &ids[1], &change("lmnopl")).unwrap();
        assert_eq!(resolve(&repo, &short).unwrap(), ids[1]);
        assert_eq!(resolve(&repo, "lmnopk").unwrap(), lone);
    }

    #[test]
    fn several_found_elsewhere_are_listed() {
        let (_dir, mut repo, ids) = project_with(&["kkkk"]);
        let one = plant(&mut repo, &[ids[0]], "mnopqk", None);
        let two = plant(&mut repo, &[ids[0]], "mnopql", None);
        let merge = plant(&mut repo, &[ids[1], one], "nopqrs", Some("main"));
        plant(&mut repo, &[merge, two], "opqrst", Some("main"));
        match resolve(&repo, "mnopq") {
            Err(ResolveError::Ambiguous { candidates, .. }) => {
                assert_eq!(candidates, [letters("mnopqk"), letters("mnopql")]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_tags_or_server_lines_history_counts() {
        let (_dir, mut repo, ids) = project_with(&[]);
        let by_tag = plant(&mut repo, &[ids[0]], "mnopqr", None);
        let tagged = plant(&mut repo, &[ids[0], by_tag], "nopqrs", None);
        name_tag(&mut repo, "v1.0", tagged);
        let by_server = plant(&mut repo, &[ids[0]], "opqrst", None);
        let fetched = plant(&mut repo, &[ids[0], by_server], "pqrstu", None);
        name_server_line(&mut repo, "origin", "main", fetched);
        for (start, found) in [("mnopqr", by_tag), ("opqrst", by_server)] {
            assert_eq!(resolve(&repo, start).unwrap(), found);
            assert_eq!(resolve(&repo, &found.to_hex()[..8]).unwrap(), found);
        }
    }

    #[test]
    fn a_save_only_a_shelf_holds_is_not_found() {
        let (_dir, mut repo, ids) = project_with(&[]);
        let shelved = plant(&mut repo, &[ids[0]], "mnopqr", None);
        shelve(&mut repo, shelved);
        for text in ["mnopqr".to_owned(), shelved.to_hex()[..8].to_owned()] {
            assert!(
                matches!(resolve(&repo, &text), Err(ResolveError::NotFound(_))),
                "{text}"
            );
        }
    }

    #[test]
    fn a_save_only_an_undone_step_holds_is_not_found() {
        let (_dir, mut repo, ids) = project_with(&["kkkk"]);
        let undone = plant(&mut repo, &[ids[1]], "mnopqr", Some("main"));
        move_line(&mut repo, "main", ids[1]);
        for text in ["mnopqr".to_owned(), undone.to_hex()[..8].to_owned()] {
            assert!(
                matches!(resolve(&repo, &text), Err(ResolveError::NotFound(_))),
                "{text}"
            );
        }
    }

    #[test]
    fn the_first_parents_come_first_for_a_fingerprint_too() {
        use crate::id::{Flavor, hash_record};
        let (_dir, mut repo, ids) = project_with(&["kkkk"]);
        // Two saves whose fingerprints share 8 hex digits, found by trying messages.
        let mut tried = std::collections::HashMap::new();
        let base = save_of(&repo, &[ids[0]], "mnop", "");
        let save = |n: u32| Save {
            message: n.to_string(),
            ..base.clone()
        };
        let (one, two) = (0u32..)
            .find_map(|n| {
                let id = hash_record(Flavor::Save, &save(n).encode());
                let start = u32::from_be_bytes(id.0[..4].try_into().unwrap());
                tried
                    .insert(start, n)
                    .map(|earlier| (save(earlier), save(n)))
            })
            .unwrap();
        let on_side = plant_save(&mut repo, &one, Some("side"));
        let lone = plant_save(&mut repo, &two, None);
        plant(&mut repo, &[ids[1], lone], "nopq", Some("main"));
        let start = on_side.to_hex()[..8].to_owned();
        assert_eq!(lone.to_hex()[..8], start);
        assert_eq!(resolve(&repo, &start).unwrap(), on_side);
        // Past the first step, both are there.
        let (_, entry) = repo.current().unwrap();
        let tags = repo.tag_list(&entry.state).unwrap();
        let (found, _) = through_every_parent(&repo, &entry, &tags, &start).unwrap();
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn a_miss_reads_each_save_once() {
        // Lines, a tag and a server line that overlap, and saves reached by several paths.
        let (_dir, mut repo, ids) = project_with(&["lmno", "lmnp", "lmnq"]);
        let s1 = plant(&mut repo, &[ids[1]], "mnok", Some("side"));
        let s2 = plant(&mut repo, &[s1], "mnol", Some("side"));
        let m1 = plant(&mut repo, &[ids[3], s2], "nopk", Some("main"));
        let x = plant(&mut repo, &[ids[2]], "nopl", None);
        plant(&mut repo, &[m1, x, s1], "nopm", Some("main"));
        name_tag(&mut repo, "v1.0", s1);
        name_server_line(&mut repo, "origin", "side", s2);
        let saves = 9;
        let reads = |repo: &Repository, f: &mut dyn FnMut()| {
            let before = repo.store().saves_read();
            f();
            repo.store().saves_read() - before
        };
        for text in ["zzzzzz", "000000000000"] {
            let missed = || matches!(resolve(&repo, text), Err(ResolveError::NotFound(_)));
            assert!(missed(), "{text}");
            let (_, entry) = repo.current().unwrap();
            let tags = repo.tag_list(&entry.state).unwrap();
            let first = reads(&repo, &mut || {
                if is_hex_start(text) {
                    by_fingerprint_prefix(&repo, &entry, &tags, text).unwrap();
                } else {
                    by_change_prefix(&repo, &entry, text).unwrap();
                }
            });
            let all = reads(&repo, &mut || assert!(missed(), "{text}"));
            assert!(all - first <= saves, "{text}: {all} - {first}");
            let wide = reads(&repo, &mut || {
                let (found, damage) = through_every_parent(&repo, &entry, &tags, text).unwrap();
                assert!(found.is_empty() && damage.is_none());
            });
            assert_eq!(wide, saves, "{text}");
        }
    }

    #[test]
    fn a_save_that_cant_be_read_is_told_only_when_nothing_is_found() {
        let (_dir, mut repo, ids) = project_with(&["kkkk"]);
        let missing = Id([7; 32]);
        // A save whose parent isn't in the store, and a tag at a save that isn't.
        let orphan = plant(&mut repo, &[missing], "mnopqr", None);
        plant(&mut repo, &[ids[1], orphan], "nopqrs", Some("main"));
        name_tag(&mut repo, "v1.0", Id([8; 32]));
        assert_eq!(resolve(&repo, "mnopqr").unwrap(), orphan);
        assert_eq!(resolve(&repo, &orphan.to_hex()[..8]).unwrap(), orphan);
        for text in ["zzzzzz", "000000000000"] {
            assert!(
                matches!(resolve(&repo, text), Err(ResolveError::Repo(_))),
                "{text}"
            );
        }
        // A name is still the line's: the damage hides nothing a line could be taken for.
        name_line(&mut repo, "qqqqqqqqqqqqqqqq", ids[1]);
        assert_eq!(resolve(&repo, "qqqqqqqqqqqqqqqq").unwrap(), ids[1]);
    }

    // Reading a name reads the tag list once, whatever the name turns out to be: a line, a tag,
    // hex text, or a line refused as a tag's name too.
    #[test]
    fn reading_a_name_reads_the_tag_list_once_whatever_it_names() {
        use crate::repo::TAG_LISTS_READ;
        let (_dir, mut repo, ids) = project_with(&["lmnopq"]);
        name_tag(&mut repo, "rel-1", ids[0]);
        let hex = ids[0].to_hex()[..8].to_owned();
        for name in ["main", "rel-1", hex.as_str()] {
            let before = TAG_LISTS_READ.get();
            resolve(&repo, name).unwrap();
            assert_eq!(TAG_LISTS_READ.get() - before, 1, "{name}");
        }
        // A line that is also a tag at another save is refused after one read too.
        name_tag(&mut repo, "main", ids[0]);
        let before = TAG_LISTS_READ.get();
        assert!(matches!(
            resolve(&repo, "main"),
            Err(ResolveError::LineOrTag { .. })
        ));
        assert_eq!(TAG_LISTS_READ.get() - before, 1);
    }

    #[test]
    fn saves_off_the_lines_that_share_a_start_are_more_than_one() {
        // No two saves can be made to share 8 digits, so the empty start stands in for one.
        let (_dir, mut repo, ids) = project_with(&[]);
        let behind = plant(&mut repo, &[ids[0]], "lmno", None);
        let tagged = plant(&mut repo, &[behind], "mnop", None);
        name_tag(&mut repo, "rel-1", tagged);
        let (_, entry) = repo.current().unwrap();
        let tags = repo.tag_list(&entry.state).unwrap();
        let found = by_fingerprint_prefix(&repo, &entry, &tags, "").unwrap();
        let mut all: Vec<Id> = found.iter().map(|(id, _)| *id).collect();
        all.sort_unstable();
        let mut want = vec![ids[0], behind, tagged];
        want.sort_unstable();
        assert_eq!(all, want);
        assert!(matches!(
            one_of("", found),
            Err(ResolveError::Ambiguous { candidates, .. }) if candidates.len() == 3
        ));
    }

    #[test]
    fn ambiguous_candidates_are_sorted_and_listed_once() {
        let a = (Id([0x12; 32]), "1234".to_owned());
        let b = (Id([0x11; 32]), "1111".to_owned());
        match one_of("1", vec![a.clone(), b.clone(), a.clone()]) {
            Err(ResolveError::Ambiguous { candidates, .. }) => {
                assert_eq!(candidates, ["1111", "1234"]);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(one_of("1", vec![a.clone(), a.clone()]).unwrap(), a.0);
    }

    /// An ID that records its position in its first four bytes.
    fn numbered(n: u32) -> Id {
        let mut bytes = [0; 32];
        bytes[..4].copy_from_slice(&n.to_be_bytes());
        Id(bytes)
    }

    fn number(id: &Id) -> u32 {
        u32::from_be_bytes(id.0[..4].try_into().unwrap())
    }

    #[test]
    fn a_chain_many_tips_lead_to_is_walked_once() {
        // 300 saves in a chain, each the child of the one before, and 100,000 tips at the newest.
        let mut reads = 0;
        let mut visited = Vec::new();
        each_once(
            &mut HashSet::new(),
            (0..100_000).map(|_| numbered(299)),
            |id| {
                reads += 1;
                Ok::<_, ()>(number(id).checked_sub(1).map(numbered))
            },
            |id| visited.push(number(id)),
        )
        .unwrap();
        assert_eq!(reads, 300);
        assert_eq!(visited.len(), 300);
    }

    #[test]
    fn each_save_is_visited_once_however_the_tips_overlap() {
        // Saves 0 to 49 in a chain, and 50 to 89 in another that joins it at 9.
        let parent = |n: u32| match n {
            0 => None,
            50 => Some(9),
            _ => Some(n - 1),
        };
        let tips = [49, 20, 89, 49, 9, 60, 0].map(numbered);
        let (mut reads, mut visited) = (0, Vec::new());
        each_once(
            &mut HashSet::new(),
            tips,
            |id| {
                reads += 1;
                Ok::<_, ()>(parent(number(id)).map(numbered))
            },
            |id| visited.push(number(id)),
        )
        .unwrap();
        assert_eq!(reads, 90);
        visited.sort_unstable();
        assert_eq!(visited, (0..=89).collect::<Vec<u32>>());
    }

    #[test]
    fn a_walk_that_fails_stops_with_the_error() {
        let result = each_once(
            &mut HashSet::new(),
            [numbered(5)],
            |id| {
                if number(id) == 3 {
                    Err("a save is missing")
                } else {
                    Ok(Some(numbered(number(id) - 1)))
                }
            },
            |_| {},
        );
        assert_eq!(result, Err("a save is missing"));
    }

    #[test]
    fn the_line_is_handed_out_newest_first_with_short_ids() {
        // Prefixes shared at several lengths, and one change ID used twice.
        let starts = [
            "klmnopk", "klmnopl", "klmnoqk", "zzzz", "klmnopk", "mmmmmmmm", "mmmmmmmn",
        ];
        let (_dir, repo, ids) = project_with(&starts);
        let mut shown = Vec::new();
        for_each_on_line::<RepoError>(&repo, |id, save, short| {
            shown.push((*id, save.change_id, short.to_owned()));
            Ok(())
        })
        .unwrap();
        let newest_first: Vec<Id> = ids.iter().rev().copied().collect();
        assert_eq!(
            shown.iter().map(|(id, ..)| *id).collect::<Vec<_>>(),
            newest_first
        );
        let change_ids: Vec<ChangeId> = shown.iter().map(|(_, c, _)| *c).collect();
        for ((id, change_id, short), len) in shown.iter().zip(plainly(&change_ids)) {
            assert_eq!(short, &change_id.to_letters()[..len]);
            // One save at a time gives the same answer.
            assert_eq!(&short_change_id(&repo, id, change_id).unwrap(), short);
        }
        // The change ID that two saves have shows whole for both.
        let twice = change("klmnopk").to_letters();
        assert_eq!(shown.iter().filter(|(_, _, s)| *s == twice).count(), 2);
    }

    #[test]
    fn a_save_off_the_line_is_told_from_the_line() {
        let (_dir, mut repo, _) = project_with(&["klmnopk"]);
        let off = plant(&mut repo, &[], "klmnopl", None);
        let short = short_change_id(&repo, &off, &change("klmnopl")).unwrap();
        assert_eq!(short, "klmnopl");
    }

    /// Points `line` at `id`, as an undo would, without a new save.
    fn move_line(repo: &mut Repository, line: &str, id: Id) {
        let lock = repo.lock_for_change("undo", &mut |_| {}).unwrap();
        let (head, entry) = repo.current().unwrap();
        let mut state = entry.state.clone();
        state.lines.insert(line.to_owned(), id);
        let diary = DiaryEntry {
            parent: Some(head),
            action: Action::Command,
            command: "undo".into(),
            who: who(),
            opal_version: "0.1.0".into(),
            state,
            folder_change: None,
        };
        let packs = repo.new_packs().unwrap();
        repo.commit(&lock, packs, &diary).unwrap();
    }

    /// The short IDs of the current line by reading it whole, newest first.
    fn uncached_line(repo: &Repository) -> Vec<String> {
        let saves: Vec<ChangeId> = chain(repo, current_save(repo).unwrap())
            .map(|s| s.unwrap().1.change_id)
            .collect();
        saves
            .iter()
            .zip(plainly(&saves))
            .map(|(c, len)| c.to_letters()[..len].to_owned())
            .collect()
    }

    fn cached_line(repo: &Repository) -> Vec<String> {
        let mut shown = Vec::new();
        for_each_on_line::<RepoError>(repo, |_, _, short| {
            shown.push(short.to_owned());
            Ok(())
        })
        .unwrap();
        shown
    }

    /// `main`'s saves as the store has them, newest first.
    fn true_line(repo: &Repository) -> Vec<(Id, ChangeId)> {
        chain(repo, repo.current().unwrap().1.state.lines["main"])
            .map(|s| s.map(|(id, save)| (id, save.change_id)).unwrap())
            .collect()
    }

    /// A cache that has `main`'s saves right, but gives its oldest a change ID that shares 7
    /// letters with `klmnopk`.
    fn lying_cache(repo: &Repository) -> crate::changeids::ChangeIds {
        let mut saves = true_line(repo);
        saves.last_mut().unwrap().1 = change("klmnopkl");
        let mut lie = crate::changeids::ChangeIds::default();
        lie.set("main", saves);
        lie
    }

    fn plant_cache(dir: &std::path::Path, bytes: &[u8]) {
        std::fs::write(dir.join(".opalro/cache/change-ids"), bytes).unwrap();
    }

    /// Plants `saves` as `main`'s list, sealed with this account's own key.
    fn plant_list(dir: &std::path::Path, saves: Vec<(Id, ChangeId)>) {
        let mut lie = crate::changeids::ChangeIds::default();
        lie.set("main", saves);
        plant_cache(dir, &crate::keyed::seal(&KEY, &lie.encode()));
    }

    /// What `resolve` gives for `text` with `main`'s list planted as `saves`, and with no cache.
    fn resolved(
        dir: &std::path::Path,
        repo: &Repository,
        saves: &[(Id, ChangeId)],
        text: &str,
    ) -> (String, String) {
        plant_list(dir, saves.to_vec());
        let cached = format!("{:?}", resolve(repo, text));
        let cold = Repository::open(dir).unwrap();
        (cached, format!("{:?}", resolve(&cold, text)))
    }

    #[test]
    fn a_cache_from_another_account_is_ignored() {
        let (dir, repo, ids) = project_with(&["klmnopl", "klmnopk"]);
        let lie = lying_cache(&repo);
        let tip = change("klmnopk");
        // The lie, sealed with this account's key, is believed where no save is read: the test
        // can tell.
        plant_cache(dir.path(), &crate::keyed::seal(&KEY, &lie.encode()));
        assert_eq!(short_change_id(&repo, &ids[2], &tip).unwrap(), "klmnopkk");
        let other = [0x5a; crate::keyed::KEY_LEN];
        assert_ne!(other, KEY);
        plant_cache(dir.path(), &crate::keyed::seal(&other, &lie.encode()));
        assert_eq!(cached_line(&repo), uncached_line(&repo));
        assert_eq!(cached_line(&repo)[0], "klmnopk");
        plant_cache(dir.path(), &crate::keyed::seal(&other, &lie.encode()));
        assert_eq!(short_change_id(&repo, &ids[2], &tip).unwrap(), "klmnopk");
    }

    #[test]
    fn an_edited_cache_is_ignored() {
        let (dir, repo, ids) = project_with(&["klmnopl", "klmnopk"]);
        let tip = change("klmnopk");
        // A cache Opalro wrote, its body swapped for the lie and its seal kept.
        assert_eq!(cached_line(&repo), uncached_line(&repo));
        let path = dir.path().join(".opalro/cache/change-ids");
        let written = std::fs::read(&path).unwrap();
        let seal = &written[written.len() - crate::keyed::KEY_LEN..];
        let mut edited = lying_cache(&repo).encode();
        edited.extend_from_slice(seal);
        plant_cache(dir.path(), &edited);
        assert_eq!(short_change_id(&repo, &ids[2], &tip).unwrap(), "klmnopk");
        // Every single bit flipped in what Opalro wrote, too.
        for at in [0, written.len() / 2, written.len() - 1] {
            let mut flipped = written.clone();
            flipped[at] ^= 1;
            plant_cache(dir.path(), &flipped);
            assert_eq!(short_change_id(&repo, &ids[2], &tip).unwrap(), "klmnopk");
        }
    }

    /// Every save the listing hands out, with its short form.
    fn listing(repo: &Repository) -> Vec<(Id, Save, String)> {
        let mut shown = Vec::new();
        for_each_on_line::<RepoError>(repo, |id, save, short| {
            shown.push((*id, save.clone(), short.to_owned()));
            Ok(())
        })
        .unwrap();
        shown
    }

    #[test]
    fn a_wrong_list_changes_no_save_handed_out() {
        let (dir, mut repo, ids) = project_with(&["klmnopl", "klmnopk", "mmmm"]);
        let side = plant(&mut repo, &[ids[1]], "klmnopkl", Some("side"));
        let truth = true_line(&repo);
        let cold = listing(&Repository::open(dir.path()).unwrap());
        let mut swapped = truth.clone();
        swapped[1] = (side, change("klmnopkl"));
        let mut crossed = truth.clone();
        (crossed[1].1, crossed[2].1) = (truth[2].1, truth[1].1);
        let mut added = truth.clone();
        added.push((Id([0x5a; 32]), change("klmnopkz")));
        let lies = [
            swapped,
            crossed,
            vec![truth[0], truth[2], truth[3]],
            truth[..2].to_vec(),
            added,
        ];
        for lie in lies {
            let wrong = (0..).find(|&at| lie.get(at) != truth.get(at)).unwrap();
            plant_list(dir.path(), lie);
            let shown = listing(&repo);
            let saves = |l: &[(Id, Save, String)]| -> Vec<(Id, Save)> {
                l.iter().map(|(id, save, _)| (*id, save.clone())).collect()
            };
            assert_eq!(saves(&shown), saves(&cold));
            // The short forms before the first wrong entry rest on the list, as all of them do.
            let shorts = |l: &[(Id, Save, String)]| -> Vec<String> {
                l.iter().skip(wrong).map(|(.., s)| s.clone()).collect()
            };
            assert_eq!(shorts(&shown), shorts(&cold));
            let fixed = crate::changeids::ChangeIds::load(&repo);
            assert_eq!(fixed.get("main").unwrap(), truth.as_slice());
        }
    }

    #[test]
    fn a_warm_listing_reads_each_save_once_and_one_before_the_first() {
        let starts = vec!["lmno"; 299];
        let (_dir, repo, _) = project_with(&starts);
        cached_line(&repo);
        let before = repo.store().saves_read();
        let mut first = None;
        for_each_on_line::<RepoError>(&repo, |_, _, _| {
            first.get_or_insert(repo.store().saves_read() - before);
            Ok(())
        })
        .unwrap();
        assert_eq!(first, Some(1));
        assert_eq!(repo.store().saves_read() - before, 300);
    }

    #[test]
    fn a_listing_cut_short_reads_no_more_of_the_line() {
        let (_dir, mut repo, ids) = project_with(&vec!["lmno"; 99]);
        cached_line(&repo);
        // A save the cache doesn't hold yet, as after `opalro history | head`.
        let newest = plant(&mut repo, &[ids[99]], "lmnp", Some("main"));
        let cold = crate::changeids::ChangeIds::load(&repo);
        assert_ne!(cold.get("main").unwrap()[0].0, newest);
        let before = repo.store().saves_read();
        let stopped = for_each_on_line::<RepoError>(&repo, |_, _, _| {
            Err(RepoError::Damaged {
                detail: "the pipe closed".into(),
            })
        });
        assert!(stopped.is_err());
        assert!(repo.store().saves_read() - before <= 2);
        // Written before the first save was handed out, the cache holds the line as it is.
        let kept = crate::changeids::ChangeIds::load(&repo);
        assert_eq!(kept.get("main").unwrap(), true_line(&repo).as_slice());
    }

    #[test]
    fn a_list_with_change_ids_swapped_finds_what_the_store_finds() {
        let (dir, repo, _) = project_with(&["klmnopl", "klmnopk", "mmmm"]);
        let truth = true_line(&repo);
        let mut lie = truth.clone();
        (lie[1].1, lie[2].1) = (truth[2].1, truth[1].1);
        for (_, change_id) in &truth[1..3] {
            let (cached, cold) = resolved(dir.path(), &repo, &lie, &change_id.to_letters());
            assert_eq!(cached, cold);
            assert!(cold.starts_with("Ok"), "{cold}");
        }
    }

    #[test]
    fn a_list_wrong_above_the_save_found_is_dropped_though_the_answer_agrees() {
        let mut starts = vec!["mmmm"; 20];
        starts.extend(["klmnopl", "nnnn"]);
        let (dir, repo, ids) = project_with(&starts);
        let mut lie = true_line(&repo);
        lie[0].1 = change("pppp");
        let text = lie[1].1.to_letters();
        let (cached, cold) = resolved(dir.path(), &repo, &lie, &text);
        assert_eq!(cached, cold);
        // The lie is gone: asked again, only the saves down to the one found are read.
        let before = repo.store().saves_read();
        assert_eq!(resolve(&repo, &text).unwrap(), ids[21]);
        let read = repo.store().saves_read() - before;
        assert!(read <= 4, "{read} saves read for a save 1 back");
    }

    #[test]
    fn a_list_that_drops_a_save_still_finds_it() {
        let (dir, repo, _) = project_with(&["klmnopl", "klmnopk", "mmmm"]);
        let truth = true_line(&repo);
        let lie = vec![truth[0], truth[2], truth[3]];
        let (cached, cold) = resolved(dir.path(), &repo, &lie, &truth[1].1.to_letters());
        assert_eq!(cached, cold);
        assert!(cold.starts_with("Ok"), "{cold}");
    }

    #[test]
    fn a_list_that_adds_a_save_invents_no_clash() {
        let (dir, mut repo, ids) = project_with(&["klmnopl", "klmnopk", "mmmm"]);
        // A real save with a change ID like one of main's, but on no line main reaches.
        let elsewhere = plant(&mut repo, &[ids[0]], "klmnopkm", Some("side"));
        let truth = true_line(&repo);
        let invented = (Id([0x5a; 32]), change("klmnopkz"));
        let mut added = truth.clone();
        added.push(invented);
        let mut from_elsewhere = truth.clone();
        from_elsewhere.push((elsewhere, change("klmnopkm")));
        // And a list where the invented save takes the place of the real one.
        let mut instead = truth.clone();
        instead[1] = invented;
        for lie in [added, from_elsewhere, instead] {
            let (cached, cold) = resolved(dir.path(), &repo, &lie, "klmnopk");
            assert_eq!(cached, cold);
            assert_eq!(cold, format!("{:?}", Ok::<Id, ()>(ids[2])));
        }
    }

    #[test]
    fn a_whole_id_a_list_holds_wrongly_is_found_as_without_the_list() {
        let (dir, mut repo, ids) = project_with(&["klmnopl", "klmnopk", "mmmm"]);
        let listing = repo.store().save(&ids[3], false).unwrap().root;
        let stray = plant(&mut repo, &[], "nnnn", None);
        let truth = true_line(&repo);
        for (stranger, change_id) in [(listing, change("pppp")), (stray, change("nnnn"))] {
            let mut lie = truth.clone();
            lie.insert(1, (stranger, change_id));
            let (cached, cold) = resolved(dir.path(), &repo, &lie, &stranger.to_hex());
            assert_eq!(cached, cold);
            assert!(cold.starts_with("Err(NotFound"), "{cold}");
            let fixed = crate::changeids::ChangeIds::load(&repo);
            assert_eq!(fixed.get("main").unwrap(), truth.as_slice());
        }
        // At a save, the lie is dropped and nothing is read but the lines' walk and the one
        // link the check reads, the newest save, which the walk reads too.
        set_position(&mut repo, Position::AtSave(ids[1]));
        let mut lie = truth.clone();
        lie.insert(1, (stray, change("nnnn")));
        plant_list(dir.path(), lie);
        let before = repo.store().saves_read();
        let cached = format!("{:?}", resolve(&repo, &stray.to_hex()));
        let cached_reads = repo.store().saves_read() - before;
        let cold = Repository::open(dir.path()).unwrap();
        let shown = format!("{:?}", resolve(&cold, &stray.to_hex()));
        assert_eq!(cached, shown);
        assert_eq!(cached_reads, cold.store().saves_read() + 1);
        let dropped = crate::changeids::ChangeIds::load(&repo);
        assert!(dropped.get("main").is_none());
    }

    #[test]
    fn a_whole_id_costs_what_its_save_is_far_from_the_newest() {
        let (_dir, repo, ids) = project_with(&vec!["lmno"; 30]);
        cached_line(&repo);
        for (back, most) in [(0, 2), (10, 12)] {
            let id = ids[ids.len() - 1 - back];
            let before = repo.store().saves_read();
            assert_eq!(resolve(&repo, &id.to_hex()).unwrap(), id);
            let read = repo.store().saves_read() - before;
            assert!(read <= most, "{read} saves read for one {back} back");
        }
    }

    #[test]
    fn a_list_with_another_line_s_save_changes_no_answer() {
        let (dir, mut repo, ids) = project_with(&["klmnopl", "klmnopk", "mmmm"]);
        // Saves on no line, so only a believed list could find them.
        let stray = plant(&mut repo, &[ids[1]], "nopqrs", None);
        let mut lie = true_line(&repo);
        lie[1] = (stray, change("nopqrs"));
        let (cached, cold) = resolved(dir.path(), &repo, &lie, "nopqr");
        assert_eq!(cached, cold);
        assert!(cold.starts_with("Err(NotFound"), "{cold}");
        // A line name that a listed stray's change ID starts with still names the line.
        let work = plant(&mut repo, &[ids[0]], "workkk", None);
        name_line(&mut repo, "work", ids[1]);
        let mut lie = true_line(&repo);
        lie[1] = (work, change("workkk"));
        let (cached, cold) = resolved(dir.path(), &repo, &lie, "work");
        assert_eq!(cached, cold);
        assert_eq!(cold, format!("{:?}", Ok::<Id, ()>(ids[1])));
    }

    #[test]
    fn a_change_id_costs_what_its_save_is_far_from_the_newest() {
        let starts: Vec<String> = (0..200u8)
            .map(|n| format!("{}{}", char::from(b'k' + n / 16), char::from(b'k' + n % 16)))
            .collect();
        let starts: Vec<&str> = starts.iter().map(String::as_str).collect();
        let (_dir, repo, ids) = project_with(&starts);
        cached_line(&repo);
        for (back, most) in [(1, 4), (50, 55)] {
            let id = ids[ids.len() - 1 - back];
            let letters = repo
                .store()
                .save(&id, false)
                .unwrap()
                .change_id
                .to_letters();
            let before = repo.store().saves_read();
            assert_eq!(resolve(&repo, &letters).unwrap(), id);
            let read = repo.store().saves_read() - before;
            assert!(read <= most, "{read} saves read for one {back} back");
        }
    }

    #[derive(Debug, Clone)]
    enum Step {
        Save(&'static str),
        /// A save of the folder through `save`, on the line the project is on.
        RealSave,
        Back(usize),
        /// Moves `main` to one of the saves made so far: back, forward or elsewhere.
        To(usize),
        Side(&'static str),
        /// Makes or moves the line `side` to one of the saves made so far.
        Line(usize),
        Amend,
        /// Goes to one of the saves made so far, off the line.
        Goto(usize),
        OnLine,
    }

    const STARTS: [&str; 5] = ["klmn", "klmo", "klmnop", "zzzz", "lmno"];

    fn step() -> impl Strategy<Value = Step> {
        let start = prop::sample::select(STARTS.to_vec());
        prop_oneof![
            3 => start.clone().prop_map(Step::Save),
            1 => Just(Step::RealSave),
            1 => (0..4usize).prop_map(Step::Back),
            1 => any::<usize>().prop_map(Step::To),
            1 => start.prop_map(Step::Side),
            1 => any::<usize>().prop_map(Step::Line),
            1 => Just(Step::Amend),
            1 => any::<usize>().prop_map(Step::Goto),
            1 => Just(Step::OnLine),
        ]
    }

    /// Saves the folder through `save`, or amends `main`'s newest save, with a message no save
    /// has yet. `None` if the amend is refused because another line shows the save.
    fn save_through(
        repo: &mut Repository,
        message: &str,
        amend: bool,
    ) -> Option<(Id, ChangeId, String)> {
        let ada = who();
        let req = crate::snapshot::SaveRequest {
            message: Some(message),
            who: &ada,
            command: "save",
            accept_crlf: false,
            git_excludes: None,
            new_line: None,
            only: None,
            amend,
        };
        match crate::snapshot::save_folder(repo, &req, &mut |_| {}, &mut |_| true) {
            Ok(crate::snapshot::SaveOutcome::Saved {
                save_id,
                save,
                short_change_id,
                ..
            }) => Some((save_id, save.change_id, short_change_id)),
            Err(crate::snapshot::SaveError::AmendShared { .. }) => None,
            other => panic!("{other:?}"),
        }
    }

    /// Whether a line other than `main` has `main`'s newest save in its history, through every
    /// parent.
    fn shown_elsewhere(repo: &Repository) -> bool {
        let lines = repo.current().unwrap().1.state.lines;
        let tip = lines["main"];
        let mut todo: Vec<Id> = (lines.iter())
            .filter(|(name, _)| *name != "main")
            .map(|(_, id)| *id)
            .collect();
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = todo.pop() {
            if id == tip {
                return true;
            }
            if seen.insert(id) {
                todo.extend(repo.store().save(&id, false).unwrap().parents);
            }
        }
        false
    }

    /// Puts the project at `position`, as `goto` would, without touching the folder.
    fn set_position(repo: &mut Repository, position: Position) {
        let lock = repo.lock_for_change("goto", &mut |_| {}).unwrap();
        let (head, entry) = repo.current().unwrap();
        let mut state = entry.state.clone();
        state.position = position;
        let diary = DiaryEntry {
            parent: Some(head),
            action: Action::Command,
            command: "goto".into(),
            who: who(),
            opal_version: "0.1.0".into(),
            state,
            folder_change: None,
        };
        let packs = repo.new_packs().unwrap();
        repo.commit(&lock, packs, &diary).unwrap();
    }

    /// Everything the cache answers for, as `repo` answers it, for the saves `saves`.
    fn answers(repo: &Repository, saves: &[Id]) -> Vec<String> {
        let mut out = vec![format!("{:?}", cached_line(repo))];
        let line = LineIds::read(repo).unwrap();
        for id in saves {
            let change_id = repo.store().save(id, false).unwrap().change_id;
            out.push(short_change_id(repo, id, &change_id).unwrap());
            out.push(format!(
                "{} {}",
                line.short(id, &change_id),
                line.shared(id, &change_id)
            ));
            out.push(format!("{:?}", resolve(repo, &change_id.to_letters())));
            out.push(format!("{:?}", resolve(repo, &id.to_hex())));
            out.push(format!("{:?}", resolve(repo, &id.to_hex().to_uppercase())));
        }
        for start in STARTS {
            out.push(format!("{:?}", resolve(repo, start)));
        }
        out.push(format!("{:?}", resolve(repo, &Id([0x5a; 32]).to_hex())));
        out
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(24))]
        // Saves the cache never saw and saves it learns of, undos that move a line back and
        // forward again, saves off the line, a second line, amends that give a new save an old
        // change ID, and the project at a save, with the cache read and rewritten in between:
        // every answer is the one a handle with no cache gives.
        #[test]
        fn answers_match_the_uncached_ones(steps in prop::collection::vec(step(), 1..12)) {
            let (dir, mut repo, mut saves) = project_with(&[]);
            for step in steps {
                let on_line =
                    matches!(repo.current().unwrap().1.state.position, Position::OnLine(_));
                match step {
                    Step::Save(start) => {
                        let tip = repo.current().unwrap().1.state.lines["main"];
                        saves.push(plant(&mut repo, &[tip], start, Some("main")));
                    }
                    Step::RealSave if on_line => {
                        let n = saves.len();
                        std::fs::write(dir.path().join("f.txt"), format!("{n}")).unwrap();
                        let (id, change_id, short) =
                            save_through(&mut repo, &format!("{n}"), false).unwrap();
                        prop_assert_eq!(short, walk_short(&repo, id, &id, &change_id));
                        saves.push(id);
                    }
                    Step::RealSave => {}
                    Step::Back(n) => {
                        let tip = repo.current().unwrap().1.state.lines["main"];
                        let back = chain(&repo, tip).nth(n).map(|s| s.unwrap().0);
                        if let Some(id) = back {
                            move_line(&mut repo, "main", id);
                        }
                    }
                    Step::To(n) => move_line(&mut repo, "main", saves[n % saves.len()]),
                    Step::Side(start) => saves.push(plant(&mut repo, &[saves[0]], start, None)),
                    Step::Line(n) => name_line(&mut repo, "side", saves[n % saves.len()]),
                    // A message no save has yet, so there is always something to amend.
                    Step::Amend if on_line => {
                        let shared = shown_elsewhere(&repo);
                        let done = save_through(&mut repo, &format!("{}", saves.len()), true);
                        prop_assert_eq!(done.is_none(), shared);
                        if let Some((id, change_id, short)) = done {
                            prop_assert_eq!(short, walk_short(&repo, id, &id, &change_id));
                            saves.push(id);
                        }
                    }
                    Step::Amend => {}
                    Step::Goto(n) => set_position(&mut repo, Position::AtSave(saves[n % saves.len()])),
                    Step::OnLine => set_position(&mut repo, Position::OnLine("main".into())),
                }
                prop_assert_eq!(cached_line(&repo), uncached_line(&repo));
                let cold = Repository::open(dir.path()).unwrap();
                prop_assert_eq!(answers(&repo, &saves), answers(&cold, &saves));
                let tip = current_save(&repo).unwrap();
                for id in &saves {
                    let change_id = repo.store().save(id, false).unwrap().change_id;
                    let walked = walk_short(&repo, tip, id, &change_id);
                    prop_assert_eq!(&short_change_id(&repo, id, &change_id).unwrap(), &walked);
                    let another = chain(&repo, tip)
                        .map(Result::unwrap)
                        .any(|(other, save)| other != *id && save.change_id == change_id);
                    let line = LineIds::read(&repo).unwrap();
                    prop_assert_eq!(line.shared(id, &change_id), another);
                }
            }
        }
    }

    /// The saves read by going to the save before the newest of a line of `len` saves and back,
    /// by its change ID and by its whole ID, once a command has shown the line.
    fn reads_to_go_back_and_forth(len: u8) -> usize {
        let starts: Vec<String> = (0..len)
            .map(|n| format!("{}{}", char::from(b'k' + n / 16), char::from(b'k' + n % 16)))
            .collect();
        let starts: Vec<&str> = starts.iter().map(String::as_str).collect();
        let (_dir, mut repo, ids) = project_with(&starts);
        let before = ids[ids.len() - 2];
        let letters = repo
            .store()
            .save(&before, false)
            .unwrap()
            .change_id
            .to_letters();
        cached_line(&repo);
        let read = repo.store().saves_read();
        for text in [&letters[..4], "main", &before.to_hex(), "main"] {
            let target = crate::lines::resolve_target(&repo, text).unwrap();
            let done =
                crate::switch::goto(&mut repo, &target, &who(), "goto", None, &mut |_| {}).unwrap();
            let save = repo.store().save(&done.save, false).unwrap();
            short_change_id(&repo, &done.save, &save.change_id).unwrap();
        }
        repo.store().saves_read() - read
    }

    #[test]
    fn a_goto_to_the_save_before_reads_no_line() {
        assert_eq!(
            reads_to_go_back_and_forth(20),
            reads_to_go_back_and_forth(120)
        );
    }

    #[test]
    fn a_line_that_cant_be_read_is_an_error() {
        let (dir, repo, ids) = project_with(&["klmnopk"]);
        for entry in std::fs::read_dir(repo.packs_dir()).unwrap() {
            let path = entry.unwrap().path();
            let found = crate::pack::PackReader::open(&path)
                .unwrap()
                .find(&ids[0])
                .unwrap();
            if let Some(found) = found {
                let mut bytes = std::fs::read(&path).unwrap();
                bytes[usize::try_from(found.offset + crate::pack::ENTRY_HEADER_LEN).unwrap()] ^= 1;
                std::fs::write(&path, bytes).unwrap();
            }
        }
        let repo = Repository::open(dir.path()).unwrap();
        assert!(matches!(
            short_change_id(&repo, &ids[1], &change("klmnopk")),
            Err(RepoError::Damaged { .. })
        ));
        let mut printed = 0;
        let result = for_each_on_line::<RepoError>(&repo, |_, _, _| {
            printed += 1;
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(printed, 0, "nothing is handed out before the line is known");
    }
}
