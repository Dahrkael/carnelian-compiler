//! Opcode table: port of `mrc_ops.h` (`mruby-compiler2 0.5.0`).
//!
//! Discriminants follow the `OPCODE(x, _)` order, so `OP_NOP == 0` through
//! `OP_STOP == 118`.

pub const OP_NOP: u8 = 0;
pub const OP_MOVE: u8 = 1;
pub const OP_LOADL: u8 = 2;
pub const OP_LOADI8: u8 = 3;
pub const OP_LOADINEG: u8 = 4;
pub const OP_LOADI__1: u8 = 5;
pub const OP_LOADI_0: u8 = 6;
pub const OP_LOADI_1: u8 = 7;
pub const OP_LOADI_2: u8 = 8;
pub const OP_LOADI_3: u8 = 9;
pub const OP_LOADI_4: u8 = 10;
pub const OP_LOADI_5: u8 = 11;
pub const OP_LOADI_6: u8 = 12;
pub const OP_LOADI_7: u8 = 13;
pub const OP_LOADI16: u8 = 14;
pub const OP_LOADI32: u8 = 15;
pub const OP_LOADSYM: u8 = 16;
pub const OP_LOADNIL: u8 = 17;
pub const OP_LOADSELF: u8 = 18;
pub const OP_LOADTRUE: u8 = 19;
pub const OP_LOADFALSE: u8 = 20;
pub const OP_GETGV: u8 = 21;
pub const OP_SETGV: u8 = 22;
pub const OP_GETSV: u8 = 23;
pub const OP_SETSV: u8 = 24;
pub const OP_GETIV: u8 = 25;
pub const OP_SETIV: u8 = 26;
pub const OP_GETCV: u8 = 27;
pub const OP_SETCV: u8 = 28;
pub const OP_GETCONST: u8 = 29;
pub const OP_SETCONST: u8 = 30;
pub const OP_GETMCNST: u8 = 31;
pub const OP_SETMCNST: u8 = 32;
pub const OP_GETUPVAR: u8 = 33;
pub const OP_SETUPVAR: u8 = 34;
pub const OP_GETIDX: u8 = 35;
pub const OP_GETIDX0: u8 = 36;
pub const OP_SETIDX: u8 = 37;
pub const OP_JMP: u8 = 38;
pub const OP_JMPIF: u8 = 39;
pub const OP_JMPNOT: u8 = 40;
pub const OP_JMPNIL: u8 = 41;
pub const OP_JMPUW: u8 = 42;
pub const OP_EXCEPT: u8 = 43;
pub const OP_RESCUE: u8 = 44;
pub const OP_RAISEIF: u8 = 45;
pub const OP_MATCHERR: u8 = 46;
pub const OP_SSEND: u8 = 47;
pub const OP_SSEND0: u8 = 48;
pub const OP_SSENDB: u8 = 49;
pub const OP_SEND: u8 = 50;
pub const OP_SEND0: u8 = 51;
pub const OP_SENDB: u8 = 52;
pub const OP_CALL: u8 = 53;
pub const OP_BLKCALL: u8 = 54;
pub const OP_SUPER: u8 = 55;
pub const OP_ARGARY: u8 = 56;
pub const OP_ENTER: u8 = 57;
pub const OP_KEY_P: u8 = 58;
pub const OP_KEYEND: u8 = 59;
pub const OP_KARG: u8 = 60;
pub const OP_RETURN: u8 = 61;
pub const OP_RETURN_BLK: u8 = 62;
pub const OP_RETSELF: u8 = 63;
pub const OP_RETNIL: u8 = 64;
pub const OP_RETTRUE: u8 = 65;
pub const OP_RETFALSE: u8 = 66;
pub const OP_BREAK: u8 = 67;
pub const OP_BLKPUSH: u8 = 68;
pub const OP_ADD: u8 = 69;
pub const OP_ADDI: u8 = 70;
pub const OP_SUB: u8 = 71;
pub const OP_SUBI: u8 = 72;
pub const OP_ADDILV: u8 = 73;
pub const OP_SUBILV: u8 = 74;
pub const OP_MUL: u8 = 75;
pub const OP_DIV: u8 = 76;
pub const OP_EQ: u8 = 77;
pub const OP_LT: u8 = 78;
pub const OP_LE: u8 = 79;
pub const OP_GT: u8 = 80;
pub const OP_GE: u8 = 81;
pub const OP_ARRAY: u8 = 82;
pub const OP_ARRAY2: u8 = 83;
pub const OP_ARYCAT: u8 = 84;
pub const OP_ARYPUSH: u8 = 85;
pub const OP_ARYSPLAT: u8 = 86;
pub const OP_AREF: u8 = 87;
pub const OP_ASET: u8 = 88;
pub const OP_APOST: u8 = 89;
pub const OP_INTERN: u8 = 90;
pub const OP_SYMBOL: u8 = 91;
pub const OP_STRING: u8 = 92;
pub const OP_STRCAT: u8 = 93;
pub const OP_HASH: u8 = 94;
pub const OP_HASHADD: u8 = 95;
pub const OP_HASHCAT: u8 = 96;
pub const OP_LAMBDA: u8 = 97;
pub const OP_BLOCK: u8 = 98;
pub const OP_METHOD: u8 = 99;
pub const OP_RANGE_INC: u8 = 100;
pub const OP_RANGE_EXC: u8 = 101;
pub const OP_OCLASS: u8 = 102;
pub const OP_CLASS: u8 = 103;
pub const OP_MODULE: u8 = 104;
pub const OP_EXEC: u8 = 105;
pub const OP_DEF: u8 = 106;
pub const OP_TDEF: u8 = 107;
pub const OP_SDEF: u8 = 108;
pub const OP_ALIAS: u8 = 109;
pub const OP_UNDEF: u8 = 110;
pub const OP_SCLASS: u8 = 111;
pub const OP_TCLASS: u8 = 112;
pub const OP_DEBUG: u8 = 113;
pub const OP_ERR: u8 = 114;
pub const OP_EXT1: u8 = 115;
pub const OP_EXT2: u8 = 116;
pub const OP_EXT3: u8 = 117;
pub const OP_STOP: u8 = 118;

