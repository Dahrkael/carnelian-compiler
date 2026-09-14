//! Frontend adapter smoke tests (borrowed nodes, no codegen yet).

use carnelian_ast::view::BackendNode;
use carnelian_ast::AstNode;
use carnelian_front_prism::{parse, PrismNode};

#[test]
fn parse_exposes_root_and_locals() {
    let source = b"x = 1\nputs x\n";
    let parsed = parse(source);
    assert!(parsed.errors().is_empty());
    let root = parsed.root();
    assert_eq!(root.kind_name(), "ProgramNode");
    assert_eq!(root.span(), carnelian_ast::Span { start: 0, end: 12 });
    assert_eq!(parsed.program_locals(), vec![b"x".to_vec()]);
}

#[test]
fn node_kinds_and_flags() {
    let parsed = parse(b"puts 1\n");
    let root = parsed.root();
    // Flags accessor works on any node (top-level has none set).
    assert_eq!(root.flags(), 0);
    let inner = root.inner();
    let program = inner.as_program_node().expect("program");
    let statements = program.statements();
    let mut count = 0;
    for statement in statements.body().iter() {
        let node = PrismNode::new(statement);
        assert_eq!(node.kind_name(), "CallNode");
        count += 1;
    }
    assert_eq!(count, 1);
}

#[test]
fn syntax_errors_surface_as_diagnostics() {
    let parsed = parse(b"def (\n");
    assert!(!parsed.errors().is_empty());
}

fn first_statement<'pr>(parsed: &'pr carnelian_front_prism::Parsed<'pr>) -> PrismNode<'pr> {
    let root = parsed.root();
    let program = root.program().expect("program");
    let statements = program.body.statements().expect("statements");
    statements.into_iter().next().expect("statement")
}

#[test]
fn hash_accessors_cover_pairs_and_splats() {
    let parsed = parse(b"y = nil\n{a: 1, **y}\n");
    let root = parsed.root();
    let program = root.program().expect("program");
    let statements = program.body.statements().expect("statements");
    let node = &statements[1];
    assert_eq!(node.kind_name(), "HashNode");
    let elements = node.hash_elements().expect("elements");
    assert_eq!(elements.len(), 2);
    let (key, value) = elements[0].assoc_pair().expect("pair");
    assert_eq!(key.kind_name(), "SymbolNode");
    assert_eq!(value.kind_name(), "IntegerNode");
    assert!(elements[0].assoc_splat_value().is_none());
    let inner = elements[1].assoc_splat_value().expect("splat");
    let value = inner.expect("splat value");
    assert_eq!(value.kind_name(), "LocalVariableReadNode");
    assert!(elements[1].assoc_pair().is_none());
}

#[test]
fn empty_hash_has_no_elements() {
    let parsed = parse(b"x = {}\n");
    let root = parsed.root();
    let program = root.program().expect("program");
    let statements = program.body.statements().expect("statements");
    let write = &statements[0];
    assert_eq!(write.kind_name(), "LocalVariableWriteNode");
    let target = write.lvar_write().expect("write");
    let value = target.value.expect("value");
    assert_eq!(value.kind_name(), "HashNode");
    assert_eq!(value.hash_elements().expect("elements").len(), 0);
}

#[test]
fn case_accessors_distinguish_null_and_empty_bodies() {
    let parsed = parse(b"case x\nwhen 1 then\nelse puts 2\nend\n");
    let node = first_statement(&parsed);
    let view = node.case_view().expect("case");
    assert!(view.predicate.is_some());
    assert!(view.else_body.is_some());
    assert_eq!(view.whens.len(), 1);
    let when = view.whens[0].when_view().expect("when");
    assert_eq!(when.conditions.len(), 1);
    // `then` with no statements is a null subtree, not an empty node.
    assert!(when.body.is_none());

    let parsed = parse(b"case\nwhen x then puts 1\nend\n");
    let node = first_statement(&parsed);
    let view = node.case_view().expect("bare case");
    assert!(view.predicate.is_none());
    assert!(view.else_body.is_none());
}

#[test]
fn splat_condition_exposes_inner_expression() {
    let parsed = parse(b"case x\nwhen *a then puts 1\nend\n");
    let node = first_statement(&parsed);
    let view = node.case_view().expect("case");
    let when = view.whens[0].when_view().expect("when");
    assert_eq!(when.conditions.len(), 1);
    let condition = &when.conditions[0];
    assert_eq!(condition.kind_name(), "SplatNode");
    let inner = condition.splat_value().expect("splat");
    assert!(inner.is_some());
}

