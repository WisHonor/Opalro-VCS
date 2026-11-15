//! Marker lines: writing a clash between them, and finding them in a file a conflict was written
//! with.

use std::io;

use super::MIN_MARKER;
use crate::id::ChangeId;
use crate::text::escape;

/// The most bytes a label takes.
const MAX_LABEL: usize = 200;

/// The three labels of a marked text, each one line of plain text of at most 200 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Labels {
    pub ours: String,
    pub base: String,
    pub theirs: String,
}

/// What the middle label says of where the lines parted, at one place: it can differ from place
/// to place in one combine (a place with nothing at the start, or one the start can't tell).
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parted {
    One(ChangeId),
    Several,
    Nothing(Option<ChangeId>),
    Unknown,
}

/// What every label of one combine is made from: the two names and saves, and where the lines
/// parted. Held once for the combine; the labels are made from it for each file merged.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sides {
    /// The names as the person combined by, shown already (escaped, or `tags::shown_name`).
    pub ours: String,
    pub ours_save: ChangeId,
    pub theirs: String,
    pub theirs_save: ChangeId,
    /// Where the lines parted: one save, or several.
    pub parted: Parted,
}

#[cfg_attr(not(test), allow(dead_code))]
impl Sides {
    /// The labels for one file: `here` is what the middle says at that place (`self.parted`
    /// where the start holds a file, `Nothing(…)` where it holds nothing, `Unknown`).
    #[must_use]
    pub fn labels(&self, here: Parted) -> Labels {
        let named = |name: &str, save: &ChangeId| label(&[name, " (", &letters(save), ")"]);
        let parted = "where the lines parted";
        let base = match here {
            Parted::One(save) => label(&[parted, " (", &letters(&save), ")"]),
            Parted::Several => label(&[parted, " (several saves)"]),
            Parted::Nothing(Some(save)) => {
                label(&[parted, " (", &letters(&save), "), which held nothing here"])
            }
            Parted::Nothing(None) => label(&[parted, ", which held nothing here"]),
            Parted::Unknown => label(&[parted, ", which Opalro can't tell here"]),
        };
        Labels {
            ours: named(&self.ours, &self.ours_save),
            base,
            theirs: named(&self.theirs, &self.theirs_save),
        }
    }
}

fn letters(save: &ChangeId) -> String {
    save.to_letters()[..8].to_owned()
}

/// The pieces escaped one by one, cut to `MAX_LABEL` bytes between characters.
fn label(pieces: &[&str]) -> String {
    let mut out: String = pieces.iter().map(|p| escape(p, false)).collect();
    let mut end = out.len().min(MAX_LABEL);
    while !out.is_char_boundary(end) {
        end -= 1;
    }
    out.truncate(end);
    out
}

/// 7, or one more than the longest run of `<`, `|`, `=` or `>` starting a line in any of
/// `texts`; `None` past 255.
#[must_use]
pub fn marker_len(texts: [&[u8]; 3]) -> Option<u8> {
    let mut longest = 0;
    for text in texts {
        for line in text.split(|&b| b == b'\n') {
            if let Some(c) = line.first().copied().filter(|c| b"<|=>".contains(c)) {
                longest = longest.max(line.iter().take_while(|&&b| b == c).count());
            }
        }
    }
    u8::try_from(longest + 1)
        .ok()
        .map(|len| len.max(MIN_MARKER))
}

/// How a clash is written into a merged text: its labels, its marker length, and the ending of
/// every marker line.
pub(crate) struct Marking<'a> {
    labels: &'a Labels,
    pub len: u8,
    end: &'static [u8],
}

impl<'a> Marking<'a> {
    /// `None` when the marker would pass 255.
    pub(crate) fn new(labels: &'a Labels, base: &[u8], ours: &[u8], theirs: &[u8]) -> Option<Self> {
        let len = marker_len([base, ours, theirs])?;
        // Marker lines end as yours does, else as theirs, else as the start.
        let end = [ours, theirs, base]
            .iter()
            .find_map(|t| {
                let at = t.iter().position(|&b| b == b'\n')?;
                Some(if at > 0 && t.get(at - 1) == Some(&b'\r') {
                    &b"\r\n"[..]
                } else {
                    &b"\n"[..]
                })
            })
            .unwrap_or(b"\n");
        Some(Marking { labels, len, end })
    }

