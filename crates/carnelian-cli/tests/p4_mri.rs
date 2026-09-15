//! P4 parity (MRI frontend): `reference` == `compile --frontend prism` ==
//! `compile --frontend owned` == `compile --frontend mri` for the shared
//! corpus, in both strip modes. Needs sibling A (lower), B (scopes) and the
//! end-to-end `compile`; until those land every `mri` comparison fails.
//!
//! Grammar-ceiling gates (`it`, anonymous 3.2 forwarding) fail under `mri`
//! with a parse diagnostic while prism/owned keep passing, so they carry
//! per-frontend expectations, not shared markers. `for` lives in the
//! shared corpus now that the backend covers all frontends.
//!
//! Wasm: `cargo check -p carnelian-ast -p carnelian-compiler
//! -p carnelian-front-mri --target wasm32-unknown-unknown` must pass
//! (front-mri is pure Rust); the host smoke below runs the same
//! `front_mri::compile` on the host and pins the `puts 1` bytes.

#[path = "corpus.rs"]
mod corpus;

use std::process::Command;

fn carnelian() -> Command {
    Command::new(env!("CARGO_BIN_EXE_carnelian"))
}

fn first_divergence(a: &[u8], b: &[u8]) -> Option<usize> {
    for (index, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        if x != y {
            return Some(index);
        }
    }
    if a.len() != b.len() {
        return Some(a.len().min(b.len()));
    }
    None
}

fn reference_bytes(name: &str, source: &str, dir: &std::path::Path) -> Vec<u8> {
    let input = dir.join(format!("{name}.rb"));
    std::fs::write(&input, source).expect("write snippet");
    let golden = dir.join(format!("{name}.mrb"));
    let reference = carnelian()
        .arg("reference")
        .arg(&input)
        .arg("-o")
        .arg(&golden)
        .output()
        .expect("run reference");
    assert!(
        reference.status.success(),
        "{name}: reference failed: {}",
        String::from_utf8_lossy(&reference.stderr)
    );
    std::fs::read(&golden).expect("read golden")
}

fn compile_bytes(
    name: &str,
    source: &str,
    dir: &std::path::Path,
    strip: bool,
    frontend: &str,
) -> Vec<u8> {
    let input = dir.join(format!("{name}.rb"));
    std::fs::write(&input, source).expect("write snippet");
    let out = dir.join(format!("{name}.{frontend}.{strip}.mrb"));
    let mut command = carnelian();
    command
        .arg("compile")
        .arg(&input)
        .arg("-o")
        .arg(&out)
        .arg("--frontend")
        .arg(frontend);
    if strip {
        command.arg("--strip");
    }
    let compiled = command.output().expect("run compile");
    assert_eq!(
        compiled.status.code(),
        Some(0),
        "{name} ({frontend}, strip={strip}): compile failed: {}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    std::fs::read(&out).expect("read output")
}

fn compile_output(
    name: &str,
    source: &str,
    dir: &std::path::Path,
    frontend: &str,
) -> std::process::Output {
    let input = dir.join(format!("{name}.rb"));
    std::fs::write(&input, source).expect("write snippet");
    let out = dir.join(format!("{name}.{frontend}.mrb"));
    carnelian()
        .arg("compile")
        .arg(&input)
        .arg("-o")
        .arg(&out)
        .arg("--frontend")
        .arg(frontend)
        .output()
        .expect("run compile")
}

/// Corpus pairs the MRI frontend cannot cover with byte identity:
/// `it` is past the 3.1.2 grammar ceiling (gated at parse per the P4
/// contract; the old grammar would read it as a plain send).
fn mri_excluded(origin: &str, name: &str) -> bool {
    matches!((origin, name), ("p23", "it_block"))
}

fn check_parity_4way(origin: &str, name: &str, source: &str) {
    let scoped = format!("{origin}_{name}");
    let dir = tempfile::tempdir().expect("tempdir");
    let golden = reference_bytes(&scoped, source, dir.path());
    for strip in [false, true] {
        let prism = compile_bytes(&scoped, source, dir.path(), strip, "prism");
        assert!(
            first_divergence(&golden, &prism).is_none(),
            "{scoped} (prism, strip={strip}): bytes diverge"
        );
        let owned = compile_bytes(&scoped, source, dir.path(), strip, "owned");
        assert!(
            first_divergence(&golden, &owned).is_none(),
            "{scoped} (owned, strip={strip}): bytes diverge"
        );
        let mri = compile_bytes(&scoped, source, dir.path(), strip, "mri");
        assert!(
            first_divergence(&golden, &mri).is_none(),
            "{scoped} (mri, strip={strip}): bytes diverge"
        );
        assert!(
            first_divergence(&prism, &mri).is_none() && first_divergence(&owned, &mri).is_none(),
            "{scoped} (strip={strip}): frontends diverge"
        );
    }
}

/// Markers that legitimately differ under `mri`: same exit 1, different
/// locus (Prism rejects at parse, MRI reaches the backend gate, or vice
/// versa). Bare `yield` is the shape: Prism errors `Invalid yield` while
/// parsing, MRI parses it and the shared backend reports `invalid yield`.
fn mri_marker<'a>(origin: &str, name: &str, marker: &'a str) -> &'a str {
    match (origin, name) {
        ("p23", "yield_naked") | ("p23", "yield_args") => "invalid yield",
        _ => marker,
    }
}

