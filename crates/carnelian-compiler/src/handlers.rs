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
const VAL_STACK_MAX: u32 = 99;
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

    /// Upvar lookup (`search_upvar`): walks parents by name, returning
    /// the slot and the level count (`lv`).
    fn search_upvar(&self, name: &[u8]) -> Result<(u16, u16), Diagnostic> {
        let mut level: u16 = 0;
        for index in (0..self.scopes.len().saturating_sub(1)).rev() {
            let slot = self.scopes[index].lv_idx(name);
            if slot > 0 {
                return Ok((slot, level));
            }
            level = level.saturating_add(1);
        }
        let message = if name == b"&" {
            "No anonymous block parameter"
        } else if name == b"*" {
            "No anonymous rest parameter"
        } else if name == b"**" {
            "No anonymous keyword rest parameter"
        } else {
            "Can't find local variables"
        };
        Err(Diagnostic {
            message: message.to_owned(),
            start: 0,
            end: 0,
        })
    }

    /// Method scope lookup for `yield` (`search_mscope`): walks while the
    /// scope is not a method scope, counting levels. `ainfo` is `-1` when
    /// no method scope exists (top-level `yield`).
    fn search_mscope(&self) -> (i32, u16, u32) {
        let mut level: u16 = 0;
        let mut cursor: Option<usize> = Some(self.scopes.len() - 1);
        while let Some(index) = cursor {
            if self.scopes[index].mscope {
                let scope = &self.scopes[index];
                return (i32::from(scope.ainfo), level, scope.aspec);
            }
            level = level.saturating_add(1);
            cursor = index.checked_sub(1);
        }
        (-1, level, 0)
    }

    /// `OP_BLKPUSH` operand (`mscope_operand`).
    fn mscope_operand(ainfo: i32, level: u16) -> Result<u16, Diagnostic> {
        if ainfo > 0xfff {
            return Err(Diagnostic {
                message: "too many formal arguments".to_owned(),
                start: 0,
                end: 0,
            });
        }
        if level > 0xf {
            return Err(Diagnostic {
                message: "too many nested blocks/methods".to_owned(),
                start: 0,
                end: 0,
            });
        }
        Ok(((ainfo as u16) << 4) | level)
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
        "LocalVariableWriteNode" => gen_lvar_write(cg, node, val),
        "ItLocalVariableReadNode" => gen_it_read(cg, node, val),
        "BlockNode" => gen_block(cg, node, val),
        "LambdaNode" => gen_lambda(cg, node, val),
        "YieldNode" => gen_yield(cg, node, val),
        "BlockArgumentNode" => gen_block_arg(cg, node, val),
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
    let depth = target.depth + u32::from(cg.scopes.last().expect("open scope").for_depth);
    if depth == 0 {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        let index = scope.lv_idx(&target.name);
        // Note the set peephole flag (`gen_move(..., 1)`).
        scope.gen_move(session, dst, index, true)?;
        scope.push_n(1)
    } else {
        // Upvar read (`gen_lvar` else branch): search by name, `GETUPVAR`.
        let (slot, level) = cg.search_upvar(&target.name)?;
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.gen_getupvar(session, dst, slot, level)?;
        scope.push_n(1)
    }
}

fn gen_lvar_write<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(target) = node.lvar_write() else {
        return Err(unsupported(&node, "variable write"));
    };
    let Some(rhs) = target.value else {
        return Err(unsupported(&node, "bare write target"));
    };
    codegen(cg, rhs, true)?;
    let depth = target.depth + u32::from(cg.scopes.last().expect("open scope").for_depth);
    if depth != 0 {
        // Upvar write (`gen_assignment_lvar` else branch): `SETUPVAR`.
        let (slot, level) = cg.search_upvar(&target.name)?;
        let (session, scope) = cg.current();
        scope.pop_n(1)?;
        let dst = scope.cursp();
        scope.gen_setupvar(session, dst, slot, level, val)?;
        if val {
            let (_, scope) = cg.current();
            scope.push_n(1)?;
        }
        return Ok(());
    }
    let (session, scope) = cg.current();
    scope.pop_n(1)?;
    let sp = scope.cursp();
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

/// `it` read (`PM_IT_LOCAL_VARIABLE_READ_NODE`): `it` lives at slot 1.
fn gen_it_read<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    if !val {
        return Ok(());
    }
    let Some(()) = node.it_read() else {
        return Err(unsupported(&node, "it read"));
    };
    let (session, scope) = cg.current();
    let dst = scope.cursp();
    scope.gen_move(session, dst, 1, true)?;
    scope.push_n(1)
}

