//! Node handlers: port of the `codegen()` tranches covering the P1 corpus.
//!
//! Dispatch is on `kind_name` (1:1 with the C `switch` on node type); values
//! travel through `BackendNode`, so FFI and owned frontends share handlers.

use carnelian_ast::view::{BackendNode, SimpleLit};

use crate::codegen::{LoopType, Scope, Session, JMPLINK_START};
use crate::diagnostics::{Diagnostic, Diagnostics};
use crate::irep::{Irep, RiteModel};
use crate::opcode;
use crate::writer::write_rite;
use crate::CompileOptions;

/// Maximum positional arguments before array packing (`GEN_LIT_ARY_MAX` dance
/// uses 14 at call sites, 64 for a bare `0` limit).
const CALL_ARG_LIMIT: usize = 14;
const LIT_ARY_MAX: usize = 64;

/// Scopes stack plus session and finished root.
pub struct Codegen {
    session: Session,
    scopes: Vec<Scope>,
    root: Option<Irep>,
}

impl Codegen {
    /// New unit with the dummy top scope (`generate_code` head).
    pub fn new() -> Self {
        Self {
            session: Session::new(),
            scopes: vec![Scope::top()],
            root: None,
        }
    }

    /// Current session and scope (disjoint borrows).
    pub fn current(&mut self) -> (&mut Session, &mut Scope) {
        let scopes = &mut self.scopes;
        let scope = scopes.last_mut().expect("open scope");
        (&mut self.session, scope)
    }

    /// Open a body scope over `locals` (`scope_new` for program bodies).
    pub fn push_body(&mut self, locals: &[Vec<u8>]) -> Result<(), Diagnostic> {
        let child = Scope::child(&mut self.session, self.scopes.last().expect("top"), locals)?;
        self.scopes.push(child);
        Ok(())
    }

    /// Attach the finished scope to its parent, or store the root.
    /// Returns the child index (like `scope_body`).
    pub fn pop_scope(&mut self) -> Result<usize, Diagnostic> {
        let parent_is_top = self.scopes.len() >= 2 && self.scopes[self.scopes.len() - 2].is_top;
        let scope = self.scopes.pop().expect("scope");
        let irep = scope.finish(&mut self.session)?;
        if parent_is_top && self.scopes.len() == 1 {
            self.root = Some(irep);
            return Ok(0);
        }
        let parent = self.scopes.last_mut().expect("parent");
        if parent.reps.len() == u16::MAX as usize {
            return Err(Diagnostic {
                message: "too many nested blocks/methods".to_owned(),
                start: 0,
                end: 0,
            });
        }
        parent.reps.push(irep);
        Ok(parent.reps.len() - 1)
    }
}

impl Default for Codegen {
    fn default() -> Self {
        Self::new()
    }
}

fn unsupported<N: BackendNode>(node: &N, what: &str) -> Diagnostic {
    let span = node.span();
    Diagnostic {
        message: format!("unsupported {what} in P1: {}", node.kind_name()),
        start: span.start,
        end: span.end,
    }
}

fn const_true<N: BackendNode>(node: &N) -> bool {
    matches!(
        node.kind_name(),
        "TrueNode" | "IntegerNode" | "StringNode" | "SymbolNode"
    )
}

fn const_false<N: BackendNode>(node: &N) -> bool {
    matches!(node.kind_name(), "FalseNode" | "NilNode")
}

/// Statement list shared by `StatementsNode` and empty `ElseNode` bodies.
fn gen_list<N: BackendNode>(cg: &mut Codegen, items: Vec<N>, val: bool) -> Result<(), Diagnostic> {
    if items.is_empty() {
        if val {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
            scope.push_n(1)?;
        }
        return Ok(());
    }
    let last = items.len() - 1;
    for (index, item) in items.into_iter().enumerate() {
        codegen(cg, item, index == last && val)?;
    }
    Ok(())
}

