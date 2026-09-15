//! MRI parsing wrapper (P4): source bytes to diagnostics plus tree.
//!
//! Syntax past the 3.1.2 grammar ceiling fails here with a clear
//! diagnostic; the backend never sees it.

/// One parse diagnostic with byte offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDiagnostic {
    /// Message text.
    pub message: String,
    /// Start byte offset.
    pub start: u32,
    /// End byte offset.
    pub end: u32,
}

/// Parsed program: error list plus the MRI tree on success.
pub struct Parsed {
    _private: (),
}

impl Parsed {
    /// Parser errors (warnings do not fail compilation).
    pub fn errors(&self) -> Vec<ParseDiagnostic> {
        todo!("P4: map MRI diagnostics")
    }

    /// Root of the MRI tree (valid only when `errors()` is empty).
    pub fn root(&self) -> &lib_ruby_parser::Node {
        todo!("P4: return the MRI root")
    }
}

/// Parse Ruby source with the pinned `lib-ruby-parser`.
pub fn parse(_source: &[u8]) -> Parsed {
    todo!("P4: run the MRI parser")
}
