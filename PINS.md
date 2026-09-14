# PINS.md — Compatibility tuple (source of truth)

Byte-identity is certified **only** against this tuple, in both modes
(with `DBG` section and stripped).

| Field | Pin | Notes |
|---|---|---|
| C compiler | `mruby-compiler2 0.5.0` | Prism route (PicoRuby/FemtoRuby); crate `mruby-compiler2-sys =0.5.0` |
| C parser | `Prism 1.9.0` | vendored inside compiler2; crate `ruby-prism =1.9.0` / `ruby-prism-sys =1.9.0` |
| Format | `RITE0400` | `RITE` + major `04` + minor `00`; anything else is rejected |
| Build config | `PICORB_VM_MRUBYC` (`MRC_TARGET_MRUBYC`), `MRBC_ALLOC_LIBC`, `MRB_NO_PRESYM`, `MRB_INT64=1`, `PRISM_BUILD_MINIMAL`, `PRISM_XALLOCATOR` | exact mirror of `mruby-compiler2-sys@0.5.0` `build.rs` |
| Int | 64-bit | `MRC_INT_BIT = 64` |
| Runtime reader (dev) | `mrubyedge 2.0.0` | `mrubyedge::rite` used to cross-check the P0 round-trip |
| Binary header | `RITE0400` / compiler `HSMK` / version `0000` / IREP section `0400` | from `mrc_dump.h` (`RITE_COMPILER_NAME`, `RITE_VM_VER`) |

A CI pin test fails if the locked reference versions differ from this file.

## Compatibility changelog

- `0.1.0` (unreleased, P0): initial tuple. Writer round-trip only, no codegen yet.
