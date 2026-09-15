//! P1 certification: `verify` compares `compile --frontend prism` against the
//! pinned C golden, ignoring `DBG` bytes, in both strip modes. Exit `0` required.

#![cfg(all(feature = "reference", feature = "prism"))]

use std::process::Command;

#[path = "corpus.rs"]
mod corpus;

use corpus::P1_SNIPPETS as SNIPPETS;

fn carnelian() -> Command {
    Command::new(env!("CARGO_BIN_EXE_carnelian"))
}

/// Bytes with the `DBG` section removed (`verify` ignores debug info until
/// the pinned reference emits it).
fn stripped(bytes: &[u8]) -> Vec<u8> {
    carnelian_compiler::without_debug(bytes).expect("strip debug")
}

#[test]
fn p1_corpus_is_byte_identical() {
    for (name, source) in SNIPPETS {
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
}
