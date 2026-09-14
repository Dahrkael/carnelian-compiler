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
/// Constant-path segment cap (`DEFINED_PATH_MAX`).
const DEFINED_PATH_MAX: usize = 32;
/// Recursion bound (`MRC_CODEGEN_LEVEL_MAX`).
const CODEGEN_LEVEL_MAX: u32 = 256;
/// `defined?` runtime helpers (`mrc_presym.inc`).
const DEFINED_CONST_Q: &[u8] = b"__defined_const?";
const DEFINED_METHOD_Q: &[u8] = b"__defined_method?";
const DEFINED_IVAR_Q: &[u8] = b"__defined_ivar?";
const DEFINED_YIELD_Q: &[u8] = b"__defined_yield?";
const DEFINED_GVAR_Q: &[u8] = b"__defined_gvar?";
const DEFINED_CVAR_Q: &[u8] = b"__defined_cvar?";
const DEFINED_SUPER_Q: &[u8] = b"__defined_super?";
const DEFINED_CONST_PATH_Q: &[u8] = b"__defined_const_path?";
const DEFINED_METHOD_ON_Q: &[u8] = b"__defined_method_on?";
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

fn internal_error(message: &str) -> Diagnostic {
    Diagnostic {
        message: format!("internal error: {message}"),
        start: 0,
        end: 0,
    }
}

