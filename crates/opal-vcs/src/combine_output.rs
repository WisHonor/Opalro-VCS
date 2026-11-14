//! What `opalro combine` shows people: which of its three cases it was, and what it wrote.

use opal_core::combine::{CombineOutcome, Combined};
use opal_core::save::Save;
use opal_core::text::escape;

use crate::goto_output::{at_save, with_details, written};
use crate::output;
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
        Combined::Made { merged_files, .. } => {
            let details: Vec<String> = (!merged_files.is_empty())
                .then(|| format!("Merged {} line by line.", output::files(merged_files.len())))
                .into_iter()
                .collect();
            let combined = s.paint(Role::Ok, "Combined");
            let text = format!("{combined} {painted_target} into {painted_line}: save {at}.\n");
            (text, details)
        }
    };
    details.extend(written(&done.applied, done.carried));
    with_details(text, &details)
}
