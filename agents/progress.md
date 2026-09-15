# Progress log (deviations from agents/plan.md)

Rule: stay on plan unless blocked by insurmountable need. Every deviation
is recorded here with cause. User decisions are not deviations.

## P0 (done, merged to dev)

- Scope: minimum workspace (`carnelian-ast`, `carnelian-compiler`,
  `carnelian-cli`) instead of all six crates. User-approved, not a deviation.
- `verify` in P0 round-trips `reference` through the Rust writer instead of
  comparing `compile` vs `reference`: there is no codegen yet. `compile` is a
  stub returning diagnostics, exit 1.
- The RITE reader/writer are our own port of `dump.c`, not a wrapper over
  `mrubyedge::rite`. Cause: the shipping library must stay dependency-free
  for `wasm32`; `mrubyedge::rite` is used only as a cross-check in the dev
  CLI. See `agents/bugs.md` for the `BIGINT` case where the cross-check is
  skipped.
- `.cargo/config.toml` sets `target-feature=-crt-static` for the musl host
  target. Cause: musl links build scripts statically by default and `dlopen`
  fails ("Dynamic loading not supported"), so `bindgen` cannot load
  `libclang` when building `mruby-compiler2-sys`. `clang` rejects
  `-crt-static` as a C flag; it is a rustc target feature. Scoped to the
  musl triple so wasm builds are unaffected.

## P1 (in progress, feature/p1-codegen)

- No `DEBUG` section emission. Cause: the pinned reference
  (`mruby-compiler2-sys 0.5.0`) hardcodes dump flags to `0`, so no C golden
  with `DBG` exists; emitting `DBG` would break the byte-identity exit
  criterion. `verify` runs with and without `--strip` against the same
  flags-`0` reference, per the certification section of the plan. The
  `debug.c` port (filename table, packed line maps) and the `lines`/`lineno`
  tracking are deferred together until a `DBG` reference path exists.
- No catch-table emission in the infra slice. Cause: nothing in the P1
  corpus raises handlers; the `CatchHandler` model and writer support exist
  from P0. Emission (`catch_handler_new`/`set`) arrives with the
  rescue/ensure tranche (plan P2.5).
- `BackendNode` trait realisation: handler-facing trait with defaulted
  typed accessors, children passed by value. FFI side uses a `Copy`
  `PrismNode` wrapper (a bare `ruby_prism::Node` cannot implement `Copy`,
  and the trait needs by-value children for recursion); owned side uses
  `&Node` (also `Copy`). This is the plan's recommended thin-access trait,
  not handler duplication.
- Non-decimal integer literals beyond `u128` range are a diagnostic error.
  Cause: `ruby-prism` exposes binary limbs (`to_u32_digits`), and the only
  exact text source is decimal source slices; radix conversion past `u128`
  is deferred until the corpus needs it. Decimal bigints of any size work
  through the `new_litbint` path.
- `mrubyedge` left the dependency tree (PINS.md row and pin test updated).
  Cause: the P0 `mrubyedge::rite` cross-check lost its purpose once `verify`
  compares real codegen output; the writer is certified directly against C
  goldens, and the `BIGINT` panic made the cross-check a liability.
- Tranche gating uses clear diagnostics (splat/keyword/forwarding args,
  blocks, upvars, `begin`-modifier loops, attribute assignment). The plan's
  own P4 gating pattern, applied per tranche; each gate lifts with its
  tranche and the corpus guards the boundary.
- The `sp >= 99` argument flush inside `gen_values` is deferred with the
  splat tranche: unreachable below 99 registers, and the corpus never gets
  close.

## Reviewer round on P1 (applied, same worktree)

- Real bugs fixed: `Decoded.a` truncated `OP_ENTER` (now `u32`, faithful to
  `mrc_insn_data`); `genjmp2` recursed on chained `MOVE`s instead of the
  single C rewrite; widened jumps stamped `lastpc` before `EXT1` (C stamps
  after); `Int64` pool dedup was missing (`mrc_common.h` defines
  `MRC_64BIT`, proven by a duplicated-literal golden); empty statement
  lists skipped `LOADNIL` in `NOVAL` mode.
