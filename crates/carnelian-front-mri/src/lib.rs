//! Pure-Rust frontend over `lib-ruby-parser` (MRI grammar 3.1.2).
//!
//! End-to-end `compile` needs no C: parse here, lower to the owned AST,
//! resolve scopes and run the generic backend. Post-3.1.2 syntax fails at
//! parse time with a diagnostic naming the grammar ceiling.

pub mod lower;
pub mod parse;
pub mod scope;

pub use lower::lower;
pub use parse::{parse, ParseDiagnostic, Parsed};
pub use scope::resolve_scopes;

use carnelian_ast::{Node, Span};
use carnelian_compiler::{CompileOptions, Diagnostics};

/// Compile Ruby source to a RITE binary without leaving pure Rust:
/// parse, lower, resolve scopes, then the generic backend.
pub fn compile(source: &str, opts: &CompileOptions) -> Result<Vec<u8>, Diagnostics> {
    let parsed = parse(source.as_bytes());
    let errors = parsed.errors();
    if !errors.is_empty() {
        let mut diagnostics = Diagnostics::new();
        for error in &errors {
            diagnostics.push(error.message.clone(), error.start, error.end);
        }
        return Err(diagnostics);
    }
    // MRI has no program envelope; synthesize it so the backend sees the
    // same root shape as the Prism path (the scope pass fills `locals`).
    let (inner, mut pool) = match parsed.root() {
        Some(root) => lower(root),
        None => {
            let span = Span { start: 0, end: 0 };
            (
                Node::StatementsNode {
                    flags: 0,
                    span,
                    body: Vec::new(),
                },
                carnelian_ast::SymbolPool::new(),
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
    resolve_scopes(&mut node, &mut pool);
    let owned = carnelian_ast::Owned {
        node: &node,
        pool: &pool,
    };
    carnelian_compiler::compile_tree(owned, opts)
}