#[test]
fn interpolated_string_accessors_cover_parts() {
    let parsed = parse(b"puts \"hi #{name}\"\n");
    let root = parsed.root();
    assert!(root.program().is_some());
    let parsed = parse(b"\"hi #{name}\"\n");
    let node = first_statement(&parsed);
    let parts = node.string_parts().expect("parts");
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].kind_name(), "StringNode");
    assert_eq!(parts[1].kind_name(), "EmbeddedStatementsNode");
    let body = parts[1].embedded_body().expect("embexpr body");
    assert_eq!(body.len(), 1);
    assert!(parts[1].embedded_var().is_none());

    // An empty `"#{}"` is a null subtree, distinct from an empty node.
    let parsed = parse(b"\"#{}\"\n");
    let node = first_statement(&parsed);
    let parts = node.string_parts().expect("parts");
    assert_eq!(parts.len(), 1);
    assert!(parts[0].embedded_body().is_none());
}

#[test]
fn alias_and_undef_accessors_cover_names() {
    let parsed = parse(b"alias bar foo\n");
    let node = first_statement(&parsed);
    assert_eq!(node.kind_name(), "AliasMethodNode");
    let (new_name, old_name) = node.alias_pair().expect("alias pair");
    assert_eq!(new_name.kind_name(), "SymbolNode");
    assert_eq!(old_name.kind_name(), "SymbolNode");
    assert_eq!(new_name.symbol_lit().expect("new"), b"bar");
    assert_eq!(old_name.symbol_lit().expect("old"), b"foo");
    assert!(node.undef_list().is_none());

    let parsed = parse(b"undef foo, :bar\n");
    let node = first_statement(&parsed);
    assert_eq!(node.kind_name(), "UndefNode");
    let names = node.undef_list().expect("undef names");
    assert_eq!(names.len(), 2);
    assert_eq!(names[0].symbol_lit().expect("first"), b"foo");
    assert_eq!(names[1].symbol_lit().expect("second"), b"bar");
    assert!(node.alias_pair().is_none());
}

#[test]
fn defined_operand_and_variable_accessors() {
    let parsed = parse(b"defined?(x)\n");
    let node = first_statement(&parsed);
    assert_eq!(node.kind_name(), "DefinedNode");
    let value = node.defined_value().expect("defined value");
    assert_eq!(value.kind_name(), "CallNode");
    assert!(value.implicit_value().is_none());

    let parsed = parse(b"x = 1\ndefined?(x)\n");
    let root = parsed.root();
    let statements = root.program().expect("program").body.statements().unwrap();
    let node = &statements[1];
    assert_eq!(
        node.defined_value().expect("defined value").kind_name(),
        "LocalVariableReadNode"
    );

    let parsed = parse(b"((x))\n");
    let node = first_statement(&parsed);
    assert_eq!(node.kind_name(), "ParenthesesNode");
    let body = node.parentheses_body().expect("parens").expect("body");
    assert_eq!(body.kind_name(), "StatementsNode");

    let parsed = parse(b"begin\nx\nend\n");
    let node = first_statement(&parsed);
    assert_eq!(node.kind_name(), "BeginNode");
    let view = node.begin_view().expect("begin");
    assert!(view.bare);
    assert_eq!(
        view.statements.expect("statements").kind_name(),
        "StatementsNode"
    );
}

#[test]
fn defined_name_and_raw_list_accessors() {
    let parsed = parse(b"@x\n");
    assert_eq!(
        first_statement(&parsed)
            .instance_var_read_name()
            .expect("ivar"),
        b"@x"
    );
    let parsed = parse(b"$x\n");
    assert_eq!(
        first_statement(&parsed)
            .global_var_read_name()
            .expect("gvar"),
        b"$x"
    );
    let parsed = parse(b"@@x\n");
    assert_eq!(
        first_statement(&parsed)
            .class_var_read_name()
            .expect("cvar"),
        b"@@x"
    );
    let parsed = parse(b"A\n");
    assert_eq!(
        first_statement(&parsed)
            .constant_read_name()
            .expect("const"),
        b"A"
    );

    let parsed = parse(b"A::B\n");
    let node = first_statement(&parsed);
    assert_eq!(node.kind_name(), "ConstantPathNode");
    let (parent, name) = node.constant_path_parts().expect("path");
    assert_eq!(parent.expect("parent").kind_name(), "ConstantReadNode");
    assert_eq!(name, b"B");

    let parsed = parse(b"foo(1, *a)\n");
    let node = first_statement(&parsed);
    let args = node.call().expect("call").args.expect("args");
    let raw = args.raw_call_args().expect("raw args");
    assert_eq!(raw.len(), 2);
    assert_eq!(raw[0].kind_name(), "IntegerNode");
    assert_eq!(raw[1].kind_name(), "SplatNode");

    let parsed = parse(b"[1, *a]\n");
    let node = first_statement(&parsed);
    let raw = node.raw_array_elements().expect("raw elements");
    assert_eq!(raw.len(), 2);
    assert_eq!(raw[1].kind_name(), "SplatNode");
    // The strict accessor still gates splats.
    assert!(node.array_elements().is_none());
}
