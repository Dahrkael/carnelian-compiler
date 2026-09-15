//! Static web playground generator (native binary).
//!
//! `cargo run -p playground [-- --out dist/]` emits a self-contained,
//! offline `dist/` directory: script editor, AST viewer, bytecode viewer
//! and execution console. Ruby compiles through the pure-Rust MRI
//! frontend and executes on the pinned mrubyedge fork; bigints stay
//! gated (fail-closed) until the loader handles pool type 7.

fn main() {
    todo!("playground generator")
}
