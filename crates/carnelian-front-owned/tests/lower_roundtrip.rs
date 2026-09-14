//! Lowering round-trip tests: FFI tree in, owned tree out.
//!
//! The census sweep proves every node survives with kind, span and flags
//! intact; targeted tests pin field shapes per family.

use carnelian_ast::{Integer, Node, SymbolPool, Visit as _};
use carnelian_front_owned::lower;
use carnelian_front_prism as front;
use ruby_prism_sys::{
    pm_arguments_node_flags, pm_call_node_flags, pm_loop_flags, pm_range_flags,
    pm_regular_expression_flags,
};

fn lowered(source: &str) -> (Node, SymbolPool) {
    let parsed = front::parse(source.as_bytes());
    assert!(parsed.errors().is_empty(), "parse errors for {source:?}");
    lower(parsed.root())
}

fn program_statements(node: &Node) -> &[Node] {
    let Node::ProgramNode { statements, .. } = node else {
        panic!("expected program, got {}", node.kind_name());
    };
    let Node::StatementsNode { body, .. } = statements.as_ref() else {
        panic!("expected statements");
    };
    body
}

fn first_stmt(source: &str) -> (Node, SymbolPool) {
    let (node, pool) = lowered(source);
    let body = program_statements(&node);
    assert_eq!(body.len(), 1, "one statement for {source:?}");
    (body[0].clone(), pool)
}

fn pool_name(pool: &SymbolPool, id: carnelian_ast::SymbolId) -> &[u8] {
    pool.lookup(id).expect("interned name")
}

// FFI census through the generated visitor; owned census through Visit.
struct FfiCensus {
    rows: Vec<(String, u32, u32, u16)>,
}

impl FfiCensus {
    fn push(&mut self, node: &ruby_prism::Node<'_>) {
        let location = node.location();
        self.rows.push((
            front::node_kind_name(node).to_owned(),
            u32::try_from(location.start_offset()).unwrap_or(u32::MAX),
            u32::try_from(location.end_offset()).unwrap_or(u32::MAX),
            front::node_flags(node),
        ));
    }
}

// The generated FFI visitor bypasses `visit` for typed children, so record
// every node through its typed hook (exactly once per node).
macro_rules! record_nodes {
    ($(($method:ident, $node:ident)),*) => {
        $(
            fn $method(&mut self, node: &ruby_prism::$node<'_>) {
                self.push(&node.as_node());
                ruby_prism::$method(self, node);
            }
        )*
    };
}

