# carnelian-compiler

Ruby → RITE (mruby) bytecode compiler written in Rust, byte-identical to
the pinned reference (`mruby-compiler2 0.5.0` + Prism 1.9.0). License:
BSD 2-Clause (see `LICENSE`).

Try it in the browser: **https://dahrkael.github.io/carnelian-compiler/**

## Layout

- `crates/carnelian-ast` — owned AST types plus the node-access boundary.
- `crates/carnelian-compiler` — codegen backend and RITE writer (pure Rust).
- `crates/carnelian-front-prism` — Prism parser adapter (needs C, dev/CLI only).
- `crates/carnelian-front-mri` — pure-Rust parser frontend (`lib-ruby-parser`).
- `crates/carnelian-cli` — the `carnelian` binary (`compile`, `reference`, `verify`).
- `crates/carnelian-playground` — static web playground generator.

Compatibility pins live in `PINS.md`.

## Frontends

`carnelian compile --frontend <name>`, one of `prism`, `owned` or `mri`.

## Build and test

```sh
cargo build -p carnelian-cli
cargo test --workspace
cargo run -p carnelian-playground --bin playground -- --out dist/
```

### Feature builds

The CLI links C code by default. Both C parts are optional features:

```sh
# Everything (default): C reference golden + C parser frontends.
cargo build -p carnelian-cli
cargo test -p carnelian-cli

# Pure Rust only: no C anywhere, `compile --frontend mri` only.
cargo build -p carnelian-cli --no-default-features
cargo test -p carnelian-cli --no-default-features

# Reference golden without the C parser (certify `mri` against C).
cargo build -p carnelian-cli --no-default-features --features reference

# C parser frontends without the reference golden.
cargo build -p carnelian-cli --no-default-features --features prism
```
