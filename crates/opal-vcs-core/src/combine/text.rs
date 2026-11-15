//! The three-way merge of one text file, line by line.

use std::ops::Range;
use std::time::Duration;

use similar::DiffTag;

use super::NotMerged;
use crate::changes::{
    Lines, MAX_LINES, MAX_LINES_SIZE, file_lines, is_binary, line_count, line_ops, lines_of,
    only_line_endings,
};
use crate::conflict::Labels;
use crate::conflict::markers::Marking;

/// A file's merged bytes, or why it isn't merged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Text {
    Merged(Vec<u8>),
    /// The merge with each clashing place written with markers of `marker` characters.
    Marked {
        bytes: Vec<u8>,
        marker: u8,
        places: u64,
    },
    Not(NotMerged),
}

/// The three-way merge of `base`, `ours` and `theirs` line by line; the `bool` says a comparison
/// ran past `deadline`. Each side is at most `MAX_LINES_SIZE` bytes: the caller reads them so.
/// With `labels`, a clash is `Marked` unless the two differ only in line endings, the marker
/// would pass 255, or the marked text 64 MiB.
#[must_use]
pub fn merge_text(
    base: &[u8],
    ours: &[u8],
    theirs: &[u8],
    deadline: Duration,
    labels: Option<&Labels>,
) -> (Text, bool) {
    merge_within(base, ours, theirs, deadline, labels, MAX_LINES_SIZE)
}

/// As `merge_text`, with a marked text of at most `limit` bytes.
fn merge_within(
    base: &[u8],
    ours: &[u8],
    theirs: &[u8],
    deadline: Duration,
    labels: Option<&Labels>,
    limit: u64,
) -> (Text, bool) {
    let sides = [base, ours, theirs];
    if sides
        .iter()
        .any(|s| u64::try_from(s.len()).unwrap_or(u64::MAX) > MAX_LINES_SIZE)
    {
        return (Text::Not(NotMerged::TooBig), false);
    }
    if sides.iter().any(|s| is_binary(s)) {
        return (Text::Not(NotMerged::Binary), false);
    }
    // Counted before anything is held per line.
    if sides.iter().any(|s| line_count(s) > MAX_LINES) {
        return (Text::Not(NotMerged::TooManyLines), false);
    }
    let (b, o, t) = (lines_of(base), lines_of(ours), lines_of(theirs));
    let (mut changes, ours_late) = changed(&b, &o, false, deadline);
    let (theirs_changes, theirs_late) = changed(&b, &t, true, deadline);
    changes.extend(theirs_changes);
    changes.sort_by_key(|c| c.base.start);
    let late = ours_late || theirs_late;
    if let Some((bytes, _)) = settle(&b, &o, &t, &changes, None, u64::MAX) {
        return (Text::Merged(bytes), late);
    }
    let (endings_only, endings_late) = match file_lines(ours, theirs, deadline) {
        Lines::Hunks { hunks, timed_out } => (only_line_endings(&hunks).is_some(), timed_out),
        _ => (false, false),
    };
    let late = late || endings_late;
    let not = (Text::Not(NotMerged::Overlap { endings_only }), late);
    let marking = labels
        .filter(|_| !endings_only)
        .and_then(|labels| Marking::new(labels, base, ours, theirs));
    let Some(marking) = marking else {
        return not;
    };
    match settle(&b, &o, &t, &changes, Some(&marking), limit) {
        Some((bytes, places)) => {
            let marker = marking.len;
            (
                Text::Marked {
                    bytes,
                    marker,
                    places,
                },
                late,
            )
        }
        None => not,
    }
}

/// A stretch of the base one side changed, and the lines that side has in its place.
struct Change {
    base: Range<usize>,
    side: Range<usize>,
    theirs: bool,
}

/// What one side changed, in the base's order; an insertion is an empty stretch at its place.
fn changed(
    base: &[&[u8]],
    side: &[&[u8]],
    theirs: bool,
    deadline: Duration,
) -> (Vec<Change>, bool) {
    let (ops, late) = line_ops(base, side, deadline);
    let changes = ops
        .iter()
        .filter(|op| op.tag() != DiffTag::Equal)
        .map(|op| Change {
            base: op.old_range(),
            side: op.new_range(),
            theirs,
        })
        .collect();
    (changes, late)
}

/// The merged lines and how many places were marked, from `all` in the base's order. A region both
/// sides changed with two different answers is written with `marking`, or gives `None` without.
/// `None` too as soon as the text passes `limit` bytes.
fn settle(
    base: &[&[u8]],
    ours: &[&[u8]],
    theirs: &[&[u8]],
    all: &[Change],
    marking: Option<&Marking>,
    limit: u64,
) -> Option<(Vec<u8>, u64)> {
    let fits = |out: &[u8]| u64::try_from(out.len()).is_ok_and(|n| n <= limit);
    let mut out = Vec::new();
    let mut places = 0;
    let mut done = 0;
    let mut rest = all.iter().peekable();
    while let Some(first) = rest.next() {
        // Stretches that overlap or touch join one region, transitively.
        let mut region = first.base.clone();
        let mut in_region = vec![first];
        while let Some(next) = rest.next_if(|c| c.base.start <= region.end) {
            region.end = region.end.max(next.base.end);
            in_region.push(next);
        }
        extend(&mut out, base.get(done..region.start)?);
        let ours_lines = lines_for(&region, &in_region, false, ours)?;
        let theirs_lines = lines_for(&region, &in_region, true, theirs)?;
        let taken = match (ours_lines, theirs_lines) {
            (Some(o), None) => o,
            (None, Some(t)) => t,
            (Some(o), Some(t)) if o == t => o,
            (Some(o), Some(t)) => {
                marking?.write(&mut out, o, base.get(region.clone())?, t);
                places += 1;
                &[]
            }
            (None, None) => return None,
        };
        extend(&mut out, taken);
        if !fits(&out) {
            return None;
        }
        done = region.end;
    }
    extend(&mut out, base.get(done..)?);
    fits(&out).then_some((out, places))
}

/// One side's lines for `region`, if it changed anything there: its changed lines with the
/// unchanged ones around them up to the region's edges. `None` only for stretches that don't fit.
fn lines_for<'a>(
    region: &Range<usize>,
    in_region: &[&Change],
    theirs: bool,
    side: &'a [&'a [u8]],
) -> Option<Option<&'a [&'a [u8]]>> {
    let mut own = in_region.iter().filter(|c| c.theirs == theirs);
    let Some(first) = own.next() else {
        return Some(None);
    };
    let last = own.next_back().unwrap_or(first);
    let start = first
        .side
        .start
        .checked_sub(first.base.start.checked_sub(region.start)?)?;
    let end = last
        .side
        .end
        .checked_add(region.end.checked_sub(last.base.end)?)?;
    side.get(start..end).map(Some)
}

fn extend(out: &mut Vec<u8>, lines: &[&[u8]]) {
    for line in lines {
        out.extend_from_slice(line);
    }
}

#[cfg(test)]
mod tests;
