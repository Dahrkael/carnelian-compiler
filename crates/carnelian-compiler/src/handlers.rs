//! Node handlers: port of the `codegen()` tranches covering the P1 corpus.
//!
//! Dispatch is on `kind_name` (1:1 with the C `switch` on node type); values
//! travel through `BackendNode`, so FFI and owned frontends share handlers.

use carnelian_ast::view::{
    BackendNode, CallView, CallWriteView, IndexWriteView, IntegerLit, SimpleLit,
};

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
/// Forwarded-argument operand (`gen_call` with `...` carries `0xFF`, not a
/// nibble-packed count).
const FORWARD_ARGS: i32 = 0xFF;
/// Catch handler kinds (`enum mrc_catch_type`).
const CATCH_RESCUE: u8 = 0;
const CATCH_ENSURE: u8 = 1;
/// `$!` symbol (`MRC_SYM_2(errinfo)`).
const ERRINFO: &[u8] = b"$!";
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

/// Required-argument `aspec` (`MRC_ARGS_REQ`).
fn args_req(count: usize) -> u32 {
    ((count as u32) & 0x1f) << 18
}

/// Optional-argument `aspec` (`MRC_ARGS_OPT`).
fn args_opt(count: usize) -> u32 {
    ((count as u32) & 0x1f) << 13
}

/// Post-rest-argument `aspec` (`MRC_ARGS_POST`).
fn args_post(count: usize) -> u32 {
    ((count as u32) & 0x1f) << 7
}

/// Keyword-argument `aspec` (`MRC_ARGS_KEY(ka, kd)`).
fn args_key(count: usize, with_rest: bool) -> u32 {
    (((count as u32) & 0x1f) << 2) | u32::from(with_rest) << 1
}

/// Method `ainfo` for required-only parameters.
fn ainfo_req(count: usize) -> u16 {
    (((count as u32) & 0x3f) << 7) as u16
}

/// Forwarding operand for `ARGARY`/`BLKPUSH` (`mscope_operand`).
fn mscope_operand(ainfo: u16, level: u32) -> Result<u16, Diagnostic> {
    if u32::from(ainfo) > 0xfff {
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
    Ok((ainfo << 4) | level as u16)
}

/// Method local in `reg` into the cursor (`gen_mscope_lvar`).
fn gen_mscope_lvar(cg: &mut Codegen, reg: u16, level: u32) -> Result<(), Diagnostic> {
    if level == 0 {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.gen_move(session, dst, reg, false)?;
    } else {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        let depth = u8::try_from(level - 1).map_err(|_| too_complex())?;
        scope.genop_3(session, opcode::OP_GETUPVAR, dst, reg, depth)?;
    }
    cg.current().1.push_n(1)
}

/// Block slot of the method scope (`gen_blkmove`).
fn gen_blkmove(cg: &mut Codegen, ainfo: u16, level: u32) -> Result<(), Diagnostic> {
    let ainfo = u32::from(ainfo);
    let reg =
        (((ainfo >> 7) & 0x3f) + ((ainfo >> 6) & 0x1) + ((ainfo >> 1) & 0x1f) + (ainfo & 0x1) + 1)
            as u16;
    gen_mscope_lvar(cg, reg, level)
}

/// Scopes stack plus session and finished root.
pub struct Codegen {
    session: Session,
    scopes: Vec<Scope>,
    root: Option<Irep>,
    /// Line-start byte offsets of the source (`None` without source: lines
    /// stay `0`, as for synthetic trees).
    source_lines: Option<Vec<u32>>,
}

impl Codegen {
    /// New unit with the dummy top scope (`generate_code` head).
    pub fn new() -> Self {
        Self {
            session: Session::new(),
            scopes: vec![Scope::top()],
            root: None,
            source_lines: None,
        }
    }

    /// Install the compile filename on the top scope; children inherit it
    /// (`generate_code` seeds `scope->filename` from the filename table).
    pub fn set_filename(&mut self, filename: &[u8]) {
        if let Some(top) = self.scopes.first_mut() {
            top.filename = filename.to_vec();
        }
    }

    /// Compile filename (`filename_table[0]`); what `__FILE__` bakes.
    fn filename(&self) -> &[u8] {
        &self.scopes.first().expect("top").filename
    }

    /// Install the source for offset-to-line mapping (`node_lineno` reads
    /// the parser newline list; this is the same table, 1-based lines).
    pub fn set_source(&mut self, source: &[u8]) {
        let mut starts = vec![0u32];
        for (index, byte) in source.iter().enumerate() {
            if *byte == b'\n' {
                starts.push(index as u32 + 1);
            }
        }
        self.source_lines = Some(starts);
    }

    /// Line of a byte offset, or `None` without source.
    fn line_for(&self, offset: u32) -> Option<u16> {
        let starts = self.source_lines.as_ref()?;
        let line = starts.partition_point(|start| *start <= offset);
        Some(line.min(u16::MAX as usize) as u16)
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

    /// Open a method scope over `locals` with `OP_ENTER` (`lambda_body`).
    pub fn push_method_body(
        &mut self,
        locals: &[Vec<u8>],
        ainfo: u16,
        aspec: u32,
    ) -> Result<(), Diagnostic> {
        let mut child = Scope::child(&mut self.session, self.scopes.last().expect("top"), locals)?;
        child.ainfo = ainfo;
        child.aspec = aspec;
        child.mscope = true;
        child.genop_w(opcode::OP_ENTER, aspec)?;
        self.scopes.push(child);
        Ok(())
    }

    /// Nearest method scope for `super` (`search_mscope` without `eval`).
    /// Returns `ainfo` (`-1` with no method), levels between, and `aspec`.
    pub fn method_scope(&self) -> (i32, u32, u32) {
        let mut level: u32 = 0;
        let mut index = self.scopes.len();
        while index > 0 {
            index -= 1;
            let scope = &self.scopes[index];
            if scope.is_top {
                break;
            }
            if scope.mscope {
                return (i32::from(scope.ainfo), level, scope.aspec);
            }
            // The dummy top scope stands for no frame; anything past it has
            // no method to belong to.
            if index == 0 {
                break;
            }
            level += 1;
        }
        (-1, level, 0)
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

/// Main dispatch (`codegen()` switch) with the `s->rlev` save/restore that
/// surrounds every C node walk.
pub fn codegen<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    // Every node sets the active line before dispatch (`s->lineno`), so
    // each emitted byte carries its node's line.
    let line = cg.line_for(node.span().start);
    if let Some(line) = line {
        cg.current().1.lineno = line;
    }
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
        "RationalNode" => gen_rational(cg, node, val),
        "ImaginaryNode" => gen_imaginary(cg, node, val),
        "StringNode" => gen_string(cg, node, val),
        "SymbolNode" => gen_symbol(cg, node, val),
        "SourceFileNode" => gen_source_file(cg, node, val),
        "SourceLineNode" => gen_source_line(cg, node, val),
        "SourceEncodingNode" => gen_source_encoding(cg, node, val),
        "CallNode" => gen_call(cg, node, val),
        "IfNode" | "UnlessNode" => gen_if(cg, node, val),
        "ArrayNode" => gen_array(cg, node, val),
        "HashNode" | "KeywordHashNode" => gen_hash_lit(cg, node, val),
        "CaseNode" => gen_case(cg, node, val),
        "CaseMatchNode" => gen_case_match(cg, node, val),
        "MatchPredicateNode" => gen_match_predicate(cg, node, val),
        "MatchRequiredNode" => gen_match_required(cg, node, val),
        "InterpolatedStringNode" => gen_interp_string(cg, node, val),
        "InterpolatedSymbolNode" => gen_interp_symbol(cg, node, val),
        "EmbeddedStatementsNode" => gen_branch(cg, node.embedded_body(), val),
        "EmbeddedVariableNode" => {
            let Some(variable) = node.embedded_var() else {
                return Err(unsupported(&node, "embedded variable"));
            };
            codegen(cg, variable, val)
        }
        "WhileNode" | "UntilNode" => gen_while(cg, node, val),
        "ForNode" => gen_for(cg, node, val),
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
        "ItLocalVariableReadNode" => gen_it_read(cg, node, val),
        "BlockNode" => gen_block(cg, node, val),
        "LambdaNode" => gen_lambda(cg, node, val),
        "YieldNode" => gen_yield(cg, node, val),
        "BlockArgumentNode" => gen_block_arg(cg, node, val),
        "BeginNode" => gen_begin_node(cg, node, val),
        "RescueModifierNode" => gen_rescue_modifier(cg, node, val),
        "MultiWriteNode" => gen_multi_write(cg, node, val),
        "SplatNode" => gen_splat(cg, node, val),
        "RetryNode" => gen_retry(cg, node, val),
        "TrueNode" | "FalseNode" | "NilNode" | "SelfNode" => gen_simple(cg, node, val),
        "DefNode" => gen_def(cg, node, val),
        "ClassNode" => gen_class(cg, node, val),
        "ModuleNode" => gen_module(cg, node, val),
        "SingletonClassNode" => gen_sclass(cg, node, val),
        "ConstantReadNode" => gen_const_read(cg, node, val),
        "ConstantWriteNode" => gen_const_write(cg, node, val),
        "ConstantPathNode" => gen_const_path_read(cg, node, val),
        "ConstantPathWriteNode" => gen_const_path_write(cg, node, val),
        "InstanceVariableReadNode" => gen_ivar_read(cg, node, val),
        "InstanceVariableWriteNode" => gen_ivar_write(cg, node, val),
        "ClassVariableReadNode" => gen_cvar_read(cg, node, val),
        "ClassVariableWriteNode" => gen_cvar_write(cg, node, val),
        "GlobalVariableReadNode" => gen_gvar_read(cg, node, val),
        "GlobalVariableWriteNode" => gen_gvar_write(cg, node, val),
        "LocalVariableOperatorWriteNode"
        | "GlobalVariableOperatorWriteNode"
        | "InstanceVariableOperatorWriteNode"
        | "ClassVariableOperatorWriteNode"
        | "ConstantOperatorWriteNode" => gen_op_write(cg, node, val),
        "LocalVariableOrWriteNode"
        | "LocalVariableAndWriteNode"
        | "GlobalVariableOrWriteNode"
        | "GlobalVariableAndWriteNode"
        | "InstanceVariableOrWriteNode"
        | "InstanceVariableAndWriteNode"
        | "ClassVariableOrWriteNode"
        | "ClassVariableAndWriteNode"
        | "ConstantOrWriteNode"
        | "ConstantAndWriteNode" => gen_logic_write(cg, node, val),
        "CallOperatorWriteNode" | "CallOrWriteNode" | "CallAndWriteNode" => {
            gen_call_write(cg, node, val)
        }
        "IndexOperatorWriteNode" | "IndexOrWriteNode" | "IndexAndWriteNode" => {
            gen_index_write(cg, node, val)
        }
        "ConstantPathOperatorWriteNode" => Err(const_reassign(&node)),
        "ConstantPathOrWriteNode" | "ConstantPathAndWriteNode" => Err(const_path_logic_gate(&node)),
        "BackReferenceReadNode" => gen_backref(cg, node, val),
        "NumberedReferenceReadNode" => gen_numbered_ref(cg, node, val),
        "SuperNode" => gen_super(cg, node, val),
        "ForwardingSuperNode" => gen_zsuper(cg, node, val),
        "AliasMethodNode" => gen_alias(cg, node, val),
        "UndefNode" => gen_undef(cg, node, val),
        "DefinedNode" => {
            let Some(value) = node.defined_value() else {
                return Err(unsupported(&node, "defined?"));
            };
            codegen_defined(cg, value, val)
        }
        "ReturnNode" => gen_return_node(cg, node, val),
        "BreakNode" => gen_break(cg, node, val),
        "NextNode" => gen_next(cg, node, val),
        "RedoNode" => gen_redo(cg, node, val),
        "RangeNode" => gen_range(cg, node, val),
        "ParenthesesNode" => gen_parentheses(cg, node, val),
        "MatchWriteNode" => {
            let Some(call) = node.match_write() else {
                return Err(unsupported(&node, "match write"));
            };
            // Named captures bind no locals; only the `=~` call emits code.
            codegen(cg, call, val)
        }
        "ImplicitNode" => {
            let Some(inner) = node.implicit_value() else {
                return Err(unsupported(&node, "implicit node"));
            };
            codegen(cg, inner, val)
        }
        "ArgumentsNode" => gen_arguments(cg, node, val),
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
    emit_integer_lit(cg, lit)
}

/// Integer literal emission shared by `IntegerNode` and the `RationalNode`
/// parts (`gen_pm_integer` plus the caller's `push`).
fn emit_integer_lit(cg: &mut Codegen, lit: IntegerLit) -> Result<(), Diagnostic> {
    match lit {
        IntegerLit::I64(value) => {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.gen_int(session, dst, value)?;
            scope.push_n(1)
        }
        IntegerLit::Bigint { digits, negative } => {
            let index = {
                let (session, scope) = cg.current();
                scope.new_litbint(session, &digits, 10, negative)? as u16
            };
            emit_load2(cg, opcode::OP_LOADL, index)
        }
    }
}

/// `Nr` literal (`PM_RATIONAL_NODE`): `Rational(numerator, denominator)`
/// through `OP_SSEND` with the block-slot reserve.
fn gen_rational<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    if !val {
        return Ok(());
    }
    let Some((numerator, denominator)) = node.rational() else {
        return Err(unsupported(&node, "rational literal"));
    };
    let recv = cg.current().1.cursp();
    cg.current().1.push_n(1)?;
    emit_integer_lit(cg, numerator)?;
    emit_integer_lit(cg, denominator)?;
    {
        let (_, scope) = cg.current();
        scope.push_n(1)?;
        scope.pop_n(1)?;
        scope.pop_n(3)?;
    }
    let sym = {
        let (session, scope) = cg.current();
        scope.new_sym(session, b"Rational")?
    };
    {
        let (session, scope) = cg.current();
        scope.genop_3(session, opcode::OP_SSEND, recv, sym, 2)?;
    }
    cg.current().1.push_n(1)
}

/// `Ni` literal (`PM_IMAGINARY_NODE`): `Complex(0, numeric)` through
/// `OP_SSEND` with the block-slot reserve.
fn gen_imaginary<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    if !val {
        return Ok(());
    }
    let Some(numeric) = node.imaginary() else {
        return Err(unsupported(&node, "imaginary literal"));
    };
    let recv = cg.current().1.cursp();
    cg.current().1.push_n(1)?;
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.gen_int(session, dst, 0)?;
        scope.push_n(1)?;
    }
    codegen(cg, numeric, true)?;
    {
        let (_, scope) = cg.current();
        scope.push_n(1)?;
        scope.pop_n(1)?;
        scope.pop_n(3)?;
    }
    let sym = {
        let (session, scope) = cg.current();
        scope.new_sym(session, b"Complex")?
    };
    {
        let (session, scope) = cg.current();
        scope.genop_3(session, opcode::OP_SSEND, recv, sym, 2)?;
    }
    cg.current().1.push_n(1)
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

/// `__FILE__` (`PM_SOURCE_FILE_NODE`): the parse filepath as a string
/// literal (`new_lit_str` + `OP_STRING`, valued only).
fn gen_source_file<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    if !val {
        return Ok(());
    }
    if node.source_file().is_none() {
        return Err(unsupported(&node, "source file"));
    }
    // C bakes `cast->filepath`, which its parser seeds from the compile
    // filename table (`filename_table[0]`, `"-e"` for string compiles).
    // Our parses carry no filepath option (the `ruby-prism` wrapper exposes
    // none), so the node bytes are empty; the Codegen filename installed by
    // `set_filename` is the same value through our own table.
    let filename = cg.filename().to_vec();
    let (session, scope) = cg.current();
    let index = scope.new_lit_str(session, &filename)? as u16;
    emit_load2(cg, opcode::OP_STRING, index)
}

/// `__LINE__` (`PM_SOURCE_LINE_NODE`): the node's source line (the
/// `node_lineno` newline-list lookup, valued only).
fn gen_source_line<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    if !val {
        return Ok(());
    }
    if node.source_line().is_none() {
        return Err(unsupported(&node, "source line"));
    }
    let line = cg.line_for(node.span().start).unwrap_or(0);
    let (session, scope) = cg.current();
    let dst = scope.cursp();
    scope.gen_int(session, dst, i64::from(line))?;
    scope.push_n(1)
}

/// `__ENCODING__` (`PM_SOURCE_ENCODING_NODE`): a zero-argument `OP_SSEND`
/// on self. The C arm has no `val` guard, so this always emits (even in
/// void context), including the trailing `push(); pop();` `nregs`
/// workaround.
fn gen_source_encoding<N: BackendNode>(
    cg: &mut Codegen,
    node: N,
    _val: bool,
) -> Result<(), Diagnostic> {
    if node.source_encoding().is_none() {
        return Err(unsupported(&node, "source encoding"));
    }
    let sym = {
        let (session, scope) = cg.current();
        scope.new_sym(session, b"__ENCODING__")?
    };
    let (session, scope) = cg.current();
    let dst = scope.cursp();
    scope.genop_3(session, opcode::OP_SSEND, dst, sym, 0)?;
    scope.push_n(1)?;
    scope.push_n(1)?;
    scope.pop_n(1)?;
    Ok(())
}

