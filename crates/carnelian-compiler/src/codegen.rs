//! Codegen core: port of scope/register/emission/peephole/pool logic from
//! `codegen.c` (`mruby-compiler2 0.5.0`). Node handlers live in `handlers.rs`.
//!
//! C long-lived arenas become owned `Vec`s; `MRC_THROW` unwinding becomes
//! `Result<_, Diagnostic>`. Behaviour (bytes, limits, messages) is unchanged.

use carnelian_ast::{SymbolId, SymbolPool};

use crate::diagnostics::Diagnostic;
use crate::irep::{CatchHandler, Irep, PoolValue};
use crate::opcode::{self, Decoded};

/// Jump chain empty marker (`JMPLINK_START`).
pub const JMPLINK_START: u32 = u32::MAX;

/// Codegen-side pool value (unpacked; packed at `scope_finish`).
#[derive(Debug, Clone, PartialEq)]
pub enum CgPool {
    /// String bytes.
    Str(Vec<u8>),
    /// 32-bit integer.
    Int32(i32),
    /// 64-bit integer (always created; dedup only matches `Int32`).
    Int64(i64),
    /// Float.
    Float(f64),
    /// Bigint raw bytes: length byte, sign byte, digits (no NUL).
    BigInt(Vec<u8>),
}

/// Loop kinds (`enum looptype`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopType {
    Normal,
    Block,
    For,
    Begin,
    Rescue,
}

/// Open loop frame (`struct loopinfo`).
#[derive(Debug, Clone)]
pub struct LoopFrame {
    /// Loop kind.
    pub kind: LoopType,
    /// `next` destination.
    pub pc0: u32,
    /// `redo` destination.
    pub pc1: u32,
    /// `break` destination chain.
    pub pc2: u32,
    /// Destination register (`-1` when the value is discarded).
    pub reg: i32,
}

/// Per-compilation shared state (the `mrc_ccontext` codegen side).
#[derive(Debug, Default)]
pub struct Session {
    /// Global symbol interning (insertion order).
    pub symbols: SymbolPool,
    /// `LVAR` symbol table in dump order (see `register_lv_names`).
    pub lvar_names: Vec<Vec<u8>>,
    /// Any scope owned locals (drives `LVAR` section presence, like `lv_defined_p`).
    pub any_lv: bool,
    /// Optimizer disabled flag (always false, as in the default context).
    pub no_optimize: bool,
    /// `EXT` opcodes prohibited flag (always false).
    pub no_ext_ops: bool,
}

/// One lexical scope (`mrc_codegen_scope`, trimmed to P1 needs).
#[derive(Debug)]
pub struct Scope {
    /// Stack pointer (next free register).
    pub sp: u16,
    /// Write cursor into `iseq`.
    pub pc: u32,
    /// Start of the last emitted instruction.
    pub lastpc: u32,
    /// Most recent jump target (peephole barrier).
    pub lastlabel: u32,
    /// Emitted bytecode.
    pub iseq: Vec<u8>,
    /// Constant pool under construction.
    pub pool: Vec<CgPool>,
    /// Interned symbols referenced by this scope.
    pub syms: Vec<SymbolId>,
    /// Symbol capacity doubler (`scapa`, starts at 256).
    pub scapa: usize,
    /// Finished child ireps.
    pub reps: Vec<Irep>,
    /// Catch handlers under construction.
    pub catch_table: Vec<CatchHandler>,
    /// Open loops.
    pub loops: Vec<LoopFrame>,
    /// Local variable names (`None` entry never occurs from the parser).
    pub lv_names: Vec<Vec<u8>>,
    /// Number of locals (registers below `sp` reserved at entry).
    pub nlocals: u16,
    /// High-water mark of `sp`.
    pub nregs: u16,
    /// `for` scopes above (upvar depth adjustment).
    pub for_depth: u16,
    /// Recursion level against `MRC_CODEGEN_LEVEL_MAX` (`s->rlev`).
    pub rlev: u32,
    /// True for the dummy top scope created by `generate_code`.
    pub is_top: bool,
}

fn error(message: &str) -> Diagnostic {
    Diagnostic {
        message: message.to_owned(),
        start: 0,
        end: 0,
    }
}

/// Codegen failure: a single diagnostic (C appends one message, then throws).
pub type CodegenError = Diagnostic;

impl Session {
    /// New session; the filename defaults to `"-e"` like `mrc_load_string_cxt`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register scope locals into the `LVAR` table in preorder, mirroring
    /// `create_lv_sym_table` (skips empty markers, keeps first-seen order).
    pub fn register_lv_names(&mut self, names: &[Vec<u8>]) {
        for name in names {
            if name.is_empty() {
                continue;
            }
            if !self.lvar_names.contains(name) {
                self.lvar_names.push(name.clone());
            }
        }
    }

