//! The three-way merge of one text file, line by line.

use std::ops::Range;
use std::time::Duration;

use similar::DiffTag;

use super::NotMerged;
use crate::changes::{
    Lines, MAX_LINES, MAX_LINES_SIZE, file_lines, is_binary, line_count, line_ops, lines_of,
    only_line_endings,
};

/// A file's merged bytes, or why it isn't merged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Text {
    Merged(Vec<u8>),
    Not(NotMerged),
}

/// The three-way merge of `base`, `ours` and `theirs` line by line; the `bool` says a comparison
/// ran past `deadline`. Each side is at most `MAX_LINES_SIZE` bytes: the caller reads them so.
#[must_use]
pub fn merge_text(base: &[u8], ours: &[u8], theirs: &[u8], deadline: Duration) -> (Text, bool) {
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
    let late = ours_late || theirs_late;
    match settle(&b, &o, &t, changes) {
        Some(bytes) => (Text::Merged(bytes), late),
        None => {
            let (endings_only, endings_late) = match file_lines(ours, theirs, deadline) {
                Lines::Hunks { hunks, timed_out } => {
                    (only_line_endings(&hunks).is_some(), timed_out)
                }
                _ => (false, false),
            };
            let not = NotMerged::Overlap { endings_only };
            (Text::Not(not), late || endings_late)
        }
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

/// The merged lines, or `None` when a region both sides changed has two different answers.
fn settle(
    base: &[&[u8]],
    ours: &[&[u8]],
    theirs: &[&[u8]],
    mut all: Vec<Change>,
) -> Option<Vec<u8>> {
    all.sort_by_key(|c| c.base.start);
    let mut out = Vec::new();
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
            _ => return None,
        };
        extend(&mut out, taken);
        done = region.end;
    }
    extend(&mut out, base.get(done..)?);
    Some(out)
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
mod tests {
    use super::*;
    use crate::changes::{DEADLINE, MAX_LINES, MAX_LINES_SIZE};
    use proptest::prelude::*;

    fn merged(base: &str, ours: &str, theirs: &str) -> Text {
        merge_text(
            base.as_bytes(),
            ours.as_bytes(),
            theirs.as_bytes(),
            DEADLINE,
        )
        .0
    }

    fn clean(bytes: &str) -> Text {
        Text::Merged(bytes.as_bytes().to_vec())
    }

    const OVERLAP: Text = Text::Not(NotMerged::Overlap {
        endings_only: false,
    });

    #[test]
    fn edits_far_apart_are_both_kept() {
        assert_eq!(
            merged("a\nb\nc\nd\ne\n", "a\nB\nc\nd\ne\n", "a\nb\nc\nD\ne\n"),
            clean("a\nB\nc\nD\ne\n")
        );
    }

    #[test]
    fn edits_that_touch_conflict() {
        assert_eq!(
            merged("a\nb\nc\nd\n", "a\nB\nc\nd\n", "a\nb\nC\nd\n"),
            OVERLAP
        );
    }

    #[test]
    fn two_insertions_at_one_place_conflict_unless_equal() {
        assert_eq!(merged("a\nb\n", "a\nx\nb\n", "a\ny\nb\n"), OVERLAP);
        assert_eq!(
            merged("a\nb\n", "a\nx\nb\n", "a\nx\nb\n"),
            clean("a\nx\nb\n")
        );
    }

    #[test]
    fn the_same_change_on_both_sides_is_taken_once() {
        assert_eq!(
            merged("a\nb\nc\nd\n", "a\nB\nc\n", "a\nB\nc\n"),
            clean("a\nB\nc\n")
        );
    }

    #[test]
    fn line_endings_are_bytes() {
        assert_eq!(
            merged("a\nb\nc\nd\n", "a\r\nb\r\nc\r\nd\r\n", "A\nb\nc\nd\n"),
            OVERLAP
        );
        assert_eq!(
            merged("a\nb\n", "a\r\nb\r\n", "a\nb\r\n"),
            Text::Not(NotMerged::Overlap { endings_only: true })
        );
    }

    #[test]
    fn a_zero_byte_in_the_first_8000_bytes_is_not_text() {
        let mut late = vec![b'a'; 7999];
        late.push(0);
        let plain = b"a\n".to_vec();
        for sides in [
            [&late, &plain, &plain],
            [&plain, &late, &plain],
            [&plain, &plain, &late],
        ] {
            assert_eq!(
                merge_text(sides[0], sides[1], sides[2], DEADLINE).0,
                Text::Not(NotMerged::Binary)
            );
        }
    }

    #[test]
    fn a_zero_byte_after_8000_bytes_is_text() {
        let mut late = vec![b'a'; 8000];
        late.extend_from_slice(b"\0\n");
        let mut ours = late.clone();
        ours.extend_from_slice(b"more\n");
        assert_eq!(
            merge_text(&late, &ours, &late, DEADLINE).0,
            Text::Merged(ours)
        );
    }

    #[test]
    fn more_than_a_million_lines_is_not_merged() {
        let over = vec![b'\n'; MAX_LINES + 1];
        assert_eq!(
            merge_text(b"", &over, b"", DEADLINE).0,
            Text::Not(NotMerged::TooManyLines)
        );
    }

    #[test]
    fn a_last_line_without_a_line_feed_merges() {
        assert_eq!(merged("a\nb\nc", "A\nb\nc", "a\nb\nC"), clean("A\nb\nC"));
    }

    #[test]
    fn a_comparison_past_its_deadline_is_coarser_never_wrong() {
        // Ours edits around theirs, so a hurried comparison of ours swallows theirs.
        let base: String = (0..2000).map(|i| format!("line {i}\n")).collect();
        let ours = base
            .replace("line 10\n", "ours\n")
            .replace("line 1990\n", "ours too\n");
        let theirs = base.replace("line 1000\n", "theirs\n");
        let (full, late) = merge_text(
            base.as_bytes(),
            ours.as_bytes(),
            theirs.as_bytes(),
            DEADLINE,
        );
        assert!(!late);
        assert_eq!(full, clean(&ours.replace("line 1000\n", "theirs\n")));
        let (hurried, timed_out) = merge_text(
            base.as_bytes(),
            ours.as_bytes(),
            theirs.as_bytes(),
            Duration::ZERO,
        );
        assert!(timed_out);
        assert_eq!(hurried, OVERLAP);
    }

    #[test]
    fn a_side_over_64_mib_is_too_big() {
        // Zeros too, so the size is seen before the zero byte would make it binary.
        let over = vec![0; usize::try_from(MAX_LINES_SIZE).unwrap() + 1];
        let plain: &[u8] = b"a\n";
        for sides in [
            [&over[..], plain, plain],
            [plain, &over[..], plain],
            [plain, plain, &over[..]],
        ] {
            assert_eq!(
                merge_text(sides[0], sides[1], sides[2], DEADLINE).0,
                Text::Not(NotMerged::TooBig)
            );
        }
    }

    /// 0 to 30 lines of 4 kinds, so they repeat, ending in `\n` or `\r\n`, the last maybe in
    /// neither.
    fn text() -> impl Strategy<Value = Vec<u8>> {
        let line = prop::sample::select(vec!["a", "b", "c", "d"]);
        let ending = prop::sample::select(vec!["\n", "\r\n"]);
        (prop::collection::vec((line, ending), 0..=30), any::<bool>()).prop_map(
            |(lines, open_end)| {
                let mut out: Vec<u8> = lines
                    .iter()
                    .flat_map(|(l, e)| [l.as_bytes(), e.as_bytes()].concat())
                    .collect();
                if open_end && out.ends_with(b"\n") {
                    out.pop();
                    if out.ends_with(b"\r") {
                        out.pop();
                    }
                }
                out
            },
        )
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]
        #[test]
        fn text_merge_properties(b in text(), o in text(), t in text()) {
            let merge = |base: &[u8], ours: &[u8], theirs: &[u8]| {
                merge_text(base, ours, theirs, DEADLINE).0
            };
            prop_assert_eq!(merge(&b, &o, &o), Text::Merged(o.clone()));
            prop_assert_eq!(merge(&b, &b, &t), Text::Merged(t.clone()));
            prop_assert_eq!(merge(&b, &o, &b), Text::Merged(o.clone()));
            match (merge(&b, &o, &t), merge(&b, &t, &o)) {
                (Text::Merged(one), Text::Merged(other)) => prop_assert_eq!(one, other),
                (Text::Not(_), Text::Not(_)) => {}
                (one, other) => prop_assert!(false, "{:?} but swapped {:?}", one, other),
            }
        }
    }
}
