//! Red-team session RT26 (milestone 11, second session): proofs.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;
use opal_core::git::craft::{Craft, PackEntry};
use opal_core::git::{GitId, HashKind, ObjectType};
use support::{Env, text};

const COMMITS: usize = 20;

/// Writes into `env` a history the comparison's 1 GiB floor stops at its first comparison: two
/// top trees, taken in turns, each holding 760 copies of a folder of 1,000 files with 765-letter
/// names (793 KB of Git tree), the copies in one differing from those in the other by one file,
/// so each comparison walks 1.2 GB of trees from the cache. Every object is loose, and `buy/`
/// holds 256 more one-file trees, the same in both. `packs` packs of one blob each go beside
/// them.
fn costly_to_compare(env: &Env, packs: usize) {
    let git = Craft::new(env.root(), HashKind::Sha1);
    let blob = |text: &str| git.object(ObjectType::Blob, text.as_bytes());
    let (x, y, z) = (blob("X\n"), blob("Y\n"), blob("Z\n"));
    let buys: Vec<(String, GitId)> = (0..256)
        .map(|i| {
            let name = format!("x{i:04}");
            (
                format!("b{i:04}"),
                git.tree(&[(b"100644", name.as_bytes(), z)]),
            )
        })
        .collect();
    let buy = git.tree(
        &buys
            .iter()
            .map(|(n, id)| (b"40000".as_slice(), n.as_bytes(), *id))
            .collect::<Vec<_>>(),
    );
    let names: Vec<String> = (0..1000)
        .map(|j| format!("{j:04}{}", "a".repeat(761)))
        .collect();
    let mut tops = Vec::new();
    for last in [x, y] {
        let files: Vec<(&[u8], &[u8], GitId)> = names
            .iter()
            .enumerate()
            .map(|(j, n)| {
                (
                    b"100644".as_slice(),
                    n.as_bytes(),
                    if j == 999 { last } else { x },
                )
            })
            .collect();
        let folder = git.tree(&files);
        let copies: Vec<String> = (0..760).map(|i| format!("d{i:04}")).collect();
        let mut top: Vec<(&[u8], &[u8], GitId)> = vec![(b"40000", b"buy", buy)];
        top.extend(
            copies
                .iter()
                .map(|n| (b"40000".as_slice(), n.as_bytes(), folder)),
        );
        tops.push(git.tree(&top));
    }
    let mut tip: Option<GitId> = None;
    for i in 0..COMMITS {
        let mut raw = format!("tree {}\n", tops[i % 2].to_hex());
        if let Some(parent) = tip {
            raw += &format!("parent {}\n", parent.to_hex());
        }
        raw += "author T <t@e> 1000000000 +0000\ncommitter T <t@e> 1000000000 +0000\n\n";
        raw += &format!("commit {i}\n");
        tip = Some(git.commit(raw.as_bytes()));
    }
    git.loose_ref("refs/heads/main", &format!("{}\n", tip.unwrap().to_hex()));
    for i in 0..packs {
        let filler = format!("filler {i}\n").into_bytes();
        git.pack(&[PackEntry::Whole(ObjectType::Blob, filler)]);
    }
}

/// RT26-1: each pack index searched to find a tree the import converted added 1 KiB to the
/// base, so bought the comparison 32 KiB, though a search of an index in memory takes tens of
/// nanoseconds. Beside 4,095 packs of one blob each (about 5 MB, nothing of the history in
/// them), every loose tree read searched 4,096 indexes and bought 128 MiB of comparing. This
/// history stopped at its first comparison without them; with them it imported all 20 commits,
/// comparing 23 GB. In release, a larger history of the same shape compared 3.4 s for every
/// commit where v0.2 imported it whole in 0.87 s (`packbuy.py`).
#[test]
fn a_history_the_floor_stops_still_stops_beside_packs_of_one_blob() {
    let import = |packs: usize| {
        let env = Env::empty();
        costly_to_compare(&env, packs);
        let out = env.opalro().arg("import").output().unwrap();
        (out.status.code(), text(&out.stdout), text(&out.stderr))
    };
    let (code, _, said) = import(0);
    assert!(
        code != Some(0) && said.contains("comparing its commits"),
        "without the packs, the floor should stop this history: {said}"
    );
    let (code, out, said) = import(4095);
    assert!(
        code != Some(0) && said.contains("comparing its commits"),
        "beside 4,095 packs of one blob each, the import of a history the 1 GiB floor stops at its \
         first comparison went on, comparing about 1.2 GB for each of {} commits: {out}{said}",
        COMMITS - 1
    );
}
