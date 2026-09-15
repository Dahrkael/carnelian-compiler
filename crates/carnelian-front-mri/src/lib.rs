//! Pure-Rust frontend over `lib-ruby-parser` (MRI grammar 3.1.2).
//!
//! End-to-end `compile` needs no C: parse here, resolve scopes, lower to
//! the owned AST and run the generic backend. Post-3.1.2 syntax fails at
//! parse time with a diagnostic naming the grammar ceiling.

pub mod lower;
pub mod parse;
pub mod scope;

pub use lower::lower;
pub use parse::{parse, ParseDiagnostic, Parsed};
pub use scope::resolve_scopes;

use carnelian_compiler::{CompileOptions, Diagnostics};

/// Compile Ruby source to a RITE binary without leaving pure Rust.
pub fn compile(source: &str, opts: &CompileOptions) -> Result<Vec<u8>, Diagnostics> {
    let _ = (source, opts);
    todo!("P4: parse, resolve scopes, lower, compile_tree")
}
