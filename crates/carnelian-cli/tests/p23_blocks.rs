//! P2.3 certification (blocks, lambdas, yield): `verify` compares
//! `compile --frontend prism` against the pinned C golden, byte for byte, in
//! both strip modes. Exit `0` required. Destructured, optional, keyword and
//! block parameters plus top-level `yield` stay gated as diagnostics, never
//! as diverging bytes.

use std::process::Command;

const SNIPPETS: &[(&str, &str)] = &[
    ("block_args", "puts [1, 2].map { |x| x + 1 }\n"),
    ("block_do", "[1].each do |x|\nputs x\nend\n"),
    ("block_noval", "[1].each { |x| puts x }\nputs 1\n"),
    ("block_noargs", "puts [1].map { 42 }\n"),
    ("block_empty", "puts [1].map { }\n"),
    ("block_empty_params", "[1].each { || puts 1 }\n"),
    ("block_multi", "[1, 2].each { |a, b| puts a }\n"),
    ("block_rest", "[1, 2].each { |*a| puts a }\n"),
    ("block_shadow", "x = 1\n[1].each { |x| puts x }\nputs x\n"),
    (
        "block_nested",
        "[1].each { |a| [2].each { |b| puts a + b } }\n",
    ),
    ("block_capture", "x = 10\n[1].each { puts x }\n"),
    ("block_write_capture", "x = 1\n[1].each { x = 2 }\nputs x\n"),
    ("lambda_basic", "f = -> { 1 }\nputs f.call\n"),
    ("lambda_args", "f = ->(x) { x + 1 }\nputs f.call(2)\n"),
    ("lambda_rest", "f = ->(*a) { puts a }\nf.call(1, 2)\n"),
    ("sym_proc", "puts [1, 2].map(&:to_s)\n"),
    ("numbered", "puts [1, 2].map { _1 + 1 }\n"),
    ("it_block", "puts [1, 2].map { it + 1 }\n"),
    ("semi_local", "[1].each { |x; y| y = x + 1\nputs y }\n"),
];

/// Still-gated forms: compilation must fail with a diagnostic, and must
/// never emit bytes that diverge from the reference.
const GATED: &[(&str, &str, &str)] = &[
    ("yield_naked", "yield\n", "Invalid yield"),
    ("yield_args", "yield 1\n", "Invalid yield"),
    (
        "block_destructure_gated",
        "[1].each { |(a, b)| puts a }\n",
        "block destructuring",
    ),
    (
        "block_optional_gated",
        "[1].each { |x = 1| puts x }\n",
        "block optional",
    ),
    (
        "block_keyword_gated",
        "[1].each { |x: 1| puts x }\n",
        "block keyword",
    ),
    (
        "block_param_gated",
        "[1].each { |&b| puts b }\n",
        "block block parameter",
    ),
];

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
fn p23_corpus_is_byte_identical() {
    for (name, source) in SNIPPETS {
        check_identical(name, source);
    }
}

#[test]
fn p23_deferred_syntax_is_gated() {
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
