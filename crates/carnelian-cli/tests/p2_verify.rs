//! P2 certification (literals tranche): `verify` compares
//! `compile --frontend prism` against the pinned C golden, byte for byte, in
//! both strip modes. Exit `0` required. Pattern matching and interpolated
//! symbols belong to later tranches and are locked as diagnostics below,
//! never as diverging bytes. Keyword hashes in call arguments opened in P2.5.

use std::process::Command;

const SNIPPETS: &[(&str, &str)] = &[
    ("hash_sym_rocket", "x = {a: 1, \"b\" => 2}\nputs x\n"),
    ("hash_empty", "x = {}\nputs x\n"),
    ("hash_empty_call", "puts({})\n"),
    ("hash_nested", "x = {a: {b: 1}}\nputs x\n"),
    ("hash_splat", "y = {b: 2}\nx = {a: 1, **y}\nputs x\n"),
    ("hash_splat_middle", "x = {a: 1, **nil, b: 2}\nputs x\n"),
    ("hash_splat_first", "x = {**nil, a: 1}\nputs x\n"),
    ("hash_multi_splat", "x = {**nil, a: 1, **nil}\nputs x\n"),
    ("hash_noval_splat", "{a: 1, **nil}\nputs 1\n"),
    ("hash_in_call", "puts({a: 1})\n"),
    ("hash_mixed_keys", "x = {1 => \"a\", :s => 2}\nputs x\n"),
    ("hash_noval", "{a: 1}\nputs 2\n"),
    (
        "case_basic",
        "x = 2\ncase x\nwhen 1 then puts 1\nwhen 2 then puts 2\nelse puts 3\nend\n",
    ),
    (
        "case_no_else",
        "x = 1\ncase x\nwhen 1 then puts 1\nend\nputs 2\n",
    ),
    (
        "case_empty_when",
        "x = 1\ncase x\nwhen 1 then\nelse puts 2\nend\nputs 3\n",
    ),
    (
        "case_no_predicate",
        "x = 1\ncase\nwhen x == 1 then puts 1\nelse puts 2\nend\n",
    ),
    (
        "case_valued",
        "x = 2\ny = case x\nwhen 1 then 10\nwhen 2 then 20\nelse 30\nend\nputs y\n",
    ),
    (
        "case_valued_empty",
        "x = 2\ny = case x\nwhen 1 then\nwhen 2 then 20\nend\nputs y\n",
    ),
    (
        "case_multi_cond",
        "x = 1\ncase x\nwhen 1, 2 then puts \"low\"\nelse puts \"high\"\nend\n",
    ),
    (
        "case_splat_when",
        "a = [1, 2]\nx = 1\ncase x\nwhen *a then puts 1\nelse puts 2\nend\n",
    ),
    (
        "case_in_call",
        "puts(case 1\nwhen 1 then \"one\"\nelse \"other\"\nend)\n",
    ),
    ("interp_basic", "name = \"bob\"\nputs \"hi #{name}\"\n"),
    ("interp_leading", "puts \"#{1} hi\"\n"),
    ("interp_only", "x = 1\nputs \"#{x}\"\n"),
    ("interp_nested", "x = \"a\"\nputs \"o#{\"i#{x}\"}e\"\n"),
    ("interp_noval", "\"hi #{1 + 2}\"\nputs 1\n"),
    ("interp_empty_embexpr", "puts \"#{}\"\n"),
    ("interp_noval_empty", "\"#{}\"\nputs 1\n"),
    ("interp_multi", "x = 1\ny = \"a\"\nputs \"a#{x}b#{y}c\"\n"),
    ("interp_side_effect", "\"hi #{puts 1}\"\nputs 2\n"),
];

/// Later-tranche syntax: compilation must fail with a diagnostic, and must
/// never emit bytes that diverge from the reference.
const GATED: &[(&str, &str, &str)] = &[
    (
        "case_match_gated",
        "x = 1\ncase x\nin 1 then puts 1\nend\n",
        "CaseMatchNode",
    ),
    (
        "interp_symbol_gated",
        "x = 1\nputs :\"s#{x}\"\n",
        "InterpolatedSymbolNode",
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
fn p2_corpus_is_byte_identical() {
    for (name, source) in SNIPPETS {
        check_identical(name, source);
    }
}

#[test]
fn p2_big_hash_hits_limit_paths() {
    // 70 pairs cross the stack flush threshold (99) mid-way, exercising the
    // mid-construction `HASH` flush and the trailing `HASHADD` paths.
    let pairs: Vec<String> = (0..70).map(|index| format!("{index} => {index}")).collect();
    let source = format!("x = {{{}}}\nputs x\n", pairs.join(", "));
    check_identical("hash_big", &source);
}

#[test]
fn p2_wide_hash_hits_array_packing() {
    // 63 locals push the cursor to 64, lifting the flush threshold past
    // `INT16_MAX`, so the 70 pairs reach the trailing `len > limit` packing
    // (`HASH`, `-1`) instead of flushing mid-way.
    let mut source = String::new();
    for index in 0..63 {
        source.push_str(&format!("v{index} = {index}\n"));
    }
    let pairs: Vec<String> = (0..70).map(|index| format!("{index} => {index}")).collect();
    source.push_str(&format!("x = {{{}}}\nputs x\n", pairs.join(", ")));
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