- `codegen()` null-tree guard discovered: a null subtree emits `LOADNIL`
  only when valued, while an empty statements *node* always emits it.
  Option branches route through `gen_branch`; verified against goldens for
  empty `while` bodies and empty `then`/`else` in both modes.
- `new_sym` mirrors the C `scapa` doubling, so the 32768-symbol limit
  matches instead of 65535.
- Declined with reason: merging the `nil?`/`if` jump skeletons (the former
  carries an extra unconditional jump), a widening helper for `genop_*`
  (hot path, no need), removing the `last_insn` fallback (it guards genuinely
  unreachable decodes).

## Reviewer round on P2 (applied, same worktree)

- No faithfulness bugs found. Applied: shared `flush_hash_pairs` tail,
  `emit_load2` for the leading empty literal, `val_stack_limit` helper,
  `emit_absent_else` at the three identical sites, `u32` limit consts,
  `try_from` counts instead of silent `as` casts.
- The new multi-splat corpus caught a real pre-existing bug: `flush_hash_pairs`
  read the destination before the extra `HASHADD` pop. Fixed; all other `dst`
  reads audited clean.
- Declined: merging the `gen_case`/`gen_if` valued merges (different shapes:
  `pos3` chain, conditional pop and move); only the identical absent-`else`
  fragment was shared.
- Skipped e2e with reason: bare `when *` (rejected by the reference parser),
  `#@v` (needs P2.4 ivars), empty-node `when` bodies (Prism only yields null;
  covered at adapter level).

## P2 remaining tranches (worktree feature/p2-rest)

Four parallel worktree branches were implemented from the dev snapshot and
integrated into `feature/p2-rest`:

- P2.3 blocks/lambdas/yield; P2.4 def/class/module/sclass, constants,
  ivar/cvar/gvar, `super`; P2.5 rescue/ensure, splat, kwargs, masgn;
  P2.6 alias/undef and `defined?`.

Deviations and decisions during integration:

- Merges were resolved by union of additions plus dedup of infrastructure the
  tranches added independently (`args_req`, `ainfo`/`aspec`/`mscope`, the
  child-scope constructors). A plain merge interleaves same-shaped trailing
  methods, so trait/impl conflicts were rebuilt by taking the accumulated side
  and re-appending the incoming branch's methods.
- Cross-tranche gates that went stale were lifted and certified, same category
  as the P2 review: `f { 1 }` (call blocks), `rescue TypeError`/`Foo::Bar`
  (constants tranche), and `defined?` over constant/ivar/gvar/cvar receivers
  and chains.
- Integration review found a real merge defect: the p23 `gen_yield` still
  assumed `call_args()` gated keyword/splat, so `yield k: 1`, `yield *a`,
  `yield **h` and 15-argument yields diverged after p25 lifted that gate.
  Ported the C `PM_YIELD_NODE` keyword/`CALL_MAXARGS` transport (BLKCALL vs
  `:call` dispatch) and added regression snippets.
- Still gated after P2: `case/in` pattern matching (`codegen_pattern`),
  back-reference/numbered-reference `defined?`, `BEGIN`/`END` and flip-flop
  (the latter three are rejected by the pinned reference parser, so no golden
  can exist), non-decimal `>u128` integer literals, and the exotic parameter
  and target forms noted per tranche.
- Remaining known duplication (not a correctness issue): `gen_class`/
  `gen_module`/`gen_sclass` share a body-scope tail that C factors as
  `scope_body`; `gen_yield`/`gen_call_impl` repeat the `CALL_MAXARGS`
  protocol.

## P2 remaining scope after the tranche integration

Reference probe results (pinned `mruby-compiler2 0.5.0`):

