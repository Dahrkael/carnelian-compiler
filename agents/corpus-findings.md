# Corpus findings (real-world tranche, `feature/real-corpus`)

Status: 264 files / 1056 rows (mri+prism × stripped/unstripped):
**1052 identical, 0 ref_reject, 4 our_gate, 0 diverge, 0 unreadable.**
The 4 gates are one file (`mruby/test/t/bs_block.rb`, nested splat in
block params, both frontends × both modes).

## Fixed during triage (all byte-certified, both frontends unless noted)

- `super(*a)` splat: `gen_values` already flushed to `OP_SUPER 15`; only
  the two `super_view`s vetoed `SplatNode` (`owned.rs`, `backend.rs`).
- `super() { }` literal block + `super(&block)`: `SuperView` gained
  `block`; `gen_super` codegens it (`OP_BLOCK` / block-arg read) instead
  of the block move, mirroring C (taken even with `arguments: None`,
  which C never sees — bare `super` is a forwarding node there).
- `super(k: v)` kwargs: `split_keywords` + `gen_hash` loop in `gen_super`
  (same pattern as `gen_yield`), `n |= nk<<4`.
- `def t(foo = foo)` circular defaults: demoted
  `CircularArgumentReference` to a warning (`parse.rs`); the reference
  compiles it (Prism reports `PM_ERR_PARAMETER_CIRCULAR` alike).
- Numbered params on MRI: `lower_numblock` takes `numargs` for
  `NumberedParametersNode.maximum` (was hardcoded 0) and lowers
  `Numblock{call: Lambda}` (`-> { _1 }`) instead of dropping it through
  `other`. Backend already encoded `REQ(max)` + `R1` reads.
- Nested-heredoc resumption (`MM1`/`XXX1-3` in `literals.rb`): the
  reference lexer flushes outer content when a nested heredoc takes over
  (`y\n` + `mm1\n`), the 3.1 grammar glues (`y\nmm1\n`).
  `split_nested_resumption` cuts the post-interpolation part after its
  first `\n`, only past an interpolation whose subtree opens a heredoc
  (plain glued parts like `q\nr\n` untouched; verified with 1- and
  2-line followers).
- `<<~` with backslash-led / mixed-indent lines (`v2` in `literals.rb`):
  the 3.1 grammar over-dedents (strips the `\t` escape as indent); the
  reference computes tab-aware widths (`\` → 0, tab → next ×8, empty
  lines skipped), skips dedent at `common == 0`, and strips per line
  with a tab-overshoot break (`parse_heredoc_dedent_string` port:
  `squiggly_width` + `squiggly_strip`). Rebuild applies only to plain
  `<<~` with mixed tab/space or backslash-led lines; every other shape
  stays on the lowering path. Lines are unescaped before stripping
  (reference order; `<<~'x'` skips unescape), with a bail-out on joined
  continuations.

## Structural / future (gated, not wrong bytes)

- `bs_block.rb` `|(v,(*))|`: nested splat inside destructured block
  params. `param_layout` has no nested-splat shape; C emits LVAR loads
  it does not verify itself. Rare in real code; needs an `mlhs`
  redesign, not a 1:1 port. Gate stays honest (`unsupported method
  parameters in P1: BlockNode`).

## Reference-behavior notes (no action)

- No `ref_reject` in the full corpus: the pinned reference compiles
  everything we feed it (after the circular-default demotion).
- `verify` ignores `DBG` by design (reference dumps with flags 0).
