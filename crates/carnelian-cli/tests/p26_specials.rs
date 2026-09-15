//! P2.6 certification (alias/undef, `defined?` and `case/in` pattern
//! matching): `verify` compares `compile --frontend prism` against the
//! pinned C golden, byte for byte, in both strip modes. Exit `0` required.
//! `BEGIN`/`END` and flip-flop belong to later work and are locked as
//! diagnostics below, never as diverging bytes, as are value patterns over
//! still-gated literal nodes (ranges,regexps) and `=~`. `defined?`
//! back-reference reads and chain links that hit still-gated call forms stay
//! gated for the same reason.

#![cfg(all(feature = "reference", feature = "prism"))]

use std::process::Command;

#[path = "corpus.rs"]
mod corpus;

use corpus::{P26_GATED as GATED, P26_SNIPPETS as SNIPPETS};

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
fn p26_corpus_is_byte_identical() {
    for (name, source) in SNIPPETS {
        check_identical(name, source);
    }
}

#[test]
fn p26_deferred_syntax_is_gated() {
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
