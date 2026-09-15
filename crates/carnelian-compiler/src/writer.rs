//! RITE writer: pure-Rust port of `dump.c` (`mrc_dump_irep`).
//!
//! Encoding rules mirror the C writer so output is byte-identical:
//! big-endian headers, `INT64` normalized to `INT32` when it fits,
//! floats as little-endian IEEE754, `LVAR` names without NUL.

use crate::irep::{Irep, PoolValue, RiteModel};

const BINARY_IDENT: &[u8; 4] = b"RITE";
const BINARY_MAJOR: &[u8; 2] = b"04";
const BINARY_MINOR: &[u8; 2] = b"00";
const COMPILER_NAME: &[u8; 4] = b"HSMK";
const COMPILER_VERSION: &[u8; 4] = b"0000";
const IREP_IDENT: &[u8; 4] = b"IREP";
const RITE_VERSION: &[u8; 4] = b"0400";
const LVAR_IDENT: &[u8; 4] = b"LVAR";
const DEBUG_IDENT: &[u8; 4] = b"DBG\0";
const END_IDENT: &[u8; 4] = b"END\0";
const NULL_SYM_LEN: u16 = 0xFFFF;
const LV_NULL_MARK: u16 = 0xFFFF;

fn push_u8(out: &mut Vec<u8>, value: u8) {
    out.push(value);
}