/// Main dispatch (`codegen()` switch).
pub fn codegen<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    match node.kind_name() {
        "ProgramNode" => gen_program(cg, node, val),
        "StatementsNode" => {
            let Some(items) = node.statements() else {
                return Err(unsupported(&node, "statements"));
            };
            gen_list(cg, items, val)
        }
        "ElseNode" => {
            let Some(items) = node.else_body() else {
                return Err(unsupported(&node, "else"));
            };
            gen_list(cg, items, val)
        }
        "IntegerNode" => gen_integer(cg, node, val),
        "FloatNode" => gen_float(cg, node, val),
        "StringNode" => gen_string(cg, node, val),
        "SymbolNode" => gen_symbol(cg, node, val),
        "CallNode" => gen_call(cg, node, val),
        "IfNode" | "UnlessNode" => gen_if(cg, node, val),
        "ArrayNode" => gen_array(cg, node, val),
        "WhileNode" | "UntilNode" => gen_while(cg, node, val),
        "AndNode" => gen_logic(cg, node, val, false),
        "OrNode" => gen_logic(cg, node, val, true),
        "LocalVariableReadNode" => gen_lvar_read(cg, node, val),
        "LocalVariableWriteNode" => gen_lvar_write(cg, node, val),
        "TrueNode" | "FalseNode" | "NilNode" | "SelfNode" => gen_simple(cg, node, val),
        _ => Err(unsupported(&node, "node")),
    }
}

fn gen_program<N: BackendNode>(cg: &mut Codegen, node: N, _val: bool) -> Result<(), Diagnostic> {
    // `scope_body` for `PM_PROGRAM_NODE` (always `VAL`, always `OP_RETURN`).
    let Some(view) = node.program() else {
        return Err(unsupported(&node, "program"));
    };
    cg.push_body(&view.locals)?;
    codegen(cg, view.body, true)?;
    let top_child = cg.scopes.len() == 2;
    {
        let (session, scope) = cg.current();
        let ret = scope.sp - 1;
        scope.gen_return(session, opcode::OP_RETURN, ret)?;
        if top_child {
            scope.genop_0(opcode::OP_STOP)?;
        }
    }
    cg.pop_scope()?;
    Ok(())
}

fn gen_integer<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    if !val {
        return Ok(());
    }
    // `gen_pm_integer`: small values inline, overflow goes to the pool.
    let Some(lit) = node.integer_lit() else {
        return Err(unsupported(&node, "integer literal"));
    };
    let (session, scope) = cg.current();
    let dst = scope.cursp();
    match lit {
        carnelian_ast::view::IntegerLit::I64(value) => scope.gen_int(session, dst, value)?,
        carnelian_ast::view::IntegerLit::Bigint { digits, negative } => {
            let index = scope.new_litbint(session, &digits, 10, negative)? as u16;
            scope.genop_2(session, opcode::OP_LOADL, dst, index)?;
        }
    }
    scope.push_n(1)
}

fn gen_float<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    if !val {
        return Ok(());
    }
    let Some(value) = node.float_lit() else {
        return Err(unsupported(&node, "float literal"));
    };
    let (session, scope) = cg.current();
    let dst = scope.cursp();
    let index = scope.new_lit_float(session, value)? as u16;
    scope.genop_2(session, opcode::OP_LOADL, dst, index)?;
    scope.push_n(1)
}

fn gen_string<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    if !val {
        return Ok(());
    }
    let Some(bytes) = node.string_lit() else {
        return Err(unsupported(&node, "string literal"));
    };
    let (session, scope) = cg.current();
    let dst = scope.cursp();
    let index = scope.new_lit_str(session, &bytes)? as u16;
    scope.genop_2(session, opcode::OP_STRING, dst, index)?;
    scope.push_n(1)
}

fn gen_symbol<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    if !val {
        return Ok(());
    }
    let Some(bytes) = node.symbol_lit() else {
        return Err(unsupported(&node, "symbol literal"));
    };
    let (session, scope) = cg.current();
    let dst = scope.cursp();
    let index = scope.new_sym(session, &bytes)?;
    scope.genop_2(session, opcode::OP_LOADSYM, dst, index)?;
    scope.push_n(1)
}

