//! Writer/reader unit tests over synthetic models (no C reference needed).

use carnelian_compiler::{
    encode_debug_section, pack_line_map, read_rite, without_debug, write_rite, DebugFile,
    DebugInfo, Irep, PoolValue, RiteModel,
};

fn minimal_root() -> Irep {
    Irep {
        nlocals: 1,
        nregs: 1,
        iseq: vec![0x00],
        ..Default::default()
    }
}

fn roundtrip(model: &RiteModel) -> RiteModel {
    let bytes = write_rite(model);
    read_rite(&bytes).expect("own output must parse")
}

#[test]
fn header_is_rite0400_hsmk() {
    let model = RiteModel {
        root: minimal_root(),
        ..Default::default()
    };
    let bytes = write_rite(&model);
    assert_eq!(&bytes[0..4], b"RITE");
    assert_eq!(&bytes[4..6], b"04");
    assert_eq!(&bytes[6..8], b"00");
    assert_eq!(&bytes[12..16], b"HSMK");
    assert_eq!(&bytes[16..20], b"0000");
    let size = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    assert_eq!(size, bytes.len());
    assert!(bytes.ends_with(b"END\0\x00\x00\x00\x08"));
}

#[test]
fn rejects_other_versions() {
    let model = RiteModel {
        root: minimal_root(),
        ..Default::default()
    };
    let mut bytes = write_rite(&model);
    bytes[4..6].copy_from_slice(b"03");
    assert!(read_rite(&bytes).is_err());
}

#[test]
fn int64_normalizes_to_int32_when_it_fits() {
    let model = RiteModel {
        root: Irep {
            pool: vec![PoolValue::Int64(42)],
            ..minimal_root()
        },
        ..Default::default()
    };
    let bytes = write_rite(&model);
    let parsed = read_rite(&bytes).expect("parses");
    assert_eq!(parsed.root.pool, vec![PoolValue::Int32(42)]);
    // Stable: re-emitting the normalized form is byte-identical.
    assert_eq!(write_rite(&parsed), bytes);
}

#[test]
fn int64_outside_i32_keeps_type_and_value() {
    for value in [
        i64::from(i32::MAX) + 1,
        i64::from(i32::MIN) - 1,
        3_000_000_000,
    ] {
        let model = RiteModel {
            root: Irep {
                pool: vec![PoolValue::Int64(value)],
                ..minimal_root()
            },
            ..Default::default()
        };
        let back = roundtrip(&model);
        assert_eq!(back.root.pool, vec![PoolValue::Int64(value)]);
    }
}

#[test]
fn pool_and_syms_roundtrip() {
    let model = RiteModel {
        root: Irep {
            nlocals: 2,
            pool: vec![
                PoolValue::Str(b"hello".to_vec()),
                PoolValue::Int32(-7),
                PoolValue::Float(1.5),
                PoolValue::BigInt(vec![0x01, 0x0a, 0x39]),
            ],
            syms: vec![Some(b"puts".to_vec()), None, Some(b"".to_vec())],
            catch_handlers: vec![carnelian_compiler::CatchHandler {
                kind: 0,
                begin: 0,
                end: 4,
                target: 8,
            }],
            ..minimal_root()
        },
        ..Default::default()
    };
    let back = roundtrip(&model);
    assert_eq!(back.root.pool, model.root.pool);
    assert_eq!(back.root.syms, model.root.syms);
    assert_eq!(back.root.catch_handlers, model.root.catch_handlers);
}

#[test]
fn nested_ireps_and_lvar_roundtrip() {
    let child = Irep {
        nlocals: 2,
        nregs: 2,
        iseq: vec![0x01, 0x02],
        lv: vec![Some(0)],
        ..Default::default()
    };
    let root = Irep {
        nlocals: 2,
        nregs: 2,
        iseq: vec![0x00],
        reps: vec![child],
        lv: vec![None],
        ..Default::default()
    };
    let model = RiteModel {
        root,
        lvar_syms: Some(vec![b"x".to_vec()]),
        debug_raw: None,
    };
    let back = roundtrip(&model);
    assert_eq!(back, model);
}

#[test]
fn bigint_matches_dump_c_wire_format() {
    // Real `mruby-compiler2 0.5.0` bytes for `x = 99999999999999999999999`:
    // type 7, length byte 0x17 (23), then 0x17 + 1 payload bytes.
    let mut raw = vec![0x17, 0x0a];
    raw.extend(core::iter::repeat_n(0x39, 23));
    let model = RiteModel {
        root: Irep {
            pool: vec![PoolValue::BigInt(raw.clone())],
            ..minimal_root()
        },
        ..Default::default()
    };
    let bytes = write_rite(&model);
    let entry_at = bytes
        .windows(raw.len() + 1)
        .position(|window| window[0] == 0x07 && window[1..] == raw[..])
        .expect("bigint entry on the wire");
    assert!(entry_at > 0);
    let back = read_rite(&bytes).expect("parses");
    assert_eq!(back.root.pool, vec![PoolValue::BigInt(raw)]);
    assert_eq!(write_rite(&back), bytes);
}

