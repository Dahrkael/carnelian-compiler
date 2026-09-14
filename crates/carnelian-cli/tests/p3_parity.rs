//! P3 parity (owned frontend): `reference` == `compile --frontend prism` ==
//! `compile --frontend owned` for every shared snippet, in both strip modes.
//! Gated cases must fail with the same diagnostic marker under both
//! frontends. Needs the P3.1 lowering and the P3.2 `BackendNode` impl; until
//! those land the owned comparisons fail.

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

fn check_parity(origin: &str, name: &str, source: &str) {
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
        assert!(
            first_divergence(&prism, &owned).is_none(),
            "{scoped} (strip={strip}): prism and owned diverge"
        );
    }
}

fn check_gated(origin: &str, name: &str, source: &str, marker: &str) {
    for frontend in ["prism", "owned"] {
        let dir = tempfile::tempdir().expect("tempdir");
        let input = dir.path().join(format!("{origin}_{name}.rb"));
        std::fs::write(&input, source).expect("write snippet");
        let out = dir.path().join(format!("{origin}_{name}.mrb"));
        let compiled = carnelian()
            .arg("compile")
            .arg(&input)
            .arg("-o")
            .arg(&out)
            .arg("--frontend")
            .arg(frontend)
            .output()
            .expect("run compile");
        assert_eq!(
            compiled.status.code(),
            Some(1),
            "{origin}_{name} ({frontend}): expected a gating diagnostic"
        );
        let stderr = String::from_utf8_lossy(&compiled.stderr);
        assert!(
            stderr.contains(marker),
            "{origin}_{name} ({frontend}): diagnostic lacks {marker:?}: {stderr}"
        );
    }
}

#[test]
fn p3_parity_is_identical() {
    for (name, source) in corpus::P1_SNIPPETS {
        check_parity("p1", name, source);
    }
    for (name, source) in corpus::P2_SNIPPETS {
        check_parity("p2", name, source);
    }
    for (name, source) in corpus::P23_SNIPPETS {
        check_parity("p23", name, source);
    }
    for (name, source) in corpus::P24_SNIPPETS {
        check_parity("p24", name, source);
    }
    for (name, source) in corpus::P25_SNIPPETS {
        check_parity("p25", name, source);
    }
    for (name, source) in corpus::P26_SNIPPETS {
        check_parity("p26", name, source);
    }
    // ROUNDTRIP_SNIPPETS stay out: they certify the writer, and several
    // hold syntax the codegen gates on both frontends (e.g. `i += 1`).
    for (name, source) in corpus::synthetic_cases() {
        check_parity("synthetic", name, &source);
    }
}

#[test]
fn p3_gated_agreement() {
    for (name, source, marker) in corpus::P2_GATED {
        check_gated("p2", name, source, marker);
    }
    for (name, source, marker) in corpus::P23_GATED {
        check_gated("p23", name, source, marker);
    }
    for (name, source, marker) in corpus::P24_GATED {
        check_gated("p24", name, source, marker);
    }
    for (name, source, marker) in corpus::P25_GATED {
        check_gated("p25", name, source, marker);
    }
    for (name, source, marker) in corpus::P26_GATED {
        check_gated("p26", name, source, marker);
    }
}

// Host-only fixture: a hand-built owned tree through `compile_prism::<Owned>`
// with no FFI involved, asserting the exact reference bytes. This is the
// shipping (`wasm32`) path exercised on the host.
const EXPECTED_EMPTY_MRB: &[u8] = &[
    0x52, 0x49, 0x54, 0x45, 0x30, 0x34, 0x30, 0x30, 0x00, 0x00, 0x00, 0x3e, 0x48, 0x53, 0x4d, 0x4b,
    0x30, 0x30, 0x30, 0x30, 0x49, 0x52, 0x45, 0x50, 0x00, 0x00, 0x00, 0x22, 0x30, 0x34, 0x30, 0x30,
    0x00, 0x00, 0x00, 0x16, 0x00, 0x01, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02,
    0x40, 0x76, 0x00, 0x00, 0x00, 0x00, 0x45, 0x4e, 0x44, 0x00, 0x00, 0x00, 0x00, 0x08,
];

const EXPECTED_PUTS1_MRB: &[u8] = &[
    0x52, 0x49, 0x54, 0x45, 0x30, 0x34, 0x30, 0x30, 0x00, 0x00, 0x00, 0x4c, 0x48, 0x53, 0x4d, 0x4b,
    0x30, 0x30, 0x30, 0x30, 0x49, 0x52, 0x45, 0x50, 0x00, 0x00, 0x00, 0x30, 0x30, 0x34, 0x30, 0x30,
    0x00, 0x00, 0x00, 0x24, 0x00, 0x01, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x09,
    0x07, 0x02, 0x2f, 0x01, 0x00, 0x01, 0x3d, 0x01, 0x76, 0x00, 0x00, 0x00, 0x01, 0x00, 0x04, 0x70,
    0x75, 0x74, 0x73, 0x00, 0x45, 0x4e, 0x44, 0x00, 0x00, 0x00, 0x00, 0x08,
];

fn compile_owned_tree(root: &carnelian_ast::Node, pool: &carnelian_ast::SymbolPool) -> Vec<u8> {
    let owned = carnelian_front_owned::Owned { node: root, pool };
    let opts = carnelian_compiler::CompileOptions {
        stripped: false,
        filename: None,
    };
    carnelian_compiler::compile_prism(owned, &opts).expect("owned fixture compiles")
}

#[test]
fn owned_fixture_empty_program_is_byte_identical() {
    use carnelian_ast::{Node, Span};
    let span = Span { start: 0, end: 0 };
    let body = Node::StatementsNode {
        flags: 0,
        span,
        body: Vec::new(),
    };
    let root = Node::ProgramNode {
        flags: 0,
        span,
        locals: Vec::new(),
        statements: Box::new(body),
    };
    let pool = carnelian_ast::SymbolPool::new();
    assert_eq!(compile_owned_tree(&root, &pool), EXPECTED_EMPTY_MRB);
}

#[test]
fn owned_fixture_puts_int_is_byte_identical() {
    use carnelian_ast::{Integer, Node, Span};
    let span = Span { start: 0, end: 0 };
    let mut pool = carnelian_ast::SymbolPool::new();
    let puts = pool.intern(b"puts");
    let argument = Node::IntegerNode {
        flags: 0,
        span,
        value: Integer::I64(1),
    };
    let arguments = Node::ArgumentsNode {
        flags: 0,
        span,
        arguments: vec![argument],
    };
    let call = Node::CallNode {
        flags: 0,
        span,
        receiver: None,
        call_operator_loc: None,
        name: puts,
        message_loc: None,
        opening_loc: None,
        arguments: Some(Box::new(arguments)),
        closing_loc: None,
        equal_loc: None,
        block: None,
    };
    let body = Node::StatementsNode {
        flags: 0,
        span,
        body: vec![call],
    };
    let root = Node::ProgramNode {
        flags: 0,
        span,
        locals: Vec::new(),
        statements: Box::new(body),
    };
    assert_eq!(compile_owned_tree(&root, &pool), EXPECTED_PUTS1_MRB);
}