impl<'pr> ruby_prism::Visit<'pr> for FfiCensus {
    record_nodes! {
        (visit_alias_global_variable_node, AliasGlobalVariableNode),
        (visit_alias_method_node, AliasMethodNode),
        (visit_alternation_pattern_node, AlternationPatternNode),
        (visit_and_node, AndNode),
        (visit_arguments_node, ArgumentsNode),
        (visit_array_node, ArrayNode),
        (visit_array_pattern_node, ArrayPatternNode),
        (visit_assoc_node, AssocNode),
        (visit_assoc_splat_node, AssocSplatNode),
        (visit_back_reference_read_node, BackReferenceReadNode),
        (visit_begin_node, BeginNode),
        (visit_block_argument_node, BlockArgumentNode),
        (visit_block_local_variable_node, BlockLocalVariableNode),
        (visit_block_node, BlockNode),
        (visit_block_parameter_node, BlockParameterNode),
        (visit_block_parameters_node, BlockParametersNode),
        (visit_break_node, BreakNode),
        (visit_call_and_write_node, CallAndWriteNode),
        (visit_call_node, CallNode),
        (visit_call_operator_write_node, CallOperatorWriteNode),
        (visit_call_or_write_node, CallOrWriteNode),
        (visit_call_target_node, CallTargetNode),
        (visit_capture_pattern_node, CapturePatternNode),
        (visit_case_match_node, CaseMatchNode),
        (visit_case_node, CaseNode),
        (visit_class_node, ClassNode),
        (visit_class_variable_and_write_node, ClassVariableAndWriteNode),
        (visit_class_variable_operator_write_node, ClassVariableOperatorWriteNode),
        (visit_class_variable_or_write_node, ClassVariableOrWriteNode),
        (visit_class_variable_read_node, ClassVariableReadNode),
        (visit_class_variable_target_node, ClassVariableTargetNode),
        (visit_class_variable_write_node, ClassVariableWriteNode),
        (visit_constant_and_write_node, ConstantAndWriteNode),
        (visit_constant_operator_write_node, ConstantOperatorWriteNode),
        (visit_constant_or_write_node, ConstantOrWriteNode),
        (visit_constant_path_and_write_node, ConstantPathAndWriteNode),
        (visit_constant_path_node, ConstantPathNode),
        (visit_constant_path_operator_write_node, ConstantPathOperatorWriteNode),
        (visit_constant_path_or_write_node, ConstantPathOrWriteNode),
        (visit_constant_path_target_node, ConstantPathTargetNode),
        (visit_constant_path_write_node, ConstantPathWriteNode),
        (visit_constant_read_node, ConstantReadNode),
        (visit_constant_target_node, ConstantTargetNode),
        (visit_constant_write_node, ConstantWriteNode),
        (visit_def_node, DefNode),
        (visit_defined_node, DefinedNode),
        (visit_else_node, ElseNode),
        (visit_embedded_statements_node, EmbeddedStatementsNode),
        (visit_embedded_variable_node, EmbeddedVariableNode),
        (visit_ensure_node, EnsureNode),
        (visit_false_node, FalseNode),
        (visit_find_pattern_node, FindPatternNode),
        (visit_flip_flop_node, FlipFlopNode),
        (visit_float_node, FloatNode),
        (visit_for_node, ForNode),
        (visit_forwarding_arguments_node, ForwardingArgumentsNode),
        (visit_forwarding_parameter_node, ForwardingParameterNode),
        (visit_forwarding_super_node, ForwardingSuperNode),
        (visit_global_variable_and_write_node, GlobalVariableAndWriteNode),
        (visit_global_variable_operator_write_node, GlobalVariableOperatorWriteNode),
        (visit_global_variable_or_write_node, GlobalVariableOrWriteNode),
        (visit_global_variable_read_node, GlobalVariableReadNode),
        (visit_global_variable_target_node, GlobalVariableTargetNode),
        (visit_global_variable_write_node, GlobalVariableWriteNode),
        (visit_hash_node, HashNode),
        (visit_hash_pattern_node, HashPatternNode),
        (visit_if_node, IfNode),
        (visit_imaginary_node, ImaginaryNode),
        (visit_implicit_node, ImplicitNode),
        (visit_implicit_rest_node, ImplicitRestNode),
        (visit_in_node, InNode),
        (visit_index_and_write_node, IndexAndWriteNode),
        (visit_index_operator_write_node, IndexOperatorWriteNode),
        (visit_index_or_write_node, IndexOrWriteNode),
        (visit_index_target_node, IndexTargetNode),
        (visit_instance_variable_and_write_node, InstanceVariableAndWriteNode),
        (visit_instance_variable_operator_write_node, InstanceVariableOperatorWriteNode),
        (visit_instance_variable_or_write_node, InstanceVariableOrWriteNode),
        (visit_instance_variable_read_node, InstanceVariableReadNode),
        (visit_instance_variable_target_node, InstanceVariableTargetNode),
        (visit_instance_variable_write_node, InstanceVariableWriteNode),
        (visit_integer_node, IntegerNode),
        (visit_interpolated_match_last_line_node, InterpolatedMatchLastLineNode),
        (visit_interpolated_regular_expression_node, InterpolatedRegularExpressionNode),
        (visit_interpolated_string_node, InterpolatedStringNode),
        (visit_interpolated_symbol_node, InterpolatedSymbolNode),
        (visit_interpolated_x_string_node, InterpolatedXStringNode),
        (visit_it_local_variable_read_node, ItLocalVariableReadNode),
        (visit_it_parameters_node, ItParametersNode),
        (visit_keyword_hash_node, KeywordHashNode),
        (visit_keyword_rest_parameter_node, KeywordRestParameterNode),
        (visit_lambda_node, LambdaNode),
        (visit_local_variable_and_write_node, LocalVariableAndWriteNode),
        (visit_local_variable_operator_write_node, LocalVariableOperatorWriteNode),
        (visit_local_variable_or_write_node, LocalVariableOrWriteNode),
        (visit_local_variable_read_node, LocalVariableReadNode),
        (visit_local_variable_target_node, LocalVariableTargetNode),
        (visit_local_variable_write_node, LocalVariableWriteNode),
        (visit_match_last_line_node, MatchLastLineNode),
        (visit_match_predicate_node, MatchPredicateNode),
        (visit_match_required_node, MatchRequiredNode),
        (visit_match_write_node, MatchWriteNode),
        (visit_missing_node, MissingNode),
        (visit_module_node, ModuleNode),
        (visit_multi_target_node, MultiTargetNode),
        (visit_multi_write_node, MultiWriteNode),
        (visit_next_node, NextNode),
        (visit_nil_node, NilNode),
        (visit_no_keywords_parameter_node, NoKeywordsParameterNode),
        (visit_numbered_parameters_node, NumberedParametersNode),
        (visit_numbered_reference_read_node, NumberedReferenceReadNode),
        (visit_optional_keyword_parameter_node, OptionalKeywordParameterNode),
        (visit_optional_parameter_node, OptionalParameterNode),
        (visit_or_node, OrNode),
        (visit_parameters_node, ParametersNode),
        (visit_parentheses_node, ParenthesesNode),
        (visit_pinned_expression_node, PinnedExpressionNode),
        (visit_pinned_variable_node, PinnedVariableNode),
        (visit_post_execution_node, PostExecutionNode),
        (visit_pre_execution_node, PreExecutionNode),
        (visit_program_node, ProgramNode),
        (visit_range_node, RangeNode),
        (visit_rational_node, RationalNode),
        (visit_redo_node, RedoNode),
        (visit_regular_expression_node, RegularExpressionNode),
        (visit_required_keyword_parameter_node, RequiredKeywordParameterNode),
        (visit_required_parameter_node, RequiredParameterNode),
        (visit_rescue_modifier_node, RescueModifierNode),
        (visit_rescue_node, RescueNode),
        (visit_rest_parameter_node, RestParameterNode),
        (visit_retry_node, RetryNode),
        (visit_return_node, ReturnNode),
        (visit_self_node, SelfNode),
        (visit_shareable_constant_node, ShareableConstantNode),
        (visit_singleton_class_node, SingletonClassNode),
        (visit_source_encoding_node, SourceEncodingNode),
        (visit_source_file_node, SourceFileNode),
        (visit_source_line_node, SourceLineNode),
        (visit_splat_node, SplatNode),
        (visit_statements_node, StatementsNode),
        (visit_string_node, StringNode),
        (visit_super_node, SuperNode),
        (visit_symbol_node, SymbolNode),
        (visit_true_node, TrueNode),
        (visit_undef_node, UndefNode),
        (visit_unless_node, UnlessNode),
        (visit_until_node, UntilNode),
        (visit_when_node, WhenNode),
        (visit_while_node, WhileNode),
        (visit_x_string_node, XStringNode),
        (visit_yield_node, YieldNode)
    }
}

