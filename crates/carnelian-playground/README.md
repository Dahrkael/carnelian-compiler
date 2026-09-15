# carnelian-playground

Offline static web playground: compiles Ruby (pure-Rust MRI frontend,
grammar ceiling 3.1.2) and executes it on the pinned mrubyedge fork, all
inside one self-contained `dist/` with no CDN, no fetch, vanilla JS.

## What it generates

`cargo run -p carnelian-playground --bin playground -- --out dist/` emits:

- `index.html` — editor + AST + bytecode + console tabs, pins footer
  parsed from `PINS.md`.
- `app.js` — vanilla JS (no deps); curated examples inlined, wasm glue
  loaded from `./playground.js`.
- `playground.js` + `playground_bg.wasm` — built via plain `cargo build
  --target wasm32-unknown-unknown` plus the `wasm-bindgen` 0.2.127 CLI
  (no trunk/wasm-pack). Pass `--no-wasm` to skip this step (tests).
- `examples/*.rb` — curated snippets, each verified end to end
  (compile+execute; the AST/disassembly viewers carry one smoke each,
  not per-example asserts).

Serve with `python3 -m http.server` inside `dist/` (file:// blocks the
wasm module load); no network needed beyond localhost.

## Console behavior

- Diagnostics carry UTF-8 byte offsets; the editor maps them to
  line:col/selection via `TextEncoder` lengths (JS strings are UTF-16).
- Curated examples carry host-baked bytes; "Verify vs host bytes"
  compares the wasm build byte for byte. Ad-hoc code has no reference
  diff in-browser — byte identity is certified via the CLI on native.
- stdout comes from capturing `puts`/`p` overrides installed after
  `VM::open` (thread-local buffer), shared by native and wasm, plus the
  return-value `inspect`. The fork's `println!` path never fires.

## Findings

- Bigint probe: the pinned fork still panics on pool type 7
  (`rite.rs:253`, u16 misread `0x170a` → out-of-bounds slice on
  `x = 99999999999999999999999`). Bigints stay gated fail-closed
  before load, on native and wasm alike.
- wasm32 check: `carnelian-ast`, `carnelian-compiler`,
  `carnelian-front-mri`, `mrubyedge` and the glue all build for
  `wasm32-unknown-unknown`; smoke-verified under node by hand (compile,
  run, gate, diagnostics, byte-identity of all curated examples) —
  no CI job runs node yet.

## Compliance split

Web/wasm builds are smoke-only, never golden asserts. All behavior is
certified via the CLI on native in `tests/` (debug and release).

## Layout

`src/lib.rs` (pipeline + wasm exports `pg_compile`/`pg_run`/`pg_bytes`),
`src/pipeline.rs`, `src/ast_json.rs` (`Visit` walker + `Debug`
fallback), `src/disasm.rs` (`OP_NAMES` over `decode_at`),
`src/run.rs` (gate + capture executor), `src/emit.rs` + `src/page.html`
+ `src/app.js` (site writer), `src/main.rs` (generator binary).
