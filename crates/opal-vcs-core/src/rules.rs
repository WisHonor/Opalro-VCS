//! The project's rules in `opalro.toml` (docs/storage-format.md section 12): what Opalro reads from
//! it, and what it passes over. Nothing in the file ever stops a command.

use std::ffi::OsStr;
use std::io;
use std::path::Path;

use opal_platform::dir::{Dir, EntryType};
use toml::{Table, Value};

use crate::changes::read_capped;
use crate::chunks::{CHUNK_FROM, CHUNK_UP_TO};
use crate::drive::Drive;
use crate::ignore::{Decision, Matcher, Rules as IgnoreRules};
use crate::listing::{EntryKind, Listing};
use crate::safe_fs;
use crate::scope::ancestors;
use crate::store::{Store, StoreError};
use crate::text::{escape, thousands};
use crate::warning::{self, Warning};

/// The longest `opalro.toml` Opalro reads.
pub const MAX_RULES_FILE_BYTES: u64 = 1 << 20;
/// The longest `preset` label, in bytes.
const MAX_PRESET_BYTES: usize = 64;
/// How much of a name or value from the file a warning shows, in characters.
const SHOWN_CHARS: usize = 64;
/// How many passed-over names one warning lists.
const NAMES_LISTED: usize = 20;

/// The units of a size, the 1,024 family first, each family largest first.
const UNITS: [(&str, u64); 9] = [
    ("TiB", 1 << 40),
    ("GiB", 1 << 30),
    ("MiB", 1 << 20),
    ("KiB", 1 << 10),
    ("TB", 1_000_000_000_000),
    ("GB", 1_000_000_000),
    ("MB", 1_000_000),
    ("KB", 1_000),
    ("B", 1),
];

const UNKNOWN: &str = "Opalro doesn't know this key; a newer Opalro may have written it";
const NO_FILES: &str =
    "it has no usable `files`: a list of one or more patterns, none of them empty";

/// Settings for the files a rule's patterns match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub files: Vec<String>,
    pub lockable: Option<bool>,
    pub conflict_markers: Option<bool>,
}

/// What `opalro.toml` says, with Opalro's own value for each setting it doesn't give.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rules {
    pub preset: Option<String>,
    /// A file of this size or more (up to 128 GiB) is stored in chunks.
    pub chunk_files_over: u64,
    /// A save that adds more new bytes than this, or more new files than the other, asks first.
    pub ask_over_bytes: u64,
    pub ask_over_files: u64,
    pub rules: Vec<Rule>,
}

impl Default for Rules {
    fn default() -> Rules {
        Rules {
            preset: None,
            chunk_files_over: CHUNK_FROM,
            ask_over_bytes: 100_000_000,
            ask_over_files: 10_000,
            rules: vec![],
        }
    }
}

/// How rule patterns compare names: ASCII case ignored on every system, since what a rule decides
/// ends up in a save and must not depend on the computer.
const RULE_DRIVE: Drive = Drive {
    ignores_case: true,
    composes_unicode: false,
};

/// Whether a file both lines changed may be merged line by line. `theirs`: only the `opalro.toml`
/// of the save being combined in is the reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineMerge {
    Yes,
    /// Its rule says `conflict-markers = false`.
    RuleSaysNo {
        theirs: bool,
    },
    /// `opalro.toml`, or a part of it, was passed over: what it says can't be known, so no file is.
    RulesNotWhole {
        theirs: bool,
    },
}

/// What a combine asks about merging: the folder's rules and those of the save combined in.
#[derive(Debug)]
pub(crate) struct RuleMatcher {
    ours: FileRules,
    theirs: FileRules,
}

impl RuleMatcher {
    #[must_use]
    pub fn new(ours: &RulesRead, theirs: &RulesRead) -> RuleMatcher {
        RuleMatcher {
            ours: FileRules::new(ours),
            theirs: FileRules::new(theirs),
        }
    }

    /// Whether the file at `path` (`a/b/c`) may be merged line by line: only when both files allow
    /// it. `was` is the name a moved file had at the starting point, whose rule must allow it too.
    /// A rule's no comes before a file not read whole, and the folder's file before the other.
    #[must_use]
    pub fn line_merge(&self, path: &str, was: Option<&str>) -> LineMerge {
        let says_no = |rules: &FileRules| {
            !rules.conflict_markers(path) || was.is_some_and(|was| !rules.conflict_markers(was))
        };
        if says_no(&self.ours) {
            LineMerge::RuleSaysNo { theirs: false }
        } else if says_no(&self.theirs) {
            LineMerge::RuleSaysNo { theirs: true }
        } else if !self.ours.whole {
            LineMerge::RulesNotWhole { theirs: false }
        } else if !self.theirs.whole {
            LineMerge::RulesNotWhole { theirs: true }
        } else {
            LineMerge::Yes
        }
    }
}

/// One `opalro.toml`'s `[[rule]]` tables, each turned into a matcher once.
#[derive(Debug)]
struct FileRules {
    rules: Vec<(Matcher, Option<bool>)>,
    /// Whether the whole of the file was read.
    whole: bool,
}

impl FileRules {
    fn new(read: &RulesRead) -> FileRules {
        let rules = read.rules.rules.iter().map(|rule| {
            let mut text = Vec::new();
            // A pattern can't be split across lines of the one rule file it becomes.
            for pattern in rule.files.iter().filter(|p| !p.contains(['\n', '\r'])) {
                // Every line starts after a line break, so no byte-order mark is taken off.
                text.push(b'\n');
                if pattern.starts_with('#') {
                    text.push(b'\\');
                }
                fold_pattern(pattern.as_bytes(), &mut text);
            }
            let mut matcher = Matcher::new(RULE_DRIVE);
            matcher.enter("", vec![IgnoreRules::parse(&text, "", false)]);
            (matcher, rule.conflict_markers)
        });
        FileRules {
            rules: rules.collect(),
            whole: read.skipped.is_none() && read.ignored.is_empty(),
        }
    }

    /// What the last rule that matches `path` and gives `conflict-markers` says, `true` when none
    /// does.
    fn conflict_markers(&self, path: &str) -> bool {
        let says = |(matcher, markers): &(Matcher, Option<bool>)| {
            markers.filter(|_| {
                matcher.decide(path, false) == Decision::Ignored
                    || ancestors(path).any(|f| matcher.decide(f, true) == Decision::Ignored)
            })
        };
        self.rules.iter().rev().find_map(says).unwrap_or(true)
    }
}

/// Writes `pattern` with each ASCII letter that stands for itself lowered, so that it matches
/// either case as the lowered name does. A range's ends and a class's name are kept as written:
/// they take either case already, and lowering them could change what they hold.
fn fold_pattern(mut rest: &[u8], out: &mut Vec<u8>) {
    while let Some((&c, after)) = rest.split_first() {
        out.push(c.to_ascii_lowercase());
        rest = after;
        match c {
            b'\\' => {
                if let Some((&escaped, after)) = rest.split_first() {
                    out.push(escaped.to_ascii_lowercase());
                    rest = after;
                }
            }
            b'[' => rest = fold_bracket(rest, out),
            _ => {}
        }
    }
}

/// Writes the inside of a `[…]` whose `[` is written, read as the ignore matcher reads it, and
/// gives what follows its `]`.
fn fold_bracket<'a>(mut rest: &'a [u8], out: &mut Vec<u8>) -> &'a [u8] {
    if let Some((&c @ (b'!' | b'^'), after)) = rest.split_first() {
        out.push(c);
        rest = after;
    }
    let mut first = true;
    loop {
        let Some(close) = rest.iter().position(|&b| b == b']') else {
            // It never closes, so the pattern matches nothing whatever it holds.
            out.extend_from_slice(rest);
            return &[];
        };
        if !first && close == 0 {
            out.push(b']');
            return rest.get(1..).unwrap_or_default();
        }
        first = false;
        let class = rest.get(..close).and_then(|t| t.strip_prefix(b"[:"));
        if class.is_some_and(|name| name.ends_with(b":")) {
            let (class, after) = split(rest, close.saturating_add(1));
            out.extend_from_slice(class);
            rest = after;
            continue;
        }
        let one = member_len(rest);
        let range = rest
            .get(one..)
            .and_then(|t| t.strip_prefix(b"-"))
            .filter(|t| t.first().is_some_and(|&b| b != b']'));
        match range {
            Some(tail) => {
                let ends = one.saturating_add(1).saturating_add(member_len(tail));
                let (member, after) = split(rest, ends);
                out.extend_from_slice(member);
                rest = after;
            }
            None => {
                let (member, after) = split(rest, one);
                out.extend(member.iter().map(u8::to_ascii_lowercase));
                rest = after;
            }
        }
    }
}