#[test]
fn debug_raw_is_preserved_verbatim() {
    let mut raw = b"DBG\0".to_vec();
    raw.extend_from_slice(&12u32.to_be_bytes());
    raw.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
    let model = RiteModel {
        root: minimal_root(),
        debug_raw: Some(raw.clone()),
        ..Default::default()
    };
    let bytes = write_rite(&model);
    assert!(bytes.windows(raw.len()).any(|window| window == raw));
    let back = read_rite(&bytes).expect("parses");
    assert_eq!(back.debug_raw, Some(raw));
}

#[test]
fn pack_line_map_collapses_runs() {
    // `mrc_debug_info_append_file` with `start_pos = 0`: each run emits
    // the pc delta then the line delta, both seeded from zero.
    assert_eq!(
        pack_line_map(0, &[1, 1, 2, 2, 2, 5]),
        vec![0, 1, 2, 1, 3, 3]
    );
    // Nonzero `start_pos` seeds the first pc delta absolutely.
    assert_eq!(pack_line_map(10, &[3, 3]), vec![10, 3]);
    // A zero first line matches the seed, so nothing is emitted.
    assert!(pack_line_map(0, &[0, 0]).is_empty());
}

fn debug_file(lines: &[u16]) -> DebugFile {
    DebugFile {
        start_pos: 0,
        filename: b"-e".to_vec(),
        lines: lines.to_vec(),
    }
}

#[test]
fn debug_section_layout_matches_dump_c() {
    // Synthetic expectations from the C layout: `DBG\0` header, filename
    // table without NULs, then one record per irep in preorder (size,
    // file count, `start_pos`, filename index, entry count, line type
    // `2` = packed map, packed bytes).
    let liney = Irep {
        debug: Some(DebugInfo {
            files: vec![debug_file(&[7])],
        }),
        ..minimal_root()
    };
    let empty = Irep {
        debug: Some(DebugInfo { files: Vec::new() }),
        ..minimal_root()
    };
    let root = Irep {
        iseq: vec![0x00, 0x00, 0x00],
        reps: vec![liney, empty],
        debug: Some(DebugInfo {
            files: vec![debug_file(&[4, 4, 9])],
        }),
        ..minimal_root()
    };
    let section = encode_debug_section(&root).expect("defined");
    let mut expected = b"DBG\0".to_vec();
    expected.extend_from_slice(&60u32.to_be_bytes());
    expected.extend_from_slice(&1u16.to_be_bytes());
    expected.extend_from_slice(&2u16.to_be_bytes());
    expected.extend_from_slice(b"-e");
    // Root record: packed `[4,4,9]` -> `[0,4,2,5]`.
    expected.extend_from_slice(&21u32.to_be_bytes());
    expected.extend_from_slice(&1u16.to_be_bytes());
    expected.extend_from_slice(&0u32.to_be_bytes());
    expected.extend_from_slice(&0u16.to_be_bytes());
    expected.extend_from_slice(&4u32.to_be_bytes());
    expected.push(2);
    expected.extend_from_slice(&[0, 4, 2, 5]);
    // Child record: packed `[7]` -> `[0,7]`.
    expected.extend_from_slice(&19u32.to_be_bytes());
    expected.extend_from_slice(&1u16.to_be_bytes());
    expected.extend_from_slice(&0u32.to_be_bytes());
    expected.extend_from_slice(&0u16.to_be_bytes());
    expected.extend_from_slice(&2u32.to_be_bytes());
    expected.push(2);
    expected.extend_from_slice(&[0, 7]);
    // Empty child: record with no files.
    expected.extend_from_slice(&6u32.to_be_bytes());
    expected.extend_from_slice(&0u16.to_be_bytes());
    assert_eq!(section, expected);

    let bytes = write_rite(&RiteModel {
        root,
        ..Default::default()
    });
    assert!(bytes.windows(section.len()).any(|window| window == section));
    let irep_at = bytes.windows(4).position(|w| w == b"IREP").expect("IREP");
    let dbg_at = bytes.windows(4).position(|w| w == b"DBG\0").expect("DBG");
    let end_at = bytes.windows(4).position(|w| w == b"END\0").expect("END");
    assert!(irep_at < dbg_at && dbg_at < end_at);
}

#[test]
fn without_debug_drops_structured_and_raw() {
    let model = RiteModel {
        root: Irep {
            debug: Some(DebugInfo {
                files: vec![debug_file(&[1])],
            }),
            ..minimal_root()
        },
        ..Default::default()
    };
    let bytes = write_rite(&model);
    assert!(bytes.windows(4).any(|window| window == b"DBG\0"));
    let stripped = without_debug(&bytes).expect("strips");
    assert!(!stripped.windows(4).any(|window| window == b"DBG\0"));
    // Stripped output equals the same tree compiled without debug info.
    let plain = RiteModel {
        root: minimal_root(),
        ..Default::default()
    };
    assert_eq!(stripped, write_rite(&plain));
    // Structured output is stable through a raw round-trip.
    let back = read_rite(&bytes).expect("parses");
    assert!(back
        .debug_raw
        .as_ref()
        .is_some_and(|raw| raw.starts_with(b"DBG\0")));
    assert_eq!(write_rite(&back), bytes);
}