/// Backward compatibility aliases from `mrc_opcode.h`.
pub const OP_LOADI: u8 = OP_LOADI8;
pub const OP_LOADT: u8 = OP_LOADTRUE;
pub const OP_LOADF: u8 = OP_LOADFALSE;

/// `LAMBDA`/`BLOCK`/`METHOD` operand flags.
pub const OP_L_STRICT: u8 = 1;
pub const OP_L_CAPTURE: u8 = 2;
pub const OP_L_METHOD: u8 = OP_L_STRICT;
pub const OP_L_LAMBDA: u8 = OP_L_STRICT | OP_L_CAPTURE;
pub const OP_L_BLOCK: u8 = OP_L_CAPTURE;

/// Operand shapes, mirroring the `mrc_ops.h` second column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Z,
    B,
    Bb,
    Bbb,
    Bs,
    Bss,
    S,
    W,
}

/// Shape per opcode, in discriminant order.
pub const SHAPES: [Shape; 119] = [
    Shape::Z,   // NOP
    Shape::Bb,  // MOVE
    Shape::Bb,  // LOADL
    Shape::Bb,  // LOADI8
    Shape::Bb,  // LOADINEG
    Shape::B,   // LOADI__1
    Shape::B,   // LOADI_0
    Shape::B,   // LOADI_1
    Shape::B,   // LOADI_2
    Shape::B,   // LOADI_3
    Shape::B,   // LOADI_4
    Shape::B,   // LOADI_5
    Shape::B,   // LOADI_6
    Shape::B,   // LOADI_7
    Shape::Bs,  // LOADI16
    Shape::Bss, // LOADI32
    Shape::Bb,  // LOADSYM
    Shape::B,   // LOADNIL
    Shape::B,   // LOADSELF
    Shape::B,   // LOADTRUE
    Shape::B,   // LOADFALSE
    Shape::Bb,  // GETGV
    Shape::Bb,  // SETGV
    Shape::Bb,  // GETSV
    Shape::Bb,  // SETSV
    Shape::Bb,  // GETIV
    Shape::Bb,  // SETIV
    Shape::Bb,  // GETCV
    Shape::Bb,  // SETCV
    Shape::Bb,  // GETCONST
    Shape::Bb,  // SETCONST
    Shape::Bb,  // GETMCNST
    Shape::Bb,  // SETMCNST
    Shape::Bbb, // GETUPVAR
    Shape::Bbb, // SETUPVAR
    Shape::B,   // GETIDX
    Shape::Bb,  // GETIDX0
    Shape::B,   // SETIDX
    Shape::S,   // JMP
    Shape::Bs,  // JMPIF
    Shape::Bs,  // JMPNOT
    Shape::Bs,  // JMPNIL
    Shape::S,   // JMPUW
    Shape::B,   // EXCEPT
    Shape::Bb,  // RESCUE
    Shape::B,   // RAISEIF
    Shape::B,   // MATCHERR
    Shape::Bbb, // SSEND
    Shape::Bb,  // SSEND0
    Shape::Bbb, // SSENDB
    Shape::Bbb, // SEND
    Shape::Bb,  // SEND0
    Shape::Bbb, // SENDB
    Shape::Z,   // CALL
    Shape::Bb,  // BLKCALL
    Shape::Bb,  // SUPER
    Shape::Bs,  // ARGARY
    Shape::W,   // ENTER
    Shape::Bb,  // KEY_P
    Shape::Z,   // KEYEND
    Shape::Bb,  // KARG
    Shape::B,   // RETURN
    Shape::B,   // RETURN_BLK
    Shape::Z,   // RETSELF
    Shape::Z,   // RETNIL
    Shape::Z,   // RETTRUE
    Shape::Z,   // RETFALSE
    Shape::B,   // BREAK
    Shape::Bs,  // BLKPUSH
    Shape::B,   // ADD
    Shape::Bb,  // ADDI
    Shape::B,   // SUB
    Shape::Bb,  // SUBI
    Shape::Bbb, // ADDILV
    Shape::Bbb, // SUBILV
    Shape::B,   // MUL
    Shape::B,   // DIV
    Shape::B,   // EQ
    Shape::B,   // LT
    Shape::B,   // LE
    Shape::B,   // GT
    Shape::B,   // GE
    Shape::Bb,  // ARRAY
    Shape::Bbb, // ARRAY2
    Shape::B,   // ARYCAT
    Shape::Bb,  // ARYPUSH
    Shape::B,   // ARYSPLAT
    Shape::Bbb, // AREF
    Shape::Bbb, // ASET
    Shape::Bbb, // APOST
    Shape::B,   // INTERN
    Shape::Bb,  // SYMBOL
    Shape::Bb,  // STRING
    Shape::B,   // STRCAT
    Shape::Bb,  // HASH
    Shape::Bb,  // HASHADD
    Shape::B,   // HASHCAT
    Shape::Bb,  // LAMBDA
    Shape::Bb,  // BLOCK
    Shape::Bb,  // METHOD
    Shape::B,   // RANGE_INC
    Shape::B,   // RANGE_EXC
    Shape::B,   // OCLASS
    Shape::Bb,  // CLASS
    Shape::Bb,  // MODULE
    Shape::Bb,  // EXEC
    Shape::Bb,  // DEF
    Shape::Bbb, // TDEF
    Shape::Bbb, // SDEF
    Shape::Bb,  // ALIAS
    Shape::B,   // UNDEF
    Shape::B,   // SCLASS
    Shape::B,   // TCLASS
    Shape::Bbb, // DEBUG
    Shape::B,   // ERR
    Shape::Z,   // EXT1
    Shape::Z,   // EXT2
    Shape::Z,   // EXT3
    Shape::Z,   // STOP
];