    /// Index of a name in the `LVAR` table.
    pub fn lvar_index(&self, name: &[u8]) -> Option<u32> {
        self.lvar_names
            .iter()
            .position(|known| known == name)
            .map(|index| index as u32)
    }
}

impl Scope {
    /// Dummy top scope (`scope_new(c, NULL, NULL)` returns before any setup).
    pub fn top() -> Self {
        Self {
            sp: 0,
            pc: 0,
            lastpc: 0,
            lastlabel: 0,
            iseq: Vec::new(),
            pool: Vec::new(),
            syms: Vec::new(),
            scapa: 256,
            reps: Vec::new(),
            catch_table: Vec::new(),
            loops: Vec::new(),
            lv_names: Vec::new(),
            nlocals: 0,
            nregs: 0,
            for_depth: 0,
            rlev: 0,
            is_top: true,
        }
    }

    /// Child scope (`scope_new` with `prev` set).
    pub fn child(
        session: &mut Session,
        _parent: &Scope,
        locals: &[Vec<u8>],
    ) -> Result<Self, Diagnostic> {
        if locals.len() >= u8::MAX as usize {
            return Err(error("too many local variables"));
        }
        // Preorder registration keeps dump order (see `register_lv_names`).
        session.register_lv_names(locals);
        if !locals.is_empty() {
            session.any_lv = true;
        }
        let mut scope = Self {
            sp: 0,
            pc: 0,
            lastpc: 0,
            lastlabel: 0,
            iseq: Vec::with_capacity(1024),
            pool: Vec::with_capacity(32),
            syms: Vec::with_capacity(256),
            scapa: 256,
            reps: Vec::with_capacity(8),
            catch_table: Vec::new(),
            loops: Vec::new(),
            lv_names: locals.to_vec(),
            nlocals: 0,
            nregs: 0,
            for_depth: 0,
            rlev: 0,
            is_top: false,
        };
        scope.sp = locals.len() as u16 + 1; // add self
        scope.nlocals = scope.sp;
        scope.nregs = scope.sp;
        Ok(scope)
    }

    /// Current stack pointer.
    pub fn cursp(&self) -> u16 {
        self.sp
    }

    fn nregs_update(&mut self) {
        if self.sp > self.nregs {
            self.nregs = self.sp;
        }
    }

    /// Reserve `n` registers (`push_n_`).
    pub fn push_n(&mut self, n: u16) -> Result<(), Diagnostic> {
        if self.sp as u32 + u32::from(n) >= 0xffff {
            return Err(error("too complex expression"));
        }
        self.sp += n;
        self.nregs_update();
        Ok(())
    }

    /// Release `n` registers (`pop_n_`).
    pub fn pop_n(&mut self, n: u16) -> Result<(), Diagnostic> {
        if (u32::from(self.sp) as i64 - i64::from(n)) < 0 {
            return Err(error("stack pointer underflow"));
        }
        self.sp -= n;
        Ok(())
    }

    /// Emit one byte at `pc`, growing the buffer (`emit_B`).
    fn emit_b(&mut self, pc: u32, byte: u8) -> Result<(), Diagnostic> {
        if pc == u32::MAX {
            return Err(error("too big code block"));
        }
        let index = pc as usize;
        if index >= self.iseq.len() {
            let grown = (self.iseq.len().max(1) * 2).max(index + 1);
            self.iseq.resize(grown, 0);
        }
        self.iseq[index] = byte;
        Ok(())
    }

    /// Emit one byte at the cursor (`gen_B`).
    pub fn gen_b(&mut self, byte: u8) -> Result<(), Diagnostic> {
        self.emit_b(self.pc, byte)?;
        self.pc += 1;
        Ok(())
    }

    /// Emit big-endian `u16` at the cursor (`gen_S`).
    pub fn gen_s(&mut self, value: u16) -> Result<(), Diagnostic> {
        self.emit_b(self.pc, (value >> 8) as u8)?;
        self.emit_b(self.pc + 1, (value & 0xff) as u8)?;
        self.pc += 2;
        Ok(())
    }

    fn check_no_ext_ops(&self, session: &Session, a: u16, b: u16) -> Result<(), Diagnostic> {
        if session.no_ext_ops && (a | b) > 0xff {
            return Err(error(
                "need OP_EXTs instruction (currently OP_EXTs are prohibited)",
            ));
        }
        Ok(())
    }

