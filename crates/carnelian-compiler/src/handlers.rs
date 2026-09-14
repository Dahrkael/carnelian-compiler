//! Node handlers: port of the `codegen()` tranches covering the P1 corpus.
//!
//! Dispatch is on `kind_name` (1:1 with the C `switch` on node type); values
//! travel through `BackendNode`, so FFI and owned frontends share handlers.

use carnelian_ast::view::{BackendNode, LvarRef, SimpleLit};

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
const VAL_STACK_MAX: u32 = 99;
/// `CALL_MAXARGS`: argument count marking an array-gathered list (`gen_call`).
const CALL_MAXARGS: i32 = 15;
/// Catch handler kinds (`enum mrc_catch_type`).
const CATCH_RESCUE: u8 = 0;
const CATCH_ENSURE: u8 = 1;
/// `$!` symbol (`MRC_SYM_2(errinfo)`).
const ERRINFO: &[u8] = b"$!";
/// Stack threshold before flushing pending hash pairs: `GEN_VAL_STACK_MAX`,
/// lifted past `INT16_MAX` once the cursor itself is past the small limit
/// (kept separate so the future `gen_values` flush can share it).
fn val_stack_limit(cursp: u16) -> u32 {
    if cursp >= LIT_ARY_MAX as u16 {
        i16::MAX as u32
    } else {
        VAL_STACK_MAX
    }
}

/// Counts as pool operands without silent truncation (the `sp` accounting in
/// `push_n` errors first in practice).
fn too_complex() -> Diagnostic {
    Diagnostic {
        message: "too complex expression".to_owned(),
        start: 0,
        end: 0,
    }
}

fn count_u16(count: i32) -> Result<u16, Diagnostic> {
    u16::try_from(count).map_err(|_| too_complex())
}

fn pair_pop(len: i32, extra: u16) -> Result<u16, Diagnostic> {
    count_u16(len)?
        .checked_mul(2)
        .and_then(|doubled| doubled.checked_add(extra))
        .ok_or_else(too_complex)
}

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

fn unsupported_text(what: &str) -> Diagnostic {
    Diagnostic {
        message: format!("unsupported {what} in P1"),
        start: 0,
        end: 0,
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
/// An empty list still emits `LOADNIL` like the C arm does.
fn gen_list<N: BackendNode>(cg: &mut Codegen, items: Vec<N>, val: bool) -> Result<(), Diagnostic> {
    if items.is_empty() {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
        scope.push_n(1)?;
        return Ok(());
    }
    let last = items.len() - 1;
    for (index, item) in items.into_iter().enumerate() {
        codegen(cg, item, index == last && val)?;
    }
    Ok(())
}

/// Null-tree guard shared by optional branches: `None` (a null subtree)
/// emits `LOADNIL` only when valued; `Some` follows the statements arm.
fn gen_branch<N: BackendNode>(
    cg: &mut Codegen,
    items: Option<Vec<N>>,
    val: bool,
) -> Result<(), Diagnostic> {
    match items {
        None => {
            if val {
                emit_absent_else(cg)?;
            }
            Ok(())
        }
        Some(list) => gen_list(cg, list, val),
    }
}

/// A missing `else` clause where presence is implied by `val`.
fn emit_absent_else(cg: &mut Codegen) -> Result<(), Diagnostic> {
    let (session, scope) = cg.current();
    let dst = scope.cursp();
    scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
    scope.push_n(1)
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
            // In this arm the node is an `else`, so `None` is a null subtree.
            gen_branch(cg, node.else_body(), val)
        }
        "IntegerNode" => gen_integer(cg, node, val),
        "FloatNode" => gen_float(cg, node, val),
        "StringNode" => gen_string(cg, node, val),
        "SymbolNode" => gen_symbol(cg, node, val),
        "CallNode" => gen_call(cg, node, val),
        "IfNode" | "UnlessNode" => gen_if(cg, node, val),
        "ArrayNode" => gen_array(cg, node, val),
        "HashNode" | "KeywordHashNode" => gen_hash_lit(cg, node, val),
        "CaseNode" => gen_case(cg, node, val),
        "InterpolatedStringNode" => gen_interp_string(cg, node, val),
        "EmbeddedStatementsNode" => gen_branch(cg, node.embedded_body(), val),
        "EmbeddedVariableNode" => {
            let Some(variable) = node.embedded_var() else {
                return Err(unsupported(&node, "embedded variable"));
            };
            codegen(cg, variable, val)
        }
        "WhileNode" | "UntilNode" => gen_while(cg, node, val),
        "AndNode" => gen_logic(cg, node, val, false),
        "OrNode" => gen_logic(cg, node, val, true),
        "LocalVariableReadNode" => gen_lvar_read(cg, node, val),
        "LocalVariableWriteNode" => {
            let Some(target) = node.lvar_write() else {
                return Err(unsupported(&node, "variable write"));
            };
            let value = target.value;
            gen_assignment(cg, node, value, 0, val)
        }
        "BeginNode" => gen_begin_node(cg, node, val),
        "RescueModifierNode" => gen_rescue_modifier(cg, node, val),
        "MultiWriteNode" => gen_multi_write(cg, node, val),
        "SplatNode" => gen_splat(cg, node, val),
        "RetryNode" => gen_retry(cg, node, val),
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
    match lit {
        carnelian_ast::view::IntegerLit::I64(value) => {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.gen_int(session, dst, value)?;
            scope.push_n(1)
        }
        carnelian_ast::view::IntegerLit::Bigint { digits, negative } => {
            let index = {
                let (session, scope) = cg.current();
                scope.new_litbint(session, &digits, 10, negative)? as u16
            };
            emit_load2(cg, opcode::OP_LOADL, index)
        }
    }
}

/// Two-operand pool load at the cursor (`OP_STRING` and friends).
fn emit_load2(cg: &mut Codegen, op: u8, index: u16) -> Result<(), Diagnostic> {
    let (session, scope) = cg.current();
    let dst = scope.cursp();
    scope.genop_2(session, op, dst, index)?;
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
    let index = scope.new_lit_float(session, value)? as u16;
    emit_load2(cg, opcode::OP_LOADL, index)
}

fn gen_string<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    if !val {
        return Ok(());
    }
    let Some(bytes) = node.string_lit() else {
        return Err(unsupported(&node, "string literal"));
    };
    let (session, scope) = cg.current();
    let index = scope.new_lit_str(session, &bytes)? as u16;
    emit_load2(cg, opcode::OP_STRING, index)
}