struct OwnedCensus {
    rows: Vec<(String, u32, u32, u16)>,
}

impl carnelian_ast::Visit for OwnedCensus {
    fn visit(&mut self, node: &Node) {
        let span = node.span();
        self.rows.push((
            node.kind_name().to_owned(),
            span.start,
            span.end,
            node.flags(),
        ));
        carnelian_ast::visit_children(self, node);
    }
}

/// Every node survives lowering with kind, span and flags intact.
fn assert_census_eq(source: &str) {
    let parsed = front::parse(source.as_bytes());
    assert!(parsed.errors().is_empty(), "parse errors for {source:?}");
    let root = parsed.root();
    let mut ffi = FfiCensus { rows: Vec::new() };
    ruby_prism::Visit::visit(&mut ffi, root.inner());
    let (node, _pool) = lower(root);
    let mut owned = OwnedCensus { rows: Vec::new() };
    owned.visit(&node);
    ffi.rows.sort();
    owned.rows.sort();
    assert_eq!(ffi.rows, owned.rows, "census mismatch for {source:?}");
}

#[test]
fn census_sweep_matches_ffi() {
    let snippets = [
        "x = 1",
        "x, y = 1, 2",
        "a, (b, *c), @d, @@e, $f, C, C::D, o.x, o[0] = r",
        "a, *b, c = r",
        "a += 1",
        "a -= 1",
        "@a ||= 1",
        "@@a &&= 1",
        "$a += 1",
        "C += 1",
        "C::D ||= 1",
        "o.x += 1",
        "o[k] += 1",
        "o[k] ||= 1",
        "def f(a, b = 1, *c, d, e:, f: 2, **g, &blk); end",
        "def f(&); end",
        "def f(...); foo(...); end",
        "def self.f(a) = a",
        "def f = 1",
        "foo",
        "foo()",
        "foo(1, *a, b: 2, **c, &d)",
        "foo { |a, b = 1| a }",
        "foo { |;x| }",
        "foo { _1 }",
        "foo { it }",
        "->(x) { x }",
        "-> {}",
        "a&.b",
        "a.b = 1",
        "A::B",
        "::A",
        "@a",
        "@@a",
        "$1",
        "$&",
        "$'",
        "$+",
        "$`",
        "$g",
        "self",
        "nil",
        "true",
        "false",
        "__FILE__",
        "__LINE__",
        "__ENCODING__",
        "1",
        "-5",
        "0xFF",
        "0b101",
        "0o17",
        "1_000",
        "9223372036854775807",
        "9223372036854775808",
        "-9223372036854775808",
        "-9223372036854775809",
        "0xFFFFFFFFFFFFFFFFFF",
        "123456789012345678901234567890",
        "1.5",
        "-1.5",
        "1.5r",
        "-1.5r",
        "1i",
        "\"abc\"",
        "\"a\\nb\"",
        "\"\\xff\"",
        ":sym",
        "\"interp #{a} end\"",
        "\"#@a\"",
        "\"#$g\"",
        "\"#{} stem\"",
        "`ls`",
        "`run #{x}`",
        "/abc/i",
        "/a#{b}c/mx",
        "if /re/ then end",
        "if a then b else c end",
        "unless a then b else c end",
        "a ? b : c",
        "a rescue b",
        "while a do b end",
        "until a do b end",
        "begin b end while a",
        "begin b end until a",
        "for i in c do b end",
        "a..b",
        "a...b",
        "(1..)",
        "(..5)",
        "if (a..b) then c end",
        "if (a...b) then c end",
        "case x; when 1, 2 then y; else z; end",
        "case x; in Integer => y; else z; end",
        "case x; in [a, *b, c] => y; end",
        "case x; in [*, y, *] => z; end",
        "case x; in {a:} => y; end",
        "case x; in Integer | String => y; end",
        "@m = 1; case x; in ^@m => y; end",
        "case x; in ^(1 + 2) => y; end",
        "case x; in CONST => y; end",
        "y in [a]",
        "1 => a",
        "begin; a; rescue E => e; b; else c; ensure d; end",
        "begin; a; rescue; b; end",
        "begin; end",
        "()",
        "(1)",
        "(1; 2)",
        "class A; end",
        "class A::B < C; end",
        "module M; end",
        "class << self; end",
        "alias a b",
        "alias $x $y",
        "undef a, b",
        "defined?(x)",
        "defined?(@x)",
        "defined?(C::D)",
        "super",
        "super()",
        "super(1, *a)",
        "super { }",
        "def f; yield; end",
        "def f; yield 1; end",
        "while true; break 1; end",
        "while true; next; end",
        "while true; redo; end",
        "def f; return 1; end",
        "begin; a; rescue; retry; end",
        "END { a }",
        "BEGIN { a }",
        "not x",
        "!x",
        "~a",
        "-a",
        "a && b",
        "a || b",
        "a and b",
        "a or b",
        "a = b = 1",
        "C &&= 1",
        "C::D &&= 1",
        "# shareable_constant_value: literal\nC = 1",
        "p *a, &b",
        "foo(1, 2)",
        "a => b",
    ];
    for source in snippets {
        assert_census_eq(source);
    }
}

