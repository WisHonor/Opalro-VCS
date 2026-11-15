//! Marker lines: finding them in a file a conflict was written with.

use std::io;

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
