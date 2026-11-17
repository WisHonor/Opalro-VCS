//! What `opalro history` shows people: its heading and its rows, in columns.

use std::fmt::Write as _;
use std::io::{self, Write};

use crate::style::{self, Role, Sym};

/// A save as `opalro history` lists it, each piece escaped: its change ID, when it was written, by
/// whom, its message's first line, the lines whose newest save it is, its tags, and the first 12
/// hex digits of each parent after its first; `here` when it is the save the project is at.
pub struct SaveRow {
    pub short: String,
    pub when: String,
    pub author: String,
    pub title: String,
    pub lines: Vec<String>,
    pub tags: Vec<String>,
    pub combined: Vec<String>,
    pub here: bool,
}

impl SaveRow {
    /// The bytes of text it holds.
    fn size(&self) -> usize {
        let named: usize = (self.lines.iter().chain(&self.tags).chain(&self.combined))
            .map(String::len)
            .sum();
        self.short.len() + self.when.len() + self.author.len() + self.title.len() + named
    }
}

/// What `opalro history` lists, which its heading says.
pub enum Listed {
    /// The line the project is on, its name escaped.
    Line(String),
    /// The saves back from the one the project is at, not on a line.
    AtSave,
    /// The saves back from one named, the first row's.
    From,
}

/// `opalro history` holds rows to set its columns, but never a whole line: it stops at this many
/// rows, or at [`HELD_BYTES`] of their text, and a wider row after them runs long.
const HELD_ROWS: usize = 1_000;
/// How much of the held rows' text `opalro history` holds before it sets its columns.
const HELD_BYTES: usize = 1 << 20;

/// `opalro history`'s list, written as its rows come.
pub struct History {
    listed: Listed,
    held: Vec<SaveRow>,
    held_bytes: usize,
    widths: Option<[usize; 3]>,
}

impl History {
    pub fn new(listed: Listed) -> History {
        History {
            listed,
            held: Vec::new(),
            held_bytes: 0,
            widths: None,
        }
    }

    pub fn add(&mut self, out: &mut dyn Write, row: SaveRow) -> io::Result<()> {
        if let Some(widths) = self.widths {
            return self.write_row(out, &row, widths);
        }
        self.held_bytes += row.size();
        self.held.push(row);
        if self.held.len() >= HELD_ROWS || self.held_bytes >= HELD_BYTES {
            self.lay_out(out)?;
        }
        Ok(())
    }

    /// Writes what is still held. With nothing read, nothing is written, not even the heading.
    pub fn finish(&mut self, out: &mut dyn Write) -> io::Result<()> {
        match self.widths {
            None if !self.held.is_empty() => self.lay_out(out),
            _ => Ok(()),
        }
    }

    /// Sets the columns from the rows held, then writes the heading and them.
    fn lay_out(&mut self, out: &mut dyn Write) -> io::Result<()> {
        let s = style::out();
        let widths = self.held.iter().fold([0; 3], |w, r| {
            [
                w[0].max(style::columns(&r.short)),
                w[1].max(style::columns(&r.when)),
                w[2].max(style::columns(&r.author)),
            ]
        });
        self.widths = Some(widths);
        let held = std::mem::take(&mut self.held);
        self.held_bytes = 0;
        // Only called with a row held, the save the listing starts at, which the heading names.
        let Some(first) = held.first() else {
            return Ok(());
        };
        let heading = match &self.listed {
            Listed::Line(line) => {
                format!("Saves on line {}, newest first:", s.paint(Role::Line, line))
            }
            Listed::AtSave => format!(
                "At save {}, not on a line. Saves back from it, newest first:",
                s.paint(Role::ChangeId, &first.short)
            ),
            Listed::From => format!(
                "Saves back from save {} \"{}\", newest first:",
                s.paint(Role::ChangeId, &first.short),
                first.title
            ),
        };
        writeln!(out, "{heading}\n")?;
        for row in held {
            self.write_row(out, &row, widths)?;
        }
        Ok(())
    }

    /// A row: the place marker on the save you're at; the message cut at its end on a terminal,
    /// so that the row and its marks fit.
    fn write_row(&self, out: &mut dyn Write, row: &SaveRow, widths: [usize; 3]) -> io::Result<()> {
        let s = style::out();
        let marker = if row.here {
            s.paint(Role::Bold, s.sym(Sym::Marker))
        } else {
            " ".to_owned()
        };
        let pad = |text: &str, width: usize| " ".repeat(width.saturating_sub(style::columns(text)));
        let lead = 2 + widths[0] + 2 + widths[1] + 2 + widths[2] + 2;
        let named: usize = row
            .lines
            .iter()
            .map(|l| 4 + style::columns(l))
            .sum::<usize>()
            + row
                .tags
                .iter()
                .map(|t| 8 + style::columns(t))
                .sum::<usize>()
            + row
                .combined
                .iter()
                .map(|c| 13 + style::columns(c))
                .sum::<usize>();
        let room = s
            .width
            .map_or(usize::MAX, |w| w.saturating_sub(lead + named));
        let mut text = format!(
            "{marker} {}{}  {}{}  {}{}  {}",
            s.paint(Role::ChangeId, &row.short),
            pad(&row.short, widths[0]),
            s.paint(Role::Dim, &row.when),
            pad(&row.when, widths[1]),
            s.paint(Role::Dim, &row.author),
            pad(&row.author, widths[2]),
            s.cut_end(&row.title, room)
        );
        for line in &row.lines {
            let _ = write!(text, "  [{}]", s.paint(Role::Line, line));
        }
        for tag in &row.tags {
            let _ = write!(text, "  [tag {}]", s.paint(Role::Line, tag));
        }
        for parent in &row.combined {
            let _ = write!(text, "  [combined {}]", s.paint(Role::Fingerprint, parent));
        }
        writeln!(out, "{}", text.trim_end())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// A save may have up to 1,024 parents: every other one is marked, as every tag is, and the
    /// marks count towards what the rows held hold.
    #[test]
    fn a_save_with_the_most_parents_marks_every_other_one() {
        let combined: Vec<String> = (1..1024).map(|n| format!("{n:012x}")).collect();
        let row = SaveRow {
            short: "kxqm".to_owned(),
            when: "now".to_owned(),
            author: "Ada".to_owned(),
            title: "Octopus".to_owned(),
            lines: vec!["main".to_owned()],
            tags: Vec::new(),
            combined: combined.clone(),
            here: true,
        };
        assert!(row.size() >= 1023 * 12);
        let mut history = History::new(Listed::Line("main".to_owned()));
        let mut out = Vec::new();
        history.add(&mut out, row).unwrap();
        history.finish(&mut out).unwrap();
        let shown = String::from_utf8(out).unwrap();
        let marks: String = combined
            .iter()
            .map(|c| format!("  [combined {c}]"))
            .collect();
        assert!(
            shown.ends_with(&format!("Octopus  [main]{marks}\n")),
            "{shown}"
        );
    }
}