fn gen_simple<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    if !val {
        return Ok(());
    }
    let Some(lit) = node.simple_lit() else {
        return Err(unsupported(&node, "literal"));
    };
    let op = match lit {
        SimpleLit::True => opcode::OP_LOADTRUE,
        SimpleLit::False => opcode::OP_LOADFALSE,
        SimpleLit::Nil => opcode::OP_LOADNIL,
        SimpleLit::SelfValue => opcode::OP_LOADSELF,
    };
    let (session, scope) = cg.current();
    let dst = scope.cursp();
    scope.genop_1(session, op, dst)?;
    scope.push_n(1)
}

fn gen_lvar_read<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    if !val {
        return Ok(());
    }
    let Some(target) = node.lvar_read() else {
        return Err(unsupported(&node, "variable read"));
    };
    let (session, scope) = cg.current();
    let depth = target.depth + u32::from(scope.for_depth);
    if depth == 0 {
        let dst = scope.cursp();
        let index = scope.lv_idx(&target.name);
        // Note the set peephole flag (`gen_move(..., 1)`).
        scope.gen_move(session, dst, index, true)?;
    } else {
        return Err(unsupported(&node, "upvar read"));
    }
    scope.push_n(1)
}

fn gen_lvar_write<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(target) = node.lvar_write() else {
        return Err(unsupported(&node, "variable write"));
    };
    let Some(rhs) = target.value else {
        return Err(unsupported(&node, "bare write target"));
    };
    codegen(cg, rhs, true)?;
    let (session, scope) = cg.current();
    scope.pop_n(1)?;
    let sp = scope.cursp();
    let depth = target.depth + u32::from(scope.for_depth);
    if depth != 0 {
        return Err(unsupported(&node, "upvar write"));
    }
    let index = scope.lv_idx(&target.name);
    if index != sp {
        scope.gen_move(session, index, sp, val)?;
    }
    if val {
        let (_, scope) = cg.current();
        scope.push_n(1)?;
    }
    Ok(())
}

/// Positional values with the `> limit` array packing (`gen_values`).
fn gen_values<N: BackendNode>(
    cg: &mut Codegen,
    items: Vec<N>,
    val: bool,
    limit: usize,
) -> Result<i32, Diagnostic> {
    if !val {
        for item in items {
            codegen(cg, item, false)?;
        }
        return Ok(0);
    }
    // The stack-flush path only triggers on splat/forwarding forms or past
    // 99 registers; both are gated out of P1, so plain codegen suffices.
    let mut count = 0;
    for item in items {
        codegen(cg, item, true)?;
        count += 1;
    }
    let limit = if limit == 0 { LIT_ARY_MAX } else { limit };
    if count > limit {
        let (session, scope) = cg.current();
        scope.pop_n(count as u16)?;
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_ARRAY, dst, count as u16)?;
        return Ok(-1);
    }
    Ok(count as i32)
}

/// `+`/`-` folding onto an integer load (`gen_addsub`).
fn gen_addsub(cg: &mut Codegen, op: u8, dst: u16) -> Result<(), Diagnostic> {
    enum Step {
        Plain,
        Fuse(u16),
    }
    let step = {
        let (session, scope) = cg.current();
        if scope.no_peephole(session) {
            Step::Plain
        } else {
            let data = scope.last_insn();
            match scope.int_operand(&data) {
                // Folding a negative would change the method sent on override.
                None => Step::Plain,
                Some(n) if !(0..=0xff).contains(&n) => Step::Plain,
                Some(n) => Step::Fuse(n as u16),
            }
        }
    };
    match step {
        Step::Plain => {
            let (session, scope) = cg.current();
            scope.genop_1(session, op, dst)
        }
        Step::Fuse(n) => {
            let (session, scope) = cg.current();
            scope.pc = scope.lastpc;
            let fused = if op == opcode::OP_ADD {
                opcode::OP_ADDI
            } else {
                opcode::OP_SUBI
            };
            scope.genop_2(session, fused, dst, n)
        }
    }
}

