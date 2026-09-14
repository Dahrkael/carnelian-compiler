//! FFI lowering: borrowed `ruby-prism` tree to the owned AST (P3.1).
//!
//! Dev/CLI only. Maps all 151 node kinds 1:1: `node?` becomes
//! `Option<Box<Node>>`, `node[]` becomes `Vec<Node>`, `constant` fields
//! intern into the returned `SymbolPool`, `location` fields become offsets,
//! `integer` fields become the `Integer` model and flags stay raw `u16`.

use carnelian_ast::{Integer, Node, Span, SymbolPool};

use crate::PrismNode;

/// Lower a parsed FFI root to its owned tree plus symbol pool.
pub fn lower(root: PrismNode<'_>) -> (Node, SymbolPool) {
    let mut pool = SymbolPool::new();
    let node = conv(root.inner(), &mut pool);
    (node, pool)
}

/// Lower one borrowed node with its whole subtree.
fn conv(node: &ruby_prism::Node<'_>, pool: &mut SymbolPool) -> Node {
    match node {
        ruby_prism::Node::AliasGlobalVariableNode { .. } => {
            let typed = node
                .as_alias_global_variable_node()
                .expect("lower: kind mismatch");
            Node::AliasGlobalVariableNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                new_name: Box::new(conv(&typed.new_name(), pool)),
                old_name: Box::new(conv(&typed.old_name(), pool)),
                keyword_loc: span_of(&typed.keyword_loc()),
            }
        }
        ruby_prism::Node::AliasMethodNode { .. } => {
            let typed = node.as_alias_method_node().expect("lower: kind mismatch");
            Node::AliasMethodNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                new_name: Box::new(conv(&typed.new_name(), pool)),
                old_name: Box::new(conv(&typed.old_name(), pool)),
                keyword_loc: span_of(&typed.keyword_loc()),
            }
        }
        ruby_prism::Node::AlternationPatternNode { .. } => {
            let typed = node
                .as_alternation_pattern_node()
                .expect("lower: kind mismatch");
            Node::AlternationPatternNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                left: Box::new(conv(&typed.left(), pool)),
                right: Box::new(conv(&typed.right(), pool)),
                operator_loc: span_of(&typed.operator_loc()),
            }
        }
        ruby_prism::Node::AndNode { .. } => {
            let typed = node.as_and_node().expect("lower: kind mismatch");
            Node::AndNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                left: Box::new(conv(&typed.left(), pool)),
                right: Box::new(conv(&typed.right(), pool)),
                operator_loc: span_of(&typed.operator_loc()),
            }
        }
        ruby_prism::Node::ArgumentsNode { .. } => {
            let typed = node.as_arguments_node().expect("lower: kind mismatch");
            Node::ArgumentsNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                arguments: typed
                    .arguments()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
            }
        }
        ruby_prism::Node::ArrayNode { .. } => {
            let typed = node.as_array_node().expect("lower: kind mismatch");
            Node::ArrayNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                elements: typed
                    .elements()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                opening_loc: typed.opening_loc().map(|loc| span_of(&loc)),
                closing_loc: typed.closing_loc().map(|loc| span_of(&loc)),
            }
        }
        ruby_prism::Node::ArrayPatternNode { .. } => {
            let typed = node.as_array_pattern_node().expect("lower: kind mismatch");
            Node::ArrayPatternNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                constant: typed.constant().map(|child| Box::new(conv(&child, pool))),
                requireds: typed
                    .requireds()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                rest: typed.rest().map(|child| Box::new(conv(&child, pool))),
                posts: typed
                    .posts()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                opening_loc: typed.opening_loc().map(|loc| span_of(&loc)),
                closing_loc: typed.closing_loc().map(|loc| span_of(&loc)),
            }
        }
        ruby_prism::Node::AssocNode { .. } => {
            let typed = node.as_assoc_node().expect("lower: kind mismatch");
            Node::AssocNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                key: Box::new(conv(&typed.key(), pool)),
                value: Box::new(conv(&typed.value(), pool)),
                operator_loc: typed.operator_loc().map(|loc| span_of(&loc)),
            }
        }
        ruby_prism::Node::AssocSplatNode { .. } => {
            let typed = node.as_assoc_splat_node().expect("lower: kind mismatch");
            Node::AssocSplatNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                value: typed.value().map(|child| Box::new(conv(&child, pool))),
                operator_loc: span_of(&typed.operator_loc()),
            }
        }
        ruby_prism::Node::BackReferenceReadNode { .. } => {
            let typed = node
                .as_back_reference_read_node()
                .expect("lower: kind mismatch");
            Node::BackReferenceReadNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
            }
        }
        ruby_prism::Node::BeginNode { .. } => {
            let typed = node.as_begin_node().expect("lower: kind mismatch");
            Node::BeginNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                begin_keyword_loc: typed.begin_keyword_loc().map(|loc| span_of(&loc)),
                statements: typed
                    .statements()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                rescue_clause: typed
                    .rescue_clause()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                else_clause: typed
                    .else_clause()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                ensure_clause: typed
                    .ensure_clause()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                end_keyword_loc: typed.end_keyword_loc().map(|loc| span_of(&loc)),
            }
        }
        ruby_prism::Node::BlockArgumentNode { .. } => {
            let typed = node.as_block_argument_node().expect("lower: kind mismatch");
            Node::BlockArgumentNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                expression: typed.expression().map(|child| Box::new(conv(&child, pool))),
                operator_loc: span_of(&typed.operator_loc()),
            }
        }
        ruby_prism::Node::BlockLocalVariableNode { .. } => {
            let typed = node
                .as_block_local_variable_node()
                .expect("lower: kind mismatch");
            Node::BlockLocalVariableNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
            }
        }
        ruby_prism::Node::BlockNode { .. } => {
            let typed = node.as_block_node().expect("lower: kind mismatch");
            Node::BlockNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                locals: typed
                    .locals()
                    .iter()
                    .map(|id| pool.intern(id.as_slice()))
                    .collect(),
                parameters: typed.parameters().map(|child| Box::new(conv(&child, pool))),
                body: typed.body().map(|child| Box::new(conv(&child, pool))),
                opening_loc: span_of(&typed.opening_loc()),
                closing_loc: span_of(&typed.closing_loc()),
            }
        }
        ruby_prism::Node::BlockParameterNode { .. } => {
            let typed = node
                .as_block_parameter_node()
                .expect("lower: kind mismatch");
            Node::BlockParameterNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: typed.name().map(|id| pool.intern(id.as_slice())),
                name_loc: typed.name_loc().map(|loc| span_of(&loc)),
                operator_loc: span_of(&typed.operator_loc()),
            }
        }
        ruby_prism::Node::BlockParametersNode { .. } => {
            let typed = node
                .as_block_parameters_node()
                .expect("lower: kind mismatch");
            Node::BlockParametersNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                parameters: typed
                    .parameters()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                locals: typed
                    .locals()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                opening_loc: typed.opening_loc().map(|loc| span_of(&loc)),
                closing_loc: typed.closing_loc().map(|loc| span_of(&loc)),
            }
        }
        ruby_prism::Node::BreakNode { .. } => {
            let typed = node.as_break_node().expect("lower: kind mismatch");
            Node::BreakNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                arguments: typed
                    .arguments()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                keyword_loc: span_of(&typed.keyword_loc()),
            }
        }
        ruby_prism::Node::CallAndWriteNode { .. } => {
            let typed = node.as_call_and_write_node().expect("lower: kind mismatch");
            Node::CallAndWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                receiver: typed.receiver().map(|child| Box::new(conv(&child, pool))),
                call_operator_loc: typed.call_operator_loc().map(|loc| span_of(&loc)),
                message_loc: typed.message_loc().map(|loc| span_of(&loc)),
                read_name: pool.intern(typed.read_name().as_slice()),
                write_name: pool.intern(typed.write_name().as_slice()),
                operator_loc: span_of(&typed.operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
            }
        }
        ruby_prism::Node::CallNode { .. } => {
            let typed = node.as_call_node().expect("lower: kind mismatch");
            Node::CallNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                receiver: typed.receiver().map(|child| Box::new(conv(&child, pool))),
                call_operator_loc: typed.call_operator_loc().map(|loc| span_of(&loc)),
                name: pool.intern(typed.name().as_slice()),
                message_loc: typed.message_loc().map(|loc| span_of(&loc)),
                opening_loc: typed.opening_loc().map(|loc| span_of(&loc)),
                arguments: typed
                    .arguments()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                closing_loc: typed.closing_loc().map(|loc| span_of(&loc)),
                equal_loc: typed.equal_loc().map(|loc| span_of(&loc)),
                block: typed.block().map(|child| Box::new(conv(&child, pool))),
            }
        }
        ruby_prism::Node::CallOperatorWriteNode { .. } => {
            let typed = node
                .as_call_operator_write_node()
                .expect("lower: kind mismatch");
            Node::CallOperatorWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                receiver: typed.receiver().map(|child| Box::new(conv(&child, pool))),
                call_operator_loc: typed.call_operator_loc().map(|loc| span_of(&loc)),
                message_loc: typed.message_loc().map(|loc| span_of(&loc)),
                read_name: pool.intern(typed.read_name().as_slice()),
                write_name: pool.intern(typed.write_name().as_slice()),
                binary_operator: pool.intern(typed.binary_operator().as_slice()),
                binary_operator_loc: span_of(&typed.binary_operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
            }
        }
        ruby_prism::Node::CallOrWriteNode { .. } => {
            let typed = node.as_call_or_write_node().expect("lower: kind mismatch");
            Node::CallOrWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                receiver: typed.receiver().map(|child| Box::new(conv(&child, pool))),
                call_operator_loc: typed.call_operator_loc().map(|loc| span_of(&loc)),
                message_loc: typed.message_loc().map(|loc| span_of(&loc)),
                read_name: pool.intern(typed.read_name().as_slice()),
                write_name: pool.intern(typed.write_name().as_slice()),
                operator_loc: span_of(&typed.operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
            }
        }
        ruby_prism::Node::CallTargetNode { .. } => {
            let typed = node.as_call_target_node().expect("lower: kind mismatch");
            Node::CallTargetNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                receiver: Box::new(conv(&typed.receiver(), pool)),
                call_operator_loc: span_of(&typed.call_operator_loc()),
                name: pool.intern(typed.name().as_slice()),
                message_loc: span_of(&typed.message_loc()),
            }
        }
        ruby_prism::Node::CapturePatternNode { .. } => {
            let typed = node
                .as_capture_pattern_node()
                .expect("lower: kind mismatch");
            Node::CapturePatternNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                value: Box::new(conv(&typed.value(), pool)),
                target: Box::new(conv(&typed.target().as_node(), pool)),
                operator_loc: span_of(&typed.operator_loc()),
            }
        }
        ruby_prism::Node::CaseMatchNode { .. } => {
            let typed = node.as_case_match_node().expect("lower: kind mismatch");
            Node::CaseMatchNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                predicate: typed.predicate().map(|child| Box::new(conv(&child, pool))),
                conditions: typed
                    .conditions()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                else_clause: typed
                    .else_clause()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                case_keyword_loc: span_of(&typed.case_keyword_loc()),
                end_keyword_loc: span_of(&typed.end_keyword_loc()),
            }
        }
        ruby_prism::Node::CaseNode { .. } => {
            let typed = node.as_case_node().expect("lower: kind mismatch");
            Node::CaseNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                predicate: typed.predicate().map(|child| Box::new(conv(&child, pool))),
                conditions: typed
                    .conditions()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                else_clause: typed
                    .else_clause()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                case_keyword_loc: span_of(&typed.case_keyword_loc()),
                end_keyword_loc: span_of(&typed.end_keyword_loc()),
            }
        }
        ruby_prism::Node::ClassNode { .. } => {
            let typed = node.as_class_node().expect("lower: kind mismatch");
            Node::ClassNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                locals: typed
                    .locals()
                    .iter()
                    .map(|id| pool.intern(id.as_slice()))
                    .collect(),
                class_keyword_loc: span_of(&typed.class_keyword_loc()),
                constant_path: Box::new(conv(&typed.constant_path(), pool)),
                inheritance_operator_loc: typed.inheritance_operator_loc().map(|loc| span_of(&loc)),
                superclass: typed.superclass().map(|child| Box::new(conv(&child, pool))),
                body: typed.body().map(|child| Box::new(conv(&child, pool))),
                end_keyword_loc: span_of(&typed.end_keyword_loc()),
                name: pool.intern(typed.name().as_slice()),
            }
        }
        ruby_prism::Node::ClassVariableAndWriteNode { .. } => {
            let typed = node
                .as_class_variable_and_write_node()
                .expect("lower: kind mismatch");
            Node::ClassVariableAndWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
                name_loc: span_of(&typed.name_loc()),
                operator_loc: span_of(&typed.operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
            }
        }
        ruby_prism::Node::ClassVariableOperatorWriteNode { .. } => {
            let typed = node
                .as_class_variable_operator_write_node()
                .expect("lower: kind mismatch");
            Node::ClassVariableOperatorWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
                name_loc: span_of(&typed.name_loc()),
                binary_operator_loc: span_of(&typed.binary_operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
                binary_operator: pool.intern(typed.binary_operator().as_slice()),
            }
        }
        ruby_prism::Node::ClassVariableOrWriteNode { .. } => {
            let typed = node
                .as_class_variable_or_write_node()
                .expect("lower: kind mismatch");
            Node::ClassVariableOrWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
                name_loc: span_of(&typed.name_loc()),
                operator_loc: span_of(&typed.operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
            }
        }
        ruby_prism::Node::ClassVariableReadNode { .. } => {
            let typed = node
                .as_class_variable_read_node()
                .expect("lower: kind mismatch");
            Node::ClassVariableReadNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
            }
        }
        ruby_prism::Node::ClassVariableTargetNode { .. } => {
            let typed = node
                .as_class_variable_target_node()
                .expect("lower: kind mismatch");
            Node::ClassVariableTargetNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
            }
        }
        ruby_prism::Node::ClassVariableWriteNode { .. } => {
            let typed = node
                .as_class_variable_write_node()
                .expect("lower: kind mismatch");
            Node::ClassVariableWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
                name_loc: span_of(&typed.name_loc()),
                value: Box::new(conv(&typed.value(), pool)),
                operator_loc: span_of(&typed.operator_loc()),
            }
        }
        ruby_prism::Node::ConstantAndWriteNode { .. } => {
            let typed = node
                .as_constant_and_write_node()
                .expect("lower: kind mismatch");
            Node::ConstantAndWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
                name_loc: span_of(&typed.name_loc()),
                operator_loc: span_of(&typed.operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
            }
        }
        ruby_prism::Node::ConstantOperatorWriteNode { .. } => {
            let typed = node
                .as_constant_operator_write_node()
                .expect("lower: kind mismatch");
            Node::ConstantOperatorWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
                name_loc: span_of(&typed.name_loc()),
                binary_operator_loc: span_of(&typed.binary_operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
                binary_operator: pool.intern(typed.binary_operator().as_slice()),
            }
        }
        ruby_prism::Node::ConstantOrWriteNode { .. } => {
            let typed = node
                .as_constant_or_write_node()
                .expect("lower: kind mismatch");
            Node::ConstantOrWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
                name_loc: span_of(&typed.name_loc()),
                operator_loc: span_of(&typed.operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
            }
        }
        ruby_prism::Node::ConstantPathAndWriteNode { .. } => {
            let typed = node
                .as_constant_path_and_write_node()
                .expect("lower: kind mismatch");
            Node::ConstantPathAndWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                target: Box::new(conv(&typed.target().as_node(), pool)),
                operator_loc: span_of(&typed.operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
            }
        }
        ruby_prism::Node::ConstantPathNode { .. } => {
            let typed = node.as_constant_path_node().expect("lower: kind mismatch");
            Node::ConstantPathNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                parent: typed.parent().map(|child| Box::new(conv(&child, pool))),
                name: typed.name().map(|id| pool.intern(id.as_slice())),
                delimiter_loc: span_of(&typed.delimiter_loc()),
                name_loc: span_of(&typed.name_loc()),
            }
        }
        ruby_prism::Node::ConstantPathOperatorWriteNode { .. } => {
            let typed = node
                .as_constant_path_operator_write_node()
                .expect("lower: kind mismatch");
            Node::ConstantPathOperatorWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                target: Box::new(conv(&typed.target().as_node(), pool)),
                binary_operator_loc: span_of(&typed.binary_operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
                binary_operator: pool.intern(typed.binary_operator().as_slice()),
            }
        }
        ruby_prism::Node::ConstantPathOrWriteNode { .. } => {
            let typed = node
                .as_constant_path_or_write_node()
                .expect("lower: kind mismatch");
            Node::ConstantPathOrWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                target: Box::new(conv(&typed.target().as_node(), pool)),
                operator_loc: span_of(&typed.operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
            }
        }
        ruby_prism::Node::ConstantPathTargetNode { .. } => {
            let typed = node
                .as_constant_path_target_node()
                .expect("lower: kind mismatch");
            Node::ConstantPathTargetNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                parent: typed.parent().map(|child| Box::new(conv(&child, pool))),
                name: typed.name().map(|id| pool.intern(id.as_slice())),
                delimiter_loc: span_of(&typed.delimiter_loc()),
                name_loc: span_of(&typed.name_loc()),
            }
        }
        ruby_prism::Node::ConstantPathWriteNode { .. } => {
            let typed = node
                .as_constant_path_write_node()
                .expect("lower: kind mismatch");
            Node::ConstantPathWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                target: Box::new(conv(&typed.target().as_node(), pool)),
                operator_loc: span_of(&typed.operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
            }
        }
        ruby_prism::Node::ConstantReadNode { .. } => {
            let typed = node.as_constant_read_node().expect("lower: kind mismatch");
            Node::ConstantReadNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
            }
        }
        ruby_prism::Node::ConstantTargetNode { .. } => {
            let typed = node
                .as_constant_target_node()
                .expect("lower: kind mismatch");
            Node::ConstantTargetNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
            }
        }
        ruby_prism::Node::ConstantWriteNode { .. } => {
            let typed = node.as_constant_write_node().expect("lower: kind mismatch");
            Node::ConstantWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
                name_loc: span_of(&typed.name_loc()),
                value: Box::new(conv(&typed.value(), pool)),
                operator_loc: span_of(&typed.operator_loc()),
            }
        }
        ruby_prism::Node::DefNode { .. } => {
            let typed = node.as_def_node().expect("lower: kind mismatch");
            Node::DefNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
                name_loc: span_of(&typed.name_loc()),
                receiver: typed.receiver().map(|child| Box::new(conv(&child, pool))),
                parameters: typed
                    .parameters()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                body: typed.body().map(|child| Box::new(conv(&child, pool))),
                locals: typed
                    .locals()
                    .iter()
                    .map(|id| pool.intern(id.as_slice()))
                    .collect(),
                def_keyword_loc: span_of(&typed.def_keyword_loc()),
                operator_loc: typed.operator_loc().map(|loc| span_of(&loc)),
                lparen_loc: typed.lparen_loc().map(|loc| span_of(&loc)),
                rparen_loc: typed.rparen_loc().map(|loc| span_of(&loc)),
                equal_loc: typed.equal_loc().map(|loc| span_of(&loc)),
                end_keyword_loc: typed.end_keyword_loc().map(|loc| span_of(&loc)),
            }
        }
        ruby_prism::Node::DefinedNode { .. } => {
            let typed = node.as_defined_node().expect("lower: kind mismatch");
            Node::DefinedNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                lparen_loc: typed.lparen_loc().map(|loc| span_of(&loc)),
                value: Box::new(conv(&typed.value(), pool)),
                rparen_loc: typed.rparen_loc().map(|loc| span_of(&loc)),
                keyword_loc: span_of(&typed.keyword_loc()),
            }
        }
        ruby_prism::Node::ElseNode { .. } => {
            let typed = node.as_else_node().expect("lower: kind mismatch");
            Node::ElseNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                else_keyword_loc: span_of(&typed.else_keyword_loc()),
                statements: typed
                    .statements()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                end_keyword_loc: typed.end_keyword_loc().map(|loc| span_of(&loc)),
            }
        }
        ruby_prism::Node::EmbeddedStatementsNode { .. } => {
            let typed = node
                .as_embedded_statements_node()
                .expect("lower: kind mismatch");
            Node::EmbeddedStatementsNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                opening_loc: span_of(&typed.opening_loc()),
                statements: typed
                    .statements()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                closing_loc: span_of(&typed.closing_loc()),
            }
        }
        ruby_prism::Node::EmbeddedVariableNode { .. } => {
            let typed = node
                .as_embedded_variable_node()
                .expect("lower: kind mismatch");
            Node::EmbeddedVariableNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                operator_loc: span_of(&typed.operator_loc()),
                variable: Box::new(conv(&typed.variable(), pool)),
            }
        }
        ruby_prism::Node::EnsureNode { .. } => {
            let typed = node.as_ensure_node().expect("lower: kind mismatch");
            Node::EnsureNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                ensure_keyword_loc: span_of(&typed.ensure_keyword_loc()),
                statements: typed
                    .statements()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                end_keyword_loc: span_of(&typed.end_keyword_loc()),
            }
        }
        ruby_prism::Node::FalseNode { .. } => {
            let typed = node.as_false_node().expect("lower: kind mismatch");
            Node::FalseNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
            }
        }
        ruby_prism::Node::FindPatternNode { .. } => {
            let typed = node.as_find_pattern_node().expect("lower: kind mismatch");
            Node::FindPatternNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                constant: typed.constant().map(|child| Box::new(conv(&child, pool))),
                left: Box::new(conv(&typed.left().as_node(), pool)),
                requireds: typed
                    .requireds()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                right: Box::new(conv(&typed.right(), pool)),
                opening_loc: typed.opening_loc().map(|loc| span_of(&loc)),
                closing_loc: typed.closing_loc().map(|loc| span_of(&loc)),
            }
        }
        ruby_prism::Node::FlipFlopNode { .. } => {
            let typed = node.as_flip_flop_node().expect("lower: kind mismatch");
            Node::FlipFlopNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                left: typed.left().map(|child| Box::new(conv(&child, pool))),
                right: typed.right().map(|child| Box::new(conv(&child, pool))),
                operator_loc: span_of(&typed.operator_loc()),
            }
        }
        ruby_prism::Node::FloatNode { .. } => {
            let typed = node.as_float_node().expect("lower: kind mismatch");
            Node::FloatNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                value: typed.value(),
            }
        }
        ruby_prism::Node::ForNode { .. } => {
            let typed = node.as_for_node().expect("lower: kind mismatch");
            Node::ForNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                index: Box::new(conv(&typed.index(), pool)),
                collection: Box::new(conv(&typed.collection(), pool)),
                statements: typed
                    .statements()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                for_keyword_loc: span_of(&typed.for_keyword_loc()),
                in_keyword_loc: span_of(&typed.in_keyword_loc()),
                do_keyword_loc: typed.do_keyword_loc().map(|loc| span_of(&loc)),
                end_keyword_loc: span_of(&typed.end_keyword_loc()),
            }
        }
        ruby_prism::Node::ForwardingArgumentsNode { .. } => {
            let typed = node
                .as_forwarding_arguments_node()
                .expect("lower: kind mismatch");
            Node::ForwardingArgumentsNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
            }
        }
        ruby_prism::Node::ForwardingParameterNode { .. } => {
            let typed = node
                .as_forwarding_parameter_node()
                .expect("lower: kind mismatch");
            Node::ForwardingParameterNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
            }
        }
        ruby_prism::Node::ForwardingSuperNode { .. } => {
            let typed = node
                .as_forwarding_super_node()
                .expect("lower: kind mismatch");
            Node::ForwardingSuperNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                block: typed
                    .block()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
            }
        }
        ruby_prism::Node::GlobalVariableAndWriteNode { .. } => {
            let typed = node
                .as_global_variable_and_write_node()
                .expect("lower: kind mismatch");
            Node::GlobalVariableAndWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
                name_loc: span_of(&typed.name_loc()),
                operator_loc: span_of(&typed.operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
            }
        }
        ruby_prism::Node::GlobalVariableOperatorWriteNode { .. } => {
            let typed = node
                .as_global_variable_operator_write_node()
                .expect("lower: kind mismatch");
            Node::GlobalVariableOperatorWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
                name_loc: span_of(&typed.name_loc()),
                binary_operator_loc: span_of(&typed.binary_operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
                binary_operator: pool.intern(typed.binary_operator().as_slice()),
            }
        }
        ruby_prism::Node::GlobalVariableOrWriteNode { .. } => {
            let typed = node
                .as_global_variable_or_write_node()
                .expect("lower: kind mismatch");
            Node::GlobalVariableOrWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
                name_loc: span_of(&typed.name_loc()),
                operator_loc: span_of(&typed.operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
            }
        }
        ruby_prism::Node::GlobalVariableReadNode { .. } => {
            let typed = node
                .as_global_variable_read_node()
                .expect("lower: kind mismatch");
            Node::GlobalVariableReadNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
            }
        }
        ruby_prism::Node::GlobalVariableTargetNode { .. } => {
            let typed = node
                .as_global_variable_target_node()
                .expect("lower: kind mismatch");
            Node::GlobalVariableTargetNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
            }
        }
        ruby_prism::Node::GlobalVariableWriteNode { .. } => {
            let typed = node
                .as_global_variable_write_node()
                .expect("lower: kind mismatch");
            Node::GlobalVariableWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
                name_loc: span_of(&typed.name_loc()),
                value: Box::new(conv(&typed.value(), pool)),
                operator_loc: span_of(&typed.operator_loc()),
            }
        }
        ruby_prism::Node::HashNode { .. } => {
            let typed = node.as_hash_node().expect("lower: kind mismatch");
            Node::HashNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                opening_loc: span_of(&typed.opening_loc()),
                elements: typed
                    .elements()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                closing_loc: span_of(&typed.closing_loc()),
            }
        }
        ruby_prism::Node::HashPatternNode { .. } => {
            let typed = node.as_hash_pattern_node().expect("lower: kind mismatch");
            Node::HashPatternNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                constant: typed.constant().map(|child| Box::new(conv(&child, pool))),
                elements: typed
                    .elements()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                rest: typed.rest().map(|child| Box::new(conv(&child, pool))),
                opening_loc: typed.opening_loc().map(|loc| span_of(&loc)),
                closing_loc: typed.closing_loc().map(|loc| span_of(&loc)),
            }
        }
        ruby_prism::Node::IfNode { .. } => {
            let typed = node.as_if_node().expect("lower: kind mismatch");
            Node::IfNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                if_keyword_loc: typed.if_keyword_loc().map(|loc| span_of(&loc)),
                predicate: Box::new(conv(&typed.predicate(), pool)),
                then_keyword_loc: typed.then_keyword_loc().map(|loc| span_of(&loc)),
                statements: typed
                    .statements()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                subsequent: typed.subsequent().map(|child| Box::new(conv(&child, pool))),
                end_keyword_loc: typed.end_keyword_loc().map(|loc| span_of(&loc)),
            }
        }
        ruby_prism::Node::ImaginaryNode { .. } => {
            let typed = node.as_imaginary_node().expect("lower: kind mismatch");
            Node::ImaginaryNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                numeric: Box::new(conv(&typed.numeric(), pool)),
            }
        }
        ruby_prism::Node::ImplicitNode { .. } => {
            let typed = node.as_implicit_node().expect("lower: kind mismatch");
            Node::ImplicitNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                value: Box::new(conv(&typed.value(), pool)),
            }
        }
        ruby_prism::Node::ImplicitRestNode { .. } => {
            let typed = node.as_implicit_rest_node().expect("lower: kind mismatch");
            Node::ImplicitRestNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
            }
        }
        ruby_prism::Node::InNode { .. } => {
            let typed = node.as_in_node().expect("lower: kind mismatch");
            Node::InNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                pattern: Box::new(conv(&typed.pattern(), pool)),
                statements: typed
                    .statements()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                in_loc: span_of(&typed.in_loc()),
                then_loc: typed.then_loc().map(|loc| span_of(&loc)),
            }
        }
        ruby_prism::Node::IndexAndWriteNode { .. } => {
            let typed = node
                .as_index_and_write_node()
                .expect("lower: kind mismatch");
            Node::IndexAndWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                receiver: typed.receiver().map(|child| Box::new(conv(&child, pool))),
                call_operator_loc: typed.call_operator_loc().map(|loc| span_of(&loc)),
                opening_loc: span_of(&typed.opening_loc()),
                arguments: typed
                    .arguments()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                closing_loc: span_of(&typed.closing_loc()),
                block: typed
                    .block()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                operator_loc: span_of(&typed.operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
            }
        }
        ruby_prism::Node::IndexOperatorWriteNode { .. } => {
            let typed = node
                .as_index_operator_write_node()
                .expect("lower: kind mismatch");
            Node::IndexOperatorWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                receiver: typed.receiver().map(|child| Box::new(conv(&child, pool))),
                call_operator_loc: typed.call_operator_loc().map(|loc| span_of(&loc)),
                opening_loc: span_of(&typed.opening_loc()),
                arguments: typed
                    .arguments()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                closing_loc: span_of(&typed.closing_loc()),
                block: typed
                    .block()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                binary_operator: pool.intern(typed.binary_operator().as_slice()),
                binary_operator_loc: span_of(&typed.binary_operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
            }
        }
        ruby_prism::Node::IndexOrWriteNode { .. } => {
            let typed = node.as_index_or_write_node().expect("lower: kind mismatch");
            Node::IndexOrWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                receiver: typed.receiver().map(|child| Box::new(conv(&child, pool))),
                call_operator_loc: typed.call_operator_loc().map(|loc| span_of(&loc)),
                opening_loc: span_of(&typed.opening_loc()),
                arguments: typed
                    .arguments()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                closing_loc: span_of(&typed.closing_loc()),
                block: typed
                    .block()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                operator_loc: span_of(&typed.operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
            }
        }
        ruby_prism::Node::IndexTargetNode { .. } => {
            let typed = node.as_index_target_node().expect("lower: kind mismatch");
            Node::IndexTargetNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                receiver: Box::new(conv(&typed.receiver(), pool)),
                opening_loc: span_of(&typed.opening_loc()),
                arguments: typed
                    .arguments()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                closing_loc: span_of(&typed.closing_loc()),
                block: typed
                    .block()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
            }
        }
        ruby_prism::Node::InstanceVariableAndWriteNode { .. } => {
            let typed = node
                .as_instance_variable_and_write_node()
                .expect("lower: kind mismatch");
            Node::InstanceVariableAndWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
                name_loc: span_of(&typed.name_loc()),
                operator_loc: span_of(&typed.operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
            }
        }
        ruby_prism::Node::InstanceVariableOperatorWriteNode { .. } => {
            let typed = node
                .as_instance_variable_operator_write_node()
                .expect("lower: kind mismatch");
            Node::InstanceVariableOperatorWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
                name_loc: span_of(&typed.name_loc()),
                binary_operator_loc: span_of(&typed.binary_operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
                binary_operator: pool.intern(typed.binary_operator().as_slice()),
            }
        }
        ruby_prism::Node::InstanceVariableOrWriteNode { .. } => {
            let typed = node
                .as_instance_variable_or_write_node()
                .expect("lower: kind mismatch");
            Node::InstanceVariableOrWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
                name_loc: span_of(&typed.name_loc()),
                operator_loc: span_of(&typed.operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
            }
        }
        ruby_prism::Node::InstanceVariableReadNode { .. } => {
            let typed = node
                .as_instance_variable_read_node()
                .expect("lower: kind mismatch");
            Node::InstanceVariableReadNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
            }
        }
        ruby_prism::Node::InstanceVariableTargetNode { .. } => {
            let typed = node
                .as_instance_variable_target_node()
                .expect("lower: kind mismatch");
            Node::InstanceVariableTargetNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
            }
        }
        ruby_prism::Node::InstanceVariableWriteNode { .. } => {
            let typed = node
                .as_instance_variable_write_node()
                .expect("lower: kind mismatch");
            Node::InstanceVariableWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
                name_loc: span_of(&typed.name_loc()),
                value: Box::new(conv(&typed.value(), pool)),
                operator_loc: span_of(&typed.operator_loc()),
            }
        }
        ruby_prism::Node::IntegerNode { .. } => {
            let typed = node.as_integer_node().expect("lower: kind mismatch");
            Node::IntegerNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                value: convert_integer(&typed.value()),
            }
        }
        ruby_prism::Node::InterpolatedMatchLastLineNode { .. } => {
            let typed = node
                .as_interpolated_match_last_line_node()
                .expect("lower: kind mismatch");
            Node::InterpolatedMatchLastLineNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                opening_loc: span_of(&typed.opening_loc()),
                parts: typed
                    .parts()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                closing_loc: span_of(&typed.closing_loc()),
            }
        }
        ruby_prism::Node::InterpolatedRegularExpressionNode { .. } => {
            let typed = node
                .as_interpolated_regular_expression_node()
                .expect("lower: kind mismatch");
            Node::InterpolatedRegularExpressionNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                opening_loc: span_of(&typed.opening_loc()),
                parts: typed
                    .parts()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                closing_loc: span_of(&typed.closing_loc()),
            }
        }
        ruby_prism::Node::InterpolatedStringNode { .. } => {
            let typed = node
                .as_interpolated_string_node()
                .expect("lower: kind mismatch");
            Node::InterpolatedStringNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                opening_loc: typed.opening_loc().map(|loc| span_of(&loc)),
                parts: typed
                    .parts()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                closing_loc: typed.closing_loc().map(|loc| span_of(&loc)),
            }
        }
        ruby_prism::Node::InterpolatedSymbolNode { .. } => {
            let typed = node
                .as_interpolated_symbol_node()
                .expect("lower: kind mismatch");
            Node::InterpolatedSymbolNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                opening_loc: typed.opening_loc().map(|loc| span_of(&loc)),
                parts: typed
                    .parts()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                closing_loc: typed.closing_loc().map(|loc| span_of(&loc)),
            }
        }
        ruby_prism::Node::InterpolatedXStringNode { .. } => {
            let typed = node
                .as_interpolated_x_string_node()
                .expect("lower: kind mismatch");
            Node::InterpolatedXStringNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                opening_loc: span_of(&typed.opening_loc()),
                parts: typed
                    .parts()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                closing_loc: span_of(&typed.closing_loc()),
            }
        }
        ruby_prism::Node::ItLocalVariableReadNode { .. } => {
            let typed = node
                .as_it_local_variable_read_node()
                .expect("lower: kind mismatch");
            Node::ItLocalVariableReadNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
            }
        }
        ruby_prism::Node::ItParametersNode { .. } => {
            let typed = node.as_it_parameters_node().expect("lower: kind mismatch");
            Node::ItParametersNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
            }
        }
        ruby_prism::Node::KeywordHashNode { .. } => {
            let typed = node.as_keyword_hash_node().expect("lower: kind mismatch");
            Node::KeywordHashNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                elements: typed
                    .elements()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
            }
        }
        ruby_prism::Node::KeywordRestParameterNode { .. } => {
            let typed = node
                .as_keyword_rest_parameter_node()
                .expect("lower: kind mismatch");
            Node::KeywordRestParameterNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: typed.name().map(|id| pool.intern(id.as_slice())),
                name_loc: typed.name_loc().map(|loc| span_of(&loc)),
                operator_loc: span_of(&typed.operator_loc()),
            }
        }
        ruby_prism::Node::LambdaNode { .. } => {
            let typed = node.as_lambda_node().expect("lower: kind mismatch");
            Node::LambdaNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                locals: typed
                    .locals()
                    .iter()
                    .map(|id| pool.intern(id.as_slice()))
                    .collect(),
                operator_loc: span_of(&typed.operator_loc()),
                opening_loc: span_of(&typed.opening_loc()),
                closing_loc: span_of(&typed.closing_loc()),
                parameters: typed.parameters().map(|child| Box::new(conv(&child, pool))),
                body: typed.body().map(|child| Box::new(conv(&child, pool))),
            }
        }
        ruby_prism::Node::LocalVariableAndWriteNode { .. } => {
            let typed = node
                .as_local_variable_and_write_node()
                .expect("lower: kind mismatch");
            Node::LocalVariableAndWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name_loc: span_of(&typed.name_loc()),
                operator_loc: span_of(&typed.operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
                name: pool.intern(typed.name().as_slice()),
                depth: typed.depth(),
            }
        }
        ruby_prism::Node::LocalVariableOperatorWriteNode { .. } => {
            let typed = node
                .as_local_variable_operator_write_node()
                .expect("lower: kind mismatch");
            Node::LocalVariableOperatorWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name_loc: span_of(&typed.name_loc()),
                binary_operator_loc: span_of(&typed.binary_operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
                name: pool.intern(typed.name().as_slice()),
                binary_operator: pool.intern(typed.binary_operator().as_slice()),
                depth: typed.depth(),
            }
        }
        ruby_prism::Node::LocalVariableOrWriteNode { .. } => {
            let typed = node
                .as_local_variable_or_write_node()
                .expect("lower: kind mismatch");
            Node::LocalVariableOrWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name_loc: span_of(&typed.name_loc()),
                operator_loc: span_of(&typed.operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
                name: pool.intern(typed.name().as_slice()),
                depth: typed.depth(),
            }
        }
        ruby_prism::Node::LocalVariableReadNode { .. } => {
            let typed = node
                .as_local_variable_read_node()
                .expect("lower: kind mismatch");
            Node::LocalVariableReadNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
                depth: typed.depth(),
            }
        }
        ruby_prism::Node::LocalVariableTargetNode { .. } => {
            let typed = node
                .as_local_variable_target_node()
                .expect("lower: kind mismatch");
            Node::LocalVariableTargetNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
                depth: typed.depth(),
            }
        }
        ruby_prism::Node::LocalVariableWriteNode { .. } => {
            let typed = node
                .as_local_variable_write_node()
                .expect("lower: kind mismatch");
            Node::LocalVariableWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
                depth: typed.depth(),
                name_loc: span_of(&typed.name_loc()),
                value: Box::new(conv(&typed.value(), pool)),
                operator_loc: span_of(&typed.operator_loc()),
            }
        }
        ruby_prism::Node::MatchLastLineNode { .. } => {
            let typed = node
                .as_match_last_line_node()
                .expect("lower: kind mismatch");
            Node::MatchLastLineNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                opening_loc: span_of(&typed.opening_loc()),
                content_loc: span_of(&typed.content_loc()),
                closing_loc: span_of(&typed.closing_loc()),
                unescaped: typed.unescaped().to_vec(),
            }
        }
        ruby_prism::Node::MatchPredicateNode { .. } => {
            let typed = node
                .as_match_predicate_node()
                .expect("lower: kind mismatch");
            Node::MatchPredicateNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                value: Box::new(conv(&typed.value(), pool)),
                pattern: Box::new(conv(&typed.pattern(), pool)),
                operator_loc: span_of(&typed.operator_loc()),
            }
        }
        ruby_prism::Node::MatchRequiredNode { .. } => {
            let typed = node.as_match_required_node().expect("lower: kind mismatch");
            Node::MatchRequiredNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                value: Box::new(conv(&typed.value(), pool)),
                pattern: Box::new(conv(&typed.pattern(), pool)),
                operator_loc: span_of(&typed.operator_loc()),
            }
        }
        ruby_prism::Node::MatchWriteNode { .. } => {
            let typed = node.as_match_write_node().expect("lower: kind mismatch");
            Node::MatchWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                call: Box::new(conv(&typed.call().as_node(), pool)),
                targets: typed
                    .targets()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
            }
        }
        ruby_prism::Node::MissingNode { .. } => {
            let typed = node.as_missing_node().expect("lower: kind mismatch");
            Node::MissingNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
            }
        }
        ruby_prism::Node::ModuleNode { .. } => {
            let typed = node.as_module_node().expect("lower: kind mismatch");
            Node::ModuleNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                locals: typed
                    .locals()
                    .iter()
                    .map(|id| pool.intern(id.as_slice()))
                    .collect(),
                module_keyword_loc: span_of(&typed.module_keyword_loc()),
                constant_path: Box::new(conv(&typed.constant_path(), pool)),
                body: typed.body().map(|child| Box::new(conv(&child, pool))),
                end_keyword_loc: span_of(&typed.end_keyword_loc()),
                name: pool.intern(typed.name().as_slice()),
            }
        }
        ruby_prism::Node::MultiTargetNode { .. } => {
            let typed = node.as_multi_target_node().expect("lower: kind mismatch");
            Node::MultiTargetNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                lefts: typed
                    .lefts()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                rest: typed.rest().map(|child| Box::new(conv(&child, pool))),
                rights: typed
                    .rights()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                lparen_loc: typed.lparen_loc().map(|loc| span_of(&loc)),
                rparen_loc: typed.rparen_loc().map(|loc| span_of(&loc)),
            }
        }
        ruby_prism::Node::MultiWriteNode { .. } => {
            let typed = node.as_multi_write_node().expect("lower: kind mismatch");
            Node::MultiWriteNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                lefts: typed
                    .lefts()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                rest: typed.rest().map(|child| Box::new(conv(&child, pool))),
                rights: typed
                    .rights()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                lparen_loc: typed.lparen_loc().map(|loc| span_of(&loc)),
                rparen_loc: typed.rparen_loc().map(|loc| span_of(&loc)),
                operator_loc: span_of(&typed.operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
            }
        }
        ruby_prism::Node::NextNode { .. } => {
            let typed = node.as_next_node().expect("lower: kind mismatch");
            Node::NextNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                arguments: typed
                    .arguments()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                keyword_loc: span_of(&typed.keyword_loc()),
            }
        }
        ruby_prism::Node::NilNode { .. } => {
            let typed = node.as_nil_node().expect("lower: kind mismatch");
            Node::NilNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
            }
        }
        ruby_prism::Node::NoKeywordsParameterNode { .. } => {
            let typed = node
                .as_no_keywords_parameter_node()
                .expect("lower: kind mismatch");
            Node::NoKeywordsParameterNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                operator_loc: span_of(&typed.operator_loc()),
                keyword_loc: span_of(&typed.keyword_loc()),
            }
        }
        ruby_prism::Node::NumberedParametersNode { .. } => {
            let typed = node
                .as_numbered_parameters_node()
                .expect("lower: kind mismatch");
            Node::NumberedParametersNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                maximum: typed.maximum(),
            }
        }
        ruby_prism::Node::NumberedReferenceReadNode { .. } => {
            let typed = node
                .as_numbered_reference_read_node()
                .expect("lower: kind mismatch");
            Node::NumberedReferenceReadNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                number: typed.number(),
            }
        }
        ruby_prism::Node::OptionalKeywordParameterNode { .. } => {
            let typed = node
                .as_optional_keyword_parameter_node()
                .expect("lower: kind mismatch");
            Node::OptionalKeywordParameterNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
                name_loc: span_of(&typed.name_loc()),
                value: Box::new(conv(&typed.value(), pool)),
            }
        }
        ruby_prism::Node::OptionalParameterNode { .. } => {
            let typed = node
                .as_optional_parameter_node()
                .expect("lower: kind mismatch");
            Node::OptionalParameterNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
                name_loc: span_of(&typed.name_loc()),
                operator_loc: span_of(&typed.operator_loc()),
                value: Box::new(conv(&typed.value(), pool)),
            }
        }
        ruby_prism::Node::OrNode { .. } => {
            let typed = node.as_or_node().expect("lower: kind mismatch");
            Node::OrNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                left: Box::new(conv(&typed.left(), pool)),
                right: Box::new(conv(&typed.right(), pool)),
                operator_loc: span_of(&typed.operator_loc()),
            }
        }
        ruby_prism::Node::ParametersNode { .. } => {
            let typed = node.as_parameters_node().expect("lower: kind mismatch");
            Node::ParametersNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                requireds: typed
                    .requireds()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                optionals: typed
                    .optionals()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                rest: typed.rest().map(|child| Box::new(conv(&child, pool))),
                posts: typed
                    .posts()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                keywords: typed
                    .keywords()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                keyword_rest: typed
                    .keyword_rest()
                    .map(|child| Box::new(conv(&child, pool))),
                block: typed
                    .block()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
            }
        }
        ruby_prism::Node::ParenthesesNode { .. } => {
            let typed = node.as_parentheses_node().expect("lower: kind mismatch");
            Node::ParenthesesNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                body: typed.body().map(|child| Box::new(conv(&child, pool))),
                opening_loc: span_of(&typed.opening_loc()),
                closing_loc: span_of(&typed.closing_loc()),
            }
        }
        ruby_prism::Node::PinnedExpressionNode { .. } => {
            let typed = node
                .as_pinned_expression_node()
                .expect("lower: kind mismatch");
            Node::PinnedExpressionNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                expression: Box::new(conv(&typed.expression(), pool)),
                operator_loc: span_of(&typed.operator_loc()),
                lparen_loc: span_of(&typed.lparen_loc()),
                rparen_loc: span_of(&typed.rparen_loc()),
            }
        }
        ruby_prism::Node::PinnedVariableNode { .. } => {
            let typed = node
                .as_pinned_variable_node()
                .expect("lower: kind mismatch");
            Node::PinnedVariableNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                variable: Box::new(conv(&typed.variable(), pool)),
                operator_loc: span_of(&typed.operator_loc()),
            }
        }
        ruby_prism::Node::PostExecutionNode { .. } => {
            let typed = node.as_post_execution_node().expect("lower: kind mismatch");
            Node::PostExecutionNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                statements: typed
                    .statements()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                keyword_loc: span_of(&typed.keyword_loc()),
                opening_loc: span_of(&typed.opening_loc()),
                closing_loc: span_of(&typed.closing_loc()),
            }
        }
        ruby_prism::Node::PreExecutionNode { .. } => {
            let typed = node.as_pre_execution_node().expect("lower: kind mismatch");
            Node::PreExecutionNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                statements: typed
                    .statements()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                keyword_loc: span_of(&typed.keyword_loc()),
                opening_loc: span_of(&typed.opening_loc()),
                closing_loc: span_of(&typed.closing_loc()),
            }
        }
        ruby_prism::Node::ProgramNode { .. } => {
            let typed = node.as_program_node().expect("lower: kind mismatch");
            Node::ProgramNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                locals: typed
                    .locals()
                    .iter()
                    .map(|id| pool.intern(id.as_slice()))
                    .collect(),
                statements: Box::new(conv(&typed.statements().as_node(), pool)),
            }
        }
        ruby_prism::Node::RangeNode { .. } => {
            let typed = node.as_range_node().expect("lower: kind mismatch");
            Node::RangeNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                left: typed.left().map(|child| Box::new(conv(&child, pool))),
                right: typed.right().map(|child| Box::new(conv(&child, pool))),
                operator_loc: span_of(&typed.operator_loc()),
            }
        }
        ruby_prism::Node::RationalNode { .. } => {
            let typed = node.as_rational_node().expect("lower: kind mismatch");
            Node::RationalNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                numerator: convert_integer(&typed.numerator()),
                denominator: convert_integer(&typed.denominator()),
            }
        }
        ruby_prism::Node::RedoNode { .. } => {
            let typed = node.as_redo_node().expect("lower: kind mismatch");
            Node::RedoNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
            }
        }
        ruby_prism::Node::RegularExpressionNode { .. } => {
            let typed = node
                .as_regular_expression_node()
                .expect("lower: kind mismatch");
            Node::RegularExpressionNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                opening_loc: span_of(&typed.opening_loc()),
                content_loc: span_of(&typed.content_loc()),
                closing_loc: span_of(&typed.closing_loc()),
                unescaped: typed.unescaped().to_vec(),
            }
        }
        ruby_prism::Node::RequiredKeywordParameterNode { .. } => {
            let typed = node
                .as_required_keyword_parameter_node()
                .expect("lower: kind mismatch");
            Node::RequiredKeywordParameterNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
                name_loc: span_of(&typed.name_loc()),
            }
        }
        ruby_prism::Node::RequiredParameterNode { .. } => {
            let typed = node
                .as_required_parameter_node()
                .expect("lower: kind mismatch");
            Node::RequiredParameterNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: pool.intern(typed.name().as_slice()),
            }
        }
        ruby_prism::Node::RescueModifierNode { .. } => {
            let typed = node
                .as_rescue_modifier_node()
                .expect("lower: kind mismatch");
            Node::RescueModifierNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                expression: Box::new(conv(&typed.expression(), pool)),
                keyword_loc: span_of(&typed.keyword_loc()),
                rescue_expression: Box::new(conv(&typed.rescue_expression(), pool)),
            }
        }
        ruby_prism::Node::RescueNode { .. } => {
            let typed = node.as_rescue_node().expect("lower: kind mismatch");
            Node::RescueNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                keyword_loc: span_of(&typed.keyword_loc()),
                exceptions: typed
                    .exceptions()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                operator_loc: typed.operator_loc().map(|loc| span_of(&loc)),
                reference: typed.reference().map(|child| Box::new(conv(&child, pool))),
                then_keyword_loc: typed.then_keyword_loc().map(|loc| span_of(&loc)),
                statements: typed
                    .statements()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                subsequent: typed
                    .subsequent()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
            }
        }
        ruby_prism::Node::RestParameterNode { .. } => {
            let typed = node.as_rest_parameter_node().expect("lower: kind mismatch");
            Node::RestParameterNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                name: typed.name().map(|id| pool.intern(id.as_slice())),
                name_loc: typed.name_loc().map(|loc| span_of(&loc)),
                operator_loc: span_of(&typed.operator_loc()),
            }
        }
        ruby_prism::Node::RetryNode { .. } => {
            let typed = node.as_retry_node().expect("lower: kind mismatch");
            Node::RetryNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
            }
        }
        ruby_prism::Node::ReturnNode { .. } => {
            let typed = node.as_return_node().expect("lower: kind mismatch");
            Node::ReturnNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                keyword_loc: span_of(&typed.keyword_loc()),
                arguments: typed
                    .arguments()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
            }
        }
        ruby_prism::Node::SelfNode { .. } => {
            let typed = node.as_self_node().expect("lower: kind mismatch");
            Node::SelfNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
            }
        }
        ruby_prism::Node::ShareableConstantNode { .. } => {
            let typed = node
                .as_shareable_constant_node()
                .expect("lower: kind mismatch");
            Node::ShareableConstantNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                write: Box::new(conv(&typed.write(), pool)),
            }
        }
        ruby_prism::Node::SingletonClassNode { .. } => {
            let typed = node
                .as_singleton_class_node()
                .expect("lower: kind mismatch");
            Node::SingletonClassNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                locals: typed
                    .locals()
                    .iter()
                    .map(|id| pool.intern(id.as_slice()))
                    .collect(),
                class_keyword_loc: span_of(&typed.class_keyword_loc()),
                operator_loc: span_of(&typed.operator_loc()),
                expression: Box::new(conv(&typed.expression(), pool)),
                body: typed.body().map(|child| Box::new(conv(&child, pool))),
                end_keyword_loc: span_of(&typed.end_keyword_loc()),
            }
        }
        ruby_prism::Node::SourceEncodingNode { .. } => {
            let typed = node
                .as_source_encoding_node()
                .expect("lower: kind mismatch");
            Node::SourceEncodingNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
            }
        }
        ruby_prism::Node::SourceFileNode { .. } => {
            let typed = node.as_source_file_node().expect("lower: kind mismatch");
            Node::SourceFileNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                filepath: typed.filepath().to_vec(),
            }
        }
        ruby_prism::Node::SourceLineNode { .. } => {
            let typed = node.as_source_line_node().expect("lower: kind mismatch");
            Node::SourceLineNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
            }
        }
        ruby_prism::Node::SplatNode { .. } => {
            let typed = node.as_splat_node().expect("lower: kind mismatch");
            Node::SplatNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                operator_loc: span_of(&typed.operator_loc()),
                expression: typed.expression().map(|child| Box::new(conv(&child, pool))),
            }
        }
        ruby_prism::Node::StatementsNode { .. } => {
            let typed = node.as_statements_node().expect("lower: kind mismatch");
            Node::StatementsNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                body: typed
                    .body()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
            }
        }
        ruby_prism::Node::StringNode { .. } => {
            let typed = node.as_string_node().expect("lower: kind mismatch");
            Node::StringNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                opening_loc: typed.opening_loc().map(|loc| span_of(&loc)),
                content_loc: span_of(&typed.content_loc()),
                closing_loc: typed.closing_loc().map(|loc| span_of(&loc)),
                unescaped: typed.unescaped().to_vec(),
            }
        }
        ruby_prism::Node::SuperNode { .. } => {
            let typed = node.as_super_node().expect("lower: kind mismatch");
            Node::SuperNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                keyword_loc: span_of(&typed.keyword_loc()),
                lparen_loc: typed.lparen_loc().map(|loc| span_of(&loc)),
                arguments: typed
                    .arguments()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                rparen_loc: typed.rparen_loc().map(|loc| span_of(&loc)),
                block: typed.block().map(|child| Box::new(conv(&child, pool))),
            }
        }
        ruby_prism::Node::SymbolNode { .. } => {
            let typed = node.as_symbol_node().expect("lower: kind mismatch");
            Node::SymbolNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                opening_loc: typed.opening_loc().map(|loc| span_of(&loc)),
                value_loc: typed.value_loc().map(|loc| span_of(&loc)),
                closing_loc: typed.closing_loc().map(|loc| span_of(&loc)),
                unescaped: typed.unescaped().to_vec(),
            }
        }
        ruby_prism::Node::TrueNode { .. } => {
            let typed = node.as_true_node().expect("lower: kind mismatch");
            Node::TrueNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
            }
        }
        ruby_prism::Node::UndefNode { .. } => {
            let typed = node.as_undef_node().expect("lower: kind mismatch");
            Node::UndefNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                names: typed
                    .names()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                keyword_loc: span_of(&typed.keyword_loc()),
            }
        }
        ruby_prism::Node::UnlessNode { .. } => {
            let typed = node.as_unless_node().expect("lower: kind mismatch");
            Node::UnlessNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                keyword_loc: span_of(&typed.keyword_loc()),
                predicate: Box::new(conv(&typed.predicate(), pool)),
                then_keyword_loc: typed.then_keyword_loc().map(|loc| span_of(&loc)),
                statements: typed
                    .statements()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                else_clause: typed
                    .else_clause()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                end_keyword_loc: typed.end_keyword_loc().map(|loc| span_of(&loc)),
            }
        }
        ruby_prism::Node::UntilNode { .. } => {
            let typed = node.as_until_node().expect("lower: kind mismatch");
            Node::UntilNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                keyword_loc: span_of(&typed.keyword_loc()),
                do_keyword_loc: typed.do_keyword_loc().map(|loc| span_of(&loc)),
                closing_loc: typed.closing_loc().map(|loc| span_of(&loc)),
                predicate: Box::new(conv(&typed.predicate(), pool)),
                statements: typed
                    .statements()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
            }
        }
        ruby_prism::Node::WhenNode { .. } => {
            let typed = node.as_when_node().expect("lower: kind mismatch");
            Node::WhenNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                keyword_loc: span_of(&typed.keyword_loc()),
                conditions: typed
                    .conditions()
                    .iter()
                    .map(|child| conv(&child, pool))
                    .collect(),
                then_keyword_loc: typed.then_keyword_loc().map(|loc| span_of(&loc)),
                statements: typed
                    .statements()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
            }
        }
        ruby_prism::Node::WhileNode { .. } => {
            let typed = node.as_while_node().expect("lower: kind mismatch");
            Node::WhileNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                keyword_loc: span_of(&typed.keyword_loc()),
                do_keyword_loc: typed.do_keyword_loc().map(|loc| span_of(&loc)),
                closing_loc: typed.closing_loc().map(|loc| span_of(&loc)),
                predicate: Box::new(conv(&typed.predicate(), pool)),
                statements: typed
                    .statements()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
            }
        }
        ruby_prism::Node::XStringNode { .. } => {
            let typed = node.as_x_string_node().expect("lower: kind mismatch");
            Node::XStringNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                opening_loc: span_of(&typed.opening_loc()),
                content_loc: span_of(&typed.content_loc()),
                closing_loc: span_of(&typed.closing_loc()),
                unescaped: typed.unescaped().to_vec(),
            }
        }
        ruby_prism::Node::YieldNode { .. } => {
            let typed = node.as_yield_node().expect("lower: kind mismatch");
            Node::YieldNode {
                flags: typed.flags(),
                span: span_of(&typed.location()),
                keyword_loc: span_of(&typed.keyword_loc()),
                lparen_loc: typed.lparen_loc().map(|loc| span_of(&loc)),
                arguments: typed
                    .arguments()
                    .map(|child| Box::new(conv(&child.as_node(), pool))),
                rparen_loc: typed.rparen_loc().map(|loc| span_of(&loc)),
            }
        }
    }
}

/// Byte offsets of a Prism location.
fn span_of(location: &ruby_prism::Location<'_>) -> Span {
    Span {
        start: u32::try_from(location.start_offset()).unwrap_or(u32::MAX),
        end: u32::try_from(location.end_offset()).unwrap_or(u32::MAX),
    }
}

/// Owned integer matching the backend literal: `I64` when the value fits,
/// otherwise normalized decimal digits (no leading zeros) with a `-`
/// prefix for negatives (the owned accessor splits the sign back off).
fn convert_integer(value: &ruby_prism::Integer<'_>) -> Integer {
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
            return Integer::I64(scalar);
        }
    }
    let mut raw = limbs_to_decimal(limbs);
    if negative {
        raw.insert(0, b'-');
    }
    Integer::Fallback { raw }
}

/// Decimal digits of little-endian base-2^32 limbs, without leading zeros.
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
