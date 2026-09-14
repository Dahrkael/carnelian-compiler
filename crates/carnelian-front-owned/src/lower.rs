//! FFI lowering: borrowed `ruby-prism` tree to the owned AST (P3.1).
//!
//! Dev/CLI only. Maps all 151 node kinds 1:1: `node?` becomes
//! `Option<Box<Node>>`, `node[]` becomes `Vec<Node>`, `constant` fields
//! intern into the returned `SymbolPool`, `location` fields become offsets,
//! `integer` fields become the `Integer` model and flags stay raw `u16`.

use carnelian_ast::{Node, SymbolPool};
use carnelian_front_prism::PrismNode;

/// Lower a parsed FFI root to its owned tree plus symbol pool.
pub fn lower(_root: PrismNode<'_>) -> (Node, SymbolPool) {
    todo!("P3.1: lower the FFI tree")
}