/// Unary `+@`/`-@` folding (`gen_uniop`).
fn gen_uniop(cg: &mut Codegen, name: &[u8], dst: u16) -> Result<bool, Diagnostic> {
    let folded = {
        let (session, scope) = cg.current();
        if scope.no_peephole(session) {
            None
        } else {
            let data = scope.last_insn();
            match scope.int_operand(&data) {
                None => None,
                Some(n) => {
                    if name == b"+" {
                        // Unary plus re-emits the same literal.
                        Some(n)
                    } else if name == b"-" {
                        if n == i64::MIN {
                            None
                        } else {
                            Some(-n)
                        }
                    } else {
                        None
                    }
                }
            }
        }
    };
    match folded {
        None => Ok(false),
        Some(n) => {
            let (session, scope) = cg.current();
            scope.pc = scope.lastpc;
            scope.gen_int(session, dst, n)?;
            Ok(true)
        }
    }
}

/// `[]` fusion (`gen_binop`, only `aref`).
fn gen_binop(cg: &mut Codegen, name: &[u8], dst: u16) -> Result<bool, Diagnostic> {
    if name != b"[]" {
        return Ok(false);
    }
    enum Step {
        No,
        Idx0(u16),
        Idx,
    }
    let step = {
        let (session, scope) = cg.current();
        if scope.no_peephole(session) {
            Step::No
        } else {
            let data = scope.last_insn();
            if data.insn == opcode::OP_LOADI_0
                && u32::from(data.a) == u32::from(dst) + 1
                && scope.lastpc != scope.lastlabel
            {
                let prev = scope.prev_pc(scope.lastpc);
                match opcode::decode_at(&scope.iseq, prev as usize) {
                    Some((data0, _))
                        if data0.insn == opcode::OP_MOVE && data0.a == dst && data0.b != dst =>
                    {
                        Step::Idx0(data0.b)
                    }
                    _ => Step::Idx,
                }
            } else {
                Step::Idx
            }
        }
    };
    match step {
        Step::No => Ok(false),
        Step::Idx0(reg) => {
            let (session, scope) = cg.current();
            scope.pc = scope.prev_pc(scope.lastpc);
            scope.genop_2(session, opcode::OP_GETIDX0, dst, reg)?;
            Ok(true)
        }
        Step::Idx => {
            let (session, scope) = cg.current();
            scope.genop_1(session, opcode::OP_GETIDX, dst)?;
            Ok(true)
        }
    }
}

/// Method call (`gen_call`; `recv_ready` paths belong to the gated
/// attribute-assign tranches).
fn gen_call<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(view) = node.call() else {
        return Err(unsupported(&node, "call"));
    };
    if view.attr_write {
        return Err(unsupported(&node, "attribute assignment"));
    }
    let safe = view.safe_nav;
    let name = view.name;
    let (mut noself, mut noop) = (false, false);
    let sp_save = cg.current().1.sp;
    match view.receiver {
        None => {
            noself = true;
            noop = true;
            cg.current().1.push_n(1)?;
        }
        Some(recv) if recv.kind_name() == "SelfNode" => {
            noself = true;
            noop = true;
            if safe {
                // `self&.m` loads the receiver for the nil check.
                codegen(cg, recv, true)?;
            } else {
                // `OP_SSEND` fills the receiver register itself.
                cg.current().1.push_n(1)?;
            }
        }
        Some(recv) => codegen(cg, recv, true)?,
    }
    let mut skip = JMPLINK_START;
    if safe {
        let (session, scope) = cg.current();
        let recv = scope.sp - 1;
        let dst = scope.cursp();
        scope.gen_move(session, dst, recv, true)?;
        skip = scope.genjmp2(
            session,
            opcode::OP_JMPNIL,
            scope.cursp(),
            JMPLINK_START,
            val,
        )?;
    }
    let mut nargs: i32 = 0;
    if let Some(args) = view.args {
        let Some(items) = args.call_args() else {
            return Err(unsupported(&node, "complex arguments"));
        };
        if !items.is_empty() {
            nargs = gen_values(cg, items, true, CALL_ARG_LIMIT)?;
            if nargs < 0 {
                // Variable length (only via the gated splat flush in C).
                noop = true;
                nargs = 15;
                cg.current().1.push_n(1)?;
            }
        }
    }
    if view.block.is_some() {
        return Err(unsupported(&node, "block argument"));
    }
    {
        let (_, scope) = cg.current();
        scope.push_n(1)?;
        scope.pop_n(1)?;
        scope.sp = sp_save;
    }
    emit_call(cg, noself, noop, &name, nargs, val, safe, skip)
}