fn gen_symbol<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    if !val {
        return Ok(());
    }
    let Some(bytes) = node.symbol_lit() else {
        return Err(unsupported(&node, "symbol literal"));
    };
    let (session, scope) = cg.current();
    let index = scope.new_sym(session, &bytes)?;
    emit_load2(cg, opcode::OP_LOADSYM, index)
}

/// Interpolated string (`PM_INTERPOLATED_STRING_NODE`): `STRING` parts
/// joined with `STRCAT`, with a leading empty literal unless the first part
/// is already a string (so `STRCAT` never mutates a shared literal).
fn gen_interp_string<N: BackendNode>(
    cg: &mut Codegen,
    node: N,
    val: bool,
) -> Result<(), Diagnostic> {
    let Some(parts) = node.string_parts() else {
        return Err(unsupported(&node, "interpolated string"));
    };
    let Some(first) = parts.first() else {
        return Err(unsupported(&node, "empty interpolated string"));
    };
    let str_begin = first.kind_name() != "StringNode";
    if val {
        if str_begin {
            let (session, scope) = cg.current();
            let index = scope.new_lit_str(session, b"")? as u16;
            emit_load2(cg, opcode::OP_STRING, index)?;
        }
        for (index, part) in parts.into_iter().enumerate() {
            codegen(cg, part, true)?;
            cg.current().1.pop_n(1)?;
            if str_begin || index > 0 {
                let (session, scope) = cg.current();
                scope.pop_n(1)?;
                let dst = scope.cursp();
                scope.genop_1(session, opcode::OP_STRCAT, dst)?;
            }
            cg.current().1.push_n(1)?;
        }
    } else {
        // String parts need no runtime value; only embedded code may have
        // side effects, and a `NOVAL` codegen leaves nothing to pop.
        for part in parts {
            if part.kind_name() != "StringNode" {
                codegen(cg, part, false)?;
            }
        }
    }
    Ok(())
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

/// Local assignment (`gen_assignment_lvar`): a move into the local slot.
fn gen_assignment_lvar(
    cg: &mut Codegen,
    sp: u16,
    name: &[u8],
    depth: u32,
    val: bool,
) -> Result<(), Diagnostic> {
    if depth == 0 {
        let index = cg.current().1.lv_idx(name);
        if index != sp {
            let (session, scope) = cg.current();
            scope.gen_move(session, index, sp, val)?;
        }
        Ok(())
    } else {
        Err(unsupported_text("upvar write"))
    }
}

/// Assignment to one target (`gen_assignment`): local writes/targets and
/// nested multiple targets; other target kinds belong to later tranches.
fn gen_assignment<N: BackendNode>(
    cg: &mut Codegen,
    tree: N,
    rhs: Option<N>,
    mut sp: u16,
    val: bool,
) -> Result<(), Diagnostic> {
    match tree.kind_name() {
        "LocalVariableWriteNode" | "LocalVariableTargetNode" | "RequiredParameterNode" => {
            if let Some(value) = rhs {
                codegen(cg, value, true)?;
                cg.current().1.pop_n(1)?;
                sp = cg.current().1.cursp();
            }
            let target = if tree.kind_name() == "LocalVariableWriteNode" {
                tree.lvar_write().map(|write| LvarRef {
                    name: write.name,
                    depth: write.depth,
                })
            } else {
                tree.lvar_target()
            };
            let Some(target) = target else {
                return Err(unsupported(&tree, "assignment target"));
            };
            let depth = target.depth + u32::from(cg.current().1.for_depth);
            gen_assignment_lvar(cg, sp, &target.name, depth, val)?;
        }
        "MultiTargetNode" => {
            let Some(view) = tree.multi_target_view() else {
                return Err(unsupported(&tree, "assignment target"));
            };
            gen_massignment(cg, view.lefts, view.rest, view.rights, i32::from(sp), val)?;
        }
        _ => return Err(unsupported(&tree, "assignment target")),
    }
    if val {
        cg.current().1.push_n(1)?;
    }
    Ok(())
}

/// Positional values (`gen_values_upto` over all arguments): plain values
/// stack up; a splat or the size threshold flushes them into an array with
/// `ARRAY`/`ARYPUSH`, `ARYCAT` absorbs each splat, and the `-1` protocol
/// tells the caller a variable-length list sits at the cursor.
fn gen_values<N: BackendNode>(
    cg: &mut Codegen,
    items: Vec<N>,
    val: bool,
    limit: usize,
) -> Result<i32, Diagnostic> {
    if items.is_empty() {
        return Ok(0);
    }
    let mut limit = if limit == 0 { LIT_ARY_MAX } else { limit };
    let mut n: i32 = 0;
    let mut first = true;
    let slimit = val_stack_limit(cg.current().1.cursp());

    if !val {
        for item in items {
            codegen(cg, item, false)?;
            n += 1;
        }
        return Ok(n);
    }

    for item in items {
        if item.kind_name() == "KeywordHashNode" {
            break;
        }
        let is_splat = item.kind_name() == "SplatNode";
        let is_forwarding = item.kind_name() == "ForwardingArgumentsNode";
        if is_splat || is_forwarding || u32::from(cg.current().1.cursp()) >= slimit {
            cg.current().1.pop_n(n as u16)?;
            if first {
                if n == 0 {
                    let (session, scope) = cg.current();
                    let dst = scope.cursp();
                    scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
                } else {
                    let (session, scope) = cg.current();
                    let dst = scope.cursp();
                    scope.genop_2(session, opcode::OP_ARRAY, dst, n as u16)?;
                }
                cg.current().1.push_n(1)?;
                first = false;
                limit = LIT_ARY_MAX;
            } else if n > 0 {
                cg.current().1.pop_n(1)?;
                let (session, scope) = cg.current();
                let dst = scope.cursp();
                scope.genop_2(session, opcode::OP_ARYPUSH, dst, n as u16)?;
                cg.current().1.push_n(1)?;
            }
            n = 0;
        }
        if is_splat {
            let Some(inner) = item.splat_value() else {
                return Err(unsupported(&item, "splat"));
            };
            match inner {
                Some(expression) => codegen(cg, expression, val)?,
                None => gen_lvar(cg, b"*", 0)?,
            }
            cg.current().1.pop_n(2)?;
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_1(session, opcode::OP_ARYCAT, dst)?;
            cg.current().1.push_n(1)?;
        } else if is_forwarding {
            return Err(unsupported(&item, "forwarding arguments"));
        } else {
            codegen(cg, item, val)?;
            n += 1;
        }
    }

    if !first {
        cg.current().1.pop_n(1)?;
        if n > 0 {
            cg.current().1.pop_n(n as u16)?;
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_2(session, opcode::OP_ARYPUSH, dst, n as u16)?;
        }
        return Ok(-1);
    } else if n > limit as i32 {
        cg.current().1.pop_n(n as u16)?;
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_ARRAY, dst, n as u16)?;
        return Ok(-1);
    }
    Ok(n)
}