    /// Zero-operand instruction (`genop_0`).
    pub fn genop_0(&mut self, op: u8) -> Result<(), Diagnostic> {
        self.lastpc = self.pc;
        self.gen_b(op)
    }

    /// One-operand instruction with `EXT1` widening (`genop_1`).
    pub fn genop_1(&mut self, session: &Session, op: u8, a: u16) -> Result<(), Diagnostic> {
        self.lastpc = self.pc;
        self.check_no_ext_ops(session, a, 0)?;
        if a > 0xff {
            self.gen_b(opcode::OP_EXT1)?;
            self.gen_b(op)?;
            self.gen_s(a)?;
        } else {
            self.gen_b(op)?;
            self.gen_b(a as u8)?;
        }
        Ok(())
    }

    /// Two-operand instruction with `EXT` widening (`genop_2`).
    pub fn genop_2(&mut self, session: &Session, op: u8, a: u16, b: u16) -> Result<(), Diagnostic> {
        self.lastpc = self.pc;
        self.check_no_ext_ops(session, a, b)?;
        if a > 0xff && b > 0xff {
            self.gen_b(opcode::OP_EXT3)?;
            self.gen_b(op)?;
            self.gen_s(a)?;
            self.gen_s(b)?;
        } else if b > 0xff {
            self.gen_b(opcode::OP_EXT2)?;
            self.gen_b(op)?;
            self.gen_b(a as u8)?;
            self.gen_s(b)?;
        } else if a > 0xff {
            self.gen_b(opcode::OP_EXT1)?;
            self.gen_b(op)?;
            self.gen_s(a)?;
            self.gen_b(b as u8)?;
        } else {
            self.gen_b(op)?;
            self.gen_b(a as u8)?;
            self.gen_b(b as u8)?;
        }
        Ok(())
    }

    /// Two-operand instruction plus trailing byte (`genop_3`).
    pub fn genop_3(
        &mut self,
        session: &Session,
        op: u8,
        a: u16,
        b: u16,
        c: u8,
    ) -> Result<(), Diagnostic> {
        self.genop_2(session, op, a, b)?;
        self.gen_b(c)
    }

    /// `B` + `S` operands (`genop_2S`).
    pub fn genop_2s(
        &mut self,
        session: &Session,
        op: u8,
        a: u16,
        b: u16,
    ) -> Result<(), Diagnostic> {
        self.genop_1(session, op, a)?;
        self.gen_s(b)
    }

    /// `B` + 32-bit operands (`genop_2SS`).
    pub fn genop_2ss(
        &mut self,
        session: &Session,
        op: u8,
        a: u16,
        b: u32,
    ) -> Result<(), Diagnostic> {
        self.genop_1(session, op, a)?;
        self.gen_s((b >> 16) as u16)?;
        self.gen_s((b & 0xffff) as u16)
    }

    /// 24-bit operand (`genop_W`).
    pub fn genop_w(&mut self, op: u8, a: u32) -> Result<(), Diagnostic> {
        self.lastpc = self.pc;
        self.gen_b(op)?;
        self.gen_b(((a >> 16) & 0xff) as u8)?;
        self.gen_b(((a >> 8) & 0xff) as u8)?;
        self.gen_b((a & 0xff) as u8)
    }

    /// Fresh jump target marker (`new_label`).
    pub fn new_label(&mut self) -> u32 {
        self.lastlabel = self.pc;
        self.pc
    }

    /// Previous instruction start before `pc` (`mrc_prev_pc`).
    pub fn prev_pc(&self, pc: u32) -> u32 {
        let mut prev = 0;
        let mut cursor = 0;
        while cursor < pc {
            let op = self.iseq[cursor as usize];
            prev = cursor;
            let step = match op {
                opcode::OP_EXT1 => {
                    u32::from(opcode::insn_size(self.iseq[cursor as usize + 1], 1)) + 1
                }
                opcode::OP_EXT2 => {
                    u32::from(opcode::insn_size(self.iseq[cursor as usize + 1], 2)) + 1
                }
                opcode::OP_EXT3 => {
                    u32::from(opcode::insn_size(self.iseq[cursor as usize + 1], 3)) + 1
                }
                _ => u32::from(opcode::insn_size(op, 0)),
            };
            cursor += step;
        }
        prev
    }

    /// Decode the last emitted instruction (`mrc_last_insn`).
    pub fn last_insn(&self) -> Decoded {
        if self.pc == 0 {
            return Decoded {
                insn: opcode::OP_NOP,
                a: 0,
                b: 0,
                cc: 0,
            };
        }
        opcode::decode_at(&self.iseq, self.lastpc as usize)
            .map(|(decoded, _)| decoded)
            .unwrap_or(Decoded {
                insn: opcode::OP_NOP,
                a: 0,
                b: 0,
                cc: 0,
            })
    }

