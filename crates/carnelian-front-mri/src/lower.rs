//! MRI lowering: `lib-ruby-parser` tree to the owned AST (P4-A).
//!
//! Covers the 3.1.2 grammar 1:1. Scope fields stay blank here (`depth`
//! zero, `locals` empty); the scope pass fills them afterwards.

use carnelian_ast::{Node, SymbolPool};

/// Lower an MRI tree to its owned tree plus symbol pool.
pub fn lower(_root: &lib_ruby_parser::Node) -> (Node, SymbolPool) {
    todo!("P4-A: lower the MRI tree")
}
