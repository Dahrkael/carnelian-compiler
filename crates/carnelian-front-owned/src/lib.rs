//! Owned frontend (P3): FFI lowering to the owned AST plus the pure
//! `BackendNode` implementation used by the shipping path.
//!
//! `lower` needs the FFI tree, so it is dev/CLI only like
//! `carnelian-front-prism`; `Owned` and its `BackendNode` impl are pure
//! Rust and safe for `wasm32-unknown-unknown`.

pub mod lower;
pub mod owned;

pub use lower::lower;
pub use owned::Owned;
