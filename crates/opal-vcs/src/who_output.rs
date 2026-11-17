//! What `opalro who` shows people: each line of a file beside the save it is from; its `--json`
//! document; and its refusals.

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use opal_core::id::Id;
use opal_core::names::GivenPathError;
use opal_core::save::Save;
use opal_core::text::{escape, thousands};
use opal_core::warning::Warning;
use opal_core::who::{By, WhoError, Whose};
use serde_json::{Value, json};

use crate::output::{self, Failure, OPALRO_JSON};
use crate::style::{self, Role, Sym};

/// A save the rows name, and its short change ID as `opalro show` prints it.
pub struct Named {
    pub id: Id,
    pub save: Save,
    pub short: String,
}

/// The saves an answer names, each read once: the one asked about, each save the runs name in
/// the order of its first run, and each run's own.
pub struct Saves {
    at: Rc<Read>,
    named: Vec<Rc<Read>>,
    runs: Vec<Option<Rc<Read>>>,
}

/// A save read, and its three columns as a row shows them: escaped, the name cut on a terminal.
struct Read {
    named: Named,
    cells: [String; 3],
}

impl Read {
    fn of(named: Named) -> Read {
        let s = style::out();
        let author = &named.save.author;
        let cells = [
            named.short.clone(),
            s.cut_end(&escape(&author.name, false), NAME_ROOM),
            output::date(author.time_ms, author.tz_offset_min),
        ];
        Read { named, cells }
    }
}

impl Saves {
    /// Reads, with `read`, the saves `whose` names.
    pub fn read(
        whose: &Whose,
        mut read: impl FnMut(&Id) -> Result<Named, Failure>,
    ) -> Result<Saves, Failure> {
        let mut by_id: HashMap<Id, Rc<Read>> = HashMap::new();
        let mut named = Vec::new();
        let mut once = |id: &Id, named: &mut Vec<Rc<Read>>, listed: bool| {
            if let Some(known) = by_id.get(id) {
                return Ok(Rc::clone(known));
            }
            let save = Rc::new(Read::of(read(id)?));
            by_id.insert(*id, Rc::clone(&save));
            if listed {
                named.push(Rc::clone(&save));
            }
            Ok::<_, Failure>(save)
        };
        let mut runs = Vec::new();
        for run in &whose.runs {
            runs.push(match &run.by {
                By::Save { id, .. } => Some(once(id, &mut named, true)?),
                By::Unsaved => None,
            });
        }
        let at = once(&whose.at, &mut named, false)?;
        Ok(Saves { at, named, runs })
    }
}

/// The most of an author's name a terminal shows.
const NAME_ROOM: usize = 20;

fn line_count(whose: &Whose) -> usize {
    whose.runs.iter().map(|r| r.lines).sum()
}

