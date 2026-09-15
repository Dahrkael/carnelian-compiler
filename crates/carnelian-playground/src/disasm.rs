//! Bytecode disassembler for the playground viewer.
//!
//! Presentation-only over `decode_at`: an `OP_NAMES` table plus a
//! per-`Shape` operand formatter. The backend is untouched.

use carnelian_compiler::opcode::{self, Decoded, Shape, SHAPES};
use carnelian_compiler::{Irep, PoolValue, RiteModel};

/// Opcode names in discriminant order (`OP_NOP == 0` .. `OP_STOP == 118`).
pub const OP_NAMES: [&str; 119] = [
    "NOP",
    "MOVE",
    "LOADL",
    "LOADI8",
    "LOADINEG",
    "LOADI__1",
    "LOADI_0",
    "LOADI_1",
    "LOADI_2",
    "LOADI_3",
    "LOADI_4",
    "LOADI_5",
    "LOADI_6",
    "LOADI_7",
    "LOADI16",
    "LOADI32",
    "LOADSYM",
    "LOADNIL",
    "LOADSELF",
    "LOADTRUE",
    "LOADFALSE",
    "GETGV",
    "SETGV",
    "GETSV",
    "SETSV",
    "GETIV",
    "SETIV",
    "GETCV",
    "SETCV",
    "GETCONST",
    "SETCONST",
    "GETMCNST",
    "SETMCNST",
    "GETUPVAR",
    "SETUPVAR",
    "GETIDX",
    "GETIDX0",
    "SETIDX",
    "JMP",
    "JMPIF",
    "JMPNOT",
    "JMPNIL",
    "JMPUW",
    "EXCEPT",
    "RESCUE",
    "RAISEIF",
    "MATCHERR",
    "SSEND",
    "SSEND0",
    "SSENDB",
    "SEND",
    "SEND0",
    "SENDB",
    "CALL",
    "BLKCALL",
    "SUPER",
    "ARGARY",
    "ENTER",
    "KEY_P",
    "KEYEND",
    "KARG",
    "RETURN",
    "RETURN_BLK",
    "RETSELF",
    "RETNIL",
    "RETTRUE",
    "RETFALSE",
    "BREAK",
    "BLKPUSH",
    "ADD",
    "ADDI",
    "SUB",
    "SUBI",
    "ADDILV",
    "SUBILV",
    "MUL",
    "DIV",
    "EQ",
    "LT",
    "LE",
    "GT",
    "GE",
    "ARRAY",
    "ARRAY2",
    "ARYCAT",
    "ARYPUSH",
    "ARYSPLAT",
    "AREF",
    "ASET",
    "APOST",
    "INTERN",
    "SYMBOL",
    "STRING",
    "STRCAT",
    "HASH",
    "HASHADD",
    "HASHCAT",
    "LAMBDA",
    "BLOCK",
    "METHOD",
    "RANGE_INC",
    "RANGE_EXC",
    "OCLASS",
    "CLASS",
    "MODULE",
    "EXEC",
    "DEF",
    "TDEF",
    "SDEF",
    "ALIAS",
    "UNDEF",
    "SCLASS",
    "TCLASS",
    "DEBUG",
    "ERR",
    "EXT1",
    "EXT2",
    "EXT3",
    "STOP",
];

/// Name for one opcode byte.
pub fn op_name(op: u8) -> &'static str {
    debug_assert_op_names();
    OP_NAMES.get(op as usize).copied().unwrap_or("???")
}

/// Pin the hand table against the backend constants (drift fails fast).
fn debug_assert_op_names() {
    debug_assert_eq!(OP_NAMES[opcode::OP_NOP as usize], "NOP");
    debug_assert_eq!(OP_NAMES[opcode::OP_SEND as usize], "SEND");
    debug_assert_eq!(OP_NAMES[opcode::OP_SENDB as usize], "SENDB");
    debug_assert_eq!(OP_NAMES[opcode::OP_RETURN as usize], "RETURN");
    debug_assert_eq!(OP_NAMES[opcode::OP_STOP as usize], "STOP");
    debug_assert_eq!(OP_NAMES.len(), 119);
}

/// Format operands per shape (`a` is 32 bits like `mrc_insn_data`).
pub fn format_operands(decoded: &Decoded) -> String {
    let shape = SHAPES
        .get(decoded.insn as usize)
        .copied()
        .unwrap_or(Shape::Z);
    match shape {
        Shape::Z => String::new(),
        Shape::B => format!("r{}", decoded.a),
        Shape::Bb => format!("r{}, {:#04x}", decoded.a, decoded.b),
        Shape::Bbb => format!("r{}, {:#04x}, {:#04x}", decoded.a, decoded.b, decoded.cc),
        Shape::Bs => format!("r{}, {:#06x}", decoded.a, decoded.b),
        Shape::Bss => format!("r{}, {:#06x}, {:#06x}", decoded.a, decoded.b, decoded.cc),
        Shape::S => format!("{:#06x}", decoded.a),
        Shape::W => format!("{:#08x}", decoded.a),
    }
}

fn pool_line(value: &PoolValue) -> String {
    match value {
        PoolValue::Str(bytes) | PoolValue::SStr(bytes) => {
            format!("str {:?}", String::from_utf8_lossy(bytes))
        }
        PoolValue::Int32(v) => format!("int32 {v}"),
        PoolValue::Int64(v) => format!("int64 {v}"),
        PoolValue::Float(v) => format!("float {v}"),
        PoolValue::BigInt(raw) => format!("bigint {} bytes (gated)", raw.len()),
    }
}

fn sym_line(sym: &Option<Vec<u8>>) -> String {
    match sym {
        Some(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        None => "(null)".to_string(),
    }
}

fn write_irep(out: &mut String, irep: &Irep, depth: usize, index: &mut usize) {
    let id = *index;
    *index += 1;
    let pad = "  ".repeat(depth);
    out.push_str(&format!(
        "{pad}irep{id}: nlocals={} nregs={} pool={} syms={} reps={}\n",
        irep.nlocals,
        irep.nregs,
        irep.pool.len(),
        irep.syms.len(),
        irep.reps.len()
    ));
    for (i, value) in irep.pool.iter().enumerate() {
        out.push_str(&format!("{pad}  pool[{i}] {}\n", pool_line(value)));
    }
    for (i, sym) in irep.syms.iter().enumerate() {
        out.push_str(&format!("{pad}  sym[{i}] {}\n", sym_line(sym)));
    }
    let mut pc = 0;
    while pc < irep.iseq.len() {
        match opcode::decode_at(&irep.iseq, pc) {
            Some((decoded, next)) => {
                let operands = format_operands(&decoded);
                if operands.is_empty() {
                    out.push_str(&format!("{pad}  {pc:04}: {}\n", op_name(decoded.insn)));
                } else {
                    out.push_str(&format!(
                        "{pad}  {pc:04}: {} {operands}\n",
                        op_name(decoded.insn)
                    ));
                }
                if next <= pc {
                    break;
                }
                pc = next;
            }
            None => {
                out.push_str(&format!("{pad}  {pc:04}: <truncated>\n"));
                break;
            }
        }
    }
    for child in &irep.reps {
        write_irep(out, child, depth + 1, index);
    }
}

/// Disassemble a whole RITE model.
pub fn disassemble(model: &RiteModel) -> String {
    let mut out = String::new();
    let mut index = 0;
    write_irep(&mut out, &model.root, 0, &mut index);
    out
}
