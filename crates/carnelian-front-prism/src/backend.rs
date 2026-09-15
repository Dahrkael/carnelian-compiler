//! `BackendNode` for borrowed nodes (thin access, no tree copies).

use carnelian_ast::view::{
    AlternationView, ArrayPatternView, BackendNode, BeginView, BlockParamView, BlockView,
    CallTargetView, CallView, CaptureView, CaseMatchView, CaseView, ClassView, ConstPathRead,
    ConstPathWrite, DefView, EnsureView, FindPatternView, ForView, GuardView, HashPatternView,
    IfView, InView, IndexTargetView, IntegerLit, KeywordParamView, LambdaView, LvarRef, LvarWrite,
    MatchView, ModuleView, MultiTargetView, MultiWriteView, ParamsView, ProgramView,
    RescueModifierView, RescueView, SclassView, SimpleLit, SuperView, VarWrite, WhenView,
    WhileView, YieldView,
};
use carnelian_ast::AstNode;

use crate::PrismNode;

fn wrap(node: ruby_prism::Node<'_>) -> PrismNode<'_> {
    PrismNode::new(node)
}

fn wrap_many(list: ruby_prism::NodeList<'_>) -> Vec<PrismNode<'_>> {
    list.iter().map(wrap).collect()
}

/// Statements of an `else` clause known to exist; a missing list is empty.
fn else_statements<'pr>(clause: &ruby_prism::ElseNode<'pr>) -> Vec<PrismNode<'pr>> {
    clause
        .statements()
        .map(|statements| wrap_many(statements.body()))
        .unwrap_or_default()
}

fn const_bytes(id: ruby_prism::ConstantId<'_>) -> Vec<u8> {
    id.as_slice().to_vec()
}

/// `PM_PARAMETER_FLAGS_NIL_BLOCK` of the patched reference Prism
/// (`&nil`, "method accepts no block").
const NIL_BLOCK_FLAG: u32 = 8;

/// Decimal digits of a little-endian base-2^32 limb slice, without leading
/// zeros (empty input reads as zero).
fn limbs_to_decimal(limbs: &[u32]) -> Vec<u8> {
    let mut words: Vec<u32> = limbs.to_vec();
    while words.last() == Some(&0) {
        words.pop();
    }
    if words.is_empty() {
        return vec![b'0'];
    }
    let mut digits = Vec::new();
    while !words.is_empty() {
        let mut remainder: u64 = 0;
        for index in (0..words.len()).rev() {
            let current = (remainder << 32) | u64::from(words[index]);
            words[index] = (current / 10) as u32;
            remainder = current % 10;
        }
        digits.push(b'0' + remainder as u8);
        while words.last() == Some(&0) {
            words.pop();
        }
    }
    digits.reverse();
    digits
}

