//! Pure owned access: `BackendNode` over borrowed owned trees (P3.2).
//!
//! Shipping-safe (no FFI, no `unsafe`). Children are plain reborrows, so
//! the wrapper stays `Clone`/`Copy` like any other cheap node handle.

use crate::view::{
    AlternationView, ArrayPatternView, BackendNode, BeginView, BlockParamView, BlockView,
    CallTargetView, CallView, CaptureView, CaseMatchView, CaseView, ClassView, ConstPathRead,
    ConstPathWrite, DefView, EnsureView, FindPatternView, ForView, GuardView, HashPatternView,
    IfView, InView, IndexTargetView, IntegerLit, KeywordParamView, LambdaView, LvarRef, LvarWrite,
    MatchView, ModuleView, MultiTargetView, MultiWriteView, ParamsView, ProgramView, RangeView,
    RescueModifierView, RescueView, SclassView, SimpleLit, SuperView, VarWrite, WhenView,
    WhileView, YieldView,
};
use crate::{
    arguments_node_flags, call_node_flags, loop_flags, range_flags, AstNode, Integer, Node, Span,
    SymbolId, SymbolPool, NIL_BLOCK,
};

/// Borrowed owned tree plus its symbol pool.
#[derive(Debug, Clone, Copy)]
pub struct Owned<'a> {
    /// Current node.
    pub node: &'a Node,
    /// Pool backing every `SymbolId` in the tree.
    pub pool: &'a SymbolPool,
}

/// Decimal digits of an overflow literal: same canonical digits as the
/// shared `crate::limbs_to_decimal`, reached from decimal fallback text
/// instead of binary limbs.
fn bigint_from_raw(raw: &[u8]) -> Option<IntegerLit> {
    let mut bytes: Vec<u8> = raw.iter().copied().filter(|b| *b != b'_').collect();
    // Small values stay `I64` even when lowered as fallback text.
    if let Ok(text) = core::str::from_utf8(&bytes) {
        if let Ok(value) = text.parse::<i64>() {
            return Some(IntegerLit::I64(value));
        }
    }
    let negative = bytes.first() == Some(&b'-');
    if bytes.first() == Some(&b'-') || bytes.first() == Some(&b'+') {
        bytes.remove(0);
    }
    if bytes.is_empty() || !bytes.iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut start = 0;
    while start < bytes.len() && bytes[start] == b'0' {
        start += 1;
    }
    let mut digits = bytes[start..].to_vec();
    if digits.is_empty() {
        digits = vec![b'0'];
    }
    let negative = negative && digits != vec![b'0'];
    Some(IntegerLit::Bigint { digits, negative })
}