/// Local variable load into the cursor (`gen_lvar`).
fn gen_lvar(cg: &mut Codegen, name: &[u8], depth: u32) -> Result<(), Diagnostic> {
    if depth == 0 {
        let index = cg.current().1.lv_idx(name);
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.gen_move(session, dst, index, true)?;
    } else {
        return Err(unsupported_text("upvar read"));
    }
    cg.current().1.push_n(1)
}

/// Splatted value as an expression (`PM_SPLAT_NODE`).
fn gen_splat<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(splat) = node.splat_value() else {
        return Err(unsupported(&node, "splat"));
    };
    match splat {
        Some(expression) => codegen(cg, expression, val),
        None => {
            if val {
                gen_lvar(cg, b"*", 0)?;
            }
            Ok(())
        }
    }
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
                && data.a == u32::from(dst) + 1
                && scope.lastpc != scope.lastlabel
            {
                let prev = scope.prev_pc(scope.lastpc);
                match opcode::decode_at(&scope.iseq, prev as usize) {
                    Some((data0, _))
                        if data0.insn == opcode::OP_MOVE
                            && data0.a == u32::from(dst)
                            && data0.b != dst =>
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
    let mut nk: i32 = 0;
    if let Some(args) = view.args {
        if args.args_forwarding() {
            return Err(unsupported(&node, "forwarding arguments"));
        }
        let Some(mut items) = args.call_args() else {
            return Err(unsupported(&node, "complex arguments"));
        };
        // Keyword arguments follow the positional ones in a single
        // `KeywordHashNode`; `gen_values` stops at it.
        let keywords = match items
            .iter()
            .position(|item| item.kind_name() == "KeywordHashNode")
        {
            Some(first) => items.split_off(first),
            None => Vec::new(),
        };
        if !items.is_empty() {
            nargs = gen_values(cg, items, true, CALL_ARG_LIMIT)?;
            if nargs < 0 {
                noop = true;
                nargs = CALL_MAXARGS;
                cg.current().1.push_n(1)?;
            }
        }
        for keyword in keywords {
            let Some(elements) = keyword.hash_elements() else {
                return Err(unsupported(&keyword, "keyword arguments"));
            };
            noop = true;
            nk = gen_hash(cg, elements, true, CALL_ARG_LIMIT)?;
            if nk < 0 {
                nk = CALL_MAXARGS;
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
    emit_call(cg, noself, noop, &name, nargs, nk, val, safe, skip)
}

/// Direct single-operand operator selection (`*`, `/`, comparisons).
fn direct_op(name: &[u8]) -> Option<u8> {
    match name {
        b"*" => Some(opcode::OP_MUL),
        b"/" => Some(opcode::OP_DIV),
        b"<" => Some(opcode::OP_LT),
        b"<=" => Some(opcode::OP_LE),
        b">" => Some(opcode::OP_GT),
        b">=" => Some(opcode::OP_GE),
        b"==" => Some(opcode::OP_EQ),
        _ => None,
    }
}

/// Tail of `gen_call`: operator specials and `SEND` selection.
#[allow(clippy::too_many_arguments)]
fn emit_call(
    cg: &mut Codegen,
    noself: bool,
    noop: bool,
    name: &[u8],
    nargs: i32,
    nk: i32,
    val: bool,
    safe: bool,
    skip: u32,
) -> Result<(), Diagnostic> {
    let dst = cg.current().1.cursp();
    if !noop && name == b"+" && nargs == 1 {
        gen_addsub(cg, opcode::OP_ADD, dst)?;
    } else if !noop && name == b"-" && nargs == 1 {
        gen_addsub(cg, opcode::OP_SUB, dst)?;
    } else if !noop && nargs == 1 {
        if let Some(op) = direct_op(name) {
            let (session, scope) = cg.current();
            scope.genop_1(session, op, dst)?;
        } else if gen_binop(cg, name, dst)? {
            // An index opcode was emitted.
        } else {
            send_call(cg, noself, name, nargs, nk, dst)?;
        }
    } else if !noop && nargs == 0 && gen_uniop(cg, name, dst)? {
        // A literal absorbed its sign.
    } else {
        send_call(cg, noself, name, nargs, nk, dst)?;
    }
    if safe {
        cg.current().1.dispatch(skip)?;
    }
    if val {
        cg.current().1.push_n(1)?;
    }
    Ok(())
}

/// Generic `SEND`/`SSEND` emission. The argument byte packs the positional
/// count in the low nibble and the keyword-table flag in the high nibble
/// (`n|(nk<<4)`), with `CALL_MAXARGS` marking a variable-length list.
fn send_call(
    cg: &mut Codegen,
    noself: bool,
    name: &[u8],
    nargs: i32,
    nk: i32,
    dst: u16,
) -> Result<(), Diagnostic> {
    let sym = {
        let (session, scope) = cg.current();
        scope.new_sym(session, name)?
    };
    let packed = ((nargs as u8) & 0x0f) | (((nk as u8) & 0x0f) << 4);
    if noself {
        let (session, scope) = cg.current();
        if nargs == 0 && nk == 0 {
            scope.genop_2(session, opcode::OP_SSEND0, dst, sym)?;
        } else {
            scope.genop_3(session, opcode::OP_SSEND, dst, sym, packed)?;
        }
    } else {
        let (session, scope) = cg.current();
        if nargs == 0 && nk == 0 {
            scope.genop_2(session, opcode::OP_SEND0, dst, sym)?;
        } else {
            scope.genop_3(session, opcode::OP_SEND, dst, sym, packed)?;
        }
    }
    Ok(())
}

fn gen_if<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(view) = node.if_branch() else {
        return Err(unsupported(&node, "conditional"));
    };
    let Some(predicate) = view.predicate else {
        // No condition: only the `else` side runs (possibly both null).
        return match view.else_body {
            Some(body) => codegen(cg, body, val),
            None => gen_branch(cg, None::<Vec<N>>, val),
        };
    };
    if const_true(&predicate) {
        return gen_branch(cg, view.then_body, val);
    }
    if const_false(&predicate) {
        return gen_branch(cg, view.else_body.map(|body| vec![body]), val);
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
            let then_body = view.then_body;
            gen_branch(cg, then_body, val)?;
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
                // A missing clause here implies `val` (see the guard above).
                match view.else_body {
                    Some(else_body) => codegen(cg, else_body, val)?,
                    None => emit_absent_else(cg)?,
                }
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
            gen_branch(cg, view.then_body, val)?;
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
                // A missing clause here implies `val` (see the guard above).
                match view.else_body {
                    Some(else_body) => codegen(cg, else_body, val)?,
                    None => emit_absent_else(cg)?,
                }
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

/// Hash literal (`gen_hash` shared by `HashNode` and `KeywordHashNode`).
fn gen_hash_lit<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(items) = node.hash_elements() else {
        return Err(unsupported(&node, "hash literal"));
    };
    let count = gen_hash(cg, items, val, LIT_ARY_MAX)?;
    if val && count >= 0 {
        flush_hash_pairs(cg, count, false)?;
    }
    Ok(())
}

/// Hash construction (`gen_hash`): codes the pairs, then reports the pending
/// count, or `-1` when a table op already ran (splat or `limit` overflow).
fn gen_hash<N: BackendNode>(
    cg: &mut Codegen,
    items: Vec<N>,
    val: bool,
    limit: usize,
) -> Result<i32, Diagnostic> {
    let slimit: u32 = val_stack_limit(cg.current().1.cursp());
    let mut len: i32 = 0;
    let mut update = false;
    let mut first = true;
    for item in items {
        if item.kind_name() == "AssocSplatNode" {
            let Some(inner) = item.assoc_splat_value() else {
                return Err(unsupported(&item, "associative splat"));
            };
            if val && first {
                let (session, scope) = cg.current();
                let dst = scope.cursp();
                scope.genop_2(session, opcode::OP_HASH, dst, 0)?;
                scope.push_n(1)?;
                update = true;
            } else if val && len > 0 {
                flush_hash_pairs(cg, len, update)?;
            }
            match inner {
                Some(value) => codegen(cg, value, val)?,
                // A bare `**` loads the anonymous keyword rest local.
                None => {
                    if val {
                        let (session, scope) = cg.current();
                        let dst = scope.cursp();
                        let index = scope.lv_idx(b"**");
                        scope.gen_move(session, dst, index, true)?;
                        scope.push_n(1)?;
                    }
                }
            }
            if val && (len > 0 || update) {
                let (session, scope) = cg.current();
                scope.pop_n(1)?;
                scope.pop_n(1)?;
                let dst = scope.cursp();
                scope.genop_1(session, opcode::OP_HASHCAT, dst)?;
                scope.push_n(1)?;
            }
            update = true;
            len = 0;
        } else {
            let Some((key, value)) = item.assoc_pair() else {
                return Err(unsupported(&item, "hash element"));
            };
            codegen(cg, key, val)?;
            codegen(cg, value, val)?;
            len += 1;
        }
        if val && u32::from(cg.current().1.cursp()) >= slimit {
            flush_hash_pairs(cg, len, update)?;
            update = true;
            len = 0;
        }
        first = false;
    }
    if val && len > limit as i32 {
        let (session, scope) = cg.current();
        scope.pop_n(pair_pop(len, 0)?)?;
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_HASH, dst, count_u16(len)?)?;
        scope.push_n(1)?;
        return Ok(-1);
    }
    if update {
        if val && len > 0 {
            let (session, scope) = cg.current();
            scope.pop_n(pair_pop(len, 1)?)?;
            let dst = scope.cursp();
            scope.genop_2(session, opcode::OP_HASHADD, dst, count_u16(len)?)?;
            scope.push_n(1)?;
        }
        return Ok(-1);
    }
    Ok(len)
}

/// Fold pending pairs into the table under construction (`OP_HASH` for the
/// first group, `OP_HASHADD` once a table exists). The destination is read
/// after the pops in each arm: the `HASHADD` arm pops one more slot.
fn flush_hash_pairs(cg: &mut Codegen, len: i32, update: bool) -> Result<(), Diagnostic> {
    let (session, scope) = cg.current();
    scope.pop_n(pair_pop(len, 0)?)?;
    if !update {
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_HASH, dst, count_u16(len)?)?;
    } else {
        scope.pop_n(1)?;
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_HASHADD, dst, count_u16(len)?)?;
    }
    scope.push_n(1)
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
        let redo = scope.new_label();
        scope.loops.last_mut().expect("loop").pc1 = redo;
        scope.genop_0(opcode::OP_NOP)?;
    }
    // A null body codes nothing; an empty node still emits `LOADNIL`.
    gen_branch(cg, view.body, false)?;
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

/// `case`/`when`/`else` (`PM_CASE_NODE`): `===` dispatch over the subject,
/// `__case_eqq` for splat conditions, valued-tail `LOADNIL` merge.
fn gen_case<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(view) = node.case_view() else {
        return Err(unsupported(&node, "case"));
    };
    let mut pos3 = JMPLINK_START;
    let mut head: Option<u16> = None;
    if let Some(predicate) = view.predicate {
        let subject = cg.current().1.cursp();
        codegen(cg, predicate, true)?;
        head = Some(subject);
    }
    for when in view.whens {
        let Some(when_view) = when.when_view() else {
            return Err(unsupported(&when, "when"));
        };
        let mut pos2 = JMPLINK_START;
        for cond in when_view.conditions {
            let splat = cond.kind_name() == "SplatNode";
            if splat {
                let Some(inner) = cond.splat_value() else {
                    return Err(unsupported(&cond, "splat condition"));
                };
                match inner {
                    Some(value) => codegen(cg, value, true)?,
                    // A bare `*` codes a null subject comparison operand.
                    None => emit_absent_else(cg)?,
                }
            } else {
                codegen(cg, cond, true)?;
            }
            if let Some(subject) = head {
                let (session, scope) = cg.current();
                let dst = scope.cursp();
                scope.gen_move(session, dst, subject, false)?;
                scope.push_n(2)?;
                scope.pop_n(3)?;
                let name: &[u8] = if splat { b"__case_eqq" } else { b"===" };
                let sym = scope.new_sym(session, name)?;
                let dst = scope.cursp();
                scope.genop_3(session, opcode::OP_SEND, dst, sym, 1)?;
            } else {
                cg.current().1.pop_n(1)?;
            }
            let chained = {
                let (session, scope) = cg.current();
                let cur = scope.cursp();
                scope.genjmp2(session, opcode::OP_JMPIF, cur, pos2, head.is_none())?
            };
            pos2 = chained;
        }
        let pos1 = cg.current().1.genjmp(opcode::OP_JMP, JMPLINK_START)?;
        cg.current().1.dispatch_linked(pos2)?;
        // A null body codes nothing valued-only; an empty node still emits
        // `LOADNIL` like the `STATEMENTS` arm.
        gen_branch(cg, when_view.body, val)?;
        if val {
            cg.current().1.pop_n(1)?;
        }
        let chained = cg.current().1.genjmp(opcode::OP_JMP, pos3)?;
        pos3 = chained;
        cg.current().1.dispatch(pos1)?;
    }
    if let Some(else_body) = view.else_body {
        codegen(cg, else_body, val)?;
        if val {
            cg.current().1.pop_n(1)?;
        }
        pos3 = cg.current().1.genjmp(opcode::OP_JMP, pos3)?;
    }
    if val {
        let pos = cg.current().1.cursp();
        {
            let (session, scope) = cg.current();
            scope.genop_1(session, opcode::OP_LOADNIL, pos)?;
        }
        if pos3 != JMPLINK_START {
            cg.current().1.dispatch_linked(pos3)?;
        }
        if head.is_some() {
            cg.current().1.pop_n(1)?;
        }
        if cg.current().1.cursp() != pos {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.gen_move(session, dst, pos, false)?;
        }
        cg.current().1.push_n(1)?;
    } else {
        if pos3 != JMPLINK_START {
            cg.current().1.dispatch_linked(pos3)?;
        }
        if head.is_some() {
            cg.current().1.pop_n(1)?;
        }
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

/// Whether an array literal's elements include a splat
/// (`PM_ARRAY_NODE_FLAGS_CONTAINS_SPLAT`).
fn array_contains_splat<N: BackendNode>(node: &N) -> bool {
    node.array_elements()
        .is_some_and(|elements| elements.iter().any(|item| item.kind_name() == "SplatNode"))
}

/// Multiple assignment (`PM_MULTI_WRITE_NODE`). A fixed array right-hand side
/// in statement position reads its elements in place; anything else evaluates
/// the right-hand side and destructures with `gen_massignment`.
fn gen_multi_write<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(view) = node.multi_write_view() else {
        return Err(unsupported(&node, "multiple assignment"));
    };
    let lefts = view.lefts;
    let rest = view.rest;
    let rights = view.rights;
    let value = view.value;
    let rhs = i32::from(cg.current().1.cursp());

    if !val && value.kind_name() == "ArrayNode" && !array_contains_splat(&value) {
        let Some(elements) = value.array_elements() else {
            return Err(unsupported(&value, "array elements"));
        };
        let len = elements.len();
        for element in elements {
            codegen(cg, element, true)?;
        }
        let mut n = 0usize;
        for (index, left) in lefts.into_iter().enumerate() {
            if index < len {
                gen_assignment(cg, left, None, (rhs + n as i32) as u16, false)?;
                n += 1;
            } else {
                let sp = cg.current().1.cursp();
                let (session, scope) = cg.current();
                scope.genop_1(session, opcode::OP_LOADNIL, sp)?;
                scope.push_n(1)?;
                gen_assignment(cg, left, None, sp, false)?;
                cg.current().1.pop_n(1)?;
            }
        }
        let post = rights.len();
        if let Some(rest_node) = rest {
            let rn = if len < post + n { 0 } else { len - post - n };
            if rest_node.kind_name() != "ImplicitRestNode" {
                emit_aref_group(cg, rhs, n, rn)?;
                if let Some(Some(expression)) = rest_node.splat_value() {
                    let sp = cg.current().1.cursp();
                    cg.current().1.push_n(1)?;
                    gen_assignment(cg, expression, None, sp, false)?;
                    cg.current().1.pop_n(1)?;
                }
            }
            n += rn;
        }
        if post > 0 {
            for right in rights {
                if n < len {
                    gen_assignment(cg, right, None, (rhs + n as i32) as u16, false)?;
                } else {
                    let sp = cg.current().1.cursp();
                    let (session, scope) = cg.current();
                    scope.genop_1(session, opcode::OP_LOADNIL, sp)?;
                    scope.push_n(1)?;
                    gen_assignment(cg, right, None, sp, false)?;
                    cg.current().1.pop_n(1)?;
                }
                n += 1;
            }
        }
        cg.current().1.pop_n(len as u16)?;
    } else {
        codegen(cg, value, true)?;
        gen_massignment(cg, lefts, rest, rights, rhs, val)?;
        if !val {
            cg.current().1.pop_n(1)?;
        }
    }
    Ok(())
}

/// The `ARRAY`/`ARRAY2` slice a fixed right-hand side leaves for a rest
/// target (`gen_massignment` fixed path).
fn emit_aref_group(cg: &mut Codegen, rhs: i32, n: usize, rn: usize) -> Result<(), Diagnostic> {
    let (session, scope) = cg.current();
    let dst = scope.cursp();
    if i32::from(dst) == rhs + n as i32 {
        scope.genop_2(session, opcode::OP_ARRAY, dst, rn as u16)
    } else {
        scope.genop_3(
            session,
            opcode::OP_ARRAY2,
            dst,
            (rhs + n as i32) as u16,
            rn as u8,
        )
    }
}

/// Destructuring assignment (`gen_massignment`): `AREF` reads the pre targets
/// and `APOST` splits the rest and post targets from the array at `rhs`.
#[allow(clippy::too_many_arguments)]
fn gen_massignment<N: BackendNode>(
    cg: &mut Codegen,
    lefts: Vec<N>,
    rest: Option<N>,
    rights: Vec<N>,
    rhs: i32,
    val: bool,
) -> Result<(), Diagnostic> {
    let n = lefts.len() as i32;
    let post = rights.len() as i32;
    let has_rest = rest
        .as_ref()
        .is_some_and(|node| node.kind_name() != "ImplicitRestNode");
    let mut base = rhs;
    let mut idx = 0i32;
    if post > 255 {
        return Err(Diagnostic {
            message: "too many post-splat assignment targets".to_owned(),
            start: 0,
            end: 0,
        });
    }
    let mut scratch = gen_aref_scratch(cg, n)?;
    if n > 0 {
        for left in lefts {
            if idx == 255 {
                base = gen_aref_rebase(cg, base, scratch)?;
                idx = 0;
            }
            let sp = cg.current().1.cursp();
            {
                let (session, scope) = cg.current();
                scope.genop_3(session, opcode::OP_AREF, sp, base as u16, idx as u8)?;
            }
            idx += 1;
            cg.current().1.push_n(1)?;
            gen_assignment(cg, left, None, sp, false)?;
            cg.current().1.pop_n(1)?;
        }
    }
    if has_rest || post > 0 {
        {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.gen_move(session, dst, base as u16, val)?;
        }
        let sp = cg.current().1.cursp();
        cg.current().1.push_n((post + 1) as u16)?;
        {
            let (session, scope) = cg.current();
            scope.genop_3(session, opcode::OP_APOST, sp, idx as u16, post as u8)?;
        }
        if has_rest {
            let rest_node = rest.expect("rest");
            if let Some(Some(expression)) = rest_node.splat_value() {
                gen_assignment(cg, expression, None, sp, false)?;
            }
        }
        for (index, right) in rights.into_iter().enumerate() {
            gen_assignment(cg, right, None, sp + index as u16 + 1, false)?;
        }
        cg.current().1.pop_n((post + 1) as u16)?;
        if scratch >= 0 {
            cg.current().1.pop_n(1)?;
            scratch = -1;
        }
        if val {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.gen_move(session, dst, rhs as u16, false)?;
        }
    }
    if scratch >= 0 {
        cg.current().1.pop_n(1)?;
    }
    Ok(())
}

/// Scratch register reserved for the `AREF` walk (`gen_aref_scratch`).
fn gen_aref_scratch(cg: &mut Codegen, len: i32) -> Result<i32, Diagnostic> {
    if len <= 255 {
        return Ok(-1);
    }
    let scratch = i32::from(cg.current().1.cursp());
    cg.current().1.push_n(1)?;
    Ok(scratch)
}

/// Rebase the `AREF` walk every 255 elements (`gen_aref_rebase`).
fn gen_aref_rebase(cg: &mut Codegen, base: i32, scratch: i32) -> Result<i32, Diagnostic> {
    if base != scratch {
        let (session, scope) = cg.current();
        scope.gen_move(session, scratch as u16, base as u16, false)?;
    }
    {
        let (session, scope) = cg.current();
        scope.genop_3(session, opcode::OP_APOST, scratch as u16, 255, 0)?;
    }
    Ok(scratch)
}

/// Begin body (`gen_begin`): a null subtree emits `LOADNIL` only when valued,
/// while an empty statements node emits nothing.
fn gen_begin<N: BackendNode>(
    cg: &mut Codegen,
    statements: Option<Vec<N>>,
    val: bool,
) -> Result<(), Diagnostic> {
    if val && statements.is_none() {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
        scope.push_n(1)?;
    }
    if let Some(items) = statements {
        let last = items.len();
        for (index, item) in items.into_iter().enumerate() {
            let item_val = if index + 1 < last { false } else { val };
            codegen(cg, item, item_val)?;
        }
    }
    Ok(())
}

/// One `rescue` clause (`gen_rescue`), including `=> e` and `$!` save/restore.
#[allow(clippy::too_many_arguments)]
fn gen_rescue<N: BackendNode>(
    cg: &mut Codegen,
    node: N,
    pos1: &mut u32,
    exc: u16,
    extend: &mut u32,
    val: bool,
    errsave: u16,
    landing: u16,
) -> Result<(), Diagnostic> {
    let Some(view) = node.rescue_view() else {
        return Err(unsupported(&node, "rescue"));
    };
    cg.current().1.dispatch(*pos1)?;
    let mut pos2 = JMPLINK_START;
    if view.exceptions.is_empty() {
        {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            let sym = scope.new_sym(session, b"StandardError")?;
            scope.genop_2(session, opcode::OP_GETCONST, dst, sym)?;
        }
        cg.current().1.push_n(1)?;
        cg.current().1.pop_n(1)?;
        {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_2(session, opcode::OP_RESCUE, exc, dst)?;
        }
        let cur = cg.current().1.cursp();
        let tmp = {
            let (session, scope) = cg.current();
            scope.genjmp2(session, opcode::OP_JMPIF, cur, pos2, val)?
        };
        pos2 = tmp;
    } else {
        for exception in view.exceptions {
            if exception.kind_name() == "SplatNode" {
                let Some(inner) = exception.splat_value() else {
                    return Err(unsupported(&exception, "splat"));
                };
                match inner {
                    Some(expression) => codegen(cg, expression, true)?,
                    None => gen_lvar(cg, b"*", 0)?,
                }
                {
                    let (session, scope) = cg.current();
                    let dst = scope.cursp();
                    scope.gen_move(session, dst, exc, false)?;
                }
                cg.current().1.push_n(2)?;
                cg.current().1.pop_n(2)?;
                cg.current().1.pop_n(1)?;
                {
                    let (session, scope) = cg.current();
                    let dst = scope.cursp();
                    let sym = scope.new_sym(session, b"__case_eqq")?;
                    scope.genop_3(session, opcode::OP_SEND, dst, sym, 1)?;
                }
            } else {
                codegen(cg, exception, true)?;
                cg.current().1.pop_n(1)?;
                {
                    let (session, scope) = cg.current();
                    let dst = scope.cursp();
                    scope.genop_2(session, opcode::OP_RESCUE, exc, dst)?;
                }
            }
            let cur = cg.current().1.cursp();
            let tmp = {
                let (session, scope) = cg.current();
                scope.genjmp2(session, opcode::OP_JMPIF, cur, pos2, val)?
            };
            pos2 = tmp;
        }
    }
    *pos1 = cg.current().1.genjmp(opcode::OP_JMP, JMPLINK_START)?;
    cg.current().1.dispatch_linked(pos2)?;
    cg.current().1.pop_n(1)?;
    {
        let (session, scope) = cg.current();
        let sym = scope.new_sym(session, ERRINFO)?;
        scope.genop_2(session, opcode::OP_SETGV, exc, sym)?;
    }
    if let Some(reference) = view.reference {
        gen_assignment(cg, reference, None, exc, false)?;
    }
    gen_branch(cg, view.statements, val)?;
    if val {
        cg.current().1.pop_n(1)?;
    }
    {
        let (session, scope) = cg.current();
        let sym = scope.new_sym(session, ERRINFO)?;
        scope.genop_2(session, opcode::OP_SETGV, errsave, sym)?;
    }
    if val {
        let src = cg.current().1.cursp();
        let (session, scope) = cg.current();
        scope.gen_move(session, landing, src, false)?;
    }
    *extend = cg.current().1.genjmp(opcode::OP_JMP, *extend)?;
    cg.current().1.push_n(1)?;
    if let Some(subsequent) = view.subsequent {
        gen_rescue(cg, subsequent, pos1, exc, extend, val, errsave, landing)?;
    }
    Ok(())
}

/// Ensure body (`gen_ensure`): runs on normal exit and while unwinding, and
/// restores `$!` on the way out.
fn gen_ensure<N: BackendNode>(
    cg: &mut Codegen,
    statements: Option<Vec<N>>,
    catch_entry: usize,
    begin: u32,
) -> Result<(), Diagnostic> {
    cg.current().1.push_n(1)?;
    let ensure_end = cg.current().1.pc;
    cg.current().1.push_n(1)?;
    let idx = cg.current().1.cursp();
    {
        let (session, scope) = cg.current();
        scope.genop_1(session, opcode::OP_EXCEPT, idx)?;
    }
    cg.current().1.push_n(1)?;
    let errsave = cg.current().1.cursp();
    {
        let (session, scope) = cg.current();
        let sym = scope.new_sym(session, ERRINFO)?;
        scope.genop_2(session, opcode::OP_GETGV, errsave, sym)?;
    }
    cg.current().1.push_n(1)?;
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_1(session, opcode::OP_OCLASS, dst)?;
        let sym = scope.new_sym(session, b"Exception")?;
        scope.genop_2(session, opcode::OP_GETMCNST, dst, sym)?;
    }
    cg.current().1.push_n(1)?;
    cg.current().1.pop_n(1)?;
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_RESCUE, idx, dst)?;
    }
    let skip = {
        let (session, scope) = cg.current();
        let cur = scope.cursp();
        scope.genjmp2(session, opcode::OP_JMPNOT, cur, JMPLINK_START, false)?
    };
    {
        let (session, scope) = cg.current();
        let sym = scope.new_sym(session, ERRINFO)?;
        scope.genop_2(session, opcode::OP_SETGV, idx, sym)?;
    }
    cg.current().1.dispatch(skip)?;
    let body_catch = cg.current().1.catch_new();
    let body_begin = cg.current().1.pc;
    if let Some(items) = statements {
        gen_list(cg, items, false)?;
    }
    {
        let (session, scope) = cg.current();
        let sym = scope.new_sym(session, ERRINFO)?;
        scope.genop_2(session, opcode::OP_SETGV, errsave, sym)?;
    }
    let restored = cg.current().1.genjmp(opcode::OP_JMP, JMPLINK_START)?;
    let body_end = cg.current().1.pc;
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_1(session, opcode::OP_EXCEPT, dst)?;
        let sym = scope.new_sym(session, ERRINFO)?;
        scope.genop_2(session, opcode::OP_SETGV, errsave, sym)?;
        scope.genop_1(session, opcode::OP_RAISEIF, dst)?;
    }
    cg.current()
        .1
        .catch_set(body_catch, CATCH_ENSURE, body_begin, body_end, body_end);
    cg.current().1.dispatch(restored)?;
    cg.current().1.pop_n(1)?;
    cg.current().1.pop_n(1)?;
    {
        let (session, scope) = cg.current();
        scope.genop_1(session, opcode::OP_RAISEIF, idx)?;
    }
    cg.current().1.pop_n(1)?;
    cg.current()
        .1
        .catch_set(catch_entry, CATCH_ENSURE, begin, ensure_end, ensure_end);
    Ok(())
}