fn split(text: &[u8], at: usize) -> (&[u8], &[u8]) {
    text.split_at_checked(at).unwrap_or((text, &[]))
}

/// How many bytes the member at the start of a `[…]`'s text takes: two after a `\`.
fn member_len(text: &[u8]) -> usize {
    match text {
        [b'\\', _, ..] => 2,
        [] => 0,
        _ => 1,
    }
}

/// A size as `opalro.toml` writes it, in bytes: digits, any number of spaces, then a unit. `None`
/// for anything else, or a size that doesn't fit 64 bits.
#[must_use]
pub fn parse_size(text: &str) -> Option<u64> {
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    let (number, unit) = text.split_at_checked(digits)?;
    let unit = unit.trim_start_matches(' ');
    let (_, times) = UNITS.iter().find(|(name, _)| *name == unit)?;
    // Digits only, so a sign or an empty number fails here.
    number.parse::<u64>().ok()?.checked_mul(*times)
}

/// A setting's size as `opalro.toml` would write it: the largest unit that gives it exactly, the
/// 1,024 family first.
#[must_use]
pub fn size_text(bytes: u64) -> String {
    let (name, times) = UNITS
        .iter()
        .find(|(_, times)| bytes.checked_rem(*times) == Some(0) && (bytes > 0 || *times == 1))
        .unwrap_or(&("B", 1));
    format!("{} {name}", bytes.checked_div(*times).unwrap_or(bytes))
}

/// What an `opalro.toml`'s text means, and each name passed over with why. `Err` says why the text
/// isn't TOML.
pub fn parse(text: &[u8]) -> Result<(Rules, Vec<(String, String)>), String> {
    let text = std::str::from_utf8(text).map_err(|e| {
        format!(
            "it isn't TOML: it isn't UTF-8 text at byte {}",
            e.valid_up_to()
        )
    })?;
    // The parser bounds how deep arrays and tables nest, so a hostile file can't overflow the
    // stack.
    let table: Table = text.parse().map_err(|e| not_toml(text, &e))?;
    let mut rules = Rules::default();
    let mut ignored = Vec::new();
    let mut listed: &[Value] = &[];
    for (key, value) in &table {
        let why = match key.as_str() {
            "preset" => match value.as_str() {
                Some(label) if label.len() <= MAX_PRESET_BYTES => {
                    rules.preset = Some(label.to_owned());
                    continue;
                }
                Some(_) => "it is longer than 64 bytes".to_owned(),
                None => "it must be text".to_owned(),
            },
            "chunk-files-over" => match size(value) {
                Ok(bytes) if (CHUNK_FROM..=CHUNK_UP_TO).contains(&bytes) => {
                    rules.chunk_files_over = bytes;
                    continue;
                }
                Ok(bytes) => format!("{} is outside 1 MiB to 128 GiB", size_text(bytes)),
                Err(why) => why,
            },
            "ask-before-saving-over" => match size(value) {
                Ok(bytes) => {
                    rules.ask_over_bytes = bytes;
                    continue;
                }
                Err(why) => why,
            },
            "ask-before-saving-files-over" => {
                match value.as_integer().and_then(|n| u64::try_from(n).ok()) {
                    Some(files) => {
                        rules.ask_over_files = files;
                        continue;
                    }
                    None => "it must be a whole number, 0 or more".to_owned(),
                }
            }
            "rule" => match value.as_array() {
                Some(list) => {
                    listed = list;
                    continue;
                }
                None => "it must be a list of rules".to_owned(),
            },
            _ => UNKNOWN.to_owned(),
        };
        ignored.push((key.clone(), why));
    }
    // Top-level names in byte order, whatever order the table keeps.
    ignored.sort();
    for (n, item) in (1u64..).zip(listed) {
        match rule(item) {
            Ok((kept, bad)) => {
                rules.rules.push(kept);
                ignored.extend(
                    bad.into_iter()
                        .map(|(k, why)| (format!("rule[{n}].{k}"), why)),
                );
            }
            Err(why) => ignored.push((format!("rule[{n}]"), why)),
        }
    }
    Ok((rules, ignored))
}

/// One `[[rule]]`, and its keys passed over in byte order; `Err` passes it over whole.
fn rule(item: &Value) -> Result<(Rule, Vec<(String, String)>), String> {
    let table = item.as_table().ok_or("it must be a table")?;
    let files = table
        .get("files")
        .and_then(Value::as_array)
        .and_then(|list| {
            let patterns = list.iter().map(|p| p.as_str().filter(|p| !p.is_empty()));
            patterns
                .map(|p| p.map(str::to_owned))
                .collect::<Option<Vec<_>>>()
        })
        .filter(|files| !files.is_empty())
        .ok_or(NO_FILES)?;
    let mut bad = Vec::new();
    let mut kept = Vec::new();
    for (n, pattern) in (1u64..).zip(files) {
        if pattern.contains(['\n', '\r']) {
            bad.push((
                format!("files[{n}]"),
                "a pattern can't hold a line break, so it matches no file".to_owned(),
            ));
        } else {
            kept.push(pattern);
        }
    }
    // A rule that can match nothing would leave the files it was meant for unprotected in silence.
    if kept.is_empty() {
        return Err(NO_FILES.to_owned());
    }
    if kept.iter().all(|p| p.starts_with('!')) {
        return Err("every pattern starts with !, so the rule matches no file".to_owned());
    }
    let mut rule = Rule {
        files: kept,
        lockable: None,
        conflict_markers: None,
    };
    for (key, value) in table {
        let setting = match key.as_str() {
            "files" => continue,
            "lockable" => &mut rule.lockable,
            "conflict-markers" => &mut rule.conflict_markers,
            _ => {
                bad.push((key.clone(), UNKNOWN.to_owned()));
                continue;
            }
        };
        match value.as_bool() {
            Some(on) => *setting = Some(on),
            None => bad.push((key.clone(), "it must be true or false".to_owned())),
        }
    }
    bad.sort();
    Ok((rule, bad))
}

/// A size's bytes, or why `value` isn't one.
fn size(value: &Value) -> Result<u64, String> {
    let Some(text) = value.as_str() else {
        return Err("it must be a size in quotes, such as \"1 MiB\"".to_owned());
    };
    parse_size(text).ok_or_else(|| {
        format!(
            "\"{}\" isn't a size: digits, then B, KB, MB, GB, TB, KiB, MiB, GiB or TiB",
            shown(text)
        )
    })
}

/// Why `text` isn't TOML, as the parser says, at its line. The parser may quote the file, so
/// what it says is escaped and cut.
fn not_toml(text: &str, e: &toml::de::Error) -> String {
    let said = shown(e.message().trim());
    let line = e
        .span()
        .and_then(|span| text.as_bytes().get(..span.start))
        .map(|before| {
            before
                .iter()
                .filter(|b| **b == b'\n')
                .count()
                .saturating_add(1)
        });
    match line {
        Some(line) => format!("it isn't TOML: line {line}: {said}"),
        None => format!("it isn't TOML: {said}"),
    }
}

/// Text from the file as a warning shows it: escaped, and cut so that it can't flood the terminal.
fn shown(text: &str) -> String {
    let mut cut: String = text.chars().take(SHOWN_CHARS).collect();
    if cut.len() < text.len() {
        cut.push_str("...");
    }
    escape(&cut, false)
}

/// The rules as read from the folder.
#[derive(Clone, Debug, Default)]
pub struct RulesRead {
    pub rules: Rules,
    /// Whether an `opalro.toml` was read and used.
    pub file: bool,
    /// Why the whole file was passed over.
    pub skipped: Option<String>,
    /// Each name passed over, with why.
    pub ignored: Vec<(String, String)>,
}

const A_LINK: &str = "it is a link, which Opalro doesn't follow here";
const NOT_A_FILE: &str = "it is a folder or special file, not a regular file";
const TOO_LARGE: &str = "it is larger than 1 MiB";
const SEVERAL: &str = "several files have its name but for letter case";