impl<'a> Owned<'a> {
    /// Reborrow a child node with the same pool.
    fn child(&self, node: &'a Node) -> Owned<'a> {
        Owned {
            node,
            pool: self.pool,
        }
    }

    /// Reborrow an optional boxed child.
    fn opt_child(&self, node: &'a Option<Box<Node>>) -> Option<Owned<'a>> {
        node.as_deref().map(|inner| self.child(inner))
    }

    /// Reborrow a child list.
    fn vec_children(&self, nodes: &'a [Node]) -> Vec<Owned<'a>> {
        nodes.iter().map(|node| self.child(node)).collect()
    }

    /// Statements body (`None` for a null subtree, never for other shapes).
    fn stmts(&self, node: &'a Option<Box<Node>>) -> Option<Vec<Owned<'a>>> {
        match node.as_deref()? {
            Node::StatementsNode { body, .. } => Some(self.vec_children(body)),
            _ => None,
        }
    }

    /// Resolve a required name (missing pool entry fails closed).
    fn name(&self, id: SymbolId) -> Option<Vec<u8>> {
        self.pool.lookup(id).map(|bytes| bytes.to_vec())
    }

    /// Resolve an optional name (`None` is an anonymous splat/block form).
    fn opt_name(&self, id: Option<SymbolId>) -> Option<Option<Vec<u8>>> {
        match id {
            None => Some(None),
            Some(id) => self.name(id).map(Some),
        }
    }

    /// Resolve a locals list (any missing entry fails closed).
    fn locals(&self, ids: &[SymbolId]) -> Option<Vec<Vec<u8>>> {
        ids.iter()
            .map(|id| self.pool.lookup(*id).map(|bytes| bytes.to_vec()))
            .collect()
    }

    /// First statement of a guard wrapper body (`None` for a null or
    /// non-statements subtree).
    fn guard_inner(&self, node: &'a Option<Box<Node>>) -> Option<Owned<'a>> {
        match node.as_deref()? {
            Node::StatementsNode { body, .. } => body.first().map(|inner| self.child(inner)),
            _ => None,
        }
    }
}

impl AstNode for Owned<'_> {
    fn kind_name(&self) -> &'static str {
        self.node.kind_name()
    }

    fn span(&self) -> Span {
        self.node.span()
    }

    fn flags(&self) -> u16 {
        self.node.flags()
    }
}

impl BackendNode for Owned<'_> {
    fn integer_lit(&self) -> Option<IntegerLit> {
        match &self.node {
            Node::IntegerNode { value, .. } => match value {
                Integer::I64(value) => Some(IntegerLit::I64(*value)),
                Integer::Fallback { raw } => bigint_from_raw(raw),
            },
            _ => None,
        }
    }

    fn float_lit(&self) -> Option<f64> {
        match &self.node {
            Node::FloatNode { value, .. } => Some(*value),
            _ => None,
        }
    }

    fn string_lit(&self) -> Option<Vec<u8>> {
        match &self.node {
            Node::StringNode { unescaped, .. } => Some(unescaped.clone()),
            _ => None,
        }
    }

    fn symbol_lit(&self) -> Option<Vec<u8>> {
        match &self.node {
            Node::SymbolNode { unescaped, .. } => Some(unescaped.clone()),
            _ => None,
        }
    }

    fn call(&self) -> Option<CallView<Self>> {
        match &self.node {
            Node::CallNode {
                receiver,
                name,
                arguments,
                block,
                flags,
                ..
            } => Some(CallView {
                name: self.name(*name)?,
                receiver: self.opt_child(receiver),
                args: self.opt_child(arguments),
                block: self.opt_child(block),
                safe_nav: flags & call_node_flags::SAFE_NAVIGATION != 0,
                attr_write: flags & call_node_flags::ATTRIBUTE_WRITE != 0,
            }),
            _ => None,
        }
    }

    fn call_args(&self) -> Option<Vec<Self>> {
        match &self.node {
            Node::ArgumentsNode { arguments, .. } => Some(self.vec_children(arguments)),
            _ => None,
        }
    }

    fn args_forwarding(&self) -> bool {
        match &self.node {
            Node::ArgumentsNode { flags, .. } => {
                flags & arguments_node_flags::CONTAINS_FORWARDING != 0
            }
            _ => false,
        }
    }

    fn if_branch(&self) -> Option<IfView<Self>> {
        match &self.node {
            Node::IfNode {
                predicate,
                statements,
                subsequent,
                ..
            } => Some(IfView {
                predicate: Some(self.child(predicate)),
                then_body: self.stmts(statements),
                else_body: self.opt_child(subsequent),
                is_unless: false,
            }),
            Node::UnlessNode {
                predicate,
                statements,
                else_clause,
                ..
            } => Some(IfView {
                predicate: Some(self.child(predicate)),
                then_body: else_clause.as_deref().map(|clause| match clause {
                    Node::ElseNode { statements, .. } => self.stmts(statements).unwrap_or_default(),
                    _ => Vec::new(),
                }),
                else_body: self.opt_child(statements),
                is_unless: true,
            }),
            _ => None,
        }
    }

    fn array_elements(&self) -> Option<Vec<Self>> {
        match &self.node {
            Node::ArrayNode { elements, .. } => Some(self.vec_children(elements)),
            _ => None,
        }
    }

    fn while_loop(&self) -> Option<WhileView<Self>> {
        match &self.node {
            Node::WhileNode {
                predicate,
                statements,
                flags,
                ..
            } => Some(WhileView {
                predicate: Some(self.child(predicate)),
                body: self.stmts(statements),
                is_until: false,
                begin_modifier: flags & loop_flags::BEGIN_MODIFIER != 0,
            }),
            Node::UntilNode {
                predicate,
                statements,
                flags,
                ..
            } => Some(WhileView {
                predicate: Some(self.child(predicate)),
                body: self.stmts(statements),
                is_until: true,
                begin_modifier: flags & loop_flags::BEGIN_MODIFIER != 0,
            }),
            _ => None,
        }
    }

    fn for_view(&self) -> Option<ForView<Self>> {
        match &self.node {
            Node::ForNode {
                index,
                collection,
                statements,
                ..
            } => Some(ForView {
                index: self.child(index),
                collection: self.child(collection),
                statements: self.stmts(statements),
            }),
            _ => None,
        }
    }

    fn statements(&self) -> Option<Vec<Self>> {
        match &self.node {
            Node::StatementsNode { body, .. } => Some(self.vec_children(body)),
            _ => None,
        }
    }

    fn logic(&self) -> Option<(Self, Self)> {
        match &self.node {
            Node::AndNode { left, right, .. } | Node::OrNode { left, right, .. } => {
                Some((self.child(left), self.child(right)))
            }
            _ => None,
        }
    }

    fn else_body(&self) -> Option<Vec<Self>> {
        match &self.node {
            Node::ElseNode { statements, .. } => self.stmts(statements),
            _ => None,
        }
    }

    fn program(&self) -> Option<ProgramView<Self>> {
        match &self.node {
            Node::ProgramNode {
                locals, statements, ..
            } => Some(ProgramView {
                locals: self.locals(locals)?,
                body: self.child(statements),
            }),
            _ => None,
        }
    }

    fn lvar_read(&self) -> Option<LvarRef> {
        match &self.node {
            Node::LocalVariableReadNode { name, depth, .. } => Some(LvarRef {
                name: self.name(*name)?,
                depth: *depth,
            }),
            _ => None,
        }
    }

    fn lvar_write(&self) -> Option<LvarWrite<Self>> {
        match &self.node {
            Node::LocalVariableWriteNode {
                name, depth, value, ..
            } => Some(LvarWrite {
                name: self.name(*name)?,
                depth: *depth,
                value: Some(self.child(value)),
            }),
            _ => None,
        }
    }

    fn simple_lit(&self) -> Option<SimpleLit> {
        match &self.node {
            Node::TrueNode { .. } => Some(SimpleLit::True),
            Node::FalseNode { .. } => Some(SimpleLit::False),
            Node::NilNode { .. } => Some(SimpleLit::Nil),
            Node::SelfNode { .. } => Some(SimpleLit::SelfValue),
            _ => None,
        }
    }

    fn hash_elements(&self) -> Option<Vec<Self>> {
        match &self.node {
            Node::HashNode { elements, .. } | Node::KeywordHashNode { elements, .. } => {
                Some(self.vec_children(elements))
            }
            _ => None,
        }
    }

    fn assoc_pair(&self) -> Option<(Self, Self)> {
        match &self.node {
            Node::AssocNode { key, value, .. } => Some((self.child(key), self.child(value))),
            _ => None,
        }
    }

    fn assoc_splat_value(&self) -> Option<Option<Self>> {
        match &self.node {
            Node::AssocSplatNode { value, .. } => Some(self.opt_child(value)),
            _ => None,
        }
    }

    fn splat_value(&self) -> Option<Option<Self>> {
        match &self.node {
            Node::SplatNode { expression, .. } => Some(self.opt_child(expression)),
            _ => None,
        }
    }

    fn case_view(&self) -> Option<CaseView<Self>> {
        match &self.node {
            Node::CaseNode {
                predicate,
                conditions,
                else_clause,
                ..
            } => Some(CaseView {
                predicate: self.opt_child(predicate),
                whens: self.vec_children(conditions),
                else_body: self.opt_child(else_clause),
            }),
            _ => None,
        }
    }

    fn when_view(&self) -> Option<WhenView<Self>> {
        match &self.node {
            Node::WhenNode {
                conditions,
                statements,
                ..
            } => Some(WhenView {
                conditions: self.vec_children(conditions),
                body: self.stmts(statements),
            }),
            _ => None,
        }
    }

    fn case_match_view(&self) -> Option<CaseMatchView<Self>> {
        match &self.node {
            Node::CaseMatchNode {
                predicate,
                conditions,
                else_clause,
                ..
            } => Some(CaseMatchView {
                predicate: self.opt_child(predicate),
                conditions: self.vec_children(conditions),
                else_body: self.opt_child(else_clause),
            }),
            _ => None,
        }
    }

    fn in_view(&self) -> Option<InView<Self>> {
        match &self.node {
            Node::InNode {
                pattern,
                statements,
                ..
            } => Some(InView {
                pattern: self.child(pattern),
                body: self.stmts(statements),
            }),
            _ => None,
        }
    }

    fn match_predicate_view(&self) -> Option<MatchView<Self>> {
        match &self.node {
            Node::MatchPredicateNode { value, pattern, .. } => Some(MatchView {
                value: self.child(value),
                pattern: self.child(pattern),
            }),
            _ => None,
        }
    }

    fn match_required_view(&self) -> Option<MatchView<Self>> {
        match &self.node {
            Node::MatchRequiredNode { value, pattern, .. } => Some(MatchView {
                value: self.child(value),
                pattern: self.child(pattern),
            }),
            _ => None,
        }
    }

    fn alternation_view(&self) -> Option<AlternationView<Self>> {
        match &self.node {
            Node::AlternationPatternNode { left, right, .. } => Some(AlternationView {
                left: self.child(left),
                right: self.child(right),
            }),
            _ => None,
        }
    }

    fn capture_view(&self) -> Option<CaptureView<Self>> {
        match &self.node {
            Node::CapturePatternNode { value, target, .. } => Some(CaptureView {
                value: self.child(value),
                target: self.child(target),
            }),
            _ => None,
        }
    }

    fn array_pattern_view(&self) -> Option<ArrayPatternView<Self>> {
        match &self.node {
            Node::ArrayPatternNode {
                constant,
                requireds,
                rest,
                posts,
                ..
            } => Some(ArrayPatternView {
                constant: self.opt_child(constant),
                requireds: self.vec_children(requireds),
                posts: self.vec_children(posts),
                rest: self.opt_child(rest),
            }),
            _ => None,
        }
    }

    fn hash_pattern_view(&self) -> Option<HashPatternView<Self>> {
        match &self.node {
            Node::HashPatternNode {
                constant,
                elements,
                rest,
                ..
            } => Some(HashPatternView {
                constant: self.opt_child(constant),
                elements: self.vec_children(elements),
                rest: self.opt_child(rest),
            }),
            _ => None,
        }
    }

    fn find_pattern_view(&self) -> Option<FindPatternView<Self>> {
        match &self.node {
            Node::FindPatternNode {
                constant,
                left,
                requireds,
                right,
                ..
            } => Some(FindPatternView {
                constant: self.opt_child(constant),
                left: self.child(left),
                requireds: self.vec_children(requireds),
                right: self.child(right),
            }),
            _ => None,
        }
    }

    fn pinned_var(&self) -> Option<Self> {
        match &self.node {
            Node::PinnedVariableNode { variable, .. } => Some(self.child(variable)),
            _ => None,
        }
    }

    fn pinned_expr(&self) -> Option<Self> {
        match &self.node {
            Node::PinnedExpressionNode { expression, .. } => Some(self.child(expression)),
            _ => None,
        }
    }

    fn guard_view(&self) -> Option<GuardView<Self>> {
        match &self.node {
            Node::IfNode {
                predicate,
                statements,
                ..
            } => Some(GuardView {
                inner: self.guard_inner(statements)?,
                condition: self.child(predicate),
                is_unless: false,
            }),
            Node::UnlessNode {
                predicate,
                statements,
                ..
            } => Some(GuardView {
                inner: self.guard_inner(statements)?,
                condition: self.child(predicate),
                is_unless: true,
            }),
            _ => None,
        }
    }

    fn string_parts(&self) -> Option<Vec<Self>> {
        match &self.node {
            Node::InterpolatedStringNode { parts, .. } => Some(self.vec_children(parts)),
            _ => None,
        }
    }

    fn embedded_body(&self) -> Option<Vec<Self>> {
        match &self.node {
            Node::EmbeddedStatementsNode { statements, .. } => self.stmts(statements),
            _ => None,
        }
    }

    fn embedded_var(&self) -> Option<Self> {
        match &self.node {
            Node::EmbeddedVariableNode { variable, .. } => Some(self.child(variable)),
            _ => None,
        }
    }

    fn block_view(&self) -> Option<BlockView<Self>> {
        match &self.node {
            Node::BlockNode {
                locals,
                parameters,
                body,
                ..
            } => Some(BlockView {
                locals: self.locals(locals)?,
                params: self.opt_child(parameters),
                body: self.opt_child(body),
            }),
            _ => None,
        }
    }

    fn lambda_view(&self) -> Option<LambdaView<Self>> {
        match &self.node {
            Node::LambdaNode {
                locals,
                parameters,
                body,
                ..
            } => Some(LambdaView {
                locals: self.locals(locals)?,
                params: self.opt_child(parameters),
                body: self.opt_child(body),
            }),
            _ => None,
        }
    }

    fn yield_view(&self) -> Option<YieldView<Self>> {
        match &self.node {
            Node::YieldNode { arguments, .. } => Some(YieldView {
                args: self.opt_child(arguments),
            }),
            _ => None,
        }
    }

    fn block_param_view(&self) -> Option<BlockParamView<Self>> {
        match &self.node {
            Node::BlockParametersNode {
                parameters, locals, ..
            } => Some(BlockParamView {
                params: self.opt_child(parameters),
                block_locals: self.vec_children(locals),
            }),
            _ => None,
        }
    }

    fn parameters_view(&self) -> Option<ParamsView<Self>> {
        match &self.node {
            Node::ParametersNode {
                requireds,
                optionals,
                rest,
                posts,
                keywords,
                keyword_rest,
                block,
                ..
            } => Some(ParamsView {
                requireds: self.vec_children(requireds),
                optionals: self.vec_children(optionals),
                rest: self.opt_child(rest),
                posts: self.vec_children(posts),
                keywords: self.vec_children(keywords),
                keyword_rest: self.opt_child(keyword_rest),
                block: self.opt_child(block),
            }),
            _ => None,
        }
    }

    fn required_param_name(&self) -> Option<Vec<u8>> {
        match &self.node {
            Node::RequiredParameterNode { name, .. } => self.name(*name),
            _ => None,
        }
    }

    fn rest_param_name(&self) -> Option<Option<Vec<u8>>> {
        match &self.node {
            Node::RestParameterNode { name, .. } => Some(self.opt_name(*name)?),
            _ => None,
        }
    }

    fn block_local_name(&self) -> Option<Vec<u8>> {
        match &self.node {
            Node::BlockLocalVariableNode { name, .. } => self.name(*name),
            _ => None,
        }
    }

    fn numbered_max(&self) -> Option<u8> {
        match &self.node {
            Node::NumberedParametersNode { maximum, .. } => Some(*maximum),
            _ => None,
        }
    }

    fn block_arg(&self) -> Option<Option<Self>> {
        match &self.node {
            Node::BlockArgumentNode { expression, .. } => Some(self.opt_child(expression)),
            _ => None,
        }
    }

    fn it_read(&self) -> Option<()> {
        match &self.node {
            Node::ItLocalVariableReadNode { .. } => Some(()),
            _ => None,
        }
    }

    fn def_view(&self) -> Option<DefView<Self>> {
        match &self.node {
            Node::DefNode {
                name,
                receiver,
                parameters,
                body,
                locals,
                ..
            } => Some(DefView {
                name: self.name(*name)?,
                receiver: self.opt_child(receiver),
                params: self.opt_child(parameters),
                body: self.opt_child(body),
                locals: self.locals(locals)?,
            }),
            _ => None,
        }
    }

    fn optional_param(&self) -> Option<(Vec<u8>, Self)> {
        match &self.node {
            Node::OptionalParameterNode { name, value, .. } => {
                Some((self.name(*name)?, self.child(value)))
            }
            _ => None,
        }
    }

    fn keyword_param(&self) -> Option<KeywordParamView<Self>> {
        match &self.node {
            Node::RequiredKeywordParameterNode { name, .. } => Some(KeywordParamView {
                name: self.name(*name)?,
                default: None,
            }),
            Node::OptionalKeywordParameterNode { name, value, .. } => Some(KeywordParamView {
                name: self.name(*name)?,
                default: Some(self.child(value)),
            }),
            _ => None,
        }
    }

    fn keyword_rest_name(&self) -> Option<Option<Vec<u8>>> {
        match &self.node {
            Node::KeywordRestParameterNode { name, .. } => Some(self.opt_name(*name)?),
            _ => None,
        }
    }

    fn block_param_name(&self) -> Option<Option<Vec<u8>>> {
        match &self.node {
            Node::BlockParameterNode { name, .. } => Some(self.opt_name(*name)?),
            _ => None,
        }
    }

    fn block_param_noblock(&self) -> bool {
        match &self.node {
            Node::BlockParameterNode { name, flags, .. } => {
                (u32::from(*flags) & NIL_BLOCK) != 0
                    || name.and_then(|id| self.pool.lookup(id)) == Some(b"nil".as_slice())
            }
            _ => false,
        }
    }

    fn class_view(&self) -> Option<ClassView<Self>> {
        match &self.node {
            Node::ClassNode {
                locals,
                constant_path,
                superclass,
                body,
                name,
                ..
            } => {
                let (cpath_is_read, cpath_parent) = match constant_path.as_ref() {
                    Node::ConstantReadNode { .. } => (true, None),
                    Node::ConstantPathNode { parent, name, .. } => {
                        name.as_ref()?;
                        (false, parent.as_deref().map(|inner| self.child(inner)))
                    }
                    _ => return None,
                };
                Some(ClassView {
                    name: self.name(*name)?,
                    cpath_is_read,
                    cpath_parent,
                    superclass: self.opt_child(superclass),
                    body: self.opt_child(body),
                    locals: self.locals(locals)?,
                })
            }
            _ => None,
        }
    }

    fn module_view(&self) -> Option<ModuleView<Self>> {
        match &self.node {
            Node::ModuleNode {
                locals,
                constant_path,
                body,
                name,
                ..
            } => {
                let (cpath_is_read, cpath_parent) = match constant_path.as_ref() {
                    Node::ConstantReadNode { .. } => (true, None),
                    Node::ConstantPathNode { parent, name, .. } => {
                        name.as_ref()?;
                        (false, parent.as_deref().map(|inner| self.child(inner)))
                    }
                    _ => return None,
                };
                Some(ModuleView {
                    name: self.name(*name)?,
                    cpath_is_read,
                    cpath_parent,
                    body: self.opt_child(body),
                    locals: self.locals(locals)?,
                })
            }
            _ => None,
        }
    }

    fn sclass_view(&self) -> Option<SclassView<Self>> {
        match &self.node {
            Node::SingletonClassNode {
                locals,
                expression,
                body,
                ..
            } => Some(SclassView {
                expression: self.child(expression),
                body: self.opt_child(body),
                locals: self.locals(locals)?,
            }),
            _ => None,
        }
    }

    fn const_read(&self) -> Option<Vec<u8>> {
        match &self.node {
            Node::ConstantReadNode { name, .. } => self.name(*name),
            _ => None,
        }
    }

    fn const_write(&self) -> Option<VarWrite<Self>> {
        match &self.node {
            Node::ConstantWriteNode { name, value, .. } => Some(VarWrite {
                name: self.name(*name)?,
                value: self.child(value),
            }),
            _ => None,
        }
    }

    fn const_path(&self) -> Option<ConstPathRead<Self>> {
        match &self.node {
            Node::ConstantPathNode { parent, name, .. } => Some(ConstPathRead {
                parent: self.opt_child(parent),
                name: self.name((*name)?)?,
            }),
            _ => None,
        }
    }

    fn const_path_write(&self) -> Option<ConstPathWrite<Self>> {
        // Like Prism itself, the write target is a plain `ConstantPathNode`.
        match &self.node {
            Node::ConstantPathWriteNode { target, value, .. } => match target.as_ref() {
                Node::ConstantPathNode { parent, name, .. } => Some(ConstPathWrite {
                    parent: self.opt_child(parent),
                    name: self.name((*name)?)?,
                    value: self.child(value),
                }),
                _ => None,
            },
            _ => None,
        }
    }

    fn ivar_read(&self) -> Option<Vec<u8>> {
        match &self.node {
            Node::InstanceVariableReadNode { name, .. } => self.name(*name),
            _ => None,
        }
    }

    fn ivar_write(&self) -> Option<VarWrite<Self>> {
        match &self.node {
            Node::InstanceVariableWriteNode { name, value, .. } => Some(VarWrite {
                name: self.name(*name)?,
                value: self.child(value),
            }),
            _ => None,
        }
    }

    fn cvar_read(&self) -> Option<Vec<u8>> {
        match &self.node {
            Node::ClassVariableReadNode { name, .. } => self.name(*name),
            _ => None,
        }
    }

    fn cvar_write(&self) -> Option<VarWrite<Self>> {
        match &self.node {
            Node::ClassVariableWriteNode { name, value, .. } => Some(VarWrite {
                name: self.name(*name)?,
                value: self.child(value),
            }),
            _ => None,
        }
    }

    fn gvar_read(&self) -> Option<Vec<u8>> {
        match &self.node {
            Node::GlobalVariableReadNode { name, .. } => self.name(*name),
            _ => None,
        }
    }

    fn gvar_write(&self) -> Option<VarWrite<Self>> {
        match &self.node {
            Node::GlobalVariableWriteNode { name, value, .. } => Some(VarWrite {
                name: self.name(*name)?,
                value: self.child(value),
            }),
            _ => None,
        }
    }

    fn super_view(&self) -> Option<SuperView<Self>> {
        match &self.node {
            Node::SuperNode {
                arguments, block, ..
            } => {
                if block.is_some() {
                    return None;
                }
                let Some(inner) = arguments.as_deref() else {
                    return Some(SuperView { args: None });
                };
                match inner {
                    Node::ArgumentsNode { arguments, .. } => {
                        let mut out = Vec::new();
                        for argument in arguments {
                            let child = self.child(argument);
                            if matches!(
                                child.node,
                                Node::SplatNode { .. } | Node::KeywordHashNode { .. }
                            ) {
                                return None;
                            }
                            out.push(child);
                        }
                        Some(SuperView { args: Some(out) })
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn forwarding_super(&self) -> Option<Option<Self>> {
        match &self.node {
            Node::ForwardingSuperNode { block, .. } => Some(self.opt_child(block)),
            _ => None,
        }
    }

    fn lvar_target(&self) -> Option<LvarRef> {
        match &self.node {
            Node::LocalVariableTargetNode { name, depth, .. } => Some(LvarRef {
                name: self.name(*name)?,
                depth: *depth,
            }),
            _ => None,
        }
    }

    fn begin_view(&self) -> Option<BeginView<Self>> {
        match &self.node {
            Node::BeginNode {
                statements,
                rescue_clause,
                else_clause,
                ensure_clause,
                ..
            } => Some(BeginView {
                statements: self.stmts(statements),
                bare: rescue_clause.is_none() && else_clause.is_none() && ensure_clause.is_none(),
                rescue_clause: self.opt_child(rescue_clause),
                else_clause: self.opt_child(else_clause),
                ensure_clause: self.opt_child(ensure_clause),
            }),
            _ => None,
        }
    }

    fn rescue_view(&self) -> Option<RescueView<Self>> {
        match &self.node {
            Node::RescueNode {
                exceptions,
                reference,
                statements,
                subsequent,
                ..
            } => Some(RescueView {
                exceptions: self.vec_children(exceptions),
                reference: self.opt_child(reference),
                statements: self.stmts(statements),
                subsequent: self.opt_child(subsequent),
            }),
            _ => None,
        }
    }

    fn rescue_modifier_view(&self) -> Option<RescueModifierView<Self>> {
        match &self.node {
            Node::RescueModifierNode {
                expression,
                rescue_expression,
                ..
            } => Some(RescueModifierView {
                expression: self.child(expression),
                rescue_expression: self.child(rescue_expression),
            }),
            _ => None,
        }
    }

    fn ensure_view(&self) -> Option<EnsureView<Self>> {
        match &self.node {
            Node::EnsureNode { statements, .. } => Some(EnsureView {
                statements: self.stmts(statements),
            }),
            _ => None,
        }
    }

    fn multi_write_view(&self) -> Option<MultiWriteView<Self>> {
        match &self.node {
            Node::MultiWriteNode {
                lefts,
                rest,
                rights,
                value,
                ..
            } => Some(MultiWriteView {
                lefts: self.vec_children(lefts),
                rest: self.opt_child(rest),
                rights: self.vec_children(rights),
                value: self.child(value),
            }),
            _ => None,
        }
    }

    fn multi_target_view(&self) -> Option<MultiTargetView<Self>> {
        match &self.node {
            Node::MultiTargetNode {
                lefts,
                rest,
                rights,
                ..
            } => Some(MultiTargetView {
                lefts: self.vec_children(lefts),
                rest: self.opt_child(rest),
                rights: self.vec_children(rights),
            }),
            _ => None,
        }
    }

    fn ivar_target_name(&self) -> Option<Vec<u8>> {
        match &self.node {
            Node::InstanceVariableTargetNode { name, .. } => self.name(*name),
            _ => None,
        }
    }

    fn cvar_target_name(&self) -> Option<Vec<u8>> {
        match &self.node {
            Node::ClassVariableTargetNode { name, .. } => self.name(*name),
            _ => None,
        }
    }

    fn gvar_target_name(&self) -> Option<Vec<u8>> {
        match &self.node {
            Node::GlobalVariableTargetNode { name, .. } => self.name(*name),
            _ => None,
        }
    }

    fn const_target_name(&self) -> Option<Vec<u8>> {
        match &self.node {
            Node::ConstantTargetNode { name, .. } => self.name(*name),
            _ => None,
        }
    }

    fn const_path_target(&self) -> Option<(Option<Self>, Vec<u8>)> {
        match &self.node {
            Node::ConstantPathTargetNode { parent, name, .. } => {
                Some((self.opt_child(parent), self.name((*name)?)?))
            }
            _ => None,
        }
    }

    fn index_target(&self) -> Option<IndexTargetView<Self>> {
        match &self.node {
            Node::IndexTargetNode {
                receiver,
                arguments,
                ..
            } => Some(IndexTargetView {
                receiver: self.child(receiver),
                args: self.opt_child(arguments),
            }),
            _ => None,
        }
    }

    fn call_target(&self) -> Option<CallTargetView<Self>> {
        match &self.node {
            Node::CallTargetNode { receiver, name, .. } => Some(CallTargetView {
                receiver: self.child(receiver),
                name: self.name(*name)?,
            }),
            _ => None,
        }
    }

    fn alias_pair(&self) -> Option<(Self, Self)> {
        match &self.node {
            Node::AliasMethodNode {
                new_name, old_name, ..
            } => Some((self.child(new_name), self.child(old_name))),
            _ => None,
        }
    }

    fn undef_list(&self) -> Option<Vec<Self>> {
        match &self.node {
            Node::UndefNode { names, .. } => Some(self.vec_children(names)),
            _ => None,
        }
    }

    fn defined_value(&self) -> Option<Self> {
        match &self.node {
            Node::DefinedNode { value, .. } => Some(self.child(value)),
            _ => None,
        }
    }

    fn implicit_value(&self) -> Option<Self> {
        match &self.node {
            Node::ImplicitNode { value, .. } => Some(self.child(value)),
            _ => None,
        }
    }

    fn return_args(&self) -> Option<Option<Self>> {
        match &self.node {
            Node::ReturnNode { arguments, .. } => Some(self.opt_child(arguments)),
            _ => None,
        }
    }

    fn break_args(&self) -> Option<Option<Self>> {
        match &self.node {
            Node::BreakNode { arguments, .. } => Some(self.opt_child(arguments)),
            _ => None,
        }
    }

    fn next_args(&self) -> Option<Option<Self>> {
        match &self.node {
            Node::NextNode { arguments, .. } => Some(self.opt_child(arguments)),
            _ => None,
        }
    }

    fn range_view(&self) -> Option<RangeView<Self>> {
        match &self.node {
            Node::RangeNode {
                left, right, flags, ..
            } => Some(RangeView {
                left: self.opt_child(left),
                right: self.opt_child(right),
                exclude_end: flags & range_flags::EXCLUDE_END != 0,
            }),
            _ => None,
        }
    }

    fn parentheses_body(&self) -> Option<Option<Self>> {
        match &self.node {
            Node::ParenthesesNode { body, .. } => Some(self.opt_child(body)),
            _ => None,
        }
    }

    fn instance_var_read_name(&self) -> Option<Vec<u8>> {
        match &self.node {
            Node::InstanceVariableReadNode { name, .. } => self.name(*name),
            _ => None,
        }
    }

    fn backref_name(&self) -> Option<Vec<u8>> {
        match &self.node {
            Node::BackReferenceReadNode { name, .. } => self.name(*name),
            _ => None,
        }
    }

    fn numbered_ref_number(&self) -> Option<u32> {
        match &self.node {
            Node::NumberedReferenceReadNode { number, .. } => Some(*number),
            _ => None,
        }
    }

    fn global_var_read_name(&self) -> Option<Vec<u8>> {
        match &self.node {
            Node::GlobalVariableReadNode { name, .. } => self.name(*name),
            _ => None,
        }
    }

    fn class_var_read_name(&self) -> Option<Vec<u8>> {
        match &self.node {
            Node::ClassVariableReadNode { name, .. } => self.name(*name),
            _ => None,
        }
    }

    fn constant_read_name(&self) -> Option<Vec<u8>> {
        match &self.node {
            Node::ConstantReadNode { name, .. } => self.name(*name),
            _ => None,
        }
    }

    fn constant_path_parts(&self) -> Option<(Option<Self>, Vec<u8>)> {
        match &self.node {
            Node::ConstantPathNode { parent, name, .. } => {
                let bytes = match *name {
                    None => Vec::new(),
                    Some(id) => self.name(id)?,
                };
                Some((self.opt_child(parent), bytes))
            }
            _ => None,
        }
    }

    fn raw_call_args(&self) -> Option<Vec<Self>> {
        match &self.node {
            Node::ArgumentsNode { arguments, .. } => Some(self.vec_children(arguments)),
            _ => None,
        }
    }

    fn raw_array_elements(&self) -> Option<Vec<Self>> {
        match &self.node {
            Node::ArrayNode { elements, .. } => Some(self.vec_children(elements)),
            _ => None,
        }
    }
}
