# PINS.md — Compatibility tuple (source of truth)

Byte-identity is certified **only** against this tuple, in both modes
(with `DBG` section and stripped).

| Field | Pin | Notes |
|---|---|---|
| C compiler | `mruby-compiler2 0.5.0` | Prism route (PicoRuby/FemtoRuby); crate `mruby-compiler2-sys =0.5.0` |
| C parser | `Prism 1.9.0` | vendored inside compiler2; crate `ruby-prism =1.9.0` / `ruby-prism-sys =1.9.0` |
| MRI parser | `lib-ruby-parser 4.0.6+ruby-3.1.2` | pure-Rust frontend (P4); grammar ceiling 3.1.2; crates `lib-ruby-parser =4.0.6+ruby-3.1.2` / `lib-ruby-parser-ast =0.55.0` |
| Format | `RITE0400` | `RITE` + major `04` + minor `00`; anything else is rejected |
| Build config | `PICORB_VM_MRUBYC` (`MRC_TARGET_MRUBYC`), `MRBC_ALLOC_LIBC`, `MRB_NO_PRESYM`, `MRB_INT64=1`, `PRISM_BUILD_MINIMAL`, `PRISM_XALLOCATOR` | exact mirror of `mruby-compiler2-sys@0.5.0` `build.rs` |
| Int | 64-bit | `MRC_INT_BIT = 64` |
| Binary header | `RITE0400` / compiler `HSMK` / version `0000` / IREP section `0400` | from `mrc_dump.h` (`RITE_COMPILER_NAME`, `RITE_VM_VER`) |

A CI pin test fails if the locked reference versions differ from this file.

## Compatibility changelog

- `0.1.0` (released 2026-09-15, tag `v0.1.0`): P0–P4 + P2.6 pattern matching + DEBUG section + deferred call threading. Initial tuple (P0); MRI parser pin added in P4.
- `0.1.1` (tag `v0.1.1`): same tuple; adds the missing codegen arms for real-world code (issue #2): `return`/`break`/`next`/`redo`, ranges, parentheses, attribute/index/call assignment, op-assign and `||=`/`&&=` family.