#[test]
fn program_keeps_locals_in_order() {
    let (node, pool) = lowered("z, a, m = 1, 2, 3");
    let Node::ProgramNode { locals, .. } = &node else {
        panic!("program");
    };
    let names: Vec<&[u8]> = locals.iter().map(|id| pool_name(&pool, *id)).collect();
    assert_eq!(names, [b"z".as_slice(), b"a".as_slice(), b"m".as_slice()]);
    assert_eq!(node.span().start, 0);
    assert_eq!(node.span().end, 17);
}

#[test]
fn full_params_shape() {
    let (node, pool) = first_stmt("def f(a, b = 1, *c, d, e:, f: 2, **g, &blk); end");
    let Node::DefNode { parameters, .. } = &node else {
        panic!("def");
    };
    let params = parameters.as_ref().expect("params");
    let Node::ParametersNode {
        requireds,
        optionals,
        rest,
        posts,
        keywords,
        keyword_rest,
        block,
        ..
    } = params.as_ref()
    else {
        panic!("parameters");
    };
    assert_eq!(requireds.len(), 1);
    let Node::RequiredParameterNode { name, .. } = &requireds[0] else {
        panic!("required");
    };
    assert_eq!(pool_name(&pool, *name), b"a");
    assert_eq!(optionals.len(), 1);
    let Node::OptionalParameterNode { name, value, .. } = &optionals[0] else {
        panic!("optional");
    };
    assert_eq!(pool_name(&pool, *name), b"b");
    let Node::IntegerNode { value, .. } = value.as_ref() else {
        panic!("default");
    };
    assert_eq!(*value, Integer::I64(1));
    let Node::RestParameterNode { name, .. } = rest.as_ref().expect("rest").as_ref() else {
        panic!("rest");
    };
    assert_eq!(pool_name(&pool, name.expect("rest name")), b"c");
    assert_eq!(posts.len(), 1);
    let Node::RequiredParameterNode { name, .. } = &posts[0] else {
        panic!("post");
    };
    assert_eq!(pool_name(&pool, *name), b"d");
    assert_eq!(keywords.len(), 2);
    let Node::RequiredKeywordParameterNode { name, .. } = &keywords[0] else {
        panic!("kw required");
    };
    assert_eq!(pool_name(&pool, *name), b"e");
    let Node::OptionalKeywordParameterNode { name, value, .. } = &keywords[1] else {
        panic!("kw optional");
    };
    assert_eq!(pool_name(&pool, *name), b"f");
    let Node::IntegerNode { value, .. } = value.as_ref() else {
        panic!("kw default");
    };
    assert_eq!(*value, Integer::I64(2));
    let Node::KeywordRestParameterNode { name, .. } =
        keyword_rest.as_ref().expect("kw rest").as_ref()
    else {
        panic!("keyword rest");
    };
    assert_eq!(pool_name(&pool, name.expect("kw rest name")), b"g");
    let Node::BlockParameterNode { name, .. } = block.as_ref().expect("block").as_ref() else {
        panic!("block param");
    };
    assert_eq!(pool_name(&pool, name.expect("block name")), b"blk");
    // Def locals follow source order.
    let Node::DefNode { locals, .. } = &node else {
        panic!("def");
    };
    let names: Vec<&[u8]> = locals.iter().map(|id| pool_name(&pool, *id)).collect();
    assert_eq!(
        names,
        [
            b"a".as_slice(),
            b"b".as_slice(),
            b"c".as_slice(),
            b"d".as_slice(),
            b"e".as_slice(),
            b"f".as_slice(),
            b"g".as_slice(),
            b"blk".as_slice()
        ]
    );
    let _ = locals;
}