    /// Peephole barrier (`no_peephole`).
    pub fn no_peephole(&self, session: &Session) -> bool {
        session.no_optimize || self.lastlabel == self.pc || self.pc == 0 || self.pc == self.lastpc
    }

    /// Integer operand of a load instruction (`get_int_operand`).
    pub fn int_operand(&self, data: &Decoded) -> Option<i64> {
        match data.insn {
            opcode::OP_LOADI__1 => Some(-1),
            opcode::OP_LOADINEG => Some(-i64::from(data.b)),
            opcode::OP_LOADI_0
            | opcode::OP_LOADI_1
            | opcode::OP_LOADI_2
            | opcode::OP_LOADI_3
            | opcode::OP_LOADI_4
            | opcode::OP_LOADI_5
            | opcode::OP_LOADI_6
            | opcode::OP_LOADI_7 => Some(i64::from(data.insn - opcode::OP_LOADI_0)),
            opcode::OP_LOADI8 | opcode::OP_LOADI16 => Some(i64::from(data.b as i16)),
            opcode::OP_LOADI32 => Some(i64::from(
                ((u32::from(data.b) << 16) | u32::from(data.cc)) as i32,
            )),
            opcode::OP_LOADL => match self.pool.get(data.b as usize) {
                Some(CgPool::Int32(value)) => Some(i64::from(*value)),
                Some(CgPool::Int64(value)) => Some(*value),
                _ => None,
            },
            _ => None,
        }
    }

    /// Jump offset writer (`gen_jmpdst`).
    pub fn gen_jmpdst(&mut self, pc: u32) -> Result<(), Diagnostic> {
        let target = if pc == JMPLINK_START { 0 } else { pc };
        let pos2 = self.pc + 2;
        let off = (target as i64) - (pos2 as i64);
        if off > i64::from(i16::MAX) || off < i64::from(i16::MIN) {
            return Err(error("too big jump offset"));
        }
        self.gen_s(off as u16)
    }

    /// Unconditional jump with patched-later offset (`genjmp`).
    pub fn genjmp(&mut self, op: u8, pc: u32) -> Result<u32, Diagnostic> {
        self.genop_0(op)?;
        let pos = self.pc;
        self.gen_jmpdst(pc)?;
        Ok(pos)
    }

    /// Conditional jump with peephole elision (`genjmp2`).
    pub fn genjmp2(
        &mut self,
        session: &Session,
        op: u8,
        mut a: u16,
        pc: u32,
        val: bool,
    ) -> Result<u32, Diagnostic> {
        if !self.no_peephole(session) && !val {
            let data = self.last_insn();
            match data.insn {
                opcode::OP_MOVE => {
                    if data.a == u32::from(a) && data.a > u32::from(self.nlocals) {
                        // Single rewrite (`rewind_pc; a = data.b`), then plain
                        // emission below like C (no recursion into peephole).
                        self.pc = self.lastpc;
                        a = data.b;
                    }
                }
                opcode::OP_LOADNIL | opcode::OP_LOADFALSE
                    if data.a == u32::from(a) || data.a > u32::from(self.nlocals) =>
                {
                    self.pc = self.lastpc;
                    if op == opcode::OP_JMPNOT
                        || (op == opcode::OP_JMPNIL && data.insn == opcode::OP_LOADNIL)
                    {
                        return self.genjmp(opcode::OP_JMP, pc);
                    }
                    return Ok(JMPLINK_START);
                }
                opcode::OP_LOADTRUE
                | opcode::OP_LOADI8
                | opcode::OP_LOADINEG
                | opcode::OP_LOADI__1
                | opcode::OP_LOADI_0
                | opcode::OP_LOADI_1
                | opcode::OP_LOADI_2
                | opcode::OP_LOADI_3
                | opcode::OP_LOADI_4
                | opcode::OP_LOADI_5
                | opcode::OP_LOADI_6
                | opcode::OP_LOADI_7
                    if data.a == u32::from(a) || data.a > u32::from(self.nlocals) =>
                {
                    self.pc = self.lastpc;
                    if op == opcode::OP_JMPIF {
                        return self.genjmp(opcode::OP_JMP, pc);
                    }
                    return Ok(JMPLINK_START);
                }
                _ => {}
            }
        }
        // Plain emission. `lastpc` points at the opcode: C emits the manual
        // `EXT1` byte first and `genop_0` stamps `lastpc` after it.
        if a > 0xff {
            self.check_no_ext_ops(session, a, 0)?;
            self.gen_b(opcode::OP_EXT1)?;
            self.genop_0(op)?;
            self.gen_s(a)?;
        } else {
            self.genop_0(op)?;
            self.gen_b(a as u8)?;
        }
        let pos = self.pc;
        self.gen_jmpdst(pc)?;
        Ok(pos)
    }

