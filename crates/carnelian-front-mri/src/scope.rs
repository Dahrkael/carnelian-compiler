//! Scope/upvar pass (P4-B): locals per scope, `depth` per local use.
//!
//! Runs on the owned tree after lowering. `for` opens no scope here and
//! `for_depth` stays zero; the `for` body scope is backend business.

use carnelian_ast::{Node, SymbolId, SymbolPool};

/// Fill scope data in place: `locals` on every scope node, `depth` on
/// every local read/write/target.
///
/// Lowering decides lvar-vs-call sequentially, so an unbound read keeps
/// depth zero instead of declaring a new local.
pub fn resolve_scopes(node: &mut Node, pool: &mut SymbolPool) {
    Resolver {
        pool,
        scopes: Vec::new(),
    }
    .walk(node);
}

/// One open lexical scope.
#[derive(Debug, Default)]
struct Frame {
    /// Declared locals in first-appearance order.
    names: Vec<SymbolId>,
    /// Fresh root (`def`/`class`/`module`/`sclass`/program): hides outer scopes.
    barrier: bool,
    /// Transparent closure scope (`block`/`lambda`): numbered-use target.
    is_block: bool,
    /// Highest `_N` attributed here (consumed by numbered blocks only).
    numbered_max: u8,
}

struct Resolver<'a> {
    pool: &'a mut SymbolPool,
    scopes: Vec<Frame>,
}

