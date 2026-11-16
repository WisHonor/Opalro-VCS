//! What `opalro who` shows people: each line of a file beside the save it is from; its `--json`
//! document; and its refusals.

use std::collections::HashMap;
use std::path::Path;

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
    pub save: Save,
    pub short: String,
}

/// The most of an author's name a terminal shows.
const NAME_ROOM: usize = 20;

/// The saves `runs` names, each once, in the order of its first run.
pub fn named_in(whose: &Whose) -> Vec<Id> {
    let mut ids: Vec<Id> = Vec::new();
    for run in &whose.runs {
        if let By::Save { id, .. } = run.by
            && !ids.contains(&id)
        {
            ids.push(id);
        }
    }
    ids
}

fn line_count(whose: &Whose) -> usize {
    whose.runs.iter().map(|r| r.lines).sum()
}

/// `opalro who`'s outcome line and rows; `asked` when `--save` named the save.
pub fn shown(whose: &Whose, asked: bool, saves: &HashMap<Id, Named>) -> String {
    let s = style::out();
    let path = s.paint(Role::Path, &escape(&whose.path, false));
    let n = line_count(whose);
    if n == 0 {
        return format!("{path} is empty.\n");
    }
    let unsaved: usize = (whose.runs.iter())
        .filter(|r| r.by == By::Unsaved)
        .map(|r| r.lines)
        .sum();
    if unsaved == n {
        return match n {
            1 => format!("{path} isn't saved yet: its 1 line isn't from a save.\n"),
            n => format!(
                "{path} isn't saved yet: none of its {} lines is from a save.\n",
                thousands(n as u64)
            ),
        };
    }
    let mut text = if asked {
        let short = saves
            .get(&whose.at)
            .map_or_else(|| whose.at.short_hex(), |named| named.short.clone());
        format!(
            "Each line of {path} as save {} has it, and the save it is from:\n\n",
            s.paint(Role::ChangeId, &short)
        )
    } else {
        format!("Each line of {path} and the save it is from:\n\n")
    };

    // The three columns of each save, escaped, the name cut on a terminal.
    let cells: HashMap<Id, [String; 3]> = named_in(whose)
        .into_iter()
        .map(|id| {
            let cells = match saves.get(&id) {
                Some(Named { save, short }) => [
                    short.clone(),
                    s.cut_end(&escape(&save.author.name, false), NAME_ROOM),
                    output::date(save.author.time_ms, save.author.tz_offset_min),
                ],
                None => [id.short_hex(), String::new(), String::new()],
            };
            (id, cells)
        })
        .collect();
    let mut widths = [0; 3];
    for row in cells.values() {
        for (width, cell) in widths.iter_mut().zip(row) {
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

    let by_line = (whose.runs.iter()).flat_map(|r| std::iter::repeat_n(&r.by, r.lines));
    let mut before: Option<&By> = None;
    for (i, (line, by)) in whose
        .text
        .split_inclusive(|b| *b == b'\n')
        .zip(by_line)
        .enumerate()
    {
        let again = before.is_some_and(|b| same_save(b, by));
        before = Some(by);
        let left = match saved(by).and_then(|id| cells.get(&id)) {
            Some([short, name, date]) => {
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

fn saved(by: &By) -> Option<Id> {
    match by {
        By::Save { id, .. } => Some(*id),
        By::Unsaved => None,
    }
}

/// Whether two rows name the same save, or both are not saved yet.
fn same_save(a: &By, b: &By) -> bool {
    saved(a) == saved(b)
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
pub fn document(
    whose: &Whose,
    asked: bool,
    saves: &HashMap<Id, Named>,
    warnings: &[Warning],
) -> Value {
    let save_json = |id: &Id| {
        saves
            .get(id)
            .map(|named| output::save_json(id, &named.save, &named.short))
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
        "save": save_json(&whose.at),
        "lines": line_count(whose),
        "runs": runs,
        "saves": named_in(whose).iter().filter_map(save_json).collect::<Vec<_>>(),
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