    /// Patch one linked jump to the current `pc` (`dispatch`).
    pub fn dispatch(&mut self, pos0: u32) -> Result<u32, Diagnostic> {
        if pos0 == JMPLINK_START {
            return Ok(0);
        }
        let pos1 = pos0 + 2;
        let offset = self.pc as i64 - pos1 as i64;
        if offset > i64::from(i16::MAX) {
            return Err(error("too big jmp offset"));
        }
        self.lastlabel = self.pc;
        // The link slot holds a signed offset (`int16_t newpos` in C; the
        // initial `JMPLINK_START` link reads back as a negative value whose
        // addition lands on zero to end the chain).
        let linked = i16::from_be_bytes([self.iseq[pos0 as usize], self.iseq[pos0 as usize + 1]]);
        self.emit_b(pos0, (offset >> 8) as u8)?;
        self.emit_b(pos0 + 1, (offset & 0xff) as u8)?;
        if linked == 0 {
            return Ok(0);
        }
        Ok((pos1 as i64 + i64::from(linked)) as u32)
    }

    /// Patch a whole jump chain (`dispatch_linked`).
    pub fn dispatch_linked(&mut self, mut pos: u32) -> Result<(), Diagnostic> {
        if pos == JMPLINK_START {
            return Ok(());
        }
        loop {
            pos = self.dispatch(pos)?;
            if pos == 0 {
                break;
            }
        }
        Ok(())
    }

    /// Integer literal emission with inline selection (`gen_int`).
    pub fn gen_int(
        &mut self,
        session: &mut Session,
        dst: u16,
        value: i64,
    ) -> Result<(), Diagnostic> {
        if value < 0 {
            if value == -1 {
                self.genop_1(session, opcode::OP_LOADI__1, dst)
            } else if value >= -0xff {
                self.genop_2(session, opcode::OP_LOADINEG, dst, (-value) as u16)
            } else if value >= i64::from(i16::MIN) {
                self.genop_2s(session, opcode::OP_LOADI16, dst, value as u16)
            } else if value >= i64::from(i32::MIN) {
                self.genop_2ss(session, opcode::OP_LOADI32, dst, value as u32)
            } else {
                let index = self.new_lit_int(session, value)? as u16;
                self.genop_2(session, opcode::OP_LOADL, dst, index)
            }
        } else if value < 8 {
            self.genop_1(session, opcode::OP_LOADI_0 + value as u8, dst)
        } else if value <= 0xff {
            self.genop_2(session, opcode::OP_LOADI8, dst, value as u16)
        } else if value <= i64::from(i16::MAX) {
            self.genop_2s(session, opcode::OP_LOADI16, dst, value as u16)
        } else if value <= i64::from(i32::MAX) {
            self.genop_2ss(session, opcode::OP_LOADI32, dst, value as u32)
        } else {
            let index = self.new_lit_int(session, value)? as u16;
            self.genop_2(session, opcode::OP_LOADL, dst, index)
        }
    }

