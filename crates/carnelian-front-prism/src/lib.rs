//! Borrowed frontend over `ruby-prism`: the `PrismNode` adapter plus the
//! `lower` pass to the owned AST.
//!
//! Dev/CLI only: the shipping library never includes this crate. The backend
//! sees nodes through `BackendNode`, implemented here in the handlers slice.

use carnelian_ast::{AstNode, Span};

pub mod backend;
pub mod lower;

pub use lower::lower;

include!(concat!(env!("OUT_DIR"), "/kind_generated.rs"));

/// Borrowed Prism node (trivially reconstructed from raw parts, so handler
/// recursion passes it by value without cloning subtrees).
#[derive(Debug)]
pub struct PrismNode<'pr> {
    inner: ruby_prism::Node<'pr>,
}

impl<'pr> PrismNode<'pr> {
    /// Wrap a raw FFI node.
    pub const fn new(inner: ruby_prism::Node<'pr>) -> Self {
        Self { inner }
    }

    /// The wrapped FFI node.
    pub const fn inner(&self) -> &ruby_prism::Node<'pr> {
        &self.inner
    }
}

impl Clone for PrismNode<'_> {
    /// Borrowed-view copy: every variant carries the same by-value parts,
    /// so rebuilding from them copies the handle without touching the tree.
    fn clone(&self) -> Self {
        Self::new(clone_node(&self.inner))
    }
}

/// Parser error mapped to offsets (converted to backend diagnostics upstream).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDiagnostic {
    /// Message text.
    pub message: String,
    /// Start byte offset.
    pub start: u32,
    /// End byte offset.
    pub end: u32,
}

/// Parse result with mapped diagnostics.
pub struct Parsed<'pr> {
    result: ruby_prism::ParseResult<'pr>,
}

impl Parsed<'_> {
    /// Root node of the program.
    pub fn root(&self) -> PrismNode<'_> {
        PrismNode::new(self.result.node())
    }

    /// Top-level local names in order (`program->locals`).
    pub fn program_locals(&self) -> Vec<Vec<u8>> {
        let Some(program) = self.result.node().as_program_node() else {
            return Vec::new();
        };
        program
            .locals()
            .iter()
            .map(|id| id.as_slice().to_vec())
            .collect()
    }

    /// Parser errors (warnings do not fail compilation).
    pub fn errors(&self) -> Vec<ParseDiagnostic> {
        self.result
            .errors()
            .map(|diagnostic| ParseDiagnostic {
                message: diagnostic.message().to_owned(),
                start: offset(&diagnostic),
                end: offset(&diagnostic),
            })
            .collect()
    }
}

fn offset(diagnostic: &ruby_prism::Diagnostic<'_>) -> u32 {
    u32::try_from(diagnostic.location().start_offset()).unwrap_or(u32::MAX)
}

/// Parse Ruby source with the pinned Prism (`ruby-prism 1.9.0`).
pub fn parse(source: &[u8]) -> Parsed<'_> {
    Parsed {
        result: ruby_prism::parse(source),
    }
}

impl AstNode for PrismNode<'_> {
    fn kind_name(&self) -> &'static str {
        node_kind_name(&self.inner)
    }

    fn span(&self) -> Span {
        let location = self.inner.location();
        Span {
            start: u32::try_from(location.start_offset()).unwrap_or(u32::MAX),
            end: u32::try_from(location.end_offset()).unwrap_or(u32::MAX),
        }
    }

    fn flags(&self) -> u16 {
        node_flags(&self.inner)
    }
}
