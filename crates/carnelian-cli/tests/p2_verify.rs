//! P2 certification (literals tranche): `verify` compares
//! `compile --frontend prism` against the pinned C golden, byte for byte, in
//! both strip modes. Exit `0` required. Pattern matching and interpolated
//! symbols belong to later tranches and are locked as diagnostics below,
//! never as diverging bytes. Keyword hashes in call arguments opened in P2.5.

#![cfg(all(feature = "reference", feature = "prism"))]

use std::process::Command;

#[path = "corpus.rs"]
mod corpus;

use corpus::{P2_GATED as GATED, P2_SNIPPETS as SNIPPETS};

fn carnelian() -> Command {
    Command::new(env!("CARGO_BIN_EXE_carnelian"))
}

fn check_identical(name: &str, source: &str) {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join(format!("{name}.rb"));
    std::fs::write(&input, source).expect("write snippet");

    // `verify` checks both strip modes against the same golden.
    let verify = carnelian()
        .arg("verify")
        .arg(&input)
        .output()
        .expect("run verify");
    assert_eq!(
        verify.status.code(),
        Some(0),
        "{name}: verify failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&verify.stdout),
        String::from_utf8_lossy(&verify.stderr)
    );

    // `compile` output must also match the golden file byte for byte.
    let golden = dir.path().join(format!("{name}.mrb"));
    let reference = carnelian()
        .arg("reference")
        .arg(&input)
        .arg("-o")
        .arg(&golden)
        .output()
        .expect("run reference");
    assert!(reference.status.success(), "{name}: reference failed");
    for strip in [false, true] {
        let out = dir.path().join(format!("{name}.{strip}.mrb"));
        let mut command = carnelian();
        command.arg("compile").arg(&input).arg("-o").arg(&out);
        if strip {
            command.arg("--strip");
        }
        let compiled = command.output().expect("run compile");
        assert_eq!(
            compiled.status.code(),
            Some(0),
            "{name} (strip={strip}): compile failed: {}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        assert_eq!(
            std::fs::read(&golden).expect("read golden"),
            std::fs::read(&out).expect("read output"),
            "{name} (strip={strip}): bytes diverge"
        );
    }
}

#[test]
fn p2_corpus_is_byte_identical() {
    for (name, source) in SNIPPETS {
        check_identical(name, source);
    }
}

#[test]
fn p2_big_hash_hits_limit_paths() {
    // 70 pairs cross the stack flush threshold (99) mid-way, exercising the
    // mid-construction `HASH` flush and the trailing `HASHADD` paths.
    let source = corpus::synthetic_source("hash_big");
    check_identical("hash_big", &source);
}

#[test]
fn p2_wide_hash_hits_array_packing() {
    // 63 locals push the cursor to 64, lifting the flush threshold past
    // `INT16_MAX`, so the 70 pairs reach the trailing `len > limit` packing
    // (`HASH`, `-1`) instead of flushing mid-way.
    let source = corpus::synthetic_source("hash_wide");
    check_identical("hash_wide", &source);
}

#[test]
fn p2_deferred_syntax_is_gated() {
    for (name, source, marker) in GATED {
        let dir = tempfile::tempdir().expect("tempdir");
        let input = dir.path().join(format!("{name}.rb"));
        std::fs::write(&input, source).expect("write snippet");
        let out = dir.path().join(format!("{name}.mrb"));
        let compiled = carnelian()
            .arg("compile")
            .arg(&input)
            .arg("-o")
            .arg(&out)
            .output()
            .expect("run compile");
        assert_eq!(
            compiled.status.code(),
            Some(1),
            "{name}: expected a gating diagnostic"
        );
        let stderr = String::from_utf8_lossy(&compiled.stderr);
        assert!(
            stderr.contains(marker),
            "{name}: diagnostic lacks {marker:?}: {stderr}"
        );
    }
}
