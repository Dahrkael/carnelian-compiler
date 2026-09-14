//! Diagnostics with source offsets (equivalent to `mrc_diagnostic_list`).

/// One diagnostic (error or warning).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Message text.
    pub message: String,
    /// Start byte offset in the source.
    pub start: u32,
    /// End byte offset (exclusive) in the source.
    pub end: u32,
}

/// Compilation diagnostics (errors and warnings).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diagnostics {
    /// All entries in emission order.
    pub entries: Vec<Diagnostic>,
}

impl Diagnostics {
    /// Empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Push one entry.
    pub fn push(&mut self, message: impl Into<String>, start: u32, end: u32) {
        self.entries.push(Diagnostic {
            message: message.into(),
            start,
            end,
        });
    }

    /// True when there are no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

impl core::fmt::Display for Diagnostics {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for entry in &self.entries {
            writeln!(f, "{}:{}: {}", entry.start, entry.end, entry.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for Diagnostics {}
