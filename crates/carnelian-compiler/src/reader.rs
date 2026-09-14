//! Minimal RITE reader for round-trips (port of the `dump.c` layout).
//!
//! Only `RITE0400` is accepted. `DBG` sections are preserved as raw bytes so
//! the writer can re-emit them verbatim; `IREP` and `LVAR` are modeled.

use crate::irep::{CatchHandler, Irep, PoolValue, RiteModel};

/// Reader failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// Input is shorter than the header claims.
    TooShort,
    /// Not a `RITE` binary or not version `0400`.
    InvalidHeader,
    /// Unknown section identifier.
    InvalidSection,
    /// Unknown pool type byte.
    UnknownPoolType(u8),
    /// LVAR data does not match the irep tree.
    InvalidLvar,
}

impl core::fmt::Display for ReadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort => write!(f, "RITE input is truncated"),
            Self::InvalidHeader => write!(f, "not a RITE0400 binary"),
            Self::InvalidSection => write!(f, "invalid RITE section"),
            Self::UnknownPoolType(t) => write!(f, "unknown pool type {t}"),
            Self::InvalidLvar => write!(f, "invalid LVAR section"),
        }
    }
}

impl std::error::Error for ReadError {}

fn u16be(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[0], bytes[1]])
}

fn u32be(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn take<'a>(input: &mut &'a [u8], len: usize) -> Result<&'a [u8], ReadError> {
    if input.len() < len {
        return Err(ReadError::TooShort);
    }
    let (head, tail) = input.split_at(len);
    *input = tail;
    Ok(head)
}

struct Cursor<'a> {
    rest: &'a [u8],
}

impl<'a> Cursor<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { rest: input }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], ReadError> {
        take(&mut self.rest, len)
    }

    fn u8(&mut self) -> Result<u8, ReadError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, ReadError> {
        Ok(u16be(self.take(2)?))
    }

    fn u32(&mut self) -> Result<u32, ReadError> {
        Ok(u32be(self.take(4)?))
    }
}

/// Parse a full RITE binary into an owned model.
pub fn read_rite(input: &[u8]) -> Result<RiteModel, ReadError> {
    let mut cursor = Cursor::new(input);

    let ident = cursor.take(4)?;
    if ident != b"RITE" {
        return Err(ReadError::InvalidHeader);
    }
    let major = cursor.take(2)?;
    let minor = cursor.take(2)?;
    if major != b"04" || minor != b"00" {
        return Err(ReadError::InvalidHeader);
    }
    let binary_size = cursor.u32()? as usize;
    if binary_size != input.len() {
        return Err(ReadError::TooShort);
    }
    let _compiler_name = cursor.take(4)?;
    let _compiler_version = cursor.take(4)?;

    let mut root: Option<Irep> = None;
    let mut lvar_syms: Option<Vec<Vec<u8>>> = None;
    let mut lvar_records: Option<&[u8]> = None;
    let mut debug_raw: Option<Vec<u8>> = None;

    while !cursor.rest.is_empty() {
        let section_start = input.len() - cursor.rest.len();
        let ident = cursor.take(4)?;
        let size = cursor.u32()? as usize;
        if size < 8 || section_start + size > input.len() {
            return Err(ReadError::TooShort);
        }
        let body_len = size - 8;
        match ident {
            b"IREP" => {
                let body = cursor.take(body_len)?;
                let mut body_cursor = Cursor::new(body);
                let rite_version = body_cursor.take(4)?;
                if rite_version != b"0400" {
                    return Err(ReadError::InvalidHeader);
                }
                let mut records = body_cursor.take(body_cursor.rest.len())?;
                let parsed = read_irep_record(&mut records)?;
                if !records.is_empty() {
                    return Err(ReadError::InvalidSection);
                }
                root = Some(parsed);
            }
            b"LVAR" => {
                let body = cursor.take(body_len)?;
                let mut body_cursor = Cursor::new(body);
                let syms_len = body_cursor.u32()? as usize;
                let mut syms = Vec::with_capacity(syms_len);
                for _ in 0..syms_len {
                    let len = body_cursor.u16()? as usize;
                    syms.push(body_cursor.take(len)?.to_vec());
                }
                lvar_syms = Some(syms);
                lvar_records = Some(body_cursor.rest);
            }
            b"DBG\0" => {
                let body_start = section_start;
                let body_end = section_start + size;
                debug_raw = Some(input[body_start..body_end].to_vec());
                let _skip = cursor.take(body_len)?;
            }
            b"END\0" => {
                if body_len != 0 {
                    return Err(ReadError::InvalidSection);
                }
            }
            _ => return Err(ReadError::InvalidSection),
        }
    }

    let mut root = root.ok_or(ReadError::InvalidSection)?;
    if let (Some(syms), Some(records)) = (lvar_syms.as_ref(), lvar_records) {
        let mut record_cursor = Cursor::new(records);
        assign_lv(&mut root, syms, &mut record_cursor)?;
        if !record_cursor.rest.is_empty() {
            return Err(ReadError::InvalidLvar);
        }
    }
    if lvar_syms.is_some() && lvar_records.is_none() {
        return Err(ReadError::InvalidLvar);
    }

    Ok(RiteModel {
        root,
        lvar_syms,
        debug_raw,
    })
}

