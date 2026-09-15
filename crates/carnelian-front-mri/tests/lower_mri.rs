//! Structural lowering tests: MRI tree to owned tree (P4-A).
//!
//! Each test parses a snippet with the pinned `lib-ruby-parser` and
//! asserts the owned shape. Scope fields stay blank here (`depth` zero,
//! `locals` empty, `NumberedParametersNode.maximum` zero).

use carnelian_ast::{
    arguments_node_flags, array_node_flags, call_node_flags, integer_base_flags,
    keyword_hash_node_flags, loop_flags, range_flags, regular_expression_flags, Integer, Node,
    SymbolId, SymbolPool,
};
use carnelian_front_mri::lower;

/// Parse, failing the test on error diagnostics (warnings are fine).
/// Options mirror production `parse()` so option-sensitive behavior
/// cannot drift between tests and the CLI.
fn parse(src: &str) -> Box<lib_ruby_parser::Node> {
    let options = lib_ruby_parser::ParserOptions {
        buffer_name: "(test)".to_string(),
        decoder: None,
        record_tokens: false,
    };
    let result = lib_ruby_parser::Parser::new(src.as_bytes().to_vec(), options).do_parse();
    let errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|diag| matches!(diag.level, lib_ruby_parser::ErrorLevel::Error))
        .collect();
    assert!(errors.is_empty(), "parse errors for {src:?}: {errors:?}");
    result.ast.expect("parser returned no tree")
}

/// Lower one snippet.
fn lowered(src: &str) -> (Node, SymbolPool) {
    let root = parse(src);
    lower(&root)
}

/// Resolve an interned name for assertions.
fn sym_name(pool: &SymbolPool, id: SymbolId) -> Vec<u8> {
    pool.lookup(id).unwrap().to_vec()
}

/// Single-expression integer lowering table.
#[test]
fn int_radix_and_edges() {
    let cases: &[(&str, Integer)] = &[
        ("0", Integer::I64(0)),
        ("42", Integer::I64(42)),
        ("-1", Integer::I64(-1)),
        ("+1", Integer::I64(1)),
        ("1_000", Integer::I64(1000)),
        ("0x10", Integer::I64(16)),
        ("0XFF", Integer::I64(255)),
        ("0b101", Integer::I64(5)),
        ("0B101", Integer::I64(5)),
        ("0o17", Integer::I64(15)),
        ("0d10", Integer::I64(10)),
        ("017", Integer::I64(15)),
        ("00", Integer::I64(0)),
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
            "99999999999999999999999",
            Integer::Fallback {
                raw: b"99999999999999999999999".to_vec(),
            },
        ),
        (
            "0xFFFFFFFFFFFFFFFF",
            Integer::Fallback {
                raw: b"18446744073709551615".to_vec(),
            },
        ),
        (
            "0x10000000000000000",
            Integer::Fallback {
                raw: b"18446744073709551616".to_vec(),
            },
        ),
        (
            "0b11111111111111111111111111111111111111111111111111111111111111111",
            Integer::Fallback {
                raw: b"36893488147419103231".to_vec(),
            },
        ),
    ];
    for (src, expect) in cases {
        let (node, _) = lowered(src);
        match node {
            Node::IntegerNode { value, .. } => assert_eq!(&value, expect, "int {src:?}"),
            other => panic!("{src:?} lowered to {}", other.kind_name()),
        }
    }
}

/// Integer base flags follow the literal prefix.
#[test]
fn int_base_flags() {
    let cases: &[(&str, u16)] = &[
        ("10", integer_base_flags::DECIMAL),
        ("0d10", integer_base_flags::DECIMAL),
        ("017", integer_base_flags::OCTAL),
        ("0o17", integer_base_flags::OCTAL),
        ("0b11", integer_base_flags::BINARY),
        ("0x11", integer_base_flags::HEXADECIMAL),
    ];
    for (src, flag) in cases {
        let (node, _) = lowered(src);
        match node {
            Node::IntegerNode { flags, .. } => assert_eq!(flags, *flag, "flags for {src:?}"),
            other => panic!("{src:?} lowered to {}", other.kind_name()),
        }
    }
}

/// Float payloads parse to `f64`.
#[test]
fn float_values() {
    for (src, expect) in [
        ("1.5", 1.5),
        ("-1.5", -1.5),
        ("1e3", 1000.0),
        ("1E-3", 0.001),
        ("1.0e-10", 1e-10),
        ("1_0.5", 10.5),
    ] {
        let (node, _) = lowered(src);
        match node {
            Node::FloatNode { value, .. } => assert_eq!(value, expect, "float {src:?}"),
            other => panic!("{src:?} lowered to {}", other.kind_name()),
        }
    }
}

/// Rational literals reduce to numerator/denominator.
#[test]
fn rational_values() {
    for (src, num, den) in [
        ("2r", Integer::I64(2), Integer::I64(1)),
        ("0.5r", Integer::I64(1), Integer::I64(2)),
        ("1.5r", Integer::I64(3), Integer::I64(2)),
        ("-2r", Integer::I64(-2), Integer::I64(1)),
        ("-1.5r", Integer::I64(-3), Integer::I64(2)),
    ] {
        let (node, _) = lowered(src);
        match node {
            Node::RationalNode {
                numerator,
                denominator,
                ..
            } => {
                assert_eq!(numerator, num, "numerator of {src:?}");
                assert_eq!(denominator, den, "denominator of {src:?}");
            }
            other => panic!("{src:?} lowered to {}", other.kind_name()),
        }
    }
}