/// Shared interpolation loop (`PM_INTERPOLATED_STRING_NODE` and
/// `PM_INTERPOLATED_SYMBOL_NODE` in C): `STRING` parts joined with
/// `STRCAT`, with a leading empty literal unless the first part is already
/// a string (so `STRCAT` never mutates a shared literal). Callers check for
/// empty parts first, so an empty list here is unreachable.
fn gen_interp_loop<N: BackendNode>(
    cg: &mut Codegen,
    parts: Vec<N>,
    val: bool,
) -> Result<(), Diagnostic> {
    let Some(first) = parts.first() else {
        return Err(internal_error("empty interpolation"));
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

/// Interpolated string (`PM_INTERPOLATED_STRING_NODE`): the shared loop.
fn gen_interp_string<N: BackendNode>(
    cg: &mut Codegen,
    node: N,
    val: bool,
) -> Result<(), Diagnostic> {
    let Some(parts) = node.string_parts() else {
        return Err(unsupported(&node, "interpolated string"));
    };
    if parts.is_empty() {
        return Err(unsupported(&node, "empty interpolated string"));
    }
    gen_interp_loop(cg, parts, val)
}

/// Interpolated symbol (`PM_INTERPOLATED_SYMBOL_NODE`): the shared loop,
/// then the symbol tail (`pop`, the `OP_STRING`-at-cursor peephole into
/// `OP_SYMBOL`, else `OP_INTERN`, `push`).
fn gen_interp_symbol<N: BackendNode>(
    cg: &mut Codegen,
    node: N,
    val: bool,
) -> Result<(), Diagnostic> {
    let Some(parts) = node.interp_symbol() else {
        return Err(unsupported(&node, "interpolated symbol"));
    };
    if parts.is_empty() {
        return Err(unsupported(&node, "empty interpolated symbol"));
    }
    gen_interp_loop(cg, parts, val)?;
    if val {
        cg.current().1.pop_n(1)?;
        let fused = {
            let (session, scope) = cg.current();
            if scope.no_peephole(session) {
                None
            } else {
                let data = scope.last_insn();
                if data.insn == opcode::OP_STRING && data.a == u32::from(scope.cursp()) {
                    Some((data.a as u16, data.b))
                } else {
                    None
                }
            }
        };
        match fused {
            Some((dst, index)) => {
                let (session, scope) = cg.current();
                scope.pc = scope.lastpc;
                scope.genop_2(session, opcode::OP_SYMBOL, dst, index)?;
                scope.push_n(1)?;
            }
            None => {
                let (session, scope) = cg.current();
                let dst = scope.cursp();
                scope.genop_1(session, opcode::OP_INTERN, dst)?;
                scope.push_n(1)?;
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
        let (slot, level) = cg.search_upvar(name)?;
        let (session, scope) = cg.current();
        scope.gen_setupvar(session, sp, slot, level, val)
    }
}

/// Assignment to one target (`gen_assignment`): like C, a value-carrying
/// right-hand side evaluates first (constant paths evaluate theirs inside,
/// after the parent), then each target kind stores from `sp`.
fn gen_assignment<N: BackendNode>(
    cg: &mut Codegen,
    tree: N,
    rhs: Option<N>,
    mut sp: u16,
    val: bool,
) -> Result<(), Diagnostic> {
    match tree.kind_name() {
        // The pinned reference accepts constant path *writes* here but
        // rejects *targets* (`Not implemented (#1)`), so targets stay
        // gated: no golden can exist for them.
        "ConstantPathWriteNode" => {
            gen_const_path_target(cg, &tree, rhs, sp)?;
            if val {
                cg.current().1.push_n(1)?;
            }
            return Ok(());
        }
        "LocalVariableWriteNode"
        | "LocalVariableTargetNode"
        | "RequiredParameterNode"
        | "InstanceVariableWriteNode"
        | "InstanceVariableTargetNode"
        | "ConstantWriteNode"
        | "ConstantTargetNode"
        | "GlobalVariableWriteNode"
        | "GlobalVariableTargetNode"
        | "ClassVariableWriteNode"
        | "ClassVariableTargetNode"
        | "MultiTargetNode"
        | "IndexTargetNode"
        | "CallTargetNode" => {
            if let Some(value) = rhs {
                codegen(cg, value, true)?;
                cg.current().1.pop_n(1)?;
                sp = cg.current().1.cursp();
            }
        }
        _ => return Err(unsupported(&tree, "assignment target")),
    }
    match tree.kind_name() {
        "LocalVariableWriteNode" | "LocalVariableTargetNode" | "RequiredParameterNode" => {
            // A parameter is always a local in the current scope: no `depth`
            // field exists on the node, so the depth is `0` (C reads the
            // sibling layout instead of the missing field for the same).
            let (name, depth) = if tree.kind_name() == "LocalVariableWriteNode" {
                let Some(write) = tree.lvar_write() else {
                    return Err(unsupported(&tree, "assignment target"));
                };
                (write.name, write.depth)
            } else if tree.kind_name() == "RequiredParameterNode" {
                let Some(name) = tree.required_param_name() else {
                    return Err(unsupported(&tree, "assignment target"));
                };
                (name, 0)
            } else {
                let Some(target) = tree.lvar_target() else {
                    return Err(unsupported(&tree, "assignment target"));
                };
                (target.name, target.depth)
            };
            let depth = depth + u32::from(cg.current().1.for_depth);
            gen_assignment_lvar(cg, sp, &name, depth, val)?;
        }
        "InstanceVariableWriteNode" | "InstanceVariableTargetNode" => {
            let Some(name) = either_target_name(
                &tree,
                tree.ivar_write().map(|write| write.name),
                tree.ivar_target_name(),
            ) else {
                return Err(unsupported(&tree, "assignment target"));
            };
            let (session, scope) = cg.current();
            scope.gen_setxv(session, opcode::OP_SETIV, sp, &name, val)?;
        }
        "ConstantWriteNode" | "ConstantTargetNode" => {
            let Some(name) = either_target_name(
                &tree,
                tree.const_write().map(|write| write.name),
                tree.const_target_name(),
            ) else {
                return Err(unsupported(&tree, "assignment target"));
            };
            let (session, scope) = cg.current();
            scope.gen_setxv(session, opcode::OP_SETCONST, sp, &name, val)?;
        }
        "GlobalVariableWriteNode" | "GlobalVariableTargetNode" => {
            let Some(name) = either_target_name(
                &tree,
                tree.gvar_write().map(|write| write.name),
                tree.gvar_target_name(),
            ) else {
                return Err(unsupported(&tree, "assignment target"));
            };
            let (session, scope) = cg.current();
            scope.gen_setxv(session, opcode::OP_SETGV, sp, &name, val)?;
        }
        "ClassVariableWriteNode" | "ClassVariableTargetNode" => {
            let Some(name) = either_target_name(
                &tree,
                tree.cvar_write().map(|write| write.name),
                tree.cvar_target_name(),
            ) else {
                return Err(unsupported(&tree, "assignment target"));
            };
            let (session, scope) = cg.current();
            scope.gen_setxv(session, opcode::OP_SETCV, sp, &name, val)?;
        }
        "MultiTargetNode" => {
            let Some(view) = tree.multi_target_view() else {
                return Err(unsupported(&tree, "assignment target"));
            };
            gen_massignment(cg, view.lefts, view.rest, view.rights, i32::from(sp), val)?;
        }
        "IndexTargetNode" => {
            gen_index_target(cg, &tree, sp)?;
        }
        "CallTargetNode" => {
            gen_call_target(cg, &tree, sp)?;
        }
        _ => return Err(unsupported(&tree, "assignment target")),
    }
    if val {
        cg.current().1.push_n(1)?;
    }
    Ok(())
}

/// Name of a variable/constant target in either node form (a write's own
/// value child is the caller's `sp` here, never read).
fn either_target_name<N: BackendNode>(
    tree: &N,
    write_name: Option<Vec<u8>>,
    target_name: Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    if tree.kind_name().ends_with("WriteNode") {
        write_name
    } else {
        target_name
    }
}

/// Constant path target (`gen_assignment` path arm): the value in `sp`
/// moves aside while the parent evaluates, then `OP_SETMCNST` stores it.
fn gen_const_path_target<N: BackendNode>(
    cg: &mut Codegen,
    tree: &N,
    rhs: Option<N>,
    sp: u16,
) -> Result<(), Diagnostic> {
    let (parent, name) = if tree.kind_name() == "ConstantPathWriteNode" {
        let Some(write) = tree.const_path_write() else {
            return Err(unsupported(tree, "assignment target"));
        };
        (write.parent, write.name)
    } else {
        let Some((parent, name)) = tree.const_path_target() else {
            return Err(unsupported(tree, "assignment target"));
        };
        (parent, name)
    };
    let mut sp = sp;
    if sp != 0 {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.gen_move(session, dst, sp, false)?;
    }
    sp = cg.current().1.cursp();
    cg.current().1.push_n(1)?;
    let sym = match parent {
        Some(parent) => {
            codegen(cg, parent, true)?;
            let (session, scope) = cg.current();
            scope.new_sym(session, &name)?
        }
        None => {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_1(session, opcode::OP_OCLASS, dst)?;
            scope.push_n(1)?;
            let (session, scope) = cg.current();
            scope.new_sym(session, &name)?
        }
    };
    if let Some(value) = rhs {
        codegen(cg, value, true)?;
        cg.current().1.pop_n(1)?;
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.gen_move(session, sp, dst, false)?;
    }
    cg.current().1.pop_n(2)?;
    {
        let (session, scope) = cg.current();
        scope.genop_2(session, opcode::OP_SETMCNST, sp, sym)?;
    }
    Ok(())
}

/// Index assignment target (`gen_assignment` index arm): `recv[args] = sp`
/// via `OP_SETIDX` for one index or an `[]=` send otherwise.
fn gen_index_target<N: BackendNode>(cg: &mut Codegen, tree: &N, sp: u16) -> Result<(), Diagnostic> {
    let Some(view) = tree.index_target() else {
        return Err(unsupported(tree, "assignment target"));
    };
    codegen(cg, view.receiver, true)?;
    // One slot less than call sites: the value in `sp` is an argument too.
    let items = match view.args {
        None => Vec::new(),
        Some(args) => args
            .raw_call_args()
            .ok_or_else(|| unsupported(tree, "assignment target"))?,
    };
    let count = gen_values(cg, items, true, 13)?;
    if count < 0 {
        cg.current().1.push_n(1)?;
        {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_2(session, opcode::OP_MOVE, dst, sp)?;
        }
        cg.current().1.push_n(1)?;
        cg.current().1.pop_n(1)?;
        cg.current().1.pop_n(1)?;
        {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_2(session, opcode::OP_ARYPUSH, dst, 1)?;
        }
        cg.current().1.pop_n(1)?;
        let sym = {
            let (session, scope) = cg.current();
            scope.new_sym(session, b"[]=")?
        };
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_3(session, opcode::OP_SEND, dst, sym, CALL_MAXARGS as u8)?;
        return Ok(());
    }
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_MOVE, dst, sp)?;
    }
    cg.current().1.push_n(1)?;
    if count == 1 {
        cg.current().1.pop_n(3)?;
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_1(session, opcode::OP_SETIDX, dst)?;
    } else {
        cg.current().1.push_n(1)?;
        cg.current().1.pop_n(1)?;
        cg.current().1.pop_n((count + 2) as u16)?;
        let sym = {
            let (session, scope) = cg.current();
            scope.new_sym(session, b"[]=")?
        };
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_3(session, opcode::OP_SEND, dst, sym, (count + 1) as u8)?;
    }
    Ok(())
}

/// Call (attribute) assignment target (`gen_assignment` call arm):
/// `recv.name = sp` via a one-argument send (`OP_SSEND` for bare `self`).
fn gen_call_target<N: BackendNode>(cg: &mut Codegen, tree: &N, sp: u16) -> Result<(), Diagnostic> {
    let Some(view) = tree.call_target() else {
        return Err(unsupported(tree, "assignment target"));
    };
    // A written `self` is a call on self, so `OP_SSEND`, which fills the
    // receiver register itself.
    let noself = view.receiver.kind_name() == "SelfNode";
    if noself {
        cg.current().1.push_n(1)?;
    } else {
        codegen(cg, view.receiver, true)?;
    }
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_MOVE, dst, sp)?;
    }
    cg.current().1.push_n(1)?;
    cg.current().1.push_n(1)?;
    cg.current().1.pop_n(1)?;
    cg.current().1.pop_n(2)?;
    let sym = {
        let (session, scope) = cg.current();
        scope.new_sym(session, &view.name)?
    };
    let (session, scope) = cg.current();
    let dst = scope.cursp();
    scope.genop_3(
        session,
        if noself {
            opcode::OP_SSEND
        } else {
            opcode::OP_SEND
        },
        dst,
        sym,
        1,
    )?;
    Ok(())
}

/// Constant path reassignment (`Parent::Name += v`): the reference fails
/// with `constant re-assignment`, so the gate carries the same marker.
fn const_reassign<N: BackendNode>(node: &N) -> Diagnostic {
    let span = node.span();
    Diagnostic {
        message: format!("constant re-assignment: {}", node.kind_name()),
        start: span.start,
        end: span.end,
    }
}

/// Constant path `||=`/`&&=` (`Parent::Name ||= v`): the reference fails with
/// `Not implemented: PM_CONSTANT_PATH_*_WRITE_NODE`, so the gate carries the
/// same marker.
fn const_path_logic_gate<N: BackendNode>(node: &N) -> Diagnostic {
    let span = node.span();
    Diagnostic {
        message: format!("Not implemented: {}", node.kind_name()),
        start: span.start,
        end: span.end,
    }
}

/// Scalar operator write (`x += v`, `@x -= v`, `$g *= v`, `@@c /= v`,
/// `C %= v`, `case PM_*_OPERATOR_WRITE_NODE`): read the target, apply the
/// binary operator, store the result.
fn gen_op_write<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let kind = node.kind_name();
    let Some(view) = node.op_write() else {
        return Err(unsupported(&node, "operator assignment"));
    };
    let is_lvar = kind == "LocalVariableOperatorWriteNode";
    let depth = if is_lvar {
        view.depth + u32::from(cg.current().1.for_depth)
    } else {
        0
    };
    if is_lvar {
        gen_lvar(cg, &view.name, depth)?;
    } else {
        let op = match kind {
            "GlobalVariableOperatorWriteNode" => opcode::OP_GETGV,
            "InstanceVariableOperatorWriteNode" => opcode::OP_GETIV,
            "ClassVariableOperatorWriteNode" => opcode::OP_GETCV,
            "ConstantOperatorWriteNode" => opcode::OP_GETCONST,
            _ => return Err(unsupported(&node, "operator assignment")),
        };
        let sym = {
            let (session, scope) = cg.current();
            scope.new_sym(session, &view.name)?
        };
        {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_2(session, op, dst, sym)?;
        }
        cg.current().1.push_n(1)?;
    }
    codegen(cg, view.value, true)?;
    cg.current().1.push_n(1)?;
    cg.current().1.pop_n(1)?;
    cg.current().1.pop_n(2)?;
    gen_binary_operator(cg, &view.binary_operator)?;
    if is_lvar {
        let sp = cg.current().1.cursp();
        gen_assignment_lvar(cg, sp, &view.name, depth, val)?;
    } else {
        let op = match kind {
            "GlobalVariableOperatorWriteNode" => opcode::OP_SETGV,
            "InstanceVariableOperatorWriteNode" => opcode::OP_SETIV,
            "ClassVariableOperatorWriteNode" => opcode::OP_SETCV,
            "ConstantOperatorWriteNode" => opcode::OP_SETCONST,
            _ => return Err(unsupported(&node, "operator assignment")),
        };
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.gen_setxv(session, op, dst, &view.name, val)?;
    }
    if val {
        cg.current().1.push_n(1)?;
    }
    Ok(())
}

/// Scalar `||=`/`&&=` write (`case PM_*_OR_WRITE_NODE` /
/// `PM_*_AND_WRITE_NODE`): read the target, keep it on a truthiness jump,
/// otherwise store the right-hand side. `@@x ||=` and `C ||=` read under a
/// rescue that answers false when the variable is undefined.
fn gen_logic_write<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let kind = node.kind_name();
    let Some(view) = node.logic_write() else {
        return Err(unsupported(&node, "or assignment"));
    };
    let op_jmp = if kind.ends_with("OrWriteNode") {
        opcode::OP_JMPIF
    } else {
        opcode::OP_JMPNOT
    };
    let is_lvar = kind == "LocalVariableOrWriteNode" || kind == "LocalVariableAndWriteNode";
    let depth = if is_lvar {
        view.depth + u32::from(cg.current().1.for_depth)
    } else {
        0
    };
    if is_lvar {
        gen_lvar(cg, &view.name, depth)?;
    } else if kind == "ClassVariableOrWriteNode" || kind == "ConstantOrWriteNode" {
        let op = if kind == "ClassVariableOrWriteNode" {
            opcode::OP_GETCV
        } else {
            opcode::OP_GETCONST
        };
        gen_logic_rescue_read(cg, &view.name, op)?;
    } else {
        let op = match kind {
            "GlobalVariableOrWriteNode" | "GlobalVariableAndWriteNode" => opcode::OP_GETGV,
            "InstanceVariableOrWriteNode" | "InstanceVariableAndWriteNode" => opcode::OP_GETIV,
            "ClassVariableAndWriteNode" => opcode::OP_GETCV,
            "ConstantAndWriteNode" => opcode::OP_GETCONST,
            _ => return Err(unsupported(&node, "or assignment")),
        };
        let sym = {
            let (session, scope) = cg.current();
            scope.new_sym(session, &view.name)?
        };
        {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_2(session, op, dst, sym)?;
        }
        cg.current().1.push_n(1)?;
    }
    cg.current().1.pop_n(1)?;
    let pos = {
        let (session, scope) = cg.current();
        let cur = scope.cursp();
        scope.genjmp2(session, op_jmp, cur, JMPLINK_START, val)?
    };
    codegen(cg, view.value, true)?;
    cg.current().1.pop_n(1)?;
    if is_lvar {
        let sp = cg.current().1.cursp();
        gen_assignment_lvar(cg, sp, &view.name, depth, val)?;
        if val {
            cg.current().1.push_n(1)?;
        }
    } else {
        let op = match kind {
            "GlobalVariableOrWriteNode" | "GlobalVariableAndWriteNode" => opcode::OP_SETGV,
            "InstanceVariableOrWriteNode" | "InstanceVariableAndWriteNode" => opcode::OP_SETIV,
            "ClassVariableOrWriteNode" | "ClassVariableAndWriteNode" => opcode::OP_SETCV,
            "ConstantOrWriteNode" | "ConstantAndWriteNode" => opcode::OP_SETCONST,
            _ => return Err(unsupported(&node, "or assignment")),
        };
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.gen_setxv(session, op, dst, &view.name, val)?;
        if val {
            cg.current().1.push_n(1)?;
        }
    }
    cg.current().1.dispatch(pos)?;
    Ok(())
}

/// Guarded read for `@@x ||=` and `C ||=`: the `GET` runs under a rescue
/// that answers false when the variable is undefined (`loop_push` with
/// `LOOP_BEGIN`, retargeted to `LOOP_RESCUE` before the pop, like `gen_begin`).
fn gen_logic_rescue_read(cg: &mut Codegen, name: &[u8], op_get: u8) -> Result<(), Diagnostic> {
    {
        let (_, scope) = cg.current();
        scope.loop_push(LoopType::Begin);
        let pc0 = scope.new_label();
        scope.loops.last_mut().expect("loop").pc0 = pc0;
    }
    let catch_entry = cg.current().1.catch_new();
    let begin = cg.current().1.pc;
    let exc = cg.current().1.cursp();
    {
        let (session, scope) = cg.current();
        let sym = scope.new_sym(session, name)?;
        let dst = scope.cursp();
        scope.genop_2(session, op_get, dst, sym)?;
    }
    cg.current().1.push_n(1)?;
    let end = cg.current().1.pc;
    let noexc = cg.current().1.genjmp(opcode::OP_JMP, JMPLINK_START)?;
    cg.current().1.loops.last_mut().expect("loop").kind = LoopType::Rescue;
    {
        let target = cg.current().1.pc;
        cg.current()
            .1
            .catch_set(catch_entry, CATCH_RESCUE, begin, end, target);
    }
    {
        let (session, scope) = cg.current();
        scope.genop_1(session, opcode::OP_EXCEPT, exc)?;
        scope.genop_1(session, opcode::OP_LOADFALSE, exc)?;
    }
    cg.current().1.dispatch(noexc)?;
    {
        let (session, scope) = cg.current();
        scope.loop_pop(session, false)?;
    }
    Ok(())
}