/// `&` block argument (`PM_BLOCK_ARGUMENT_NODE`).
fn gen_block_arg<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(inner) = node.block_arg() else {
        return Err(unsupported(&node, "block argument"));
    };
    match inner {
        None => {
            // Bare `&`: load the `&` local (or upvar when forwarded).
            let slot = cg.current().1.lv_idx(b"&");
            if slot > 0 {
                let (session, scope) = cg.current();
                let dst = scope.cursp();
                scope.gen_move(session, dst, slot, val)?;
                if val {
                    scope.push_n(1)?;
                }
            } else {
                let (slot, level) = cg.search_upvar(b"&")?;
                let (session, scope) = cg.current();
                let dst = scope.cursp();
                scope.gen_getupvar(session, dst, slot, level)?;
                if val {
                    scope.push_n(1)?;
                }
            }
            Ok(())
        }
        Some(expr) => codegen(cg, expr, val),
    }
}

/// `MRC_ARGS_REQ(n)` (`codegen.c` aspec layout).
fn args_req(count: usize) -> u32 {
    ((count as u32) & 0x1f) << 18
}

/// `MRC_ARGS_REST()` (`codegen.c` aspec layout).
fn args_rest() -> u32 {
    1 << 12
}

/// Plain block body (`BlockNode`): child scope with `OP_BLOCK`.
fn gen_block<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    if !val {
        return Ok(());
    }
    let Some(view) = node.block_view() else {
        return Err(unsupported(&node, "block"));
    };
    gen_lambda_body(
        cg,
        view.locals,
        view.params,
        view.body,
        opcode::OP_BLOCK,
        &node,
    )
}

/// Lambda literal (`LambdaNode`): child scope with `OP_LAMBDA`.
fn gen_lambda<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    if !val {
        return Ok(());
    }
    let Some(view) = node.lambda_view() else {
        return Err(unsupported(&node, "lambda"));
    };
    gen_lambda_body(
        cg,
        view.locals,
        view.params,
        view.body,
        opcode::OP_LAMBDA,
        &node,
    )
}