impl Resolver<'_> {
    fn push_frame(&mut self, barrier: bool, is_block: bool) {
        self.scopes.push(Frame {
            names: Vec::new(),
            barrier,
            is_block,
            numbered_max: 0,
        });
    }

    fn pop_frame(&mut self) -> Frame {
        self.scopes.pop().unwrap_or_default()
    }

    fn walk_boxed(&mut self, node: &mut Box<Node>) {
        self.walk(node.as_mut());
    }

    fn walk_opt(&mut self, node: &mut Option<Box<Node>>) {
        if let Some(inner) = node.as_deref_mut() {
            self.walk(inner);
        }
    }

    fn walk_vec(&mut self, nodes: &mut [Node]) {
        for node in nodes.iter_mut() {
            self.walk(node);
        }
    }

    /// Declare a name in the current scope, keeping first position.
    fn declare(&mut self, id: SymbolId) {
        if self.scopes.is_empty() {
            self.push_frame(true, false);
        }
        if let Some(top) = self.scopes.last_mut() {
            if !top.names.contains(&id) {
                top.names.push(id);
            }
        }
    }

    /// Lexical distance to the defining scope, stopping at barriers.
    fn lookup(&self, id: SymbolId) -> Option<u32> {
        let mut depth = 0u32;
        for frame in self.scopes.iter().rev() {
            if frame.names.contains(&id) {
                return Some(depth);
            }
            if frame.barrier {
                return None;
            }
            depth = depth.saturating_add(1);
        }
        None
    }

    /// Depth for a read; unbound reads (lowering bug) stay zero.
    fn resolve_read(&mut self, id: SymbolId) -> u32 {
        if let Some(depth) = self.lookup(id) {
            return depth;
        }
        if let Some(number) = self.numbered_value(id) {
            if self.note_numbered(number, id) {
                return self.lookup(id).unwrap_or_default();
            }
        }
        0
    }

    /// Depth for a write/target; unbound names declare a fresh local here.
    fn bind_write(&mut self, id: SymbolId) -> u32 {
        if let Some(depth) = self.lookup(id) {
            return depth;
        }
        self.declare(id);
        0
    }

    /// Value of a `_1`..`_9` name, if any.
    fn numbered_value(&self, id: SymbolId) -> Option<u8> {
        let bytes = self.pool.lookup(id)?;
        numbered_of(bytes)
    }

    /// Attribute an unbound `_N` use to the nearest open block scope.
    fn note_numbered(&mut self, number: u8, id: SymbolId) -> bool {
        let mut target = None;
        for (index, frame) in self.scopes.iter().enumerate().rev() {
            if frame.is_block {
                target = Some(index);
                break;
            }
            if frame.barrier {
                return false;
            }
        }
        if let Some(index) = target {
            if let Some(frame) = self.scopes.get_mut(index) {
                if number > frame.numbered_max {
                    frame.numbered_max = number;
                }
                if !frame.names.contains(&id) {
                    frame.names.push(id);
                }
                return true;
            }
        }
        false
    }

    /// Block/lambda locals: `_1`..`_N` prefix, then body locals in order.
    fn numbered_locals(&mut self, names: &[SymbolId], maximum: u8) -> Vec<SymbolId> {
        let snapshot: Vec<(SymbolId, Vec<u8>)> = names
            .iter()
            .filter_map(|id| self.pool.lookup(*id).map(|bytes| (*id, bytes.to_vec())))
            .collect();
        let mut out = Vec::with_capacity(names.len() + usize::from(maximum));
        for index in 1..=maximum {
            out.push(self.pool.intern(format!("_{index}").as_bytes()));
        }
        for (id, bytes) in &snapshot {
            if !out.contains(id) && numbered_of(bytes).is_none() {
                out.push(*id);
            }
        }
        out
    }

    /// Shared `BlockNode`/`LambdaNode` body: params, body, locals fill.
    fn walk_block_like(
        &mut self,
        locals: &mut Vec<SymbolId>,
        parameters: &mut Option<Box<Node>>,
        body: &mut Option<Box<Node>>,
    ) {
        self.push_frame(false, true);
        if let Some(params) = parameters.as_deref_mut() {
            self.walk_block_params(params);
        }
        self.walk_opt(body);
        let frame = self.pop_frame();
        if matches!(
            parameters.as_deref(),
            Some(Node::NumberedParametersNode { .. })
        ) {
            if let Some(Node::NumberedParametersNode { maximum, .. }) = parameters.as_deref_mut() {
                *maximum = frame.numbered_max;
            }
            *locals = self.numbered_locals(&frame.names, frame.numbered_max);
        } else {
            *locals = frame.names;
        }
    }

    /// Block/lambda parameters: numbered/`it` markers, else names plus `;`-locals.
    fn walk_block_params(&mut self, params: &mut Node) {
        match params {
            Node::NumberedParametersNode { .. } | Node::ItParametersNode { .. } => {}
            Node::BlockParametersNode {
                parameters, locals, ..
            } => {
                if let Some(inner) = parameters.as_deref_mut() {
                    self.declare_method_params(inner);
                    self.walk_param_defaults(inner);
                }
                for local in locals.iter_mut() {
                    if let Node::BlockLocalVariableNode { name, .. } = local {
                        self.declare(*name);
                    }
                }
            }
            other => self.walk(other),
        }
    }

    /// Declare parameter names up front in backend register order; defaults
    /// are walked afterwards with every param already visible.
    fn declare_method_params(&mut self, params: &mut Node) {
        let Node::ParametersNode {
            requireds,
            optionals,
            rest,
            posts,
            keywords,
            keyword_rest,
            block,
            ..
        } = params
        else {
            self.walk(params);
            return;
        };
        for item in requireds.iter() {
            self.declare_positional(item);
        }
        for item in optionals.iter() {
            if let Node::OptionalParameterNode { name, .. } = item {
                self.declare(*name);
            }
        }
        if let Some(rest) = rest.as_deref() {
            self.declare_rest(rest);
        }
        for item in posts.iter() {
            self.declare_positional(item);
        }
        if let Some(rest) = keyword_rest.as_deref() {
            self.declare_keyword_rest(rest);
        }
        if let Some(block) = block.as_deref() {
            self.declare_block_name(block);
        }
        for item in keywords.iter() {
            match item {
                Node::RequiredKeywordParameterNode { name, .. }
                | Node::OptionalKeywordParameterNode { name, .. } => self.declare(*name),
                _ => {}
            }
        }
        for item in requireds.iter().chain(posts.iter()) {
            self.declare_destructured(item);
        }
    }

    /// Walk default values of optional/keyword parameters in order.
    fn walk_param_defaults(&mut self, params: &mut Node) {
        let Node::ParametersNode {
            optionals,
            keywords,
            ..
        } = params
        else {
            return;
        };
        for item in optionals.iter_mut() {
            if let Node::OptionalParameterNode { value, .. } = item {
                self.walk_boxed(value);
            }
        }
        for item in keywords.iter_mut() {
            if let Node::OptionalKeywordParameterNode { value, .. } = item {
                self.walk_boxed(value);
            }
        }
    }

    /// One positional slot: a name, or nothing for a destructured slot.
    fn declare_positional(&mut self, item: &Node) {
        if let Node::RequiredParameterNode { name, .. } = item {
            self.declare(*name);
        }
    }

    fn declare_rest(&mut self, node: &Node) {
        if let Node::RestParameterNode { name: Some(id), .. } = node {
            self.declare(*id);
        }
    }

    fn declare_keyword_rest(&mut self, node: &Node) {
        if let Node::KeywordRestParameterNode { name: Some(id), .. } = node {
            self.declare(*id);
        }
    }

    /// Named `&`-parameters, except anonymous/nil forms.
    fn declare_block_name(&mut self, node: &Node) {
        if let Node::BlockParameterNode { name: Some(id), .. } = node {
            let is_nil = self.pool.lookup(*id) == Some(b"nil".as_slice());
            if !is_nil {
                self.declare(*id);
            }
        }
    }

    /// Leaf names of a destructured positional slot, in field order.
    fn declare_destructured(&mut self, node: &Node) {
        match node {
            Node::RequiredParameterNode { name, .. }
            | Node::LocalVariableTargetNode { name, .. } => self.declare(*name),
            Node::RestParameterNode { name: Some(id), .. } => self.declare(*id),
            Node::MultiTargetNode {
                lefts,
                rest,
                rights,
                ..
            } => {
                for part in lefts.iter() {
                    self.declare_destructured(part);
                }
                if let Some(rest) = rest.as_deref() {
                    self.declare_destructured(rest);
                }
                for part in rights.iter() {
                    self.declare_destructured(part);
                }
            }
            Node::SplatNode { expression, .. } => {
                if let Some(inner) = expression.as_deref() {
                    self.declare_destructured(inner);
                }
            }
            _ => {}
        }
    }

    #[allow(clippy::too_many_lines)]
    fn walk(&mut self, node: &mut Node) {
        // A bare call (`name`, no receiver/args/block/parens) naming an
        // already-declared local is a read: locals shadow methods. This heals
        // parsers that cannot pre-declare names — MRI `=~` capture bindings
        // surface here as `MatchWriteNode` targets bound by this same walk,
        // so later uses resolve while earlier ones stay calls, like Prism.
        let read = match node {
            Node::CallNode {
                receiver: None,
                arguments: None,
                block: None,
                opening_loc: None,
                closing_loc: None,
                call_operator_loc: None,
                equal_loc: None,
                name,
                span,
                ..
            } => self.lookup(*name).map(|depth| (*span, *name, depth)),
            _ => None,
        };
        if let Some((span, name, depth)) = read {
            *node = Node::LocalVariableReadNode {
                flags: 0,
                span,
                name,
                depth,
            };
            return;
        }
        match node {
            Node::AliasGlobalVariableNode {
                new_name, old_name, ..
            } => {
                self.walk_boxed(new_name);
                self.walk_boxed(old_name);
            }
            Node::AliasMethodNode {
                new_name, old_name, ..
            } => {
                self.walk_boxed(new_name);
                self.walk_boxed(old_name);
            }
            Node::AlternationPatternNode { left, right, .. } => {
                self.walk_boxed(left);
                self.walk_boxed(right);
            }
            Node::AndNode { left, right, .. } | Node::OrNode { left, right, .. } => {
                self.walk_boxed(left);
                self.walk_boxed(right);
            }
            Node::ArgumentsNode { arguments, .. } => self.walk_vec(arguments),
            Node::ArrayNode { elements, .. } => self.walk_vec(elements),
            Node::ArrayPatternNode {
                constant,
                requireds,
                rest,
                posts,
                ..
            } => {
                self.walk_opt(constant);
                self.walk_vec(requireds);
                self.walk_opt(rest);
                self.walk_vec(posts);
            }
            Node::AssocNode { key, value, .. } => {
                self.walk_boxed(key);
                self.walk_boxed(value);
            }
            Node::AssocSplatNode { value, .. } => self.walk_opt(value),
            Node::BackReferenceReadNode { .. } => {}
            Node::BeginNode {
                statements,
                rescue_clause,
                else_clause,
                ensure_clause,
                ..
            } => {
                self.walk_opt(statements);
                self.walk_opt(rescue_clause);
                self.walk_opt(else_clause);
                self.walk_opt(ensure_clause);
            }
            Node::BlockArgumentNode { expression, .. } => self.walk_opt(expression),
            Node::BlockLocalVariableNode { name, .. } => self.declare(*name),
            Node::BlockNode {
                locals,
                parameters,
                body,
                ..
            } => self.walk_block_like(locals, parameters, body),
            Node::BlockParameterNode { name, .. } => {
                if let Some(id) = *name {
                    let is_nil = self.pool.lookup(id) == Some(b"nil".as_slice());
                    if !is_nil {
                        self.declare(id);
                    }
                }
            }
            Node::BlockParametersNode {
                parameters, locals, ..
            } => {
                self.walk_opt(parameters);
                self.walk_vec(locals);
            }
            Node::BreakNode { arguments, .. } => self.walk_opt(arguments),
            Node::CallAndWriteNode {
                receiver, value, ..
            }
            | Node::CallOperatorWriteNode {
                receiver, value, ..
            }
            | Node::CallOrWriteNode {
                receiver, value, ..
            } => {
                self.walk_opt(receiver);
                self.walk_boxed(value);
            }
            Node::CallNode {
                receiver,
                arguments,
                block,
                ..
            } => {
                self.walk_opt(receiver);
                self.walk_opt(arguments);
                self.walk_opt(block);
            }
            Node::CallTargetNode { receiver, .. } => self.walk_boxed(receiver),
            Node::CapturePatternNode { value, target, .. } => {
                self.walk_boxed(value);
                self.walk_boxed(target);
            }
            Node::CaseMatchNode {
                predicate,
                conditions,
                else_clause,
                ..
            }
            | Node::CaseNode {
                predicate,
                conditions,
                else_clause,
                ..
            } => {
                self.walk_opt(predicate);
                self.walk_vec(conditions);
                self.walk_opt(else_clause);
            }
            Node::ClassNode {
                locals,
                constant_path,
                superclass,
                body,
                ..
            } => {
                self.walk_boxed(constant_path);
                self.walk_opt(superclass);
                self.push_frame(true, false);
                self.walk_opt(body);
                let frame = self.pop_frame();
                *locals = frame.names;
            }
            Node::ClassVariableAndWriteNode { value, .. }
            | Node::ClassVariableOperatorWriteNode { value, .. }
            | Node::ClassVariableOrWriteNode { value, .. }
            | Node::ClassVariableWriteNode { value, .. }
            | Node::ConstantAndWriteNode { value, .. }
            | Node::ConstantOperatorWriteNode { value, .. }
            | Node::ConstantOrWriteNode { value, .. }
            | Node::ConstantWriteNode { value, .. }
            | Node::GlobalVariableAndWriteNode { value, .. }
            | Node::GlobalVariableOperatorWriteNode { value, .. }
            | Node::GlobalVariableOrWriteNode { value, .. }
            | Node::GlobalVariableWriteNode { value, .. }
            | Node::InstanceVariableAndWriteNode { value, .. }
            | Node::InstanceVariableOperatorWriteNode { value, .. }
            | Node::InstanceVariableOrWriteNode { value, .. }
            | Node::InstanceVariableWriteNode { value, .. } => self.walk_boxed(value),
            Node::ClassVariableReadNode { .. }
            | Node::ClassVariableTargetNode { .. }
            | Node::ConstantReadNode { .. }
            | Node::ConstantTargetNode { .. }
            | Node::FalseNode { .. }
            | Node::FloatNode { .. }
            | Node::ForwardingArgumentsNode { .. }
            | Node::ForwardingParameterNode { .. }
            | Node::GlobalVariableReadNode { .. }
            | Node::GlobalVariableTargetNode { .. }
            | Node::ImplicitRestNode { .. }
            | Node::InstanceVariableReadNode { .. }
            | Node::InstanceVariableTargetNode { .. }
            | Node::IntegerNode { .. }
            | Node::ItLocalVariableReadNode { .. }
            | Node::ItParametersNode { .. }
            | Node::MatchLastLineNode { .. }
            | Node::MissingNode { .. }
            | Node::NilNode { .. }
            | Node::NoKeywordsParameterNode { .. }
            | Node::NumberedParametersNode { .. }
            | Node::NumberedReferenceReadNode { .. }
            | Node::RationalNode { .. }
            | Node::RedoNode { .. }
            | Node::RegularExpressionNode { .. }
            | Node::RetryNode { .. }
            | Node::SelfNode { .. }
            | Node::SourceEncodingNode { .. }
            | Node::SourceFileNode { .. }
            | Node::SourceLineNode { .. }
            | Node::StringNode { .. }
            | Node::SymbolNode { .. }
            | Node::TrueNode { .. }
            | Node::XStringNode { .. } => {}
            Node::ConstantPathAndWriteNode { target, value, .. }
            | Node::ConstantPathOperatorWriteNode { target, value, .. }
            | Node::ConstantPathOrWriteNode { target, value, .. }
            | Node::ConstantPathWriteNode { target, value, .. } => {
                self.walk_boxed(target);
                self.walk_boxed(value);
            }
            Node::ConstantPathNode { parent, .. } | Node::ConstantPathTargetNode { parent, .. } => {
                self.walk_opt(parent)
            }
            Node::DefNode {
                receiver,
                parameters,
                body,
                locals,
                ..
            } => {
                self.walk_opt(receiver);
                self.push_frame(true, false);
                if let Some(params) = parameters.as_deref_mut() {
                    self.declare_method_params(params);
                    self.walk_param_defaults(params);
                }
                self.walk_opt(body);
                let frame = self.pop_frame();
                *locals = frame.names;
            }
            Node::DefinedNode { value, .. } => self.walk_boxed(value),
            Node::ElseNode { statements, .. } | Node::EnsureNode { statements, .. } => {
                self.walk_opt(statements);
            }
            Node::EmbeddedStatementsNode { statements, .. } => self.walk_opt(statements),
            Node::EmbeddedVariableNode { variable, .. } => self.walk_boxed(variable),
            Node::FindPatternNode {
                constant,
                left,
                requireds,
                right,
                ..
            } => {
                self.walk_opt(constant);
                self.walk_boxed(left);
                self.walk_vec(requireds);
                self.walk_boxed(right);
            }
            Node::FlipFlopNode { left, right, .. } => {
                self.walk_opt(left);
                self.walk_opt(right);
            }
            Node::ForNode {
                index,
                collection,
                statements,
                ..
            } => {
                self.walk_boxed(index);
                self.walk_boxed(collection);
                self.walk_opt(statements);
            }
            Node::ForwardingSuperNode { block, .. } => self.walk_opt(block),
            Node::HashNode { elements, .. } | Node::KeywordHashNode { elements, .. } => {
                self.walk_vec(elements);
            }
            Node::HashPatternNode {
                constant,
                elements,
                rest,
                ..
            } => {
                self.walk_opt(constant);
                self.walk_vec(elements);
                self.walk_opt(rest);
            }
            Node::IfNode {
                predicate,
                statements,
                subsequent,
                ..
            } => {
                self.walk_boxed(predicate);
                self.walk_opt(statements);
                self.walk_opt(subsequent);
            }
            Node::UnlessNode {
                predicate,
                statements,
                else_clause,
                ..
            } => {
                self.walk_boxed(predicate);
                self.walk_opt(statements);
                self.walk_opt(else_clause);
            }
            Node::ImaginaryNode { numeric, .. } => self.walk_boxed(numeric),
            Node::ImplicitNode { value, .. } => self.walk_boxed(value),
            Node::InNode {
                pattern,
                statements,
                ..
            } => {
                self.walk_boxed(pattern);
                self.walk_opt(statements);
            }
            Node::IndexAndWriteNode {
                receiver,
                arguments,
                block,
                value,
                ..
            }
            | Node::IndexOperatorWriteNode {
                receiver,
                arguments,
                block,
                value,
                ..
            }
            | Node::IndexOrWriteNode {
                receiver,
                arguments,
                block,
                value,
                ..
            } => {
                self.walk_opt(receiver);
                self.walk_opt(arguments);
                self.walk_opt(block);
                self.walk_boxed(value);
            }
            Node::IndexTargetNode {
                receiver,
                arguments,
                block,
                ..
            } => {
                self.walk_boxed(receiver);
                self.walk_opt(arguments);
                self.walk_opt(block);
            }
            Node::InterpolatedMatchLastLineNode { parts, .. }
            | Node::InterpolatedRegularExpressionNode { parts, .. }
            | Node::InterpolatedStringNode { parts, .. }
            | Node::InterpolatedSymbolNode { parts, .. }
            | Node::InterpolatedXStringNode { parts, .. } => self.walk_vec(parts),
            Node::KeywordRestParameterNode { name, .. } | Node::RestParameterNode { name, .. } => {
                if let Some(id) = name {
                    self.declare(*id);
                }
            }
            Node::LambdaNode {
                locals,
                parameters,
                body,
                ..
            } => self.walk_block_like(locals, parameters, body),
            Node::LocalVariableAndWriteNode {
                name, depth, value, ..
            }
            | Node::LocalVariableOperatorWriteNode {
                name, depth, value, ..
            }
            | Node::LocalVariableOrWriteNode {
                name, depth, value, ..
            } => {
                self.walk_boxed(value);
                *depth = self.bind_write(*name);
            }
            Node::LocalVariableReadNode { name, depth, .. } => {
                *depth = self.resolve_read(*name);
            }
            Node::LocalVariableTargetNode { name, depth, .. } => {
                *depth = self.bind_write(*name);
            }
            Node::LocalVariableWriteNode {
                name, depth, value, ..
            } => {
                // Source order: the target precedes its value, so a value
                // that binds pattern locals keeps Prism's slot order.
                *depth = self.bind_write(*name);
                self.walk_boxed(value);
            }
            Node::MatchPredicateNode { value, pattern, .. }
            | Node::MatchRequiredNode { value, pattern, .. } => {
                self.walk_boxed(value);
                self.walk_boxed(pattern);
            }
            Node::MatchWriteNode { call, targets, .. } => {
                self.walk_boxed(call);
                self.walk_vec(targets);
            }
            Node::ModuleNode {
                locals,
                constant_path,
                body,
                ..
            } => {
                self.walk_boxed(constant_path);
                self.push_frame(true, false);
                self.walk_opt(body);
                let frame = self.pop_frame();
                *locals = frame.names;
            }
            Node::MultiTargetNode {
                lefts,
                rest,
                rights,
                ..
            } => {
                self.walk_vec(lefts);
                self.walk_opt(rest);
                self.walk_vec(rights);
            }
            Node::MultiWriteNode {
                lefts,
                rest,
                rights,
                value,
                ..
            } => {
                self.walk_vec(lefts);
                self.walk_opt(rest);
                self.walk_vec(rights);
                self.walk_boxed(value);
            }
            Node::NextNode { arguments, .. } | Node::ReturnNode { arguments, .. } => {
                self.walk_opt(arguments);
            }
            Node::OptionalKeywordParameterNode { name, value, .. } => {
                self.declare(*name);
                self.walk_boxed(value);
            }
            Node::OptionalParameterNode { name, value, .. } => {
                self.declare(*name);
                self.walk_boxed(value);
            }
            Node::ParametersNode {
                requireds,
                optionals,
                rest,
                posts,
                keywords,
                keyword_rest,
                block,
                ..
            } => {
                self.walk_vec(requireds);
                self.walk_vec(optionals);
                self.walk_opt(rest);
                self.walk_vec(posts);
                self.walk_vec(keywords);
                self.walk_opt(keyword_rest);
                self.walk_opt(block);
            }
            Node::ParenthesesNode { body, .. } => self.walk_opt(body),
            Node::PinnedExpressionNode { expression, .. } => self.walk_boxed(expression),
            Node::PinnedVariableNode { variable, .. } => self.walk_boxed(variable),
            Node::PostExecutionNode { statements, .. }
            | Node::PreExecutionNode { statements, .. } => self.walk_opt(statements),
            Node::ProgramNode {
                locals, statements, ..
            } => {
                self.push_frame(true, false);
                self.walk_boxed(statements);
                let frame = self.pop_frame();
                *locals = frame.names;
            }
            Node::RangeNode { left, right, .. } => {
                self.walk_opt(left);
                self.walk_opt(right);
            }
            Node::RequiredKeywordParameterNode { name, .. }
            | Node::RequiredParameterNode { name, .. } => self.declare(*name),
            Node::RescueModifierNode {
                expression,
                rescue_expression,
                ..
            } => {
                self.walk_boxed(expression);
                self.walk_boxed(rescue_expression);
            }
            Node::RescueNode {
                exceptions,
                reference,
                statements,
                subsequent,
                ..
            } => {
                self.walk_vec(exceptions);
                self.walk_opt(reference);
                self.walk_opt(statements);
                self.walk_opt(subsequent);
            }
            Node::ShareableConstantNode { write, .. } => self.walk_boxed(write),
            Node::SingletonClassNode {
                locals,
                expression,
                body,
                ..
            } => {
                self.walk_boxed(expression);
                self.push_frame(true, false);
                self.walk_opt(body);
                let frame = self.pop_frame();
                *locals = frame.names;
            }
            Node::SplatNode { expression, .. } => self.walk_opt(expression),
            Node::StatementsNode { body, .. } => self.walk_vec(body),
            Node::SuperNode {
                arguments, block, ..
            } => {
                self.walk_opt(arguments);
                self.walk_opt(block);
            }
            Node::UndefNode { names, .. } => self.walk_vec(names),
            Node::UntilNode {
                predicate,
                statements,
                ..
            }
            | Node::WhileNode {
                predicate,
                statements,
                ..
            } => {
                self.walk_boxed(predicate);
                self.walk_opt(statements);
            }
            Node::WhenNode {
                conditions,
                statements,
                ..
            } => {
                self.walk_vec(conditions);
                self.walk_opt(statements);
            }
            Node::YieldNode { arguments, .. } => self.walk_opt(arguments),
        }
    }
}