/// Gate a `defined?` sub-case that depends on machinery from a later tranche.
fn defined_gate<N: BackendNode>(node: &N, what: &str) -> Diagnostic {
    let span = node.span();
    Diagnostic {
        message: format!("unsupported defined? {what} in P1: {}", node.kind_name()),
        start: span.start,
        end: span.end,
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

/// Main dispatch (`codegen()` switch) with the `s->rlev` save/restore that
/// surrounds every C node walk.
pub fn codegen<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let rlev = cg.current().1.rlev;
    let result = codegen_dispatch(cg, node, val);
    cg.current().1.rlev = rlev;
    result
}

fn codegen_dispatch<N: BackendNode>(
    cg: &mut Codegen,
    node: N,
    val: bool,
) -> Result<(), Diagnostic> {
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
        "TrueNode" | "FalseNode" | "NilNode" | "SelfNode" => gen_simple(cg, node, val),
        "AliasMethodNode" => gen_alias(cg, node, val),
        "UndefNode" => gen_undef(cg, node, val),
        "DefinedNode" => {
            let Some(value) = node.defined_value() else {
                return Err(unsupported(&node, "defined?"));
            };
            codegen_defined(cg, value, val)
        }
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

fn gen_lvar_write<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(target) = node.lvar_write() else {
        return Err(unsupported(&node, "bare write target"));
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

/// Compile-time symbol for `alias`/`undef` (`alias_sym`).
fn alias_sym<N: BackendNode>(cg: &mut Codegen, node: N) -> Result<u16, Diagnostic> {
    match node.kind_name() {
        "SymbolNode" => {
            let Some(bytes) = node.symbol_lit() else {
                return Err(Diagnostic {
                    message: "invalid alias/undef argument".to_owned(),
                    start: 0,
                    end: 0,
                });
            };
            let (session, scope) = cg.current();
            scope.new_sym(session, &bytes)
        }
        "InterpolatedSymbolNode" => Err(Diagnostic {
            message: "dynamic symbol is not supported by alias/undef".to_owned(),
            start: 0,
            end: 0,
        }),
        _ => Err(Diagnostic {
            message: "invalid alias/undef argument".to_owned(),
            start: 0,
            end: 0,
        }),
    }
}

/// `alias new old` (`PM_ALIAS_METHOD_NODE`).
fn gen_alias<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some((new_name, old_name)) = node.alias_pair() else {
        return Err(unsupported(&node, "alias"));
    };
    let a = alias_sym(cg, new_name)?;
    let b = alias_sym(cg, old_name)?;
    let (session, scope) = cg.current();
    scope.genop_2(session, opcode::OP_ALIAS, a, b)?;
    if val {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
        scope.push_n(1)?;
    }
    Ok(())
}

/// `undef a, b` (`PM_UNDEF_NODE`).
fn gen_undef<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(names) = node.undef_list() else {
        return Err(unsupported(&node, "undef"));
    };
    for name in names {
        let sym = alias_sym(cg, name)?;
        let (session, scope) = cg.current();
        scope.genop_1(session, opcode::OP_UNDEF, sym)?;
    }
    if val {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
        scope.push_n(1)?;
    }
    Ok(())
}

/// The answer `defined?` gives for one operand (`struct defined_answer`).
struct DefinedAnswer<N> {
    literal: Option<&'static [u8]>,
    helper: Option<&'static [u8]>,
    arg: Option<Vec<u8>>,
    path: Vec<Vec<u8>>,
    path_toplevel: bool,
    recv: Option<N>,
    unless_nil: Option<&'static [u8]>,
}

impl<N> Default for DefinedAnswer<N> {
    fn default() -> Self {
        Self {
            literal: None,
            helper: None,
            arg: None,
            path: Vec::new(),
            path_toplevel: false,
            recv: None,
            unless_nil: None,
        }
    }
}

/// Whether a body holds no statement at all (`defined_body_empty_p`).
fn defined_body_empty<N: BackendNode>(body: Option<&N>) -> bool {
    match body {
        None => true,
        Some(node) if node.kind_name() == "StatementsNode" => node
            .statements()
            .map(|stmts| stmts.is_empty())
            .unwrap_or(true),
        _ => false,
    }
}

/// Unwrap implicit nodes, parentheses and single-statement bare `begin`s
/// (`defined_operand`).
fn defined_operand<N: BackendNode>(mut value: N) -> N {
    loop {
        let body: Option<N> = if value.kind_name() == "ImplicitNode" {
            match value.implicit_value() {
                Some(inner) => {
                    value = inner;
                    continue;
                }
                None => return value,
            }
        } else if value.kind_name() == "ParenthesesNode" {
            match value.parentheses_body() {
                Some(body) => body,
                None => return value,
            }
        } else if value.kind_name() == "BeginNode" {
            match value.begin_view() {
                Some(view) if view.bare => view.statements,
                _ => return value,
            }
        } else {
            return value;
        };
        let Some(body) = body else { return value };
        if body.kind_name() != "StatementsNode" {
            return value;
        }
        let Some(stmts) = body.statements() else {
            return value;
        };
        if stmts.len() != 1 {
            return value;
        }
        value = stmts.into_iter().next().expect("single statement");
    }
}

/// Operand kinds CRuby answers "expression" for without looking inside.
fn is_defined_expression_kind(kind: &str) -> bool {
    matches!(
        kind,
        "IntegerNode"
            | "FloatNode"
            | "RationalNode"
            | "ImaginaryNode"
            | "StringNode"
            | "InterpolatedStringNode"
            | "XStringNode"
            | "InterpolatedXStringNode"
            | "SymbolNode"
            | "InterpolatedSymbolNode"
            | "RegularExpressionNode"
            | "InterpolatedRegularExpressionNode"
            | "ArrayNode"
            | "HashNode"
            | "KeywordHashNode"
            | "RangeNode"
            | "LambdaNode"
            | "DefinedNode"
            | "SourceFileNode"
            | "SourceLineNode"
            | "SourceEncodingNode"
            | "AndNode"
            | "OrNode"
            | "IfNode"
            | "UnlessNode"
            | "CaseNode"
            | "CaseMatchNode"
            | "WhileNode"
            | "UntilNode"
            | "ForNode"
            | "ReturnNode"
            | "BreakNode"
            | "NextNode"
            | "RedoNode"
            | "RetryNode"
            | "DefNode"
            | "ClassNode"
            | "ModuleNode"
            | "SingletonClassNode"
            | "MatchPredicateNode"
            | "MatchRequiredNode"
            | "RescueModifierNode"
            | "MatchWriteNode"
            | "AliasMethodNode"
            | "UndefNode"
            | "PostExecutionNode"
    )
}

/// Operand kinds CRuby answers "assignment" for.
fn is_defined_assignment_kind(kind: &str) -> bool {
    matches!(
        kind,
        "LocalVariableWriteNode"
            | "InstanceVariableWriteNode"
            | "GlobalVariableWriteNode"
            | "ClassVariableWriteNode"
            | "ConstantWriteNode"
            | "ConstantPathWriteNode"
            | "MultiWriteNode"
            | "LocalVariableOperatorWriteNode"
            | "LocalVariableOrWriteNode"
            | "LocalVariableAndWriteNode"
            | "InstanceVariableOperatorWriteNode"
            | "InstanceVariableOrWriteNode"
            | "InstanceVariableAndWriteNode"
            | "GlobalVariableOperatorWriteNode"
            | "GlobalVariableOrWriteNode"
            | "GlobalVariableAndWriteNode"
            | "ClassVariableOperatorWriteNode"
            | "ClassVariableOrWriteNode"
            | "ClassVariableAndWriteNode"
            | "ConstantOperatorWriteNode"
            | "ConstantOrWriteNode"
            | "ConstantAndWriteNode"
            | "ConstantPathOperatorWriteNode"
            | "ConstantPathOrWriteNode"
            | "ConstantPathAndWriteNode"
            | "IndexOperatorWriteNode"
            | "IndexOrWriteNode"
            | "IndexAndWriteNode"
            | "CallOperatorWriteNode"
            | "CallOrWriteNode"
            | "CallAndWriteNode"
    )
}

/// Walk a constant path to its root, collecting names leaf first
/// (`defined_answer_for`, `PM_CONSTANT_PATH_NODE`).
fn defined_const_path<N: BackendNode>(value: &N, a: &mut DefinedAnswer<N>) {
    let Some((mut parent, first)) = value.constant_path_parts() else {
        return;
    };
    let mut names = vec![first];
    let mut rooted = false;
    let mut toplevel = false;
    let mut recv: Option<N> = None;
    loop {
        match parent {
            None => {
                toplevel = true;
                rooted = true;
                break;
            }
            Some(node) => {
                let kind = node.kind_name();
                if kind == "ConstantPathNode" {
                    if names.len() >= DEFINED_PATH_MAX {
                        break;
                    }
                    let Some((next, name)) = node.constant_path_parts() else {
                        break;
                    };
                    names.push(name);
                    parent = next;
                } else if kind == "ConstantReadNode" {
                    if names.len() >= DEFINED_PATH_MAX {
                        break;
                    }
                    names.push(node.constant_read_name().unwrap_or_default());
                    rooted = true;
                    break;
                } else {
                    recv = Some(node);
                    rooted = true;
                    break;
                }
            }
        }
    }
    if rooted {
        a.helper = Some(DEFINED_CONST_PATH_Q);
        names.reverse();
        a.path = names;
        a.path_toplevel = toplevel;
        a.recv = recv;
    }
}

/// Classify one `defined?` operand (`defined_answer_for`).
fn defined_answer_for<N: BackendNode>(value: &N) -> DefinedAnswer<N> {
    let mut a = DefinedAnswer::default();
    let kind = value.kind_name();
    if is_defined_expression_kind(kind) {
        a.literal = Some(b"expression");
        return a;
    }
    if is_defined_assignment_kind(kind) {
        a.literal = Some(b"assignment");
        return a;
    }
    match kind {
        "NilNode" => a.literal = Some(b"nil"),
        "TrueNode" => a.literal = Some(b"true"),
        "FalseNode" => a.literal = Some(b"false"),
        "SelfNode" => a.literal = Some(b"self"),
        "LocalVariableReadNode" | "ItLocalVariableReadNode" => {
            a.literal = Some(b"local-variable");
        }
        "ParenthesesNode" => {
            let body = value.parentheses_body().flatten();
            a.literal = Some(if defined_body_empty(body.as_ref()) {
                b"nil"
            } else {
                b"expression"
            });
        }
        "BeginNode" => {
            let Some(view) = value.begin_view() else {
                return a;
            };
            let empty = view.bare && defined_body_empty(view.statements.as_ref());
            a.literal = Some(if empty { b"nil" } else { b"expression" });
        }
        "InstanceVariableReadNode" => {
            a.helper = Some(DEFINED_IVAR_Q);
            a.arg = value.instance_var_read_name();
        }
        "ConstantReadNode" => {
            a.helper = Some(DEFINED_CONST_Q);
            a.arg = value.constant_read_name();
        }
        "ConstantPathNode" => defined_const_path(value, &mut a),
        "GlobalVariableReadNode" => {
            a.helper = Some(DEFINED_GVAR_Q);
            a.arg = value.global_var_read_name();
        }
        "BackReferenceReadNode" | "NumberedReferenceReadNode" => {
            a.unless_nil = Some(b"global-variable");
        }
        "ClassVariableReadNode" => {
            a.helper = Some(DEFINED_CVAR_Q);
            a.arg = value.class_var_read_name();
        }
        "YieldNode" => a.helper = Some(DEFINED_YIELD_Q),
        "SuperNode" | "ForwardingSuperNode" => a.helper = Some(DEFINED_SUPER_Q),
        "CallNode" => {
            if let Some(call) = value.call() {
                let literal_block = call
                    .block
                    .as_ref()
                    .map(|block| block.kind_name() == "BlockNode")
                    .unwrap_or(false);
                if literal_block {
                    a.literal = Some(b"expression");
                } else if call.receiver.is_none() {
                    a.helper = Some(DEFINED_METHOD_Q);
                    a.arg = Some(call.name);
                } else {
                    a.helper = Some(DEFINED_METHOD_ON_Q);
                    a.arg = Some(call.name);
                    a.recv = call.receiver;
                }
            }
        }
        _ => {}
    }
    a
}

/// Leave a constant path's names, root first, as an array at `cursp()`
/// (`gen_defined_path`).
fn gen_defined_path<N: BackendNode>(
    cg: &mut Codegen,
    a: &DefinedAnswer<N>,
) -> Result<(), Diagnostic> {
    for name in &a.path {
        let (session, scope) = cg.current();
        let sym = scope.new_sym(session, name)?;
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_LOADSYM, dst, sym)?;
        scope.push_n(1)?;
    }
    let len = count_u16(a.path.len() as i32)?;
    let (session, scope) = cg.current();
    scope.pop_n(len)?;
    let dst = scope.cursp();
    scope.genop_2(session, opcode::OP_ARRAY, dst, len)?;
    scope.push_n(1)
}

