//! Real-world corpus gate: `carnelian corpus --check` enforces
//! `corpus/baseline.tsv` on both frontends. Skips when `.corpus/` is absent
//! (gitignored; fetch with `tools/fetch_corpus.sh`). Needs the `reference`
//! feature (dev/CLI only, like the sibling parity files).

#![cfg(all(feature = "reference", feature = "prism"))]

use std::process::Command;

fn workspace_root() -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.parent()
        .and_then(|parent| parent.parent())
        .expect("cli crate sits in crates/<name>")
        .to_path_buf()
}

#[test]
fn real_world_corpus_matches_baseline() {
    let root = workspace_root();
    if !root.join(".corpus").is_dir() {
        eprintln!("skip: no .corpus/ (run tools/fetch_corpus.sh)");
        return;
    }
    let out = Command::new(env!("CARGO_BIN_EXE_carnelian"))
        .arg("corpus")
        .arg("--frontends")
        .arg("all")
        .arg("--check")
        .current_dir(&root)
        .output()
        .expect("run carnelian corpus --check");
    assert!(
        out.status.success(),
        "corpus drift:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