/// Tail of `gen_call`: operator specials and `SEND` selection.
#[allow(clippy::too_many_arguments)]
fn emit_call(
    cg: &mut Codegen,
    noself: bool,
    noop: bool,
    name: &[u8],
    nargs: i32,
    val: bool,
    safe: bool,
    skip: u32,
) -> Result<(), Diagnostic> {
    let dst = cg.current().1.cursp();
    if !noop && name == b"+" && nargs == 1 {
        gen_addsub(cg, opcode::OP_ADD, dst)?;
    } else if !noop && name == b"-" && nargs == 1 {
        gen_addsub(cg, opcode::OP_SUB, dst)?;
    } else if !noop && name == b"*" && nargs == 1 {
        let (session, scope) = cg.current();
        scope.genop_1(session, opcode::OP_MUL, dst)?;
    } else if !noop && name == b"/" && nargs == 1 {
        let (session, scope) = cg.current();
        scope.genop_1(session, opcode::OP_DIV, dst)?;
    } else if !noop && name == b"<" && nargs == 1 {
        let (session, scope) = cg.current();
        scope.genop_1(session, opcode::OP_LT, dst)?;
    } else if !noop && name == b"<=" && nargs == 1 {
        let (session, scope) = cg.current();
        scope.genop_1(session, opcode::OP_LE, dst)?;
    } else if !noop && name == b">" && nargs == 1 {
        let (session, scope) = cg.current();
        scope.genop_1(session, opcode::OP_GT, dst)?;
    } else if !noop && name == b">=" && nargs == 1 {
        let (session, scope) = cg.current();
        scope.genop_1(session, opcode::OP_GE, dst)?;
    } else if !noop && name == b"==" && nargs == 1 {
        let (session, scope) = cg.current();
        scope.genop_1(session, opcode::OP_EQ, dst)?;
    } else if !noop && nargs == 0 && gen_uniop(cg, name, dst)? {
        // A literal absorbed its sign.
    } else if !noop && nargs == 1 && gen_binop(cg, name, dst)? {
        // An index opcode was emitted.
    } else {
        let sym = {
            let (session, scope) = cg.current();
            scope.new_sym(session, name)?
        };
        if noself {
            let (session, scope) = cg.current();
            if nargs == 0 {
                scope.genop_2(session, opcode::OP_SSEND0, dst, sym)?;
            } else {
                scope.genop_3(session, opcode::OP_SSEND, dst, sym, nargs as u8)?;
            }
        } else {
            let (session, scope) = cg.current();
            if nargs == 0 {
                scope.genop_2(session, opcode::OP_SEND0, dst, sym)?;
            } else {
                scope.genop_3(session, opcode::OP_SEND, dst, sym, nargs as u8)?;
            }
        }
    }
    if safe {
        cg.current().1.dispatch(skip)?;
    }
    if val {
        cg.current().1.push_n(1)?;
    }
    Ok(())
}