/// Call operator/`||=`/`&&=` write (`obj.foo += v`, `obj.foo ||= v`,
/// `obj.foo &&= v`, `case PM_CALL_*_WRITE_NODE`): send the getter, combine
/// or test, send the setter. A written `self` reads and writes through
/// `OP_SSEND` so a private accessor stays reachable.
fn gen_call_write<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let kind = node.kind_name();
    let Some(view) = node.call_write() else {
        return Err(unsupported(&node, "call assignment"));
    };
    let CallWriteView {
        receiver,
        read_name,
        write_name,
        binary_operator,
        value,
        safe_nav,
    } = view;
    let is_or = kind == "CallOrWriteNode";
    let op_jmp = if is_or {
        opcode::OP_JMPIF
    } else {
        opcode::OP_JMPNOT
    };
    let op_send = match &receiver {
        Some(recv) if recv.kind_name() != "SelfNode" => opcode::OP_SEND,
        // An absent receiver is an implicit `self`, like a bare call.
        _ => opcode::OP_SSEND,
    };
    let mut vsp: Option<u16> = None;
    if val {
        let slot = cg.current().1.cursp();
        cg.current().1.push_n(1)?;
        vsp = Some(slot);
    }
    match receiver {
        None => cg.current().1.push_n(1)?,
        Some(recv) => codegen(cg, recv, true)?,
    }
    let mut skip = JMPLINK_START;
    if safe_nav {
        let recv = cg.current().1.cursp() - 1;
        {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.gen_move(session, dst, recv, true)?;
        }
        skip = {
            let (session, scope) = cg.current();
            let cur = scope.cursp();
            scope.genjmp2(session, opcode::OP_JMPNIL, cur, JMPLINK_START, val)?
        };
    }
    let read_sym = {
        let (session, scope) = cg.current();
        scope.new_sym(session, &read_name)?
    };
    let base = cg.current().1.cursp() - 1;
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.gen_move(session, dst, base, true)?;
    }
    cg.current().1.push_n(2)?;
    cg.current().1.pop_n(2)?;
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_3(session, op_send, dst, read_sym, 0)?;
    }
    let mut pos = JMPLINK_START;
    if let Some(operator) = binary_operator {
        cg.current().1.push_n(1)?;
        codegen(cg, value, true)?;
        cg.current().1.push_n(1)?;
        cg.current().1.pop_n(1)?;
        cg.current().1.pop_n(2)?;
        gen_binary_operator(cg, &operator)?;
    } else {
        if let Some(slot) = vsp {
            let (session, scope) = cg.current();
            let cur = scope.cursp();
            scope.gen_move(session, slot, cur, false)?;
        }
        pos = {
            let (session, scope) = cg.current();
            let cur = scope.cursp();
            scope.genjmp2(session, op_jmp, cur, JMPLINK_START, val)?
        };
        codegen(cg, value, true)?;
        cg.current().1.pop_n(1)?;
    }
    if let Some(slot) = vsp {
        // A real move: the peephole must not hoist a loaded right-hand side
        // out of the argument register the write send reads below.
        let (session, scope) = cg.current();
        let cur = scope.cursp();
        scope.gen_move(session, slot, cur, true)?;
    }
    cg.current().1.pop_n(1)?;
    let write_sym = {
        let (session, scope) = cg.current();
        scope.new_sym(session, &write_name)?
    };
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_3(session, op_send, dst, write_sym, 1)?;
    }
    cg.current().1.dispatch(pos)?;
    if safe_nav {
        cg.current().1.dispatch(skip)?;
    }
    Ok(())
}

/// Index operator/`||=`/`&&=` write (`a[i] += v`, `a[i] ||= v`,
/// `a[i] &&= v`, `case PM_INDEX_*_WRITE_NODE`): send `[]`, combine or test,
/// send `[]=` (or `OP_SETIDX` for a single index through the call-assign
/// path, which handles plain writes; here both sends stay explicit).
fn gen_index_write<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let kind = node.kind_name();
    let Some(view) = node.index_write() else {
        return Err(unsupported(&node, "index assignment"));
    };
    let IndexWriteView {
        receiver,
        args,
        value,
        binary_operator,
    } = view;
    let Some(recv_node) = receiver else {
        return Err(unsupported(&node, "index assignment"));
    };
    let is_or = kind == "IndexOrWriteNode";
    let op_jmp = if is_or {
        opcode::OP_JMPIF
    } else {
        opcode::OP_JMPNOT
    };
    // A written `self` reads and writes through `OP_SSEND` so a private
    // `[]` stays reachable.
    let op_send = if recv_node.kind_name() == "SelfNode" {
        opcode::OP_SSEND
    } else {
        opcode::OP_SEND
    };
    let mut vsp: Option<u16> = None;
    if val {
        let slot = cg.current().1.cursp();
        cg.current().1.push_n(1)?;
        vsp = Some(slot);
    }
    codegen(cg, recv_node, true)?;
    let aref_sym = {
        let (session, scope) = cg.current();
        scope.new_sym(session, b"[]")?
    };
    let base = cg.current().1.cursp() - 1;
    let items = match args {
        None => Vec::new(),
        Some(args) => args
            .raw_call_args()
            .ok_or_else(|| unsupported(&node, "index assignment"))?,
    };
    let nargs = gen_values(cg, items, true, 13)?;
    let (nargs, mut callargs) = if nargs >= 0 {
        (nargs, nargs)
    } else {
        cg.current().1.push_n(1)?;
        (1, CALL_MAXARGS)
    };
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.gen_move(session, dst, base, true)?;
    }
    for index in 0..nargs {
        let (session, scope) = cg.current();
        let dst = scope.cursp() + index as u16 + 1;
        scope.gen_move(session, dst, (base as i32 + index + 1) as u16, true)?;
    }
    cg.current().1.push_n((nargs + 2) as u16)?;
    cg.current().1.pop_n((nargs + 2) as u16)?;
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_3(session, op_send, dst, aref_sym, callargs as u8)?;
    }
    let mut pos = JMPLINK_START;
    if let Some(operator) = binary_operator {
        cg.current().1.push_n(1)?;
        codegen(cg, value, true)?;
        cg.current().1.push_n(1)?;
        cg.current().1.pop_n(1)?;
        cg.current().1.pop_n(2)?;
        gen_binary_operator(cg, &operator)?;
    } else {
        if let Some(slot) = vsp {
            let (session, scope) = cg.current();
            let cur = scope.cursp();
            scope.gen_move(session, slot, cur, false)?;
        }
        pos = {
            let (session, scope) = cg.current();
            let cur = scope.cursp();
            scope.genjmp2(session, op_jmp, cur, JMPLINK_START, val)?
        };
        codegen(cg, value, true)?;
        cg.current().1.pop_n(1)?;
        cg.current().1.dispatch(pos)?;
    }
    if let Some(slot) = vsp {
        let (session, scope) = cg.current();
        let cur = scope.cursp();
        scope.gen_move(session, slot, cur, false)?;
    }
    if callargs == CALL_MAXARGS {
        cg.current().1.pop_n(1)?;
        {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_2(session, opcode::OP_ARYPUSH, dst, 1)?;
        }
    } else {
        cg.current().1.pop_n(callargs as u16)?;
        callargs += 1;
    }
    cg.current().1.pop_n(1)?;
    let aset_sym = {
        let (session, scope) = cg.current();
        scope.new_sym(session, b"[]=")?
    };
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_3(session, op_send, dst, aset_sym, callargs as u8)?;
    }
    if pos != JMPLINK_START {
        cg.current().1.dispatch(pos)?;
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

/// `MRC_ARGS_REST()` (`codegen.c` aspec layout).
fn args_rest() -> u32 {
    1 << 12
}

/// `MRC_ARGS_BLOCK()` (`codegen.c` aspec layout).
fn args_block() -> u32 {
    1
}

/// `MRC_ARGS_NOBLOCK()` (`codegen.c` aspec layout, `&nil`).
fn args_noblock() -> u32 {
    1 << 23
}

/// Decoded parameter counts (`lambda_body` head): the `OP_ENTER` operand
/// layout plus the block-move register.
struct ParamCounts {
    /// Mandatory positional parameters (multi-targets count as one).
    ma: usize,
    /// Optional positional parameters.
    oa: usize,
    /// Rest parameter present (`*`, `...` forwarding folds in later).
    ra: bool,
    /// Post-rest mandatory parameters.
    pa: usize,
    /// Keyword parameters.
    ka: usize,
    /// Keyword rest present (`**`, `...` forwarding folds in later).
    kd: bool,
    /// Block parameter present (`&`, `...` forwarding folds in later).
    ba: bool,
    /// Named block parameter (`None` for anonymous `&`).
    block_name: Option<Vec<u8>>,
    /// `&nil` (no block accepted, `MRC_ARGS_NOBLOCK`).
    noblock: bool,
    /// `...` forwarding (`ForwardingParameterNode` keyword rest).
    forwarding: bool,
    /// Register moved into the block slot (`0` for none).
    block_reg: u16,
}

/// Anonymous rest marker (`MRC_OPSYM_2(mul)`).
const REST_MARK: &[u8] = b"*";
/// Anonymous keyword-rest marker (`MRC_OPSYM_2(pow)`).
const KEYWORD_REST_MARK: &[u8] = b"**";
/// Anonymous block marker (`MRC_OPSYM_2(and)`).
const BLOCK_MARK: &[u8] = b"&";

/// `OP_ENTER` operand and `ainfo` for decoded counts (`lambda_body` tail of
/// the head: `(23bits = 5:5:1:5:5:1:1)` and `(12bits = 5:1:5:1)`).
fn param_enter(counts: &ParamCounts) -> Result<(u16, u32), Diagnostic> {
    if counts.ma > 0x1f || counts.oa > 0x1f || counts.pa > 0x1f || counts.ka > 0x1f {
        return Err(Diagnostic {
            message: "too many formal arguments".to_owned(),
            start: 0,
            end: 0,
        });
    }
    let ra = counts.ra || counts.forwarding;
    let ba = counts.ba || counts.forwarding;
    let aspec = (if counts.noblock { args_noblock() } else { 0 })
        | args_req(counts.ma)
        | args_opt(counts.oa)
        | (if ra { args_rest() } else { 0 })
        | args_post(counts.pa)
        | args_key(counts.ka, counts.kd)
        | (if ba { args_block() } else { 0 });
    let ainfo = ((((counts.ma + counts.oa) as u16) & 0x3f) << 7)
        | ((u16::from(ra)) << 6)
        | (((counts.pa as u16) & 0x1f) << 1)
        | u16::from(counts.ka > 0 || counts.kd);
    Ok((ainfo, aspec))
}

/// Append a required/post positional slot: the name, or a null placeholder
/// for a destructured (`MultiTargetNode`) slot whose parts land later.
fn push_positional<N: BackendNode>(
    lv: &mut Vec<Vec<u8>>,
    item: &N,
    site: &N,
    what: &str,
) -> Result<(), Diagnostic> {
    if item.kind_name() == "MultiTargetNode" {
        lv.push(Vec::new());
        return Ok(());
    }
    let Some(name) = item.required_param_name() else {
        return Err(unsupported(site, what));
    };
    lv.push(name);
    Ok(())
}

/// Parameter registers (`lambda_body` head for `ParametersNode`): lv layout
/// plus counts. `body_locals` are the scope locals (block `;` locals pass as
/// `extra_locals`); both land after the parameter registers in order.
fn param_layout<N: BackendNode>(
    params: &N,
    body_locals: &[Vec<u8>],
    extra_locals: &[Vec<u8>],
    site: &N,
) -> Result<(Vec<Vec<u8>>, ParamCounts), Diagnostic> {
    let Some(view) = params.parameters_view() else {
        return Err(unsupported(site, "block parameters"));
    };
    let mut counts = ParamCounts {
        ma: view.requireds.len(),
        oa: view.optionals.len(),
        ra: view.rest.is_some(),
        pa: view.posts.len(),
        ka: view.keywords.len(),
        kd: view.keyword_rest.is_some(),
        ba: false,
        block_name: None,
        noblock: false,
        forwarding: false,
        block_reg: 0,
    };
    let mut lv: Vec<Vec<u8>> = Vec::new();
    for item in &view.requireds {
        push_positional(&mut lv, item, site, "method parameters")?;
    }
    for item in &view.optionals {
        let Some((name, _)) = item.optional_param() else {
            return Err(unsupported(site, "optional parameter"));
        };
        lv.push(name);
    }
    if let Some(rest) = &view.rest {
        match rest.kind_name() {
            "RestParameterNode" => {
                let Some(maybe) = rest.rest_param_name() else {
                    return Err(unsupported(site, "rest parameter"));
                };
                lv.push(maybe.unwrap_or_else(|| REST_MARK.to_vec()));
            }
            "ImplicitRestNode" => lv.push(REST_MARK.to_vec()),
            _ => return Err(unsupported(site, "rest parameter")),
        }
    }
    for item in &view.posts {
        push_positional(&mut lv, item, site, "method parameters")?;
    }
    if let Some(block) = &view.block {
        if block.block_param_noblock() {
            counts.noblock = true;
        } else {
            let Some(maybe) = block.block_param_name() else {
                return Err(unsupported(site, "block parameter"));
            };
            counts.ba = true;
            counts.block_name = maybe;
        }
    }
    if counts.ka > 0 || counts.kd || counts.ba {
        let mut write_dastr = false;
        if counts.ka > 0 || counts.kd {
            write_dastr = true;
        }
        if counts.kd {
            let rest = view.keyword_rest.as_ref().expect("keyword rest");
            match rest.kind_name() {
                "KeywordRestParameterNode" => {
                    let Some(maybe) = rest.keyword_rest_name() else {
                        return Err(unsupported(site, "keyword rest parameter"));
                    };
                    lv.push(maybe.unwrap_or_else(|| KEYWORD_REST_MARK.to_vec()));
                    write_dastr = false;
                }
                "ForwardingParameterNode" => {
                    counts.forwarding = true;
                    write_dastr = false;
                    lv.push(REST_MARK.to_vec());
                    lv.push(KEYWORD_REST_MARK.to_vec());
                    lv.push(Vec::new());
                    lv.push(BLOCK_MARK.to_vec());
                    counts.block_reg = u16::try_from(lv.len()).map_err(|_| too_complex())?;
                }
                _ => return Err(unsupported(site, "keyword rest parameter")),
            }
        }
        if write_dastr {
            lv.push(KEYWORD_REST_MARK.to_vec());
        }
    }
    if !counts.forwarding {
        lv.push(Vec::new());
    }
    if counts.ba {
        let name = counts
            .block_name
            .take()
            .unwrap_or_else(|| BLOCK_MARK.to_vec());
        lv.push(name);
        counts.block_reg = u16::try_from(lv.len()).map_err(|_| too_complex())?;
    }
    for item in &view.keywords {
        let Some(part) = item.keyword_param() else {
            return Err(unsupported(site, "keyword parameter"));
        };
        lv.push(part.name);
    }
    append_destructured_parts(&mut lv, &view.requireds, site)?;
    append_destructured_parts(&mut lv, &view.posts, site)?;
    for name in body_locals.iter().chain(extra_locals.iter()) {
        if !lv.contains(name) {
            lv.push(name.clone());
        }
    }
    Ok((lv, counts))
}

/// Names a destructured (`MultiTargetNode`) positional slot expands to
/// (`lambda_body` head tail: requireds then posts).
fn append_destructured_parts<N: BackendNode>(
    lv: &mut Vec<Vec<u8>>,
    items: &[N],
    site: &N,
) -> Result<(), Diagnostic> {
    for item in items {
        if item.kind_name() != "MultiTargetNode" {
            continue;
        }
        let Some(view) = item.multi_target_view() else {
            return Err(unsupported(site, "method parameters"));
        };
        for part in &view.lefts {
            if let Some(name) = part.required_param_name() {
                lv.push(name);
            } else if part.kind_name() == "MultiTargetNode" {
                // A nested target misreads its `lefts.size` as a pool id
                // (unchecked C cast); small sizes land on presymbols.
                let Some(nested) = part.multi_target_view() else {
                    return Err(unsupported(site, "method parameters"));
                };
                let Some(bytes) = presym_bytes(nested.lefts.len()) else {
                    return Err(unsupported(site, "method parameters"));
                };
                lv.push(bytes.to_vec());
            } else {
                return Err(unsupported(site, "method parameters"));
            }
        }
    }
    Ok(())
}

/// Pinned presymbol bytes by pool id (`mrc_presym.inc` of
/// `mruby-compiler2 0.5.0`, inserted first into a fresh pool).
fn presym_bytes(id: usize) -> Option<&'static [u8]> {
    Some(match id {
        1 => b"[]",
        2 => b"<<",
        3 => b">>",
        4 => b"%",
        5 => b"&",
        6 => b"|",
        7 => b"^",
        8 => b"~",
        9 => b"**",
        10 => b"+",
        11 => b"-",
        12 => b"*",
        13 => b"/",
        14 => b"<",
        15 => b"<=",
        16 => b">",
        17 => b">=",
        18 => b"==",
        19 => b"[]=",
        20 => b"===",
        21 => b"`",
        22 => b"each",
        23 => b"__case_eqq",
        24 => b"StandardError",
        25 => b"call",
        26 => b"Kernel",
        27 => b"Regexp",
        28 => b"compile",
        29 => b"__ENCODING__",
        30 => b"nil?",
        31 => b"$+",
        32 => b"defined?",
        33 => b"deconstruct",
        34 => b"deconstruct_keys",
        35 => b"size",
        36 => b"has_key?",
        37 => b"__pat_values",
        38 => b"__except",
        39 => b"dup",
        40 => b"__defined_const?",
        41 => b"__defined_method?",
        42 => b"__defined_ivar?",
        43 => b"__defined_yield?",
        44 => b"__defined_gvar?",
        45 => b"__defined_cvar?",
        46 => b"__defined_super?",
        47 => b"__defined_const_path?",
        48 => b"__defined_method_on?",
        49 => b"$~",
        50 => b"__pre_match",
        51 => b"__post_match",
        52 => b"__last_group",
        53 => b"__group",
        54 => b"$!",
        55 => b"freeze",
        56 => b"respond_to?",
        57 => b"Exception",
        _ => return None,
    })
}