fn check_gated_3way(origin: &str, name: &str, source: &str, marker: &str) {
    for frontend in ["prism", "owned", "mri"] {
        let expected = if frontend == "mri" {
            mri_marker(origin, name, marker)
        } else {
            marker
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let compiled = compile_output(&format!("{origin}_{name}"), source, dir.path(), frontend);
        assert_eq!(
            compiled.status.code(),
            Some(1),
            "{origin}_{name} ({frontend}): expected a gating diagnostic"
        );
        let stderr = String::from_utf8_lossy(&compiled.stderr);
        assert!(
            stderr.contains(expected),
            "{origin}_{name} ({frontend}): diagnostic lacks {expected:?}: {stderr}"
        );
    }
}

/// Post-3.1.2 syntax: `mri` fails at parse (exit 1 with a diagnostic)
/// while prism/owned keep passing. `bar(*)`/`bar(**)` anonymous forwarding
/// is Ruby 3.2 (the pinned 3.1.2 grammar rejects it); `it` is Ruby 3.4.
const MRI_CEILING: &[(&str, &str)] = &[
    ("it_param", "puts [1, 2].map { it + 1 }\n"),
    ("it_param_in_def", "def f\n  [1].each { it }\nend\n"),
    (
        "anon_rest_forward",
        "def foo(*)\n  bar(*)\nend\nputs foo(1, 2)\n",
    ),
    (
        "anon_kwrest_forward",
        "def foo(**)\n  bar(**)\nend\nputs foo(a: 1)\n",
    ),
];

/// Scope vectors for the upvar pass: nested capture depths, shadowing at
/// each level, sibling blocks, block-locals, deep writes and a def
/// boundary. All pass under prism/owned/reference today.
const SCOPE_SNIPPETS: &[(&str, &str)] = &[
    (
        "nested_3deep",
        "x = 1\n[1].each { [2].each { [3].each { puts x } } }\n",
    ),
    (
        "shadow_middle",
        "x = 1\n[1].each { |x| [2].each { puts x } }\nputs x\n",
    ),
    (
        "sibling_blocks",
        "x = 1\n[1].each { puts x }\n[2].each { puts x + 1 }\n",
    ),
    (
        "block_local_shadow",
        "x = 99\n[1].each { |a; x| x = a + 1\nputs x }\nputs x\n",
    ),
    (
        "write_capture_deep",
        "x = 1\n[1].each { [2].each { x = 3 } }\nputs x\n",
    ),
    (
        "def_param_capture",
        "x = 1\ndef foo(x)\n  [1].each { puts x }\nend\nputs foo(2)\n",
    ),
];

/// `for` loops live in the shared corpus (`P25_SNIPPETS`) now that the
/// backend `gen_for` covers all frontends (4-way via the P25 table).

#[test]
fn p4_parity_is_identical() {
    // 315 table snippets minus the `it` ceiling exclusion, plus the 7
    // synthetic limit-path sources: 321 snippets x 2 modes x 4-way.
    // ROUNDTRIP_SNIPPETS stay out (writer cert; gated `i += 1`), like p3.
    for (origin, table) in [
        ("p1", corpus::P1_SNIPPETS),
        ("p2", corpus::P2_SNIPPETS),
        ("p23", corpus::P23_SNIPPETS),
        ("p24", corpus::P24_SNIPPETS),
        ("p25", corpus::P25_SNIPPETS),
        ("p26", corpus::P26_SNIPPETS),
    ] {
        for (name, source) in table {
            if mri_excluded(origin, name) {
                continue;
            }
            check_parity_4way(origin, name, source);
        }
    }
    for (name, source) in corpus::synthetic_cases() {
        check_parity_4way("synthetic", name, &source);
    }
}

#[test]
fn p4_gated_agreement() {
    // Backend gates fail with the same marker under all three frontends
    // (`for` lives in the shared corpus since the backend covers it).
    for (origin, table) in [
        ("p2", corpus::P2_GATED),
        ("p23", corpus::P23_GATED),
        ("p24", corpus::P24_GATED),
        ("p25", corpus::P25_GATED),
        ("p26", corpus::P26_GATED),
    ] {
        for (name, source, marker) in table {
            check_gated_3way(origin, name, source, marker);
        }
    }
}

#[test]
fn mri_grammar_ceiling_gates() {
    for (name, source) in MRI_CEILING {
        let dir = tempfile::tempdir().expect("tempdir");
        let mri = compile_output(&format!("ceiling_{name}"), source, dir.path(), "mri");
        assert_eq!(
            mri.status.code(),
            Some(1),
            "ceiling_{name} (mri): expected a parse diagnostic"
        );
        let stderr = String::from_utf8_lossy(&mri.stderr);
        assert!(
            stderr.contains("error"),
            "ceiling_{name} (mri): diagnostic lacks an error line: {stderr}"
        );
        for frontend in ["prism", "owned"] {
            let dir = tempfile::tempdir().expect("tempdir");
            let passed = compile_output(&format!("ceiling_{name}"), source, dir.path(), frontend);
            assert_eq!(
                passed.status.code(),
                Some(0),
                "ceiling_{name} ({frontend}): must keep passing: {}",
                String::from_utf8_lossy(&passed.stderr)
            );
        }
    }
}

#[test]
fn mri_scope_vectors() {
    for (name, source) in SCOPE_SNIPPETS {
        check_parity_4way("scope", name, source);
    }
}

// Host smoke for the shipping path: `front_mri::compile` on `puts 1`
// must emit the pinned golden (same bytes as the p3 `puts 1` fixture).
const EXPECTED_PUTS1_MRB: &[u8] = &[
    0x52, 0x49, 0x54, 0x45, 0x30, 0x34, 0x30, 0x30, 0x00, 0x00, 0x00, 0x4c, 0x48, 0x53, 0x4d, 0x4b,
    0x30, 0x30, 0x30, 0x30, 0x49, 0x52, 0x45, 0x50, 0x00, 0x00, 0x00, 0x30, 0x30, 0x34, 0x30, 0x30,
    0x00, 0x00, 0x00, 0x24, 0x00, 0x01, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x09,
    0x07, 0x02, 0x2f, 0x01, 0x00, 0x01, 0x3d, 0x01, 0x76, 0x00, 0x00, 0x00, 0x01, 0x00, 0x04, 0x70,
    0x75, 0x74, 0x73, 0x00, 0x45, 0x4e, 0x44, 0x00, 0x00, 0x00, 0x00, 0x08,
];

#[test]
fn mri_host_smoke_puts1_matches_golden() {
    let opts = carnelian_compiler::CompileOptions {
        stripped: false,
        filename: None,
    };
    let bytes = carnelian_front_mri::compile("puts 1\n", &opts).expect("mri smoke compiles");
    assert_eq!(bytes, EXPECTED_PUTS1_MRB);
}