/// `opalro who`'s outcome line and rows; `asked` when `--save` named the save.
pub fn shown(whose: &Whose, asked: bool, saves: &Saves) -> String {
    let s = style::out();
    let path = s.paint(Role::Path, &escape(&whose.path, false));
    let n = line_count(whose);
    if n == 0 {
        return format!("{path} is empty.\n");
    }
    if !whose.in_save {
        return match n {
            1 => format!("{path} isn't saved yet: its 1 line isn't from a save.\n"),
            n => format!(
                "{path} isn't saved yet: none of its {} lines is from a save.\n",
                thousands(n as u64)
            ),
        };
    }
    let mut text = if asked {
        format!(
            "Each line of {path} as save {} has it, and the save it is from:\n\n",
            s.paint(Role::ChangeId, &saves.at.named.short)
        )
    } else {
        format!("Each line of {path} and the save it is from:\n\n")
    };

    let mut widths = [0; 3];
    for read in &saves.named {
        for (width, cell) in widths.iter_mut().zip(&read.cells) {
            *width = (*width).max(style::columns(cell));
        }
    }
    let pad = |text: &str, width: usize| {
        format!(
            "{text}{}",
            " ".repeat(width.saturating_sub(style::columns(text)))
        )
    };
    let across = widths[0] + 2 + widths[1] + 2 + widths[2];
    let number = n.to_string().len();
    let bar = s.paint(Role::Dim, s.sym(Sym::Bar));

    let by_line = (whose.runs.iter().zip(&saves.runs))
        .flat_map(|(r, read)| std::iter::repeat_n(read, r.lines));
    let mut before = None;
    let mut unsaved = 0;
    for (i, (line, read)) in whose
        .text
        .split_inclusive(|b| *b == b'\n')
        .zip(by_line)
        .enumerate()
    {
        // A row is dim when the row above names the same save, or is not saved yet too.
        let id = read.as_ref().map(|r| r.named.id);
        let again = before == Some(id);
        before = Some(id);
        let left = match read {
            Some(read) => {
                let [short, name, date] = &read.cells;
                let plain = [
                    pad(short, widths[0]),
                    pad(name, widths[1]),
                    pad(date, widths[2]),
                ];
                if again {
                    s.paint(Role::Dim, &plain.join("  "))
                } else {
                    let [short, name, date] = plain;
                    format!("{}  {name}  {date}", s.paint(Role::ChangeId, &short))
                }
            }
            None => {
                unsaved += 1;
                let plain = pad("not saved yet", across);
                if again {
                    s.paint(Role::Dim, &plain)
                } else {
                    plain
                }
            }
        };
        let at = s.paint(Role::Dim, &format!("{:>number$}", i + 1));
        let line = output::file_line(line);
        let space = if line.is_empty() { "" } else { " " };
        text += &format!("{left}  {at} {bar}{space}{line}\n");
    }
    if unsaved > 0 && !asked {
        text += &match unsaved {
            1 => "\n1 line isn't saved yet.\n".to_owned(),
            n => format!("\n{} lines aren't saved yet.\n", thousands(n as u64)),
        };
    }
    text
}

/// What a comparison past its deadline means for the answer, said once after it.
pub fn coarse_warning() -> Warning {
    Warning::new(
        opal_core::warning::DIFF_TIMED_OUT,
        "Comparing some versions took too long, so a line may be named for a newer save than the \
         one that first had it.",
    )
}

/// `opalro who --json`'s document (docs/json.md).
pub fn document(whose: &Whose, asked: bool, saves: &Saves, warnings: &[Warning]) -> Value {
    let save_json = |read: &Read| {
        let named = &read.named;
        output::save_json(&named.id, &named.save, &named.short)
    };
    let runs: Vec<Value> = whose
        .runs
        .iter()
        .map(|run| {
            let (save, was) = match &run.by {
                By::Save { id, was } => (Some(id.to_hex()), was.as_deref()),
                By::Unsaved => (None, None),
            };
            json!({"first": run.first, "lines": run.lines, "save": save, "was": was})
        })
        .collect();
    json!({
        "opalro_json": OPALRO_JSON,
        "path": whose.path,
        "from": if asked { "save" } else { "folder" },
        "save": save_json(&saves.at),
        "lines": line_count(whose),
        "runs": runs,
        "saves": saves.named.iter().map(|read| save_json(read)).collect::<Vec<_>>(),
        "coarse": whose.coarse,
        "warnings": output::warnings_json(warnings),
    })
}

/// The refusal for `e`; `typed` is the path as it was typed.
pub fn failure(e: WhoError, typed: &Path) -> Failure {
    let hint = match e {
        WhoError::NoFile { save: None, .. } => {
            Some("To read it as a save has it: opalro who <file> --save <save>".to_owned())
        }
        // A path the command line made is always one; if not, it is refused as `changes` would.
        WhoError::NotAPath { reason, .. } => {
            let typed = typed.to_string_lossy().into_owned();
            return GivenPathError::Invalid(typed, reason).into();
        }
        WhoError::Repo(e) => return e.into(),
        // Only reading, so the hint never says that nothing was saved.
        WhoError::Save(e) => {
            return Failure {
                hint: e.hint_when_reading(),
                message: e.to_string(),
                printed: false,
            };
        }
        _ => None,
    };
    Failure {
        message: e.to_string(),
        hint,
        printed: false,
    }
}