/// Optional-default jump table (`lambda_body`: `pos` chain over the
/// `OP_ENTER` that was just emitted).
fn emit_optional_setup<N: BackendNode>(
    cg: &mut Codegen,
    optionals: &[N],
) -> Result<(), Diagnostic> {
    if optionals.is_empty() {
        return Ok(());
    }
    let mut jumps = Vec::with_capacity(optionals.len() + 1);
    {
        let (_, scope) = cg.current();
        scope.new_label();
    }
    for _ in optionals {
        let (_, scope) = cg.current();
        scope.new_label();
        jumps.push(scope.genjmp(opcode::OP_JMP, JMPLINK_START)?);
    }
    {
        let (_, scope) = cg.current();
        jumps.push(scope.genjmp(opcode::OP_JMP, JMPLINK_START)?);
    }
    for (index, item) in optionals.iter().enumerate() {
        let Some((name, default)) = item.optional_param() else {
            return Err(unsupported(item, "optional parameter"));
        };
        cg.current().1.dispatch(jumps[index])?;
        codegen(cg, default, true)?;
        cg.current().1.pop_n(1)?;
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        let slot = scope.lv_idx(&name);
        if slot == 0 {
            return Err(internal_error("optional parameter must be local variable"));
        }
        scope.gen_move(session, slot, dst, false)?;
    }
    cg.current()
        .1
        .dispatch(*jumps.last().expect("trailing jump"))?;
    Ok(())
}

/// Keyword-argument setup (`lambda_body`: `OP_KEY_P` defaults, `OP_KARG`
/// reads, `OP_KEYEND` when no rest gathers the remainder).
fn emit_keyword_setup<N: BackendNode>(
    cg: &mut Codegen,
    keywords: &[N],
    with_rest: bool,
) -> Result<(), Diagnostic> {
    if keywords.is_empty() {
        return Ok(());
    }
    for item in keywords {
        let Some(part) = item.keyword_param() else {
            return Err(unsupported(item, "keyword parameter"));
        };
        let mut default_jump = None;
        if let Some(default) = part.default {
            let sym = {
                let (session, scope) = cg.current();
                scope.new_sym(session, &part.name)?
            };
            let slot = cg.current().1.lv_idx(&part.name);
            if slot == 0 {
                return Err(internal_error("keyword parameter must be local variable"));
            }
            {
                let (session, scope) = cg.current();
                scope.genop_2(session, opcode::OP_KEY_P, slot, sym)?;
            }
            let pos = {
                let (session, scope) = cg.current();
                scope.genjmp2(session, opcode::OP_JMPIF, slot, JMPLINK_START, false)?
            };
            codegen(cg, default, true)?;
            cg.current().1.pop_n(1)?;
            {
                let (session, scope) = cg.current();
                let dst = scope.cursp();
                scope.gen_move(session, slot, dst, false)?;
            }
            {
                let (_, scope) = cg.current();
                default_jump = Some(scope.genjmp(opcode::OP_JMP, JMPLINK_START)?);
            }
            {
                let (_, scope) = cg.current();
                scope.dispatch(pos)?;
            }
        }
        let sym = {
            let (session, scope) = cg.current();
            scope.new_sym(session, &part.name)?
        };
        let slot = cg.current().1.lv_idx(&part.name);
        if slot == 0 {
            return Err(internal_error("keyword parameter must be local variable"));
        }
        {
            let (session, scope) = cg.current();
            scope.genop_2(session, opcode::OP_KARG, slot, sym)?;
        }
        if let Some(pos) = default_jump {
            cg.current().1.dispatch(pos)?;
        }
    }
    if !with_rest {
        let (_, scope) = cg.current();
        scope.genop_0(opcode::OP_KEYEND)?;
    }
    Ok(())
}

/// Destructured positional slots (`lambda_body`: `gen_massignment` over the
/// parameter register plus the `APOST` reacquire).
fn emit_destructure_setup<N: BackendNode>(
    cg: &mut Codegen,
    items: &[N],
    mut pos: u16,
) -> Result<(), Diagnostic> {
    for item in items {
        if item.kind_name() == "MultiTargetNode" {
            let Some(view) = item.multi_target_view() else {
                return Err(unsupported(item, "method parameters"));
            };
            let count = u16::try_from(view.lefts.len()).map_err(|_| too_complex())?;
            gen_massignment(
                cg,
                view.lefts,
                view.rest,
                view.rights,
                i32::from(pos),
                false,
            )?;
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.gen_move(session, dst, pos, false)?;
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_3(session, opcode::OP_APOST, dst, count, 0)?;
        }
        pos += 1;
    }
    Ok(())
}

/// Full argument setup after `OP_ENTER` (`lambda_body` body head): optional
/// defaults, keyword reads, the block move and destructured slots.
fn emit_param_setup<N: BackendNode>(
    cg: &mut Codegen,
    params: &N,
    counts: &ParamCounts,
) -> Result<(), Diagnostic> {
    let Some(view) = params.parameters_view() else {
        return Err(unsupported(params, "method parameters"));
    };
    emit_optional_setup(cg, &view.optionals)?;
    emit_keyword_setup(cg, &view.keywords, counts.kd)?;
    if counts.block_reg != 0 {
        let (session, scope) = cg.current();
        scope.gen_move(session, counts.block_reg, counts.block_reg - 1, false)?;
    }
    emit_destructure_setup(cg, &view.requireds, 1)?;
    let rest_slots = usize::from(counts.ra || counts.forwarding);
    let post_base = (counts.ma + counts.oa + rest_slots) as u16 + 1;
    emit_destructure_setup(cg, &view.posts, post_base)?;
    Ok(())
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
/// Covers empty, numbered (`_1`) and `it` forms plus the full
/// optional/rest/post/keyword/block/destructured layout.
fn gen_lambda_body<N: BackendNode>(
    cg: &mut Codegen,
    locals: Vec<Vec<u8>>,
    params: Option<N>,
    body: Option<N>,
    op: u8,
    site: &N,
) -> Result<(), Diagnostic> {
    let mut lv: Vec<Vec<u8>> = Vec::new();
    let mut setup: Option<(N, ParamCounts)> = None;
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
                match view.params {
                    None => {
                        // `||` or `|;local|`: no positional layout.
                        lv.push(Vec::new());
                        for name in locals.iter().chain(semi.iter()) {
                            if !lv.contains(name) {
                                lv.push(name.clone());
                            }
                        }
                        (0, args_req(0))
                    }
                    Some(inner) => {
                        let (layout, counts) = param_layout(&inner, &locals, &semi, site)?;
                        let (ainfo, aspec) = param_enter(&counts)?;
                        lv = layout;
                        setup = Some((inner, counts));
                        (ainfo, aspec)
                    }
                }
            }
            _ => return Err(unsupported(site, "block parameters")),
        },
    };
    enter_block_scope(
        cg,
        lv,
        ainfo,
        aspec,
        body,
        op,
        setup.as_ref().map(|(p, c)| (p, c)),
    )
}

