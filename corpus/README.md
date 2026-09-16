# Real-world corpus (dev only, gitignored sources)

Byte-certification against 264 real Ruby files (mruby 3.4.0 + picoruby
3.4.5): every `.rb` under `.corpus/` compiles with `mri` and `prism` and
is compared to the pinned reference (`mruby-compiler2 0.5.0` + Prism
1.9.0) in both strip modes — 1056 rows total.

Current status: **1052 identical, 0 ref_reject, 4 our_gate, 0 diverge**
(the 4 gates are `mruby/test/t/bs_block.rb`, nested splat in block
params; see `agents/corpus-findings.md`).

## Fetch (no vendoring)

```sh
tools/fetch_corpus.sh   # pins SHAs, prunes non-Ruby, writes .corpus/PROVENANCE
```

Third-party sources stay in `.corpus/` (gitignored); only the pins and
`PROVENANCE` are reviewed. Never commit `.corpus/`.

## Run

```sh
tools/corpus.sh                              # fetch if missing + check
cargo run -p carnelian-cli -- corpus --frontends all --check
cargo run -p carnelian-cli -- corpus --frontends all --update   # rewrite baseline.tsv
cargo run -p carnelian-cli -- corpus --frontends all --report corpus/report.md
```

Row statuses: `identical`, `ref_reject` (reference fails — must stay 0),
`our_gate` (honest `unsupported`, triage per file), `diverge` (bytes
differ — fix or record), `unreadable`.

## Triage policy (option b)

Fix 1-instruction/operand and shape divergences now (port the C arm,
lock a golden); record structural ones (lexer-chunk boundaries the
grammar cannot see, unverified C casts, `mlhs` redesigns) in
`agents/corpus-findings.md` with root cause and reproducer. The baseline
protects the future: `--check` fails on any drift.
