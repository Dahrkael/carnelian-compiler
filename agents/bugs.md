# Known bugs (external)

## mrubyedge 2.0.0 panics on pool type 7 (BIGINT)

- Reference output for `x = 99999999999999999999999` contains the pool entry
  `07 17 0a 39…` (type 7, length byte `0x17` = 23, then `0x17 + 1` bytes).
- `dump.c` is the source of truth: `len = (unsigned char)str[0]`, emits
  `len + 2` bytes verbatim after the type tag.
- `mrubyedge 2.0.0` (`src/rite/rite.rs`) reads the length as `u16` big-endian
  (`0x170a` = 5898) and slices out of bounds, panicking instead of
  returning `Err`.
- Handling in P0: the carnelian reader/writer preserve the raw `len + 2`
  bytes; `carnelian verify` skips the mrubyedge cross-check with a note when
  the model contains a `BigInt` pool entry.
- Revisit if mrubyedge fixes the parsing upstream.