fn push_u16be(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn push_u32be(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn pool_entry_size(entry: &PoolValue) -> usize {
    match entry {
        PoolValue::Int32(_) => 1 + 4,
        PoolValue::Int64(value) => {
            if *value < i64::from(i32::MIN) || *value > i64::from(i32::MAX) {
                1 + 8
            } else {
                1 + 4
            }
        }
        PoolValue::Float(_) => 1 + 8,
        PoolValue::BigInt(raw) => 1 + raw.len(),
        PoolValue::Str(bytes) | PoolValue::SStr(bytes) => 1 + 2 + bytes.len() + 1,
    }
}

fn pool_block_size(irep: &Irep) -> usize {
    2 + irep.pool.iter().map(pool_entry_size).sum::<usize>()
}

fn syms_block_size(irep: &Irep) -> usize {
    2 + irep
        .syms
        .iter()
        .map(|sym| match sym {
            None => 2,
            Some(name) => 2 + name.len() + 1,
        })
        .sum::<usize>()
}

fn iseq_block_size(irep: &Irep) -> usize {
    2 + 4 + irep.iseq.len() + irep.catch_handlers.len() * 13
}

/// Own record size, excluding children: this is what the size field carries
/// (`dump.c: write_irep_header` uses `get_irep_record_size_1`).
fn irep_own_size(irep: &Irep) -> usize {
    // Header: record size (4) + nlocals/nregs/rlen (3x2).
    let header = 4 + 2 + 2 + 2;
    header + iseq_block_size(irep) + pool_block_size(irep) + syms_block_size(irep)
}

fn irep_record_size(irep: &Irep) -> usize {
    irep_own_size(irep) + irep.reps.iter().map(irep_record_size).sum::<usize>()
}

fn lv_records_size(irep: &Irep) -> usize {
    let own = (irep.nlocals.saturating_sub(1) as usize) * 2;
    own + irep.reps.iter().map(lv_records_size).sum::<usize>()
}

fn write_pool_block(out: &mut Vec<u8>, irep: &Irep) {
    push_u16be(out, irep.pool.len() as u16);
    for entry in &irep.pool {
        match entry {
            PoolValue::Int32(value) => {
                push_u8(out, 1);
                push_u32be(out, *value as u32);
            }
            PoolValue::Int64(value) => {
                if *value < i64::from(i32::MIN) || *value > i64::from(i32::MAX) {
                    push_u8(out, 3);
                    push_u32be(out, ((value >> 32) & 0xffff_ffff) as u32);
                    push_u32be(out, (value & 0xffff_ffff) as u32);
                } else {
                    push_u8(out, 1);
                    push_u32be(out, (*value as i32) as u32);
                }
            }
            PoolValue::Float(value) => {
                push_u8(out, 5);
                out.extend_from_slice(&value.to_le_bytes());
            }
            PoolValue::BigInt(raw) => {
                push_u8(out, 7);
                out.extend_from_slice(raw);
            }
            PoolValue::Str(bytes) => {
                push_u8(out, 0);
                push_u16be(out, bytes.len() as u16);
                out.extend_from_slice(bytes);
                push_u8(out, 0);
            }
            PoolValue::SStr(bytes) => {
                push_u8(out, 2);
                push_u16be(out, bytes.len() as u16);
                out.extend_from_slice(bytes);
                push_u8(out, 0);
            }
        }
    }
}

fn write_syms_block(out: &mut Vec<u8>, irep: &Irep) {
    push_u16be(out, irep.syms.len() as u16);
    for sym in &irep.syms {
        match sym {
            None => push_u16be(out, NULL_SYM_LEN),
            Some(name) => {
                push_u16be(out, name.len() as u16);
                out.extend_from_slice(name);
                push_u8(out, 0);
            }
        }
    }
}

fn write_irep_record(out: &mut Vec<u8>, irep: &Irep) {
    push_u32be(out, irep_own_size(irep) as u32);
    push_u16be(out, irep.nlocals);
    push_u16be(out, irep.nregs);
    push_u16be(out, irep.reps.len() as u16);

    push_u16be(out, irep.catch_handlers.len() as u16);
    push_u32be(out, irep.iseq.len() as u32);
    out.extend_from_slice(&irep.iseq);
    for handler in &irep.catch_handlers {
        push_u8(out, handler.kind);
        push_u32be(out, handler.begin);
        push_u32be(out, handler.end);
        push_u32be(out, handler.target);
    }

    write_pool_block(out, irep);
    write_syms_block(out, irep);

    for child in &irep.reps {
        write_irep_record(out, child);
    }
}

fn write_lv_records(out: &mut Vec<u8>, irep: &Irep) {
    for slot in &irep.lv {
        match slot {
            None => push_u16be(out, LV_NULL_MARK),
            Some(index) => push_u16be(out, *index as u16),
        }
    }
    for child in &irep.reps {
        write_lv_records(out, child);
    }
}

/// Emit a full RITE binary, byte-identical to `dump.c` for the same model.
#[must_use]
pub fn write_rite(model: &RiteModel) -> Vec<u8> {
    let irep_section_size = 12 + irep_record_size(&model.root);
    // Structured debug (codegen) takes the `dump.c` encode path; otherwise
    // preserved raw bytes go out verbatim (reader round-trips).
    let debug_built = crate::debug::encode_debug_section(&model.root);
    let debug_bytes = debug_built.as_deref().or(model.debug_raw.as_deref());
    let debug_size = debug_bytes.map_or(0, <[u8]>::len);
    let lvar_size = model.lvar_syms.as_ref().map_or(0, |syms| {
        8 + 4 + syms.iter().map(|name| 2 + name.len()).sum::<usize>() + lv_records_size(&model.root)
    });
    let binary_size = 20 + irep_section_size + debug_size + lvar_size + 8;

    let mut out = Vec::with_capacity(binary_size);

    out.extend_from_slice(BINARY_IDENT);
    out.extend_from_slice(BINARY_MAJOR);
    out.extend_from_slice(BINARY_MINOR);
    push_u32be(&mut out, binary_size as u32);
    out.extend_from_slice(COMPILER_NAME);
    out.extend_from_slice(COMPILER_VERSION);

    out.extend_from_slice(IREP_IDENT);
    push_u32be(&mut out, irep_section_size as u32);
    out.extend_from_slice(RITE_VERSION);
    write_irep_record(&mut out, &model.root);

    // `dump.c` order is IREP, DEBUG, LVAR, END.
    if let Some(raw) = debug_bytes {
        debug_assert!(raw.starts_with(DEBUG_IDENT));
        out.extend_from_slice(raw);
    }
    if let Some(syms) = &model.lvar_syms {
        out.extend_from_slice(LVAR_IDENT);
        let section_size = 8
            + 4
            + syms.iter().map(|name| 2 + name.len()).sum::<usize>()
            + lv_records_size(&model.root);
        push_u32be(&mut out, section_size as u32);
        push_u32be(&mut out, syms.len() as u32);
        for name in syms {
            push_u16be(&mut out, name.len() as u16);
            out.extend_from_slice(name);
        }
        write_lv_records(&mut out, &model.root);
    }

    out.extend_from_slice(END_IDENT);
    push_u32be(&mut out, 8);

    debug_assert_eq!(out.len(), binary_size);
    out
}

/// Parse `input` with the local reader and re-emit it (P0 round-trip).
pub fn roundtrip(input: &[u8]) -> Result<Vec<u8>, crate::reader::ReadError> {
    let model = crate::reader::read_rite(input)?;
    Ok(write_rite(&model))
}