    /// Register move with peephole fusion (`gen_move`).
    pub fn gen_move(
        &mut self,
        session: &mut Session,
        dst: u16,
        src: u16,
        nopeep: bool,
    ) -> Result<(), Diagnostic> {
        if !nopeep && !self.no_peephole(session) {
            if dst == src {
                return Ok(());
            }
            let data = self.last_insn();
            match data.insn {
                opcode::OP_MOVE => {
                    if data.a == u32::from(src) {
                        if data.b == dst {
                            return Ok(());
                        }
                        if data.a < u32::from(self.nlocals) {
                            return self.plain_move(session, dst, src);
                        }
                        self.pc = self.lastpc;
                        self.lastpc = self.prev_pc(self.pc);
                        return self.gen_move(session, dst, data.b, false);
                    }
                    if u32::from(dst) == data.a {
                        self.pc = self.lastpc;
                        self.lastpc = self.prev_pc(self.pc);
                        return self.gen_move(session, dst, src, false);
                    }
                }
                opcode::OP_LOADNIL
                | opcode::OP_LOADSELF
                | opcode::OP_LOADTRUE
                | opcode::OP_LOADFALSE
                | opcode::OP_LOADI__1
                | opcode::OP_LOADI_0
                | opcode::OP_LOADI_1
                | opcode::OP_LOADI_2
                | opcode::OP_LOADI_3
                | opcode::OP_LOADI_4
                | opcode::OP_LOADI_5
                | opcode::OP_LOADI_6
                | opcode::OP_LOADI_7 => {
                    if data.a == u32::from(src) && data.a >= u32::from(self.nlocals) {
                        self.pc = self.lastpc;
                        return self.genop_1(session, data.insn, dst);
                    }
                }
                opcode::OP_HASH
                | opcode::OP_LOADI8
                | opcode::OP_LOADINEG
                | opcode::OP_LOADL
                | opcode::OP_LOADSYM
                | opcode::OP_GETGV
                | opcode::OP_GETSV
                | opcode::OP_GETIV
                | opcode::OP_GETCV
                | opcode::OP_GETCONST
                | opcode::OP_STRING
                | opcode::OP_LAMBDA
                | opcode::OP_BLOCK
                | opcode::OP_METHOD
                | opcode::OP_BLKPUSH => {
                    // C falls from `OP_HASH` (with `b == 0`) into this fusion.
                    if data.insn == opcode::OP_HASH && data.b != 0 {
                        return self.plain_move(session, dst, src);
                    }
                    return self.fused_move_2(session, dst, src, &data);
                }
                opcode::OP_LOADI16 => {
                    if data.a != u32::from(src) || data.a < u32::from(self.nlocals) {
                        return self.plain_move(session, dst, src);
                    }
                    self.pc = self.lastpc;
                    return self.genop_2s(session, data.insn, dst, data.b);
                }
                opcode::OP_LOADI32 => {
                    if data.a != u32::from(src) || data.a < u32::from(self.nlocals) {
                        return self.plain_move(session, dst, src);
                    }
                    let value = (u32::from(data.b) << 16) | u32::from(data.cc);
                    self.pc = self.lastpc;
                    return self.genop_2ss(session, data.insn, dst, value);
                }
                opcode::OP_ARRAY => {
                    if data.a != u32::from(src)
                        || data.a < u32::from(self.nlocals)
                        || data.a < u32::from(dst)
                    {
                        return self.plain_move(session, dst, src);
                    }
                    self.pc = self.lastpc;
                    if data.b == 0 || u32::from(dst) == data.a {
                        return self.genop_2(session, opcode::OP_ARRAY, dst, 0);
                    }
                    return self.genop_3(
                        session,
                        opcode::OP_ARRAY2,
                        dst,
                        data.a as u16,
                        data.b as u8,
                    );
                }
                opcode::OP_ARRAY2 => {
                    if data.a != u32::from(src)
                        || data.a < u32::from(self.nlocals)
                        || data.a < u32::from(dst)
                    {
                        return self.plain_move(session, dst, src);
                    }
                    self.pc = self.lastpc;
                    return self.genop_3(session, opcode::OP_ARRAY2, dst, data.b, data.cc as u8);
                }
                opcode::OP_AREF | opcode::OP_GETUPVAR => {
                    if data.a != u32::from(src) || data.a < u32::from(self.nlocals) {
                        return self.plain_move(session, dst, src);
                    }
                    self.pc = self.lastpc;
                    return self.genop_3(session, data.insn, dst, data.b, data.cc as u8);
                }
                opcode::OP_ADDI | opcode::OP_SUBI => {
                    // ADDILV/SUBILV fusion needs the preceding MOVE; the
                    // trailing `genop_2` in C is unreachable. A zero `lastpc`
                    // would make `prev_pc` read the first instruction where C
                    // decodes `NULL`, so it takes the plain path too.
                    if self.lastpc == 0
                        || self.lastpc == self.lastlabel
                        || data.a != u32::from(src)
                        || data.a < u32::from(self.nlocals)
                    {
                        return self.plain_move(session, dst, src);
                    }
                    let prev = self.prev_pc(self.lastpc);
                    let Some((data0, _)) = opcode::decode_at(&self.iseq, prev as usize) else {
                        return self.plain_move(session, dst, src);
                    };
                    if data0.insn != opcode::OP_MOVE || data0.a != data.a || data0.b != dst {
                        return self.plain_move(session, dst, src);
                    }
                    self.pc = prev;
                    let fused = if data.insn == opcode::OP_ADDI {
                        opcode::OP_ADDILV
                    } else {
                        opcode::OP_SUBILV
                    };
                    return self.genop_3(session, fused, dst, data.a as u16, data.b as u8);
                }
                _ => {}
            }
        }
        self.plain_move(session, dst, src)
    }

    fn plain_move(&mut self, session: &Session, dst: u16, src: u16) -> Result<(), Diagnostic> {
        self.genop_2(session, opcode::OP_MOVE, dst, src)
    }