/// Complex literals wrap their numeric payload in `ImaginaryNode`.
#[test]
fn complex_values() {
    let (node, _) = lowered("2i");
    assert!(matches!(
        node,
        Node::ImaginaryNode { ref numeric, .. }
            if matches!(**numeric, Node::IntegerNode { value: Integer::I64(2), .. })
    ));
    let (node, _) = lowered("-2i");
    assert!(matches!(
        node,
        Node::ImaginaryNode { ref numeric, .. }
            if matches!(**numeric, Node::IntegerNode { value: Integer::I64(-2), .. })
    ));
    let (node, _) = lowered("2.5i");
    assert!(matches!(
        node,
        Node::ImaginaryNode { ref numeric, .. }
            if matches!(**numeric, Node::FloatNode { value, .. } if value == 2.5)
    ));
}

/// Plain strings copy bytes and quote spans.
#[test]
fn plain_string() {
    let (node, _) = lowered("\"abc\"");
    match node {
        Node::StringNode {
            opening_loc,
            content_loc,
            closing_loc,
            unescaped,
            ..
        } => {
            assert_eq!(unescaped, b"abc");
            assert!(opening_loc.is_some());
            assert_eq!((content_loc.start, content_loc.end), (1, 4));
            assert!(closing_loc.is_some());
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Single-quoted strings keep literal bytes (no interpolation).
#[test]
fn single_quoted_string_keeps_bytes() {
    let (node, _) = lowered("'a#{b}'");
    match node {
        Node::StringNode { unescaped, .. } => assert_eq!(unescaped, b"a#{b}"),
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Symbols copy bytes with a value span.
#[test]
fn plain_symbol() {
    let (node, _) = lowered(":sym");
    match node {
        Node::SymbolNode {
            value_loc,
            unescaped,
            ..
        } => {
            assert_eq!(unescaped, b"sym");
            assert!(value_loc.is_some());
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Interpolated strings split parts with embedded statements.
#[test]
fn interpolated_string_parts() {
    let (node, _) = lowered("\"a#{b}c\"");
    match node {
        Node::InterpolatedStringNode { parts, .. } => {
            assert_eq!(parts.len(), 3);
            assert!(matches!(parts[0], Node::StringNode { .. }));
            assert!(matches!(parts[1], Node::EmbeddedStatementsNode { .. }));
            assert!(matches!(parts[2], Node::StringNode { .. }));
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// `#@ivar` style interpolation wraps reads in embedded variables.
#[test]
fn interpolated_ivar_part() {
    let (node, _) = lowered("\"#@a\"");
    match node {
        Node::InterpolatedStringNode { parts, .. } => {
            assert_eq!(parts.len(), 1);
            assert!(matches!(parts[0], Node::EmbeddedVariableNode { .. }));
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Plain regexps copy bytes; option letters become flag bits.
#[test]
fn regexp_plain_and_flags() {
    let (node, _) = lowered("/re/imx");
    match node {
        Node::RegularExpressionNode {
            flags, unescaped, ..
        } => {
            assert_eq!(unescaped, b"re");
            assert_eq!(
                flags,
                regular_expression_flags::IGNORE_CASE
                    | regular_expression_flags::EXTENDED
                    | regular_expression_flags::MULTI_LINE
            );
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("/re#{x}/");
    assert!(matches!(
        node,
        Node::InterpolatedRegularExpressionNode { .. }
    ));
}

/// Backtick strings lower to `XString` shapes.
#[test]
fn xstring_shapes() {
    let (node, _) = lowered("`cmd`");
    assert!(matches!(node, Node::XStringNode { .. }));
    let (node, _) = lowered("`cmd#{x}`");
    assert!(matches!(node, Node::InterpolatedXStringNode { .. }));
}

/// Heredocs lower to string shapes with body content spans.
#[test]
fn heredoc_shapes() {
    let (node, _) = lowered("<<~EOS\ntext\nEOS");
    match node {
        Node::StringNode {
            content_loc,
            unescaped,
            ..
        } => {
            assert_eq!(unescaped, b"text\n");
            assert_eq!((content_loc.start, content_loc.end), (7, 12));
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("a = <<~EOS\n#{x}\nEOS");
    assert!(matches!(node, Node::LocalVariableWriteNode { .. }));
}

/// Dynamic symbols lower to interpolated symbols.
#[test]
fn dynamic_symbol() {
    let (node, _) = lowered(":\"a#{b}\"");
    assert!(matches!(node, Node::InterpolatedSymbolNode { .. }));
}

/// Bare calls keep `VARIABLE_CALL` and intern the name.
#[test]
fn bare_call_flag_and_name() {
    let (node, pool) = lowered("foo");
    match node {
        Node::CallNode {
            flags,
            name,
            arguments,
            ..
        } => {
            assert_eq!(flags, call_node_flags::VARIABLE_CALL);
            assert_eq!(sym_name(&pool, name), b"foo");
            assert!(arguments.is_none());
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
    // Parenthesized calls are not variable calls; empty args stay absent.
    let (node, _) = lowered("foo()");
    match node {
        Node::CallNode {
            flags,
            arguments,
            opening_loc,
            ..
        } => {
            assert_eq!(flags, 0);
            assert!(arguments.is_none());
            assert!(opening_loc.is_some());
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Setters keep the `=` on the name with `ATTRIBUTE_WRITE` plus `=` loc.
#[test]
fn setter_call() {
    let (node, pool) = lowered("foo.bar = 1");
    match node {
        Node::CallNode {
            flags,
            name,
            equal_loc,
            arguments,
            ..
        } => {
            assert_eq!(flags, call_node_flags::ATTRIBUTE_WRITE);
            assert_eq!(sym_name(&pool, name), b"bar=");
            assert!(equal_loc.is_some());
            assert!(arguments.is_some());
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// `&.` calls set `SAFE_NAVIGATION` with the `&.` loc.
#[test]
fn csend_call() {
    let (node, pool) = lowered("foo&.bar(1)");
    match node {
        Node::CallNode {
            flags,
            name,
            call_operator_loc,
            receiver,
            ..
        } => {
            assert_eq!(flags, call_node_flags::SAFE_NAVIGATION);
            assert_eq!(sym_name(&pool, name), b"bar");
            assert!(call_operator_loc.is_some());
            assert!(receiver.is_some());
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Index reads/writes become `[]` / `[]=` calls.
#[test]
fn index_calls() {
    let (node, pool) = lowered("foo[1]");
    match node {
        Node::CallNode { name, .. } => assert_eq!(sym_name(&pool, name), b"[]"),
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, pool) = lowered("foo[1] = 2");
    match node {
        Node::CallNode {
            name, equal_loc, ..
        } => {
            assert_eq!(sym_name(&pool, name), b"[]=");
            assert!(equal_loc.is_some());
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Keyword calls nest a flagged `KeywordHashNode` in the arguments.
#[test]
fn keyword_hash_call() {
    let (node, _) = lowered("foo(a: 1)");
    match node {
        Node::CallNode { arguments, .. } => {
            let args = arguments.expect("call args");
            match *args {
                Node::ArgumentsNode {
                    flags,
                    ref arguments,
                    ..
                } => {
                    assert_eq!(flags, arguments_node_flags::CONTAINS_KEYWORDS);
                    assert_eq!(arguments.len(), 1);
                    match arguments[0] {
                        Node::KeywordHashNode {
                            flags,
                            ref elements,
                            ..
                        } => {
                            assert_eq!(flags, keyword_hash_node_flags::SYMBOL_KEYS);
                            assert_eq!(elements.len(), 1);
                        }
                        ref other => panic!("arg lowered to {}", other.kind_name()),
                    }
                }
                ref other => panic!("args lowered to {}", other.kind_name()),
            }
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Non-symbol keyword keys clear `SYMBOL_KEYS`; splats set presence flags.
#[test]
fn keyword_hash_flags() {
    let (node, _) = lowered("foo(\"a\" => 1)");
    match node {
        Node::CallNode { arguments, .. } => match *arguments.expect("call args") {
            Node::ArgumentsNode { ref arguments, .. } => match arguments[0] {
                Node::KeywordHashNode { flags, .. } => assert_eq!(flags, 0),
                ref other => panic!("arg lowered to {}", other.kind_name()),
            },
            ref other => panic!("args lowered to {}", other.kind_name()),
        },
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("foo(*a, **k)");
    match node {
        Node::CallNode { arguments, .. } => match *arguments.expect("call args") {
            Node::ArgumentsNode { flags, .. } => assert_eq!(
                flags,
                arguments_node_flags::CONTAINS_KEYWORDS
                    | arguments_node_flags::CONTAINS_KEYWORD_SPLAT
                    | arguments_node_flags::CONTAINS_SPLAT
            ),
            ref other => panic!("args lowered to {}", other.kind_name()),
        },
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Arrays flag splats; hashes keep braces and pairs.
#[test]
fn array_and_hash() {
    let (node, _) = lowered("[1, *a]");
    match node {
        Node::ArrayNode { flags, .. } => assert_eq!(flags, array_node_flags::CONTAINS_SPLAT),
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("{ a: 1 }");
    match node {
        Node::HashNode {
            opening_loc,
            ref elements,
            ..
        } => {
            assert!(opening_loc.start == 0);
            assert_eq!(elements.len(), 1);
            assert!(matches!(elements[0], Node::AssocNode { .. }));
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Block pipes map to parameter slots; `;`-locals nest under the holder.
#[test]
fn block_params_and_shadow_locals() {
    let (node, pool) = lowered("foo { |a, *b, c| a }");
    match node {
        Node::CallNode { block, .. } => {
            let block = block.expect("block");
            match *block {
                Node::BlockNode { ref parameters, .. } => {
                    let params = parameters.as_deref().expect("block params");
                    match params {
                        Node::BlockParametersNode {
                            parameters,
                            ref locals,
                            ..
                        } => {
                            assert!(locals.is_empty());
                            match parameters.as_deref().expect("params") {
                                Node::ParametersNode {
                                    ref requireds,
                                    ref rest,
                                    ref posts,
                                    ..
                                } => {
                                    assert_eq!(requireds.len(), 1);
                                    assert!(rest.is_some());
                                    assert_eq!(posts.len(), 1);
                                }
                                other => panic!("params lowered to {}", other.kind_name()),
                            }
                        }
                        other => panic!("holder lowered to {}", other.kind_name()),
                    }
                }
                other => panic!("block lowered to {}", other.kind_name()),
            }
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
    let _ = pool;
}

/// Single `|x|` pipes splice (no `MultiTarget`); shadow args become locals.
#[test]
fn block_single_arg_splices() {
    let (node, pool) = lowered("foo { |a; b| a }");
    match node {
        Node::CallNode { block, .. } => match *block.expect("block") {
            Node::BlockNode { ref parameters, .. } => {
                match parameters.as_deref().expect("params") {
                    Node::BlockParametersNode {
                        parameters,
                        ref locals,
                        ..
                    } => {
                        assert_eq!(locals.len(), 1);
                        assert_eq!(
                            sym_name(
                                &pool,
                                match &locals[0] {
                                    Node::BlockLocalVariableNode { name, .. } => *name,
                                    other => panic!("local lowered to {}", other.kind_name()),
                                }
                            ),
                            b"b"
                        );
                        match parameters.as_deref().expect("inner params") {
                            Node::ParametersNode { ref requireds, .. } => {
                                assert_eq!(requireds.len(), 1);
                                assert!(matches!(requireds[0], Node::RequiredParameterNode { .. }));
                            }
                            other => panic!("params lowered to {}", other.kind_name()),
                        }
                    }
                    other => panic!("holder lowered to {}", other.kind_name()),
                }
            }
            other => panic!("block lowered to {}", other.kind_name()),
        },
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Destructured pipes nest a `MultiTargetNode` with required parts.
#[test]
fn block_destructured_arg() {
    let (node, _) = lowered("proc { |(a, *b)| a }");
    match node {
        Node::CallNode { block, .. } => match *block.expect("block") {
            Node::BlockNode { ref parameters, .. } => {
                match parameters.as_deref().expect("params") {
                    Node::BlockParametersNode { parameters, .. } => {
                        match parameters.as_deref().expect("inner") {
                            Node::ParametersNode { ref requireds, .. } => {
                                assert_eq!(requireds.len(), 1);
                                match requireds[0] {
                                    Node::MultiTargetNode {
                                        ref lefts,
                                        ref rest,
                                        ref rights,
                                        ..
                                    } => {
                                        // A rest item fills `rest`, never
                                        // `lefts` (Prism shape for `(a, *b)`).
                                        assert_eq!(lefts.len(), 1);
                                        assert!(matches!(
                                            lefts[0],
                                            Node::RequiredParameterNode { .. }
                                        ));
                                        assert!(matches!(
                                            rest.as_deref(),
                                            Some(Node::SplatNode { .. })
                                        ));
                                        assert!(rights.is_empty());
                                    }
                                    ref other => {
                                        panic!("required lowered to {}", other.kind_name())
                                    }
                                }
                            }
                            other => panic!("params lowered to {}", other.kind_name()),
                        }
                    }
                    other => panic!("holder lowered to {}", other.kind_name()),
                }
            }
            other => panic!("block lowered to {}", other.kind_name()),
        },
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Numbered blocks keep a blank `NumberedParametersNode` over reads.
#[test]
fn numblock_blank_maximum() {
    let (node, pool) = lowered("foo { _1 }");
    match node {
        Node::CallNode { block, .. } => match *block.expect("block") {
            Node::BlockNode {
                ref parameters,
                ref body,
                ..
            } => {
                match parameters.as_deref().expect("numbered params") {
                    Node::NumberedParametersNode { maximum, .. } => assert_eq!(*maximum, 0),
                    other => panic!("params lowered to {}", other.kind_name()),
                }
                match body.as_deref().expect("body") {
                    Node::StatementsNode { ref body, .. } => {
                        assert_eq!(body.len(), 1);
                        match body[0] {
                            Node::LocalVariableReadNode { name, depth, .. } => {
                                assert_eq!(sym_name(&pool, name), b"_1");
                                assert_eq!(depth, 0);
                            }
                            ref other => panic!("body lowered to {}", other.kind_name()),
                        }
                    }
                    other => panic!("body lowered to {}", other.kind_name()),
                }
            }
            other => panic!("block lowered to {}", other.kind_name()),
        },
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Lambdas carry operator and block-parameter holders.
#[test]
fn lambda_shape() {
    let (node, _) = lowered("->(x) { x }");
    match node {
        Node::LambdaNode {
            ref parameters,
            ref body,
            ref locals,
            ..
        } => {
            assert!(locals.is_empty());
            assert!(parameters.is_some());
            assert!(body.is_some());
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// `super() {}` attaches the block to the `SuperNode`.
#[test]
fn super_block() {
    let (node, _) = lowered("super() {}");
    assert!(matches!(node, Node::SuperNode { block: Some(_), .. }));
}

/// Regular and endless defs map keywords, receivers and ends.
#[test]
fn def_shapes() {
    let (node, pool) = lowered("def f(a); end");
    match node {
        Node::DefNode {
            name,
            ref receiver,
            ref parameters,
            ref body,
            ref locals,
            ref end_keyword_loc,
            ref equal_loc,
            ..
        } => {
            assert_eq!(sym_name(&pool, name), b"f");
            assert!(receiver.is_none());
            assert!(parameters.is_some());
            assert!(body.is_none());
            assert!(locals.is_empty());
            assert!(end_keyword_loc.is_some());
            assert!(equal_loc.is_none());
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, pool) = lowered("def self.f(a) = 1");
    match node {
        Node::DefNode {
            name,
            ref receiver,
            ref operator_loc,
            ref equal_loc,
            ref end_keyword_loc,
            ref body,
            ..
        } => {
            assert_eq!(sym_name(&pool, name), b"f");
            assert!(receiver.is_some());
            assert!(operator_loc.is_some());
            assert!(equal_loc.is_some());
            assert!(end_keyword_loc.is_none());
            // Endless bodies still wrap in statements.
            assert!(matches!(body.as_deref(), Some(Node::StatementsNode { .. })));
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Post-rest method params land in `posts` with paren locs on the def.
#[test]
fn def_post_params() {
    let (node, _) = lowered("def f(a, *b, c); end");
    match node {
        Node::DefNode {
            ref parameters,
            ref lparen_loc,
            ref rparen_loc,
            ..
        } => {
            assert!(lparen_loc.is_some());
            assert!(rparen_loc.is_some());
            match parameters.as_deref().expect("params") {
                Node::ParametersNode {
                    ref requireds,
                    ref rest,
                    ref posts,
                    ..
                } => {
                    assert_eq!(requireds.len(), 1);
                    assert!(rest.is_some());
                    assert_eq!(posts.len(), 1);
                }
                other => panic!("params lowered to {}", other.kind_name()),
            }
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Full method params cover optional, keyword, rest and block slots.
#[test]
fn def_full_params() {
    let (node, _) = lowered("def f(a, b = 1, *c, d, e:, f: 1, **g, &h); end");
    match node {
        Node::DefNode { ref parameters, .. } => match parameters.as_deref().expect("params") {
            Node::ParametersNode {
                ref requireds,
                ref optionals,
                ref rest,
                ref posts,
                ref keywords,
                ref keyword_rest,
                ref block,
                ..
            } => {
                assert_eq!(requireds.len(), 1);
                assert_eq!(optionals.len(), 1);
                assert!(rest.is_some());
                assert_eq!(posts.len(), 1);
                assert_eq!(keywords.len(), 2);
                assert!(keyword_rest.is_some());
                assert!(block.is_some());
            }
            other => panic!("params lowered to {}", other.kind_name()),
        },
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// `masgn` splits lefts, splat rest and rights with the `=` loc.
#[test]
fn masgn_split() {
    let (node, _) = lowered("a, *b, c = d");
    match node {
        Node::MultiWriteNode {
            ref lefts,
            ref rest,
            ref rights,
            ref operator_loc,
            ref value,
            ..
        } => {
            assert_eq!(lefts.len(), 1);
            assert!(rest.is_some());
            assert!(matches!(rest.as_deref(), Some(Node::SplatNode { .. })));
            assert_eq!(rights.len(), 1);
            assert_eq!((operator_loc.start, operator_loc.end), (9, 10));
            assert!(matches!(**value, Node::CallNode { .. }));
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Nested destructuring nests `MultiTargetNode`s.
#[test]
fn masgn_nested() {
    let (node, _) = lowered("a, (b, c) = d");
    match node {
        Node::MultiWriteNode { ref lefts, .. } => {
            assert_eq!(lefts.len(), 2);
            assert!(matches!(lefts[1], Node::MultiTargetNode { .. }));
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// `mlhs` items map to target nodes (index, call, constant path).
#[test]
fn masgn_targets() {
    let (node, pool) = lowered("a[0], self.foo, A::B = c");
    match node {
        Node::MultiWriteNode { ref lefts, .. } => {
            assert_eq!(lefts.len(), 3);
            assert!(matches!(lefts[0], Node::IndexTargetNode { .. }));
            match &lefts[1] {
                Node::CallTargetNode { name, .. } => assert_eq!(sym_name(&pool, *name), b"foo="),
                other => panic!("target lowered to {}", other.kind_name()),
            }
            assert!(matches!(lefts[2], Node::ConstantPathTargetNode { .. }));
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// `if/elsif/else` chains nest `IfNode`s under `ElseNode`s.
#[test]
fn if_elsif_else() {
    let (node, _) = lowered("if a then b elsif c then d else e end");
    match node {
        Node::IfNode {
            ref statements,
            ref subsequent,
            ref end_keyword_loc,
            ..
        } => {
            assert!(statements.is_some());
            assert!(end_keyword_loc.is_some());
            match subsequent.as_deref().expect("subsequent") {
                Node::IfNode { .. } => {}
                other => panic!("subsequent lowered to {}", other.kind_name()),
            }
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Present-but-empty `else` keeps an empty `ElseNode`.
#[test]
fn empty_else_kept() {
    let (node, _) = lowered("if a then b else end");
    match node {
        Node::IfNode { ref subsequent, .. } => match subsequent.as_deref().expect("else") {
            Node::ElseNode { ref statements, .. } => assert!(statements.is_none()),
            other => panic!("else lowered to {}", other.kind_name()),
        },
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("case x; when 1; a; else; end");
    match node {
        Node::CaseNode {
            ref else_clause, ..
        } => match else_clause.as_deref().expect("else") {
            Node::ElseNode { ref statements, .. } => assert!(statements.is_none()),
            other => panic!("else lowered to {}", other.kind_name()),
        },
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Modifier conditionals keep the keyword and drop statement locs.
#[test]
fn modifier_conditionals() {
    let (node, _) = lowered("a if b");
    match node {
        Node::IfNode {
            ref if_keyword_loc,
            ref statements,
            ref subsequent,
            ref end_keyword_loc,
            ..
        } => {
            assert!(if_keyword_loc.is_some());
            assert!(statements.is_some());
            assert!(subsequent.is_none());
            assert!(end_keyword_loc.is_none());
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("a unless b");
    assert!(matches!(
        node,
        Node::UnlessNode {
            statements: Some(_),
            else_clause: None,
            end_keyword_loc: None,
            ..
        }
    ));
}

/// Bare `unless` statements map to `UnlessNode` with swapped branches.
#[test]
fn unless_statement() {
    let (node, _) = lowered("unless a then b end");
    match node {
        Node::UnlessNode {
            ref statements,
            ref else_clause,
            ..
        } => {
            assert!(statements.is_some());
            assert!(else_clause.is_none());
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Ternaries keep `?`/`:` locs with an `ElseNode` subsequent.
#[test]
fn ternary_shape() {
    let (node, _) = lowered("a ? b : c");
    match node {
        Node::IfNode {
            ref if_keyword_loc,
            ref then_keyword_loc,
            ref subsequent,
            ..
        } => {
            assert!(if_keyword_loc.is_none());
            assert!(then_keyword_loc.is_some());
            assert!(matches!(subsequent.as_deref(), Some(Node::ElseNode { .. })));
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Post-condition loops set `BEGIN_MODIFIER`; `for` maps index targets.
#[test]
fn loops_and_for() {
    let (node, _) = lowered("begin; a; end while b");
    match node {
        Node::WhileNode { flags, .. } => assert_eq!(flags, loop_flags::BEGIN_MODIFIER),
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("while b; a; end");
    match node {
        Node::WhileNode {
            flags,
            ref closing_loc,
            ..
        } => {
            assert_eq!(flags, 0);
            assert!(closing_loc.is_some());
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("for a, b in x; end");
    match node {
        Node::ForNode { ref index, .. } => assert!(matches!(**index, Node::MultiTargetNode { .. })),
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("for foo.bar in x; end");
    match node {
        Node::ForNode { ref index, .. } => assert!(matches!(**index, Node::CallTargetNode { .. })),
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Full `begin/rescue/else/ensure` splits into clause nodes.
#[test]
fn kwbegin_clauses() {
    let (node, pool) = lowered("begin; a; rescue E => e; b; else; c; ensure; d; end");
    match node {
        Node::BeginNode {
            ref statements,
            ref rescue_clause,
            ref else_clause,
            ref ensure_clause,
            ref begin_keyword_loc,
            ref end_keyword_loc,
            ..
        } => {
            assert!(begin_keyword_loc.is_some());
            assert!(end_keyword_loc.is_some());
            assert!(statements.is_some());
            match rescue_clause.as_deref().expect("rescue") {
                Node::RescueNode {
                    ref exceptions,
                    ref reference,
                    ..
                } => {
                    assert_eq!(exceptions.len(), 1);
                    assert!(matches!(exceptions[0], Node::ConstantReadNode { .. }));
                    match reference.as_deref().expect("reference") {
                        Node::LocalVariableTargetNode { name, depth, .. } => {
                            assert_eq!(sym_name(&pool, *name), b"e");
                            assert_eq!(*depth, 0);
                        }
                        other => panic!("reference lowered to {}", other.kind_name()),
                    }
                }
                other => panic!("rescue lowered to {}", other.kind_name()),
            }
            assert!(matches!(
                else_clause.as_deref(),
                Some(Node::ElseNode { .. })
            ));
            assert!(matches!(
                ensure_clause.as_deref(),
                Some(Node::EnsureNode { .. })
            ));
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Modifier rescue maps to `RescueModifierNode`.
#[test]
fn rescue_modifier() {
    let (node, _) = lowered("a rescue b");
    match node {
        Node::RescueModifierNode {
            ref expression,
            ref rescue_expression,
            ref keyword_loc,
            ..
        } => {
            assert!(matches!(**expression, Node::CallNode { .. }));
            assert!(matches!(**rescue_expression, Node::CallNode { .. }));
            assert_eq!((keyword_loc.start, keyword_loc.end), (2, 8));
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Method-level rescue wraps in a `BeginNode` like Prism def bodies.
#[test]
fn def_rescue_body() {
    let (node, _) = lowered("def f; a; rescue; b; else; c; end");
    match node {
        Node::DefNode { ref body, .. } => match body.as_deref().expect("body") {
            Node::StatementsNode { ref body, .. } => match body.first() {
                Some(Node::BeginNode {
                    ref rescue_clause,
                    ref else_clause,
                    ..
                }) => {
                    assert!(rescue_clause.is_some());
                    assert!(else_clause.is_some());
                }
                other => panic!("def body lowered to {other:?}"),
            },
            other => panic!("def body lowered to {}", other.kind_name()),
        },
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Pattern matching: predicates, guards, pins and pattern nodes.
#[test]
fn pattern_matching() {
    let (node, _) = lowered("x => { a: }");
    assert!(matches!(node, Node::MatchRequiredNode { .. }));
    let (node, _) = lowered("x in { a: }");
    assert!(matches!(node, Node::MatchPredicateNode { .. }));
    let (node, _) = lowered("case x; in [1, *r, 2]; end");
    match node {
        Node::CaseMatchNode { ref conditions, .. } => {
            assert_eq!(conditions.len(), 1);
            match conditions[0] {
                Node::InNode { ref pattern, .. } => match **pattern {
                    Node::ArrayPatternNode {
                        ref requireds,
                        ref rest,
                        ref posts,
                        ..
                    } => {
                        assert_eq!(requireds.len(), 1);
                        assert!(rest.is_some());
                        assert_eq!(posts.len(), 1);
                    }
                    ref other => panic!("pattern lowered to {}", other.kind_name()),
                },
                ref other => panic!("condition lowered to {}", other.kind_name()),
            }
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("case x; in a if b; c; end");
    match node {
        Node::CaseMatchNode { ref conditions, .. } => match conditions[0] {
            Node::InNode { ref pattern, .. } => {
                assert!(matches!(**pattern, Node::IfNode { .. }))
            }
            ref other => panic!("condition lowered to {}", other.kind_name()),
        },
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("case x; in ^(a); end");
    match node {
        Node::CaseMatchNode { ref conditions, .. } => match conditions[0] {
            Node::InNode { ref pattern, .. } => {
                assert!(matches!(**pattern, Node::PinnedExpressionNode { .. }))
            }
            ref other => panic!("condition lowered to {}", other.kind_name()),
        },
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("case x; in A(v); end");
    match node {
        Node::CaseMatchNode { ref conditions, .. } => match conditions[0] {
            Node::InNode { ref pattern, .. } => match **pattern {
                Node::ArrayPatternNode { ref constant, .. } => assert!(constant.is_some()),
                ref other => panic!("pattern lowered to {}", other.kind_name()),
            },
            ref other => panic!("condition lowered to {}", other.kind_name()),
        },
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("case x; in [a] => y; end");
    match node {
        Node::CaseMatchNode { ref conditions, .. } => match conditions[0] {
            Node::InNode { ref pattern, .. } => {
                assert!(matches!(**pattern, Node::CapturePatternNode { .. }))
            }
            ref other => panic!("condition lowered to {}", other.kind_name()),
        },
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("case x; in 1 | 2; end");
    match node {
        Node::CaseMatchNode { ref conditions, .. } => match conditions[0] {
            Node::InNode { ref pattern, .. } => {
                assert!(matches!(**pattern, Node::AlternationPatternNode { .. }))
            }
            ref other => panic!("condition lowered to {}", other.kind_name()),
        },
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("case x; in [*, a, *]; end");
    match node {
        Node::CaseMatchNode { ref conditions, .. } => match conditions[0] {
            Node::InNode { ref pattern, .. } => {
                assert!(matches!(**pattern, Node::FindPatternNode { .. }))
            }
            ref other => panic!("condition lowered to {}", other.kind_name()),
        },
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Named-capture matches synthesize a `=~` call with empty targets.
///
/// The pinned parser never emits this node without the `onig` feature
/// (its `match_op` falls back to `Send`), so the test builds it by hand.
#[test]
fn match_write_shape() {
    let re = parse("/(?<w>q)/");
    let value = parse("y");
    let root = lib_ruby_parser::Node::MatchWithLvasgn(lib_ruby_parser::nodes::MatchWithLvasgn {
        re: Box::new(*re),
        value: Box::new(*value),
        operator_l: lib_ruby_parser::Loc { begin: 9, end: 11 },
        expression_l: lib_ruby_parser::Loc { begin: 0, end: 14 },
    });
    let (node, pool) = lower(&root);
    match node {
        Node::MatchWriteNode {
            ref call,
            ref targets,
            ..
        } => {
            assert!(targets.is_empty());
            match **call {
                Node::CallNode { name, .. } => assert_eq!(sym_name(&pool, name), b"=~"),
                ref other => panic!("call lowered to {}", other.kind_name()),
            }
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// `if /re/` conditions become match-last-line nodes.
#[test]
fn match_current_line() {
    let (node, _) = lowered("if /re/; a; end");
    match node {
        Node::IfNode { ref predicate, .. } => match **predicate {
            Node::MatchLastLineNode { ref unescaped, .. } => assert_eq!(unescaped, b"re"),
            ref other => panic!("predicate lowered to {}", other.kind_name()),
        },
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Reads and writes map to scoped nodes with blank depths.
#[test]
fn variables_blank_depth() {
    let (node, pool) = lowered("@a = 1; @a");
    match node {
        Node::StatementsNode { ref body, .. } => {
            assert_eq!(body.len(), 2);
            match body[0] {
                Node::InstanceVariableWriteNode { name, .. } => {
                    assert_eq!(sym_name(&pool, name), b"@a")
                }
                ref other => panic!("write lowered to {}", other.kind_name()),
            }
            match body[1] {
                Node::InstanceVariableReadNode { name, .. } => {
                    assert_eq!(sym_name(&pool, name), b"@a")
                }
                ref other => panic!("read lowered to {}", other.kind_name()),
            }
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, pool) = lowered("$g = 1; @@a = 1; A = 1");
    match node {
        Node::StatementsNode { ref body, .. } => {
            assert_eq!(body.len(), 3);
            match body[0] {
                Node::GlobalVariableWriteNode { name, .. } => {
                    assert_eq!(sym_name(&pool, name), b"$g")
                }
                ref other => panic!("gvar lowered to {}", other.kind_name()),
            }
            match body[1] {
                Node::ClassVariableWriteNode { name, .. } => {
                    assert_eq!(sym_name(&pool, name), b"@@a")
                }
                ref other => panic!("cvar lowered to {}", other.kind_name()),
            }
            match body[2] {
                Node::ConstantWriteNode { name, .. } => assert_eq!(sym_name(&pool, name), b"A"),
                ref other => panic!("const lowered to {}", other.kind_name()),
            }
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Constant paths nest reads; leading `::` drops the parent.
#[test]
fn constant_paths() {
    let (node, pool) = lowered("A::B");
    match node {
        Node::ConstantPathNode {
            ref parent, name, ..
        } => {
            assert_eq!(sym_name(&pool, name.expect("path name")), b"B");
            assert!(matches!(
                parent.as_deref(),
                Some(Node::ConstantReadNode { .. })
            ));
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, pool) = lowered("::A");
    match node {
        Node::ConstantPathNode {
            ref parent, name, ..
        } => {
            assert_eq!(sym_name(&pool, name.expect("path name")), b"A");
            assert!(parent.is_none());
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Back and numbered references keep their source names.
#[test]
fn backrefs() {
    let (node, pool) = lowered("$&");
    match node {
        Node::BackReferenceReadNode { name, .. } => assert_eq!(sym_name(&pool, name), b"$&"),
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("$1");
    match node {
        Node::NumberedReferenceReadNode { number, .. } => assert_eq!(number, 1),
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// All `op=` receiver families map to operator-write nodes.
#[test]
fn op_assign_families() {
    for (src, kind) in [
        ("a += 1", "LocalVariableOperatorWriteNode"),
        ("@a += 1", "InstanceVariableOperatorWriteNode"),
        ("@@a += 1", "ClassVariableOperatorWriteNode"),
        ("$g += 1", "GlobalVariableOperatorWriteNode"),
        ("A += 1", "ConstantOperatorWriteNode"),
        ("A::B += 1", "ConstantPathOperatorWriteNode"),
        ("foo.bar += 1", "CallOperatorWriteNode"),
        ("a[1] += 1", "IndexOperatorWriteNode"),
    ] {
        let (node, _) = lowered(src);
        assert_eq!(node.kind_name(), kind, "opassign {src:?}");
    }
    for (src, kind) in [
        ("a &&= 1", "LocalVariableAndWriteNode"),
        ("a ||= 1", "LocalVariableOrWriteNode"),
        ("@a &&= 1", "InstanceVariableAndWriteNode"),
        ("@a ||= 1", "InstanceVariableOrWriteNode"),
        ("A &&= 1", "ConstantAndWriteNode"),
        ("A::B ||= 1", "ConstantPathOrWriteNode"),
        ("foo.bar &&= 1", "CallAndWriteNode"),
        ("foo.bar ||= 1", "CallOrWriteNode"),
        ("a[1] &&= 1", "IndexAndWriteNode"),
        ("a[1] ||= 1", "IndexOrWriteNode"),
    ] {
        let (node, _) = lowered(src);
        assert_eq!(node.kind_name(), kind, "logic assign {src:?}");
    }
}

/// `super`, `yield`, exits and `defined?` keep keyword locs and arg forms.
#[test]
fn keyword_calls() {
    let (node, _) = lowered("super");
    assert!(matches!(node, Node::ForwardingSuperNode { .. }));
    let (node, _) = lowered("super(1)");
    match node {
        Node::SuperNode { ref arguments, .. } => assert!(arguments.is_some()),
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("yield");
    match node {
        Node::YieldNode { ref arguments, .. } => assert!(arguments.is_none()),
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("break");
    match node {
        Node::BreakNode { ref arguments, .. } => assert!(arguments.is_none()),
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("return 1");
    match node {
        Node::ReturnNode { ref arguments, .. } => assert!(arguments.is_some()),
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("defined?(a)");
    match node {
        Node::DefinedNode {
            ref lparen_loc,
            ref rparen_loc,
            ..
        } => {
            assert!(lparen_loc.is_some());
            assert!(rparen_loc.is_some());
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Aliases split by target kind; `undef` keeps names and the keyword.
#[test]
fn alias_undef() {
    let (node, _) = lowered("alias a b");
    assert!(matches!(node, Node::AliasMethodNode { .. }));
    let (node, _) = lowered("alias $a $b");
    assert!(matches!(node, Node::AliasGlobalVariableNode { .. }));
    let (node, _) = lowered("undef foo, bar");
    match node {
        Node::UndefNode {
            ref names,
            ref keyword_loc,
            ..
        } => {
            assert_eq!(names.len(), 2);
            assert_eq!((keyword_loc.start, keyword_loc.end), (0, 5));
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// `BEGIN`/`END` blocks map to pre/post execution nodes.
#[test]
fn pre_post_execution() {
    let (node, _) = lowered("BEGIN { a }");
    assert!(matches!(node, Node::PreExecutionNode { .. }));
    let (node, _) = lowered("END { a }");
    assert!(matches!(node, Node::PostExecutionNode { .. }));
}

/// Classes, modules and singletons keep paths, names and ends.
#[test]
fn class_module_sclass() {
    let (node, pool) = lowered("class A < B; end");
    match node {
        Node::ClassNode {
            name,
            ref superclass,
            ref inheritance_operator_loc,
            ..
        } => {
            assert_eq!(sym_name(&pool, name), b"A");
            assert!(superclass.is_some());
            assert!(inheritance_operator_loc.is_some());
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, pool) = lowered("module A; end");
    match node {
        Node::ModuleNode { name, .. } => assert_eq!(sym_name(&pool, name), b"A"),
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("class << self; end");
    assert!(matches!(node, Node::SingletonClassNode { .. }));
}

/// Ranges flag `...`; flip-flops share one node kind.
#[test]
fn ranges_and_flipflops() {
    let (node, _) = lowered("a...b");
    match node {
        Node::RangeNode { flags, .. } => assert_eq!(flags, range_flags::EXCLUDE_END),
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("a..b");
    match node {
        Node::RangeNode { flags, .. } => assert_eq!(flags, 0),
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("if a..b then c end");
    match node {
        Node::IfNode { ref predicate, .. } => {
            assert!(matches!(**predicate, Node::FlipFlopNode { .. }))
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("if a...b then c end");
    match node {
        Node::IfNode { ref predicate, .. } => {
            assert!(matches!(**predicate, Node::FlipFlopNode { .. }))
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Logic operators map to `And`/`Or` nodes.
#[test]
fn logic_nodes() {
    let (node, _) = lowered("a && b");
    assert!(matches!(node, Node::AndNode { .. }));
    let (node, _) = lowered("a or b");
    assert!(matches!(node, Node::OrNode { .. }));
}

/// Parenthesized groups stay `ParenthesesNode`s; bare groups spread.
#[test]
fn begin_shapes() {
    let (node, _) = lowered("(1)");
    match node {
        Node::ParenthesesNode {
            ref body,
            ref opening_loc,
            ..
        } => {
            assert!(body.is_some());
            assert_eq!((opening_loc.start, opening_loc.end), (0, 1));
        }
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("1; 2");
    match node {
        Node::StatementsNode { ref body, .. } => assert_eq!(body.len(), 2),
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// None-vs-empty: absent lists stay `None` rather than empty nodes.
#[test]
fn none_vs_empty() {
    let (node, _) = lowered("foo()");
    assert!(matches!(
        node,
        Node::CallNode {
            arguments: None,
            block: None,
            ..
        }
    ));
    let (node, _) = lowered("foo {}");
    match node {
        Node::CallNode { ref block, .. } => match block.as_deref().expect("block") {
            Node::BlockNode {
                parameters: None,
                body: None,
                ..
            } => {}
            other => panic!("block lowered to {}", other.kind_name()),
        },
        other => panic!("lowered to {}", other.kind_name()),
    }
    let (node, _) = lowered("def f; end");
    assert!(matches!(
        node,
        Node::DefNode {
            parameters: None,
            body: None,
            ..
        }
    ));
    let (node, _) = lowered("if a then b end");
    assert!(matches!(
        node,
        Node::IfNode {
            subsequent: None,
            end_keyword_loc: Some(_),
            ..
        }
    ));
    let (node, _) = lowered("while b; a; end");
    match node {
        Node::WhileNode { ref statements, .. } => assert!(matches!(
            statements.as_deref(),
            Some(Node::StatementsNode { .. })
        )),
        other => panic!("lowered to {}", other.kind_name()),
    }
}

/// Magic comments nodes map to source nodes (`__FILE__` has no path here).
#[test]
fn magic_nodes() {
    let (node, _) = lowered("__FILE__");
    assert!(matches!(node, Node::SourceFileNode { .. }));
    let (node, _) = lowered("__LINE__");
    assert!(matches!(node, Node::SourceLineNode { .. }));
    let (node, _) = lowered("__ENCODING__");
    assert!(matches!(node, Node::SourceEncodingNode { .. }));
}

/// Literals map to leaf nodes with full spans.
#[test]
fn leaf_nodes() {
    for (src, kind) in [
        ("nil", "NilNode"),
        ("true", "TrueNode"),
        ("false", "FalseNode"),
        ("self", "SelfNode"),
        ("redo", "RedoNode"),
        ("retry", "RetryNode"),
    ] {
        let (node, _) = lowered(src);
        assert_eq!(node.kind_name(), kind, "leaf {src:?}");
    }
}