/// `begin` with rescue/else/ensure (`PM_BEGIN_NODE`).
fn gen_begin_node<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(view) = node.begin_view() else {
        return Err(unsupported(&node, "begin"));
    };
    let has_rescue = view.rescue_clause.is_some();
    let has_else = view.else_clause.is_some();
    let has_ensure = view.ensure_clause.is_some();
    if !has_rescue && !has_else && !has_ensure {
        return gen_begin(cg, view.statements, val);
    }

    let mut ensure_catch_entry: Option<usize> = None;
    let mut ensure_begin = 0u32;
    if let Some(ensure_clause) = &view.ensure_clause {
        if let Some(ensure_view) = ensure_clause.ensure_view() {
            if ensure_view.statements.is_some() {
                ensure_catch_entry = Some(cg.current().1.catch_new());
                ensure_begin = cg.current().1.pc;
            }
        }
    }

    let catch_entry;
    let begin;
    {
        let (_, scope) = cg.current();
        scope.loop_push(LoopType::Begin);
        let pc0 = scope.new_label();
        scope.loops.last_mut().expect("loop").pc0 = pc0;
        catch_entry = scope.catch_new();
        begin = scope.pc;
    }
    gen_begin(cg, view.statements, true)?;
    cg.current().1.pop_n(1)?;
    cg.current().1.loops.last_mut().expect("loop").kind = LoopType::Rescue;
    let end = cg.current().1.pc;
    let noexc = cg.current().1.genjmp(opcode::OP_JMP, JMPLINK_START)?;
    let target = cg.current().1.pc;
    cg.current()
        .1
        .catch_set(catch_entry, CATCH_RESCUE, begin, end, target);

    let mut exend = JMPLINK_START;
    let mut pos1 = JMPLINK_START;
    if let Some(rescue) = view.rescue_clause {
        let landing = cg.current().1.cursp();
        cg.current().1.push_n(1)?;
        let errsave = cg.current().1.cursp();
        {
            let (session, scope) = cg.current();
            let sym = scope.new_sym(session, ERRINFO)?;
            scope.genop_2(session, opcode::OP_GETGV, errsave, sym)?;
        }
        cg.current().1.push_n(1)?;
        let exc = cg.current().1.cursp();
        {
            let (session, scope) = cg.current();
            scope.genop_1(session, opcode::OP_EXCEPT, exc)?;
        }
        cg.current().1.push_n(1)?;
        let err_catch = cg.current().1.catch_new();
        let err_begin = cg.current().1.pc;
        gen_rescue(
            cg, rescue, &mut pos1, exc, &mut exend, val, errsave, landing,
        )?;
        if pos1 != JMPLINK_START {
            cg.current().1.dispatch(pos1)?;
            let (session, scope) = cg.current();
            let sym = scope.new_sym(session, ERRINFO)?;
            scope.genop_2(session, opcode::OP_SETGV, errsave, sym)?;
            scope.genop_1(session, opcode::OP_RAISEIF, exc)?;
        }
        cg.current().1.pop_n(1)?;
        cg.current().1.push_n(1)?;
        let err_end = cg.current().1.pc;
        cg.current().1.push_n(1)?;
        let idx = cg.current().1.cursp();
        {
            let (session, scope) = cg.current();
            scope.genop_1(session, opcode::OP_EXCEPT, idx)?;
            let sym = scope.new_sym(session, ERRINFO)?;
            scope.genop_2(session, opcode::OP_SETGV, errsave, sym)?;
            scope.genop_1(session, opcode::OP_RAISEIF, idx)?;
        }
        cg.current().1.pop_n(1)?;
        cg.current().1.pop_n(1)?;
        cg.current()
            .1
            .catch_set(err_catch, CATCH_ENSURE, err_begin, err_end, err_end);
        cg.current().1.pop_n(1)?;
    }
    cg.current().1.pop_n(1)?;
    cg.current().1.dispatch(noexc)?;
    if let Some(else_clause) = view.else_clause {
        codegen(cg, else_clause, val)?;
    } else if val {
        cg.current().1.push_n(1)?;
    }
    cg.current().1.dispatch_linked(exend)?;
    {
        let (session, scope) = cg.current();
        scope.loop_pop(session, false)?;
    }
    if let Some(ensure_clause) = view.ensure_clause {
        let statements = ensure_clause
            .ensure_view()
            .and_then(|ensure_view| ensure_view.statements);
        if let Some(statements) = statements {
            if has_rescue {
                cg.current().1.pop_n(1)?;
            }
            gen_ensure(
                cg,
                Some(statements),
                ensure_catch_entry.expect("ensure catch entry"),
                ensure_begin,
            )?;
        }
    }
    Ok(())
}