/// Shared `lambda_body` for blocks and lambdas (`blk = 1`).
/// Covers empty, plain `|x|`, `|x, y|`, `|*a|` (named or anonymous),
/// numbered (`_1`) and `it` forms; optional, post, keyword, block and
/// destructured parameters are gated with diagnostics.
fn gen_lambda_body<N: BackendNode>(
    cg: &mut Codegen,
    locals: Vec<Vec<u8>>,
    params: Option<N>,
    body: Option<N>,
    op: u8,
    site: &N,
) -> Result<(), Diagnostic> {
    let mut lv: Vec<Vec<u8>> = Vec::new();
    let (ainfo, aspec) = match params {
        None => {
            // Empty block: placeholder plus declared locals.
            lv.push(Vec::new());
            for name in &locals {
                if !lv.contains(name) {
                    lv.push(name.clone());
                }
            }
            (0, args_req(0))
        }
        Some(holder) => match holder.kind_name() {
            "NumberedParametersNode" => {
                let Some(max) = holder.numbered_max() else {
                    return Err(unsupported(site, "numbered parameters"));
                };
                let count = usize::from(max);
                for index in 0..count {
                    lv.push(format!("_{}", index + 1).into_bytes());
                }
                for name in &locals {
                    if !lv.contains(name) {
                        lv.push(name.clone());
                    }
                }
                let info = (u16::from(max) & 0x3f) << 7;
                (info, args_req(count))
            }
            "ItParametersNode" => {
                lv.push(Vec::new());
                for name in &locals {
                    if !lv.contains(name) {
                        lv.push(name.clone());
                    }
                }
                (1 << 7, args_req(1))
            }
            "BlockParametersNode" => {
                let Some(view) = holder.block_param_view() else {
                    return Err(unsupported(site, "block parameters"));
                };
                let mut semi: Vec<Vec<u8>> = Vec::new();
                for local in view.block_locals {
                    let Some(name) = local.block_local_name() else {
                        return Err(unsupported(site, "block local"));
                    };
                    semi.push(name);
                }
                let Some(inner) = view.params else {
                    // `||` or `|;local|`: no positional layout.
                    lv.push(Vec::new());
                    for name in locals.iter().chain(semi.iter()) {
                        if !lv.contains(name) {
                            lv.push(name.clone());
                        }
                    }
                    return enter_block_scope(cg, lv, 0, args_req(0), body, op);
                };
                if inner.kind_name() != "ParametersNode" {
                    return Err(unsupported(site, "block parameters"));
                }
                let Some(view) = inner.parameters_view() else {
                    return Err(unsupported(site, "block parameters"));
                };
                if !view.optionals.is_empty() {
                    return Err(unsupported(site, "block optional parameter"));
                }
                if !view.posts.is_empty() {
                    return Err(unsupported(site, "block post parameter"));
                }
                if !view.keywords.is_empty() || view.keyword_rest.is_some() {
                    return Err(unsupported(site, "block keyword parameter"));
                }
                if view.block.is_some() {
                    return Err(unsupported(site, "block block parameter"));
                }
                let mut required_names: Vec<Vec<u8>> = Vec::new();
                for item in &view.requireds {
                    let Some(name) = item.required_param_name() else {
                        return Err(unsupported(site, "block destructuring"));
                    };
                    required_names.push(name);
                }
                let mut rest_name: Option<Vec<u8>> = None;
                if let Some(rest) = &view.rest {
                    match rest.kind_name() {
                        "RestParameterNode" => {
                            let Some(maybe) = rest.rest_param_name() else {
                                return Err(unsupported(site, "block rest parameter"));
                            };
                            rest_name = Some(maybe.unwrap_or_else(|| b"*".to_vec()));
                        }
                        "ImplicitRestNode" => {
                            rest_name = Some(b"*".to_vec());
                        }
                        _ => return Err(unsupported(site, "block rest parameter")),
                    }
                }
                if required_names.len() > 0x1f {
                    return Err(Diagnostic {
                        message: "too many formal arguments".to_owned(),
                        start: 0,
                        end: 0,
                    });
                }
                let ma = required_names.len();
                let ra = u16::from(rest_name.is_some());
                lv.extend(required_names);
                if let Some(name) = rest_name {
                    lv.push(name);
                }
                lv.push(Vec::new());
                for name in locals.iter().chain(semi.iter()) {
                    if !lv.contains(name) {
                        lv.push(name.clone());
                    }
                }
                let info = (((ma as u16) & 0x3f) << 7) | (ra << 6);
                let spec = args_req(ma) | if ra == 1 { args_rest() } else { 0 };
                (info, spec)
            }
            _ => return Err(unsupported(site, "block parameters")),
        },
    };
    enter_block_scope(cg, lv, ainfo, aspec, body, op)
}

/// Pushes the child scope, emits `OP_ENTER`, codes the body and emits
/// `OP_BLOCK`/`OP_LAMBDA` in the parent (`lambda_body` tail with `blk`).
fn enter_block_scope<N: BackendNode>(
    cg: &mut Codegen,
    lv: Vec<Vec<u8>>,
    ainfo: u16,
    aspec: u32,
    body: Option<N>,
    op: u8,
) -> Result<(), Diagnostic> {
    let child = Scope::child(&mut cg.session, cg.scopes.last().expect("open scope"), &lv)?;
    cg.scopes.push(child);
    {
        let scope = cg.scopes.last_mut().expect("block scope");
        scope.ainfo = ainfo;
        scope.aspec = aspec;
        scope.mscope = false;
        scope.for_depth = 0;
    }
    {
        let (_, scope) = cg.current();
        scope.genop_w(opcode::OP_ENTER, aspec)?;
    }
    {
        let (_, scope) = cg.current();
        scope.loop_push(LoopType::Block);
        let label = scope.new_label();
        scope.loops.last_mut().expect("block loop").pc0 = label;
    }
    match body {
        Some(node) => codegen(cg, node, true)?,
        None => {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
            scope.push_n(1)?;
        }
    }
    cg.current().1.pop_n(1)?;
    {
        // `ENTER` always precedes, so `pc > 0` holds like in C.
        let (session, scope) = cg.current();
        let ret = scope.cursp();
        scope.gen_return(session, opcode::OP_RETURN, ret)?;
    }
    {
        let (session, scope) = cg.current();
        scope.loop_pop(session, false)?;
    }
    let index = cg.pop_scope()?;
    let child_index = u16::try_from(index).map_err(|_| too_complex())?;
    let (session, scope) = cg.current();
    let dst = scope.cursp();
    scope.genop_2(session, op, dst, child_index)?;
    scope.push_n(1)
}