    /// Writes one clashing place: yours, the start's lines (always, even none), theirs.
    pub(crate) fn write(
        &self,
        out: &mut Vec<u8>,
        ours: &[&[u8]],
        base: &[&[u8]],
        theirs: &[&[u8]],
    ) {
        self.line(out, b'<', Some(&self.labels.ours));
        self.part(out, ours);
        self.line(out, b'|', Some(&self.labels.base));
        self.part(out, base);
        self.line(out, b'=', None);
        self.part(out, theirs);
        self.line(out, b'>', Some(&self.labels.theirs));
    }

    fn line(&self, out: &mut Vec<u8>, c: u8, label: Option<&str>) {
        out.extend(std::iter::repeat_n(c, usize::from(self.len)));
        if let Some(label) = label {
            out.push(b' ');
            out.extend_from_slice(label.as_bytes());
        }
        out.extend_from_slice(self.end);
    }

    /// A part's lines; a last line without a line break gets one, so the next marker starts a line.
    fn part(&self, out: &mut Vec<u8>, lines: &[&[u8]]) {
        for line in lines {
            out.extend_from_slice(line);
        }
        if lines.last().is_some_and(|l| !l.ends_with(b"\n")) {
            out.extend_from_slice(self.end);
        }
    }
}

/// Whether `line` (with its ending, if any) is a marker line of `len` `c`s: exactly that many at
/// the start, then a space, `\r\n`, `\n` or the end.
pub(crate) fn is_marker(line: &[u8], c: u8, len: u8) -> bool {
    let len = usize::from(len);
    let (Some(start), Some(rest)) = (line.get(..len), line.get(len..)) else {
        return false;
    };
    start.iter().all(|&b| b == c) && matches!(rest, [] | [b' ', ..] | b"\n" | b"\r\n")
}

/// How many opening and closing marker lines a file holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Left {
    pub opening: u64,
    pub closing: u64,
}

