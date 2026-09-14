//! `BackendNode` for borrowed nodes (thin access, no tree copies).

use carnelian_ast::view::{
    BackendNode, CallView, IfView, IntegerLit, LvarRef, LvarWrite, ProgramView, SimpleLit,
    WhileView,
};
use carnelian_ast::AstNode;

use crate::PrismNode;

fn wrap(node: ruby_prism::Node<'_>) -> PrismNode<'_> {
    PrismNode::new(node)
}

fn wrap_many(list: ruby_prism::NodeList<'_>) -> Vec<PrismNode<'_>> {
    list.iter().map(wrap).collect()
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
                magnitude <= 1 << 63
            } else {
                magnitude <= i64::MAX as u128
            };
            if fits {
                let scalar = if negative {
                    if magnitude == 1 << 63 {
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
        let mut out = Vec::new();
        for argument in node.arguments().iter() {
            let child = wrap(argument);
            // Forms with dedicated emission paths (splat flush, keyword
            // hashes, forwarding) are gated to their tranches.
            if matches!(
                child.kind_name(),
                "SplatNode" | "KeywordHashNode" | "ForwardingArgumentsNode"
            ) {
                return None;
            }
            out.push(child);
        }
        Some(out)
    }

    fn if_branch(&self) -> Option<IfView<Self>> {
        if let Some(node) = self.inner.as_if_node() {
            return Some(IfView {
                predicate: Some(wrap(node.predicate())),
                then_body: node
                    .statements()
                    .map(|statements| wrap(statements.as_node())),
                else_body: node.subsequent().map(wrap),
                is_unless: false,
            });
        }
        let node = self.inner.as_unless_node()?;
        Some(IfView {
            predicate: Some(wrap(node.predicate())),
            then_body: node.else_clause().map(|clause| wrap(clause.as_node())),
            else_body: node
                .statements()
                .map(|statements| wrap(statements.as_node())),
            is_unless: true,
        })
    }

    fn array_elements(&self) -> Option<Vec<Self>> {
        let node = self.inner.as_array_node()?;
        let mut out = Vec::new();
        for element in node.elements().iter() {
            let child = wrap(element);
            if child.kind_name() == "SplatNode" {
                return None;
            }
            out.push(child);
        }
        Some(out)
    }

    fn while_loop(&self) -> Option<WhileView<Self>> {
        if let Some(node) = self.inner.as_while_node() {
            return Some(WhileView {
                predicate: Some(wrap(node.predicate())),
                body: node
                    .statements()
                    .map(|statements| wrap(statements.as_node())),
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
                .map(|statements| wrap(statements.as_node())),
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
        Some(match node.statements() {
            Some(statements) => vec![wrap(statements.as_node())],
            None => Vec::new(),
        })
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
}