fn skipped(why: String) -> RulesRead {
    RulesRead {
        skipped: Some(why),
        ..RulesRead::default()
    }
}

/// What the bytes of an `opalro.toml` give.
fn parsed(bytes: &[u8]) -> RulesRead {
    match parse(bytes) {
        Ok((rules, ignored)) => RulesRead {
            rules,
            file: true,
            skipped: None,
            ignored,
        },
        Err(why) => skipped(why),
    }
}

/// Reads the `opalro.toml` in the top folder of a save, `root` its listing, on the terms `read`
/// has for the folder's: the size its listing gives is checked before anything is read.
pub(crate) fn read_saved(store: &Store, root: &Listing) -> Result<RulesRead, StoreError> {
    // Failing the exact name, the one name alike but for the case of ASCII letters, as the
    // folder's would be found where letter case is ignored; whichever system this is.
    let entry = match root.get("opalro.toml") {
        Some(entry) => entry,
        None => {
            let mut alike = root
                .entries()
                .iter()
                .filter(|e| e.name.eq_ignore_ascii_case("opalro.toml"));
            match (alike.next(), alike.next()) {
                (None, _) => return Ok(RulesRead::default()),
                (Some(entry), None) => entry,
                (Some(_), Some(_)) => return Ok(skipped(SEVERAL.to_owned())),
            }
        }
    };
    let why = match entry.kind {
        EntryKind::File | EntryKind::Executable if entry.size > MAX_RULES_FILE_BYTES => TOO_LARGE,
        EntryKind::File | EntryKind::Executable => {
            return Ok(match read_capped(store, &entry.id, MAX_RULES_FILE_BYTES)? {
                Some(bytes) => parsed(&bytes),
                None => skipped(TOO_LARGE.to_owned()),
            });
        }
        EntryKind::Symlink => A_LINK,
        EntryKind::Folder | EntryKind::ProjectLink | EntryKind::Conflict => NOT_A_FILE,
    };
    Ok(skipped(why.to_owned()))
}

/// Reads `opalro.toml` in `top`. Never fails: what can't be used is in `skipped` or `ignored`.
#[must_use]
pub fn read(top: &Dir) -> RulesRead {
    let name = OsStr::new("opalro.toml");
    let shown = Path::new("opalro.toml");
    let bytes = match safe_fs::read_capped_in(top, name, shown, MAX_RULES_FILE_BYTES) {
        Ok(bytes) => bytes,
        // A link to nothing is there all the same, and is passed over as a link.
        Err(e)
            if e.kind() == io::ErrorKind::NotFound
                && top
                    .metadata(name)
                    .is_err_and(|m| m.kind() == io::ErrorKind::NotFound) =>
        {
            return RulesRead::default();
        }
        Err(e) => return skipped(unusable(top, name, &e)),
    };
    parsed(&bytes)
}

/// Why the file `name` couldn't be read, judged by what it is.
fn unusable(top: &Dir, name: &OsStr, e: &io::Error) -> String {
    match top.metadata(name).map(|m| (m.kind, m.size)) {
        Ok((EntryType::Symlink, _)) => A_LINK.to_owned(),
        Ok((EntryType::Dir | EntryType::Other, _)) => NOT_A_FILE.to_owned(),
        Ok((EntryType::File, size)) if size > MAX_RULES_FILE_BYTES => TOO_LARGE.to_owned(),
        _ => format!("Opalro couldn't read it: {}", escape(&e.to_string(), false)),
    }
}

impl RulesRead {
    /// `rules_file_skipped` or `rules_ignored`, if anything was passed over.
    #[must_use]
    pub fn warnings(&self) -> Vec<Warning> {
        if let Some(why) = &self.skipped {
            let own = Rules::default();
            let message = format!(
                "Opalro passed over opalro.toml: {why}. Its own values apply instead: files of {} or \
                 more are stored in chunks, and a save asks first over {} or {} new files. Fix or \
                 remove the file to use its rules. Until it is fixed or removed, opalro combine \
                 merges no file line by line.",
                size_text(own.chunk_files_over),
                size_text(own.ask_over_bytes),
                thousands(own.ask_over_files)
            );
            return vec![Warning::new(warning::RULES_FILE_SKIPPED, message)];
        }
        if self.ignored.is_empty() {
            return vec![];
        }
        let mut message = "Opalro passed over these parts of opalro.toml, which it can't use, and \
                           used its own values for them; the rest of the file counts:"
            .to_owned();
        for (name, why) in self.ignored.iter().take(NAMES_LISTED) {
            message.push_str(&format!("\n  {}: {why}", shown(name)));
        }
        let more = self.ignored.len().saturating_sub(NAMES_LISTED);
        if more > 0 {
            message.push_str(&format!("\n  and {more} more"));
        }
        message.push_str(
            "\nFix or remove them, or update Opalro if a newer one wrote them. Until then, opalro \
             combine merges no file line by line.",
        );
        vec![Warning::new(warning::RULES_IGNORED, message)]
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::test_support::{HOSTILE, assert_hostile_text_escaped, symlink};
    use crate::warning;
    use std::fs;
    use std::path::Path;

    fn read_in(dir: &Path) -> RulesRead {
        read(&Dir::open_top(dir).unwrap())
    }

    fn kinds(read: &RulesRead) -> Vec<&'static str> {
        read.warnings().iter().map(|w| w.kind).collect()
    }

