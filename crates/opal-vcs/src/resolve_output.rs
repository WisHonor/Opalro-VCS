//! What `opalro resolve` shows people: what it recorded and what is left, or the version it
//! copied out.

use opal_core::resolve::{Copied, Pick, ResolveOutcome, Side};
use opal_core::snapshot::who_shows;
use opal_core::status::ConflictAt;
use opal_core::text::{escape, thousands};
use opal_core::warning::Warning;
use serde_json::{Value, json};

use crate::goto_output::{at_save, with_details};
use crate::output::{self, OPALRO_JSON, counted};
use crate::style::{self, Role};

/// `opalro resolve`'s outcome line, details and what is left. `keep` is the side `--keep` took;
/// `at` names the save that holds no conflict, as `goto` names one.
pub fn shown(done: &ResolveOutcome, keep: Option<Side>, at: &str) -> String {
    let s = style::out();
    let (save, short, replaced, shown_by, recorded, left, left_count, safety_copies) = match done {
        ResolveOutcome::NoConflicts { .. } => {
            return format!("Nothing to resolve: save {at} holds no conflict.\n");
        }
        ResolveOutcome::Nothing { left, left_count } => {
            let text = "Nothing to record: no place in conflict is fixed in your folder.\n";
            if left.is_empty() {
                return text.to_owned();
            }
            return format!("{text}\n{}", rows(s, left, *left_count, ""));
        }
        ResolveOutcome::Recorded {
            save,
            short_change_id,
            replaced,
            shown_by,
            recorded,
            left,
            left_count,
            safety_copies,
            ..
        } => (
            save,
            short_change_id,
            replaced,
            shown_by,
            recorded,
            left,
            *left_count,
            *safety_copies,
        ),
    };
    let fixes = counted(recorded.len() as u64, "fix", "fixes");
    let into = if replaced.is_some() {
        "save"
    } else {
        "a new save"
    };
    let on = save.line().map_or(String::new(), |line| {
        format!(" on {}", s.paint(Role::Line, &escape(line, false)))
    });
    let text = format!(
        "{} {fixes} in {into} {}{on}.\n",
        s.paint(Role::Ok, "Recorded"),
        at_save(short, Some(save))
    );
    let one = recorded.len() == 1;
    let them = if one { "it" } else { "them" };
    let mut details = vec![if replaced.is_some() {
        format!("The combined save now holds {them}; its ID stays the same.")
    } else if shown_by.is_empty() {
        let own = if one {
            "the fix is a save of its own"
        } else {
            "the fixes are a save of their own"
        };
        format!("Your newest save isn't a combined save, so {own}.")
    } else {
        format!(
            "The combined save couldn't take {them}: {} it too.",
            who_shows(shown_by)
        )
    }];
    if let Some(side) = keep {
        let side = match side {
            Side::Mine => "yours",
            Side::Theirs => "theirs",
        };
        let places = counted(recorded.len() as u64, "place", "places");
        details.push(format!("Took {side} for {places}."));
    }
    if safety_copies > 0 {
        details.push("A safety copy of what your folder held was kept.".to_owned());
    }
    if left_count > 0 {
        details.push(match left_count {
            1 => "1 place is still in conflict:".to_owned(),
            n => format!("{} places are still in conflict:", thousands(n)),
        });
    }
    let mut text = with_details(text, &details);
    // The rows sit under that last detail, so they have four columns less.
    let under = style::Screen {
        width: s.width.map(|w| w.saturating_sub(4)),
        ..s
    };
    text += &rows(under, left, left_count, "    ");
    text
}

/// `status`'s rows for `left`, each after `indent`, then how many more there are.
fn rows(s: style::Screen, left: &[ConflictAt], left_count: u64, indent: &str) -> String {
    let mut text: String = output::conflict_rows(s, left)
        .iter()
        .map(|row| format!("{indent}{row}\n"))
        .collect();
    let more = left_count.saturating_sub(left.len() as u64);
    if more > 0 {
        text += &format!("{indent}  … and {} more\n", thousands(more));
    }
    text
}

/// `opalro resolve --json`'s document (docs/json.md).
pub fn document(done: &ResolveOutcome, warnings: &[Warning]) -> Value {
    let (result, left, left_count) = match done {
        ResolveOutcome::Recorded {
            left, left_count, ..
        } => ("recorded", &left[..], *left_count),
        ResolveOutcome::Nothing { left, left_count } => ("nothing", &left[..], *left_count),
        ResolveOutcome::NoConflicts { .. } => ("no_conflicts", &[][..], 0),
    };
    let mut document = json!({
        "opalro_json": OPALRO_JSON,
        "result": result,
        "save": null,
        "replaced": null,
        "recorded": [],
        "conflicts": left.iter().map(output::conflict_json).collect::<Vec<_>>(),
        "more_conflicts": left_count.saturating_sub(left.len() as u64),
        "conflict_count": left_count,
        "written": 0,
        "removed": 0,
        "safety_copies": 0,
        "warnings": output::warnings_json(warnings),
    });
    if let ResolveOutcome::Recorded {
        save_id,
        save,
        short_change_id,
        replaced,
        recorded,
        applied,
        safety_copies,
        ..
    } = done
    {
        document["save"] = output::save_json(save_id, save, short_change_id);
        document["replaced"] = json!(replaced.map(|id| id.to_hex()));
        document["recorded"] = json!(recorded);
        document["written"] = applied.written.into();
        document["removed"] = applied.removed.into();
        document["safety_copies"] = (*safety_copies).into();
    }
    document
}

/// `--copy`'s outcome line: `path`'s version `pick` written to `to`, as typed.
pub fn copied(path: &str, pick: Pick, to: &str) -> String {
    let s = style::out();
    let path = s.paint(Role::Path, &escape(path, false));
    let what = match pick {
        Pick::Mine => format!("your version of {path}"),
        Pick::Theirs => format!("the other line's version of {path}"),
        Pick::Before => format!("{path} as it was where the lines parted"),
    };
    format!(
        "{} {what} to {}.\n",
        s.paint(Role::Ok, "Wrote"),
        s.paint(Role::Path, &escape(to, false))
    )
}

/// `opalro resolve --copy --json`'s document (docs/json.md).
pub fn copied_document(
    path: &str,
    pick: Pick,
    to: &str,
    copied: Copied,
    warnings: &[Warning],
) -> Value {
    json!({
        "opalro_json": OPALRO_JSON,
        "result": "copied",
        "path": path,
        "version": match pick {
            Pick::Mine => "mine",
            Pick::Theirs => "theirs",
            Pick::Before => "before",
        },
        "to": to,
        "size": copied.size,
        "warnings": output::warnings_json(warnings),
    })
}