/// Leave an answer at `cursp()` as a frozen string (`gen_defined_literal`).
fn gen_defined_literal(cg: &mut Codegen, answer: &[u8]) -> Result<(), Diagnostic> {
    let index = {
        let (session, scope) = cg.current();
        scope.new_lit_str(session, answer)? as u16
    };
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_STRING, dst, index)?;
        scope.push_n(1)?;
        scope.push_n(1)?;
        scope.pop_n(1)?;
        scope.pop_n(1)?;
    }
    let sym = {
        let (session, scope) = cg.current();
        scope.new_sym(session, b"freeze")?
    };
    let (session, scope) = cg.current();
    let dst = scope.cursp();
    scope.genop_2(session, opcode::OP_SEND0, dst, sym)?;
    scope.push_n(1)
}

/// Emit the answer an operand's node type alone decides (`gen_defined_answer`).
fn gen_defined_answer<N: BackendNode>(
    cg: &mut Codegen,
    a: &DefinedAnswer<N>,
) -> Result<(), Diagnostic> {
    if let Some(literal) = a.literal {
        return gen_defined_literal(cg, literal);
    }
    let Some(helper) = a.helper else {
        return Err(internal_error("defined? answer has no helper"));
    };
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_1(session, opcode::OP_LOADSELF, dst)?;
    }
    if !a.path.is_empty() {
        {
            let (_, scope) = cg.current();
            scope.push_n(1)?;
        }
        {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            let op = if a.path_toplevel {
                opcode::OP_OCLASS
            } else {
                opcode::OP_LOADNIL
            };
            scope.genop_1(session, op, dst)?;
            scope.push_n(1)?;
        }
        gen_defined_path(cg, a)?;
        let sym = {
            let (session, scope) = cg.current();
            scope.new_sym(session, helper)?
        };
        {
            let (_, scope) = cg.current();
            scope.push_n(1)?;
            scope.pop_n(4)?;
        }
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_3(session, opcode::OP_SSEND, dst, sym, 2)?;
    } else if a.arg.is_none() {
        {
            let (_, scope) = cg.current();
            scope.push_n(1)?;
            scope.push_n(1)?;
            scope.pop_n(2)?;
        }
        let sym = {
            let (session, scope) = cg.current();
            scope.new_sym(session, helper)?
        };
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_SSEND0, dst, sym)?;
    } else {
        let arg = a.arg.as_deref().unwrap_or_default();
        let sym = {
            let (session, scope) = cg.current();
            scope.new_sym(session, arg)?
        };
        {
            let (_, scope) = cg.current();
            scope.push_n(1)?;
        }
        {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_2(session, opcode::OP_LOADSYM, dst, sym)?;
            scope.push_n(1)?;
            scope.push_n(1)?;
            scope.pop_n(3)?;
        }
        let sym = {
            let (session, scope) = cg.current();
            scope.new_sym(session, helper)?
        };
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_3(session, opcode::OP_SSEND, dst, sym, 1)?;
    }
    let (_, scope) = cg.current();
    scope.push_n(1)
}