/// Pushes the child scope, emits `OP_ENTER`, codes the body and emits
/// `OP_BLOCK`/`OP_LAMBDA` in the parent (`lambda_body` tail with `blk`).
/// `setup` carries the argument setup (defaults, keywords, block move,
/// destructuring) emitted between `OP_ENTER` and the body.
fn enter_block_scope<N: BackendNode>(
    cg: &mut Codegen,
    lv: Vec<Vec<u8>>,
    ainfo: u16,
    aspec: u32,
    body: Option<N>,
    op: u8,
    setup: Option<(&N, &ParamCounts)>,
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
    if let Some((params, counts)) = setup {
        emit_param_setup(cg, params, counts)?;
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

/// Split trailing keyword hashes off gathered call items (`gen_call` and
/// `gen_yield` share the `KeywordHashNode` tail; `gen_values` stops at it).
fn split_keywords<N: BackendNode>(mut items: Vec<N>) -> (Vec<N>, Vec<N>) {
    match items
        .iter()
        .position(|item| item.kind_name() == "KeywordHashNode")
    {
        Some(first) => {
            let keywords = items.split_off(first);
            (items, keywords)
        }
        None => (items, Vec::new()),
    }
}

/// `yield` (`PM_YIELD_NODE`): `BLKPUSH` plus a direct `BLKCALL` for plain
/// positional arguments, falling back to `:call` dispatch when keyword
/// arguments or an array-gathered list are present.
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
    let mut n: i32 = 0;
    let mut nk: i32 = 0;
    let mut st: i32 = 0;
    if let Some(args) = view.args {
        let Some(items) = args.call_args() else {
            return Err(unsupported(&node, "complex arguments"));
        };
        let (items, keywords) = split_keywords(items);
        if !items.is_empty() {
            n = gen_values(cg, items, true, CALL_ARG_LIMIT)?;
            if n < 0 {
                st = 1;
                n = CALL_MAXARGS;
                cg.current().1.push_n(1)?;
            } else {
                st = n;
            }
        }
        for keyword in keywords {
            let Some(elements) = keyword.hash_elements() else {
                return Err(unsupported(&keyword, "keyword arguments"));
            };
            nk = gen_hash(cg, elements, true, CALL_ARG_LIMIT)?;
            if nk < 0 {
                st += 1;
                nk = CALL_MAXARGS;
            } else {
                st += nk * 2;
            }
            n |= nk << 4;
        }
    }
    {
        let (_, scope) = cg.current();
        scope.push_n(1)?;
        scope.pop_n(1)?;
        scope.pop_n(u16::try_from(st + 1).map_err(|_| too_complex())?)?;
    }
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_2s(session, opcode::OP_BLKPUSH, dst, operand)?;
    }
    if nk == 0 && n < CALL_MAXARGS {
        // Fast path: direct block call without method dispatch.
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_BLKCALL, dst, n as u16)?;
    } else {
        // `SEND` carries the keyword count / splat array to `Proc#call`.
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        let sym = scope.new_sym(session, b"call")?;
        scope.genop_3(session, opcode::OP_SEND, dst, sym, n as u8)?;
    }
    if val {
        cg.current().1.push_n(1)?;
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
            let Some(view) = value.begin_view() else {
                return value;
            };
            if !view.bare {
                return value;
            }
            let Some(mut stmts) = view.statements else {
                return value;
            };
            if stmts.len() != 1 {
                return value;
            }
            value = stmts.remove(0);
            continue;
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
            let empty = view.bare && view.statements.as_ref().map(Vec::is_empty).unwrap_or(true);
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
            | "CaseMatchNode"
            | "MatchPredicateNode"
            | "MatchRequiredNode"
            | "InterpolatedStringNode"
            | "InterpolatedSymbolNode"
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
            | "ConstantReadNode"
            | "ConstantPathNode"
            | "InstanceVariableReadNode"
            | "GlobalVariableReadNode"
            | "ClassVariableReadNode"
            | "BeginNode"
            | "BlockNode"
            | "LambdaNode"
            | "YieldNode"
            | "MultiWriteNode"
            | "BackReferenceReadNode"
            | "NumberedReferenceReadNode"
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
        } else if a.unless_nil.is_some() {
            // The check and the receiver value both evaluate the operand
            // (`gen_defined_part` plus `codegen`, like C).
            gen_defined_part(cg, value.clone(), nil_jmps)?;
        } else {
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
            // `...`: the flushed array gathers `*`, a fresh hash gathers
            // `**`, and `&` rides along (`gen_values_upto` forwarding arm).
            gen_forward_arg(cg, b"*", val)?;
            cg.current().1.pop_n(1)?;
            {
                let (session, scope) = cg.current();
                let dst = scope.cursp();
                scope.genop_1(session, opcode::OP_ARYCAT, dst)?;
            }
            cg.current().1.push_n(1)?;
            {
                let (session, scope) = cg.current();
                let dst = scope.cursp();
                scope.genop_2(session, opcode::OP_HASH, dst, 0)?;
            }
            cg.current().1.push_n(1)?;
            gen_forward_arg(cg, b"**", val)?;
            cg.current().1.pop_n(1)?;
            {
                let (session, scope) = cg.current();
                let dst = scope.cursp();
                scope.genop_1(session, opcode::OP_HASHCAT, dst)?;
            }
            cg.current().1.push_n(1)?;
            gen_forward_arg(cg, b"&", val)?;
            break;
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

/// Anonymous forwarding variable load (`gen_forward_arg`): a method-scope
/// local when present, otherwise an upvar (forwarding from inside a block).
fn gen_forward_arg(cg: &mut Codegen, name: &[u8], val: bool) -> Result<(), Diagnostic> {
    let slot = cg.current().1.lv_idx(name);
    if slot > 0 {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.gen_move(session, dst, slot, val)?;
    } else {
        let (slot, level) = cg.search_upvar(name)?;
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.gen_getupvar(session, dst, slot, level)?;
    }
    Ok(())
}

/// Local variable load into the cursor (`gen_lvar`): a local move, or an
/// upvar load past enclosing scopes (operator writes inside blocks).
fn gen_lvar(cg: &mut Codegen, name: &[u8], depth: u32) -> Result<(), Diagnostic> {
    if depth == 0 {
        let index = cg.current().1.lv_idx(name);
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.gen_move(session, dst, index, true)?;
    } else {
        let (slot, level) = cg.search_upvar(name)?;
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.gen_getupvar(session, dst, slot, level)?;
    }
    cg.current().1.push_n(1)
}

/// Binary operator on the two values at the cursor (`gen_binary_operator`):
/// `+`/`-` fold onto integer loads, `*`/`/` have direct opcodes, and anything
/// else sends the operator as a one-argument call.
fn gen_binary_operator(cg: &mut Codegen, name: &[u8]) -> Result<(), Diagnostic> {
    let dst = cg.current().1.cursp();
    if name == b"+" {
        gen_addsub(cg, opcode::OP_ADD, dst)
    } else if name == b"-" {
        gen_addsub(cg, opcode::OP_SUB, dst)
    } else if name == b"*" {
        let (session, scope) = cg.current();
        scope.genop_1(session, opcode::OP_MUL, dst)
    } else if name == b"/" {
        let (session, scope) = cg.current();
        scope.genop_1(session, opcode::OP_DIV, dst)
    } else {
        let sym = {
            let (session, scope) = cg.current();
            scope.new_sym(session, name)?
        };
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_3(session, opcode::OP_SEND, dst, sym, 1)
    }
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

/// Method call (`gen_call`).
fn gen_call<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    gen_call_impl(cg, node, val, false)
}

/// Whether a call's arguments are simple enough for `gen_call_assign` (no
/// keyword hash or forwarding that would obscure the right-hand side, which
/// rides as the last positional argument).
fn attr_assign_simple_args<N: BackendNode>(view: &CallView<N>) -> bool {
    let Some(args) = &view.args else {
        return false;
    };
    let Some(items) = args.call_args() else {
        return false;
    };
    if items.is_empty() {
        return false;
    }
    items.iter().all(|item| {
        let kind = item.kind_name();
        kind != "KeywordHashNode" && kind != "ForwardingArgumentsNode"
    })
}

/// Attribute assignment (`recv.attr = v`, `recv[i] = v`) as an expression
/// (`gen_call_assign`): the right-hand side is the last positional argument,
/// copied into a reserved slot below the call frame so the expression keeps
/// its value while the send result is discarded. With `recv_ready` the
/// receiver already sits at `cursp()-1`.
fn gen_call_assign<N: BackendNode>(
    cg: &mut Codegen,
    node: &N,
    view: CallView<N>,
    val: bool,
    safe: bool,
    recv_ready: bool,
) -> Result<(), Diagnostic> {
    let name = view.name;
    let noop = cg.session.no_optimize;
    let opt_setidx = !noop && name == b"[]=";
    let (top, callsp, noself) = if recv_ready {
        // The receiver's slot becomes the room for the result, and the
        // receiver moves up above it.
        let top = cg.current().1.cursp() - 1;
        {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.gen_move(session, dst, top, true)?;
        }
        cg.current().1.push_n(1)?;
        let callsp = cg.current().1.cursp() - 1;
        (top, callsp, false)
    } else {
        let top = cg.current().1.cursp();
        cg.current().1.push_n(1)?;
        let callsp = cg.current().1.cursp();
        // A written `self` is a call on self, so a private setter stays
        // reachable; the register is still loaded where an instruction reads
        // it before the send (that is, for `OP_SETIDX` and the `&.` check).
        let noself = match &view.receiver {
            None => {
                cg.current().1.push_n(1)?;
                true
            }
            Some(recv) if recv.kind_name() == "SelfNode" => {
                if opt_setidx || safe {
                    codegen(cg, recv.clone(), true)?;
                } else {
                    cg.current().1.push_n(1)?;
                }
                true
            }
            Some(recv) => {
                codegen(cg, recv.clone(), true)?;
                false
            }
        };
        (top, callsp, noself)
    };
    let mut skip = JMPLINK_START;
    if safe {
        let recv = cg.current().1.cursp() - 1;
        {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.gen_move(session, dst, recv, true)?;
        }
        skip = {
            let (session, scope) = cg.current();
            let cur = scope.cursp();
            scope.genjmp2(session, opcode::OP_JMPNIL, cur, JMPLINK_START, val)?
        };
    }
    // The indices, then the right-hand side apart from them: a splat among
    // the indices gathers them into an array, and the right-hand side has to
    // be held back from it until it has been copied to the result slot.
    let mut n: i32 = 0;
    let mut gathered = false;
    if let Some(args_node) = view.args {
        let Some(items) = args_node.call_args() else {
            return Err(unsupported(node, "complex arguments"));
        };
        if !items.is_empty() {
            let last = items.len() - 1;
            let count = gen_values(cg, items[..last].to_vec(), true, 13)?;
            if count < 0 {
                gathered = true;
                cg.current().1.push_n(1)?;
            }
            codegen(cg, items[last].clone(), true)?;
            if !gathered {
                n = count + 1;
            }
        }
    }
    if val {
        // Keep the right-hand side in its argument slot for the send, while
        // also copying it to the reserved result slot.
        let (session, scope) = cg.current();
        let src = scope.cursp() - 1;
        scope.gen_move(session, top, src, true)?;
    }
    if gathered {
        // The right-hand side joins the indices in their array, which is the
        // one argument.
        cg.current().1.pop_n(2)?;
        {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_2(session, opcode::OP_ARYPUSH, dst, 1)?;
        }
        cg.current().1.push_n(1)?;
        n = CALL_MAXARGS;
    }
    cg.current().1.push_n(1)?;
    cg.current().1.pop_n(1)?;
    cg.current().1.sp = callsp;
    if opt_setidx && n == 2 {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_1(session, opcode::OP_SETIDX, dst)?;
    } else {
        let sym = {
            let (session, scope) = cg.current();
            scope.new_sym(session, &name)?
        };
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_3(
            session,
            if noself {
                opcode::OP_SSEND
            } else {
                opcode::OP_SEND
            },
            dst,
            sym,
            n as u8,
        )?;
    }
    if safe {
        cg.current().1.dispatch(skip)?;
    }
    cg.current().1.sp = top;
    if val {
        cg.current().1.push_n(1)?;
    }
    Ok(())
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
    // Attribute assignment (`recv.attr = v`, `recv[i] = v`) evaluates to the
    // right-hand side, not to the setter's return value, so valued writes
    // with simple arguments take the `gen_call_assign` path. Anything else
    // (a discarded value, a splat-gathered tail aside, keywords, or
    // forwarding) falls through to the normal call, like `gen_call`.
    if view.attr_write && val && attr_assign_simple_args(&view) {
        let safe = view.safe_nav;
        return gen_call_assign(cg, &node, view, val, safe, recv_ready);
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
    let mut nk: i32 = 0;
    let mut forwarding = false;
    if let Some(args) = view.args {
        // `...` rides `gen_values` like a splat, then forces the block
        // call shape with a full `0xFF` operand (`gen_call` tail).
        forwarding = args.args_forwarding();
        let Some(items) = args.call_args() else {
            return Err(unsupported(&node, "complex arguments"));
        };
        let (items, keywords) = split_keywords(items);
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
    let mut blk = false;
    if let Some(block) = view.block {
        // Literal blocks and `&` block args share the `SENDB` path
        // (`gen_call` codes the block `VAL`, then pops it).
        codegen(cg, block, true)?;
        cg.current().1.pop_n(1)?;
        noop = true;
        blk = true;
    }
    if forwarding {
        blk = true;
        nargs = FORWARD_ARGS;
    }
    {
        let (_, scope) = cg.current();
        scope.push_n(1)?;
        scope.pop_n(1)?;
        scope.sp = sp_save;
    }
    emit_call(cg, noself, noop, blk, &name, nargs, nk, val, safe, skip)
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
            send_call(cg, noself, blk, name, nargs, nk, dst)?;
        }
    } else if !noop && name == b"[]=" && nargs == 2 {
        // A discarded index write still stores through `OP_SETIDX`.
        let (session, scope) = cg.current();
        scope.genop_1(session, opcode::OP_SETIDX, dst)?;
    } else if !noop && nargs == 0 && gen_uniop(cg, name, dst)? {
        // A literal absorbed its sign.
    } else {
        send_call(cg, noself, blk, name, nargs, nk, dst)?;
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
    blk: bool,
    name: &[u8],
    nargs: i32,
    nk: i32,
    dst: u16,
) -> Result<(), Diagnostic> {
    let sym = {
        let (session, scope) = cg.current();
        scope.new_sym(session, name)?
    };
    let packed = if nargs == FORWARD_ARGS {
        FORWARD_ARGS as u8
    } else {
        ((nargs as u8) & 0x0f) | (((nk as u8) & 0x0f) << 4)
    };
    if noself {
        let (session, scope) = cg.current();
        if !blk && nargs == 0 && nk == 0 {
            scope.genop_2(session, opcode::OP_SSEND0, dst, sym)?;
        } else {
            let op = if blk {
                opcode::OP_SSENDB
            } else {
                opcode::OP_SSEND
            };
            scope.genop_3(session, op, dst, sym, packed)?;
        }
    } else {
        let (session, scope) = cg.current();
        if !blk && nargs == 0 && nk == 0 {
            scope.genop_2(session, opcode::OP_SEND0, dst, sym)?;
        } else {
            let op = if blk {
                opcode::OP_SENDB
            } else {
                opcode::OP_SEND
            };
            scope.genop_3(session, op, dst, sym, packed)?;
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

/// `for` loop (`for_body`): the collection evaluates in the current scope,
/// then an invisible child scope (`lv == NULL`) takes the block parameter
/// in register 1, assigns the loop variable, and codes the body; back in
/// the parent the child becomes a block sent to `each`.
fn gen_for<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(view) = node.for_view() else {
        return Err(unsupported(&node, "for"));
    };
    // Receiver in the current scope (`VAL`).
    codegen(cg, view.collection, true)?;
    // Invisible child scope over the parent register layout.
    let child = Scope::for_child(&mut cg.session, cg.scopes.last().expect("open scope"))?;
    cg.scopes.push(child);
    {
        // `push()` for the block parameter.
        let (_, scope) = cg.current();
        scope.push_n(1)?;
    }
    {
        // Block-like entry, not the normal `aspec` computation.
        let (_, scope) = cg.current();
        scope.genop_w(opcode::OP_ENTER, 0x40000)?;
    }
    // Loop variable from register 1 (`VAL` for multi, `NOVAL` otherwise).
    if view.index.kind_name() == "MultiTargetNode" {
        let Some(target) = view.index.multi_target_view() else {
            return Err(unsupported(&node, "for index"));
        };
        gen_massignment(cg, target.lefts, target.rest, target.rights, 1, true)?;
    } else {
        gen_assignment(cg, view.index, None, 1, false)?;
    }
    {
        // Loop frame (`LOOP_FOR`, so `break`/`next` later take the
        // proc-style path like `BLOCK`); `redo` lands on the `NOP`.
        let (_, scope) = cg.current();
        scope.loop_push(LoopType::For);
        let redo = scope.new_label();
        scope.loops.last_mut().expect("for loop").pc1 = redo;
        scope.genop_0(opcode::OP_NOP)?;
    }
    // Body (`VAL`), `RETURN`, close frame, finish child.
    gen_branch(cg, view.statements, true)?;
    cg.current().1.pop_n(1)?;
    {
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
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_BLOCK, dst, child_index)?;
        // `push();pop();` leaves space for the block, then `pop()` drops
        // the collection the `SENDB` below consumes.
        scope.push_n(1)?;
        scope.pop_n(1)?;
        scope.pop_n(1)?;
    }
    let sym = {
        let (session, scope) = cg.current();
        scope.new_sym(session, b"each")?
    };
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_3(session, opcode::OP_SENDB, dst, sym, 0)?;
    }
    if val {
        cg.current().1.push_n(1)?;
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

/// Pattern nesting bound (`codegen_pattern`, `"too complex pattern"`).
fn too_complex_pattern() -> Diagnostic {
    Diagnostic {
        message: "too complex pattern".to_owned(),
        start: 0,
        end: 0,
    }
}

/// Chain a pattern failure jump, keeping the old chain when the peephole
/// emits nothing (`gen_pattern_fail_jmp`).
fn gen_pattern_fail_jmp(
    cg: &mut Codegen,
    op: u8,
    a: u16,
    fail_pos: &mut u32,
    val: bool,
) -> Result<(), Diagnostic> {
    let (session, scope) = cg.current();
    let tmp = scope.genjmp2(session, op, a, *fail_pos, val)?;
    if tmp != JMPLINK_START {
        *fail_pos = tmp;
    }
    Ok(())
}

/// Fail unless `value` answers `===` for the value at `target`
/// (`gen_pattern_eqq`).
fn gen_pattern_eqq<N: BackendNode>(
    cg: &mut Codegen,
    value: N,
    target: u16,
    fail_pos: &mut u32,
) -> Result<(), Diagnostic> {
    codegen(cg, value, true)?;
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.gen_move(session, dst, target, false)?;
        scope.push_n(2)?;
        scope.pop_n(3)?;
        let sym = scope.new_sym(session, b"===")?;
        let dst = scope.cursp();
        scope.genop_3(session, opcode::OP_SEND, dst, sym, 1)?;
    }
    let cur = cg.current().1.cursp();
    let (session, scope) = cg.current();
    let pos = scope.genjmp2(session, opcode::OP_JMPNOT, cur, *fail_pos, true)?;
    *fail_pos = pos;
    Ok(())
}

/// Fail unless the value at `target` answers `mid`
/// (`gen_pattern_respond_to`). `cache` is the `case/in` register keeping
/// the answer across clauses (`None` for patterns with none).
fn gen_pattern_respond_to(
    cg: &mut Codegen,
    target: u16,
    mid: &[u8],
    fail_pos: &mut u32,
    cache: Option<u16>,
) -> Result<(), Diagnostic> {
    let reg = cg.current().1.cursp();
    {
        let (session, scope) = cg.current();
        scope.gen_move(session, reg, target, false)?;
        scope.push_n(1)?;
        let dst = scope.cursp();
        let sym = scope.new_sym(session, mid)?;
        scope.genop_2(session, opcode::OP_LOADSYM, dst, sym)?;
        scope.push_n(1)?;
        scope.push_n(1)?;
        scope.pop_n(1)?;
        scope.sp = reg;
        let sym = scope.new_sym(session, b"respond_to?")?;
        scope.genop_3(session, opcode::OP_SEND, reg, sym, 1)?;
    }
    if let Some(slot) = cache {
        let (session, scope) = cg.current();
        scope.gen_move(session, slot, reg, true)?;
    }
    let (session, scope) = cg.current();
    let pos = scope.genjmp2(session, opcode::OP_JMPNOT, reg, *fail_pos, true)?;
    *fail_pos = pos;
    Ok(())
}

/// Send `deconstruct` to `target`, leaving the array at `cursp()`
/// (`gen_pattern_deconstruct`).
fn gen_pattern_deconstruct(
    cg: &mut Codegen,
    target: u16,
    fail_pos: &mut u32,
    cache: Option<u16>,
) -> Result<(), Diagnostic> {
    let reg = cg.current().1.cursp();
    let mut have = JMPLINK_START;
    if let Some(slot) = cache {
        let (session, scope) = cg.current();
        let ask = scope.genjmp2(session, opcode::OP_JMPNIL, slot, JMPLINK_START, true)?;
        let (session, scope) = cg.current();
        let pos = scope.genjmp2(session, opcode::OP_JMPNOT, slot, *fail_pos, true)?;
        *fail_pos = pos;
        {
            let (session, scope) = cg.current();
            scope.gen_move(session, reg, slot, true)?;
        }
        have = cg.current().1.genjmp(opcode::OP_JMP, JMPLINK_START)?;
        cg.current().1.dispatch(ask)?;
    }
    gen_pattern_respond_to(cg, target, b"deconstruct", fail_pos, cache)?;
    {
        let (session, scope) = cg.current();
        scope.gen_move(session, reg, target, false)?;
        scope.push_n(2)?;
        scope.pop_n(2)?;
        let sym = scope.new_sym(session, b"deconstruct")?;
        scope.genop_3(session, opcode::OP_SEND, reg, sym, 0)?;
    }
    if let Some(slot) = cache {
        {
            let (session, scope) = cg.current();
            scope.gen_move(session, slot, reg, true)?;
        }
        cg.current().1.dispatch(have)?;
    }
    Ok(())
}

/// Whether the pattern at the top of an `in` clause would send
/// `deconstruct` to the subject (`pattern_deconstructs`).
fn pattern_deconstructs<N: BackendNode>(pattern: &N) -> bool {
    let mut current = pattern.clone();
    loop {
        match current.kind_name() {
            "IfNode" | "UnlessNode" => {
                let Some(guard) = current.guard_view() else {
                    return false;
                };
                current = guard.inner;
            }
            "CapturePatternNode" => {
                let Some(view) = current.capture_view() else {
                    return false;
                };
                current = view.value;
            }
            "AlternationPatternNode" => {
                let Some(view) = current.alternation_view() else {
                    return false;
                };
                if pattern_deconstructs(&view.left) {
                    return true;
                }
                current = view.right;
            }
            "ArrayPatternNode" | "FindPatternNode" => return true,
            _ => return false,
        }
    }
}

/// Bind a captured value to its target local (`gen_pattern_bind`).
fn gen_pattern_bind<N: BackendNode>(cg: &mut Codegen, var: N, src: u16) -> Result<(), Diagnostic> {
    let Some(target) = var.lvar_target() else {
        return Err(unsupported(&var, "pattern binding"));
    };
    let depth = target.depth + u32::from(cg.current().1.for_depth);
    gen_assignment_lvar(cg, src, &target.name, depth, true)
}

/// Pattern walk with its own nesting count (`codegen_pattern`).
fn codegen_pattern<N: BackendNode>(
    cg: &mut Codegen,
    pattern: N,
    target: u16,
    fail_pos: &mut u32,
    known_array_len: i32,
    cache: Option<u16>,
) -> Result<(), Diagnostic> {
    let rlev = cg.current().1.rlev;
    cg.current().1.rlev += 1;
    if cg.current().1.rlev > CODEGEN_LEVEL_MAX {
        cg.current().1.rlev = rlev;
        return Err(too_complex_pattern());
    }
    let result = codegen_pattern_1(cg, pattern, target, fail_pos, known_array_len, cache);
    cg.current().1.rlev = rlev;
    result
}

/// One pattern (`codegen_pattern_1`): value tests, bindings, guards,
/// alternation, capture, pins, array/hash/find shapes.
fn codegen_pattern_1<N: BackendNode>(
    cg: &mut Codegen,
    pattern: N,
    target: u16,
    fail_pos: &mut u32,
    known_array_len: i32,
    cache: Option<u16>,
) -> Result<(), Diagnostic> {
    cg.current().1.new_label();
    if pattern.kind_name() == "IfNode" || pattern.kind_name() == "UnlessNode" {
        return gen_pattern_guard(cg, pattern, target, fail_pos, known_array_len, cache);
    }
    match pattern.kind_name() {
        "IntegerNode"
        | "FloatNode"
        | "RationalNode"
        | "ImaginaryNode"
        | "StringNode"
        | "InterpolatedStringNode"
        | "XStringNode"
        | "SymbolNode"
        | "InterpolatedSymbolNode"
        | "RegularExpressionNode"
        | "InterpolatedRegularExpressionNode"
        | "RangeNode"
        | "TrueNode"
        | "FalseNode"
        | "NilNode"
        | "ConstantReadNode"
        | "ConstantPathNode" => {
            codegen(cg, pattern, true)?;
            {
                let (session, scope) = cg.current();
                let dst = scope.cursp();
                scope.gen_move(session, dst, target, false)?;
                scope.push_n(2)?;
                scope.pop_n(3)?;
                let sym = scope.new_sym(session, b"===")?;
                let dst = scope.cursp();
                scope.genop_3(session, opcode::OP_SEND, dst, sym, 1)?;
            }
            let cur = cg.current().1.cursp();
            let (session, scope) = cg.current();
            let pos = scope.genjmp2(session, opcode::OP_JMPNOT, cur, *fail_pos, true)?;
            *fail_pos = pos;
            Ok(())
        }
        "LocalVariableTargetNode" => gen_pattern_bind(cg, pattern, target),
        "ImplicitNode" => {
            let Some(inner) = pattern.implicit_value() else {
                return Err(unsupported(&pattern, "implicit pattern"));
            };
            codegen_pattern(cg, inner, target, fail_pos, known_array_len, None)
        }
        "AlternationPatternNode" => {
            gen_pattern_alternation(cg, pattern, target, fail_pos, known_array_len, cache)
        }
        "CapturePatternNode" => {
            let Some(view) = pattern.capture_view() else {
                return Err(unsupported(&pattern, "capture pattern"));
            };
            codegen_pattern(cg, view.value, target, fail_pos, known_array_len, cache)?;
            gen_pattern_bind(cg, view.target, target)
        }
        "PinnedVariableNode" => {
            let Some(inner) = pattern.pinned_var() else {
                return Err(unsupported(&pattern, "pinned variable"));
            };
            gen_pattern_eqq(cg, inner, target, fail_pos)
        }
        "PinnedExpressionNode" => {
            let Some(inner) = pattern.pinned_expr() else {
                return Err(unsupported(&pattern, "pinned expression"));
            };
            gen_pattern_eqq(cg, inner, target, fail_pos)
        }
        "ArrayPatternNode" => {
            gen_array_pattern(cg, pattern, target, fail_pos, known_array_len, cache)
        }
        "HashPatternNode" => gen_hash_pattern(cg, pattern, target, fail_pos),
        "FindPatternNode" => gen_find_pattern(cg, pattern, target, fail_pos, cache),
        _ => {
            let pos = cg.current().1.genjmp(opcode::OP_JMP, *fail_pos)?;
            *fail_pos = pos;
            Ok(())
        }
    }
}

/// Guard wrapper (`if`/`unless` around the inner pattern).
fn gen_pattern_guard<N: BackendNode>(
    cg: &mut Codegen,
    pattern: N,
    target: u16,
    fail_pos: &mut u32,
    known_array_len: i32,
    cache: Option<u16>,
) -> Result<(), Diagnostic> {
    let Some(guard) = pattern.guard_view() else {
        return Err(unsupported(&pattern, "pattern guard"));
    };
    codegen_pattern(cg, guard.inner, target, fail_pos, known_array_len, cache)?;
    codegen(cg, guard.condition, true)?;
    cg.current().1.pop_n(1)?;
    let cur = cg.current().1.cursp();
    let op = if guard.is_unless {
        opcode::OP_JMPIF
    } else {
        opcode::OP_JMPNOT
    };
    gen_pattern_fail_jmp(cg, op, cur, fail_pos, false)
}

/// Alternation with the `JMPNOT`-to-`JMPIF` success-chain rewrite.
fn gen_pattern_alternation<N: BackendNode>(
    cg: &mut Codegen,
    pattern: N,
    target: u16,
    fail_pos: &mut u32,
    known_array_len: i32,
    cache: Option<u16>,
) -> Result<(), Diagnostic> {
    let Some(view) = pattern.alternation_view() else {
        return Err(unsupported(&pattern, "alternation pattern"));
    };
    let left_is_alt = view.left.kind_name() == "AlternationPatternNode";
    let mut left_fail = JMPLINK_START;
    let mut success_pos = JMPLINK_START;
    codegen_pattern(
        cg,
        view.left,
        target,
        &mut left_fail,
        known_array_len,
        cache,
    )?;
    let tail_is_jmpnot = {
        let scope = &cg.current().1;
        !left_is_alt
            && left_fail != JMPLINK_START
            && left_fail >= 2
            && left_fail + 2 == scope.pc
            && scope.iseq[(left_fail - 2) as usize] == opcode::OP_JMPNOT
    };
    if tail_is_jmpnot {
        let prev_offset = i16::from_be_bytes([
            cg.current().1.iseq[left_fail as usize],
            cg.current().1.iseq[(left_fail + 1) as usize],
        ]);
        let next_addr = (left_fail + 2) as i32 + i32::from(prev_offset);
        let prev_link = if next_addr == 0 {
            JMPLINK_START
        } else {
            next_addr as u32
        };
        cg.current().1.iseq[(left_fail - 2) as usize] = opcode::OP_JMPIF;
        cg.current().1.emit_s(left_fail, 0)?;
        success_pos = left_fail;
        left_fail = prev_link;
    } else {
        success_pos = cg.current().1.genjmp(opcode::OP_JMP, success_pos)?;
    }
    if left_fail != JMPLINK_START {
        cg.current().1.dispatch_linked(left_fail)?;
    }
    codegen_pattern(cg, view.right, target, fail_pos, known_array_len, cache)?;
    if success_pos != JMPLINK_START {
        cg.current().1.dispatch_linked(success_pos)?;
    }
    Ok(())
}

/// Pre-rest elements of an array pattern through `AREF`.
fn gen_array_pattern_pres<N: BackendNode>(
    cg: &mut Codegen,
    requireds: &[N],
    base_reg: u16,
    fail_pos: &mut u32,
) -> Result<(), Diagnostic> {
    let mut base = i32::from(base_reg);
    let mut idx = 0;
    let scratch = gen_aref_scratch(cg, requireds.len() as i32)?;
    for element in requireds {
        if idx == 255 {
            base = gen_aref_rebase(cg, base, scratch)?;
            idx = 0;
        }
        let sp = cg.current().1.cursp();
        {
            let (session, scope) = cg.current();
            scope.genop_3(session, opcode::OP_AREF, sp, base as u16, idx as u8)?;
            scope.push_n(1)?;
        }
        codegen_pattern(cg, element.clone(), sp, fail_pos, -1, None)?;
        cg.current().1.pop_n(1)?;
        idx += 1;
    }
    if scratch >= 0 {
        cg.current().1.pop_n(1)?;
    }
    Ok(())
}

/// Rest binding of an array pattern (`arr[pre..-(post+1)]`).
fn gen_array_pattern_rest<N: BackendNode>(
    cg: &mut Codegen,
    rest: &Option<N>,
    arr_reg: u16,
    pre_len: i32,
    post_len: i32,
) -> Result<(), Diagnostic> {
    let Some(node) = rest else {
        return Ok(());
    };
    if node.kind_name() != "SplatNode" {
        return Ok(());
    }
    let Some(Some(inner)) = node.splat_value() else {
        return Ok(());
    };
    if inner.kind_name() != "LocalVariableTargetNode" {
        return Ok(());
    }
    let sp_save = cg.current().1.cursp();
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.gen_move(session, dst, arr_reg, false)?;
        scope.push_n(1)?;
        let dst = scope.cursp();
        scope.gen_int(session, dst, i64::from(pre_len))?;
        scope.push_n(1)?;
        let dst = scope.cursp();
        let end: i64 = if post_len > 0 {
            i64::from(-(post_len + 1))
        } else {
            -1
        };
        scope.gen_int(session, dst, end)?;
        let dst = scope.cursp();
        scope.genop_1(session, opcode::OP_RANGE_INC, dst - 1)?;
        scope.push_n(1)?;
        scope.pop_n(1)?;
        scope.sp = sp_save;
        let dst = scope.cursp();
        let sym = scope.new_sym(session, b"[]")?;
        scope.genop_3(session, opcode::OP_SEND, dst, sym, 1)?;
    }
    gen_pattern_bind(cg, inner, sp_save)
}

/// Post-rest elements of an array pattern through negative `GETIDX`.
fn gen_array_pattern_posts<N: BackendNode>(
    cg: &mut Codegen,
    posts: &[N],
    arr_reg: u16,
    fail_pos: &mut u32,
) -> Result<(), Diagnostic> {
    let post_len = posts.len() as i32;
    for (index, element) in posts.iter().enumerate() {
        {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.gen_move(session, dst, arr_reg, false)?;
            scope.push_n(1)?;
            let dst = scope.cursp();
            scope.gen_int(session, dst, i64::from(-(post_len - index as i32)))?;
            scope.push_n(1)?;
            scope.pop_n(1)?;
            let dst = scope.cursp();
            scope.genop_1(session, opcode::OP_GETIDX, dst - 1)?;
        }
        let reg = cg.current().1.cursp() - 1;
        codegen_pattern(cg, element.clone(), reg, fail_pos, -1, None)?;
        cg.current().1.pop_n(1)?;
    }
    Ok(())
}

/// Array pattern, with the known-length fast path for array literals.
fn gen_array_pattern<N: BackendNode>(
    cg: &mut Codegen,
    pattern: N,
    target: u16,
    fail_pos: &mut u32,
    known_array_len: i32,
    cache: Option<u16>,
) -> Result<(), Diagnostic> {
    let Some(view) = pattern.array_pattern_view() else {
        return Err(unsupported(&pattern, "array pattern"));
    };
    if let Some(constant) = view.constant {
        gen_pattern_eqq(cg, constant, target, fail_pos)?;
    }
    let pre_len = view.requireds.len() as i32;
    let post_len = view.posts.len() as i32;
    if known_array_len >= 0 {
        if view.rest.is_none() {
            if known_array_len != pre_len + post_len {
                let pos = cg.current().1.genjmp(opcode::OP_JMP, *fail_pos)?;
                *fail_pos = pos;
                return Ok(());
            }
        } else if known_array_len < pre_len + post_len {
            let pos = cg.current().1.genjmp(opcode::OP_JMP, *fail_pos)?;
            *fail_pos = pos;
            return Ok(());
        }
        gen_array_pattern_pres(cg, &view.requireds, target, fail_pos)?;
        gen_array_pattern_rest(cg, &view.rest, target, pre_len, post_len)?;
        gen_array_pattern_posts(cg, &view.posts, target, fail_pos)?;
        return Ok(());
    }
    gen_pattern_deconstruct(cg, target, fail_pos, cache)?;
    let arr_reg = cg.current().1.cursp();
    cg.current().1.push_n(1)?;
    gen_pattern_fail_jmp(cg, opcode::OP_JMPNIL, arr_reg, fail_pos, false)?;
    {
        let chk = cg.current().1.cursp();
        {
            let (session, scope) = cg.current();
            scope.gen_move(session, chk, arr_reg, false)?;
            scope.push_n(2)?;
            scope.pop_n(2)?;
            let sym = scope.new_sym(session, b"size")?;
            scope.genop_3(session, opcode::OP_SEND, chk, sym, 0)?;
        }
        {
            let (session, scope) = cg.current();
            scope.gen_int(session, chk + 1, i64::from(pre_len + post_len))?;
        }
        {
            let (session, scope) = cg.current();
            let op = if view.rest.is_none() {
                opcode::OP_EQ
            } else {
                opcode::OP_GE
            };
            scope.genop_1(session, op, chk)?;
        }
        let (session, scope) = cg.current();
        let pos = scope.genjmp2(session, opcode::OP_JMPNOT, chk, *fail_pos, true)?;
        *fail_pos = pos;
    }
    gen_array_pattern_pres(cg, &view.requireds, arr_reg, fail_pos)?;
    gen_array_pattern_rest(cg, &view.rest, arr_reg, pre_len, post_len)?;
    gen_array_pattern_posts(cg, &view.posts, arr_reg, fail_pos)?;
    cg.current().1.pop_n(1)?;
    Ok(())
}

/// Hash pattern through `deconstruct_keys` and `__pat_values`.
fn gen_hash_pattern<N: BackendNode>(
    cg: &mut Codegen,
    pattern: N,
    target: u16,
    fail_pos: &mut u32,
) -> Result<(), Diagnostic> {
    let Some(view) = pattern.hash_pattern_view() else {
        return Err(unsupported(&pattern, "hash pattern"));
    };
    if let Some(constant) = view.constant {
        gen_pattern_eqq(cg, constant, target, fail_pos)?;
    }
    let mut num_keys: u16 = 0;
    for element in &view.elements {
        if element.kind_name() == "AssocNode" {
            num_keys += 1;
        }
    }
    let has_rest = view.rest.is_some();
    let has_double_nil = view
        .rest
        .as_ref()
        .is_some_and(|rest| rest.kind_name() == "NoKeywordsParameterNode");
    gen_pattern_respond_to(cg, target, b"deconstruct_keys", fail_pos, None)?;
    let hash_reg = cg.current().1.cursp();
    {
        let (session, scope) = cg.current();
        scope.gen_move(session, hash_reg, target, false)?;
        scope.push_n(1)?;
    }
    if !has_rest && num_keys > 0 {
        let keys_base = cg.current().1.cursp();
        for element in &view.elements {
            if element.kind_name() != "AssocNode" {
                continue;
            }
            let Some((key, _)) = element.assoc_pair() else {
                return Err(unsupported(element, "hash pattern key"));
            };
            codegen(cg, key, true)?;
        }
        {
            let (session, scope) = cg.current();
            scope.pop_n(num_keys)?;
            scope.genop_2(session, opcode::OP_ARRAY, keys_base, num_keys)?;
            scope.push_n(1)?;
            scope.push_n(1)?;
            scope.pop_n(1)?;
            scope.sp = hash_reg;
            let sym = scope.new_sym(session, b"deconstruct_keys")?;
            scope.genop_3(session, opcode::OP_SEND, hash_reg, sym, 1)?;
        }
    } else {
        let dst = cg.current().1.cursp();
        {
            let (session, scope) = cg.current();
            scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
            scope.push_n(1)?;
            scope.push_n(1)?;
            scope.pop_n(1)?;
            scope.sp = hash_reg;
            let sym = scope.new_sym(session, b"deconstruct_keys")?;
            scope.genop_3(session, opcode::OP_SEND, hash_reg, sym, 1)?;
        }
    }
    cg.current().1.push_n(1)?;
    {
        let vals_reg = cg.current().1.cursp();
        {
            let (session, scope) = cg.current();
            scope.gen_move(session, vals_reg, hash_reg, false)?;
            scope.push_n(1)?;
        }
        let keys_base = cg.current().1.cursp();
        for element in &view.elements {
            if element.kind_name() != "AssocNode" {
                continue;
            }
            let Some((key, _)) = element.assoc_pair() else {
                return Err(unsupported(element, "hash pattern key"));
            };
            codegen(cg, key, true)?;
        }
        {
            let (session, scope) = cg.current();
            scope.pop_n(num_keys)?;
            scope.genop_2(session, opcode::OP_ARRAY, keys_base, num_keys)?;
            scope.push_n(1)?;
            scope.push_n(1)?;
            scope.pop_n(1)?;
            scope.sp = vals_reg;
            let sym = scope.new_sym(session, b"__pat_values")?;
            scope.genop_3(session, opcode::OP_SEND, vals_reg, sym, 1)?;
            scope.push_n(1)?;
        }
        {
            let (session, scope) = cg.current();
            let pos = scope.genjmp2(session, opcode::OP_JMPNOT, vals_reg, *fail_pos, true)?;
            *fail_pos = pos;
        }
        let loop_sp = cg.current().1.cursp();
        let mut key_idx: i64 = 0;
        for element in &view.elements {
            if element.kind_name() != "AssocNode" {
                continue;
            }
            let Some((_, sub)) = element.assoc_pair() else {
                return Err(unsupported(element, "hash pattern key"));
            };
            {
                let (session, scope) = cg.current();
                let val_reg = scope.cursp();
                scope.gen_move(session, val_reg, vals_reg, false)?;
                scope.push_n(1)?;
                let dst = scope.cursp();
                scope.gen_int(session, dst, key_idx)?;
                scope.push_n(1)?;
                scope.push_n(1)?;
                scope.pop_n(1)?;
                scope.sp = val_reg;
                let sym = scope.new_sym(session, b"[]")?;
                scope.genop_3(session, opcode::OP_SEND, val_reg, sym, 1)?;
                scope.push_n(1)?;
            }
            let val_reg = cg.current().1.cursp() - 1;
            codegen_pattern(cg, sub, val_reg, fail_pos, -1, None)?;
            cg.current().1.sp = loop_sp;
            key_idx += 1;
        }
        cg.current().1.pop_n(1)?;
    }
    if has_double_nil || (!has_rest && num_keys == 0) {
        let chk = cg.current().1.cursp();
        {
            let (session, scope) = cg.current();
            scope.gen_move(session, chk, hash_reg, false)?;
            scope.push_n(2)?;
            scope.pop_n(2)?;
            let sym = scope.new_sym(session, b"size")?;
            scope.genop_3(session, opcode::OP_SEND, chk, sym, 0)?;
        }
        {
            let (session, scope) = cg.current();
            scope.gen_int(session, chk + 1, i64::from(num_keys))?;
        }
        {
            let (session, scope) = cg.current();
            scope.genop_1(session, opcode::OP_EQ, chk)?;
        }
        let (session, scope) = cg.current();
        let pos = scope.genjmp2(session, opcode::OP_JMPNOT, chk, *fail_pos, true)?;
        *fail_pos = pos;
    } else if has_rest && !has_double_nil {
        if let Some(rest) = &view.rest {
            if rest.kind_name() == "AssocSplatNode" {
                let bound = rest.assoc_splat_value().is_some_and(|inner| match inner {
                    Some(node) => node.kind_name() == "LocalVariableTargetNode",
                    None => false,
                });
                if bound {
                    let Some(Some(target_node)) = rest.assoc_splat_value() else {
                        return Err(unsupported(rest, "hash rest pattern"));
                    };
                    let recv = cg.current().1.cursp();
                    {
                        let (session, scope) = cg.current();
                        scope.gen_move(session, recv, hash_reg, false)?;
                        scope.push_n(1)?;
                    }
                    if num_keys > 0 {
                        let keys_base = cg.current().1.cursp();
                        for element in &view.elements {
                            if element.kind_name() != "AssocNode" {
                                continue;
                            }
                            let Some((key, _)) = element.assoc_pair() else {
                                return Err(unsupported(element, "hash pattern key"));
                            };
                            codegen(cg, key, true)?;
                        }
                        {
                            let (session, scope) = cg.current();
                            scope.pop_n(num_keys)?;
                            scope.genop_2(session, opcode::OP_ARRAY, keys_base, num_keys)?;
                            scope.push_n(1)?;
                            scope.push_n(1)?;
                            scope.pop_n(1)?;
                            scope.sp = recv;
                            let sym = scope.new_sym(session, b"__except")?;
                            scope.genop_3(session, opcode::OP_SEND, recv, sym, 1)?;
                        }
                    } else {
                        let (session, scope) = cg.current();
                        scope.push_n(1)?;
                        scope.pop_n(1)?;
                        scope.sp = recv;
                        let sym = scope.new_sym(session, b"dup")?;
                        scope.genop_3(session, opcode::OP_SEND, recv, sym, 0)?;
                    }
                    gen_pattern_bind(cg, target_node, recv)?;
                }
            }
        }
    }
    cg.current().1.pop_n(1)?;
    Ok(())
}

/// Find pattern (`*pre, mid, *post`) as a search loop.
fn gen_find_pattern<N: BackendNode>(
    cg: &mut Codegen,
    pattern: N,
    target: u16,
    fail_pos: &mut u32,
    cache: Option<u16>,
) -> Result<(), Diagnostic> {
    let Some(view) = pattern.find_pattern_view() else {
        return Err(unsupported(&pattern, "find pattern"));
    };
    let elems_len = view.requireds.len() as i32;
    if let Some(constant) = view.constant {
        gen_pattern_eqq(cg, constant, target, fail_pos)?;
    }
    gen_pattern_deconstruct(cg, target, fail_pos, cache)?;
    let arr_reg = cg.current().1.cursp();
    cg.current().1.push_n(1)?;
    gen_pattern_fail_jmp(cg, opcode::OP_JMPNIL, arr_reg, fail_pos, false)?;
    {
        let dst = cg.current().1.cursp();
        {
            let (session, scope) = cg.current();
            scope.gen_move(session, dst, arr_reg, false)?;
            scope.push_n(2)?;
            scope.pop_n(2)?;
            let sym = scope.new_sym(session, b"size")?;
            scope.genop_3(session, opcode::OP_SEND, dst, sym, 0)?;
        }
        {
            let (session, scope) = cg.current();
            scope.gen_int(session, dst + 1, i64::from(elems_len))?;
        }
        {
            let (session, scope) = cg.current();
            scope.genop_1(session, opcode::OP_GE, dst)?;
        }
        let (session, scope) = cg.current();
        let pos = scope.genjmp2(session, opcode::OP_JMPNOT, dst, *fail_pos, true)?;
        *fail_pos = pos;
    }
    let idx_reg = cg.current().1.cursp();
    {
        let (session, scope) = cg.current();
        scope.gen_int(session, idx_reg, 0)?;
        scope.push_n(1)?;
    }
    let loop_start = cg.current().1.new_label();
    let mut match_fail = JMPLINK_START;
    {
        let dst = cg.current().1.cursp();
        {
            let (session, scope) = cg.current();
            scope.gen_move(session, dst, arr_reg, false)?;
            scope.push_n(2)?;
            scope.pop_n(2)?;
            let sym = scope.new_sym(session, b"size")?;
            scope.genop_3(session, opcode::OP_SEND, dst, sym, 0)?;
        }
        {
            let (session, scope) = cg.current();
            scope.gen_int(session, dst + 1, i64::from(elems_len))?;
            scope.genop_1(session, opcode::OP_SUB, dst)?;
        }
        {
            let (session, scope) = cg.current();
            scope.gen_move(session, dst + 1, idx_reg, false)?;
            scope.genop_1(session, opcode::OP_GE, dst)?;
        }
        let (session, scope) = cg.current();
        let pos = scope.genjmp2(session, opcode::OP_JMPNOT, dst, *fail_pos, true)?;
        *fail_pos = pos;
    }
    for (index, element) in view.requireds.iter().enumerate() {
        {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.gen_move(session, dst, arr_reg, false)?;
            scope.push_n(1)?;
            if index == 0 {
                let dst = scope.cursp();
                scope.gen_move(session, dst, idx_reg, false)?;
            } else {
                let dst = scope.cursp();
                scope.gen_move(session, dst, idx_reg, false)?;
                scope.gen_int(session, dst + 1, index as i64)?;
                scope.genop_1(session, opcode::OP_ADD, dst)?;
            }
            scope.push_n(2)?;
            scope.pop_n(2)?;
            let dst = scope.cursp();
            scope.genop_1(session, opcode::OP_GETIDX, dst - 1)?;
        }
        let elem_reg = cg.current().1.cursp() - 1;
        codegen_pattern(cg, element.clone(), elem_reg, &mut match_fail, -1, None)?;
        cg.current().1.pop_n(1)?;
    }
    gen_find_pattern_end(cg, &view.left, arr_reg, idx_reg, elems_len, true)?;
    gen_find_pattern_end(cg, &view.right, arr_reg, idx_reg, elems_len, false)?;
    let loop_end = cg.current().1.genjmp(opcode::OP_JMP, JMPLINK_START)?;
    if match_fail != JMPLINK_START {
        cg.current().1.dispatch_linked(match_fail)?;
    }
    {
        let (session, scope) = cg.current();
        scope.genop_2(session, opcode::OP_ADDI, idx_reg, 1)?;
    }
    cg.current().1.genjmp(opcode::OP_JMP, loop_start)?;
    cg.current().1.dispatch(loop_end)?;
    cg.current().1.pop_n(1)?;
    cg.current().1.pop_n(1)?;
    Ok(())
}

/// One rest binding of a find pattern (`arr[0...idx]`, `arr[idx+len..-1]`).
fn gen_find_pattern_end<N: BackendNode>(
    cg: &mut Codegen,
    end: &N,
    arr_reg: u16,
    idx_reg: u16,
    elems_len: i32,
    is_pre: bool,
) -> Result<(), Diagnostic> {
    if end.kind_name() != "SplatNode" {
        return Ok(());
    }
    let Some(Some(inner)) = end.splat_value() else {
        return Ok(());
    };
    if inner.kind_name() != "LocalVariableTargetNode" {
        return Ok(());
    }
    if is_pre {
        {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.gen_move(session, dst, arr_reg, false)?;
            scope.push_n(1)?;
            let dst = scope.cursp();
            scope.gen_int(session, dst, 0)?;
            scope.push_n(1)?;
            let dst = scope.cursp();
            scope.gen_move(session, dst, idx_reg, false)?;
            scope.push_n(1)?;
            scope.pop_n(1)?;
            let dst = scope.cursp();
            scope.genop_1(session, opcode::OP_RANGE_EXC, dst - 1)?;
            scope.pop_n(2)?;
            let dst = scope.cursp();
            let sym = scope.new_sym(session, b"[]")?;
            scope.genop_3(session, opcode::OP_SEND, dst, sym, 1)?;
        }
        let cur = cg.current().1.cursp();
        gen_pattern_bind(cg, inner, cur)
    } else {
        {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.gen_move(session, dst, arr_reg, false)?;
            scope.push_n(1)?;
            let dst = scope.cursp();
            scope.gen_move(session, dst, idx_reg, false)?;
            scope.gen_int(session, dst + 1, i64::from(elems_len))?;
            scope.genop_1(session, opcode::OP_ADD, dst)?;
            scope.push_n(1)?;
            let dst = scope.cursp();
            scope.gen_int(session, dst, -1)?;
            scope.push_n(1)?;
            scope.pop_n(1)?;
            let dst = scope.cursp();
            scope.genop_1(session, opcode::OP_RANGE_INC, dst - 1)?;
            scope.pop_n(2)?;
            let dst = scope.cursp();
            let sym = scope.new_sym(session, b"[]")?;
            scope.genop_3(session, opcode::OP_SEND, dst, sym, 1)?;
        }
        let cur = cg.current().1.cursp();
        gen_pattern_bind(cg, inner, cur)
    }
}

/// One-line `expr in pattern` (`PM_MATCH_PREDICATE_NODE`).
fn gen_match_predicate<N: BackendNode>(
    cg: &mut Codegen,
    node: N,
    val: bool,
) -> Result<(), Diagnostic> {
    let Some(view) = node.match_predicate_view() else {
        return Err(unsupported(&node, "match predicate"));
    };
    let head = cg.current().1.cursp();
    codegen(cg, view.value, true)?;
    let mut fail_pos = JMPLINK_START;
    codegen_pattern(cg, view.pattern, head, &mut fail_pos, -1, None)?;
    {
        let (session, scope) = cg.current();
        scope.genop_1(session, opcode::OP_LOADTRUE, head)?;
    }
    let done = cg.current().1.genjmp(opcode::OP_JMP, JMPLINK_START)?;
    if fail_pos != JMPLINK_START {
        cg.current().1.dispatch_linked(fail_pos)?;
    }
    {
        let (session, scope) = cg.current();
        scope.genop_1(session, opcode::OP_LOADFALSE, head)?;
    }
    cg.current().1.dispatch(done)?;
    if !val {
        cg.current().1.pop_n(1)?;
    }
    Ok(())
}

/// One-line `expr => pattern` (`PM_MATCH_REQUIRED_NODE`).
fn gen_match_required<N: BackendNode>(
    cg: &mut Codegen,
    node: N,
    val: bool,
) -> Result<(), Diagnostic> {
    let Some(view) = node.match_required_view() else {
        return Err(unsupported(&node, "match required"));
    };
    let head = cg.current().1.cursp();
    codegen(cg, view.value, true)?;
    let mut fail_pos = JMPLINK_START;
    codegen_pattern(cg, view.pattern, head, &mut fail_pos, -1, None)?;
    let ok = cg.current().1.genjmp(opcode::OP_JMP, JMPLINK_START)?;
    if fail_pos != JMPLINK_START {
        cg.current().1.dispatch_linked(fail_pos)?;
    }
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_1(session, opcode::OP_LOADFALSE, dst)?;
        let dst = scope.cursp();
        scope.genop_1(session, opcode::OP_MATCHERR, dst)?;
    }
    cg.current().1.dispatch(ok)?;
    cg.current().1.pop_n(1)?;
    if val {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
        scope.push_n(1)?;
    }
    Ok(())
}

/// `case/in` (`PM_CASE_MATCH_NODE`).
fn gen_case_match<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(view) = node.case_match_view() else {
        return Err(unsupported(&node, "case match"));
    };
    let mut head: u16 = 0;
    let mut case_end_jumps = JMPLINK_START;
    let mut known_array_len: i32 = -1;
    if let Some(predicate) = &view.predicate {
        if predicate.kind_name() == "ArrayNode" {
            if let Some(elements) = predicate.raw_array_elements() {
                if !elements
                    .iter()
                    .any(|element| element.kind_name() == "SplatNode")
                {
                    known_array_len = elements.len() as i32;
                }
            }
        }
    }
    let mut cache: Option<u16> = None;
    if let Some(predicate) = view.predicate {
        head = cg.current().1.cursp();
        codegen(cg, predicate, true)?;
        if known_array_len < 0 && view.conditions.len() > 1 {
            for condition in &view.conditions {
                let is_in = condition.kind_name() == "InNode";
                let deconstructs = condition
                    .in_view()
                    .is_some_and(|in_view| pattern_deconstructs(&in_view.pattern));
                if is_in && deconstructs {
                    let slot = cg.current().1.cursp();
                    {
                        let (session, scope) = cg.current();
                        scope.genop_1(session, opcode::OP_LOADNIL, slot)?;
                        scope.push_n(1)?;
                    }
                    cache = Some(slot);
                    break;
                }
            }
        }
    }
    for condition in view.conditions {
        let Some(in_view) = condition.in_view() else {
            return Err(unsupported(&condition, "in clause"));
        };
        let mut fail_pos = JMPLINK_START;
        codegen_pattern(
            cg,
            in_view.pattern,
            head,
            &mut fail_pos,
            known_array_len,
            cache,
        )?;
        gen_branch(cg, in_view.body, val)?;
        if val {
            cg.current().1.pop_n(1)?;
        }
        let pos = cg.current().1.genjmp(opcode::OP_JMP, case_end_jumps)?;
        case_end_jumps = pos;
        if fail_pos != JMPLINK_START {
            cg.current().1.dispatch_linked(fail_pos)?;
        }
    }
    if let Some(else_body) = view.else_body {
        codegen(cg, else_body, val)?;
        if val {
            cg.current().1.pop_n(1)?;
        }
    } else {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_1(session, opcode::OP_LOADFALSE, dst)?;
        let dst = scope.cursp();
        scope.genop_1(session, opcode::OP_MATCHERR, dst)?;
    }
    if case_end_jumps != JMPLINK_START {
        cg.current().1.dispatch_linked(case_end_jumps)?;
    }
    if val {
        if head != 0 {
            {
                let (session, scope) = cg.current();
                let dst = scope.cursp();
                scope.gen_move(session, head, dst, false)?;
            }
            cg.current().1.pop_n(if cache.is_some() { 2 } else { 1 })?;
        }
        cg.current().1.push_n(1)?;
    } else if head != 0 {
        cg.current().1.pop_n(if cache.is_some() { 2 } else { 1 })?;
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

/// Method definition (`PM_DEF_NODE`, endless defs share the node).
/// Only required positional parameters are covered; the accessor gates
/// optional/rest/keyword/block forms (`lambda_body` `blk=0` slice).
fn gen_def<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(view) = node.def_view() else {
        return Err(unsupported(&node, "method parameters"));
    };
    // Full `lambda_body` (`blk=0`): parameter registers, `OP_ENTER`,
    // defaults, keywords, block move and destructured slots.
    let (locals, setup) = match view.params {
        None => {
            let mut locals = vec![Vec::new()];
            for name in &view.locals {
                if !locals.contains(name) {
                    locals.push(name.clone());
                }
            }
            (locals, None)
        }
        Some(params) => {
            let (lv, counts) = param_layout(&params, &view.locals, &[], &node)?;
            let (ainfo, aspec) = param_enter(&counts)?;
            (lv, Some((params, counts, ainfo, aspec)))
        }
    };
    match &setup {
        None => cg.push_method_body(&locals, ainfo_req(0), args_req(0))?,
        Some((_, _, ainfo, aspec)) => cg.push_method_body(&locals, *ainfo, *aspec)?,
    }
    if let Some((params, counts, _, _)) = &setup {
        emit_param_setup(cg, params, counts)?;
    }
    match view.body {
        Some(body) => codegen(cg, body, true)?,
        None => emit_absent_else(cg)?,
    }
    {
        let (session, scope) = cg.current();
        scope.pop_n(1)?;
        let ret = scope.cursp();
        scope.gen_return(session, opcode::OP_RETURN, ret)?;
    }
    let idx = cg.pop_scope()?;
    let sym = {
        let (session, scope) = cg.current();
        scope.new_sym(session, &view.name)?
    };
    match view.receiver {
        None => {
            if idx <= 0xff {
                let (session, scope) = cg.current();
                let dst = scope.cursp();
                scope.genop_3(session, opcode::OP_TDEF, dst, sym, idx as u8)?;
            } else {
                let (session, scope) = cg.current();
                let dst = scope.cursp();
                scope.genop_1(session, opcode::OP_TCLASS, dst)?;
                scope.push_n(1)?;
                let dst = scope.cursp();
                scope.genop_2(session, opcode::OP_METHOD, dst, idx as u16)?;
                scope.push_n(1)?;
                scope.pop_n(1)?;
                scope.pop_n(1)?;
                let dst = scope.cursp();
                scope.genop_2(session, opcode::OP_DEF, dst, sym)?;
            }
        }
        Some(receiver) => {
            codegen(cg, receiver, true)?;
            cg.current().1.pop_n(1)?;
            if idx <= 0xff {
                let (session, scope) = cg.current();
                let dst = scope.cursp();
                scope.genop_3(session, opcode::OP_SDEF, dst, sym, idx as u8)?;
            } else {
                let (session, scope) = cg.current();
                let dst = scope.cursp();
                scope.genop_1(session, opcode::OP_SCLASS, dst)?;
                scope.push_n(1)?;
                let dst = scope.cursp();
                scope.genop_2(session, opcode::OP_METHOD, dst, idx as u16)?;
                scope.push_n(1)?;
                scope.pop_n(1)?;
                scope.pop_n(1)?;
                let dst = scope.cursp();
                scope.genop_2(session, opcode::OP_DEF, dst, sym)?;
            }
        }
    }
    if val {
        cg.current().1.push_n(1)?;
    }
    Ok(())
}

/// Class definition (`PM_CLASS_NODE` with `scope_body` for the body).
fn gen_class<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(view) = node.class_view() else {
        return Err(unsupported(&node, "class path"));
    };
    if view.cpath_is_read {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
        scope.push_n(1)?;
    } else {
        match view.cpath_parent {
            Some(parent) => codegen(cg, parent, true)?,
            None => {
                let (session, scope) = cg.current();
                let dst = scope.cursp();
                scope.genop_1(session, opcode::OP_OCLASS, dst)?;
                scope.push_n(1)?;
            }
        }
    }
    match view.superclass {
        Some(superclass) => codegen(cg, superclass, true)?,
        None => {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
            scope.push_n(1)?;
        }
    }
    cg.current().1.pop_n(2)?;
    let sym = {
        let (session, scope) = cg.current();
        scope.new_sym(session, &view.name)?
    };
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_CLASS, dst, sym)?;
    }
    match view.body {
        None => {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
        }
        Some(body) => {
            cg.push_body(&view.locals)?;
            codegen(cg, body, true)?;
            {
                let (session, scope) = cg.current();
                let ret = scope.sp - 1;
                scope.gen_return(session, opcode::OP_RETURN, ret)?;
            }
            let idx = cg.pop_scope()?;
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_2(session, opcode::OP_EXEC, dst, idx as u16)?;
        }
    }
    if val {
        cg.current().1.push_n(1)?;
    }
    Ok(())
}