    #[test]
    fn sizes_read_in_both_families() {
        for (text, bytes) in [
            ("0 B", 0),
            ("7B", 7),
            ("3 KB", 3_000),
            ("3 KiB", 3_072),
            ("100 MB", 100_000_000),
            ("1 MiB", 1 << 20),
            ("5   GB", 5_000_000_000),
            ("1GiB", 1 << 30),
            ("2 TB", 2_000_000_000_000),
            ("2 TiB", 2 << 40),
            ("0010 KiB", 10 << 10),
        ] {
            assert_eq!(parse_size(text), Some(bytes), "{text}");
        }
        for text in [
            "",
            "MiB",
            "1",
            "1 ",
            "1 mib",
            "1 KIB",
            "1.5 MiB",
            "-1 B",
            "+1 B",
            " 1 B",
            "1 B ",
            "1\tB",
            "1 B B",
            "1 PiB",
            "\u{661} B",
        ] {
            assert_eq!(parse_size(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_size_that_does_not_fit_64_bits_is_no_size() {
        assert_eq!(parse_size("18446744073709551615 B"), Some(u64::MAX));
        assert_eq!(parse_size("18446744073709551616 B"), None);
        assert_eq!(parse_size("16777215 TiB"), Some(16_777_215 << 40));
        assert_eq!(parse_size("16777216 TiB"), None);
        assert_eq!(parse_size(&format!("{} B", "9".repeat(10_000))), None);
    }

    #[test]
    fn a_settings_size_is_written_in_the_largest_exact_unit_and_reads_back() {
        for (bytes, text) in [
            (1 << 20, "1 MiB"),
            (100_000_000, "100 MB"),
            (1 << 30, "1 GiB"),
            (5_000_000_000, "5 GB"),
            (1500, "1500 B"),
            (0, "0 B"),
            (1_024_000, "1000 KiB"),
            (137_438_953_472, "128 GiB"),
            (u64::MAX, "18446744073709551615 B"),
        ] {
            assert_eq!(size_text(bytes), text, "{bytes}");
            assert_eq!(parse_size(&size_text(bytes)), Some(bytes), "{bytes}");
        }
    }

    #[test]
    fn a_file_of_nested_brackets_is_passed_over_without_overflowing_the_stack() {
        for text in [
            "[".repeat(100_000),
            format!("a = {}", "[".repeat(100_000)),
            format!("a = {}1{}", "[".repeat(100_000), "]".repeat(100_000)),
            format!("a = {}", "{ b = ".repeat(100_000)),
            format!("[{}]", vec!["a"; 100_000].join(".")),
        ] {
            assert!(parse(text.as_bytes()).is_err());
        }
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("opalro.toml"), "[".repeat(100_000)).unwrap();
        let read = read_in(dir.path());
        assert!(read.skipped.is_some());
        assert_eq!(read.rules, Rules::default());
        assert_eq!(kinds(&read), [warning::RULES_FILE_SKIPPED]);
        let message = &read.warnings()[0].message;
        assert!(
            message.contains("over 100 MB or 10,000 new files"),
            "{message}"
        );
    }

    #[test]
    fn opals_own_values_are_the_ones_before_rules_existed() {
        let own = Rules {
            preset: None,
            chunk_files_over: 1_048_576,
            ask_over_bytes: 100_000_000,
            ask_over_files: 10_000,
            rules: vec![],
        };
        assert_eq!(Rules::default(), own);
        let dir = tempfile::tempdir().unwrap();
        let read = read_in(dir.path());
        assert_eq!(read.rules, own);
        assert!(!read.file);
        assert_eq!(read.skipped, None);
        assert!(read.ignored.is_empty());
    }

    #[test]
    fn no_file_means_opals_own_values_without_a_word() {
        let dir = tempfile::tempdir().unwrap();
        let read = read_in(dir.path());
        assert!(!read.file);
        assert_eq!(read.rules, Rules::default());
        assert_eq!(read.rules.chunk_files_over, 1 << 20);
        assert_eq!(read.rules.ask_over_bytes, 100_000_000);
        assert_eq!(read.rules.ask_over_files, 10_000);
        assert!(read.warnings().is_empty());
    }

    #[test]
    fn a_file_of_1_mib_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let mut text = b"chunk-files-over = \"2 MiB\"\n#".to_vec();
        text.resize(1 << 20, b'x');
        fs::write(dir.path().join("opalro.toml"), &text).unwrap();
        let read = read_in(dir.path());
        assert!(read.file);
        assert_eq!(read.rules.chunk_files_over, 2 << 20);
        assert!(read.warnings().is_empty());
    }

    #[test]
    fn a_link_a_folder_and_a_file_over_1_mib_are_each_passed_over() {
        let over = tempfile::tempdir().unwrap();
        let mut text = b"chunk-files-over = \"2 MiB\"\n#".to_vec();
        text.resize((1 << 20) + 1, b'x');
        fs::write(over.path().join("opalro.toml"), &text).unwrap();
        let folder = tempfile::tempdir().unwrap();
        fs::create_dir(folder.path().join("opalro.toml")).unwrap();
        let mut cases = vec![(over, "larger than 1 MiB"), (folder, "not a regular file")];
        let link = tempfile::tempdir().unwrap();
        fs::write(
            link.path().join("real.toml"),
            "chunk-files-over = \"2 MiB\"\n",
        )
        .unwrap();
        if symlink(
            &link.path().join("real.toml"),
            &link.path().join("opalro.toml"),
        ) {
            cases.push((link, "a link"));
        }
        for (dir, why) in &cases {
            let read = read_in(dir.path());
            assert!(!read.file, "{why}");
            assert_eq!(read.rules, Rules::default(), "{why}");
            let warnings = read.warnings();
            assert_eq!(warnings.len(), 1, "{why}");
            assert_eq!(warnings[0].kind, warning::RULES_FILE_SKIPPED);
            assert!(warnings[0].message.contains(why), "{}", warnings[0].message);
        }
    }

    #[test]
    fn the_file_is_never_read_through_a_link() {
        let outside = tempfile::tempdir().unwrap();
        let secret = "preset = \"the-secret-text\"\nzebra-secret = 1\n";
        fs::write(outside.path().join("secret.toml"), secret).unwrap();
        let dir = tempfile::tempdir().unwrap();
        if !symlink(
            &outside.path().join("secret.toml"),
            &dir.path().join("opalro.toml"),
        ) {
            return;
        }
        let read = read_in(dir.path());
        assert_eq!(read.rules, Rules::default());
        assert!(read.ignored.is_empty());
        let warnings = read.warnings();
        assert_eq!(warnings.len(), 1);
        assert!(
            !warnings[0].message.contains("secret"),
            "{}",
            warnings[0].message
        );
        // A link to nothing is no missing file: it is passed over, and said so.
        let dangling = tempfile::tempdir().unwrap();
        let nowhere = dangling.path().join("nowhere.toml");
        assert!(symlink(&nowhere, &dangling.path().join("opalro.toml")));
        let read = read_in(dangling.path());
        assert_eq!(kinds(&read), [warning::RULES_FILE_SKIPPED]);
        assert!(read.warnings()[0].message.contains("a link"));
    }

    #[cfg(unix)]
    #[test]
    fn a_pipe_is_passed_over_without_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let made = std::process::Command::new("mkfifo")
            .arg(dir.path().join("opalro.toml"))
            .status()
            .unwrap();
        assert!(made.success());
        let read = read_in(dir.path());
        assert_eq!(kinds(&read), [warning::RULES_FILE_SKIPPED]);
    }

    #[test]
    fn warnings_escape_what_the_file_says() {
        let hostile = HOSTILE
            .replace('\u{1b}', "\\u001b")
            .replace('\u{202e}', "\\u202e")
            .replace('\u{2028}', "\\u2028");
        let text = format!("\"{hostile}\" = 1\nchunk-files-over = \"{hostile}\"\n");
        let (rules, ignored) = parse(text.as_bytes()).unwrap();
        assert_eq!(rules, Rules::default());
        assert_eq!(ignored.len(), 2);
        let read = RulesRead {
            rules,
            file: true,
            skipped: None,
            ignored,
        };
        let warnings = read.warnings();
        assert_eq!(kinds(&read), [warning::RULES_IGNORED]);
        let message = &warnings[0].message;
        assert_hostile_text_escaped(message);
        assert_eq!(message.matches("a\\x1b[2Jb").count(), 2, "{message}");
        // What the parser says about a file that isn't TOML can hold the file's text too.
        let bad = format!("\"{hostile}\" = = 1\n");
        let why = parse(bad.as_bytes()).unwrap_err();
        let read = RulesRead {
            skipped: Some(why),
            ..RulesRead::default()
        };
        assert!(!read.warnings()[0].message.contains('\u{1b}'));
    }

    #[test]
    fn a_long_value_or_name_is_cut_and_many_names_are_not_all_listed() {
        let long = "x".repeat(10_000);
        let mut text = format!("chunk-files-over = \"{long}\"\n{long} = 1\n");
        for n in 0..1_000 {
            text.push_str(&format!("key{n} = 1\n"));
        }
        let (rules, ignored) = parse(text.as_bytes()).unwrap();
        assert_eq!(ignored.len(), 1_002);
        let read = RulesRead {
            rules,
            file: true,
            skipped: None,
            ignored,
        };
        let message = &read.warnings()[0].message;
        assert!(message.len() < 4_000, "{}", message.len());
        assert!(!message.contains(&"x".repeat(65)), "{message}");
        assert!(message.contains("and 982 more"), "{message}");
    }

    fn matcher(rules: &[(&[&str], Option<bool>)]) -> RuleMatcher {
        let rules = rules.iter().map(|(files, markers)| Rule {
            files: files.iter().map(|f| (*f).to_owned()).collect(),
            lockable: None,
            conflict_markers: *markers,
        });
        RuleMatcher::new(
            &RulesRead {
                rules: Rules {
                    rules: rules.collect(),
                    ..Rules::default()
                },
                ..RulesRead::default()
            },
            &RulesRead::default(),
        )
    }

    /// Whether a rule of `patterns` matches `path`.
    fn matches(patterns: &[&str], path: &str) -> bool {
        !matcher(&[(patterns, Some(false))])
            .ours
            .conflict_markers(path)
    }

    fn read_text(text: &str) -> RulesRead {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("opalro.toml"), text).unwrap();
        read_in(dir.path())
    }