/// Ask `__defined_method_on?`/`__defined_const_path?` about the receiver at
/// `cursp()-1` (`gen_defined_ask_method_on`).
fn gen_defined_ask_method_on<N: BackendNode>(
    cg: &mut Codegen,
    a: &DefinedAnswer<N>,
    keep: bool,
) -> Result<(), Diagnostic> {
    let recv = cg.current().1.cursp() - 1;
    if keep {
        {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_1(session, opcode::OP_LOADSELF, dst)?;
            scope.push_n(1)?;
        }
        {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.gen_move(session, dst, recv, true)?;
        }
    } else {
        {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.gen_move(session, dst, recv, true)?;
        }
        {
            let (session, scope) = cg.current();
            scope.genop_1(session, opcode::OP_LOADSELF, recv)?;
        }
    }
    {
        let (_, scope) = cg.current();
        scope.push_n(1)?;
    }
    if !a.path.is_empty() {
        gen_defined_path(cg, a)?;
    } else {
        let Some(arg) = a.arg.as_deref() else {
            return Err(internal_error("defined? symbol operand missing"));
        };
        let sym = {
            let (session, scope) = cg.current();
            scope.new_sym(session, arg)?
        };
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_LOADSYM, dst, sym)?;
        scope.push_n(1)?;
    }
    {
        let (_, scope) = cg.current();
        scope.push_n(1)?;
        scope.pop_n(4)?;
    }
    let Some(helper) = a.helper else {
        return Err(internal_error("defined? helper missing"));
    };
    let sym = {
        let (session, scope) = cg.current();
        scope.new_sym(session, helper)?
    };
    let (session, scope) = cg.current();
    let dst = scope.cursp();
    scope.genop_3(session, opcode::OP_SSEND, dst, sym, 2)?;
    scope.push_n(1)
}

