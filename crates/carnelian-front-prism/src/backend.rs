//! `BackendNode` for borrowed nodes (thin access, no tree copies).

use carnelian_ast::view::{
    BackendNode, BeginView, BlockParamView, BlockView, CallView, CaseView, ClassView,
    ConstPathRead, ConstPathWrite, DefView, EnsureView, IfView, IntegerLit, LambdaView, LvarRef,
    LvarWrite, ModuleView, MultiTargetView, MultiWriteView, ParamsView, ProgramView,
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
        // Overflow: decimal source digits (mirrors `pm_integer_string`).
        let mut text = node.location().as_slice();
        if negative && !text.is_empty() && (text[0] == b'-' || text[0] == b'+') {
            text = &text[1..];
        }
        let digits: Vec<u8> = text.iter().copied().filter(|b| *b != b'_').collect();
        if digits.is_empty() || !digits.iter().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Some(IntegerLit::Bigint { digits, negative })
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
        let mut required_params = Vec::new();
        if let Some(params) = node.parameters() {
            if !params.optionals().is_empty() {
                return None;
            }
            if params.rest().is_some() {
                return None;
            }
            if !params.posts().is_empty() {
                return None;
            }
            if !params.keywords().is_empty() {
                return None;
            }
            if params.keyword_rest().is_some() {
                return None;
            }
            if params.block().is_some() {
                return None;
            }
            for argument in params.requireds().iter() {
                let child = wrap(argument);
                let param = child.inner.as_required_parameter_node()?;
                required_params.push(const_bytes(param.name()));
            }
        }
        Some(DefView {
            name: const_bytes(node.name()),
            receiver: node.receiver().map(wrap),
            required_params,
            body: node.body().map(wrap),
            locals: node
                .locals()
                .iter()
                .map(|id| id.as_slice().to_vec())
                .collect(),
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
            if matches!(
                child.kind_name(),
                "SplatNode" | "KeywordHashNode" | "ForwardingArgumentsNode"
            ) {
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
}