fn read_irep_record(input: &mut &[u8]) -> Result<Irep, ReadError> {
    let mut cursor = Cursor::new(input);
    let record_size = cursor.u32()? as usize;
    if record_size < 10 || record_size > cursor.rest.len() + 4 {
        return Err(ReadError::TooShort);
    }
    // The size counts its own 4-byte field (`dump.c: get_irep_record_size_1`
    // includes the 10-byte header of size + nlocals/nregs/rlen).
    let record_body = cursor.take(record_size - 4)?;
    // Children follow the own record inline in the outer input (the size
    // field covers the own record only: `dump.c: get_irep_record_size_1`).
    *input = cursor.rest;

    let mut body = Cursor::new(record_body);
    let nlocals = body.u16()?;
    let nregs = body.u16()?;
    let rlen = body.u16()? as usize;
    let clen = body.u16()?;
    let ilen = body.u32()? as usize;
    let iseq = body.take(ilen)?.to_vec();

    let mut catch_handlers = Vec::with_capacity(clen as usize);
    for _ in 0..clen {
        let kind = body.u8()?;
        let begin = body.u32()?;
        let end = body.u32()?;
        let target = body.u32()?;
        catch_handlers.push(CatchHandler {
            kind,
            begin,
            end,
            target,
        });
    }

    let plen = body.u16()? as usize;
    let mut pool = Vec::with_capacity(plen);
    for _ in 0..plen {
        let kind = body.u8()?;
        match kind {
            0 => {
                let len = body.u16()? as usize;
                let bytes = body.take(len)?;
                let nul = body.u8()?;
                if nul != 0 {
                    return Err(ReadError::InvalidSection);
                }
                pool.push(PoolValue::Str(bytes.to_vec()));
            }
            1 => {
                let value = body.u32()? as i32;
                pool.push(PoolValue::Int32(value));
            }
            2 => {
                let len = body.u16()? as usize;
                let bytes = body.take(len)?;
                let nul = body.u8()?;
                if nul != 0 {
                    return Err(ReadError::InvalidSection);
                }
                pool.push(PoolValue::SStr(bytes.to_vec()));
            }
            3 => {
                let hi = body.u32()? as i64;
                let lo = body.u32()? as i64;
                pool.push(PoolValue::Int64((hi << 32) | (lo & 0xffff_ffff)));
            }
            5 => {
                let bytes = body.take(8)?;
                let mut raw = [0u8; 8];
                raw.copy_from_slice(bytes);
                pool.push(PoolValue::Float(f64::from_le_bytes(raw)));
            }
            7 => {
                // `dump.c` writes `str[0] + 2` bytes: an unsigned length
                // byte followed by `str[0] + 1` payload bytes. Preserve them
                // verbatim for byte-exact re-emission.
                let len = body.u8()?;
                let rest = body.take(len as usize + 1)?;
                let mut raw = Vec::with_capacity(len as usize + 2);
                raw.push(len);
                raw.extend_from_slice(rest);
                pool.push(PoolValue::BigInt(raw));
            }
            other => return Err(ReadError::UnknownPoolType(other)),
        }
    }

    let slen = body.u16()? as usize;
    let mut syms = Vec::with_capacity(slen);
    for _ in 0..slen {
        let len = body.u16()?;
        if len == 0xFFFF {
            syms.push(None);
        } else {
            let bytes = body.take(len as usize)?;
            let nul = body.u8()?;
            if nul != 0 {
                return Err(ReadError::InvalidSection);
            }
            syms.push(Some(bytes.to_vec()));
        }
    }

    // Child records follow inline in the outer input; the own body must be
    // fully consumed here.
    if !body.rest.is_empty() {
        return Err(ReadError::InvalidSection);
    }
    let mut reps = Vec::with_capacity(rlen);
    for _ in 0..rlen {
        reps.push(read_irep_record(input)?);
    }

    Ok(Irep {
        nlocals,
        nregs,
        iseq,
        catch_handlers,
        pool,
        syms,
        reps,
        lv: Vec::new(),
    })
}

fn assign_lv(irep: &mut Irep, syms: &[Vec<u8>], records: &mut Cursor<'_>) -> Result<(), ReadError> {
    if irep.nlocals == 0 {
        return Err(ReadError::InvalidLvar);
    }
    irep.lv.clear();
    for _ in 0..(irep.nlocals as usize - 1) {
        let index = records.u16()?;
        if index == 0xFFFF {
            irep.lv.push(None);
        } else {
            if (index as usize) >= syms.len() {
                return Err(ReadError::InvalidLvar);
            }
            irep.lv.push(Some(u32::from(index)));
        }
    }
    for child in &mut irep.reps {
        assign_lv(child, syms, records)?;
    }
    Ok(())
}