/// Jump to the shared nil answer and lay it down (`codegen_defined` tail).
fn defined_finish(cg: &mut Codegen, nil_jmps: &mut u32) -> Result<(), Diagnostic> {
    if *nil_jmps == JMPLINK_START {
        return Ok(());
    }
    let done = cg.current().1.genjmp(opcode::OP_JMP, JMPLINK_START)?;
    cg.current().1.dispatch_linked(*nil_jmps)?;
    cg.current().1.pop_n(1)?;
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
        scope.push_n(1)?;
    }
    cg.current().1.dispatch(done)?;
    Ok(())
}

/// Parts of an operand whose own answers `defined?` weighs
/// (`defined_parts_of`).
fn defined_parts_of<N: BackendNode>(value: &N) -> Option<Vec<N>> {
    let list = match value.kind_name() {
        "ArrayNode" => value.raw_array_elements(),
        "HashNode" | "KeywordHashNode" => value.hash_elements(),
        "CallNode" => match value.call() {
            Some(call) => {
                let literal_block = call
                    .block
                    .as_ref()
                    .map(|block| block.kind_name() == "BlockNode")
                    .unwrap_or(false);
                if literal_block {
                    None
                } else {
                    call.args.and_then(|args| args.raw_call_args())
                }
            }
            None => None,
        },
        _ => None,
    };
    match list {
        Some(items) if !items.is_empty() => Some(items),
        _ => None,
    }
}

