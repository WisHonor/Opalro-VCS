//! Opalro's line-by-line merge against `git merge-file`, on texts drawn at random.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::cell::RefCell;

use opal_core::changes::DEADLINE;
use opal_core::combine::{Text, merge_text};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};
use support::git_with_input;

const ALPHABET: &[&str] = &["a\n", "b\n", "c\n", "d\n"];

/// Replace, delete or insert a run of 1 to 3 lines somewhere, or (unique lines only) flip one line's
/// ending; `picks` choose the new lines when they come from the alphabet.
#[derive(Clone, Debug)]
struct Edit {
    kind: u8,
    at: prop::sample::Index,
    count: usize,
    picks: [usize; 3],
}

fn edits() -> impl Strategy<Value = Vec<Edit>> {
    prop::collection::vec(
        (
            0..4u8,
            any::<prop::sample::Index>(),
            1..=3usize,
            [0..4usize, 0..4usize, 0..4usize],
        )
            .prop_map(|(kind, at, count, picks)| Edit {
                kind,
                at,
                count,
                picks,
            }),
        0..=4,
    )
}

/// `base` with `edits` made; new lines are fresh (`<side> <n>`) or drawn from the alphabet.
fn edited(base: &[String], edits: &[Edit], side: &str, unique: bool) -> Vec<String> {
    let mut lines = base.to_vec();
    let mut fresh = 0;
    for e in edits {
        let at = e.at.index(lines.len() + 1);
        let new: Vec<String> = (0..e.count)
            .map(|i| {
                if unique {
                    fresh += 1;
                    format!("{side} {fresh}\n")
                } else {
                    ALPHABET[e.picks[i]].to_owned()
                }
            })
            .collect();
        let gone = e.count.min(lines.len() - at);
        match e.kind {
            0 => {
                lines.splice(at..at + gone, new);
            }
            1 => {
                lines.drain(at..at + gone);
            }
            // A `\r` here turns the line's ending the other way in `with_endings`.
            3 if unique => {
                if let Some(line) = lines.get_mut(at) {
                    *line = match line.strip_suffix("\r\n") {
                        Some(body) => format!("{body}\n"),
                        None => line.replace('\n', "\r\n"),
                    };
                }
            }
            _ => {
                lines.splice(at..at, new);
            }
        }
    }
    lines
}

fn three(unique: bool) -> impl Strategy<Value = [Vec<u8>; 3]> {
    let base = if unique {
        (5..=40usize)
            .prop_map(|n| (0..n).map(|i| format!("line {i}\n")).collect::<Vec<_>>())
            .boxed()
    } else {
        prop::collection::vec(prop::sample::select(ALPHABET), 5..=40)
            .prop_map(|lines| lines.into_iter().map(str::to_owned).collect())
            .boxed()
    };
    let endings = (any::<u64>(), any::<[bool; 3]>());
    (base, edits(), edits(), endings).prop_map(move |(base, ours, theirs, (crlf, open))| {
        let ours = edited(&base, &ours, "ours", unique);
        let theirs = edited(&base, &theirs, "theirs", unique);
        let mut texts = [base, ours, theirs].map(|lines| lines.concat().into_bytes());
        if unique {
            for (text, open) in texts.iter_mut().zip(open) {
                *text = with_endings(text, crlf, open);
            }
        }
        texts
    })
}

/// `text` with some lines ending in `\r\n`, chosen by `crlf` and the line's contents so that a
/// line keeps its ending in all three texts unless an edit flipped it, and maybe no line feed at
/// the end.
fn with_endings(text: &[u8], crlf: u64, open: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for line in text.split_inclusive(|&b| b == b'\n') {
        let (body, flipped) = match line.strip_suffix(b"\r\n") {
            Some(body) => (body, true),
            None => (&line[..line.len() - 1], false),
        };
        let sum: u32 = body.iter().map(|&b| u32::from(b)).sum();
        out.extend_from_slice(body);
        if (crlf >> (sum % 64) & 1 == 1) != flipped {
            out.push(b'\r');
        }
        out.push(b'\n');
    }
    if open {
        out.pop();
        if out.ends_with(b"\r") {
            out.pop();
        }
    }
    out
}

