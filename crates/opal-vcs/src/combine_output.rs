//! What `opalro combine` shows people: which of its three cases it was, and what it wrote.

use opal_core::combine::{Clash, CombineOutcome, Combined, Conflict, NotMerged, Stands};
use opal_core::save::Save;
use opal_core::text::{escape, thousands};
use serde_json::{Value, json};

use crate::goto_output::{at_save, with_details, written};
use crate::output::{self, counted};
use crate::style::{self, Role};

/// `opalro combine`'s outcome line and details. `target` names the target, made safe to print, and
/// `by_id` says whether that is a save's ID; `short` and `save` name the line's newest save as
/// `goto` names one; `message_given` whether the user gave a message.
pub fn combined(
    done: &CombineOutcome,
    target: &str,
    by_id: bool,
    short: &str,
    save: Option<&Save>,
    message_given: bool,
) -> String {
    let s = style::out();
    let line = escape(&done.line, false);
    let painted_line = s.paint(Role::Line, &line);
    let painted_target = s.paint(if by_id { Role::ChangeId } else { Role::Line }, target);
    let at = at_save(short, save);
    let (text, mut details) = match &done.result {
        Combined::Nothing => {
            return format!(
                "Nothing to combine: {painted_line} already has everything {painted_target} has.\n"
            );
        }
        Combined::Forward => {
            let mut details = vec![format!(
                "{line} had nothing {target} lacks, so no combined save was needed."
            )];
            if message_given {
                details.push("No save was made, so your message wasn't used.".to_owned());
            }
            let moved = s.paint(Role::Ok, "Moved");
            let text = format!("{moved} {painted_line} forward to {painted_target}: save {at}.\n");
            (text, details)
        }
        Combined::Made {
            merged_files,
            conflicts,
            more,
            conflict_count,
            ..
        } => {
            let mut details = vec![];
            let made = conflicts.len() as u64 + more;
            let held = conflict_count.saturating_sub(made);
            let (noun, them) = if held == 1 {
                ("place", "it")
            } else {
                ("places", "them")
            };
            if held > 0 && made == 0 {
                details.push(format!(
                    "The two lines held {them} in conflict already; the save keeps {them}."
                ));
            } else if held > 0 {
                details.push(format!(
                    "The two lines held {} more {noun} in conflict already; the save keeps {them}.",
                    thousands(held)
                ));
            }
            if !merged_files.is_empty() {
                details.push(format!(
                    "Merged {} line by line.",
                    output::files(merged_files.len())
                ));
            }
            let combined = s.paint(Role::Ok, "Combined");
            let mut text = format!("{combined} {painted_target} into {painted_line}: save {at}");
            if *conflict_count == 0 {
                text += ".\n";
            } else if made == 0 {
                let places = counted(*conflict_count, "place", "places");
                text += &format!(", with {places} in conflict.\n");
            } else {
                let places = counted(*conflict_count, "place", "places");
                text += &format!(", with {places} in conflict:\n");
                for c in conflicts {
                    text += &format!("  {}\n", row(c, &line, target));
                }
                if *more > 0 {
                    text += &format!("  … and {} more\n", thousands(*more));
                }
            }
            (text, details)
        }
    };
    details.extend(written(&done.applied, done.carried));
    if done.result == Combined::Forward && done.holds_conflicts {
        let status = style::whole(&s.paint(Role::Command, "opalro status"));
        details.push(format!(
            "The save your line moved to holds places in conflict: {status} lists them."
        ));
    }
    with_details(text, &details)
}

/// `<path>: <what happened>; <how it stands>`, for a conflict the combine kept.
fn row(c: &Conflict, line: &str, target: &str) -> String {
    let stands = match c.stands {
        Some(Stands::Markers { places }) => format!(
            "marked in the file ({})",
            counted(places, "place", "places")
        ),
        Some(Stands::Yours) => "your version is in the folder".to_owned(),
        Some(Stands::Nothing) => "nothing is in your folder".to_owned(),
        None => return c.describe(line, target),
    };
    format!("{}; {stands}", c.describe(line, target))
}

/// A kept conflict as `opalro combine --json` lists it.
pub fn conflict_json(c: &Conflict) -> Value {
    let (clash, why) = match &c.clash {
        Clash::BothChanged(why) => (
            "changed_on_both",
            Some(match why {
                NotMerged::Overlap {
                    endings_only: false,
                } => "overlap",
                NotMerged::Overlap { endings_only: true } => "endings_only",
                NotMerged::Binary => "binary",
                NotMerged::TooBig => "too_big",
                NotMerged::TooManyLines => "too_many_lines",
                NotMerged::NoMarkers { .. } => "no_markers",
                NotMerged::RulesNotWhole { .. } => "rules_not_whole",
                NotMerged::NotFiles => "not_files",
            }),
        ),
        Clash::ChangedRemoved { .. } => ("changed_and_removed", None),
        Clash::BothAdded => ("added_on_both", None),
        Clash::FileAndFolder { .. } => ("file_and_folder", None),
        Clash::MovedApart { .. } => ("moved_apart", None),
        Clash::MovedRemoved { .. } => ("moved_and_removed", None),
        Clash::MovedOnto { .. } => ("moved_onto", None),
        Clash::StartUnknown => ("start_unknown", None),
    };
    let (stands, places) = match c.stands {
        Some(Stands::Markers { places }) => (Some("markers"), Some(places)),
        Some(Stands::Yours) => (Some("yours"), None),
        Some(Stands::Nothing) => (Some("nothing"), None),
        None => (None, None),
    };
    json!({
        "path": c.path,
        "clash": clash,
        "why": why,
        "stands": stands,
        "places": places,
    })
}