/// Weigh the parts of an operand (`gen_defined_parts`).
fn gen_defined_parts<N: BackendNode>(
    cg: &mut Codegen,
    value: &N,
    nil_jmps: &mut u32,
) -> Result<(), Diagnostic> {
    let rlev = cg.current().1.rlev;
    let Some(list) = defined_parts_of(value) else {
        return Ok(());
    };
    cg.current().1.rlev += 1;
    if cg.current().1.rlev > CODEGEN_LEVEL_MAX {
        cg.current().1.rlev = rlev;
        return Err(too_complex());
    }
    for part in list {
        gen_defined_part(cg, part, nil_jmps)?;
    }
    cg.current().1.rlev = rlev;
    Ok(())
}

/// Weigh one part of an operand (`gen_defined_part`).
fn gen_defined_part<N: BackendNode>(
    cg: &mut Codegen,
    mut part: N,
    nil_jmps: &mut u32,
) -> Result<(), Diagnostic> {
    match part.kind_name() {
        "BlockArgumentNode" | "ForwardingArgumentsNode" => return Ok(()),
        "SplatNode" => match part.splat_value() {
            Some(Some(inner)) => part = inner,
            _ => return Ok(()),
        },
        "AssocSplatNode" => match part.assoc_splat_value() {
            Some(Some(inner)) => part = inner,
            _ => return Ok(()),
        },
        "AssocNode" => {
            let Some((key, value)) = part.assoc_pair() else {
                return Ok(());
            };
            gen_defined_part(cg, key, nil_jmps)?;
            part = value;
        }
        _ => {}
    }
    let part = defined_operand(part);
    let a = defined_answer_for(&part);
    if a.literal.is_some() {
        return gen_defined_parts(cg, &part, nil_jmps);
    }
    codegen_defined(cg, part, true)?;
    cg.current().1.pop_n(1)?;
    let (session, scope) = cg.current();
    let cur = scope.cursp();
    *nil_jmps = scope.genjmp2(session, opcode::OP_JMPNOT, cur, *nil_jmps, false)?;
    Ok(())
}

