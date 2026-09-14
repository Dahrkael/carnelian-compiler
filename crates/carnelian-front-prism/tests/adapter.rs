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
fn def_accessor_covers_plain_and_gated_params() {
    let parsed = parse(b"def foo(a, b)\n  a\nend\n");
    let node = first_statement(&parsed);
    assert_eq!(node.kind_name(), "DefNode");
    let view = node.def_view().expect("def");
    assert_eq!(view.name, b"foo");
    assert!(view.receiver.is_none());
    assert_eq!(view.required_params, vec![b"a".to_vec(), b"b".to_vec()]);
    assert!(view.body.is_some());

    // Optional parameters stay gated.
    let parsed = parse(b"def foo(a = 1)\n  a\nend\n");
    let node = first_statement(&parsed);
    assert!(node.def_view().is_none());
}

#[test]
fn class_module_sclass_accessors_cover_paths() {
    let parsed = parse(b"class Foo\nend\n");
    let node = first_statement(&parsed);
    let view = node.class_view().expect("class");
    assert_eq!(view.name, b"Foo");
    assert!(view.cpath_is_read);
    assert!(view.cpath_parent.is_none());
    assert!(view.superclass.is_none());
    assert!(view.body.is_none());

    let parsed = parse(b"class Foo::Bar < Baz\nend\n");
    let node = first_statement(&parsed);
    let view = node.class_view().expect("scoped class");
    assert!(!view.cpath_is_read);
    assert!(view.cpath_parent.is_some());
    assert!(view.superclass.is_some());

    let parsed = parse(b"module Foo::Bar\nend\n");
    let node = first_statement(&parsed);
    let view = node.module_view().expect("scoped module");
    assert!(!view.cpath_is_read);
    assert!(view.cpath_parent.is_some());

    let parsed = parse(b"class << self\nend\n");
    let node = first_statement(&parsed);
    let view = node.sclass_view().expect("sclass");
    assert_eq!(view.expression.kind_name(), "SelfNode");
    assert!(view.body.is_none());
}

#[test]
fn variable_and_super_accessors_cover_plain_forms() {
    let parsed = parse(b"@x = 1\n");
    let node = first_statement(&parsed);
    let write = node.ivar_write().expect("ivar write");
    assert_eq!(write.name, b"@x");

    let parsed = parse(b"@@x\n");
    let node = first_statement(&parsed);
    assert_eq!(node.cvar_read().expect("cvar"), b"@@x");

    let parsed = parse(b"$x = 1\n");
    let node = first_statement(&parsed);
    assert!(node.gvar_write().is_some());

    let parsed = parse(b"Foo::Bar\n");
    let node = first_statement(&parsed);
    let path = node.const_path().expect("path");
    assert_eq!(path.name, b"Bar");
    assert!(path.parent.is_some());
}

#[test]
fn begin_accessors_cover_rescue_else_ensure() {
    let parsed = parse(b"begin\n  1\nrescue TypeError => e\n  2\nelse\n  3\nensure\n  4\nend\n");
    let node = first_statement(&parsed);
    assert_eq!(node.kind_name(), "BeginNode");
    let view = node.begin_view().expect("begin");
    assert_eq!(view.statements.as_ref().map(Vec::len), Some(1));
    assert!(view.else_clause.is_some());
    assert!(view.ensure_clause.is_some());

    let rescue = view.rescue_clause.expect("rescue clause");
    let clause = rescue.rescue_view().expect("rescue view");
    assert_eq!(clause.exceptions.len(), 1);
    assert_eq!(clause.exceptions[0].kind_name(), "ConstantReadNode");
    assert!(clause.reference.is_some());
    assert_eq!(clause.statements.as_ref().map(Vec::len), Some(1));
    assert!(clause.subsequent.is_none());

    let ensure = view.ensure_clause.expect("ensure clause");
    assert_eq!(
        ensure
            .ensure_view()
            .expect("ensure view")
            .statements
            .map(|s| s.len()),
        Some(1)
    );
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
    assert_eq!(view.statements.as_ref().map(Vec::len), Some(1));
}

#[test]
fn begin_accessors_distinguish_bare_and_typed() {
    let parsed = parse(b"begin\n  1\nend\n");
    let node = first_statement(&parsed);
    let view = node.begin_view().expect("begin");
    assert!(view.rescue_clause.is_none());
    assert!(view.else_clause.is_none());
    assert!(view.ensure_clause.is_none());
    assert_eq!(view.statements.map(|s| s.len()), Some(1));
}

#[test]
fn rescue_modifier_exposes_both_expressions() {
    let parsed = parse(b"1 rescue 2\n");
    let node = first_statement(&parsed);
    assert_eq!(node.kind_name(), "RescueModifierNode");
    let view = node.rescue_modifier_view().expect("modifier");
    assert_eq!(view.expression.kind_name(), "IntegerNode");
    assert_eq!(view.rescue_expression.kind_name(), "IntegerNode");
}

#[test]
fn multi_write_accessors_cover_targets_and_rest() {
    let parsed = parse(b"a, *b, c = [1, 2, 3]\n");
    let node = first_statement(&parsed);
    assert_eq!(node.kind_name(), "MultiWriteNode");
    let view = node.multi_write_view().expect("multi write");
    assert_eq!(view.lefts.len(), 1);
    assert_eq!(view.lefts[0].kind_name(), "LocalVariableTargetNode");
    assert!(view.lefts[0].lvar_target().is_some());
    let rest = view.rest.expect("rest");
    assert_eq!(rest.kind_name(), "SplatNode");
    assert!(rest.splat_value().expect("splat").is_some());
    assert_eq!(view.rights.len(), 1);
    assert_eq!(view.value.kind_name(), "ArrayNode");
}

#[test]
fn multi_write_accessors_cover_nested_target_and_implicit_rest() {
    let parsed = parse(b"(a, b), c = [1, 2], 3\n");
    let node = first_statement(&parsed);
    let view = node.multi_write_view().expect("multi write");
    assert_eq!(view.lefts.len(), 2);
    assert_eq!(view.lefts[0].kind_name(), "MultiTargetNode");
    assert_eq!(view.lefts[1].kind_name(), "LocalVariableTargetNode");
    let inner = view.lefts[0].multi_target_view().expect("multi target");
    assert_eq!(inner.lefts.len(), 2);
    assert!(inner.rest.is_none());
    assert!(view.rights.is_empty());

    let parsed = parse(b"a, = [1, 2]\n");
    let node = first_statement(&parsed);
    let view = node.multi_write_view().expect("multi write");
    assert_eq!(view.rest.expect("rest").kind_name(), "ImplicitRestNode");
}

#[test]
fn call_args_include_splats_and_keywords() {
    let parsed = parse(b"f(*a, b: 1)\n");
    let node = first_statement(&parsed);
    let call = node.call().expect("call");
    let args = call.args.expect("args");
    let items = args.call_args().expect("items");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].kind_name(), "SplatNode");
    assert_eq!(items[1].kind_name(), "KeywordHashNode");
    assert!(!args.args_forwarding());
}

#[test]
fn array_elements_include_splats() {
    let parsed = parse(b"[1, *a]\n");
    let node = first_statement(&parsed);
    assert_eq!(node.kind_name(), "ArrayNode");
    let elements = node.array_elements().expect("elements");
    assert_eq!(elements.len(), 2);
    assert_eq!(elements[1].kind_name(), "SplatNode");
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
    assert_eq!(node.array_elements().expect("elements").len(), 2);
}