    fn fused_move_2(
        &mut self,
        session: &mut Session,
        dst: u16,
        src: u16,
        data: &Decoded,
    ) -> Result<(), Diagnostic> {
        // `OP_HASH` with `b == 0` falls through to the same fusion in C.
        if data.a != u32::from(src) || data.a < u32::from(self.nlocals) {
            return self.plain_move(session, dst, src);
        }
        let (insn, operand) = (data.insn, data.b);
        self.pc = self.lastpc;
        self.genop_2(session, insn, dst, operand)
    }

    /// Return emission with `RET*` fusion (`gen_return`).
    pub fn gen_return(&mut self, session: &Session, op: u8, src: u16) -> Result<(), Diagnostic> {
        if self.no_peephole(session) {
            return self.genop_1(session, op, src);
        }
        let data = self.last_insn();
        if data.insn == opcode::OP_MOVE && u32::from(src) == data.a {
            self.pc = self.lastpc;
            return self.genop_1(session, op, data.b);
        }
        if u32::from(src) == data.a && op == opcode::OP_RETURN {
            let fused = match data.insn {
                opcode::OP_LOADSELF => Some(opcode::OP_RETSELF),
                opcode::OP_LOADNIL => Some(opcode::OP_RETNIL),
                opcode::OP_LOADTRUE => Some(opcode::OP_RETTRUE),
                opcode::OP_LOADFALSE => Some(opcode::OP_RETFALSE),
                _ => None,
            };
            if let Some(fused) = fused {
                self.pc = self.lastpc;
                return self.genop_0(fused);
            }
        }
        match data.insn {
            opcode::OP_RETURN
            | opcode::OP_RETSELF
            | opcode::OP_RETNIL
            | opcode::OP_RETTRUE
            | opcode::OP_RETFALSE => Ok(()),
            _ => self.genop_1(session, op, src),
        }
    }

    /// Local slot for a name (`lv_idx`, 1-based, `0` when absent).
    pub fn lv_idx(&self, name: &[u8]) -> u16 {
        self.lv_names
            .iter()
            .position(|known| known == name)
            .map(|index| index as u16 + 1)
            .unwrap_or(0)
    }

    /// Grow the pool and return the fresh slot (`lit_pool_extend`).
    fn lit_pool_extend(&mut self) -> Result<usize, Diagnostic> {
        if self.pool.len() == 0xffff {
            return Err(error("too many literals"));
        }
        let index = self.pool.len();
        self.pool.push(CgPool::Int32(0));
        Ok(index)
    }

    /// Integer pool interning (`new_lit_int`; dedups `INT32` and `INT64`
    /// because `mrc_common.h` defines both `MRC_INT64` and `MRC_64BIT`).
    pub fn new_lit_int(&mut self, _session: &Session, num: i64) -> Result<usize, Diagnostic> {
        for (index, entry) in self.pool.iter().enumerate() {
            match entry {
                CgPool::Int32(value) if num == i64::from(*value) => return Ok(index),
                CgPool::Int64(value) if num == *value => return Ok(index),
                _ => {}
            }
        }
        let index = self.lit_pool_extend()?;
        self.pool[index] = CgPool::Int64(num);
        Ok(index)
    }

    /// Float pool interning (`new_lit_float`, sign-aware dedup).
    pub fn new_lit_float(&mut self, _session: &Session, num: f64) -> Result<usize, Diagnostic> {
        for (index, entry) in self.pool.iter().enumerate() {
            if let CgPool::Float(value) = entry {
                if *value == num && value.is_sign_positive() == num.is_sign_positive() {
                    return Ok(index);
                }
            }
        }
        let index = self.lit_pool_extend()?;
        self.pool[index] = CgPool::Float(num);
        Ok(index)
    }

    /// String pool interning (`new_lit_str`).
    pub fn new_lit_str(&mut self, _session: &Session, bytes: &[u8]) -> Result<usize, Diagnostic> {
        if bytes.len() > u16::MAX as usize {
            return Err(error("string literal too long"));
        }
        for (index, entry) in self.pool.iter().enumerate() {
            if let CgPool::Str(known) = entry {
                if known.len() == bytes.len() && (known.is_empty() || known == bytes) {
                    return Ok(index);
                }
            }
        }
        let index = self.lit_pool_extend()?;
        self.pool[index] = CgPool::Str(bytes.to_vec());
        Ok(index)
    }

