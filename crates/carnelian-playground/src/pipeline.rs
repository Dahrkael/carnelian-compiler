//! Parse/lower/compile helpers shared by generator, tests and wasm.
//!
//! Mirrors `front_mri::compile` staging so the AST tab shows the exact
//! tree the backend lowers.

use carnelian_ast::{Node, Span, SymbolPool};
use carnelian_compiler::{read_rite, CompileOptions, PoolValue, RiteModel};

/// One diagnostic with UTF-8 byte offsets into the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diag {
    /// Message text.
    pub message: String,
    /// Start byte offset.
    pub start: u32,
    /// End byte offset (exclusive).
    pub end: u32,
}

/// Parse and lower to the owned tree the backend compiles.
pub fn parse_and_lower(source: &str) -> Result<(Node, SymbolPool), Vec<Diag>> {
    let parsed = carnelian_front_mri::parse(source.as_bytes());
    let errors = parsed.errors();
    if !errors.is_empty() {
        return Err(errors
            .iter()
            .map(|error| Diag {
                message: error.message.clone(),
                start: error.start,
                end: error.end,
            })
            .collect());
    }
    let (inner, mut pool) = match parsed.root() {
        Some(root) => carnelian_front_mri::lower(root),
        None => {
            let span = Span { start: 0, end: 0 };
            (
                Node::StatementsNode {
                    flags: 0,
                    span,
                    body: Vec::new(),
                },
                SymbolPool::new(),
            )
        }
    };
    let span = inner.span();
    let statements = match inner {
        Node::StatementsNode { .. } => inner,
        other => Node::StatementsNode {
            flags: 0,
            span,
            body: vec![other],
        },
    };
    let mut node = Node::ProgramNode {
        flags: 0,
        span,
        locals: Vec::new(),
        statements: Box::new(statements),
    };
    carnelian_front_mri::resolve_scopes(&mut node, &mut pool);
    Ok((node, pool))
}

/// Compile to RITE bytes on the pure-Rust MRI path.
pub fn compile_source(source: &str) -> Result<Vec<u8>, Vec<Diag>> {
    let opts = CompileOptions::default();
    carnelian_front_mri::compile(source, &opts).map_err(|diagnostics| {
        diagnostics
            .entries
            .iter()
            .map(|entry| Diag {
                message: entry.message.clone(),
                start: entry.start,
                end: entry.end,
            })
            .collect()
    })
}

/// True when any pool entry is a bigint (loader pool type 7).
pub fn has_bigint(model: &RiteModel) -> bool {
    let mut found = false;
    check_irep(&model.root, &mut found);
    found
}

fn check_irep(irep: &carnelian_compiler::Irep, found: &mut bool) {
    for value in &irep.pool {
        if matches!(value, PoolValue::BigInt(_)) {
            *found = true;
            return;
        }
    }
    for child in &irep.reps {
        check_irep(child, found);
    }
}

/// Fail-closed bigint gate over compiled bytes.
pub fn gate_bigint(bytes: &[u8]) -> Result<(), String> {
    let model = read_rite(bytes).map_err(|error| format!("unreadable RITE: {error:?}"))?;
    if has_bigint(&model) {
        return Err("bigint literal gated: pinned mrubyedge misreads pool type 7 (u16 length) and panics; execution refused".to_string());
    }
    Ok(())
}
