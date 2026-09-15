//! P2.5 certification (rescue/ensure, splat, kwargs, masgn): `verify` compares
//! `compile --frontend prism` against the pinned C golden, ignoring `DBG`
//! bytes, in both strip modes. Exit `0` required. Pattern matching and the still-gated
//! assignment targets are locked as diagnostics below, never as diverging
//! bytes.

#![cfg(all(feature = "reference", feature = "prism"))]

use std::process::Command;

#[path = "corpus.rs"]
mod corpus;

use corpus::{P25_GATED as GATED, P25_SNIPPETS as SNIPPETS};

fn carnelian() -> Command {
    Command::new(env!("CARGO_BIN_EXE_carnelian"))
}

/// Bytes with the `DBG` section removed (`verify` ignores debug info until
/// the pinned reference emits it).
fn stripped(bytes: &[u8]) -> Vec<u8> {
    carnelian_compiler::without_debug(bytes).expect("strip debug")
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

    // `compile` output must match the golden file ignoring `DBG` bytes.
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
            stripped(&std::fs::read(&golden).expect("read golden")),
            stripped(&std::fs::read(&out).expect("read output")),
            "{name} (strip={strip}): bytes diverge"
        );
    }
}

#[test]
fn p25_corpus_is_byte_identical() {
    for (name, source) in SNIPPETS {
        check_identical(name, source);
    }
}

#[test]
fn p25_splat_flush_paths() {
    // A splat after a long positional run flushes the pending values into an
    // array (`ARRAY`), then the splat concatenates (`ARYCAT`).
    let source = corpus::synthetic_source("splat_flush_positional");
    check_identical("splat_flush_positional", &source);

    // A splat followed by more values flushes, then appends them
    // (`ARYPUSH`).
    let source = corpus::synthetic_source("splat_flush_trailing");
    check_identical("splat_flush_trailing", &source);

    // A splat after a long run flushes and concatenates a wide array.
    let source = corpus::synthetic_source("splat_flush_wide");
    check_identical("splat_flush_wide", &source);

    // Crossing `GEN_VAL_STACK_MAX` (99) flushes plain values mid-list.
    let source = corpus::synthetic_source("values_stack_limit_flush");
    check_identical("values_stack_limit_flush", &source);

    // More keywords than the small-hash limit pack into a variable hash.
    let source = corpus::synthetic_source("kwargs_limit_flush");
    check_identical("kwargs_limit_flush", &source);
}

#[test]
fn p25_deferred_syntax_is_gated() {
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