- `BEGIN` (`PM_PRE_EXECUTION_NODE`), `END` (`PM_POST_EXECUTION_NODE`) and
  flip-flop (`PM_FLIP_FLOP_NODE`) are **not implemented by the reference
  compiler itself** (`Not implemented: ...` / compile failure). No
  byte-identical golden can exist, so our fail-closed diagnostics are the
  correct end state, not a gap. Revisit only if the pin changes.
- Done on `feature/p2-params` (certified byte-identical in both modes):
  - Non-trivial parameters (P2.3/P2.4): optional, keyword (with/without
    default), keyword-rest, rest, post, block parameter and destructured
    parameters in `def`, blocks and lambdas, plus the full `OP_ENTER`/
    aspec/ainfo computation and argument-setup opcodes (`OP_KEY_P`,
    `OP_KARG`, `OP_KEYEND`, optional jump table, block move, `APOST`
    destructuring). Includes `...` forwarding in definitions; call-side
    `...` (`bar(...)`) stays gated as call-expression work.
  - masgn targets beyond plain locals (P2.5): ivar/cvar/gvar/const/index/
    call, including splat-index (`OP_ARYPUSH` gather path), `self.x`
    (`OP_SSEND`) and nested plain-local multis.
  - `defined?` and plain reads of back-references (`$&`, `$~`, `` $` ``,
    `$'`, `$+`, `$1`… including unrepresentable numbers as nil).
  - Non-decimal integer literals past `u128` (binary/octal/hex): overflow
    digits now stringify the limb value like `pm_integer_string` instead of
    slicing decimal source text.
- Not possible (reference rejects; fail-closed diagnostics are the end
  state, revisit only on pin bump):
  - `&nil` (`MRC_ARGS_NOBLOCK`): all forms fail in the reference, and
    upstream `ruby-prism 1.9.0` cannot parse them either.
  - Constant-path masgn targets (`a, Foo::B = x`): reference errors with
    `Not implemented (#1)` (no `PM_CONSTANT_PATH_TARGET_NODE` arm in its
    `gen_assignment`).
  - Destructured parameters with non-local parts (`def foo((@a, b))`):
    both the reference and upstream Prism reject them at parse level.
- Pending (reference-supported, still gated):
  - `case/in` pattern matching (`codegen_pattern`, ~1000 lines of
    failure-jump chaining and caching) — out of scope for this branch,
    belongs to P2.6-pattern.
- Corners closed after the first pass (same worktree):
  - Nested destructured parameters (`def foo((a, (b, c)))`): the reference
    emits bytes derived from an unchecked C cast (the inner multi's
    `lefts.size` misread as a pool id). The ids land on presymbols for any
    realistic arity (verified `<<` for two lefts, `>>` for three), so the
    port maps the size through the pinned `mrc_presym.inc` table
    (`presym_bytes`); sizes past the table stay gated. Proven by
    `def_destructure_nested*` goldens.
  - `defined?` with a back-reference receiver (`defined?($&.foo)`): needs
    the operand twice (receiver check plus receiver value). `PrismNode`
    gained a generated `Clone` (build-script `clone_node` over the config
    node list, so pin bumps stay exhaustive-checked) and `BackendNode` a
    `Clone` bound; the receiver arm now codes the check plus the value
    like C.
  - `...` (like splats/keywords before it) in a `recv_ready` chain link
    (`defined?(o.b(...).c)`, same family as the gated
    `defined?(x.foo(*a).bar)`): the reference compiles it, the port gates
    it as complex arguments. Fail-closed and rare; threading `recv_ready`
    through gathered-argument calls is deferred.
  - Call-side `...` forwarding (`bar(...)`, `bar(1, ...)`, `super(...)`):
    `gen_values` forwarding arm (`ARGCAT`/`HASH`/`HASHCAT` plus `&`) with
    upvar fallback (`gen_forward_arg`), and calls force the block shape
    with a literal `0xFF` operand (`FORWARD_ARGS`). `...` mixed with other
    declared parameters (`def foo(a, b: 1, ...)`) is rejected at parse
    level by both prisms ("unexpected parameter order"), so no golden can
    exist there either.

## Reviewer round on P2-params (applied, same worktree)

- No byte-divergence bugs found; all suites pass, clippy clean under
  `-D warnings`, wasm32 build of the pure-Rust path passes.
- Applied: unified the four `*_target_name` helpers into one
  `either_target_name`; `ParamCounts::block_name` is taken, not cloned;
  the `NIL_BLOCK` flag bit is a named constant.
- Declined with reason: merging the two `gen_assignment` kind matches
  (mirrors C's two switches: rhs prologue, then the store); sharing the
  `gen_def` receiver arms or the class/module/sclass tails (pre-existing
  shape, out of scope); replacing `format!("_{}", i)` numbered params
  with a static table (pre-existing, equivalent output); comments noting
  gates and reference workarounds (file convention; decisions also live
  here); non-zero diagnostic offsets (file convention is `0, 0` for these).

## Reviewer round on the tiny corners (applied, same worktree)

- No byte-divergence bugs found; all suites pass, clippy clean, wasm32
  build passes. The 57-entry presym table was verified byte-for-byte
  against `mrc_presym.inc`; the `0xFF` operand, super forwarding flow,
  backref triple-use and fail-closed agreement (index/yield/`...`-mix
  parse rejections on both sides) all check out.
- Applied: goldens for `def foo(a = 1, ...)` forwarding, `bar(*a, ...)`
  splat-plus-forwarding, rest inside nested multis, chained/splat
  backref receivers and block-scoped `super(...)`; doc fixes for
  `SuperView`/`call_args`; progress-note correction (two evaluations,
  not three) plus the `recv_ready` forwarding gap line.
- Declined with reason: threading `recv_ready` through gathered-argument
  calls (deferred; same fail-closed family as the pre-existing splat
  chain gate).

## P3.3 parity harness + CLI owned path (worktree feature/p3-owned, unmerged)

Scope: `carnelian-cli` only (`src/main.rs`, `Cargo.toml`, `tests/*`).
Frozen files untouched: `view.rs`, `handlers.rs`, `front-owned`
`lower.rs`/`owned.rs`/`lib.rs` (siblings A/B implement them in parallel).

- Corpus refactor: the 7 `SNIPPETS` tables (44+30+30+47+85+79+23 entries)
  and 5 `GATED` tables (22 entries) moved byte-identical into
  `tests/corpus.rs` (`P1/P2/P23/P24/P25/P26/ROUNDTRIP_*`, `P2/P23/P24/P25/P26_GATED`)
  plus `synthetic_cases()`/`synthetic_source()` for the 7 generated
  limit-path sources. Test files share it via `#[path]`; all refactored
  suites pass unchanged (`corpus.rs` carries `#![allow(dead_code)]` since
  each target uses only its own tables).
- CLI: `compile --frontend owned` = parse (FFI) → `lower` →
  `compile_tree::<Owned>`; unknown frontends stay exit 2. `verify`
  gained `--frontend {prism|owned}`, default `prism`.
  `roundtrip::cli_exit_codes` now expects owned exit 0 with
  prism-identical bytes (plus an unknown-frontend exit 2 check).
- New `tests/p3_parity.rs`: 345 snippets (338 table + 7 synthetic) × 2
  modes 3-way `reference`/`prism`/`owned` compare; 22 gated agreements
  (exit 1 + same marker under both frontends); two host fixtures
  (hand-built `Node` → `compile_tree::<Owned>` asserting the exact
  reference bytes for `` and `puts 1`).
- Wasm: `cargo check -p carnelian-ast -p carnelian-compiler --target
  wasm32-unknown-unknown` passes. Fixtures live in CLI tests (host-only);
  `prism_pin.rs` untouched, still host-only via its `ruby-prism` dev-dep.
- Blocker (not in P3.3 scope; needs a shared-infra fix): the generated
  flag constants in `carnelian-ast` (`build.rs` emits `1 << index`, but
  Prism shares the `u16` word with the generic `NEWLINE=0x1` /
  `STATIC_LITERAL=0x2` bits, so every node-specific group really starts at
  bit 2: call `SAFE_NAVIGATION` is 4, not 1; same ×4 shift for all 15
  groups, verified against vendored `ast.h`). Lowered `puts 1` carries real
  flags `33` (`NEWLINE|IGNORE_VISIBILITY`), which `Owned::call` misreads as
  `safe_nav`, emitting a phantom `MOVE+JMPNIL` (+7 bytes). Characterization:
  345 snippets, 24 pass (all call-free), 321 fail owned-only with the same
  +7-byte shape; prism matches everywhere; all 22 gated agreements and both
  fixtures pass. Fix proposal: emit `1 << (index + 2)` in `build.rs` with a
  comment citing `pm_node_flags_t`; sibling B unit tests that build flags
  from the same constants stay self-consistent.
- Deviation: used a `python` one-liner to patch a throwaway debug test that
  was deleted afterwards (AGENTS.md wants the Edit tool even there).

## P3 integration (coordinator)

- Flag constants: applied the proposed `1 << (index + 2)` fix in
  `carnelian-ast/build.rs` (verified against vendored `ast.h`:
  `PM_NODE_FLAG_NEWLINE=0x1`, `STATIC_LITERAL=0x2`,
  `PM_CALL_NODE_FLAGS_SAFE_NAVIGATION=4`). Was a real bug, not drift.
- Negative bigints: `lower` keeps the `-` prefix in `Fallback.raw`
  (agent B's `bigint_from_raw` already splits it back off); agent A's
  signless test expectation flipped to match.
- `const_path_write` on owned expected a `ConstantPathTargetNode`, but
  Prism models the write target as a plain `ConstantPathNode` (FFI
  `target()` return type confirms). Fixed the accessor, plus the
  hand-built test tree. Was a real bug (every plain `A::B = 1` failed).
- Parity corpus: `ROUNDTRIP_SNIPPETS` dropped from `p3_parity.rs` (they
  certify the writer; `while_loop` holds gated `i += 1`). `roundtrip.rs`
  still covers all 23.
- Reviewer round on P3 (applied): all three fixes verified correct
  against headers/bindings; goldens added for optional-forwarding defs,
  splat-plus-forwarding calls, rest-in-nested-multis, backref receiver
  chains and block-scoped `super(...)`; `SuperView`/`call_args` docs
  fixed; `Integer::from_decimal` documented as reserved for text-source
  frontends (P4 MRI). Declined: sharing `limbs_to_decimal`/`NIL_BLOCK`
  helpers across frontends (stable ports, parity-guarded) and threading
  `recv_ready` through gathered calls (fail-closed family, deferred).
- Exit state: full workspace suite green (incl. 345×2×3 parity and 22
  gated agreements), clippy `-D warnings` clean, fmt clean,
  `wasm32-unknown-unknown` check of the pure path passes. Follow-up for
  later: `recv_ready` gathered-call threading.

## Reorg: front-owned dissolved (same worktree)

- `Owned` + `impl BackendNode` moved to `carnelian-ast` (`src/owned.rs`,
  pure/shipping); `lower` moved to `carnelian-front-prism` (`src/lower.rs`,
  dev/CLI); `front-owned` crate deleted. CLI and fixtures use
  `carnelian_ast::Owned` / `carnelian_front_prism::lower`.
- `compile_prism` renamed to `compile_tree` (generic entry; the
  `compile(source)` stub stays reserved for P4). Call sites, docs and
  notes updated.
- Reviewer round on the reorg (applied): import grouping, `Span` import,
  `front-prism` crate docs, `plan.md` workspace map (also fixed its
  `front-mri` phase typo: Fase 4, not 3). Declined: wiring the inert
  `front-*` feature flags (P4 owns that).

## P4-C: CLI wiring + parity suite + pins (worktree feature/p4-mri, unmerged)

Scope: `carnelian-cli` only (`Cargo.toml`, `src/main.rs`, `tests/*`,
plus the `PINS.md` row and this note). Frozen contracts untouched:
`front-mri` `parse`/`lower`/`scope`/`lib`, `carnelian-ast`,
`carnelian-compiler`, `front-prism`.

- CLI: `compile --frontend mri` = `front_mri::parse` (parse errors print
  as `error: msg (start:end)`, exit 1, same shape as prism) then the
  end-to-end `front_mri::compile(source, opts)`. Unknown frontends stay
  exit 2; exit codes 0/1/2 unchanged.
- New `tests/p4_mri.rs`: 321 snippets (315 tables minus the `it` ceiling
  exclusion, plus 7 synthetic) x 2 modes x 4-way
  `reference`/`prism`/`owned`/`mri`; 21 gated agreements x 3 frontends
  with shared markers; 3 grammar-ceiling gates (`it`, anonymous 3.2
  `*`/`**` forwarding) with per-frontend expectations (`mri` exit 1,
  prism/owned exit 0); 6 scope-vector snippets (nesting, shadowing,
  upvar depths, block-locals, def boundary); 5 `for` parity snippets;
  one host smoke pinning `front_mri::compile("puts 1")` bytes.
- `roundtrip.rs::cli_exit_codes` gained the `mri` exit-0 block (existing
  assertions identical); `locked_pins_match_pins_md` covers
  `lib-ruby-parser 4.0.6+ruby-3.1.2` and `lib-ruby-parser-ast 0.55.0`.
- Wasm: `cargo check -p carnelian-ast -p carnelian-compiler
  -p carnelian-front-mri --target wasm32-unknown-unknown` passes, so
  `lib-ruby-parser` needs no C on the shipping path.
- Blocked green: `front_mri::parse` (`parse.rs`, `todo!`), `lower`
  (`lower.rs`, `todo!`) and the end-to-end `compile` (`lib.rs`, `todo!`).
  Sibling B's `resolve_scopes` (`scope.rs`) landed. Every `mri` test
  fails with exit 101 and `not yet implemented: P4: run the MRI parser`
  until parse/lower/compile land.
- Sibling D landed mid-session: backend `gen_for` plus `ForView` on the
  trait, `Owned` and `PrismNode`, so `for` compiles exit 0 and matches
  the reference on all probed variants. `P25_GATED::for_gated` was removed
  from the corpus table (not by P4-C); no `for` snippet was added to
  `P25_SNIPPETS`, so `for` currently has no shared-corpus coverage — the
  five `FOR_SNIPPETS` in `tests/p4_mri.rs` cover the gap until the
  coordinator adds entries. The pre-existing `p25`/`p3` gated suites are
  green again after the removal.
- Probe notes (pinned `lib-ruby-parser`, throwaway crate outside the
  repo): bare `bar(*)`/`bar(**)` forwarding and endless setters fail at
  parse; `it` parses as a send, so its gate needs explicit frontend
  logic; endless setters also fail under prism/reference, so they are
  not ceiling gates. No deviation from the plan.

## P4 integration (coordinator)

- `parse.rs` + `compile()` (unassigned in the split) implemented here:
  `Parser::new`/`do_parse`, errors-only mapping via `render_message`,
  `root()` returns `Option` (empty source has no AST). `compile()` wraps
  the lowered tree in a synthesized `ProgramNode` (`lower()` keeps its
  bare contract pinned by 59 tests) and synthesizes an empty program for
  `""`; then resolve scopes and `compile_tree`.
- `it` ceiling scan: bare `it` sends inside block bodies fail with a
  ceiling diagnostic (explicit `|it|`-family params shadow per block;
  `def`/`class`/`module`/`sclass` seal the outer stack but keep
  scanning inside; lambdas/numblocks inherit). Top-level `it` still
  compiles as a plain call (3.1-correct).
- `BlockPass` split: trailing `&block` fills `CallNode.block`,
  `SuperNode.block` and index-read `[]` calls (was an argument item →
  `SEND` instead of `SENDB`); `YieldNode` has no block field, stays
  inline like FFI. Index writes with `&` fail on both sides (parse
  reject vs attribute gate), no action.
- Destructured rest: `def` params arrive as `Mlhs` with `Restarg`
  (masgn uses `Splat`), so both `split_mlhs` and `lower_destructured`
  split rest/rights; anonymous rest matches Prism's `SplatNode`
  (probe-verified). One agent-A test expectation fixed to the Prism shape.
- Parenthesized single statements wrap in `StatementsNode` (Prism shape;
  `defined?` unwraps exactly that) — fixes `defined?((x))`.
- Gated markers may differ by locus: `mri_marker` override table in
  `p4_mri.rs` (Prism parse-rejects bare `yield` as `Invalid yield`,
  MRI reaches the backend `invalid yield`).
- `for` snippets moved to shared `P25_SNIPPETS` (4-way for all
  frontends); agent C's `FOR_SNIPPETS`/`for_parity` removed as duplicate.
  `index_block_arg` added to the shared corpus for the read path.
- Exit state: full workspace suite green (incl. 326×2×4 parity, 21
  gated agreements, scope vectors, ceiling gates, host smoke),
  clippy `-D warnings` clean, fmt clean, `wasm32` check passes for
  `ast`+`compiler`+`front-mri` (pure-Rust end-to-end closure).
  Follow-ups: `recv_ready` gathered-call threading (deferred, fail-closed
  family); sharing small helpers across frontends (stable ports).

## Reviewer round on P4 integration (applied, same worktree)

- Real bugs fixed: the `it` scan skipped block calls (`foo(it) {}`),
  sealed superclass/`sclass`/definee expressions that evaluate outside,
  and `Lvar`-bound `it` reads from nested param-less blocks (all silent
  divergences, all now gated with the ceiling diagnostic); binder
  detection looks through `Procarg0`/`Mlhs` wrappers.
- Declined with reason: `debug_assert!` guards on unreachable
  `BlockPass` positions (both grammars parse-reject; the shapes are
  documented); single-parse CLI (harmless, matches existing arms);
  span-style nits (byte-harmless).

## Playground + CLI features (worktree feature/playground, unmerged)

- CLI: two-axis split `reference` (C golden) + `prism` (C parser),
  `default = both`. Pure build (`--no-default-features`) keeps
  `compile --frontend mri`, `--pins`, `--help`; `reference`/`verify` and
  `prism`/`owned` vanish (exit 2 with pointer to `mri`). Proven pure via
  `cargo tree` (no sys/bindgen/cc) and green pure test legs.
- Playground crate: offline static site generator (textarea editor,
  JSON AST, hex+sections+disasm viewers, console). Executes on the
  pinned mrubyedge fork (playground-only dep) with capturing `puts`/`p`
  natives (thread-local, shared native/wasm path) and a fail-closed
  bigint gate before load — the fork still panics on pool type 7
  (probed `rite.rs:253`), so the gate stands.
- `puts` splats arrays per MRI (depth-capped `[...]` on cycles);
  multi-arg/nil/bare forms covered by tests. `OP_NAMES` pinned to
  backend constants via `debug_assert`s.
- Exit state: workspace suite green (default + pure legs), playground
  tests green in debug and release, clippy/fmt clean, wasm checks pass
  (`ast`+`compiler`+`front-mri`+playground lib).
- Reviewer round (applied): fixed vacuous mri-vs-mri compare in
  `roundtrip.rs`, per-combo comments, `pg_*` ungated marker, `main.rs`
  missing-subcommand message per feature set, `--release` threading in
  `build_wasm`, README claim scoping. Declined: middle-combo test
  splits (default-full + pure are the certification vehicles),
  `dist/` gitignore change (root `/dist/` already covers it),
  `recv_ready` threading (still deferred).