#[test]
fn anonymous_block_param_keeps_absent_name() {
    // Stock Prism rejects `&nil`, so only the unnamed form is covered here.
    let (node, _) = first_stmt("def f(&); end");
    let Node::DefNode { parameters, .. } = &node else {
        panic!("def");
    };
    let params = parameters.as_ref().expect("params");
    let Node::ParametersNode { block, .. } = params.as_ref() else {
        panic!("parameters");
    };
    let block = block.as_ref().expect("block param");
    let Node::BlockParameterNode { name, .. } = block.as_ref() else {
        panic!("block");
    };
    assert!(name.is_none());
}

#[test]
fn none_vs_empty_optionals() {
    // Bare call has no arguments and no parens; empty parens keep locations.
    let (bare, _) = first_stmt("foo");
    let Node::CallNode {
        arguments,
        block,
        opening_loc,
        ..
    } = &bare
    else {
        panic!("call");
    };
    assert!(arguments.is_none());
    assert!(block.is_none());
    assert!(opening_loc.is_none());
    let (parens, _) = first_stmt("foo()");
    let Node::CallNode {
        arguments,
        opening_loc,
        closing_loc,
        ..
    } = &parens
    else {
        panic!("call");
    };
    assert!(arguments.is_none());
    assert!(opening_loc.is_some());
    assert!(closing_loc.is_some());
    // Empty parens body is absent; a value body is present.
    let (empty, _) = first_stmt("()");
    let Node::ParenthesesNode { body, .. } = &empty else {
        panic!("parens");
    };
    assert!(body.is_none());
    let (one, _) = first_stmt("(1)");
    let Node::ParenthesesNode { body, .. } = &one else {
        panic!("parens");
    };
    assert!(body.is_some());
    // Empty interpolation has no statements node; an empty program keeps one.
    let (interp, _) = first_stmt("\"#{}\"");
    let Node::InterpolatedStringNode { parts, .. } = &interp else {
        panic!("istring");
    };
    assert_eq!(parts.len(), 1);
    let Node::EmbeddedStatementsNode { statements, .. } = &parts[0] else {
        panic!("embedded");
    };
    assert!(statements.is_none());
    let (empty_program, _) = lowered("");
    let Node::ProgramNode { statements, .. } = &empty_program else {
        panic!("program");
    };
    let Node::StatementsNode { body, .. } = statements.as_ref() else {
        panic!("stmts");
    };
    assert!(body.is_empty());
    // Valueless if branch is None.
    let (branch, _) = first_stmt("if a then end");
    let Node::IfNode { statements, .. } = &branch else {
        panic!("if");
    };
    assert!(statements.is_none());
}

