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
    if let Some((bytes, _)) = settle(&b, &o, &t, &changes, None) {
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
    match settle(&b, &o, &t, &changes, Some(&marking)) {
        Some((bytes, places)) if u64::try_from(bytes.len()).is_ok_and(|n| n <= limit) => {
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
        _ => not,
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
fn settle(
    base: &[&[u8]],
    ours: &[&[u8]],
    theirs: &[&[u8]],
    all: &[Change],
    marking: Option<&Marking>,
) -> Option<(Vec<u8>, u64)> {
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
        done = region.end;
    }
    extend(&mut out, base.get(done..)?);
    Some((out, places))
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
    use crate::conflict::markers::{self, Parted, Sides};
    use crate::id::ChangeId;
    use proptest::prelude::*;

    fn merged(base: &str, ours: &str, theirs: &str) -> Text {
        merge_text(
            base.as_bytes(),
            ours.as_bytes(),
            theirs.as_bytes(),
            DEADLINE,
            None,
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
                merge_text(sides[0], sides[1], sides[2], DEADLINE, None).0,
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
            merge_text(&late, &ours, &late, DEADLINE, None).0,
            Text::Merged(ours)
        );
    }

    #[test]
    fn more_than_a_million_lines_is_not_merged() {
        let over = vec![b'\n'; MAX_LINES + 1];
        assert_eq!(
            merge_text(b"", &over, b"", DEADLINE, None).0,
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
            None,
        );
        assert!(!late);
        assert_eq!(full, clean(&ours.replace("line 1000\n", "theirs\n")));
        let (hurried, timed_out) = merge_text(
            base.as_bytes(),
            ours.as_bytes(),
            theirs.as_bytes(),
            Duration::ZERO,
            None,
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
                merge_text(sides[0], sides[1], sides[2], DEADLINE, None).0,
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
                merge_text(base, ours, theirs, DEADLINE, None).0
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

    fn labels() -> Labels {
        Labels {
            ours: "mine".into(),
            base: "start".into(),
            theirs: "side".into(),
        }
    }

    fn marked_with(base: &str, ours: &str, theirs: &str, labels: &Labels) -> Text {
        let (base, ours, theirs) = (base.as_bytes(), ours.as_bytes(), theirs.as_bytes());
        merge_text(base, ours, theirs, DEADLINE, Some(labels)).0
    }

    fn marked(base: &str, ours: &str, theirs: &str) -> Text {
        marked_with(base, ours, theirs, &labels())
    }

    fn marks(bytes: &str, marker: u8, places: u64) -> Text {
        Text::Marked {
            bytes: bytes.as_bytes().to_vec(),
            marker,
            places,
        }
    }

    /// One place written with markers of `len`, the labels `mine`, `start` and `side`, and `\n`.
    fn place(len: usize, ours: &str, base: &str, theirs: &str) -> String {
        let m = |c: &str| c.repeat(len);
        let (open, bar, equal, close) = (m("<"), m("|"), m("="), m(">"));
        format!("{open} mine\n{ours}{bar} start\n{base}{equal}\n{theirs}{close} side\n")
    }

    #[test]
    fn a_clash_is_written_with_markers_and_labels() {
        assert_eq!(
            marked("a\nb\nc\n", "a\nB\nc\n", "a\nX\nc\n"),
            marks(
                "a\n<<<<<<< mine\nB\n||||||| start\nb\n=======\nX\n>>>>>>> side\nc\n",
                7,
                1
            )
        );
        // Two places, with a change only theirs made between them taken as usual.
        assert_eq!(
            marked(
                "a\nb\nc\nd\ne\nf\ng\n",
                "a\nB\nc\nd\ne\nF\ng\n",
                "a\nX\nc\nD\ne\nY\ng\n"
            ),
            marks(
                &format!(
                    "a\n{}c\nD\ne\n{}g\n",
                    place(7, "B\n", "b\n", "X\n"),
                    place(7, "F\n", "f\n", "Y\n")
                ),
                7,
                2
            )
        );
    }

    #[test]
    fn the_middle_part_is_always_there_and_says_when_nothing_was() {
        assert_eq!(
            marked("a\nb\n", "a\nx\nb\n", "a\ny\nb\n"),
            marks(&format!("a\n{}b\n", place(7, "x\n", "", "y\n")), 7, 1)
        );
        let sides = Sides {
            ours: "main".into(),
            ours_save: ChangeId([1; 16]),
            theirs: "side".into(),
            theirs_save: ChangeId([2; 16]),
            parted: Parted::One(ChangeId([3; 16])),
        };
        assert_eq!(
            marked_with("", "x\n", "y\n", &sides.labels(Parted::Nothing(None))),
            marks(
                "<<<<<<< main (zyzyzyzy)\nx\n\
                 ||||||| where the lines parted, which held nothing here\n\
                 =======\ny\n>>>>>>> side (zxzxzxzx)\n",
                7,
                1
            )
        );
    }

    #[test]
    fn the_middle_label_differs_from_place_to_place() {
        let sides = Sides {
            ours: "main".into(),
            ours_save: ChangeId([1; 16]),
            theirs: "side".into(),
            theirs_save: ChangeId([2; 16]),
            parted: Parted::One(ChangeId([3; 16])),
        };
        // One combine, three places: the start holds the file, holds nothing, can't tell.
        let places = [
            ("b\n", sides.parted, "where the lines parted (zwzwzwzw)"),
            (
                "",
                Parted::Nothing(Some(ChangeId([3; 16]))),
                "where the lines parted (zwzwzwzw), which held nothing here",
            ),
            (
                "b\n",
                Parted::Unknown,
                "where the lines parted, which Opalro can't tell here",
            ),
        ];
        for (base, here, middle) in places {
            let want = format!(
                "<<<<<<< main (zyzyzyzy)\nB\n||||||| {middle}\n{base}=======\nX\n\
                 >>>>>>> side (zxzxzxzx)\n"
            );
            assert_eq!(
                marked_with(base, "B\n", "X\n", &sides.labels(here)),
                marks(&want, 7, 1)
            );
        }
    }

    #[test]
    fn marker_length_grows_past_the_files_own_runs() {
        let nine = "<".repeat(9);
        assert_eq!(
            marked("t\nb\n", "t\nB\n", &format!("t\nX\n{nine}\n")),
            marks(
                &format!("t\n{}", place(10, "B\n", "b\n", &format!("X\n{nine}\n"))),
                10,
                1
            )
        );
        assert_eq!(
            marked("a\nb\n", "a\nB\n=======\n", "a\nX\n"),
            marks(
                &format!("a\n{}", place(8, "B\n=======\n", "b\n", "X\n")),
                8,
                1
            )
        );
    }

    #[test]
    fn past_255_the_clash_is_not_marked() {
        let run = |n: usize| format!("a\nX\n{}\n", ">".repeat(n));
        assert_eq!(
            marked("a\nb\n", "a\nB\n", &run(254)),
            marks(
                &format!("a\n{}", place(255, "B\n", "b\n", &run(254)[2..])),
                255,
                1
            )
        );
        assert_eq!(marked("a\nb\n", "a\nB\n", &run(255)), OVERLAP);
    }

    #[test]
    fn marker_lines_end_as_your_first_line_ends() {
        let crlf = |text: &str| text.replace('\n', "\r\n");
        assert_eq!(
            marked("a\nb\n", "a\r\nB\r\n", "a\nX\n"),
            marks(
                &[
                    crlf("<<<<<<< mine\n"),
                    "a\r\nB\r\n".into(),
                    crlf("||||||| start\n"),
                    "a\nb\n".into(),
                    crlf("=======\n"),
                    "a\nX\n".into(),
                    crlf(">>>>>>> side\n"),
                ]
                .concat(),
                7,
                1
            )
        );
        // Yours without a line break: as theirs; neither: as the start; none: `\n`.
        assert_eq!(
            marked("b\n", "B", "X\r\n"),
            marks(
                &[
                    crlf("<<<<<<< mine\nB\n||||||| start\n"),
                    "b\n".into(),
                    crlf("=======\nX\n>>>>>>> side\n"),
                ]
                .concat(),
                7,
                1
            )
        );
        assert_eq!(
            marked("b\r\n", "B", "X"),
            marks(&crlf(&place(7, "B\n", "b\n", "X\n")), 7, 1)
        );
        assert_eq!(
            marked("b", "B", "X"),
            marks(&place(7, "B\n", "b\n", "X\n"), 7, 1)
        );
    }

    #[test]
    fn a_part_without_a_last_line_break_gets_one_before_the_next_marker() {
        let want = |ours| marks(&format!("a\n{}", place(7, ours, "b\n", "X\n")), 7, 1);
        assert_eq!(marked("a\nb", "a\nB", "a\nX"), want("B\n"));
        assert_eq!(marked("a\nb\n", "a\nB\n", "a\nX"), want("B\n"));
        assert_eq!(marked("a\nb", "a\n", "a\nX"), want(""));
    }

    #[test]
    fn endings_only_is_not_marked() {
        assert_eq!(
            marked("a\nb\n", "a\r\nb\r\n", "a\nb\r\n"),
            Text::Not(NotMerged::Overlap { endings_only: true })
        );
    }

    #[test]
    fn a_marked_text_past_the_limit_is_not_marked() {
        let want = format!("a\n{}c\n", place(7, "B\n", "b\n", "X\n"));
        let run = |limit: usize| {
            let labels = labels();
            let (b, o, t) = (b"a\nb\nc\n", b"a\nB\nc\n", b"a\nX\nc\n");
            merge_within(b, o, t, DEADLINE, Some(&labels), limit as u64).0
        };
        assert_eq!(run(want.len()), marks(&want, 7, 1));
        assert_eq!(run(want.len() - 1), OVERLAP);
    }

    #[test]
    fn without_labels_nothing_changes() {
        assert_eq!(merged("a\nb\nc\n", "a\nB\nc\n", "a\nX\nc\n"), OVERLAP);
        assert_eq!(merged("b", "B", "X"), OVERLAP);
        // A merge with no clash is the same with labels.
        assert_eq!(
            marked("a\nb\nc\nd\ne\n", "a\nB\nc\nd\ne\n", "a\nb\nc\nD\ne\n"),
            clean("a\nB\nc\nD\ne\n")
        );
    }

    /// A line before it is given its text: plain, or a run of one marker character alone or
    /// followed by text; and whether it ends in `\r\n`.
    #[derive(Debug, Clone)]
    enum Shape {
        Plain(u8),
        Run(u8, usize, bool),
    }

    type Line = (Shape, bool);

    fn line() -> impl Strategy<Value = Line> {
        let run = prop_oneof![8 => 1..=12usize, 1 => 13..=300usize];
        let shape = prop_oneof![
            8 => (0..4u8).prop_map(Shape::Plain),
            2 => (prop::sample::select(b"<|=>".to_vec()), run, any::<bool>())
                .prop_map(|(c, n, text)| Shape::Run(c, n, text)),
        ];
        (shape, any::<bool>())
    }

    fn ended(mut body: Vec<u8>, crlf: bool) -> Vec<u8> {
        body.extend_from_slice(if crlf { b"\r\n" } else { b"\n" });
        body
    }

    fn strip(text: &mut Vec<u8>) {
        if text.ends_with(b"\n") {
            text.pop();
            if text.ends_with(b"\r") {
                text.pop();
            }
        }
    }

    /// Lines that may repeat: four plain ones, and runs of a marker character.
    fn any_text() -> impl Strategy<Value = Vec<u8>> {
        (prop::collection::vec(line(), 0..=30), any::<bool>()).prop_map(|(lines, open)| {
            let mut out: Vec<u8> = lines
                .iter()
                .flat_map(|(shape, crlf)| {
                    let body = match shape {
                        Shape::Plain(k) => vec![b'a' + k],
                        Shape::Run(c, n, text) => {
                            let mut body = vec![*c; *n];
                            if *text {
                                body.extend_from_slice(b" x");
                            }
                            body
                        }
                    };
                    ended(body, *crlf)
                })
                .collect();
            if open {
                strip(&mut out);
            }
            out
        })
    }

    /// Gives every line text of its own, so no line repeats.
    #[derive(Default)]
    struct Namer {
        next: usize,
        used: std::collections::HashSet<Vec<u8>>,
    }

    impl Namer {
        fn line(&mut self, (shape, crlf): &Line) -> Vec<u8> {
            self.next += 1;
            let n = self.next;
            let mut body = match shape {
                Shape::Plain(_) => format!("w{n}").into_bytes(),
                Shape::Run(c, len, text) => {
                    let mut body = vec![*c; *len];
                    if *text {
                        body.extend(format!(" t{n}").bytes());
                    }
                    body
                }
            };
            if !self.used.insert(body.clone()) {
                body.extend(format!(" u{n}").bytes());
                self.used.insert(body.clone());
            }
            ended(body, *crlf)
        }
    }

    type Segment = (Vec<Line>, Option<(Vec<Line>, Vec<Line>)>);

    /// Base, yours and theirs, of lines that don't repeat, where every stretch one side changed
    /// the other changed too, differently: kept stretches between clashing ones.
    fn clashing() -> impl Strategy<Value = [Vec<u8>; 3]> {
        let lines = || prop::collection::vec(line(), 0..=3);
        let segment = (lines(), prop::option::weighted(0.5, (lines(), lines())));
        (prop::collection::vec(segment, 1..=6), any::<[bool; 4]>())
            .prop_filter_map("no clash", |(segments, open)| {
                build_clashing(&segments, open)
            })
    }

    fn build_clashing(segments: &[Segment], open: [bool; 4]) -> Option<[Vec<u8>; 3]> {
        let mut namer = Namer::default();
        let mut texts: [Vec<Vec<u8>>; 3] = Default::default();
        // Whether the last stretch clashes, and which texts have lines in it.
        let mut last = None;
        for (base, both) in segments {
            let base: Vec<Vec<u8>> = base.iter().map(|l| namer.line(l)).collect();
            match both {
                None if base.is_empty() => {}
                None => {
                    for text in &mut texts {
                        text.extend(base.iter().cloned());
                    }
                    last = Some(None);
                }
                Some((ours, theirs)) => {
                    let ours: Vec<Vec<u8>> = ours.iter().map(|l| namer.line(l)).collect();
                    let theirs: Vec<Vec<u8>> = theirs.iter().map(|l| namer.line(l)).collect();
                    let has = [!base.is_empty(), !ours.is_empty(), !theirs.is_empty()];
                    // Two empty would be one side changing nothing, or both the same.
                    if has.iter().filter(|h| !**h).count() > 1 {
                        return None;
                    }
                    texts[0].extend(base);
                    texts[1].extend(ours);
                    texts[2].extend(theirs);
                    last = Some(Some(has));
                }
            }
        }
        let mut out = texts.map(|t| t.concat());
        match last? {
            None if open[3] => out.iter_mut().for_each(strip),
            None => {}
            Some(has) => {
                for ((text, has), open) in out.iter_mut().zip(has).zip(open) {
                    if has && open {
                        strip(text);
                    }
                }
            }
        }
        // At least one clash.
        (out[1] != out[0]).then_some(out)
    }

    /// Base, yours and theirs, of lines that don't repeat, each side any edit of the base.
    fn edited() -> impl Strategy<Value = [Vec<u8>; 3]> {
        prop::collection::vec(line(), 0..=15)
            .prop_flat_map(|base| {
                let n = base.len();
                let edit = || {
                    let new = prop_oneof![
                        3 => Just(Vec::new()),
                        1 => prop::collection::vec(line(), 1..=2),
                    ];
                    prop::collection::vec((prop::bool::weighted(0.75), new), n + 1)
                };
                (Just(base), edit(), edit(), any::<[bool; 3]>())
            })
            .prop_map(|(base, ours, theirs, open)| {
                let mut namer = Namer::default();
                let base: Vec<Vec<u8>> = base.iter().map(|l| namer.line(l)).collect();
                let mut side = |edit: &[(bool, Vec<Line>)]| {
                    let mut out: Vec<Vec<u8>> = Vec::new();
                    for (i, (keep, new)) in edit.iter().enumerate() {
                        out.extend(new.iter().map(|l| namer.line(l)));
                        if let Some(line) = base.get(i).filter(|_| *keep) {
                            out.push(line.clone());
                        }
                    }
                    out.concat()
                };
                let (ours, theirs) = (side(&ours), side(&theirs));
                let mut texts = [base.concat(), ours, theirs];
                for (text, open) in texts.iter_mut().zip(open) {
                    if open {
                        strip(text);
                    }
                }
                texts
            })
    }

    fn runner() -> proptest::test_runner::TestRunner {
        use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};
        let config = Config {
            cases: 256,
            failure_persistence: None,
            ..Config::default()
        };
        TestRunner::new_with_rng(config, TestRng::deterministic_rng(RngAlgorithm::ChaCha))
    }

    /// Whether `line` is a marker line: exactly `len` of `c`, then a space or the line's end.
    fn is_marker_line(line: &[u8], c: u8, len: u8) -> bool {
        let len = usize::from(len);
        line.len() >= len
            && line[..len].iter().all(|&b| b == c)
            && matches!(&line[len..], [] | [b' ', ..] | b"\n" | b"\r\n")
    }

    fn lines(text: &[u8]) -> impl Iterator<Item = &[u8]> {
        text.split_inclusive(|&b| b == b'\n')
    }

    /// The marked text with one part of every marked place taken: 0 yours, 1 the start's,
    /// 2 theirs.
    fn take(bytes: &[u8], marker: u8, part: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut inside = None;
        for line in lines(bytes) {
            if is_marker_line(line, b'<', marker) {
                inside = Some(0);
            } else if is_marker_line(line, b'|', marker) {
                inside = Some(1);
            } else if is_marker_line(line, b'=', marker) {
                inside = Some(2);
            } else if is_marker_line(line, b'>', marker) {
                inside = None;
            } else if inside.is_none_or(|p| p == part) {
                out.extend_from_slice(line);
            }
        }
        out
    }

    /// How marker lines end: as yours' first line, else theirs', else the start's, else `\n`.
    fn first_ending(texts: [&[u8]; 3]) -> &'static [u8] {
        for text in texts {
            match text.iter().position(|&b| b == b'\n') {
                Some(at) if at > 0 && text[at - 1] == b'\r' => return b"\r\n",
                Some(_) => return b"\n",
                None => {}
            }
        }
        b"\n"
    }

    /// `got` is `want`, or `want` with the line break added after its last line, which had none,
    /// before a marker line.
    fn same_but_for_a_break(got: &[u8], want: &[u8], end: &[u8]) -> bool {
        got == want || (!want.is_empty() && !want.ends_with(b"\n") && got == [want, end].concat())
    }

    fn has_long_run(texts: [&[u8]; 3]) -> bool {
        texts.iter().flat_map(|t| lines(t)).any(|line| {
            line.first()
                .is_some_and(|&c| line.iter().take_while(|&&b| b == c).count() > 254)
        })
    }

    const KINDS: [&str; 13] = [
        "plain",
        "< alone",
        "< with text",
        "| alone",
        "| with text",
        "= alone",
        "= with text",
        "> alone",
        "> with text",
        "\\r\\n",
        "\\n",
        "no last break",
        "a run past 254",
    ];

    type Seen = std::collections::BTreeMap<String, usize>;

    fn bump(seen: &mut Seen, kind: &str) {
        *seen.entry(kind.to_owned()).or_default() += 1;
    }

    fn count_lines(seen: &mut Seen, texts: [&[u8]; 3]) {
        for line in texts.iter().flat_map(|t| lines(t)) {
            let body = match line.strip_suffix(b"\n") {
                Some(body) => {
                    let crlf = body.ends_with(b"\r");
                    bump(seen, if crlf { "\\r\\n" } else { "\\n" });
                    body.strip_suffix(b"\r").unwrap_or(body)
                }
                None => {
                    bump(seen, "no last break");
                    line
                }
            };
            match body.first() {
                Some(&c) if b"<|=>".contains(&c) => {
                    let run = body.iter().take_while(|&&b| b == c).count();
                    if run > 254 {
                        bump(seen, "a run past 254");
                    }
                    let rest = if run == body.len() {
                        "alone"
                    } else {
                        "with text"
                    };
                    bump(seen, &format!("{} {rest}", char::from(c)));
                }
                _ => bump(seen, "plain"),
            }
        }
    }

    fn assert_seen(seen: &Seen, outcomes: &[&str]) {
        println!("{seen:?}");
        for kind in KINDS.iter().chain(outcomes) {
            let n = seen.get(*kind).copied().unwrap_or(0);
            assert!(n >= 3, "{kind}: {n} in {seen:?}");
        }
    }

    #[test]
    fn every_marked_text_reads_back_with_its_places() {
        let seen = std::cell::RefCell::new(Seen::new());
        runner()
            .run(&(any_text(), any_text(), any_text()), |(b, o, t)| {
                let mut seen = seen.borrow_mut();
                count_lines(&mut seen, [&b, &o, &t]);
                match merge_text(&b, &o, &t, DEADLINE, Some(&labels())).0 {
                    Text::Marked {
                        bytes,
                        marker,
                        places,
                    } => {
                        bump(&mut seen, "marked");
                        prop_assert!(places >= 1);
                        let left = markers::scan(&mut &bytes[..], marker).unwrap();
                        prop_assert_eq!((left.opening, left.closing), (places, places));
                        for line in [&b, &o, &t].iter().flat_map(|t| lines(t)) {
                            for c in *b"<|=>" {
                                prop_assert!(!is_marker_line(line, c, marker));
                            }
                        }
                    }
                    Text::Not(NotMerged::Overlap {
                        endings_only: false,
                    }) if has_long_run([&b, &o, &t]) => bump(&mut seen, "past 255"),
                    _ => {}
                }
                Ok(())
            })
            .unwrap();
        assert_seen(&seen.into_inner(), &["marked", "past 255"]);
    }

    #[test]
    fn taking_one_side_in_every_marked_place_gives_that_side() {
        let seen = std::cell::RefCell::new(Seen::new());
        runner()
            .run(&clashing(), |[b, o, t]| {
                let mut seen = seen.borrow_mut();
                count_lines(&mut seen, [&b, &o, &t]);
                let end = first_ending([&o, &t, &b]);
                match merge_text(&b, &o, &t, DEADLINE, Some(&labels())).0 {
                    Text::Marked { bytes, marker, .. } => {
                        bump(&mut seen, "marked");
                        for (part, want) in [(0, &o), (1, &b), (2, &t)] {
                            let got = take(&bytes, marker, part);
                            prop_assert!(
                                same_but_for_a_break(&got, want, end),
                                "part {}: {:?}\nin {:?}",
                                part,
                                String::from_utf8_lossy(&got),
                                String::from_utf8_lossy(&bytes)
                            );
                        }
                    }
                    Text::Not(NotMerged::Overlap {
                        endings_only: false,
                    }) if has_long_run([&b, &o, &t]) => bump(&mut seen, "past 255"),
                    other => prop_assert!(false, "{:?}", other),
                }
                Ok(())
            })
            .unwrap();
        assert_seen(&seen.into_inner(), &["marked", "past 255"]);
    }

    /// The stretches of `base` that `side` changed, by a longest common subsequence of lines:
    /// `(from, to, lines, side)`.
    fn stretches<'a>(
        base: &[&[u8]],
        side: &[&'a [u8]],
        which: usize,
    ) -> Vec<(usize, usize, Vec<&'a [u8]>, usize)> {
        let (n, k) = (base.len(), side.len());
        let mut common = vec![vec![0usize; k + 1]; n + 1];
        for i in (0..n).rev() {
            for j in (0..k).rev() {
                common[i][j] = if base[i] == side[j] {
                    common[i + 1][j + 1] + 1
                } else {
                    common[i + 1][j].max(common[i][j + 1])
                };
            }
        }
        let mut out = Vec::new();
        let (mut i, mut j, mut from) = (0, 0, (0, 0));
        loop {
            let at_end = i == n || j == k;
            if at_end || base[i] == side[j] {
                let (to_i, to_j) = if at_end { (n, k) } else { (i, j) };
                if from != (to_i, to_j) {
                    out.push((from.0, to_i, side[from.1..to_j].to_vec(), which));
                }
                if at_end {
                    return out;
                }
                (i, j) = (i + 1, j + 1);
                from = (i, j);
            } else if common[i + 1][j] >= common[i][j + 1] {
                i += 1;
            } else {
                j += 1;
            }
        }
    }

    /// A three-way merge with every clash settled by yours.
    fn ours_settled(base: &[u8], ours: &[u8], theirs: &[u8]) -> Vec<u8> {
        let s: Vec<&[u8]> = lines(base).collect();
        let o: Vec<&[u8]> = lines(ours).collect();
        let t: Vec<&[u8]> = lines(theirs).collect();
        let mut all = stretches(&s, &o, 0);
        all.extend(stretches(&s, &t, 1));
        all.sort_by_key(|h| (h.0, h.1));
        let mut out: Vec<&[u8]> = Vec::new();
        let (mut done, mut k) = (0, 0);
        while k < all.len() {
            let (lo, mut hi, mut end) = (all[k].0, all[k].1, k + 1);
            while end < all.len() && all[end].0 <= hi {
                hi = hi.max(all[end].1);
                end += 1;
            }
            let region = &all[k..end];
            let made = |which: usize| {
                let mut at = lo;
                let mut lines: Vec<&[u8]> = Vec::new();
                for h in region.iter().filter(|h| h.3 == which) {
                    lines.extend(&s[at..h.0]);
                    lines.extend(&h.2);
                    at = h.1;
                }
                lines.extend(&s[at..hi]);
                region.iter().any(|h| h.3 == which).then_some(lines)
            };
            out.extend(&s[done..lo]);
            out.extend(made(0).or_else(|| made(1)).unwrap_or_default());
            (done, k) = (hi, end);
        }
        out.extend(&s[done..]);
        out.concat()
    }

    #[test]
    fn taking_ours_everywhere_agrees_with_a_model() {
        let seen = std::cell::RefCell::new(Seen::new());
        runner()
            .run(&edited(), |[b, o, t]| {
                let mut seen = seen.borrow_mut();
                count_lines(&mut seen, [&b, &o, &t]);
                let want = ours_settled(&b, &o, &t);
                let end = first_ending([&o, &t, &b]);
                match merge_text(&b, &o, &t, DEADLINE, Some(&labels())).0 {
                    Text::Marked { bytes, marker, .. } => {
                        bump(&mut seen, "marked");
                        let got = take(&bytes, marker, 0);
                        prop_assert!(
                            same_but_for_a_break(&got, &want, end),
                            "{:?}\nwant {:?}",
                            String::from_utf8_lossy(&bytes),
                            String::from_utf8_lossy(&want)
                        );
                    }
                    Text::Merged(merged) => {
                        bump(&mut seen, "merged");
                        prop_assert_eq!(merged, want);
                    }
                    Text::Not(NotMerged::Overlap {
                        endings_only: false,
                    }) if has_long_run([&b, &o, &t]) => bump(&mut seen, "past 255"),
                    other => prop_assert!(false, "{:?}", other),
                }
                Ok(())
            })
            .unwrap();
        assert_seen(&seen.into_inner(), &["marked", "merged", "past 255"]);
    }
}