    /// Bigint pool interning (`new_litbint`; `base` is signed: negative input).
    pub fn new_litbint(
        &mut self,
        _session: &Session,
        digits: &[u8],
        base: u8,
        negative: bool,
    ) -> Result<usize, Diagnostic> {
        if digits.len() > 255 {
            return Err(error("integer too big"));
        }
        let sign = if negative {
            (base as i8).wrapping_neg() as u8
        } else {
            base
        };
        for (index, entry) in self.pool.iter().enumerate() {
            if let CgPool::BigInt(raw) = entry {
                if raw.len() == digits.len() + 2 && raw[1] == sign && raw[2..] == digits[..] {
                    return Ok(index);
                }
            }
        }
        let index = self.lit_pool_extend()?;
        let mut raw = Vec::with_capacity(digits.len() + 2);
        raw.push(digits.len() as u8);
        raw.push(sign);
        raw.extend_from_slice(digits);
        self.pool[index] = CgPool::BigInt(raw);
        Ok(index)
    }

    /// Symbol interning (`new_sym`, with the C doubling capacity).
    pub fn new_sym(&mut self, session: &mut Session, name: &[u8]) -> Result<u16, Diagnostic> {
        let id = session.symbols.intern(name);
        if let Some(index) = self.syms.iter().position(|known| *known == id) {
            return Ok(index as u16);
        }
        if name.len() >= 0xffff {
            return Err(error("symbol name too long"));
        }
        // `scapa` starts at 256 and doubles; past `0xffff` is an error, so at
        // most 32768 symbols fit, like in C.
        if self.syms.len() >= self.scapa {
            self.scapa *= 2;
            if self.scapa > 0xffff {
                return Err(error("too many symbols"));
            }
        }
        self.syms.push(id);
        Ok((self.syms.len() - 1) as u16)
    }

    /// Reserve a catch handler slot (`catch_handler_new`).
    pub fn catch_new(&mut self) -> usize {
        let entry = self.catch_table.len();
        self.catch_table.push(CatchHandler {
            kind: 0,
            begin: 0,
            end: 0,
            target: 0,
        });
        entry
    }

    /// Fill a catch handler slot (`catch_handler_set`).
    pub fn catch_set(&mut self, entry: usize, kind: u8, begin: u32, end: u32, target: u32) {
        self.catch_table[entry] = CatchHandler {
            kind,
            begin,
            end,
            target,
        };
    }

    /// Open a loop frame (`loop_push`).
    pub fn loop_push(&mut self, kind: LoopType) {
        let reg = self.sp as i32;
        self.loops.push(LoopFrame {
            kind,
            pc0: JMPLINK_START,
            pc1: JMPLINK_START,
            pc2: JMPLINK_START,
            reg,
        });
    }

    /// Close a loop frame, patching `break` jumps (`loop_pop`).
    pub fn loop_pop(&mut self, session: &Session, val: bool) -> Result<(), Diagnostic> {
        if val {
            self.genop_1(session, opcode::OP_LOADNIL, self.sp)?;
        }
        let pc2 = self
            .loops
            .pop()
            .map(|frame| frame.pc2)
            .unwrap_or(JMPLINK_START);
        self.dispatch_linked(pc2)?;
        if val {
            self.push_n(1)?;
        }
        Ok(())
    }

    /// Finish the scope into an irep (`scope_finish`).
    pub fn finish(mut self, session: &mut Session) -> Result<Irep, Diagnostic> {
        if self.nlocals > 0xff {
            return Err(error("too many local variables"));
        }
        let mut pool = Vec::with_capacity(self.pool.len());
        for entry in self.pool {
            pool.push(match entry {
                CgPool::Str(bytes) => PoolValue::Str(bytes),
                CgPool::Int32(value) => PoolValue::Int32(value),
                CgPool::Int64(value) => PoolValue::Int64(value),
                CgPool::Float(value) => PoolValue::Float(value),
                CgPool::BigInt(raw) => PoolValue::BigInt(raw),
            });
        }
        let mut syms = Vec::with_capacity(self.syms.len());
        for id in self.syms {
            let name = session
                .symbols
                .lookup(id)
                .ok_or_else(|| error("internal error: missing interned symbol"))?;
            syms.push(Some(name.to_vec()));
        }
        let mut lv = Vec::with_capacity(self.lv_names.len());
        for name in &self.lv_names {
            if name.is_empty() {
                lv.push(None);
            } else {
                let index = session
                    .lvar_index(name)
                    .ok_or_else(|| error("internal error: local missing from LVAR table"))?;
                lv.push(Some(index));
            }
        }
        self.iseq.truncate(self.pc as usize);
        Ok(Irep {
            nlocals: self.nlocals,
            nregs: self.nregs,
            iseq: self.iseq,
            catch_handlers: self.catch_table,
            pool,
            syms,
            reps: self.reps,
            lv,
        })
    }
}