#[test]
fn integer_models_cover_radix_and_limits() {
    let cases: &[(&str, Integer)] = &[
        ("1", Integer::I64(1)),
        ("-5", Integer::I64(-5)),
        ("0xFF", Integer::I64(255)),
        ("0b101", Integer::I64(5)),
        ("0o17", Integer::I64(15)),
        ("1_000", Integer::I64(1000)),
        ("0_0_7", Integer::I64(7)),
        ("9223372036854775807", Integer::I64(i64::MAX)),
        ("-9223372036854775808", Integer::I64(i64::MIN)),
        (
            "9223372036854775808",
            Integer::Fallback {
                raw: b"9223372036854775808".to_vec(),
            },
        ),
        (
            "-9223372036854775809",
            Integer::Fallback {
                raw: b"-9223372036854775809".to_vec(),
            },
        ),
        (
            "0xFFFFFFFFFFFFFFFFFF",
            Integer::Fallback {
                raw: b"4722366482869645213695".to_vec(),
            },
        ),
        (
            "123456789012345678901234567890",
            Integer::Fallback {
                raw: b"123456789012345678901234567890".to_vec(),
            },
        ),
    ];
    for (source, expected) in cases {
        let (node, _) = first_stmt(source);
        let Node::IntegerNode { value, .. } = &node else {
            panic!("integer for {source:?}");
        };
        assert_eq!(value, expected, "integer for {source:?}");
    }
    // Folded minus extends the node span over the sign.
    let (node, _) = first_stmt("-5");
    assert_eq!((node.span().start, node.span().end), (0, 2));
}

#[test]
fn rational_float_imaginary_shapes() {
    let (node, _) = first_stmt("1.5r");
    let Node::RationalNode {
        numerator,
        denominator,
        ..
    } = &node
    else {
        panic!("rational");
    };
    assert_eq!(*numerator, Integer::I64(3));
    assert_eq!(*denominator, Integer::I64(2));
    let (node, _) = first_stmt("-1.5r");
    let Node::RationalNode { numerator, .. } = &node else {
        panic!("rational");
    };
    assert_eq!(*numerator, Integer::I64(-3));
    let (node, _) = first_stmt("-1.5");
    let Node::FloatNode { value, .. } = &node else {
        panic!("float");
    };
    assert_eq!(*value, -1.5);
    let (node, _) = first_stmt("1i");
    let Node::ImaginaryNode { numeric, .. } = &node else {
        panic!("imaginary");
    };
    assert!(matches!(numeric.as_ref(), Node::IntegerNode { .. }));
}

#[test]
fn strings_keep_unescaped_bytes() {
    let (node, _) = first_stmt("\"a\\nb\"");
    let Node::StringNode { unescaped, .. } = &node else {
        panic!("string");
    };
    assert_eq!(unescaped, b"a\nb");
    let (node, _) = first_stmt("\"\\xff\"");
    let Node::StringNode { unescaped, .. } = &node else {
        panic!("string");
    };
    assert_eq!(unescaped, &[0xff]);
    let (node, _) = first_stmt(":sym");
    let Node::SymbolNode { unescaped, .. } = &node else {
        panic!("symbol");
    };
    assert_eq!(unescaped, b"sym");
    let (node, _) = first_stmt("/abc/i");
    assert_ne!(
        node.flags()
            & (pm_regular_expression_flags::PM_REGULAR_EXPRESSION_FLAGS_IGNORE_CASE as u16),
        0
    );
}

#[test]
fn kwargs_forms() {
    let (node, _) = first_stmt("f(a: 1, \"b\" => 2, **c)");
    let Node::CallNode { arguments, .. } = &node else {
        panic!("call");
    };
    let args = arguments.as_ref().expect("args");
    let Node::ArgumentsNode { arguments, .. } = args.as_ref() else {
        panic!("args");
    };
    assert_eq!(arguments.len(), 1);
    let Node::KeywordHashNode { elements, .. } = &arguments[0] else {
        panic!("kwhash");
    };
    assert_eq!(elements.len(), 3);
    let Node::AssocNode { key, .. } = &elements[0] else {
        panic!("assoc");
    };
    assert!(matches!(key.as_ref(), Node::SymbolNode { .. }));
    let Node::AssocNode { key, .. } = &elements[1] else {
        panic!("assoc");
    };
    assert!(matches!(key.as_ref(), Node::StringNode { .. }));
    let Node::AssocSplatNode { value, .. } = &elements[2] else {
        panic!("splat");
    };
    assert!(value.is_some());
}