/// `expr rescue expr` (`PM_RESCUE_MODIFIER_NODE`).
fn gen_rescue_modifier<N: BackendNode>(
    cg: &mut Codegen,
    node: N,
    val: bool,
) -> Result<(), Diagnostic> {
    let Some(view) = node.rescue_modifier_view() else {
        return Err(unsupported(&node, "rescue modifier"));
    };
    let catch_entry;
    let begin_pos;
    {
        let (_, scope) = cg.current();
        scope.loop_push(LoopType::Begin);
        let pc0 = scope.new_label();
        scope.loops.last_mut().expect("loop").pc0 = pc0;
        catch_entry = scope.catch_new();
        begin_pos = scope.pc;
    }
    codegen(cg, view.expression, val)?;
    if val {
        cg.current().1.pop_n(1)?;
    }
    cg.current().1.loops.last_mut().expect("loop").kind = LoopType::Rescue;
    let end_pos = cg.current().1.pc;
    let noexc = cg.current().1.genjmp(opcode::OP_JMP, JMPLINK_START)?;
    let target = cg.current().1.pc;
    cg.current()
        .1
        .catch_set(catch_entry, CATCH_RESCUE, begin_pos, end_pos, target);

    let landing = cg.current().1.cursp();
    cg.current().1.push_n(1)?;
    let errsave = cg.current().1.cursp();
    {
        let (session, scope) = cg.current();
        let sym = scope.new_sym(session, ERRINFO)?;
        scope.genop_2(session, opcode::OP_GETGV, errsave, sym)?;
    }
    cg.current().1.push_n(1)?;
    let exc = cg.current().1.cursp();
    {
        let (session, scope) = cg.current();
        scope.genop_1(session, opcode::OP_EXCEPT, exc)?;
    }
    cg.current().1.push_n(1)?;
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        let sym = scope.new_sym(session, b"StandardError")?;
        scope.genop_2(session, opcode::OP_GETCONST, dst, sym)?;
    }
    cg.current().1.push_n(1)?;
    cg.current().1.pop_n(1)?;
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_RESCUE, exc, dst)?;
    }
    let rescue_jmp = {
        let (session, scope) = cg.current();
        let cur = scope.cursp();
        scope.genjmp2(session, opcode::OP_JMPIF, cur, JMPLINK_START, val)?
    };
    {
        let (session, scope) = cg.current();
        scope.genop_1(session, opcode::OP_RAISEIF, exc)?;
    }
    cg.current().1.dispatch(rescue_jmp)?;
    cg.current().1.pop_n(1)?;
    {
        let (session, scope) = cg.current();
        let sym = scope.new_sym(session, ERRINFO)?;
        scope.genop_2(session, opcode::OP_SETGV, exc, sym)?;
    }
    let err_catch = cg.current().1.catch_new();
    let err_begin = cg.current().1.pc;
    codegen(cg, view.rescue_expression, val)?;
    if val {
        cg.current().1.pop_n(1)?;
    }
    {
        let (session, scope) = cg.current();
        let sym = scope.new_sym(session, ERRINFO)?;
        scope.genop_2(session, opcode::OP_SETGV, errsave, sym)?;
    }
    if val {
        let src = cg.current().1.cursp();
        let (session, scope) = cg.current();
        scope.gen_move(session, landing, src, false)?;
    }
    let restored = cg.current().1.genjmp(opcode::OP_JMP, JMPLINK_START)?;
    {
        let err_end = cg.current().1.pc;
        cg.current().1.push_n(1)?;
        let idx = cg.current().1.cursp();
        {
            let (session, scope) = cg.current();
            scope.genop_1(session, opcode::OP_EXCEPT, idx)?;
            let sym = scope.new_sym(session, ERRINFO)?;
            scope.genop_2(session, opcode::OP_SETGV, errsave, sym)?;
            scope.genop_1(session, opcode::OP_RAISEIF, idx)?;
        }
        cg.current().1.pop_n(1)?;
        cg.current()
            .1
            .catch_set(err_catch, CATCH_ENSURE, err_begin, err_end, err_end);
    }
    cg.current().1.pop_n(1)?;
    cg.current().1.pop_n(1)?;
    cg.current().1.dispatch(restored)?;
    cg.current().1.dispatch(noexc)?;
    if val {
        cg.current().1.push_n(1)?;
    }
    {
        let (session, scope) = cg.current();
        scope.loop_pop(session, false)?;
    }
    Ok(())
}

/// `retry` (`PM_RETRY_NODE`): jumps to the enclosing rescue clause's start.
fn gen_retry<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let pc0 = cg
        .current()
        .1
        .loops
        .iter()
        .rev()
        .find(|frame| frame.kind == LoopType::Rescue)
        .map(|frame| frame.pc0);
    let Some(pc0) = pc0 else {
        return Err(unsupported(&node, "retry outside rescue"));
    };
    cg.current().1.genjmp(opcode::OP_JMPUW, pc0)?;
    if val {
        cg.current().1.push_n(1)?;
    }
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