/// `defined?(recv.meth)` and `defined?(expr::NAME)`
/// (`gen_defined_method_on`).
fn gen_defined_method_on<N: BackendNode>(
    cg: &mut Codegen,
    a: &mut DefinedAnswer<N>,
    nil_jmps: &mut u32,
) -> Result<(), Diagnostic> {
    let sp = cg.current().1.cursp();
    let catch_entry = cg.current().1.catch_new();
    let begin = cg.current().1.pc;
    let recv = a
        .recv
        .take()
        .ok_or_else(|| internal_error("defined? receiver missing"))?;
    gen_defined_recv(cg, recv, nil_jmps)?;
    gen_defined_ask_method_on(cg, a, false)?;
    let end = cg.current().1.pc;
    let ok = cg.current().1.genjmp(opcode::OP_JMP, JMPLINK_START)?;
    let target = cg.current().1.pc;
    cg.current().1.catch_set(catch_entry, 0, begin, end, target);
    {
        let (session, scope) = cg.current();
        scope.genop_1(session, opcode::OP_EXCEPT, sp)?;
    }
    {
        let (_, scope) = cg.current();
        *nil_jmps = scope.genjmp(opcode::OP_JMP, *nil_jmps)?;
    }
    cg.current().1.dispatch(ok)?;
    Ok(())
}

/// Node kinds the current codegen can evaluate as a receiver.
fn codegen_supports(kind: &str) -> bool {
    matches!(
        kind,
        "ProgramNode"
            | "StatementsNode"
            | "ElseNode"
            | "IntegerNode"
            | "FloatNode"
            | "StringNode"
            | "SymbolNode"
            | "CallNode"
            | "IfNode"
            | "UnlessNode"
            | "ArrayNode"
            | "HashNode"
            | "KeywordHashNode"
            | "CaseNode"
            | "InterpolatedStringNode"
            | "EmbeddedStatementsNode"
            | "EmbeddedVariableNode"
            | "WhileNode"
            | "UntilNode"
            | "AndNode"
            | "OrNode"
            | "LocalVariableReadNode"
            | "LocalVariableWriteNode"
            | "TrueNode"
            | "FalseNode"
            | "NilNode"
            | "SelfNode"
            | "AliasMethodNode"
            | "UndefNode"
    )
}

/// Leave a receiver's value at `cursp()-1`, or jump to the nil answer
/// (`gen_defined_recv`).
fn gen_defined_recv<N: BackendNode>(
    cg: &mut Codegen,
    value: N,
    nil_jmps: &mut u32,
) -> Result<(), Diagnostic> {
    let rlev = cg.current().1.rlev;
    let value = defined_operand(value);
    let mut a = defined_answer_for(&value);
    if a.recv.is_none() {
        if a.literal.is_some() {
            gen_defined_parts(cg, &value, nil_jmps)?;
        } else {
            if a.unless_nil.is_some() {
                cg.current().1.rlev = rlev;
                return Err(defined_gate(&value, "back-reference operand"));
            }
            // `gen_defined_part` finishes its own chain through
            // `codegen_defined`; only its JMPNOT joins the caller's chain.
            let mut local = JMPLINK_START;
            if a.helper.is_some() {
                gen_defined_parts(cg, &value, &mut local)?;
                gen_defined_answer(cg, &a)?;
            } else {
                let (session, scope) = cg.current();
                let dst = scope.cursp();
                scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
                scope.push_n(1)?;
            }
            defined_finish(cg, &mut local)?;
            cg.current().1.pop_n(1)?;
            let (session, scope) = cg.current();
            let cur = scope.cursp();
            *nil_jmps = scope.genjmp2(session, opcode::OP_JMPNOT, cur, *nil_jmps, false)?;
        }
        if !codegen_supports(value.kind_name()) {
            cg.current().1.rlev = rlev;
            return Err(defined_gate(&value, "receiver"));
        }
        codegen(cg, value, true)?;
        cg.current().1.rlev = rlev;
        return Ok(());
    }
    cg.current().1.rlev += 1;
    if cg.current().1.rlev > CODEGEN_LEVEL_MAX {
        cg.current().1.rlev = rlev;
        return Err(too_complex());
    }
    gen_defined_parts(cg, &value, nil_jmps)?;
    let recv = a
        .recv
        .take()
        .ok_or_else(|| internal_error("defined? receiver missing"))?;
    gen_defined_recv(cg, recv, nil_jmps)?;
    gen_defined_ask_method_on(cg, &a, true)?;
    cg.current().1.pop_n(1)?;
    {
        let (session, scope) = cg.current();
        let cur = scope.cursp();
        *nil_jmps = scope.genjmp2(session, opcode::OP_JMPNOT, cur, *nil_jmps, false)?;
    }
    if !a.path.is_empty() {
        for name in &a.path {
            let (session, scope) = cg.current();
            let sym = scope.new_sym(session, name)?;
            let dst = scope.cursp() - 1;
            scope.genop_2(session, opcode::OP_GETMCNST, dst, sym)?;
        }
    } else {
        gen_call_impl(cg, value, true, true)?;
    }
    cg.current().1.rlev = rlev;
    Ok(())
}

