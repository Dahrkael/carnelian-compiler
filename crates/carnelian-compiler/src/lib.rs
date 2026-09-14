//! `carnelian-compiler`: pure-Rust RITE backend.
//!
//! P0 covers the irep model plus the `dump.c` writer with a byte-exact
//! round-trip. Codegen tranches (P1/P2) build on the [`carnelian_ast::AstNode`]
//! boundary; this crate never sees C.

pub mod diagnostics;
pub mod irep;
pub mod reader;
pub mod writer;

pub use diagnostics::{Diagnostic, Diagnostics};
pub use irep::{CatchHandler, Irep, PoolValue, RiteModel};
pub use reader::{read_rite, ReadError};
pub use writer::{roundtrip, write_rite};

/// Compile options (stable API for P1 codegen).
#[derive(Debug, Clone, Default)]
pub struct CompileOptions {
    /// Omit the `DBG` section.
    pub stripped: bool,
    /// Source filename recorded in debug info.
    pub filename: Option<String>,
}

/// Compile Ruby source to a RITE binary.
///
/// P0 has no codegen yet and always returns an error directing callers to
/// `carnelian reference` for the pinned C golden. The signature is frozen so
/// P1 can fill it without breaking callers.
pub fn compile(_source: &str, _opts: &CompileOptions) -> Result<Vec<u8>, Diagnostics> {
    let mut diagnostics = Diagnostics::new();
    diagnostics.push(
        "codegen is not implemented in P0; use `carnelian reference`",
        0,
        0,
    );
    Err(diagnostics)
}
