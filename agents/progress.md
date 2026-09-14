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