/// What `git merge-file -p ours base theirs` gives: the bytes when it merged cleanly.
fn git_merge(texts: &[Vec<u8>; 3]) -> Option<Vec<u8>> {
    let dir = tempfile::tempdir().unwrap();
    for (name, bytes) in ["base", "ours", "theirs"].iter().zip(texts) {
        std::fs::write(dir.path().join(name), bytes).unwrap();
    }
    let out = git_with_input(
        dir.path(),
        &["merge-file", "-p", "ours", "base", "theirs"],
        b"",
    );
    let code = out.status.code().unwrap();
    assert!(
        (0..=127).contains(&code),
        "git merge-file failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    (code == 0).then_some(out.stdout)
}

fn ours_merge(texts: &[Vec<u8>; 3]) -> Option<Vec<u8>> {
    match merge_text(&texts[0], &texts[1], &texts[2], DEADLINE).0 {
        Text::Merged(bytes) => Some(bytes),
        Text::Not(_) => None,
    }
}

fn shown(texts: &[Vec<u8>; 3]) -> String {
    let [base, ours, theirs] = texts
        .each_ref()
        .map(|t| String::from_utf8_lossy(t).into_owned());
    format!("base {base:?}\nours {ours:?}\ntheirs {theirs:?}")
}

fn config() -> Config {
    Config {
        cases: 256,
        failure_persistence: None,
        ..Config::default()
    }
}

/// Unique lines, ending in `\n` or `\r\n`, the last maybe in neither: the same answer as Git's,
/// always. The seed is fixed so that both outcomes surely occur often enough.
#[test]
fn text_merge_matches_git_merge_file_on_unique_lines() {
    let counts = RefCell::new((0, 0));
    let rng = TestRng::deterministic_rng(RngAlgorithm::ChaCha);
    TestRunner::new_with_rng(config(), rng)
        .run(&three(true), |texts| {
            let git = git_merge(&texts);
            prop_assert_eq!(&ours_merge(&texts), &git, "{}", shown(&texts));
            let mut c = counts.borrow_mut();
            if git.is_some() {
                c.0 += 1
            } else {
                c.1 += 1
            }
            Ok(())
        })
        .unwrap();
    let (clean, conflicts) = counts.into_inner();
    println!("{clean} clean, {conflicts} conflicts");
    assert!(
        clean >= 25 && conflicts >= 25,
        "{clean} clean, {conflicts} conflicts"
    );
}

/// Where lines repeat, base and a side can be paired in two ways that cost the same, and each
/// program's merge is right for its own pairing: only Opalro merges, only Git does, or both do
/// with different bytes. The seed is fixed and each count kept under a ceiling, so a large change
/// in how lines are paired shows. What holds whatever the pairing is checked in every case.
#[test]
fn text_merge_and_git_merge_file_on_repeating_lines() {
    const KINDS: [&str; 3] = [
        "both merged, different bytes",
        "only Opalro merged",
        "only Git merged",
    ];
    let found: RefCell<[Vec<String>; 3]> = RefCell::default();
    let rng = TestRng::deterministic_rng(RngAlgorithm::ChaCha);
    TestRunner::new_with_rng(config(), rng)
        .run(&three(false), |texts| {
            let [b, o, t] = &texts;
            let merged = |base: &[u8], ours: &[u8], theirs: &[u8]| {
                merge_text(base, ours, theirs, DEADLINE).0
            };
            prop_assert_eq!(merged(b, o, o), Text::Merged(o.clone()));
            prop_assert_eq!(merged(b, b, t), Text::Merged(t.clone()));
            prop_assert_eq!(merged(b, o, b), Text::Merged(o.clone()));
            let ours = ours_merge(&texts);
            let swapped = ours_merge(&[b.clone(), t.clone(), o.clone()]);
            prop_assert_eq!(&ours, &swapped, "{}", shown(&texts));
            let kind = match (ours, git_merge(&texts)) {
                (Some(ours), Some(git)) if ours != git => Some(0),
                (Some(_), None) => Some(1),
                (None, Some(_)) => Some(2),
                _ => None,
            };
            if let Some(k) = kind {
                found.borrow_mut()[k].push(shown(&texts));
            }
            Ok(())
        })
        .unwrap();
    let found = found.into_inner();
    for (kind, cases) in KINDS.iter().zip(&found) {
        println!("{kind}: {} cases", cases.len());
        if let Some(case) = cases.first() {
            println!("for example:\n{case}\n");
        }
    }
    // Ceilings, not exact counts: another Git may pair lines its own way.
    let counts = found.map(|c| c.len());
    let ceilings = [3, 6, 24];
    assert!(
        counts.iter().zip(ceilings).all(|(n, most)| *n <= most),
        "{KINDS:?}: {counts:?}, at most {ceilings:?}"
    );
}