/// Decoded instruction with `EXT` prefixes resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decoded {
    /// Real opcode (`EXT` consumed).
    pub insn: u8,
    /// First operand.
    pub a: u16,
    /// Second operand.
    pub b: u16,
    /// Third operand.
    pub cc: u16,
}

/// Instruction size in bytes, excluding the `EXT` prefix byte.
///
/// `ext` is `0` (none) through `3`. The `EXT2` values for `BB`/`BBB` (4/5)
/// mirror a C macro leak: `mrc_insn_size2` never redefines them, so the
/// `EXT1` values stay in effect. The compositions still match the
/// `FETCH_*_2` decoders (`BB` = `B`+`S`, `BBB` = `B`+`S`+`B`).
pub const fn insn_size(op: u8, ext: u8) -> u8 {
    let shape = SHAPES[op as usize];
    match (shape, ext) {
        (Shape::Z, _) => 1,
        (Shape::S, _) | (Shape::W, _) => {
            if matches!(shape, Shape::S) {
                3
            } else {
                4
            }
        }
        (Shape::B, 0) | (Shape::B, 2) => 2,
        (Shape::B, _) => 3,
        (Shape::Bb, 0) => 3,
        (Shape::Bb, 1) | (Shape::Bb, 2) => 4,
        (Shape::Bb, _) => 5,
        (Shape::Bbb, 0) => 4,
        (Shape::Bbb, 1) | (Shape::Bbb, 2) => 5,
        (Shape::Bbb, _) => 6,
        (Shape::Bs, 0) | (Shape::Bs, 2) => 4,
        (Shape::Bs, _) => 5,
        (Shape::Bss, 0) | (Shape::Bss, 2) => 6,
        (Shape::Bss, _) => 7,
    }
}