/// Module definition (`PM_MODULE_NODE`).
fn gen_module<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(view) = node.module_view() else {
        return Err(unsupported(&node, "module path"));
    };
    if view.cpath_is_read {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
        scope.push_n(1)?;
    } else {
        match view.cpath_parent {
            Some(parent) => codegen(cg, parent, true)?,
            None => {
                let (session, scope) = cg.current();
                let dst = scope.cursp();
                scope.genop_1(session, opcode::OP_OCLASS, dst)?;
                scope.push_n(1)?;
            }
        }
    }
    cg.current().1.pop_n(1)?;
    let sym = {
        let (session, scope) = cg.current();
        scope.new_sym(session, &view.name)?
    };
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_MODULE, dst, sym)?;
    }
    match view.body {
        None => {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
        }
        Some(body) => {
            cg.push_body(&view.locals)?;
            codegen(cg, body, true)?;
            {
                let (session, scope) = cg.current();
                let ret = scope.sp - 1;
                scope.gen_return(session, opcode::OP_RETURN, ret)?;
            }
            let idx = cg.pop_scope()?;
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_2(session, opcode::OP_EXEC, dst, idx as u16)?;
        }
    }
    if val {
        cg.current().1.push_n(1)?;
    }
    Ok(())
}

/// Singleton class (`PM_SINGLETON_CLASS_NODE`).
fn gen_sclass<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(view) = node.sclass_view() else {
        return Err(unsupported(&node, "singleton class"));
    };
    codegen(cg, view.expression, true)?;
    cg.current().1.pop_n(1)?;
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_1(session, opcode::OP_SCLASS, dst)?;
    }
    match view.body {
        None => {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
        }
        Some(body) => {
            cg.push_body(&view.locals)?;
            codegen(cg, body, true)?;
            {
                let (session, scope) = cg.current();
                let ret = scope.sp - 1;
                scope.gen_return(session, opcode::OP_RETURN, ret)?;
            }
            let idx = cg.pop_scope()?;
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_2(session, opcode::OP_EXEC, dst, idx as u16)?;
        }
    }
    if val {
        cg.current().1.push_n(1)?;
    }
    Ok(())
}