    #[test]
    fn no_file_is_merged_line_by_line_while_the_rules_are_not_read_whole() {
        let rule = "[[rule]]\nfiles = [\"*.unity\"]\nconflict-markers = false\n";
        let whole = RuleMatcher::new(&read_text(rule), &RulesRead::default());
        assert_eq!(whole.line_merge("a.txt", None), LineMerge::Yes);
        assert_eq!(
            whole.line_merge("Main.unity", None),
            LineMerge::RuleSaysNo { theirs: false }
        );
        let none = RuleMatcher::new(&RulesRead::default(), &RulesRead::default());
        assert_eq!(none.line_merge("Main.unity", None), LineMerge::Yes);
        // Passed over whole: no rule is known, and no file is merged.
        let skipped = RuleMatcher::new(
            &read_text("[[rule]\nfiles = [\"*.unity\"]\n"),
            &RulesRead::default(),
        );
        for path in ["a.txt", "Main.unity"] {
            assert_eq!(
                skipped.line_merge(path, None),
                LineMerge::RulesNotWhole { theirs: false },
                "{path}"
            );
        }
        // One part passed over: the rules read still say no where they do.
        for part in ["colour = \"red\"\n", ""] {
            let inside = if part.is_empty() {
                "lockabel = true\n"
            } else {
                ""
            };
            let text = format!(
                "{part}[[rule]]\nfiles = [\"*.unity\"]\n{inside}conflict-markers = false\n"
            );
            let read = read_text(&text);
            assert_eq!(read.ignored.len(), 1, "{text}");
            let m = RuleMatcher::new(&read, &RulesRead::default());
            assert_eq!(
                m.line_merge("a.txt", None),
                LineMerge::RulesNotWhole { theirs: false },
                "{text}"
            );
            assert_eq!(
                m.line_merge("Main.unity", None),
                LineMerge::RuleSaysNo { theirs: false },
                "{text}"
            );
        }
    }

    #[test]
    fn a_moved_file_is_merged_only_when_both_its_names_allow_it() {
        let m = matcher(&[(&["*.unity"], Some(false))]);
        assert_eq!(m.line_merge("Main.txt", Some("a.txt")), LineMerge::Yes);
        assert_eq!(
            m.line_merge("Main.txt", Some("Main.unity")),
            LineMerge::RuleSaysNo { theirs: false }
        );
        assert_eq!(
            m.line_merge("Main.unity", Some("Main.txt")),
            LineMerge::RuleSaysNo { theirs: false }
        );
    }

    #[test]
    fn a_file_is_merged_only_when_both_rules_files_allow_it() {
        let none = RulesRead::default();
        let ruled = read_text("[[rule]]\nfiles = [\"*.unity\"]\nconflict-markers = false\n");
        let broken = read_text("[[rule]\n");
        let ask = |ours: &RulesRead, theirs: &RulesRead, path: &str, was: Option<&str>| {
            RuleMatcher::new(ours, theirs).line_merge(path, was)
        };
        let no = |theirs| LineMerge::RuleSaysNo { theirs };
        let not_whole = |theirs| LineMerge::RulesNotWhole { theirs };
        assert_eq!(ask(&none, &none, "Main.unity", None), LineMerge::Yes);
        assert_eq!(ask(&none, &ruled, "a.txt", None), LineMerge::Yes);
        // The folder's file is named when it is a reason, the other side's when only it is.
        assert_eq!(ask(&ruled, &none, "Main.unity", None), no(false));
        assert_eq!(ask(&none, &ruled, "Main.unity", None), no(true));
        assert_eq!(ask(&ruled, &ruled, "Main.unity", None), no(false));
        assert_eq!(ask(&broken, &none, "a.txt", None), not_whole(false));
        assert_eq!(ask(&none, &broken, "a.txt", None), not_whole(true));
        assert_eq!(ask(&broken, &broken, "a.txt", None), not_whole(false));
        // A rule that says no is the file's own reason, whichever file holds it.
        assert_eq!(ask(&broken, &ruled, "Main.unity", None), no(true));
        assert_eq!(ask(&ruled, &broken, "Main.unity", None), no(false));
        assert_eq!(ask(&ruled, &broken, "a.txt", None), not_whole(true));
        // A moved file whose old name only the other side's rules protect.
        assert_eq!(ask(&none, &ruled, "Main.txt", Some("Main.unity")), no(true));
        assert_eq!(
            ask(&none, &ruled, "Main.txt", Some("a.txt")),
            LineMerge::Yes
        );
    }

    const LINE_BREAK: &str = "a pattern can't hold a line break, so it matches no file";
    const ALL_BANG: &str = "every pattern starts with !, so the rule matches no file";
    const NO_FILES: &str =
        "it has no usable `files`: a list of one or more patterns, none of them empty";

    /// What `[[rule]]` with `files` as given, after a setting and before a usable rule, leaves
    /// passed over; the rest of the file must count.
    fn passed_over(files: &str) -> Vec<(String, String)> {
        let text = format!(
            "chunk-files-over = \"4 MiB\"\n[[rule]]\nfiles = {files}\nconflict-markers = false\n\
             [[rule]]\nfiles = [\"*.psd\"]\nlockable = true\n"
        );
        let (rules, ignored) = parse(text.as_bytes()).unwrap();
        assert_eq!(rules.chunk_files_over, 4 << 20, "{files}");
        let psd = rules.rules.iter().find(|r| r.files == ["*.psd"]);
        assert_eq!(psd.unwrap().lockable, Some(true), "{files}");
        ignored
    }

    fn named(name: &str, why: &str) -> Vec<(String, String)> {
        vec![(name.to_owned(), why.to_owned())]
    }

    #[test]
    fn a_pattern_holding_a_line_break_is_passed_over_and_the_rules_other_patterns_count() {
        for files in [r#"["a\nb", "*.unity"]"#, r#"["a\rb", "*.unity"]"#] {
            assert_eq!(
                passed_over(files),
                named("rule[1].files[1]", LINE_BREAK),
                "{files}"
            );
        }
        let text = "[[rule]]\nfiles = [\"*.unity\", \"x\\r\\ny\"]\nconflict-markers = false\n";
        let (rules, ignored) = parse(text.as_bytes()).unwrap();
        assert_eq!(ignored, named("rule[1].files[2]", LINE_BREAK));
        assert_eq!(rules.rules[0].files, ["*.unity"]);
        assert_eq!(rules.rules[0].conflict_markers, Some(false));
    }

