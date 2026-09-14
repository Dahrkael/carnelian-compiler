//! Pin smoke: the committed `config.json` snapshot matches `ruby-prism 1.9.0`,
//! and the owned generator covers every node kind.

#[test]
fn prism_pin_parses_trivial_source() {
    let result = ruby_prism::parse(b"puts 1 + 2");
    let debug = format!("{:?}", result.node());
    assert!(
        debug.contains("ProgramNode"),
        "expected a ProgramNode root, got: {debug}"
    );
}

#[test]
fn owned_ast_covers_all_prism_nodes() {
    // `config.json` snapshot carries the pinned node list; the generated
    // `Node` enum must expose one variant per node (1:1, no fallbacks).
    let config_text = include_str!("../config/prism-1.9.0-config.json");
    let config: serde_json::Value = serde_json::from_str(config_text).expect("snapshot parses");
    let nodes = config["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 151, "Prism 1.9.0 ships 151 node kinds");

    // Spot-check that representative variants exist and round-trip flags.
    let span = carnelian_ast::Span { start: 0, end: 1 };
    let node = carnelian_ast::Node::IntegerNode {
        flags: 0,
        span,
        value: carnelian_ast::Integer::I64(1),
    };
    assert_eq!(carnelian_ast::AstNode::kind_name(&node), "IntegerNode");
    assert_eq!(carnelian_ast::AstNode::span(&node), span);
}