fn gen_if<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(view) = node.if_branch() else {
        return Err(unsupported(&node, "conditional"));
    };
    let Some(predicate) = view.predicate else {
        let Some(then_body) = view.then_body else {
            return Err(unsupported(&node, "empty conditional"));
        };
        return codegen(cg, then_body, val);
    };
    if const_true(&predicate) {
        let Some(then_body) = view.then_body else {
            return Err(unsupported(&node, "empty then"));
        };
        return codegen(cg, then_body, val);
    }
    if const_false(&predicate) {
        let Some(else_body) = view.else_body else {
            return Err(unsupported(&node, "empty else"));
        };
        return codegen(cg, else_body, val);
    }
    // `nil?` predicate shortcut.
    let mut nil_check = false;
    if predicate.kind_name() == "CallNode" {
        if let Some(call) = predicate.call() {
            if call.name == b"nil?" && call.args.is_none() {
                nil_check = true;
                match call.receiver {
                    Some(recv) => codegen(cg, recv, true)?,
                    None => {
                        let (session, scope) = cg.current();
                        let dst = scope.cursp();
                        scope.genop_1(session, opcode::OP_LOADSELF, dst)?;
                        scope.push_n(1)?;
                    }
                }
            }
        }
    }
    if !nil_check {
        codegen(cg, predicate, true)?;
    }
    {
        let (_, scope) = cg.current();
        scope.pop_n(1)?;
    }
    if val || view.then_body.is_some() {
        if nil_check {
            let pos2: u32;
            let pos1: u32;
            {
                let (session, scope) = cg.current();
                let cur = scope.cursp();
                pos2 = scope.genjmp2(session, opcode::OP_JMPNIL, cur, JMPLINK_START, val)?;
                pos1 = scope.genjmp(opcode::OP_JMP, JMPLINK_START)?;
                scope.dispatch(pos2)?;
            }
            let Some(then_body) = view.then_body else {
                return Err(unsupported(&node, "empty then"));
            };
            codegen(cg, then_body, val)?;
            if val {
                let (_, scope) = cg.current();
                scope.pop_n(1)?;
            }
            if view.else_body.is_some() || val {
                let pos2b: u32;
                {
                    let (_, scope) = cg.current();
                    pos2b = scope.genjmp(opcode::OP_JMP, JMPLINK_START)?;
                    scope.dispatch(pos1)?;
                }
                let Some(else_body) = view.else_body else {
                    return Err(unsupported(&node, "empty else"));
                };
                codegen(cg, else_body, val)?;
                let (_, scope) = cg.current();
                scope.dispatch(pos2b)?;
            } else {
                let (_, scope) = cg.current();
                scope.dispatch(pos1)?;
            }
        } else {
            let pos1: u32;
            {
                let (session, scope) = cg.current();
                let cur = scope.cursp();
                pos1 = scope.genjmp2(session, opcode::OP_JMPNOT, cur, JMPLINK_START, val)?;
            }
            let Some(then_body) = view.then_body else {
                return Err(unsupported(&node, "empty then"));
            };
            codegen(cg, then_body, val)?;
            if val {
                let (_, scope) = cg.current();
                scope.pop_n(1)?;
            }
            if view.else_body.is_some() || val {
                let pos2: u32;
                {
                    let (_, scope) = cg.current();
                    pos2 = scope.genjmp(opcode::OP_JMP, JMPLINK_START)?;
                    scope.dispatch(pos1)?;
                }
                let Some(else_body) = view.else_body else {
                    return Err(unsupported(&node, "empty else"));
                };
                codegen(cg, else_body, val)?;
                let (_, scope) = cg.current();
                scope.dispatch(pos2)?;
            } else {
                let (_, scope) = cg.current();
                scope.dispatch(pos1)?;
            }
        }
    } else if let Some(else_body) = view.else_body {
        if nil_check {
            let pos1: u32;
            {
                let (session, scope) = cg.current();
                let cur = scope.cursp();
                pos1 = scope.genjmp2(session, opcode::OP_JMPNIL, cur, JMPLINK_START, val)?;
            }
            codegen(cg, else_body, val)?;
            let (_, scope) = cg.current();
            scope.dispatch(pos1)?;
        } else {
            let pos1: u32;
            {
                let (session, scope) = cg.current();
                let cur = scope.cursp();
                pos1 = scope.genjmp2(session, opcode::OP_JMPIF, cur, JMPLINK_START, val)?;
            }
            codegen(cg, else_body, val)?;
            let (_, scope) = cg.current();
            scope.dispatch(pos1)?;
        }
    } else if val && !nil_check {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
        scope.push_n(1)?;
    }
    Ok(())
}

fn gen_array<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(items) = node.array_elements() else {
        return Err(unsupported(&node, "splat array"));
    };
    let count = gen_values(cg, items, val, 0)?;
    if val {
        if count >= 0 {
            let (session, scope) = cg.current();
            scope.pop_n(count as u16)?;
            let dst = scope.cursp();
            scope.genop_2(session, opcode::OP_ARRAY, dst, count as u16)?;
        }
        let (_, scope) = cg.current();
        scope.push_n(1)?;
    }
    Ok(())
}

