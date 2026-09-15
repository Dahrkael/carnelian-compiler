//! Debug section: pure-Rust port of `debug.c` plus the `DBG` half of
//! `dump.c` (`get_debug_record_size`, `write_debug_record`,
//! `write_section_debug`, `debug_info_defined_p`).
//!
//! Only the packed line map (`mrc_debug_line_packed_map = 2`) is emitted;
//! that is all `mrc_debug_info_append_file` ever produces.

use crate::irep::Irep;

/// Re-emit `bytes` without the `DBG` section: parses with the reader and
/// drops both raw and structured debug before writing back. `verify` and
/// the parity suites compare these bytes until the pinned reference emits
/// debug info itself.
pub fn without_debug(bytes: &[u8]) -> Result<Vec<u8>, crate::reader::ReadError> {
    let mut model = crate::reader::read_rite(bytes)?;
    model.debug_raw = None;
    clear_debug(&mut model.root);
    Ok(crate::writer::write_rite(&model))
}

/// Line-table type tag on the wire (`mrc_debug_line_packed_map`).
const LINE_TYPE_PACKED_MAP: u8 = 2;

/// Packed-int width (`mrc_packed_int_len`).
fn packed_int_len(mut num: u32) -> usize {
    let mut len = 0;
    loop {
        len += 1;
        num >>= 7;
        if num == 0 {
            break;
        }
    }
    len
}

/// Packed-int encoding (`mrc_packed_int_encode`): 7 bits per byte, low
/// first, high bit marks continuation.
fn packed_int_encode(mut num: u32, out: &mut Vec<u8>) {
    loop {
        let mut byte = (num & 0x7f) as u8;
        num >>= 7;
        if num != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if num == 0 {
            break;
        }
    }
}

/// Pack one file's line table (`mrc_debug_info_append_file`): runs of equal
/// lines collapse; each run emits the pc delta then the line delta, both
/// seeded from zero with absolute pcs and wrapping `u16` line arithmetic.
#[must_use]
pub fn pack_line_map(start_pos: u32, lines: &[u16]) -> Vec<u8> {
    // Two passes like C (`packed_size` first, then the bytes).
    let mut size = 0;
    let mut prev_line: u16 = 0;
    let mut prev_pc: u32 = 0;
    for (index, line) in lines.iter().enumerate() {
        if *line == prev_line {
            continue;
        }
        let pc = start_pos + index as u32;
        size += packed_int_len(pc - prev_pc);
        prev_pc = pc;
        size += packed_int_len(u32::from(line.wrapping_sub(prev_line)));
        prev_line = *line;
    }
    let mut packed = Vec::with_capacity(size);
    prev_line = 0;
    prev_pc = 0;
    for (index, line) in lines.iter().enumerate() {
        if *line == prev_line {
            continue;
        }
        let pc = start_pos + index as u32;
        packed_int_encode(pc - prev_pc, &mut packed);
        prev_pc = pc;
        packed_int_encode(u32::from(line.wrapping_sub(prev_line)), &mut packed);
        prev_line = *line;
    }
    debug_assert_eq!(packed.len(), size);
    packed
}

/// True when every irep in the tree carries debug info (port of
/// `debug_info_defined_p`; `dump.c` omits `DBG` otherwise).
#[must_use]
pub fn debug_defined(root: &Irep) -> bool {
    if root.debug.is_none() {
        return false;
    }
    root.reps.iter().all(debug_defined)
}

/// Clear structured debug on the whole tree (stripped mode).
pub fn clear_debug(irep: &mut Irep) {
    irep.debug = None;
    for child in &mut irep.reps {
        clear_debug(child);
    }
}

/// Filename table in first-seen preorder (like `get_filename_table_size`).
fn filenames(root: &Irep, table: &mut Vec<Vec<u8>>) {
    if let Some(debug) = &root.debug {
        for file in &debug.files {
            if !table.contains(&file.filename) {
                table.push(file.filename.clone());
            }
        }
    }
    for child in &root.reps {
        filenames(child, table);
    }
}

fn filename_index(table: &[Vec<u8>], name: &[u8]) -> u16 {
    table
        .iter()
        .position(|known| known == name)
        .map(|index| index as u16)
        .expect("filename registered in preorder")
}

/// Own debug record size, excluding children (like `get_debug_record_size`
/// for one irep: size field + file count + per-file entries).
fn record_own_size(packed: &[Vec<u8>]) -> usize {
    let mut size = 4 + 2;
    for entry in packed {
        size += 4 + 2 + 4 + 1 + entry.len();
    }
    size
}

/// Encode one irep's debug record plus children preorder (like
/// `write_debug_record_1` + `write_debug_record`).
fn write_record(out: &mut Vec<u8>, irep: &Irep, table: &[Vec<u8>]) {
    let debug = irep.debug.as_ref().expect("debug defined");
    let packed: Vec<Vec<u8>> = debug
        .files
        .iter()
        .map(|file| pack_line_map(file.start_pos, &file.lines))
        .collect();
    let own = record_own_size(&packed);
    out.extend_from_slice(&(own as u32).to_be_bytes());
    out.extend_from_slice(&(debug.files.len() as u16).to_be_bytes());
    for (file, entry) in debug.files.iter().zip(packed.iter()) {
        out.extend_from_slice(&file.start_pos.to_be_bytes());
        out.extend_from_slice(&filename_index(table, &file.filename).to_be_bytes());
        out.extend_from_slice(&(entry.len() as u32).to_be_bytes());
        out.push(LINE_TYPE_PACKED_MAP);
        out.extend_from_slice(entry);
    }
    for child in &irep.reps {
        write_record(out, child, table);
    }
}

/// Encode the full `DBG` section (header included), or `None` when any
/// irep lacks debug info. Mirrors `write_section_debug`.
#[must_use]
pub fn encode_debug_section(root: &Irep) -> Option<Vec<u8>> {
    if !debug_defined(root) {
        return None;
    }
    let mut table = Vec::new();
    filenames(root, &mut table);
    let mut out = Vec::new();
    out.extend_from_slice(b"DBG\0");
    out.extend_from_slice(&[0, 0, 0, 0]); // size placeholder
    out.extend_from_slice(&(table.len() as u16).to_be_bytes());
    for name in &table {
        out.extend_from_slice(&(name.len() as u16).to_be_bytes());
        out.extend_from_slice(name);
    }
    write_record(&mut out, root, &table);
    let size = out.len() as u32;
    out[4..8].copy_from_slice(&size.to_be_bytes());
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_int_vectors() {
        let mut out = Vec::new();
        packed_int_encode(0, &mut out);
        assert_eq!(out, vec![0x00]);
        out.clear();
        packed_int_encode(127, &mut out);
        assert_eq!(out, vec![0x7f]);
        out.clear();
        packed_int_encode(128, &mut out);
        assert_eq!(out, vec![0x80, 0x01]);
        out.clear();
        packed_int_encode(300, &mut out);
        assert_eq!(out, vec![0xac, 0x02]);
        assert_eq!(packed_int_len(0), 1);
        assert_eq!(packed_int_len(300), 2);
    }
}
