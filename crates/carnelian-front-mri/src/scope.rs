//! Scope/upvar pass (P4-B): locals ordered per scope, `depth` per local
//! use, block-locals and numbered/`it` markers.
//!
//! Runs on the owned tree after lowering. `for_depth` stays zero here;
//! the `for` body scope is backend business (P4-D).

use carnelian_ast::{Node, SymbolPool};

/// Fill scope data in place: `locals` on every scope node, `depth` on
/// every local read/write/target.
pub fn resolve_scopes(_node: &mut Node, _pool: &mut SymbolPool) {
    todo!("P4-B: resolve scopes and upvars")
}