/// Constant read (`PM_CONSTANT_READ_NODE`).
fn gen_const_read<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(name) = node.const_read() else {
        return Err(unsupported(&node, "constant read"));
    };
    let sym = {
        let (session, scope) = cg.current();
        scope.new_sym(session, &name)?
    };
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_GETCONST, dst, sym)?;
    }
    if val {
        cg.current().1.push_n(1)?;
    }
    Ok(())
}

/// Plain constant or variable store shared by `=` writes (`gen_assignment`
/// tail with `gen_setxv` plus the valued push).
fn gen_plain_store(cg: &mut Codegen, op: u8, name: &[u8], val: bool) -> Result<(), Diagnostic> {
    let sp = cg.current().1.cursp();
    {
        let (session, scope) = cg.current();
        scope.gen_setxv(session, op, sp, name, val)?;
    }
    if val {
        cg.current().1.push_n(1)?;
    }
    Ok(())
}

/// Constant write (`PM_CONSTANT_WRITE_NODE`).
fn gen_const_write<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(write) = node.const_write() else {
        return Err(unsupported(&node, "constant write"));
    };
    codegen(cg, write.value, true)?;
    cg.current().1.pop_n(1)?;
    gen_plain_store(cg, opcode::OP_SETCONST, &write.name, val)
}

/// Constant path read (`PM_CONSTANT_PATH_NODE`).
fn gen_const_path_read<N: BackendNode>(
    cg: &mut Codegen,
    node: N,
    val: bool,
) -> Result<(), Diagnostic> {
    let Some(path) = node.const_path() else {
        return Err(unsupported(&node, "constant path"));
    };
    let sym = {
        let (session, scope) = cg.current();
        scope.new_sym(session, &path.name)?
    };
    match path.parent {
        Some(parent) => {
            codegen(cg, parent, true)?;
            cg.current().1.pop_n(1)?;
        }
        None => {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_1(session, opcode::OP_OCLASS, dst)?;
        }
    }
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_GETMCNST, dst, sym)?;
    }
    if val {
        cg.current().1.push_n(1)?;
    }
    Ok(())
}

/// Constant path write (`PM_CONSTANT_PATH_WRITE_NODE`).
fn gen_const_path_write<N: BackendNode>(
    cg: &mut Codegen,
    node: N,
    val: bool,
) -> Result<(), Diagnostic> {
    let Some(write) = node.const_path_write() else {
        return Err(unsupported(&node, "constant path write"));
    };
    let base = cg.current().1.cursp();
    cg.current().1.push_n(1)?;
    let sym = match write.parent {
        Some(parent) => {
            codegen(cg, parent, true)?;
            let (session, scope) = cg.current();
            scope.new_sym(session, &write.name)?
        }
        None => {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_1(session, opcode::OP_OCLASS, dst)?;
            scope.push_n(1)?;
            let (session, scope) = cg.current();
            scope.new_sym(session, &write.name)?
        }
    };
    codegen(cg, write.value, true)?;
    {
        let (session, scope) = cg.current();
        scope.pop_n(1)?;
        let src = scope.cursp();
        scope.gen_move(session, base, src, false)?;
        scope.pop_n(2)?;
        scope.genop_2(session, opcode::OP_SETMCNST, base, sym)?;
    }
    if val {
        cg.current().1.push_n(1)?;
    }
    Ok(())
}

