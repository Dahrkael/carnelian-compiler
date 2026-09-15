//! Irep model: pure-Rust port of `mrc_irep.h` + `mrc_irep_pool_type.h`.
//!
//! Layout and encoding rules mirror `dump.c`; see `writer.rs`.
//! No hash maps: pool/syms order is insertion order (`Vec`).

/// Pool value types (`IREP_TT_*` in `mrc_irep_pool_type.h`).
#[derive(Debug, Clone, PartialEq)]
pub enum PoolValue {
    /// `IREP_TT_STR = 0`: owned string bytes (without NUL).
    Str(Vec<u8>),
    /// `IREP_TT_INT32 = 1`.
    Int32(i32),
    /// `IREP_TT_SSTR = 2`: static string bytes (without NUL, preserved).
    SStr(Vec<u8>),
    /// `IREP_TT_INT64 = 3`.
    Int64(i64),
    /// `IREP_TT_FLOAT = 5`.
    Float(f64),
    /// `IREP_TT_BIGINT = 7`: raw bytes after the type tag, length byte
    /// included (`dump.c` stores `str[0]` as an unsigned length and emits
    /// `str[0] + 2` bytes verbatim).
    BigInt(Vec<u8>),
}

/// Catch handler (`mrc_irep_catch_handler`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatchHandler {
    /// `MRC_CATCH_RESCUE = 0`, `MRC_CATCH_ENSURE = 1`.
    pub kind: u8,
    /// Start address (inclusive).
    pub begin: u32,
    /// End address (exclusive).
    pub end: u32,
    /// Jump target address.
    pub target: u32,
}

/// One debug file entry (`mrc_irep_debug_info_file`, packed-map form only).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DebugFile {
    /// First pc covered (`start_pos`).
    pub start_pos: u32,
    /// Filename bytes (no NUL).
    pub filename: Vec<u8>,
    /// Per-pc lines for `[start_pos, start_pos + lines.len())`.
    pub lines: Vec<u16>,
}

/// Per-irep debug info (`mrc_irep_debug_info`); `None` on `Irep` means the
/// scope carried none (like a `NULL debug_info` in C).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DebugInfo {
    /// File entries in order.
    pub files: Vec<DebugFile>,
}

/// One irep record with its child ireps.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Irep {
    /// Number of local variables.
    pub nlocals: u16,
    /// Number of register variables.
    pub nregs: u16,
    /// Instruction bytes.
    pub iseq: Vec<u8>,
    /// Catch handlers (in order).
    pub catch_handlers: Vec<CatchHandler>,
    /// Constant pool (in order).
    pub pool: Vec<PoolValue>,
    /// Symbols (in order); `None` is the null symbol (`0xFFFF`).
    pub syms: Vec<Option<Vec<u8>>>,
    /// Child ireps (in order).
    pub reps: Vec<Irep>,
    /// Local names as LVAR symbol-table indices (`None` = `0xFFFF`).
    /// Empty when the binary carries no `LVAR` section.
    pub lv: Vec<Option<u32>>,
    /// Structured debug info; encoded to `DBG` by the writer. `None` emits
    /// no record (the reader never fills this: it preserves `debug_raw`).
    pub debug: Option<DebugInfo>,
}

/// Full RITE image for the writer.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RiteModel {
    /// Root irep.
    pub root: Irep,
    /// `LVAR` symbol table (raw names, no NUL). `None` = no section.
    pub lvar_syms: Option<Vec<Vec<u8>>>,
    /// Raw `DBG` section bytes (including its 8-byte header), preserved
    /// verbatim for byte-exact round-trips. `None` = no section.
    pub debug_raw: Option<Vec<u8>>,
}