/// Value of a `_1`..`_9` name, if any.
fn numbered_of(bytes: &[u8]) -> Option<u8> {
    if bytes.len() == 2 && bytes[0] == b'_' && bytes[1] >= b'1' && bytes[1] <= b'9' {
        Some(bytes[1] - b'0')
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use carnelian_ast::{Integer, Span};

    fn span() -> Span {
        Span { start: 0, end: 0 }
    }

    fn sym(pool: &mut SymbolPool, name: &str) -> SymbolId {
        pool.intern(name.as_bytes())
    }

    fn program(body: Node) -> Node {
        Node::ProgramNode {
            flags: 0,
            span: span(),
            locals: Vec::new(),
            statements: Box::new(body),
        }
    }

    fn stmts(body: Vec<Node>) -> Node {
        Node::StatementsNode {
            flags: 0,
            span: span(),
            body,
        }
    }

    fn read(pool: &mut SymbolPool, name: &str) -> Node {
        Node::LocalVariableReadNode {
            flags: 0,
            span: span(),
            name: sym(pool, name),
            depth: 0,
        }
    }

    fn write(pool: &mut SymbolPool, name: &str, value: Node) -> Node {
        Node::LocalVariableWriteNode {
            flags: 0,
            span: span(),
            name: sym(pool, name),
            depth: 0,
            name_loc: span(),
            value: Box::new(value),
            operator_loc: span(),
        }
    }

    fn target(pool: &mut SymbolPool, name: &str) -> Node {
        Node::LocalVariableTargetNode {
            flags: 0,
            span: span(),
            name: sym(pool, name),
            depth: 0,
        }
    }

    fn nil() -> Node {
        Node::NilNode {
            flags: 0,
            span: span(),
        }
    }

    fn int(value: i64) -> Node {
        Node::IntegerNode {
            flags: 0,
            span: span(),
            value: Integer::I64(value),
        }
    }

    fn const_read(pool: &mut SymbolPool, name: &str) -> Node {
        Node::ConstantReadNode {
            flags: 0,
            span: span(),
            name: sym(pool, name),
        }
    }

    fn block(params: Option<Node>, body: Option<Node>) -> Node {
        Node::BlockNode {
            flags: 0,
            span: span(),
            locals: Vec::new(),
            parameters: params.map(Box::new),
            body: body.map(Box::new),
            opening_loc: span(),
            closing_loc: span(),
        }
    }

    fn call_block(pool: &mut SymbolPool, name: &str, block: Node) -> Node {
        Node::CallNode {
            flags: 0,
            span: span(),
            receiver: None,
            call_operator_loc: None,
            name: sym(pool, name),
            message_loc: None,
            opening_loc: None,
            arguments: None,
            closing_loc: None,
            equal_loc: None,
            block: Some(Box::new(block)),
        }
    }

    fn block_params(params: Option<Node>, locals: Vec<Node>) -> Node {
        Node::BlockParametersNode {
            flags: 0,
            span: span(),
            parameters: params.map(Box::new),
            locals,
            opening_loc: None,
            closing_loc: None,
        }
    }

    fn method_params(
        requireds: Vec<Node>,
        optionals: Vec<Node>,
        posts: Vec<Node>,
        keywords: Vec<Node>,
        rest: Option<Node>,
        keyword_rest: Option<Node>,
        block: Option<Node>,
    ) -> Node {
        Node::ParametersNode {
            flags: 0,
            span: span(),
            requireds,
            optionals,
            rest: rest.map(Box::new),
            posts,
            keywords,
            keyword_rest: keyword_rest.map(Box::new),
            block: block.map(Box::new),
        }
    }

    fn required(pool: &mut SymbolPool, name: &str) -> Node {
        Node::RequiredParameterNode {
            flags: 0,
            span: span(),
            name: sym(pool, name),
        }
    }

    fn optional(pool: &mut SymbolPool, name: &str, value: Node) -> Node {
        Node::OptionalParameterNode {
            flags: 0,
            span: span(),
            name: sym(pool, name),
            name_loc: span(),
            operator_loc: span(),
            value: Box::new(value),
        }
    }

    fn def(pool: &mut SymbolPool, name: &str, params: Option<Node>, body: Option<Node>) -> Node {
        Node::DefNode {
            flags: 0,
            span: span(),
            name: sym(pool, name),
            name_loc: span(),
            receiver: None,
            parameters: params.map(Box::new),
            body: body.map(Box::new),
            locals: Vec::new(),
            def_keyword_loc: span(),
            operator_loc: None,
            lparen_loc: None,
            rparen_loc: None,
            equal_loc: None,
            end_keyword_loc: None,
        }
    }

    fn local_names(pool: &SymbolPool, ids: &[SymbolId]) -> Vec<String> {
        ids.iter()
            .map(|id| {
                pool.lookup(*id)
                    .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
                    .unwrap_or_default()
            })
            .collect()
    }

    fn program_locals(node: &Node, pool: &SymbolPool) -> Option<Vec<String>> {
        if let Node::ProgramNode { locals, .. } = node {
            Some(local_names(pool, locals))
        } else {
            None
        }
    }

    fn block_locals(node: &Node, pool: &SymbolPool) -> Option<Vec<String>> {
        if let Node::BlockNode { locals, .. } = node {
            Some(local_names(pool, locals))
        } else {
            None
        }
    }

    fn def_locals(node: &Node, pool: &SymbolPool) -> Option<Vec<String>> {
        if let Node::DefNode { locals, .. } = node {
            Some(local_names(pool, locals))
        } else {
            None
        }
    }

    fn read_depth(node: &Node) -> Option<u32> {
        if let Node::LocalVariableReadNode { depth, .. } = node {
            Some(*depth)
        } else {
            None
        }
    }

    fn write_depth(node: &Node) -> Option<u32> {
        if let Node::LocalVariableWriteNode { depth, .. } = node {
            Some(*depth)
        } else {
            None
        }
    }

    fn target_depth(node: &Node) -> Option<u32> {
        if let Node::LocalVariableTargetNode { depth, .. } = node {
            Some(*depth)
        } else {
            None
        }
    }

    fn stmts_body(node: &Node) -> Option<&Vec<Node>> {
        if let Node::StatementsNode { body, .. } = node {
            Some(body)
        } else {
            None
        }
    }

    fn program_stmts(node: &Node) -> Option<&Vec<Node>> {
        if let Node::ProgramNode { statements, .. } = node {
            stmts_body(statements)
        } else {
            None
        }
    }

    fn call_block_node(node: &Node) -> Option<&Node> {
        if let Node::CallNode {
            block: Some(block), ..
        } = node
        {
            Some(block)
        } else {
            None
        }
    }

    fn block_body(node: &Node) -> Option<&Vec<Node>> {
        if let Node::BlockNode {
            body: Some(body), ..
        } = node
        {
            stmts_body(body)
        } else {
            None
        }
    }

    #[test]
    fn program_write_then_read() {
        let mut pool = SymbolPool::new();
        let mut root = program(stmts(vec![
            write(&mut pool, "x", int(1)),
            read(&mut pool, "x"),
        ]));
        resolve_scopes(&mut root, &mut pool);
        assert_eq!(program_locals(&root, &pool), Some(vec!["x".to_owned()]));
        let body = program_stmts(&root).expect("program body");
        assert_eq!(write_depth(&body[0]), Some(0));
        assert_eq!(read_depth(&body[1]), Some(0));
    }

    #[test]
    fn block_param_shadows_outer() {
        let mut pool = SymbolPool::new();
        let inner = block(
            Some(block_params(
                Some(method_params(
                    vec![required(&mut pool, "x")],
                    vec![],
                    vec![],
                    vec![],
                    None,
                    None,
                    None,
                )),
                vec![],
            )),
            Some(stmts(vec![read(&mut pool, "x")])),
        );
        let mut root = program(stmts(vec![
            write(&mut pool, "x", int(1)),
            call_block(&mut pool, "foo", inner),
            read(&mut pool, "x"),
        ]));
        resolve_scopes(&mut root, &mut pool);
        assert_eq!(program_locals(&root, &pool), Some(vec!["x".to_owned()]));
        let body = program_stmts(&root).expect("program body");
        let flown = call_block_node(&body[1]).expect("call block");
        assert_eq!(block_locals(flown, &pool), Some(vec!["x".to_owned()]));
        let inner_body = block_body(flown).expect("block body");
        assert_eq!(read_depth(&inner_body[0]), Some(0));
        assert_eq!(read_depth(&body[2]), Some(0));
    }

    #[test]
    fn upvar_read_depths() {
        let mut pool = SymbolPool::new();
        let inner = block(None, Some(stmts(vec![read(&mut pool, "x")])));
        let outer = block(None, Some(stmts(vec![call_block(&mut pool, "bar", inner)])));
        let mut root = program(stmts(vec![
            write(&mut pool, "x", int(1)),
            call_block(&mut pool, "foo", outer),
        ]));
        resolve_scopes(&mut root, &mut pool);
        let body = program_stmts(&root).expect("program body");
        let outer_block = call_block_node(&body[1]).expect("outer call");
        assert_eq!(block_locals(outer_block, &pool), Some(Vec::new()));
        let outer_body = block_body(outer_block).expect("outer body");
        let inner_block = call_block_node(&outer_body[0]).expect("inner call");
        assert_eq!(block_locals(inner_block, &pool), Some(Vec::new()));
        let inner_body = block_body(inner_block).expect("inner body");
        assert_eq!(read_depth(&inner_body[0]), Some(2));
    }

    #[test]
    fn block_write_scopes() {
        let mut pool = SymbolPool::new();
        let upvar_body = stmts(vec![write(&mut pool, "x", int(2))]);
        let fresh_body = stmts(vec![write(&mut pool, "y", int(3))]);
        let mut root = program(stmts(vec![
            write(&mut pool, "x", int(1)),
            call_block(&mut pool, "foo", block(None, Some(upvar_body))),
            call_block(&mut pool, "foo", block(None, Some(fresh_body))),
        ]));
        resolve_scopes(&mut root, &mut pool);
        let body = program_stmts(&root).expect("program body");
        let upvar = call_block_node(&body[1]).expect("upvar block");
        assert_eq!(block_locals(upvar, &pool), Some(Vec::new()));
        let upvar_body = block_body(upvar).expect("upvar body");
        assert_eq!(write_depth(&upvar_body[0]), Some(1));
        let fresh = call_block_node(&body[2]).expect("fresh block");
        assert_eq!(block_locals(fresh, &pool), Some(vec!["y".to_owned()]));
        let fresh_body = block_body(fresh).expect("fresh body");
        assert_eq!(write_depth(&fresh_body[0]), Some(0));
    }

    #[test]
    fn write_value_sees_outer_first() {
        let mut pool = SymbolPool::new();
        let rhs = read(&mut pool, "x");
        let inner = block(None, Some(stmts(vec![write(&mut pool, "x", rhs)])));
        let mut root = program(stmts(vec![
            write(&mut pool, "x", int(1)),
            call_block(&mut pool, "foo", inner),
        ]));
        resolve_scopes(&mut root, &mut pool);
        let body = program_stmts(&root).expect("program body");
        let flown = call_block_node(&body[1]).expect("call block");
        let inner_body = block_body(flown).expect("block body");
        if let Node::LocalVariableWriteNode { depth, value, .. } = &inner_body[0] {
            assert_eq!(*depth, 1);
            assert_eq!(read_depth(value), Some(1));
        } else {
            panic!("expected write node");
        }
    }

    #[test]
    fn def_is_barrier() {
        let mut pool = SymbolPool::new();
        let def_body = stmts(vec![write(&mut pool, "x", int(2)), read(&mut pool, "x")]);
        let method = def(&mut pool, "m", None, Some(def_body));
        let mut root = program(stmts(vec![write(&mut pool, "x", int(1)), method]));
        resolve_scopes(&mut root, &mut pool);
        assert_eq!(program_locals(&root, &pool), Some(vec!["x".to_owned()]));
        let body = program_stmts(&root).expect("program body");
        assert_eq!(def_locals(&body[1], &pool), Some(vec!["x".to_owned()]));
        if let Node::DefNode {
            body: Some(body), ..
        } = &body[1]
        {
            let def_body = stmts_body(body).expect("def body");
            assert_eq!(write_depth(&def_body[0]), Some(0));
            assert_eq!(read_depth(&def_body[1]), Some(0));
        } else {
            panic!("expected def body");
        }
    }

    #[test]
    fn def_params_with_defaults() {
        let mut pool = SymbolPool::new();
        let default = read(&mut pool, "a");
        let params = method_params(
            vec![required(&mut pool, "a")],
            vec![optional(&mut pool, "b", default)],
            vec![],
            vec![],
            None,
            None,
            None,
        );
        let def_body = stmts(vec![read(&mut pool, "b")]);
        let method = def(&mut pool, "m", Some(params), Some(def_body));
        let mut root = program(stmts(vec![method]));
        resolve_scopes(&mut root, &mut pool);
        let body = program_stmts(&root).expect("program body");
        assert_eq!(
            def_locals(&body[0], &pool),
            Some(vec!["a".to_owned(), "b".to_owned()])
        );
        if let Node::DefNode {
            parameters: Some(params),
            body: Some(body),
            ..
        } = &body[0]
        {
            if let Node::ParametersNode { optionals, .. } = params.as_ref() {
                if let Node::OptionalParameterNode { value, .. } = &optionals[0] {
                    assert_eq!(read_depth(value), Some(0));
                } else {
                    panic!("expected optional param");
                }
            } else {
                panic!("expected parameters");
            }
            let def_body = stmts_body(body).expect("def body");
            assert_eq!(read_depth(&def_body[0]), Some(0));
        } else {
            panic!("expected def parts");
        }
    }

    #[test]
    fn endless_def_body_locals() {
        let mut pool = SymbolPool::new();
        let body_write = write(&mut pool, "y", int(1));
        let method = def(&mut pool, "m", None, Some(body_write));
        let mut root = program(stmts(vec![method]));
        resolve_scopes(&mut root, &mut pool);
        let body = program_stmts(&root).expect("program body");
        assert_eq!(def_locals(&body[0], &pool), Some(vec!["y".to_owned()]));
    }

    #[test]
    fn masgn_targets_bind_in_order() {
        let mut pool = SymbolPool::new();
        let nested = Node::MultiTargetNode {
            flags: 0,
            span: span(),
            lefts: vec![target(&mut pool, "b"), target(&mut pool, "c")],
            rest: None,
            rights: vec![],
            lparen_loc: None,
            rparen_loc: None,
        };
        let masgn = Node::MultiWriteNode {
            flags: 0,
            span: span(),
            lefts: vec![target(&mut pool, "a"), nested],
            rest: None,
            rights: vec![],
            lparen_loc: None,
            rparen_loc: None,
            operator_loc: span(),
            value: Box::new(int(1)),
        };
        let mut root = program(stmts(vec![masgn]));
        resolve_scopes(&mut root, &mut pool);
        assert_eq!(
            program_locals(&root, &pool),
            Some(vec!["a".to_owned(), "b".to_owned(), "c".to_owned()])
        );
        let body = program_stmts(&root).expect("program body");
        if let Node::MultiWriteNode { lefts, .. } = &body[0] {
            assert_eq!(target_depth(&lefts[0]), Some(0));
            if let Node::MultiTargetNode { lefts, .. } = &lefts[1] {
                assert_eq!(target_depth(&lefts[0]), Some(0));
                assert_eq!(target_depth(&lefts[1]), Some(0));
            } else {
                panic!("expected nested multi target");
            }
        } else {
            panic!("expected multi write");
        }
    }

    #[test]
    fn masgn_splat_rest_binds() {
        let mut pool = SymbolPool::new();
        let rest = Node::SplatNode {
            flags: 0,
            span: span(),
            operator_loc: span(),
            expression: Some(Box::new(target(&mut pool, "b"))),
        };
        let masgn = Node::MultiWriteNode {
            flags: 0,
            span: span(),
            lefts: vec![target(&mut pool, "a")],
            rest: Some(Box::new(rest)),
            rights: vec![],
            lparen_loc: None,
            rparen_loc: None,
            operator_loc: span(),
            value: Box::new(int(1)),
        };
        let mut root = program(stmts(vec![masgn]));
        resolve_scopes(&mut root, &mut pool);
        assert_eq!(
            program_locals(&root, &pool),
            Some(vec!["a".to_owned(), "b".to_owned()])
        );
    }

    #[test]
    fn for_index_binds_enclosing() {
        let mut pool = SymbolPool::new();
        let loop_ = Node::ForNode {
            flags: 0,
            span: span(),
            index: Box::new(target(&mut pool, "i")),
            collection: Box::new(int(1)),
            statements: Some(Box::new(stmts(vec![read(&mut pool, "i")]))),
            for_keyword_loc: span(),
            in_keyword_loc: span(),
            do_keyword_loc: None,
            end_keyword_loc: span(),
        };
        let mut root = program(stmts(vec![loop_]));
        resolve_scopes(&mut root, &mut pool);
        assert_eq!(program_locals(&root, &pool), Some(vec!["i".to_owned()]));
        let body = program_stmts(&root).expect("program body");
        if let Node::ForNode {
            index,
            statements: Some(statements),
            ..
        } = &body[0]
        {
            assert_eq!(target_depth(index), Some(0));
            let loop_body = stmts_body(statements).expect("for body");
            assert_eq!(read_depth(&loop_body[0]), Some(0));
        } else {
            panic!("expected for node");
        }
    }

    #[test]
    fn numbered_block_maximum_and_prefix() {
        let mut pool = SymbolPool::new();
        let params = Node::NumberedParametersNode {
            flags: 0,
            span: span(),
            maximum: 0,
        };
        let inner = block(
            Some(params),
            Some(stmts(vec![read(&mut pool, "_2"), read(&mut pool, "_1")])),
        );
        let mut root = program(stmts(vec![call_block(&mut pool, "foo", inner)]));
        resolve_scopes(&mut root, &mut pool);
        let body = program_stmts(&root).expect("program body");
        let flown = call_block_node(&body[0]).expect("call block");
        if let Node::BlockNode {
            locals,
            parameters: Some(params),
            body: Some(body),
            ..
        } = flown
        {
            assert_eq!(
                local_names(&pool, locals),
                vec!["_1".to_owned(), "_2".to_owned()]
            );
            if let Node::NumberedParametersNode { maximum, .. } = params.as_ref() {
                assert_eq!(*maximum, 2);
            } else {
                panic!("expected numbered params");
            }
            let inner_body = stmts_body(body).expect("block body");
            assert_eq!(read_depth(&inner_body[0]), Some(0));
            assert_eq!(read_depth(&inner_body[1]), Some(0));
        } else {
            panic!("expected numbered block");
        }
    }

    #[test]
    fn it_block_has_no_it_local() {
        let mut pool = SymbolPool::new();
        let params = Node::ItParametersNode {
            flags: 0,
            span: span(),
        };
        let it_read = Node::ItLocalVariableReadNode {
            flags: 0,
            span: span(),
        };
        let inner = block(
            Some(params),
            Some(stmts(vec![it_read, write(&mut pool, "z", int(1))])),
        );
        let mut root = program(stmts(vec![call_block(&mut pool, "foo", inner)]));
        resolve_scopes(&mut root, &mut pool);
        let body = program_stmts(&root).expect("program body");
        let flown = call_block_node(&body[0]).expect("call block");
        assert_eq!(block_locals(flown, &pool), Some(vec!["z".to_owned()]));
    }

    #[test]
    fn block_semicolon_locals() {
        let mut pool = SymbolPool::new();
        let semi = Node::BlockLocalVariableNode {
            flags: 0,
            span: span(),
            name: sym(&mut pool, "z"),
        };
        let inner = block(
            Some(block_params(
                Some(method_params(
                    vec![required(&mut pool, "a")],
                    vec![],
                    vec![],
                    vec![],
                    None,
                    None,
                    None,
                )),
                vec![semi],
            )),
            Some(stmts(vec![read(&mut pool, "z")])),
        );
        let mut root = program(stmts(vec![call_block(&mut pool, "foo", inner)]));
        resolve_scopes(&mut root, &mut pool);
        let body = program_stmts(&root).expect("program body");
        let flown = call_block_node(&body[0]).expect("call block");
        assert_eq!(
            block_locals(flown, &pool),
            Some(vec!["a".to_owned(), "z".to_owned()])
        );
        let inner_body = block_body(flown).expect("block body");
        assert_eq!(read_depth(&inner_body[0]), Some(0));
    }

    #[test]
    fn rescue_reference_binds_enclosing() {
        let mut pool = SymbolPool::new();
        let rescue = Node::RescueNode {
            flags: 0,
            span: span(),
            keyword_loc: span(),
            exceptions: vec![const_read(&mut pool, "E")],
            operator_loc: None,
            reference: Some(Box::new(target(&mut pool, "e"))),
            then_keyword_loc: None,
            statements: Some(Box::new(stmts(vec![read(&mut pool, "e")]))),
            subsequent: None,
        };
        let begin = Node::BeginNode {
            flags: 0,
            span: span(),
            begin_keyword_loc: None,
            statements: Some(Box::new(stmts(vec![nil()]))),
            rescue_clause: Some(Box::new(rescue)),
            else_clause: None,
            ensure_clause: None,
            end_keyword_loc: None,
        };
        let mut root = program(stmts(vec![begin]));
        resolve_scopes(&mut root, &mut pool);
        assert_eq!(program_locals(&root, &pool), Some(vec!["e".to_owned()]));
        let body = program_stmts(&root).expect("program body");
        if let Node::BeginNode {
            rescue_clause: Some(rescue),
            ..
        } = &body[0]
        {
            if let Node::RescueNode {
                reference: Some(reference),
                statements: Some(statements),
                ..
            } = rescue.as_ref()
            {
                assert_eq!(target_depth(reference), Some(0));
                let rescue_body = stmts_body(statements).expect("rescue body");
                assert_eq!(read_depth(&rescue_body[0]), Some(0));
            } else {
                panic!("expected rescue parts");
            }
        } else {
            panic!("expected begin node");
        }
    }

    #[test]
    fn class_body_is_barrier() {
        let mut pool = SymbolPool::new();
        let class = Node::ClassNode {
            flags: 0,
            span: span(),
            locals: Vec::new(),
            class_keyword_loc: span(),
            constant_path: Box::new(const_read(&mut pool, "C")),
            inheritance_operator_loc: None,
            superclass: None,
            body: Some(Box::new(stmts(vec![write(&mut pool, "x", int(2))]))),
            end_keyword_loc: span(),
            name: sym(&mut pool, "C"),
        };
        let mut root = program(stmts(vec![write(&mut pool, "x", int(1)), class]));
        resolve_scopes(&mut root, &mut pool);
        assert_eq!(program_locals(&root, &pool), Some(vec!["x".to_owned()]));
        let body = program_stmts(&root).expect("program body");
        if let Node::ClassNode {
            locals,
            body: Some(body),
            ..
        } = &body[1]
        {
            assert_eq!(local_names(&pool, locals), vec!["x".to_owned()]);
            let class_body = stmts_body(body).expect("class body");
            assert_eq!(write_depth(&class_body[0]), Some(0));
        } else {
            panic!("expected class node");
        }
    }

    #[test]
    fn anonymous_params_leave_no_locals() {
        let mut pool = SymbolPool::new();
        let rest = Node::RestParameterNode {
            flags: 0,
            span: span(),
            name: None,
            name_loc: None,
            operator_loc: span(),
        };
        let block_param = Node::BlockParameterNode {
            flags: 0,
            span: span(),
            name: None,
            name_loc: None,
            operator_loc: span(),
        };
        let params = method_params(
            vec![],
            vec![],
            vec![],
            vec![],
            Some(rest),
            None,
            Some(block_param),
        );
        let def_body = stmts(vec![write(&mut pool, "q", int(1))]);
        let method = def(&mut pool, "m", Some(params), Some(def_body));
        let mut root = program(stmts(vec![method]));
        resolve_scopes(&mut root, &mut pool);
        let body = program_stmts(&root).expect("program body");
        assert_eq!(def_locals(&body[0], &pool), Some(vec!["q".to_owned()]));
    }

    #[test]
    fn op_and_or_writes_bind() {
        let mut pool = SymbolPool::new();
        let add = Node::LocalVariableOperatorWriteNode {
            flags: 0,
            span: span(),
            name_loc: span(),
            binary_operator_loc: span(),
            value: Box::new(int(2)),
            name: sym(&mut pool, "x"),
            binary_operator: sym(&mut pool, "+"),
            depth: 0,
        };
        let or = Node::LocalVariableOrWriteNode {
            flags: 0,
            span: span(),
            name_loc: span(),
            operator_loc: span(),
            value: Box::new(int(1)),
            name: sym(&mut pool, "y"),
            depth: 0,
        };
        let mut root = program(stmts(vec![write(&mut pool, "x", int(1)), add, or]));
        resolve_scopes(&mut root, &mut pool);
        assert_eq!(
            program_locals(&root, &pool),
            Some(vec!["x".to_owned(), "y".to_owned()])
        );
        let body = program_stmts(&root).expect("program body");
        if let Node::LocalVariableOperatorWriteNode { depth, .. } = &body[1] {
            assert_eq!(*depth, 0);
        } else {
            panic!("expected operator write");
        }
        if let Node::LocalVariableOrWriteNode { depth, .. } = &body[2] {
            assert_eq!(*depth, 0);
        } else {
            panic!("expected or write");
        }
    }

    #[test]
    fn undefined_read_keeps_zero_depth() {
        let mut pool = SymbolPool::new();
        let mut root = program(stmts(vec![read(&mut pool, "zzz")]));
        resolve_scopes(&mut root, &mut pool);
        assert_eq!(program_locals(&root, &pool), Some(Vec::new()));
        let body = program_stmts(&root).expect("program body");
        assert_eq!(read_depth(&body[0]), Some(0));
    }

    #[test]
    fn keyword_and_block_param_order() {
        let mut pool = SymbolPool::new();
        let keyword = Node::RequiredKeywordParameterNode {
            flags: 0,
            span: span(),
            name: sym(&mut pool, "k"),
            name_loc: span(),
        };
        let block_param = Node::BlockParameterNode {
            flags: 0,
            span: span(),
            name: Some(sym(&mut pool, "b")),
            name_loc: Some(span()),
            operator_loc: span(),
        };
        let params = method_params(
            vec![required(&mut pool, "a")],
            vec![],
            vec![],
            vec![keyword],
            None,
            None,
            Some(block_param),
        );
        let method = def(&mut pool, "m", Some(params), Some(stmts(vec![nil()])));
        let mut root = program(stmts(vec![method]));
        resolve_scopes(&mut root, &mut pool);
        let body = program_stmts(&root).expect("program body");
        assert_eq!(
            def_locals(&body[0], &pool),
            Some(vec!["a".to_owned(), "b".to_owned(), "k".to_owned()])
        );
    }

    #[test]
    fn destructured_params_bind_leaves() {
        let mut pool = SymbolPool::new();
        let multi = Node::MultiTargetNode {
            flags: 0,
            span: span(),
            lefts: vec![required(&mut pool, "a"), required(&mut pool, "b")],
            rest: None,
            rights: vec![],
            lparen_loc: None,
            rparen_loc: None,
        };
        let params = method_params(vec![multi], vec![], vec![], vec![], None, None, None);
        let def_body = stmts(vec![read(&mut pool, "a"), read(&mut pool, "b")]);
        let method = def(&mut pool, "m", Some(params), Some(def_body));
        let mut root = program(stmts(vec![method]));
        resolve_scopes(&mut root, &mut pool);
        let body = program_stmts(&root).expect("program body");
        assert_eq!(
            def_locals(&body[0], &pool),
            Some(vec!["a".to_owned(), "b".to_owned()])
        );
        if let Node::DefNode {
            body: Some(body), ..
        } = &body[0]
        {
            let def_body = stmts_body(body).expect("def body");
            assert_eq!(read_depth(&def_body[0]), Some(0));
            assert_eq!(read_depth(&def_body[1]), Some(0));
        } else {
            panic!("expected def body");
        }
    }
}