#[test]
fn masgn_covers_all_target_families() {
    let (node, _) = first_stmt("a, (b, *c), @d, @@e, $f, C, C::D, o.x, o[0] = 1");
    let Node::MultiWriteNode {
        lefts,
        rest,
        rights,
        value,
        ..
    } = &node
    else {
        panic!("masgn");
    };
    let kinds: Vec<&str> = lefts.iter().map(|child| child.kind_name()).collect();
    assert_eq!(
        kinds,
        [
            "LocalVariableTargetNode",
            "MultiTargetNode",
            "InstanceVariableTargetNode",
            "ClassVariableTargetNode",
            "GlobalVariableTargetNode",
            "ConstantTargetNode",
            "ConstantPathTargetNode",
            "CallTargetNode",
            "IndexTargetNode"
        ]
    );
    assert!(rest.is_none());
    assert!(rights.is_empty());
    assert!(matches!(value.as_ref(), Node::IntegerNode { .. }));
    let Node::MultiTargetNode { rest, .. } = &lefts[1] else {
        panic!("nested");
    };
    assert!(matches!(
        rest.as_ref().expect("splat").as_ref(),
        Node::SplatNode { .. }
    ));
}

#[test]
fn operator_writes_cover_families() {
    let (node, pool) = first_stmt("a += 1");
    let Node::LocalVariableOperatorWriteNode {
        name,
        binary_operator,
        depth,
        ..
    } = &node
    else {
        panic!("op write");
    };
    assert_eq!(pool_name(&pool, *name), b"a");
    assert_eq!(pool_name(&pool, *binary_operator), b"+");
    assert_eq!(*depth, 0);
    let (node, pool) = first_stmt("o.x += 1");
    let Node::CallOperatorWriteNode {
        read_name,
        write_name,
        binary_operator,
        receiver,
        ..
    } = &node
    else {
        panic!("call op write");
    };
    assert_eq!(pool_name(&pool, *read_name), b"x");
    assert_eq!(pool_name(&pool, *write_name), b"x=");
    assert_eq!(pool_name(&pool, *binary_operator), b"+");
    assert!(matches!(
        receiver.as_ref().expect("recv").as_ref(),
        Node::CallNode { .. }
    ));
    let (node, _) = first_stmt("C::D ||= 1");
    let Node::ConstantPathOrWriteNode { target, .. } = &node else {
        panic!("cpath or write");
    };
    assert!(matches!(target.as_ref(), Node::ConstantPathNode { .. }));
}

#[test]
fn blocks_cover_locals_and_numbered_forms() {
    let (node, pool) = first_stmt("foo { |;x| }");
    let Node::CallNode { block, .. } = &node else {
        panic!("call");
    };
    let block = block.as_ref().expect("block");
    let Node::BlockNode { parameters, .. } = block.as_ref() else {
        panic!("block");
    };
    let params = parameters.as_ref().expect("block parameters");
    let Node::BlockParametersNode { locals, .. } = params.as_ref() else {
        panic!("params");
    };
    assert_eq!(locals.len(), 1);
    let Node::BlockLocalVariableNode { name, .. } = &locals[0] else {
        panic!("local");
    };
    assert_eq!(pool_name(&pool, *name), b"x");
    let (node, pool) = first_stmt("foo { _1 }");
    let Node::CallNode { block, .. } = &node else {
        panic!("call");
    };
    let block = block.as_ref().expect("block");
    let Node::BlockNode {
        parameters, body, ..
    } = block.as_ref()
    else {
        panic!("block");
    };
    // Numbered reads desugar to a local plus a max-1 numbered params node.
    let params = parameters.as_ref().expect("params");
    let Node::NumberedParametersNode { maximum, .. } = params.as_ref() else {
        panic!("numbered params");
    };
    assert_eq!(*maximum, 1);
    let body = body.as_ref().expect("body");
    let Node::StatementsNode { body, .. } = body.as_ref() else {
        panic!("stmts");
    };
    assert_eq!(body.len(), 1);
    let Node::LocalVariableReadNode { name, .. } = &body[0] else {
        panic!("read");
    };
    assert_eq!(pool_name(&pool, *name), b"_1");
    let (node, _) = first_stmt("foo { it }");
    let Node::CallNode { block, .. } = &node else {
        panic!("call");
    };
    let block = block.as_ref().expect("block");
    let Node::BlockNode {
        parameters, body, ..
    } = block.as_ref()
    else {
        panic!("block");
    };
    let params = parameters.as_ref().expect("params");
    assert!(matches!(params.as_ref(), Node::ItParametersNode { .. }));
    let body = body.as_ref().expect("body");
    let Node::StatementsNode { body, .. } = body.as_ref() else {
        panic!("stmts");
    };
    assert_eq!(body.len(), 1);
    assert!(matches!(&body[0], Node::ItLocalVariableReadNode { .. }));
}

