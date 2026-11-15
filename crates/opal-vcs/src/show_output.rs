//! What `opalro show` shows people before its table of changes: the save, its facts and its
//! message.

use std::io::{self, Write};

use opal_core::id::Id;
use opal_core::save::{Person, Save};
use opal_core::text::escape;

use crate::output;
use crate::style::{self, Role};

/// The save `opalro show` shows: `id`, shown as `short`, with its first parent's message's first
/// line quoted as `parent_title`, its other parents' alike as `other_titles` (`None` for one that
/// can't be read), and the names of its tags, in order and not yet escaped.
pub struct Head<'a> {
    pub id: &'a Id,
    pub save: &'a Save,
    pub short: &'a str,
    pub parent_title: &'a str,
    pub other_titles: &'a [Option<String>],
    pub tags: &'a [String],
}

/// Writes the save, its facts and the rest of its message, then the line that heads its `count`
/// changes.
pub fn write_head(out: &mut dyn Write, head: &Head, count: usize, now: i64) -> io::Result<()> {
    let Head {
        id,
        save,
        short,
        parent_title,
        other_titles,
        tags,
    } = *head;
    let s = style::out();
    let person = |p: &Person| {
        let ago = output::relative_time(p.time_ms, p.tz_offset_min, now);
        format!(
            "{} <{}>, {}  {}",
            escape(&p.name, false),
            escape(&p.email, false),
            output::date_time(p.time_ms, p.tz_offset_min),
            s.paint(Role::Dim, &format!("({ago})"))
        )
    };
    writeln!(
        out,
        "Save {} {}\n",
        s.paint(Role::ChangeId, short),
        output::quoted_title(save)
    )?;
    writeln!(out, "    Author       {}", person(&save.author))?;
    writeln!(out, "    Saved by     {}", person(&save.saver))?;
    let line = save.line().map_or("none".to_owned(), |line| {
        s.paint(Role::Line, &escape(line, false))
    });
    writeln!(out, "    Line         {line}")?;
    if !tags.is_empty() {
        let names: Vec<String> = (tags.iter())
            .map(|t| s.paint(Role::Line, &opal_core::tags::shown_name(t)))
            .collect();
        writeln!(out, "    Tags         {}", names.join(", "))?;
    }
    // By fingerprint: shortening a change ID for it could read the whole line. The first parent
    // is the one compared with; a combined save has a row for each other one.
    let mut parents = save.parents.iter();
    if let Some(first) = parents.next() {
        let first = s.paint(Role::Fingerprint, &first.short_hex());
        writeln!(out, "    After        {first} {parent_title}")?;
    }
    for (at, (parent, title)) in parents.zip(other_titles).enumerate() {
        let label = if at == 0 { "Combined" } else { "" };
        let id = s.paint(Role::Fingerprint, &parent.short_hex());
        let line = format!(
            "    {label:<13}{id} {}",
            title.as_deref().unwrap_or_default()
        );
        writeln!(out, "{}", line.trim_end())?;
    }
    writeln!(
        out,
        "    Fingerprint  {}",
        s.paint(Role::Fingerprint, &id.to_hex())
    )?;
    // The rest of the message, indented eight, so that none of it can pass for a change row,
    // which is indented four.
    let message = escape(save.message.trim_end(), true);
    let body: Vec<&str> = message
        .lines()
        .skip(1)
        .skip_while(|l| l.trim().is_empty())
        .collect();
    if !body.is_empty() {
        writeln!(out)?;
    }
    for line in body {
        if line.trim().is_empty() {
            writeln!(out)?;
        } else {
            writeln!(out, "        {line}")?;
        }
    }
    if count == 0 {
        writeln!(out, "\nNo files changed.")
    } else {
        writeln!(out, "\n{} changed:\n", output::files(count))
    }
}

/// Writes, under the changes, how many places the save put in conflict, kept in conflict with
/// another version, and fixed: each count only when it isn't 0, and nothing when all are.
pub fn write_conflicts(out: &mut dyn Write, [made, changed, fixed]: [u64; 3]) -> io::Result<()> {
    let lines = [
        (
            made,
            "place became a conflict",
            "places became conflicts",
            " in this save",
        ),
        (
            changed,
            "place stayed",
            "places stayed",
            " in conflict, with another version",
        ),
        (
            fixed,
            "conflict was fixed",
            "conflicts were fixed",
            " in this save",
        ),
    ];
    let mut first = true;
    for (n, one, many, rest) in lines {
        if n == 0 {
            continue;
        }
        if std::mem::take(&mut first) {
            writeln!(out)?;
        }
        writeln!(out, "{}{rest}.", output::counted(n, one, many))?;
    }
    Ok(())
}