/// `yield` (`PM_YIELD_NODE`): `BLKPUSH` plus `BLKCALL` for plain
/// positional arguments. Keyword and splat forms stay gated like calls.
fn gen_yield<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(view) = node.yield_view() else {
        return Err(unsupported(&node, "yield"));
    };
    let (ainfo, level, _aspec) = cg.search_mscope();
    if ainfo < 0 {
        return Err(Diagnostic {
            message: "invalid yield (SyntaxError)".to_owned(),
            start: node.span().start,
            end: node.span().end,
        });
    }
    let operand = Codegen::mscope_operand(ainfo, level)?;
    cg.current().1.push_n(1)?;
    let mut count: i32 = 0;
    let mut have = 0;
    if let Some(args) = view.args {
        let Some(items) = args.call_args() else {
            return Err(unsupported(&node, "complex arguments"));
        };
        if !items.is_empty() {
            count = gen_values(cg, items, true, CALL_ARG_LIMIT)?;
            if count < 0 {
                count = 15;
                have = 1;
                cg.current().1.push_n(1)?;
            } else {
                have = count;
            }
        }
    }
    {
        let (_, scope) = cg.current();
        scope.push_n(1)?;
        scope.pop_n(1)?;
        scope.pop_n((have as u16).checked_add(1).ok_or_else(too_complex)?)?;
    }
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_2s(session, opcode::OP_BLKPUSH, dst, operand)?;
    }
    {
        // Plain path only (`nk == 0 && n < 15`); keyword and splat
        // arguments are gated above via `call_args`.
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_BLKCALL, dst, count as u16)?;
    }
    if val {
        cg.current().1.push_n(1)?;
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
    let mut blk = false;
    if let Some(block) = view.block {
        // Literal blocks and `&` block args share the `SENDB` path
        // (`gen_call` codes the block `VAL`, then pops it).
        codegen(cg, block, true)?;
        cg.current().1.pop_n(1)?;
        noop = true;
        blk = true;
    }
    {
        let (_, scope) = cg.current();
        scope.push_n(1)?;
        scope.pop_n(1)?;
        scope.sp = sp_save;
    }
    emit_call(cg, noself, noop, blk, &name, nargs, val, safe, skip)
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
    blk: bool,
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
    } else if !noop && nargs == 1 {
        if let Some(op) = direct_op(name) {
            let (session, scope) = cg.current();
            scope.genop_1(session, op, dst)?;
        } else if gen_binop(cg, name, dst)? {
            // An index opcode was emitted.
        } else {
            send_call(cg, noself, blk, name, nargs, dst)?;
        }
    } else if !noop && nargs == 0 && gen_uniop(cg, name, dst)? {
        // A literal absorbed its sign.
    } else {
        send_call(cg, noself, blk, name, nargs, dst)?;
    }
    if safe {
        cg.current().1.dispatch(skip)?;
    }
    if val {
        cg.current().1.push_n(1)?;
    }
    Ok(())
}

/// Generic `SEND`/`SSEND` emission (`SENDB`/`SSENDB` when a block is present).
fn send_call(
    cg: &mut Codegen,
    noself: bool,
    blk: bool,
    name: &[u8],
    nargs: i32,
    dst: u16,
) -> Result<(), Diagnostic> {
    let sym = {
        let (session, scope) = cg.current();
        scope.new_sym(session, name)?
    };
    if noself {
        let (session, scope) = cg.current();
        if !blk && nargs == 0 {
            scope.genop_2(session, opcode::OP_SSEND0, dst, sym)?;
        } else if blk {
            scope.genop_3(session, opcode::OP_SSENDB, dst, sym, nargs as u8)?;
        } else {
            scope.genop_3(session, opcode::OP_SSEND, dst, sym, nargs as u8)?;
        }
    } else {
        let (session, scope) = cg.current();
        if !blk && nargs == 0 {
            scope.genop_2(session, opcode::OP_SEND0, dst, sym)?;
        } else if blk {
            scope.genop_3(session, opcode::OP_SENDB, dst, sym, nargs as u8)?;
        } else {
            scope.genop_3(session, opcode::OP_SEND, dst, sym, nargs as u8)?;
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