fn read_b(iseq: &[u8], pc: &mut usize) -> Option<u8> {
    let value = *iseq.get(*pc)?;
    *pc += 1;
    Some(value)
}

fn read_s(iseq: &[u8], pc: &mut usize) -> Option<u16> {
    let hi = read_b(iseq, pc)? as u16;
    let lo = read_b(iseq, pc)? as u16;
    Some((hi << 8) | lo)
}

fn fetch(iseq: &[u8], pc: &mut usize, insn: u8, ext: u8) -> Option<(u16, u16, u16)> {
    let (mut a, mut b, mut cc) = (0, 0, 0);
    // Widened operand positions per EXT level, mirroring FETCH_* macros.
    let wide_a = ext == 1 || ext == 3;
    let wide_b = ext == 2 || ext == 3;
    match SHAPES[insn as usize] {
        Shape::Z => {}
        Shape::B => {
            // NB: FETCH_B_3 is narrow in C despite the EXT3 prefix.
            a = if wide_a && ext != 3 {
                read_s(iseq, pc)?
            } else {
                u16::from(read_b(iseq, pc)?)
            };
        }
        Shape::Bb => {
            a = if wide_a {
                read_s(iseq, pc)?
            } else {
                u16::from(read_b(iseq, pc)?)
            };
            b = if wide_b {
                read_s(iseq, pc)?
            } else {
                u16::from(read_b(iseq, pc)?)
            };
        }
        Shape::Bbb => {
            a = if wide_a {
                read_s(iseq, pc)?
            } else {
                u16::from(read_b(iseq, pc)?)
            };
            b = if wide_b {
                read_s(iseq, pc)?
            } else {
                u16::from(read_b(iseq, pc)?)
            };
            cc = u16::from(read_b(iseq, pc)?);
        }
        Shape::Bs => {
            a = if wide_a {
                read_s(iseq, pc)?
            } else {
                u16::from(read_b(iseq, pc)?)
            };
            b = read_s(iseq, pc)?;
        }
        Shape::Bss => {
            a = if wide_a {
                read_s(iseq, pc)?
            } else {
                u16::from(read_b(iseq, pc)?)
            };
            b = read_s(iseq, pc)?;
            cc = read_s(iseq, pc)?;
        }
        Shape::S => {
            a = read_s(iseq, pc)?;
        }
        Shape::W => {
            let b1 = u32::from(read_b(iseq, pc)?);
            let b2 = u32::from(read_b(iseq, pc)?);
            let b3 = u32::from(read_b(iseq, pc)?);
            a = ((b1 << 16) | (b2 << 8) | b3) as u16;
        }
    }
    Some((a, b, cc))
}

/// Decode the instruction at `pc`, resolving `EXT` prefixes.
/// Returns the decoded instruction and the next pc.
pub fn decode_at(iseq: &[u8], pc: usize) -> Option<(Decoded, usize)> {
    let mut cursor = pc;
    let mut insn = read_b(iseq, &mut cursor)?;
    let mut ext = 0;
    if insn == OP_EXT1 || insn == OP_EXT2 || insn == OP_EXT3 {
        ext = insn - OP_EXT1 + 1;
        insn = read_b(iseq, &mut cursor)?;
    }
    let (a, b, cc) = fetch(iseq, &mut cursor, insn, ext)?;
    Some((Decoded { insn, a, b, cc }, cursor))
}