/// `defined?` (`codegen_defined`).
fn codegen_defined<N: BackendNode>(
    cg: &mut Codegen,
    value: N,
    val: bool,
) -> Result<(), Diagnostic> {
    let mut nil_jmps = JMPLINK_START;
    let rlev = cg.current().1.rlev;
    let value = defined_operand(value);
    let mut a = defined_answer_for(&value);
    if !val {
        return Ok(());
    }
    cg.current().1.rlev += 1;
    if cg.current().1.rlev > CODEGEN_LEVEL_MAX {
        cg.current().1.rlev = rlev;
        return Err(too_complex());
    }
    if a.literal.is_some() || a.helper.is_some() {
        gen_defined_parts(cg, &value, &mut nil_jmps)?;
        if a.recv.is_some() {
            gen_defined_method_on(cg, &mut a, &mut nil_jmps)?;
        } else {
            gen_defined_answer(cg, &a)?;
        }
    } else if let Some(unless_nil) = a.unless_nil {
        if !codegen_supports(value.kind_name()) {
            cg.current().1.rlev = rlev;
            return Err(defined_gate(&value, "back-reference operand"));
        }
        codegen(cg, value, true)?;
        cg.current().1.pop_n(1)?;
        {
            let (session, scope) = cg.current();
            let cur = scope.cursp();
            nil_jmps = scope.genjmp2(session, opcode::OP_JMPNIL, cur, nil_jmps, false)?;
        }
        gen_defined_literal(cg, unless_nil)?;
    } else {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
        scope.push_n(1)?;
    }
    defined_finish(cg, &mut nil_jmps)?;
    cg.current().1.rlev = rlev;
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

/// Method call (`gen_call`).
fn gen_call<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    gen_call_impl(cg, node, val, false)
}

/// Method call (`gen_call`); `recv_ready` means the receiver is already
/// evaluated at `cursp()-1` (chain links in `defined?`).
fn gen_call_impl<N: BackendNode>(
    cg: &mut Codegen,
    node: N,
    val: bool,
    recv_ready: bool,
) -> Result<(), Diagnostic> {
    let Some(view) = node.call() else {
        return Err(unsupported(&node, "call"));
    };
    if view.attr_write {
        return Err(unsupported(&node, "attribute assignment"));
    }
    let safe = view.safe_nav;
    let name = view.name;
    let (mut noself, mut noop) = (false, false);
    let sp_save = if recv_ready {
        cg.current().1.sp - 1
    } else {
        cg.current().1.sp
    };
    // With `recv_ready` the receiver already sits at `cursp()-1`.
    if !recv_ready {
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
            send_call(cg, noself, name, nargs, dst)?;
        }
    } else if !noop && nargs == 0 && gen_uniop(cg, name, dst)? {
        // A literal absorbed its sign.
    } else {
        send_call(cg, noself, name, nargs, dst)?;
    }
    if safe {
        cg.current().1.dispatch(skip)?;
    }
    if val {
        cg.current().1.push_n(1)?;
    }
    Ok(())
}

/// Generic `SEND`/`SSEND` emission.
fn send_call(
    cg: &mut Codegen,
    noself: bool,
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