    #[test]
    fn a_rule_that_can_match_no_file_is_passed_over_whole() {
        assert_eq!(
            passed_over(r#"["!*.unity", "!keep/"]"#),
            named("rule[1]", ALL_BANG)
        );
        assert_eq!(passed_over(r#"["a\nb", "!c"]"#), named("rule[1]", ALL_BANG));
        assert_eq!(
            passed_over(r#"["a\nb", "c\rd"]"#),
            named("rule[1]", NO_FILES)
        );
        // One pattern that can match is enough.
        assert_eq!(passed_over(r#"["!keep.unity", "*.unity"]"#), vec![]);
    }

    #[test]
    fn a_rule_that_can_match_no_file_is_warned_of_and_stops_line_merging() {
        let read = read_text("[[rule]]\nfiles = [\"!*.unity\"]\nconflict-markers = false\n");
        let message = &read.warnings()[0].message;
        assert!(
            message.contains(&format!("\n  rule[1]: {ALL_BANG}\n")),
            "{message}"
        );
        let m = RuleMatcher::new(&read, &RulesRead::default());
        assert_eq!(
            m.line_merge("a.txt", None),
            LineMerge::RulesNotWhole { theirs: false }
        );
    }

    #[test]
    fn the_warnings_say_a_combine_merges_no_file_line_by_line_until_the_file_is_fixed() {
        let skipped = read_text("[[rule]\n");
        let warnings = skipped.warnings();
        assert_eq!(
            warnings[0].message,
            "Opalro passed over opalro.toml: it isn't TOML: line 1: unclosed array table, expected `]`. Its \
             own values apply instead: files of 1 MiB or more are stored in chunks, and a save \
             asks first over 100 MB or 10,000 new files. Fix or remove the file to use its rules. \
             Until it is fixed or removed, opalro combine merges no file line by line."
        );
        let ignored = read_text("colour = \"red\"\n");
        assert_eq!(
            ignored.warnings()[0].message,
            "Opalro passed over these parts of opalro.toml, which it can't use, and used its own \
             values for them; the rest of the file counts:\n  colour: Opalro doesn't know this \
             key; a newer Opalro may have written it\nFix or remove them, or update Opalro if a \
             newer one wrote them. Until then, opalro combine merges no file line by line."
        );
    }

    #[test]
    fn no_rule_means_markers() {
        assert!(matcher(&[]).ours.conflict_markers("a.txt"));
    }

    #[test]
    fn the_last_matching_rule_that_gives_the_key_decides() {
        let m = matcher(&[
            (&["*.unity"], Some(false)),
            (&["Assets/Keep/*.unity"], Some(true)),
            (&["*.unity"], None),
        ]);
        assert!(!m.ours.conflict_markers("Assets/a.unity"));
        assert!(m.ours.conflict_markers("Assets/Keep/b.unity"));
        assert!(m.ours.conflict_markers("a.txt"));
    }

    #[test]
    fn patterns_match_as_opalroignore_lines_do() {
        assert!(matches(&["*.meta"], "a/b/c.meta"));
        assert!(matches(&["/top.txt"], "top.txt"));
        assert!(!matches(&["/top.txt"], "a/top.txt"));
        assert!(matches(&["art/**/*.png"], "art/x/y.png"));
        assert!(matches(&["*.unity", "!keep.unity"], "a.unity"));
        assert!(!matches(&["*.unity", "!keep.unity"], "keep.unity"));
    }

    #[test]
    fn a_folder_pattern_matches_the_files_below_it() {
        assert!(matches(&["Assets/Scenes/"], "Assets/Scenes/x/y.unity"));
        assert!(!matches(&["Assets/Scenes/"], "Assets/Scenes.unity"));
    }

    #[test]
    fn ascii_letter_case_is_ignored_everywhere() {
        // The same on every system, whatever the drive the tests run on.
        assert!(matches(&["*.UNITY"], "Main.unity"));
        assert!(matches(&["*.unity"], "MAIN.UNITY"));
        assert!(matches(&["Assets/"], "assets/a.txt"));
        assert!(!matches(&["\u{c9}.txt"], "\u{e9}.txt"));
    }

    #[test]
    fn letters_in_escapes_and_brackets_take_either_case() {
        for path in ["Main.unity", "main.unity", "MAIN.UNITY"] {
            for pattern in [
                "\\Main.unity",
                "\\main.unity",
                "[M]ain.unity",
                "[m]ain.unity",
            ] {
                assert!(matches(&[pattern], path), "{pattern} {path}");
            }
            for pattern in ["[A-Z]*.unity", "[a-z]*.unity", "[[:upper:]]*", "[!x]ain.*"] {
                assert!(matches(&[pattern], path), "{pattern} {path}");
            }
        }
        assert!(!matches(&["[!M]ain.unity"], "main.unity"));
        // Range ends and class names are kept as written: `[A-z]` still takes `_`.
        assert!(matches(&["[A-z]x"], "_x"));
        assert!(matches(&["[Z-a]x"], "_x"));
        assert!(!matches(&["[[:UPPER:]]x"], "ax"));
    }

    #[test]
    fn each_entry_is_a_pattern_never_a_comment() {
        assert!(matches(&["#notes"], "#notes"));
        assert!(matches(&["#notes"], "a/#NOTES"));
        assert!(!matches(&["#notes"], "notes"));
        assert!(!matches(&["#keep*", "!#keep"], "#keep"));
        assert!(matches(&["#keep*", "!#keep"], "#keep2"));
        // A byte-order mark is part of the pattern wherever it stands.
        for patterns in [&["\u{feff}*.unity", "x"][..], &["x", "\u{feff}*.unity"]] {
            assert!(!matches(patterns, "a.unity"), "{patterns:?}");
            assert!(matches(patterns, "\u{feff}a.unity"), "{patterns:?}");
        }
    }

    #[test]
    fn a_rule_of_only_reincluding_patterns_matches_nothing() {
        let m = matcher(&[(&["!*.unity", "!keep/"], Some(false))]);
        for path in ["a.unity", "keep/a.txt", "a.txt"] {
            assert!(m.ours.conflict_markers(path), "{path}");
        }
    }

    #[test]
    fn a_pattern_holding_a_line_break_matches_nothing() {
        for pattern in ["a\nb", "a\r\nb", "a\rb"] {
            for path in ["a", "b", "a\nb"] {
                assert!(!matches(&[pattern], path), "{pattern:?} {path:?}");
            }
        }
        // The rule's other patterns still count.
        assert!(matches(&["a\nb", "c"], "c"));
    }

    #[test]
    fn the_game_presets_scenes_prefabs_and_meta_files_get_no_markers() {
        use crate::presets::{Engine, Found, Preset, opal_toml};
        let found = Found {
            engines: vec![Engine::Unity],
            languages: vec![],
        };
        let text = opal_toml(Preset::Game, &found).unwrap();
        let (rules, ignored) = parse(text.as_bytes()).unwrap();
        assert!(ignored.is_empty(), "{ignored:?}");
        let m = RuleMatcher::new(
            &RulesRead {
                rules,
                ..RulesRead::default()
            },
            &RulesRead::default(),
        );
        assert!(!m.ours.conflict_markers("Assets/Main.unity"));
        assert!(!m.ours.conflict_markers("Assets/P.prefab"));
        assert!(!m.ours.conflict_markers("Assets/P.prefab.meta"));
        assert!(m.ours.conflict_markers("Assets/Player.cs"));
    }

    proptest::proptest! {
        #[test]
        fn what_is_kept_keeps_to_the_limits(
            text in "((chunk-files-over|ask-before-saving-over|preset|rule|files|lockable|x) \
                     ?= ?(\"[0-9 ]{0,12}[KMGT]?i?B?\"|-?[0-9]{1,20}|true|\\[\\]|\\{\\}|\\[\\{ ?files = \
                     \\[\"[a-z*.]{0,3}\"\\] ?\\}\\])\n|\\[\\[rule\\]\\]\n){0,8}"
        ) {
            if let Ok((rules, _)) = parse(text.as_bytes()) {
                let usable = |r: &Rule| !r.files.is_empty() && r.files.iter().all(|p| !p.is_empty());
                proptest::prop_assert!((CHUNK_FROM..=CHUNK_UP_TO).contains(&rules.chunk_files_over));
                proptest::prop_assert!(rules.rules.iter().all(usable));
            }
        }
    }

    /// The rules as saves use them.
    mod saves {
        use super::*;
        use crate::id::{Id, hash_content};
        use crate::pack::{ObjectKind, PackReader};
        use crate::repo::Repository;
        use crate::save::Person;
        use crate::snapshot::{LargeSave, SaveError, SaveOutcome, SaveRequest, save_folder};
        use crate::test_support::noise;
        use std::collections::HashSet;

        fn project() -> (tempfile::TempDir, Repository) {
            let dir = tempfile::tempdir().unwrap();
            let who = Person {
                name: "Ada".into(),
                email: "ada@example.com".into(),
                time_ms: 1_700_000_000_000,
                tz_offset_min: 0,
            };
            let repo = Repository::init(dir.path(), &who).unwrap();
            (dir, repo)
        }

        /// A save whose question about adding much is answered `answer`, and what it asked.
        fn save(repo: &mut Repository, answer: bool) -> (Result<SaveOutcome, SaveError>, bool) {
            let who = Person {
                name: "Ada".into(),
                email: "ada@example.com".into(),
                time_ms: 1_700_000_000_000,
                tz_offset_min: 0,
            };
            let req = SaveRequest {
                message: Some("Save"),
                who: &who,
                command: "save",
                accept_crlf: false,
                git_excludes: None,
                new_line: None,
                only: None,
                amend: false,
            };
            let mut asked = false;
            let result = save_folder(repo, &req, &mut |_| {}, &mut |_: &LargeSave| {
                asked = true;
                answer
            });
            (result, asked)
        }

        /// The files the store holds as chunk lists.
        fn chunk_lists(repo: &Repository) -> HashSet<Id> {
            let packs = fs::read_dir(repo.packs_dir()).unwrap();
            let paths = packs.map(|p| p.unwrap().path());
            let paths = paths.filter(|p| p.extension().is_some_and(|e| e == "pack"));
            let mut found = HashSet::new();
            for path in paths {
                for entry in PackReader::open(&path).unwrap().entries() {
                    let entry = entry.unwrap();
                    if entry.kind == ObjectKind::ChunkList {
                        found.insert(entry.id);
                    }
                }
            }
            found
        }

        /// The kinds of a save's warnings about `opalro.toml`.
        fn rules_warnings(outcome: &SaveOutcome) -> Vec<&'static str> {
            let (SaveOutcome::Saved { warnings, .. }
            | SaveOutcome::NothingChanged { warnings, .. }) = outcome;
            let about = [warning::RULES_FILE_SKIPPED, warning::RULES_IGNORED];
            let kinds = warnings.iter().map(|w| w.kind);
            kinds.filter(|k| about.contains(k)).collect()
        }

        #[test]
        fn chunk_files_over_decides_which_files_are_cut() {
            let (two, four, five) = (noise(1, 2 << 20), noise(2, 4 << 20), noise(3, 5 << 20));
            for (rules, two_cut) in [
                (Some("chunk-files-over = \"4 MiB\"\n"), false),
                (None, true),
            ] {
                let (dir, mut repo) = project();
                if let Some(rules) = rules {
                    fs::write(dir.path().join("opalro.toml"), rules).unwrap();
                }
                for (name, bytes) in [("two", &two), ("four", &four), ("five", &five)] {
                    fs::write(dir.path().join(name), bytes).unwrap();
                }
                let (saved, _) = save(&mut repo, true);
                let SaveOutcome::Saved { save, warnings, .. } = saved.unwrap() else {
                    panic!("nothing was saved");
                };
                assert!(warnings.is_empty(), "{warnings:?}");
                let lists = chunk_lists(&repo);
                assert_eq!(lists.contains(&hash_content(&two)), two_cut, "{rules:?}");
                // From that size on, both ends counted.
                assert!(lists.contains(&hash_content(&four)), "{rules:?}");
                assert!(lists.contains(&hash_content(&five)), "{rules:?}");
                // How a file is stored never changes its ID.
                let root = repo.store().listing(&save.root).unwrap();
                for (name, bytes) in [("two", &two), ("four", &four), ("five", &five)] {
                    assert_eq!(root.get(name).unwrap().id, hash_content(bytes), "{name}");
                }
            }
        }

        #[test]
        fn only_files_under_1_mib_are_read_whole_whatever_the_threshold() {
            use crate::ingest::{IngestError, store_bytes};
            use crate::pack::PackError;
            let (dir, repo) = project();
            let store = repo.store();
            store.set_chunk_from(8 << 20);
            assert!(store.is_small((1 << 20) - 1));
            for size in [1 << 20, 3 << 20, (8 << 20) - 1, (128 << 30) + 1] {
                assert!(!store.is_small(size), "{size}");
            }
            assert!(!store.is_cut((8 << 20) - 1) && store.is_cut(8 << 20));
            // Bytes that aren't small are never read whole into memory...
            let mut packs = repo.new_packs().unwrap();
            let data = noise(6, 3 << 20);
            let refused = store_bytes(&mut packs, store, &mut &*data, data.len() as u64);
            let too_large = matches!(refused, Err(IngestError::Pack(PackError::TooLarge)));
            assert!(too_large, "{refused:?}");
            // ...so a save streams such a file, and stores it whole.
            fs::write(dir.path().join("mid"), &data).unwrap();
            let top = Dir::open_top(dir.path()).unwrap();
            let seen = top.metadata(OsStr::new("mid")).unwrap();
            let stored = crate::snapshot::store_file(
                &mut packs,
                store,
                &top,
                "mid",
                "mid",
                Some(&seen),
                &mut || {},
            );
            let id = hash_content(&data);
            assert_eq!(stored.unwrap(), (id, data.len() as u64));
            assert_eq!(packs.kind_of(&id), Some(ObjectKind::Content));
        }

        #[test]
        fn the_question_uses_the_files_numbers() {
            let (dir, mut repo) = project();
            let rules = "ask-before-saving-over = \"1 KiB\"\n";
            fs::write(dir.path().join("opalro.toml"), rules).unwrap();
            fs::write(dir.path().join("a"), [7; 2048]).unwrap();
            let (result, asked) = save(&mut repo, false);
            assert!(asked);
            assert!(matches!(result, Err(SaveError::LargeSave(_))), "{result:?}");

            let (dir, mut repo) = project();
            let rules = "ask-before-saving-files-over = 0\n";
            fs::write(dir.path().join("opalro.toml"), rules).unwrap();
            let (result, asked) = save(&mut repo, false);
            assert!(asked, "the rules file is a new file");
            assert!(matches!(result, Err(SaveError::LargeSave(_))), "{result:?}");

            // Over Opalro's own 10,000 files, which these rules raise.
            let (dir, mut repo) = project();
            let rules =
                "ask-before-saving-over = \"1 TiB\"\nask-before-saving-files-over = 1000000\n";
            fs::write(dir.path().join("opalro.toml"), rules).unwrap();
            for i in 0..10_001 {
                let folder = dir.path().join(format!("many/{}", i % 10));
                fs::create_dir_all(&folder).unwrap();
                fs::write(folder.join(format!("{i}.txt")), b"x").unwrap();
            }
            let (result, asked) = save(&mut repo, false);
            assert!(!asked);
            assert!(
                matches!(result, Ok(SaveOutcome::Saved { .. })),
                "{result:?}"
            );
        }

        #[test]
        fn a_rules_file_opal_cannot_use_never_stops_a_save() {
            let mut over = b"preset = \"big\"\n#".to_vec();
            over.resize((1 << 20) + 1, b'x');
            let cases: [(&str, Option<&[u8]>, &str); 5] = [
                (
                    "not TOML",
                    Some(b"this is not = = toml\n"),
                    warning::RULES_FILE_SKIPPED,
                ),
                ("over 1 MiB", Some(&over), warning::RULES_FILE_SKIPPED),
                ("a folder", None, warning::RULES_FILE_SKIPPED),
                (
                    "not UTF-8",
                    Some(b"preset = \"\xff\"\n"),
                    warning::RULES_FILE_SKIPPED,
                ),
                (
                    "bad keys",
                    Some(b"chunk-files-over = \"1 KiB\"\nask-before-saving-over = 0\nzebra = 1\n"),
                    warning::RULES_IGNORED,
                ),
            ];
            for (case, text, kind) in cases {
                let (dir, mut repo) = project();
                match text {
                    Some(text) => fs::write(dir.path().join("opalro.toml"), text).unwrap(),
                    None => fs::create_dir(dir.path().join("opalro.toml")).unwrap(),
                }
                fs::write(dir.path().join("a.txt"), "a").unwrap();
                let (result, asked) = save(&mut repo, false);
                assert!(!asked, "{case}");
                let outcome = result.unwrap_or_else(|e| panic!("{case}: {e}"));
                assert!(matches!(outcome, SaveOutcome::Saved { .. }), "{case}");
                assert_eq!(rules_warnings(&outcome), [kind], "{case}");
                assert_eq!(repo.rules().rules.chunk_files_over, 1 << 20, "{case}");
            }
            let (dir, mut repo) = project();
            fs::write(dir.path().join("real.toml"), "zebra = 1\n").unwrap();
            if symlink(
                &dir.path().join("real.toml"),
                &dir.path().join("opalro.toml"),
            ) {
                let (result, _) = save(&mut repo, false);
                let outcome = result.unwrap();
                assert_eq!(rules_warnings(&outcome), [warning::RULES_FILE_SKIPPED]);
            }
        }

        #[test]
        fn the_rules_are_read_from_the_folder_not_the_save() {
            let (dir, mut repo) = project();
            let rules = dir.path().join("opalro.toml");
            fs::write(&rules, "chunk-files-over = \"4 MiB\"\n").unwrap();
            assert_eq!(repo.rules().rules, Rules::default(), "nothing read yet");
            assert_eq!(repo.read_rules().rules.chunk_files_over, 4 << 20);
            save(&mut repo, true).0.unwrap();
            assert_eq!(repo.rules().rules.chunk_files_over, 4 << 20);
            // Changed, not saved: the folder's copy is the one that counts.
            fs::write(&rules, "chunk-files-over = \"2 MiB\"\n").unwrap();
            let three = noise(4, 3 << 20);
            fs::write(dir.path().join("three"), &three).unwrap();
            save(&mut repo, true).0.unwrap();
            assert!(chunk_lists(&repo).contains(&hash_content(&three)));
            // Gone from the folder, it no longer counts, though the last save holds it.
            fs::remove_file(&rules).unwrap();
            let one_and_a_half = noise(5, 3 << 19);
            fs::write(dir.path().join("one-and-a-half"), &one_and_a_half).unwrap();
            let (result, _) = save(&mut repo, true);
            assert!(rules_warnings(&result.unwrap()).is_empty());
            assert_eq!(repo.rules().rules, Rules::default());
            assert!(chunk_lists(&repo).contains(&hash_content(&one_and_a_half)));
        }

        /// The rules read from a save whose top folder holds `entry` as `opalro.toml`, its
        /// contents stored first when there are any.
        fn saved(
            repo: &mut Repository,
            kind: EntryKind,
            contents: Option<&[u8]>,
            size: u64,
        ) -> RulesRead {
            let id = match contents {
                Some(bytes) => {
                    let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
                    let mut packs = repo.new_packs().unwrap();
                    let id = packs.add_content(bytes).unwrap();
                    repo.store_packs(&lock, packs).unwrap();
                    id
                }
                // Nothing stored under it: reading it would fail.
                None => Id([7; 32]),
            };
            let entry = crate::listing::Entry {
                name: "opalro.toml".into(),
                kind,
                id,
                size,
            };
            let root = Listing::new(vec![entry]).unwrap();
            read_saved(repo.store(), &root).unwrap()
        }

        #[test]
        fn a_saves_rules_file_is_read_on_the_folders_terms() {
            let (_dir, mut repo) = project();
            let none = read_saved(repo.store(), &Listing::empty()).unwrap();
            assert!(!none.file && none.skipped.is_none() && none.ignored.is_empty());
            let text = b"[[rule]]\nfiles = [\"*.unity\"]\nconflict-markers = false\n";
            for kind in [EntryKind::File, EntryKind::Executable] {
                let read = saved(&mut repo, kind, Some(text), text.len() as u64);
                assert!(read.file && read.skipped.is_none() && read.ignored.is_empty());
                assert_eq!(read.rules.rules.len(), 1);
            }
            let broken = saved(&mut repo, EntryKind::File, Some(b"[[rule]\n"), 8);
            assert!(broken.skipped.unwrap().starts_with("it isn't TOML: line 1"));
            let part = saved(&mut repo, EntryKind::File, Some(b"colour = \"red\"\n"), 15);
            assert_eq!(part.ignored.len(), 1);
            // Exactly 1 MiB is read.
            let mut full = vec![b'#'; 1 << 20];
            full[(1 << 20) - 1] = b'\n';
            let read = saved(&mut repo, EntryKind::File, Some(&full), 1 << 20);
            assert!(read.file && read.skipped.is_none());
            // What isn't a regular file is passed over whole, with the reason a drive's gets.
            let link = saved(&mut repo, EntryKind::Symlink, Some(b"real.toml"), 9);
            assert_eq!(
                link.skipped.unwrap(),
                "it is a link, which Opalro doesn't follow here"
            );
            for kind in [EntryKind::Folder, EntryKind::ProjectLink] {
                let read = saved(&mut repo, kind, None, 0);
                assert_eq!(
                    read.skipped.unwrap(),
                    "it is a folder or special file, not a regular file"
                );
            }
        }

        #[test]
        fn a_saves_rules_file_over_1_mib_is_passed_over_without_reading_more() {
            let (_dir, mut repo) = project();
            // Its listed size decides: nothing is read, though nothing could be.
            let listed = saved(&mut repo, EntryKind::File, None, (1 << 20) + 1);
            assert_eq!(listed.skipped.unwrap(), "it is larger than 1 MiB");
            // A listing that says less than there is: the read stops past 1 MiB.
            let mut over = vec![b'#'; (1 << 20) + 1];
            over[1 << 20] = b'\n';
            let lied = saved(&mut repo, EntryKind::File, Some(&over), 10);
            assert_eq!(lied.skipped.unwrap(), "it is larger than 1 MiB");
        }

        const NO_MARKERS: &[u8] = b"[[rule]]\nfiles = [\"*.unity\"]\nconflict-markers = false\n";

        /// The rules read from a save whose top folder holds a file of `NO_MARKERS` under each
        /// name of `names`, and `other.txt`.
        fn saved_as(repo: &mut Repository, names: &[&str]) -> Result<RulesRead, StoreError> {
            let named: Vec<_> = names.iter().map(|n| (*n, NO_MARKERS)).collect();
            saved_with(repo, &named)
        }

        /// The rules read from a save whose top folder holds each `(name, contents)`, and
        /// `other.txt`.
        fn saved_with(
            repo: &mut Repository,
            files: &[(&str, &[u8])],
        ) -> Result<RulesRead, StoreError> {
            let lock = repo.lock_for_change("test", &mut |_| {}).unwrap();
            let mut packs = repo.new_packs().unwrap();
            let mut entry = |name: &str, bytes: &[u8]| crate::listing::Entry {
                name: name.into(),
                kind: EntryKind::File,
                id: packs.add_content(bytes).unwrap(),
                size: bytes.len() as u64,
            };
            let mut entries: Vec<_> = files.iter().map(|(n, b)| entry(n, b)).collect();
            entries.push(entry("other.txt", NO_MARKERS));
            repo.store_packs(&lock, packs).unwrap();
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            read_saved(repo.store(), &Listing::new(entries).unwrap())
        }

        #[test]
        fn the_other_sides_rules_file_is_found_by_its_exact_name() {
            let (_dir, mut repo) = project();
            // The exact name wins over one alike but for case, which holds two rules.
            let two = [NO_MARKERS, NO_MARKERS].concat();
            for files in [
                &[("opalro.toml", NO_MARKERS)][..],
                &[("OPALRO.TOML", &two[..]), ("opalro.toml", NO_MARKERS)],
            ] {
                let read = saved_with(&mut repo, files).unwrap();
                assert!(read.file && read.skipped.is_none());
                assert_eq!(read.rules.rules.len(), 1);
            }
            let read = saved_with(&mut repo, &[("OPALRO.TOML", &two[..])]).unwrap();
            assert_eq!(read.rules.rules.len(), 2);
        }

        #[test]
        fn failing_that_by_a_name_differing_only_in_letter_case() {
            let (_dir, mut repo) = project();
            for name in ["Opalro.toml", "OPALRO.TOML"] {
                let read = saved_as(&mut repo, &[name]).unwrap();
                assert!(read.file && read.skipped.is_none(), "{name}");
                assert_eq!(read.rules.rules.len(), 1);
            }
            // Only ASCII letters' case: anything else is another name.
            let read = saved_as(&mut repo, &["opalro.toml.txt", "\u{f8}palro.toml"]).unwrap();
            assert!(!read.file && read.skipped.is_none());
        }

        #[test]
        fn several_names_alike_but_for_case_count_as_not_read_whole() {
            let (_dir, mut repo) = project();
            let read = saved_as(&mut repo, &["OPALRO.toml", "Opalro.toml"]).unwrap();
            assert_eq!(
                read.skipped.as_deref(),
                Some("several files have its name but for letter case")
            );
            let matcher = RuleMatcher::new(&RulesRead::default(), &read);
            assert_eq!(
                matcher.line_merge("a.txt", None),
                LineMerge::RulesNotWhole { theirs: true }
            );
        }

        #[test]
        fn a_missing_or_damaged_rules_file_object_is_the_stores_error() {
            let (_dir, repo) = project();
            for name in ["opalro.toml", "Opalro.toml"] {
                let entry = crate::listing::Entry {
                    name: name.into(),
                    kind: EntryKind::File,
                    // Nothing stored under it.
                    id: Id([7; 32]),
                    size: 10,
                };
                let root = Listing::new(vec![entry]).unwrap();
                let err = read_saved(repo.store(), &root).unwrap_err();
                assert!(matches!(err, StoreError::Missing(_)), "{err:?}");
            }
            // Its only copy fails its check.
            let damaged = b"[[rule]]
files = [\"*.x\"]
conflict-markers = false
";
            crate::test_support::plant_damaged_copies(
                repo.packs_dir(),
                &[(ObjectKind::Content, damaged)],
            );
            let repo = Repository::open(repo.root()).unwrap();
            for name in ["opalro.toml", "Opalro.toml"] {
                let entry = crate::listing::Entry {
                    name: name.into(),
                    kind: EntryKind::File,
                    id: hash_content(damaged),
                    size: damaged.len() as u64,
                };
                let root = Listing::new(vec![entry]).unwrap();
                let err = read_saved(repo.store(), &root).unwrap_err();
                assert!(!matches!(err, StoreError::Missing(_)), "{err:?}");
            }
        }
    }
}
