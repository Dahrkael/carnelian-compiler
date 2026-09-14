//! Pure owned access: `BackendNode` over borrowed owned trees (P3.2).
//!
//! Shipping-safe (no FFI, no `unsafe`). Children are plain reborrows, so
//! the wrapper stays `Clone`/`Copy` like any other cheap node handle.

use carnelian_ast::view::BackendNode;
use carnelian_ast::{AstNode, Node, SymbolPool};

/// Borrowed owned tree plus its symbol pool.
#[derive(Debug, Clone, Copy)]
pub struct Owned<'a> {
    /// Current node.
    pub node: &'a Node,
    /// Pool backing every `SymbolId` in the tree.
    pub pool: &'a SymbolPool,
}

impl AstNode for Owned<'_> {
    fn kind_name(&self) -> &'static str {
        self.node.kind_name()
    }

    fn span(&self) -> carnelian_ast::Span {
        self.node.span()
    }

    fn flags(&self) -> u16 {
        self.node.flags()
    }
}

impl BackendNode for Owned<'_> {
    // P3.2: mirror `carnelian-front-prism/src/backend.rs` accessor by
    // accessor (86 overrides); children reborrow `node`/`pool`.
}