impl BackendNode for PrismNode<'_> {
    fn integer_lit(&self) -> Option<IntegerLit> {
        let node = self.inner.as_integer_node()?;
        let value = node.value();
        let (negative, limbs) = value.to_u32_digits();
        if limbs.len() <= 4 {
            let mut magnitude: u128 = 0;
            for (index, limb) in limbs.iter().enumerate() {
                magnitude |= u128::from(*limb) << (32 * index);
            }
            let fits = if negative {
                magnitude <= 1u128 << 63
            } else {
                magnitude <= i64::MAX as u128
            };
            if fits {
                let scalar = if negative {
                    if magnitude == 1u128 << 63 {
                        i64::MIN
                    } else {
                        -(magnitude as i64)
                    }
                } else {
                    magnitude as i64
                };
                return Some(IntegerLit::I64(scalar));
            }
        }
        // Overflow: decimal digits of the value (`pm_integer_string`
        // stringifies, so source radix, underscores and leading zeros never
        // reach the pool).
        Some(IntegerLit::Bigint {
            digits: limbs_to_decimal(limbs),
            negative,
        })
    }

    fn float_lit(&self) -> Option<f64> {
        self.inner.as_float_node().map(|node| node.value())
    }

    fn string_lit(&self) -> Option<Vec<u8>> {
        self.inner
            .as_string_node()
            .map(|node| node.unescaped().to_vec())
    }

    fn symbol_lit(&self) -> Option<Vec<u8>> {
        self.inner
            .as_symbol_node()
            .map(|node| node.unescaped().to_vec())
    }

    fn call(&self) -> Option<CallView<Self>> {
        let node = self.inner.as_call_node()?;
        let flags = u32::from(node.flags());
        Some(CallView {
            name: const_bytes(node.name()),
            receiver: node.receiver().map(wrap),
            args: node.arguments().map(|arguments| wrap(arguments.as_node())),
            block: node.block().map(wrap),
            safe_nav: flags
                & ruby_prism_sys::pm_call_node_flags::PM_CALL_NODE_FLAGS_SAFE_NAVIGATION as u32
                != 0,
            attr_write: flags
                & ruby_prism_sys::pm_call_node_flags::PM_CALL_NODE_FLAGS_ATTRIBUTE_WRITE as u32
                != 0,
        })
    }

    fn call_args(&self) -> Option<Vec<Self>> {
        let node = self.inner.as_arguments_node()?;
        Some(node.arguments().iter().map(wrap).collect())
    }

    fn args_forwarding(&self) -> bool {
        self.inner
            .as_arguments_node()
            .is_some_and(|node| node.is_contains_forwarding())
    }

    fn if_branch(&self) -> Option<IfView<Self>> {
        if let Some(node) = self.inner.as_if_node() {
            return Some(IfView {
                predicate: Some(wrap(node.predicate())),
                then_body: node
                    .statements()
                    .map(|statements| wrap_many(statements.body())),
                else_body: node.subsequent().map(wrap),
                is_unless: false,
            });
        }
        let node = self.inner.as_unless_node()?;
        Some(IfView {
            predicate: Some(wrap(node.predicate())),
            then_body: node.else_clause().map(|clause| else_statements(&clause)),
            else_body: node
                .statements()
                .map(|statements| wrap(statements.as_node())),
            is_unless: true,
        })
    }

    fn array_elements(&self) -> Option<Vec<Self>> {
        let node = self.inner.as_array_node()?;
        Some(node.elements().iter().map(wrap).collect())
    }

    fn while_loop(&self) -> Option<WhileView<Self>> {
        if let Some(node) = self.inner.as_while_node() {
            return Some(WhileView {
                predicate: Some(wrap(node.predicate())),
                body: node
                    .statements()
                    .map(|statements| wrap_many(statements.body())),
                is_until: false,
                begin_modifier: u32::from(node.flags())
                    & ruby_prism_sys::pm_loop_flags::PM_LOOP_FLAGS_BEGIN_MODIFIER as u32
                    != 0,
            });
        }
        let node = self.inner.as_until_node()?;
        Some(WhileView {
            predicate: Some(wrap(node.predicate())),
            body: node
                .statements()
                .map(|statements| wrap_many(statements.body())),
            is_until: true,
            begin_modifier: u32::from(node.flags())
                & ruby_prism_sys::pm_loop_flags::PM_LOOP_FLAGS_BEGIN_MODIFIER as u32
                != 0,
        })
    }

    fn for_view(&self) -> Option<ForView<Self>> {
        let node = self.inner.as_for_node()?;
        Some(ForView {
            index: wrap(node.index()),
            collection: wrap(node.collection()),
            statements: node
                .statements()
                .map(|statements| wrap_many(statements.body())),
        })
    }

    fn statements(&self) -> Option<Vec<Self>> {
        self.inner
            .as_statements_node()
            .map(|node| wrap_many(node.body()))
    }

    fn logic(&self) -> Option<(Self, Self)> {
        if let Some(node) = self.inner.as_and_node() {
            return Some((wrap(node.left()), wrap(node.right())));
        }
        let node = self.inner.as_or_node()?;
        Some((wrap(node.left()), wrap(node.right())))
    }

    fn else_body(&self) -> Option<Vec<Self>> {
        let node = self.inner.as_else_node()?;
        node.statements()
            .map(|statements| wrap_many(statements.body()))
    }

    fn program(&self) -> Option<ProgramView<Self>> {
        let node = self.inner.as_program_node()?;
        Some(ProgramView {
            locals: node
                .locals()
                .iter()
                .map(|id| id.as_slice().to_vec())
                .collect(),
            body: wrap(node.statements().as_node()),
        })
    }

    fn lvar_read(&self) -> Option<LvarRef> {
        let node = self.inner.as_local_variable_read_node()?;
        Some(LvarRef {
            name: const_bytes(node.name()),
            depth: node.depth(),
        })
    }

    fn lvar_write(&self) -> Option<LvarWrite<Self>> {
        let node = self.inner.as_local_variable_write_node()?;
        Some(LvarWrite {
            name: const_bytes(node.name()),
            depth: node.depth(),
            value: Some(wrap(node.value())),
        })
    }

    fn simple_lit(&self) -> Option<SimpleLit> {
        if self.inner.as_true_node().is_some() {
            Some(SimpleLit::True)
        } else if self.inner.as_false_node().is_some() {
            Some(SimpleLit::False)
        } else if self.inner.as_nil_node().is_some() {
            Some(SimpleLit::Nil)
        } else if self.inner.as_self_node().is_some() {
            Some(SimpleLit::SelfValue)
        } else {
            None
        }
    }

    fn hash_elements(&self) -> Option<Vec<Self>> {
        if let Some(node) = self.inner.as_hash_node() {
            return Some(wrap_many(node.elements()));
        }
        let node = self.inner.as_keyword_hash_node()?;
        Some(wrap_many(node.elements()))
    }

    fn assoc_pair(&self) -> Option<(Self, Self)> {
        let node = self.inner.as_assoc_node()?;
        Some((wrap(node.key()), wrap(node.value())))
    }

    fn assoc_splat_value(&self) -> Option<Option<Self>> {
        let node = self.inner.as_assoc_splat_node()?;
        Some(node.value().map(wrap))
    }

    fn splat_value(&self) -> Option<Option<Self>> {
        let node = self.inner.as_splat_node()?;
        Some(node.expression().map(wrap))
    }

    fn case_view(&self) -> Option<CaseView<Self>> {
        let node = self.inner.as_case_node()?;
        Some(CaseView {
            predicate: node.predicate().map(wrap),
            whens: wrap_many(node.conditions()),
            else_body: node.else_clause().map(|clause| wrap(clause.as_node())),
        })
    }

    fn when_view(&self) -> Option<WhenView<Self>> {
        let node = self.inner.as_when_node()?;
        Some(WhenView {
            conditions: wrap_many(node.conditions()),
            body: node
                .statements()
                .map(|statements| wrap_many(statements.body())),
        })
    }

    fn case_match_view(&self) -> Option<CaseMatchView<Self>> {
        let node = self.inner.as_case_match_node()?;
        Some(CaseMatchView {
            predicate: node.predicate().map(wrap),
            conditions: wrap_many(node.conditions()),
            else_body: node.else_clause().map(|clause| wrap(clause.as_node())),
        })
    }

    fn in_view(&self) -> Option<InView<Self>> {
        let node = self.inner.as_in_node()?;
        Some(InView {
            pattern: wrap(node.pattern()),
            body: node
                .statements()
                .map(|statements| wrap_many(statements.body())),
        })
    }

    fn match_predicate_view(&self) -> Option<MatchView<Self>> {
        let node = self.inner.as_match_predicate_node()?;
        Some(MatchView {
            value: wrap(node.value()),
            pattern: wrap(node.pattern()),
        })
    }

    fn match_required_view(&self) -> Option<MatchView<Self>> {
        let node = self.inner.as_match_required_node()?;
        Some(MatchView {
            value: wrap(node.value()),
            pattern: wrap(node.pattern()),
        })
    }

    fn alternation_view(&self) -> Option<AlternationView<Self>> {
        let node = self.inner.as_alternation_pattern_node()?;
        Some(AlternationView {
            left: wrap(node.left()),
            right: wrap(node.right()),
        })
    }

    fn capture_view(&self) -> Option<CaptureView<Self>> {
        let node = self.inner.as_capture_pattern_node()?;
        Some(CaptureView {
            value: wrap(node.value()),
            target: wrap(node.target().as_node()),
        })
    }

    fn array_pattern_view(&self) -> Option<ArrayPatternView<Self>> {
        let node = self.inner.as_array_pattern_node()?;
        Some(ArrayPatternView {
            constant: node.constant().map(wrap),
            requireds: wrap_many(node.requireds()),
            rest: node.rest().map(wrap),
            posts: wrap_many(node.posts()),
        })
    }

    fn hash_pattern_view(&self) -> Option<HashPatternView<Self>> {
        let node = self.inner.as_hash_pattern_node()?;
        Some(HashPatternView {
            constant: node.constant().map(wrap),
            elements: wrap_many(node.elements()),
            rest: node.rest().map(wrap),
        })
    }

    fn find_pattern_view(&self) -> Option<FindPatternView<Self>> {
        let node = self.inner.as_find_pattern_node()?;
        Some(FindPatternView {
            constant: node.constant().map(wrap),
            left: wrap(node.left().as_node()),
            requireds: wrap_many(node.requireds()),
            right: wrap(node.right()),
        })
    }

    fn pinned_var(&self) -> Option<Self> {
        let node = self.inner.as_pinned_variable_node()?;
        Some(wrap(node.variable()))
    }

    fn pinned_expr(&self) -> Option<Self> {
        let node = self.inner.as_pinned_expression_node()?;
        Some(wrap(node.expression()))
    }

    fn guard_view(&self) -> Option<GuardView<Self>> {
        if let Some(node) = self.inner.as_if_node() {
            let statements = node.statements()?;
            let inner = wrap_many(statements.body()).into_iter().next()?;
            return Some(GuardView {
                inner,
                condition: wrap(node.predicate()),
                is_unless: false,
            });
        }
        let node = self.inner.as_unless_node()?;
        let statements = node.statements()?;
        let inner = wrap_many(statements.body()).into_iter().next()?;
        Some(GuardView {
            inner,
            condition: wrap(node.predicate()),
            is_unless: true,
        })
    }

    fn string_parts(&self) -> Option<Vec<Self>> {
        let node = self.inner.as_interpolated_string_node()?;
        Some(wrap_many(node.parts()))
    }

    fn embedded_body(&self) -> Option<Vec<Self>> {
        let node = self.inner.as_embedded_statements_node()?;
        // A null subtree is `None` (valued-only `LOADNIL` via `gen_branch`),
        // distinct from an empty node (`Some` empty vec). See `gen_branch`.
        let statements = node.statements()?;
        Some(wrap_many(statements.body()))
    }

    fn embedded_var(&self) -> Option<Self> {
        let node = self.inner.as_embedded_variable_node()?;
        Some(wrap(node.variable()))
    }

    fn block_view(&self) -> Option<BlockView<Self>> {
        let node = self.inner.as_block_node()?;
        Some(BlockView {
            locals: node
                .locals()
                .iter()
                .map(|id| id.as_slice().to_vec())
                .collect(),
            params: node.parameters().map(wrap),
            body: node.body().map(wrap),
        })
    }

    fn lambda_view(&self) -> Option<LambdaView<Self>> {
        let node = self.inner.as_lambda_node()?;
        Some(LambdaView {
            locals: node
                .locals()
                .iter()
                .map(|id| id.as_slice().to_vec())
                .collect(),
            params: node.parameters().map(wrap),
            body: node.body().map(wrap),
        })
    }

    fn yield_view(&self) -> Option<YieldView<Self>> {
        let node = self.inner.as_yield_node()?;
        Some(YieldView {
            args: node.arguments().map(|arguments| wrap(arguments.as_node())),
        })
    }

    fn block_param_view(&self) -> Option<BlockParamView<Self>> {
        let node = self.inner.as_block_parameters_node()?;
        Some(BlockParamView {
            params: node
                .parameters()
                .map(|parameters| wrap(parameters.as_node())),
            block_locals: wrap_many(node.locals()),
        })
    }

    fn parameters_view(&self) -> Option<ParamsView<Self>> {
        let node = self.inner.as_parameters_node()?;
        Some(ParamsView {
            requireds: wrap_many(node.requireds()),
            optionals: wrap_many(node.optionals()),
            rest: node.rest().map(wrap),
            posts: wrap_many(node.posts()),
            keywords: wrap_many(node.keywords()),
            keyword_rest: node.keyword_rest().map(wrap),
            block: node.block().map(|block| wrap(block.as_node())),
        })
    }

    fn required_param_name(&self) -> Option<Vec<u8>> {
        let node = self.inner.as_required_parameter_node()?;
        Some(const_bytes(node.name()))
    }

    fn rest_param_name(&self) -> Option<Option<Vec<u8>>> {
        let node = self.inner.as_rest_parameter_node()?;
        Some(node.name().map(|id| id.as_slice().to_vec()))
    }

    fn block_local_name(&self) -> Option<Vec<u8>> {
        let node = self.inner.as_block_local_variable_node()?;
        Some(const_bytes(node.name()))
    }

    fn numbered_max(&self) -> Option<u8> {
        let node = self.inner.as_numbered_parameters_node()?;
        Some(node.maximum())
    }

    fn block_arg(&self) -> Option<Option<Self>> {
        let node = self.inner.as_block_argument_node()?;
        Some(node.expression().map(wrap))
    }

    fn it_read(&self) -> Option<()> {
        self.inner.as_it_local_variable_read_node().map(|_| ())
    }

    fn def_view(&self) -> Option<DefView<Self>> {
        let node = self.inner.as_def_node()?;
        Some(DefView {
            name: const_bytes(node.name()),
            receiver: node.receiver().map(wrap),
            params: node
                .parameters()
                .map(|parameters| wrap(parameters.as_node())),
            body: node.body().map(wrap),
            locals: node
                .locals()
                .iter()
                .map(|id| id.as_slice().to_vec())
                .collect(),
        })
    }

    fn optional_param(&self) -> Option<(Vec<u8>, Self)> {
        let node = self.inner.as_optional_parameter_node()?;
        Some((const_bytes(node.name()), wrap(node.value())))
    }

    fn keyword_param(&self) -> Option<KeywordParamView<Self>> {
        if let Some(node) = self.inner.as_required_keyword_parameter_node() {
            return Some(KeywordParamView {
                name: const_bytes(node.name()),
                default: None,
            });
        }
        let node = self.inner.as_optional_keyword_parameter_node()?;
        Some(KeywordParamView {
            name: const_bytes(node.name()),
            default: Some(wrap(node.value())),
        })
    }

    fn keyword_rest_name(&self) -> Option<Option<Vec<u8>>> {
        let node = self.inner.as_keyword_rest_parameter_node()?;
        Some(node.name().map(|id| id.as_slice().to_vec()))
    }

    fn block_param_name(&self) -> Option<Option<Vec<u8>>> {
        let node = self.inner.as_block_parameter_node()?;
        Some(node.name().map(|id| id.as_slice().to_vec()))
    }

    fn block_param_noblock(&self) -> bool {
        // Patched reference Prism sets `PM_PARAMETER_FLAGS_NIL_BLOCK`;
        // upstream `ruby-prism` has no such flag constant, so test the bit.
        self.inner.as_block_parameter_node().is_some_and(|node| {
            (u32::from(node.flags()) & NIL_BLOCK_FLAG) != 0
                || node.name().is_some_and(|id| id.as_slice() == b"nil")
        })
    }

    fn class_view(&self) -> Option<ClassView<Self>> {
        let node = self.inner.as_class_node()?;
        let raw_path = node.constant_path();
        let path = wrap(raw_path);
        let (cpath_is_read, cpath_parent) = if path.kind_name() == "ConstantReadNode" {
            (true, None)
        } else if path.kind_name() == "ConstantPathNode" {
            let path_node = path.inner.as_constant_path_node()?;
            path_node.name()?;
            (false, path_node.parent().map(wrap))
        } else {
            return None;
        };
        Some(ClassView {
            name: const_bytes(node.name()),
            cpath_is_read,
            cpath_parent,
            superclass: node.superclass().map(wrap),
            body: node.body().map(wrap),
            locals: node
                .locals()
                .iter()
                .map(|id| id.as_slice().to_vec())
                .collect(),
        })
    }

    fn module_view(&self) -> Option<ModuleView<Self>> {
        let node = self.inner.as_module_node()?;
        let raw_path = node.constant_path();
        let path = wrap(raw_path);
        let (cpath_is_read, cpath_parent) = if path.kind_name() == "ConstantReadNode" {
            (true, None)
        } else if path.kind_name() == "ConstantPathNode" {
            let path_node = path.inner.as_constant_path_node()?;
            path_node.name()?;
            (false, path_node.parent().map(wrap))
        } else {
            return None;
        };
        Some(ModuleView {
            name: const_bytes(node.name()),
            cpath_is_read,
            cpath_parent,
            body: node.body().map(wrap),
            locals: node
                .locals()
                .iter()
                .map(|id| id.as_slice().to_vec())
                .collect(),
        })
    }

    fn sclass_view(&self) -> Option<SclassView<Self>> {
        let node = self.inner.as_singleton_class_node()?;
        Some(SclassView {
            expression: wrap(node.expression()),
            body: node.body().map(wrap),
            locals: node
                .locals()
                .iter()
                .map(|id| id.as_slice().to_vec())
                .collect(),
        })
    }

    fn const_read(&self) -> Option<Vec<u8>> {
        let node = self.inner.as_constant_read_node()?;
        Some(const_bytes(node.name()))
    }

    fn const_write(&self) -> Option<VarWrite<Self>> {
        let node = self.inner.as_constant_write_node()?;
        Some(VarWrite {
            name: const_bytes(node.name()),
            value: wrap(node.value()),
        })
    }

    fn const_path(&self) -> Option<ConstPathRead<Self>> {
        let node = self.inner.as_constant_path_node()?;
        Some(ConstPathRead {
            parent: node.parent().map(wrap),
            name: const_bytes(node.name()?),
        })
    }

    fn const_path_write(&self) -> Option<ConstPathWrite<Self>> {
        let node = self.inner.as_constant_path_write_node()?;
        let target = node.target();
        Some(ConstPathWrite {
            parent: target.parent().map(wrap),
            name: const_bytes(target.name()?),
            value: wrap(node.value()),
        })
    }

    fn ivar_read(&self) -> Option<Vec<u8>> {
        let node = self.inner.as_instance_variable_read_node()?;
        Some(const_bytes(node.name()))
    }

    fn ivar_write(&self) -> Option<VarWrite<Self>> {
        let node = self.inner.as_instance_variable_write_node()?;
        Some(VarWrite {
            name: const_bytes(node.name()),
            value: wrap(node.value()),
        })
    }

    fn cvar_read(&self) -> Option<Vec<u8>> {
        let node = self.inner.as_class_variable_read_node()?;
        Some(const_bytes(node.name()))
    }

    fn cvar_write(&self) -> Option<VarWrite<Self>> {
        let node = self.inner.as_class_variable_write_node()?;
        Some(VarWrite {
            name: const_bytes(node.name()),
            value: wrap(node.value()),
        })
    }

    fn gvar_read(&self) -> Option<Vec<u8>> {
        let node = self.inner.as_global_variable_read_node()?;
        Some(const_bytes(node.name()))
    }

    fn gvar_write(&self) -> Option<VarWrite<Self>> {
        let node = self.inner.as_global_variable_write_node()?;
        Some(VarWrite {
            name: const_bytes(node.name()),
            value: wrap(node.value()),
        })
    }

    fn super_view(&self) -> Option<SuperView<Self>> {
        let node = self.inner.as_super_node()?;
        if node.block().is_some() {
            return None;
        }
        let Some(arguments) = node.arguments() else {
            return Some(SuperView { args: None });
        };
        let mut out = Vec::new();
        for argument in arguments.arguments().iter() {
            let child = wrap(argument);
            // `...` rides `gen_values` like a splat; other complex
            // argument forms stay gated.
            if matches!(child.kind_name(), "SplatNode" | "KeywordHashNode") {
                return None;
            }
            out.push(child);
        }
        Some(SuperView { args: Some(out) })
    }

    fn forwarding_super(&self) -> Option<Option<Self>> {
        let node = self.inner.as_forwarding_super_node()?;
        Some(node.block().map(|block| wrap(block.as_node())))
    }

    fn lvar_target(&self) -> Option<LvarRef> {
        let node = self.inner.as_local_variable_target_node()?;
        Some(LvarRef {
            name: const_bytes(node.name()),
            depth: node.depth(),
        })
    }

    fn begin_view(&self) -> Option<BeginView<Self>> {
        let node = self.inner.as_begin_node()?;
        Some(BeginView {
            statements: node
                .statements()
                .map(|statements| wrap_many(statements.body())),
            bare: node.rescue_clause().is_none()
                && node.else_clause().is_none()
                && node.ensure_clause().is_none(),
            rescue_clause: node.rescue_clause().map(|clause| wrap(clause.as_node())),
            else_clause: node.else_clause().map(|clause| wrap(clause.as_node())),
            ensure_clause: node.ensure_clause().map(|clause| wrap(clause.as_node())),
        })
    }

    fn rescue_view(&self) -> Option<RescueView<Self>> {
        let node = self.inner.as_rescue_node()?;
        Some(RescueView {
            exceptions: wrap_many(node.exceptions()),
            reference: node.reference().map(wrap),
            statements: node
                .statements()
                .map(|statements| wrap_many(statements.body())),
            subsequent: node.subsequent().map(|clause| wrap(clause.as_node())),
        })
    }

    fn rescue_modifier_view(&self) -> Option<RescueModifierView<Self>> {
        let node = self.inner.as_rescue_modifier_node()?;
        Some(RescueModifierView {
            expression: wrap(node.expression()),
            rescue_expression: wrap(node.rescue_expression()),
        })
    }

    fn ensure_view(&self) -> Option<EnsureView<Self>> {
        let node = self.inner.as_ensure_node()?;
        Some(EnsureView {
            statements: node
                .statements()
                .map(|statements| wrap_many(statements.body())),
        })
    }

    fn multi_write_view(&self) -> Option<MultiWriteView<Self>> {
        let node = self.inner.as_multi_write_node()?;
        Some(MultiWriteView {
            lefts: wrap_many(node.lefts()),
            rest: node.rest().map(wrap),
            rights: wrap_many(node.rights()),
            value: wrap(node.value()),
        })
    }

    fn multi_target_view(&self) -> Option<MultiTargetView<Self>> {
        let node = self.inner.as_multi_target_node()?;
        Some(MultiTargetView {
            lefts: wrap_many(node.lefts()),
            rest: node.rest().map(wrap),
            rights: wrap_many(node.rights()),
        })
    }

    fn ivar_target_name(&self) -> Option<Vec<u8>> {
        let node = self.inner.as_instance_variable_target_node()?;
        Some(const_bytes(node.name()))
    }

    fn cvar_target_name(&self) -> Option<Vec<u8>> {
        let node = self.inner.as_class_variable_target_node()?;
        Some(const_bytes(node.name()))
    }

    fn gvar_target_name(&self) -> Option<Vec<u8>> {
        let node = self.inner.as_global_variable_target_node()?;
        Some(const_bytes(node.name()))
    }

    fn const_target_name(&self) -> Option<Vec<u8>> {
        let node = self.inner.as_constant_target_node()?;
        Some(const_bytes(node.name()))
    }

    fn const_path_target(&self) -> Option<(Option<Self>, Vec<u8>)> {
        let node = self.inner.as_constant_path_target_node()?;
        Some((node.parent().map(wrap), const_bytes(node.name()?)))
    }

    fn index_target(&self) -> Option<IndexTargetView<Self>> {
        let node = self.inner.as_index_target_node()?;
        Some(IndexTargetView {
            receiver: wrap(node.receiver()),
            args: node.arguments().map(|arguments| wrap(arguments.as_node())),
        })
    }

    fn call_target(&self) -> Option<CallTargetView<Self>> {
        let node = self.inner.as_call_target_node()?;
        Some(CallTargetView {
            receiver: wrap(node.receiver()),
            name: const_bytes(node.name()),
        })
    }

    fn alias_pair(&self) -> Option<(Self, Self)> {
        let node = self.inner.as_alias_method_node()?;
        Some((wrap(node.new_name()), wrap(node.old_name())))
    }

    fn undef_list(&self) -> Option<Vec<Self>> {
        let node = self.inner.as_undef_node()?;
        Some(wrap_many(node.names()))
    }

    fn defined_value(&self) -> Option<Self> {
        let node = self.inner.as_defined_node()?;
        Some(wrap(node.value()))
    }

    fn implicit_value(&self) -> Option<Self> {
        let node = self.inner.as_implicit_node()?;
        Some(wrap(node.value()))
    }

    fn parentheses_body(&self) -> Option<Option<Self>> {
        let node = self.inner.as_parentheses_node()?;
        Some(node.body().map(wrap))
    }

    fn instance_var_read_name(&self) -> Option<Vec<u8>> {
        let node = self.inner.as_instance_variable_read_node()?;
        Some(const_bytes(node.name()))
    }

    fn backref_name(&self) -> Option<Vec<u8>> {
        let node = self.inner.as_back_reference_read_node()?;
        Some(const_bytes(node.name()))
    }

    fn numbered_ref_number(&self) -> Option<u32> {
        let node = self.inner.as_numbered_reference_read_node()?;
        Some(node.number())
    }

    fn global_var_read_name(&self) -> Option<Vec<u8>> {
        let node = self.inner.as_global_variable_read_node()?;
        Some(const_bytes(node.name()))
    }

    fn class_var_read_name(&self) -> Option<Vec<u8>> {
        let node = self.inner.as_class_variable_read_node()?;
        Some(const_bytes(node.name()))
    }

    fn constant_read_name(&self) -> Option<Vec<u8>> {
        let node = self.inner.as_constant_read_node()?;
        Some(const_bytes(node.name()))
    }

    fn constant_path_parts(&self) -> Option<(Option<Self>, Vec<u8>)> {
        let node = self.inner.as_constant_path_node()?;
        Some((
            node.parent().map(wrap),
            node.name().map(const_bytes).unwrap_or_default(),
        ))
    }

    fn raw_call_args(&self) -> Option<Vec<Self>> {
        let node = self.inner.as_arguments_node()?;
        Some(node.arguments().iter().map(wrap).collect())
    }

    fn raw_array_elements(&self) -> Option<Vec<Self>> {
        let node = self.inner.as_array_node()?;
        Some(node.elements().iter().map(wrap).collect())
    }
}