/// Instance variable read (`PM_INSTANCE_VARIABLE_READ_NODE`).
fn gen_ivar_read<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(name) = node.ivar_read() else {
        return Err(unsupported(&node, "instance variable read"));
    };
    let sym = {
        let (session, scope) = cg.current();
        scope.new_sym(session, &name)?
    };
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_GETIV, dst, sym)?;
    }
    if val {
        cg.current().1.push_n(1)?;
    }
    Ok(())
}

/// Instance variable write (`PM_INSTANCE_VARIABLE_WRITE_NODE`).
fn gen_ivar_write<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(write) = node.ivar_write() else {
        return Err(unsupported(&node, "instance variable write"));
    };
    codegen(cg, write.value, true)?;
    cg.current().1.pop_n(1)?;
    gen_plain_store(cg, opcode::OP_SETIV, &write.name, val)
}

/// Class variable read (`PM_CLASS_VARIABLE_READ_NODE`).
fn gen_cvar_read<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(name) = node.cvar_read() else {
        return Err(unsupported(&node, "class variable read"));
    };
    let sym = {
        let (session, scope) = cg.current();
        scope.new_sym(session, &name)?
    };
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_GETCV, dst, sym)?;
    }
    if val {
        cg.current().1.push_n(1)?;
    }
    Ok(())
}

/// Class variable write (`PM_CLASS_VARIABLE_WRITE_NODE`).
fn gen_cvar_write<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(write) = node.cvar_write() else {
        return Err(unsupported(&node, "class variable write"));
    };
    codegen(cg, write.value, true)?;
    cg.current().1.pop_n(1)?;
    gen_plain_store(cg, opcode::OP_SETCV, &write.name, val)
}

/// Global variable read (`PM_GLOBAL_VARIABLE_READ_NODE`).
fn gen_gvar_read<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(name) = node.gvar_read() else {
        return Err(unsupported(&node, "global variable read"));
    };
    let sym = {
        let (session, scope) = cg.current();
        scope.new_sym(session, &name)?
    };
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_GETGV, dst, sym)?;
    }
    if val {
        cg.current().1.push_n(1)?;
    }
    Ok(())
}

/// Match-back-reference read (`gen_match_ref`): `$&`, `` $` ``, `$'`,
/// `$+` and `$N` read `$~` and, where it is not nil, send it the private
/// reader (`__group` with `n` for `$&`/`$N`, `__pre_match`, `__post_match`
/// or `__last_group`); a nil `$~` reads as nil like an unset global.
fn gen_match_ref(cg: &mut Codegen, meth: &[u8], n: i64) -> Result<(), Diagnostic> {
    let last_match = {
        let (session, scope) = cg.current();
        scope.new_sym(session, b"$~")?
    };
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_GETGV, dst, last_match)?;
    }
    let skip = {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genjmp2(session, opcode::OP_JMPNIL, dst, JMPLINK_START, true)?
    };
    cg.current().1.push_n(1)?;
    if n >= 0 {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.gen_int(session, dst, n)?;
        cg.current().1.push_n(1)?;
    }
    cg.current().1.push_n(1)?;
    cg.current().1.pop_n(1)?;
    cg.current().1.pop_n(if n >= 0 { 2 } else { 1 })?;
    let sym = {
        let (session, scope) = cg.current();
        scope.new_sym(session, meth)?
    };
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        if n >= 0 {
            scope.genop_3(session, opcode::OP_SEND, dst, sym, 1)?;
        } else {
            scope.genop_2(session, opcode::OP_SEND0, dst, sym)?;
        }
    }
    cg.current().1.dispatch(skip)?;
    cg.current().1.push_n(1)
}

/// `$&`, `` $` ``, `$'` and `$+` (`PM_BACK_REFERENCE_READ_NODE`).
fn gen_backref<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    if !val {
        return Ok(());
    }
    let Some(name) = node.backref_name() else {
        return Err(unsupported(&node, "back-reference read"));
    };
    // The parser admits no other name here; `$+` takes the default arm.
    match name.get(1).copied().unwrap_or(0) {
        b'&' => gen_match_ref(cg, b"__group", 0),
        b'`' => gen_match_ref(cg, b"__pre_match", -1),
        b'\'' => gen_match_ref(cg, b"__post_match", -1),
        _ => gen_match_ref(cg, b"__last_group", -1),
    }
}

/// `$N` (`PM_NUMBERED_REFERENCE_READ_NODE`): an unrepresentable number
/// reads as nil without asking, like CRuby.
fn gen_numbered_ref<N: BackendNode>(
    cg: &mut Codegen,
    node: N,
    val: bool,
) -> Result<(), Diagnostic> {
    if !val {
        return Ok(());
    }
    let Some(number) = node.numbered_ref_number() else {
        return Err(unsupported(&node, "numbered reference read"));
    };
    if number == 0 || number > i32::MAX as u32 {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
        return scope.push_n(1);
    }
    gen_match_ref(cg, b"__group", i64::from(number))
}

/// Global variable write (`PM_GLOBAL_VARIABLE_WRITE_NODE`).
fn gen_gvar_write<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(write) = node.gvar_write() else {
        return Err(unsupported(&node, "global variable write"));
    };
    codegen(cg, write.value, true)?;
    cg.current().1.pop_n(1)?;
    gen_plain_store(cg, opcode::OP_SETGV, &write.name, val)
}

/// Explicit `super` with plain arguments (`PM_SUPER_NODE`).
fn gen_super<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(view) = node.super_view() else {
        return Err(unsupported(&node, "super arguments"));
    };
    let (ainfo, level, _) = cg.method_scope();
    cg.current().1.push_n(1)?;
    let mut count: i32 = 0;
    let mut stacked: i32 = 0;
    if let Some(args) = view.args {
        let arg_count = gen_values(cg, args, true, CALL_ARG_LIMIT)?;
        if arg_count < 0 {
            stacked = 1;
            count = CALL_MAXARGS;
            cg.current().1.push_n(1)?;
        } else {
            stacked = arg_count;
            count = arg_count;
        }
    }
    if ainfo >= 0 {
        gen_blkmove(cg, ainfo as u16, level)?;
    } else {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
        scope.push_n(1)?;
    }
    stacked += 1;
    cg.current().1.pop_n((stacked + 1) as u16)?;
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_SUPER, dst, count as u16)?;
    }
    if val {
        cg.current().1.push_n(1)?;
    }
    Ok(())
}

/// Bare `super` forwarding the method arguments (`PM_FORWARDING_SUPER_NODE`).
/// Keyword-bearing methods need `gen_zsuper_kwargs` and stay gated.
fn gen_zsuper<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(block) = node.forwarding_super() else {
        return Err(unsupported(&node, "super"));
    };
    if block.is_some() {
        return Err(unsupported(&node, "block argument"));
    }
    let (ainfo, level, _) = cg.method_scope();
    let saved = cg.current().1.cursp();
    cg.current().1.push_n(1)?;
    let count: i32 = if ainfo > 0 {
        if (ainfo as u16 & 0x1) != 0 {
            return Err(unsupported(&node, "keyword forwarding"));
        }
        let operand = mscope_operand(ainfo as u16, level)?;
        {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_2s(session, opcode::OP_ARGARY, dst, operand)?;
            scope.push_n(3)?;
            scope.pop_n(3)?;
        }
        CALL_MAXARGS
    } else {
        if ainfo >= 0 {
            gen_blkmove(cg, ainfo as u16, level)?;
        } else {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
            scope.push_n(1)?;
        }
        0
    };
    cg.current().1.sp = saved;
    {
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_2(session, opcode::OP_SUPER, dst, count as u16)?;
    }
    if val {
        cg.current().1.push_n(1)?;
    }
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

/// Jump outside any loop (`raise_error` for `unexpected break/next/redo`):
/// the C message with the site span.
fn unexpected<N: BackendNode>(site: &N, message: &str) -> Diagnostic {
    let span = site.span();
    Diagnostic {
        message: message.to_owned(),
        start: span.start,
        end: span.end,
    }
}

/// Innermost loop visible through `begin`/`rescue` scaffolds (`loop_break`
/// and friends skip `LOOP_BEGIN`/`LOOP_RESCUE` frames the same way).
fn jump_target(cg: &mut Codegen) -> Option<usize> {
    cg.current()
        .1
        .loops
        .iter()
        .rposition(|frame| !matches!(frame.kind, LoopType::Begin | LoopType::Rescue))
}

/// Single-or-array jump value (`gen_retval`): one argument codes bare (a
/// lone splat via `ARYSPLAT`), several ride the arguments array; the value
/// stays at the cursor with no net stack change.
fn gen_retval<N: BackendNode>(cg: &mut Codegen, args: N) -> Result<(), Diagnostic> {
    let Some(items) = args.call_args() else {
        return Err(unsupported(&args, "complex arguments"));
    };
    if items.len() == 1 {
        let item = items.into_iter().next().expect("single argument");
        if item.kind_name() == "SplatNode" {
            let Some(inner) = item.splat_value() else {
                return Err(unsupported(&item, "splat"));
            };
            match inner {
                Some(expression) => codegen(cg, expression, true)?,
                None => gen_lvar(cg, b"*", 0)?,
            }
            cg.current().1.pop_n(1)?;
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_1(session, opcode::OP_ARYSPLAT, dst)?;
        } else {
            codegen(cg, item, true)?;
            cg.current().1.pop_n(1)?;
        }
        return Ok(());
    }
    codegen(cg, args, true)?;
    cg.current().1.pop_n(1)?;
    Ok(())
}

/// Bare arguments array (`PM_ARGUMENTS_NODE`): `gen_values` plus the
/// `ARRAY` gather, reached only through `gen_retval`'s multi-value path.
fn gen_arguments<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(items) = node.call_args() else {
        return Err(unsupported(&node, "complex arguments"));
    };
    let n = gen_values(cg, items, val, 0)?;
    if val {
        if n >= 0 {
            cg.current().1.pop_n(n as u16)?;
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_2(session, opcode::OP_ARRAY, dst, n as u16)?;
        }
        cg.current().1.push_n(1)?;
    }
    Ok(())
}

/// `return` (`PM_RETURN_NODE`): valued or bare, `RETURN_BLK` inside any
/// loop frame (blocks, loops and the `begin`/`rescue` scaffolds all push
/// one), else `RETURN` (`return_leaves_upper_p` is always false under
/// `MRC_TARGET_MRUBYC`).
fn gen_return_node<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(args) = node.return_args() else {
        return Err(unsupported(&node, "return"));
    };
    match args {
        Some(arguments) => gen_retval(cg, arguments)?,
        None => {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
        }
    }
    {
        let (session, scope) = cg.current();
        let src = scope.cursp();
        let op = if scope.loops.is_empty() {
            opcode::OP_RETURN
        } else {
            opcode::OP_RETURN_BLK
        };
        scope.gen_return(session, op, src)?;
    }
    if val {
        cg.current().1.push_n(1)?;
    }
    Ok(())
}

/// `break` value and target (`loop_break`): the value lands in the loop
/// register for `LOOP_NORMAL` (a `JMPUW` chain out), or leaves via
/// `OP_BREAK` for block-style frames.
fn loop_break<N: BackendNode>(
    cg: &mut Codegen,
    args: Option<N>,
    site: &N,
) -> Result<(), Diagnostic> {
    let has_value = args.is_some();
    if cg.current().1.loops.is_empty() {
        if let Some(arguments) = args {
            codegen(cg, arguments, false)?;
        }
        return Err(unexpected(site, "unexpected break"));
    }
    if let Some(arguments) = args {
        let reg = cg.current().1.loops.last().expect("loop").reg;
        if reg < 0 {
            codegen(cg, arguments, false)?;
        } else {
            gen_retval(cg, arguments)?;
        }
    }
    let Some(index) = jump_target(cg) else {
        return Err(unexpected(site, "unexpected break"));
    };
    let kind = cg.current().1.loops[index].kind;
    if kind == LoopType::Normal {
        let reg = cg.current().1.loops[index].reg;
        if reg >= 0 {
            if has_value {
                let (session, scope) = cg.current();
                let src = scope.cursp();
                scope.gen_move(session, reg as u16, src, false)?;
            } else {
                let (session, scope) = cg.current();
                scope.genop_1(session, opcode::OP_LOADNIL, reg as u16)?;
            }
        }
        let pc2 = cg.current().1.loops[index].pc2;
        let chained = cg.current().1.genjmp(opcode::OP_JMPUW, pc2)?;
        cg.current().1.loops[index].pc2 = chained;
    } else {
        if !has_value {
            let (session, scope) = cg.current();
            let dst = scope.cursp();
            scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
        }
        let (session, scope) = cg.current();
        let src = scope.cursp();
        scope.gen_return(session, opcode::OP_BREAK, src)?;
    }
    Ok(())
}

/// `break` (`PM_BREAK_NODE`).
fn gen_break<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(args) = node.break_args() else {
        return Err(unsupported(&node, "break"));
    };
    loop_break(cg, args, &node)?;
    if val {
        cg.current().1.push_n(1)?;
    }
    Ok(())
}

/// `next` (`PM_NEXT_NODE`): `JMPUW` to the loop head for `LOOP_NORMAL`
/// (the operand codes `NOVAL`), else the value returns (`OP_RETURN`) for
/// block-style frames.
fn gen_next<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(args) = node.next_args() else {
        return Err(unsupported(&node, "next"));
    };
    let Some(index) = jump_target(cg) else {
        return Err(unexpected(&node, "unexpected next"));
    };
    let kind = cg.current().1.loops[index].kind;
    if kind == LoopType::Normal {
        if let Some(arguments) = args {
            codegen(cg, arguments, false)?;
        }
        let pc0 = cg.current().1.loops[index].pc0;
        cg.current().1.genjmp(opcode::OP_JMPUW, pc0)?;
    } else {
        match args {
            Some(arguments) => gen_retval(cg, arguments)?,
            None => {
                let (session, scope) = cg.current();
                let dst = scope.cursp();
                scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
            }
        }
        let (session, scope) = cg.current();
        let src = scope.cursp();
        scope.gen_return(session, opcode::OP_RETURN, src)?;
    }
    if val {
        cg.current().1.push_n(1)?;
    }
    Ok(())
}

/// `redo` (`PM_REDO_NODE`): `JMPUW` to the loop's redo label.
fn gen_redo<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(index) = jump_target(cg) else {
        return Err(unexpected(&node, "unexpected redo"));
    };
    let pc1 = cg.current().1.loops[index].pc1;
    cg.current().1.genjmp(opcode::OP_JMPUW, pc1)?;
    if val {
        cg.current().1.push_n(1)?;
    }
    Ok(())
}

/// Range literal (`PM_RANGE_NODE`): both sides code in order (a missing
/// side, endless `1..` or beginless `..3`, codes nil like any null subtree),
/// then `RANGE_INC`/`RANGE_EXC` gathers them.
fn gen_range<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(view) = node.range_view() else {
        return Err(unsupported(&node, "range"));
    };
    match view.left {
        Some(left) => codegen(cg, left, val)?,
        None => {
            if val {
                let (session, scope) = cg.current();
                let dst = scope.cursp();
                scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
                scope.push_n(1)?;
            }
        }
    }
    match view.right {
        Some(right) => codegen(cg, right, val)?,
        None => {
            if val {
                let (session, scope) = cg.current();
                let dst = scope.cursp();
                scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
                scope.push_n(1)?;
            }
        }
    }
    if val {
        let op = if view.exclude_end {
            opcode::OP_RANGE_EXC
        } else {
            opcode::OP_RANGE_INC
        };
        cg.current().1.pop_n(2)?;
        let (session, scope) = cg.current();
        let dst = scope.cursp();
        scope.genop_1(session, op, dst)?;
        scope.push_n(1)?;
    }
    Ok(())
}

/// Parenthesized expression (`PM_PARENTHESES_NODE`): a transparent value
/// passthrough; an empty `()` is a nil literal when valued.
fn gen_parentheses<N: BackendNode>(cg: &mut Codegen, node: N, val: bool) -> Result<(), Diagnostic> {
    let Some(body) = node.parentheses_body() else {
        return Err(unsupported(&node, "parentheses"));
    };
    match body {
        Some(inner) => codegen(cg, inner, val),
        None => {
            if val {
                let (session, scope) = cg.current();
                let dst = scope.cursp();
                scope.genop_1(session, opcode::OP_LOADNIL, dst)?;
                scope.push_n(1)?;
            }
            Ok(())
        }
    }
}

/// Compile a parsed program tree to a RITE binary (any `BackendNode`).
///
/// `stripped=false` emits a real `DBG` section (filename table plus packed
/// line maps, parent and child ireps); `stripped=true` omits it. The
/// reference still dumps with flags `0`, so `verify` compares
/// debug-stripped bytes (see `without_debug`).
pub fn compile_tree<N: BackendNode>(
    root: N,
    opts: &CompileOptions,
) -> Result<Vec<u8>, Diagnostics> {
    compile_tree_impl(root, opts, None)
}

/// Compile with source for line mapping; lines land in the `DBG` section.
/// `filename` in `opts` names the debug file entry (default `"-e"`, like
/// `mrc_load_string_cxt`).
pub fn compile_tree_with_source<N: BackendNode>(
    root: N,
    opts: &CompileOptions,
    source: &[u8],
) -> Result<Vec<u8>, Diagnostics> {
    compile_tree_impl(root, opts, Some(source))
}

fn compile_tree_impl<N: BackendNode>(
    root: N,
    opts: &CompileOptions,
    source: Option<&[u8]>,
) -> Result<Vec<u8>, Diagnostics> {
    let mut cg = Codegen::new();
    let filename = opts.filename.clone().unwrap_or_else(|| "-e".to_owned());
    cg.set_filename(filename.as_bytes());
    if let Some(source) = source {
        cg.set_source(source);
    }
    codegen(&mut cg, root, true).map_err(|single| Diagnostics {
        entries: vec![single],
    })?;
    let Some(mut irep) = cg.root.take() else {
        return Err(Diagnostics {
            entries: vec![Diagnostic {
                message: "expected a program root".to_owned(),
                start: 0,
                end: 0,
            }],
        });
    };
    if opts.stripped {
        crate::debug::clear_debug(&mut irep);
    }
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