#[test]
fn defs_cover_endless_and_singleton_forms() {
    let (node, pool) = first_stmt("def self.f(a) = a");
    let Node::DefNode {
        receiver,
        end_keyword_loc,
        equal_loc,
        body,
        name,
        ..
    } = &node
    else {
        panic!("def");
    };
    assert_eq!(pool_name(&pool, *name), b"f");
    assert!(matches!(
        receiver.as_ref().expect("recv").as_ref(),
        Node::SelfNode { .. }
    ));
    assert!(end_keyword_loc.is_none());
    assert!(equal_loc.is_some());
    assert!(body.is_some());
    let (node, _) = first_stmt("class A::B < C; end");
    let Node::ClassNode {
        superclass, body, ..
    } = &node
    else {
        panic!("class");
    };
    assert!(superclass.is_some());
    assert!(body.is_none());
}

#[test]
fn backrefs_keep_names() {
    for (source, expected) in [
        ("$1", b"1".as_slice()),
        ("$&", b"$&".as_slice()),
        ("$'", b"$'".as_slice()),
        ("$+", b"$+".as_slice()),
    ] {
        let (node, pool) = first_stmt(source);
        match &node {
            Node::NumberedReferenceReadNode { number, .. } => {
                assert_eq!((*number, expected), (1, b"1".as_slice()));
            }
            Node::BackReferenceReadNode { name, .. } => {
                assert_eq!(pool_name(&pool, *name), expected, "{source:?}");
            }
            other => panic!("backref for {source:?}, got {}", other.kind_name()),
        }
    }
}

#[test]
fn call_flags_and_kinds_survive() {
    // Raw C flag bits (sys values); the generated name mods start at bit 0.
    let (node, _) = first_stmt("a&.b");
    assert_ne!(
        node.flags() & (pm_call_node_flags::PM_CALL_NODE_FLAGS_SAFE_NAVIGATION as u16),
        0
    );
    let (node, _) = first_stmt("a.b = 1");
    assert_ne!(
        node.flags() & (pm_call_node_flags::PM_CALL_NODE_FLAGS_ATTRIBUTE_WRITE as u16),
        0
    );
    let (node, _) = first_stmt("begin b end while a");
    assert_ne!(
        node.flags() & (pm_loop_flags::PM_LOOP_FLAGS_BEGIN_MODIFIER as u16),
        0
    );
    let (node, _) = first_stmt("a...b");
    assert_ne!(
        node.flags() & (pm_range_flags::PM_RANGE_FLAGS_EXCLUDE_END as u16),
        0
    );
    let (node, _) = first_stmt("def f(...); foo(...); end");
    let Node::DefNode { body, .. } = &node else {
        panic!("def");
    };
    let body = body.as_ref().expect("body");
    let Node::StatementsNode { body, .. } = body.as_ref() else {
        panic!("stmts");
    };
    let Node::CallNode { arguments, .. } = &body[0] else {
        panic!("call");
    };
    let args = arguments.as_ref().expect("args");
    let Node::ArgumentsNode { arguments, .. } = args.as_ref() else {
        panic!("args");
    };
    assert_ne!(
        args.flags()
            & (pm_arguments_node_flags::PM_ARGUMENTS_NODE_FLAGS_CONTAINS_FORWARDING as u16),
        0
    );
    let _ = arguments;
}

#[test]
fn const_paths_cover_rooted_forms() {
    let (node, _) = first_stmt("::A");
    let Node::ConstantPathNode { parent, name, .. } = &node else {
        panic!("cpath");
    };
    assert!(parent.is_none());
    assert!(name.is_some());
    let (node, _) = first_stmt("A::B");
    let Node::ConstantPathNode { parent, .. } = &node else {
        panic!("cpath");
    };
    assert!(parent.is_some());
}

#[test]
fn missing_node_survives_error_recovery() {
    let parsed = front::parse(b"1 +");
    assert!(!parsed.errors().is_empty());
    let (node, _pool) = lower(parsed.root());
    let mut census = OwnedCensus { rows: Vec::new() };
    census.visit(&node);
    assert!(
        census.rows.iter().any(|row| row.0 == "MissingNode"),
        "expected a MissingNode, got {:?}",
        census.rows.iter().map(|row| &row.0).collect::<Vec<_>>()
    );
}