#[cfg(test)]
thread_local! {
    /// The most bytes of one line `scan` has held on this thread.
    static HELD_MOST: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The opening and closing marker lines of length `len` in what `read` gives, holding at most
/// `len + 2` bytes of any line: all `is_marker` needs to tell.
pub(crate) fn scan(read: &mut dyn io::Read, len: u8) -> io::Result<Left> {
    let most = usize::from(len) + 2;
    let mut head: Vec<u8> = Vec::with_capacity(most);
    let mut left = Left::default();
    let mut count = |head: &[u8]| {
        if is_marker(head, b'<', len) {
            left.opening += 1;
        } else if is_marker(head, b'>', len) {
            left.closing += 1;
        }
    };
    let mut buf = [0u8; 8192];
    loop {
        let n = match read.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        for &b in buf.get(..n).unwrap_or_default() {
            if head.len() < most {
                head.push(b);
                #[cfg(test)]
                HELD_MOST.set(HELD_MOST.get().max(head.len()));
            }
            if b == b'\n' {
                count(&head);
                head.clear();
            }
        }
    }
    if !head.is_empty() {
        count(&head);
    }
    Ok(left)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn a_marker_line_is_exactly_that_many_then_a_space_or_the_end() {
        for c in *b"<>" {
            let line = |s: &str| s.replace('<', &char::from(c).to_string()).into_bytes();
            for yes in ["<<<<<<< a", "<<<<<<<", "<<<<<<<\r\n", "<<<<<<<\n"] {
                assert!(is_marker(&line(yes), c, 7), "{yes:?}");
            }
            for no in [
                "<<<<<<<<",
                "<<<<<<<x",
                " <<<<<<<",
                "<<<<<<",
                "<<<<<<<\r",
                "",
            ] {
                assert!(!is_marker(&line(no), c, 7), "{no:?}");
            }
            assert!(is_marker(&line("<<<<<<<<"), c, 8));
        }
        for c in *b"<>" {
            assert!(!is_marker(b"=======", c, 7));
            assert!(!is_marker(b"|||||||", c, 7));
        }
    }

    #[test]
    fn scan_counts_opening_and_closing_lines() {
        let text = b"a\n<<<<<<< mine\nb\n||||||| base\n=======\nc\n>>>>>>> theirs\r\n\
                     <<<<<<<< longer\n>>>>>>>x\n <<<<<<<\n<<<<<<<";
        let left = scan(&mut &text[..], 7).unwrap();
        assert_eq!(
            left,
            Left {
                opening: 2,
                closing: 1
            }
        );
        // The same text with longer markers holds one opening line of 8.
        let left = scan(&mut &text[..], 8).unwrap();
        assert_eq!(left.opening, 1);
        assert_eq!(scan(&mut &b""[..], 7).unwrap(), Left::default());
    }

    fn sides(ours: &str) -> Sides {
        Sides {
            ours: ours.into(),
            ours_save: ChangeId([1; 16]),
            theirs: "side".into(),
            theirs_save: ChangeId([2; 16]),
            parted: Parted::Several,
        }
    }

    #[test]
    fn labels_say_the_names_saves_and_where_the_lines_parted() {
        let labels = sides("main").labels(Parted::Several);
        assert_eq!(
            labels,
            Labels {
                ours: "main (zyzyzyzy)".into(),
                base: "where the lines parted (several saves)".into(),
                theirs: "side (zxzxzxzx)".into(),
            }
        );
        let middle = |here| sides("main").labels(here).base;
        let one = ChangeId([3; 16]);
        assert_eq!(
            middle(Parted::One(one)),
            "where the lines parted (zwzwzwzw)"
        );
        assert_eq!(
            middle(Parted::Nothing(Some(one))),
            "where the lines parted (zwzwzwzw), which held nothing here"
        );
        assert_eq!(
            middle(Parted::Nothing(None)),
            "where the lines parted, which held nothing here"
        );
        assert_eq!(
            middle(Parted::Unknown),
            "where the lines parted, which Opalro can't tell here"
        );
    }

    #[test]
    fn labels_are_plain_one_line_and_at_most_200_bytes() {
        let name = format!("a\u{1b}\n\u{202e}{}", "é".repeat(250));
        let ours = sides(&name).labels(Parted::Several).ours;
        // `a\x1b\x0a\u{202e}` takes 17 bytes, so 200 would fall inside an `é`.
        assert_eq!(ours.len(), 199);
        assert!(ours.starts_with("a\\x1b\\x0a\\u{202e}éé"));
        assert!(ours.ends_with('é'));
        assert!(!ours.chars().any(|c| c.is_control() || c == '\u{202e}'));
    }

    #[test]
    fn the_marker_is_longer_than_any_run_that_starts_a_line() {
        let len = |text: &[u8]| marker_len([b"", text, b""]);
        assert_eq!(marker_len([b"", b"", b""]), Some(7));
        assert_eq!(len(b"<<<<<<"), Some(7));
        assert_eq!(len(b"a\n<<<<<<< x"), Some(8));
        assert_eq!(len(b"=========\n"), Some(10));
        assert_eq!(len(b"||||||||x"), Some(9));
        assert_eq!(len(b"\r\n>>>>>>>>"), Some(9));
        // Not at a line's start, or broken by another character.
        assert_eq!(len(b" <<<<<<<<<<"), Some(7));
        assert_eq!(len(b"<<<<<<=>>>>>>"), Some(7));
        assert_eq!(len(&[b'>'; 254]), Some(255));
        assert_eq!(len(&[b'>'; 255]), None);
    }

    /// 64 MiB of `<` with no line break, made as it is read.
    struct Endless(u64);

    impl io::Read for Endless {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = buf.len().min(usize::try_from(self.0).unwrap_or(usize::MAX));
            buf[..n].fill(b'<');
            self.0 -= n as u64;
            Ok(n)
        }
    }

    #[test]
    fn scan_holds_a_bounded_amount_of_a_long_line() {
        HELD_MOST.set(0);
        let left = scan(&mut Endless(64 << 20), 7).unwrap();
        assert_eq!(left, Left::default());
        assert_eq!(HELD_MOST.get(), 9);
    }
}
