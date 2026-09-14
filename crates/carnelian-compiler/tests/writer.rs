//! Writer/reader unit tests over synthetic models (no C reference needed).

use carnelian_compiler::{read_rite, write_rite, Irep, PoolValue, RiteModel};

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