fn gen_while<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(view) = node.while_loop() else {
        return Err(unsupported(&node, "loop"));
    };
    if view.begin_modifier {
        return Err(unsupported(&node, "begin-modifier loop"));
    }
    let Some(predicate) = view.predicate else {
        return Err(unsupported(&node, "loop without condition"));
    };
    if const_true(&predicate) {
        if view.is_until {
            if val {
                let (session, scope) = cg.current();
                let dst = scope.cursp();
                scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
                scope.push_n(1)?;
            }
            return Ok(());
        }
    } else if const_false(&predicate) && !view.is_until {
        if val {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
            scope.push_n(1)?;
        }
        return Ok(());
    }
    {
        let (_, scope) = cg.current();
        scope.loop_push(LoopType::Normal);
        if !val {
            scope.loops.last_mut().expect("loop").reg = -1;
        }
    }
    let pc0: u32;
    {
        let (_, scope) = cg.current();
        pc0 = scope.new_label();
    }
    codegen(cg, predicate, true)?;
    {
        let (_, scope) = cg.current();
        scope.pop_n(1)?;
    }
    let pos: u32;
    {
        let (session, scope) = cg.current();
        let cur = scope.cursp();
        let op = if view.is_until {
            opcode::OP_JMPIF
        } else {
            opcode::OP_JMPNOT
        };
        pos = scope.genjmp2(session, op, cur, JMPLINK_START, false)?;
    }
    {
        let (_, scope) = cg.current();
        scope.new_label();
        scope.genop_0(opcode::OP_NOP)?;
    }
    let Some(body) = view.body else {
        return Err(unsupported(&node, "loop without body"));
    };
    codegen(cg, body, false)?;
    {
        let (_, scope) = cg.current();
        scope.genjmp(opcode::OP_JMP, pc0)?;
        scope.dispatch(pos)?;
    }
    {
        let (session, scope) = cg.current();
        scope.loop_pop(session, val)?;
    }
    Ok(())
}

fn gen_logic<N: BackendNode>(
    cg: &mut Codegen,
    node: N,
    val: bool,
    is_or: bool,
) -> Result<(), Diagnostic> {
    let Some((left, right)) = node.logic() else {
        return Err(unsupported(&node, "logic operands"));
    };
    if const_true(&left) {
        // `a && b` with truthy `a` is `b`; `a || b` with truthy `a` is `a`.
        return codegen(cg, if is_or { left } else { right }, val);
    }
    if const_false(&left) {
        // `a && b` with falsy `a` is `a`; `a || b` with falsy `a` is `b`.
        return codegen(cg, if is_or { right } else { left }, val);
    }
    codegen(cg, left, true)?;
    {
        let (_, scope) = cg.current();
        scope.pop_n(1)?;
    }
    let pos: u32;
    {
        let (session, scope) = cg.current();
        let cur = scope.cursp();
        let op = if is_or {
            opcode::OP_JMPIF
        } else {
            opcode::OP_JMPNOT
        };
        pos = scope.genjmp2(session, op, cur, JMPLINK_START, val)?;
    }
    codegen(cg, right, val)?;
    let (_, scope) = cg.current();
    scope.dispatch(pos)?;
    Ok(())
}

/// Compile a parsed program root to a RITE binary (P1 entry).
///
/// The reference dumps with flags `0`, so `stripped` currently changes no
/// byte; both modes must match the same golden (see `agents/progress.md`).
pub fn compile_prism<N: BackendNode>(
    root: N,
    _opts: &CompileOptions,
) -> Result<Vec<u8>, Diagnostics> {
    let mut cg = Codegen::new();
    codegen(&mut cg, root, true).map_err(|single| Diagnostics {
        entries: vec![single],
    })?;
    let Some(irep) = cg.root.take() else {
        return Err(Diagnostics {
            entries: vec![Diagnostic {
                message: "expected a program root".to_owned(),
                start: 0,
                end: 0,
            }],
        });
    };
    let lvar_syms = if cg.session.any_lv {
        Some(cg.session.lvar_names.clone())
    } else {
        None
    };
    Ok(write_rite(&RiteModel {
        root: irep,
        lvar_syms,
        debug_raw: None,
    }))
}
