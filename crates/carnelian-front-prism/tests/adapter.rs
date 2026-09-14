//! Frontend adapter smoke tests (borrowed nodes, no codegen yet).

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
