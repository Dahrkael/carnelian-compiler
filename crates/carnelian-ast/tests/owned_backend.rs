//! Owned backend smoke tests (hand-built trees, no lowering, no codegen).

use carnelian_ast::view::{BackendNode, IntegerLit};
use carnelian_ast::AstNode;
use carnelian_ast::{
    arguments_node_flags, call_node_flags, loop_flags, Integer, Node, Owned, Span, SymbolId,
    SymbolPool,
};

fn span() -> Span {
    Span { start: 0, end: 1 }
}

fn pool_with(names: &[&[u8]]) -> (SymbolPool, Vec<SymbolId>) {
    let mut pool = SymbolPool::new();
    let mut ids = Vec::new();
    for name in names {
        ids.push(pool.intern(name));
    }
    (pool, ids)
}

fn int(value: i64) -> Node {
    Node::IntegerNode {
        flags: 0,
        span: span(),
        value: Integer::I64(value),
    }
}

fn fallback(raw: &[u8]) -> Node {
    Node::IntegerNode {
        flags: 0,
        span: span(),
        value: Integer::Fallback { raw: raw.to_vec() },
    }
}

fn stmts(body: Vec<Node>) -> Node {
    Node::StatementsNode {
        flags: 0,
        span: span(),
        body,
    }
}

fn lvar_read(id: SymbolId) -> Node {
    Node::LocalVariableReadNode {
        flags: 0,
        span: span(),
        name: id,
        depth: 0,
    }
}

fn owned<'a>(node: &'a Node, pool: &'a SymbolPool) -> Owned<'a> {
    Owned { node, pool }
}

#[test]
fn program_exposes_locals_and_body() {
    let (pool, ids) = pool_with(&[b"x"]);
    let program = Node::ProgramNode {
        flags: 0,
        span: span(),
        locals: vec![ids[0]],
        statements: Box::new(stmts(vec![int(1)])),
    };
    let root = owned(&program, &pool);
    assert_eq!(root.kind_name(), "ProgramNode");
    assert_eq!(root.flags(), 0);
    assert_eq!(root.span(), span());
    let view = root.program().expect("program");
    assert_eq!(view.locals, vec![b"x".to_vec()]);
    let body = owned(view.body.node, &pool);
    let list = body.statements().expect("statements");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].kind_name(), "IntegerNode");
    // Wrong kinds answer None.
    assert!(list[0].program().is_none());
    assert!(root.integer_lit().is_none());
}

#[test]
fn literal_accessors_cover_scalar_forms() {
    let (pool, _) = pool_with(&[]);
    let float = Node::FloatNode {
        flags: 0,
        span: span(),
        value: 1.5,
    };
    assert_eq!(owned(&float, &pool).float_lit(), Some(1.5));
    let string = Node::StringNode {
        flags: 0,
        span: span(),
        opening_loc: None,
        content_loc: span(),
        closing_loc: None,
        unescaped: b"hi".to_vec(),
    };
    assert_eq!(owned(&string, &pool).string_lit(), Some(b"hi".to_vec()));
    let symbol = Node::SymbolNode {
        flags: 0,
        span: span(),
        opening_loc: None,
        value_loc: None,
        closing_loc: None,
        unescaped: b"sym".to_vec(),
    };
    assert_eq!(owned(&symbol, &pool).symbol_lit(), Some(b"sym".to_vec()));
    let truth = Node::TrueNode {
        flags: 0,
        span: span(),
    };
    assert!(matches!(
        owned(&truth, &pool).simple_lit(),
        Some(carnelian_ast::view::SimpleLit::True)
    ));
    let falsum = Node::FalseNode {
        flags: 0,
        span: span(),
    };
    assert!(owned(&falsum, &pool).simple_lit().is_some());
    let nil = Node::NilNode {
        flags: 0,
        span: span(),
    };
    assert!(owned(&nil, &pool).simple_lit().is_some());
    let slf = Node::SelfNode {
        flags: 0,
        span: span(),
    };
    assert!(owned(&slf, &pool).simple_lit().is_some());
    assert!(owned(&slf, &pool).float_lit().is_none());
    let small = int(7);
    assert_eq!(owned(&small, &pool).integer_lit(), Some(IntegerLit::I64(7)));
}

#[test]
fn bigint_fallback_normalizes_like_ffi() {
    let (pool, _) = pool_with(&[]);
    // 2^127 overflows i64: canonical digits, positive.
    let big = fallback(b"170141183460469231731687303715884105728");
    assert_eq!(
        owned(&big, &pool).integer_lit(),
        Some(IntegerLit::Bigint {
            digits: b"170141183460469231731687303715884105728".to_vec(),
            negative: false,
        })
    );
    // Signed, underscored and zero-padded input normalizes the same way.
    let noisy = fallback(b"-000_170141183460469231731687303715884105728");
    assert_eq!(
        owned(&noisy, &pool).integer_lit(),
        Some(IntegerLit::Bigint {
            digits: b"170141183460469231731687303715884105728".to_vec(),
            negative: true,
        })
    );
    // Small values stay I64 even in fallback text; zero is unsigned.
    assert_eq!(
        owned(&fallback(b"1_2_3"), &pool).integer_lit(),
        Some(IntegerLit::I64(123))
    );
    assert_eq!(
        owned(&fallback(b"-0"), &pool).integer_lit(),
        Some(IntegerLit::I64(0))
    );
    // Non-decimal bytes fail closed.
    assert!(owned(&fallback(b"12x3"), &pool).integer_lit().is_none());
    // FFI cross-checks live in `front-prism/tests/lower_roundtrip.rs`,
    // where both implementations are linked; this crate stays FFI-free.
}

#[test]
fn call_view_covers_receiver_args_block_and_flags() {
    let (mut pool, _) = pool_with(&[]);
    let name = pool.intern(b"foo");
    let recv = pool.intern(b"recv");
    let _ = (name, recv);
    let (pool, ids) = pool_with(&[b"foo", b"recv"]);
    let call = Node::CallNode {
        flags: call_node_flags::SAFE_NAVIGATION | call_node_flags::ATTRIBUTE_WRITE,
        span: span(),
        receiver: Some(Box::new(lvar_read(ids[1]))),
        call_operator_loc: None,
        name: ids[0],
        message_loc: None,
        opening_loc: None,
        arguments: Some(Box::new(Node::ArgumentsNode {
            flags: 0,
            span: span(),
            arguments: vec![int(1)],
        })),
        closing_loc: None,
        equal_loc: None,
        block: Some(Box::new(Node::BlockArgumentNode {
            flags: 0,
            span: span(),
            expression: None,
            operator_loc: span(),
        })),
    };
    let view = owned(&call, &pool).call().expect("call");
    assert_eq!(view.name, b"foo");
    assert_eq!(
        view.receiver.expect("recv").kind_name(),
        "LocalVariableReadNode"
    );
    assert_eq!(view.args.expect("args").kind_name(), "ArgumentsNode");
    assert_eq!(view.block.expect("block").kind_name(), "BlockArgumentNode");
    assert!(view.safe_nav);
    assert!(view.attr_write);
    // Plain call has no flags, receiver, args or block.
    let plain = Node::CallNode {
        flags: 0,
        span: span(),
        receiver: None,
        call_operator_loc: None,
        name: ids[0],
        message_loc: None,
        opening_loc: None,
        arguments: None,
        closing_loc: None,
        equal_loc: None,
        block: None,
    };
    let view = owned(&plain, &pool).call().expect("plain call");
    assert!(!view.safe_nav && !view.attr_write);
    assert!(view.receiver.is_none() && view.args.is_none() && view.block.is_none());
    // Missing pool entry fails closed.
    let broken = Node::CallNode {
        flags: 0,
        span: span(),
        receiver: None,
        call_operator_loc: None,
        name: SymbolId(999),
        message_loc: None,
        opening_loc: None,
        arguments: None,
        closing_loc: None,
        equal_loc: None,
        block: None,
    };
    assert!(owned(&broken, &pool).call().is_none());
}

#[test]
fn call_args_array_and_forwarding_forms() {
    let (pool, _) = pool_with(&[]);
    let args = Node::ArgumentsNode {
        flags: 0,
        span: span(),
        arguments: vec![
            int(1),
            Node::SplatNode {
                flags: 0,
                span: span(),
                operator_loc: span(),
                expression: Some(Box::new(int(2))),
            },
            Node::KeywordHashNode {
                flags: 0,
                span: span(),
                elements: vec![],
            },
        ],
    };
    let items = owned(&args, &pool).call_args().expect("args");
    assert_eq!(items.len(), 3);
    assert_eq!(items[1].kind_name(), "SplatNode");
    assert_eq!(owned(&args, &pool).raw_call_args().expect("raw").len(), 3);
    assert!(!owned(&args, &pool).args_forwarding());
    let fwd = Node::ArgumentsNode {
        flags: arguments_node_flags::CONTAINS_FORWARDING,
        span: span(),
        arguments: vec![],
    };
    assert!(owned(&fwd, &pool).args_forwarding());
    assert!(!owned(&int(1), &pool).args_forwarding());

    let array = Node::ArrayNode {
        flags: 0,
        span: span(),
        elements: vec![int(1), int(2)],
        opening_loc: None,
        closing_loc: None,
    };
    assert_eq!(
        owned(&array, &pool).array_elements().expect("elts").len(),
        2
    );
    assert_eq!(
        owned(&array, &pool)
            .raw_array_elements()
            .expect("raw")
            .len(),
        2
    );
    assert!(owned(&int(1), &pool).array_elements().is_none());
}

#[test]
fn if_unless_branches_cover_else_forms() {
    let (pool, _) = pool_with(&[]);
    let cond = int(1);
    let _ = &cond;
    let branch = Node::IfNode {
        flags: 0,
        span: span(),
        if_keyword_loc: None,
        predicate: Box::new(int(1)),
        then_keyword_loc: None,
        statements: Some(Box::new(stmts(vec![int(2)]))),
        subsequent: Some(Box::new(Node::ElseNode {
            flags: 0,
            span: span(),
            else_keyword_loc: span(),
            statements: Some(Box::new(stmts(vec![int(3)]))),
            end_keyword_loc: None,
        })),
        end_keyword_loc: None,
    };
    let view = owned(&branch, &pool).if_branch().expect("if");
    assert!(!view.is_unless);
    assert_eq!(view.predicate.expect("pred").kind_name(), "IntegerNode");
    assert_eq!(view.then_body.expect("then").len(), 1);
    let els = view.else_body.expect("else");
    assert_eq!(els.kind_name(), "ElseNode");
    assert_eq!(owned(els.node, &pool).else_body().expect("body").len(), 1);

    // `unless` swaps the else clause into the then body.
    let unless = Node::UnlessNode {
        flags: 0,
        span: span(),
        keyword_loc: span(),
        predicate: Box::new(int(1)),
        then_keyword_loc: None,
        statements: Some(Box::new(stmts(vec![int(2)]))),
        else_clause: Some(Box::new(Node::ElseNode {
            flags: 0,
            span: span(),
            else_keyword_loc: span(),
            statements: None,
            end_keyword_loc: None,
        })),
        end_keyword_loc: None,
    };
    let view = owned(&unless, &pool).if_branch().expect("unless");
    assert!(view.is_unless);
    assert_eq!(view.then_body.expect("then").len(), 0);
    assert_eq!(view.else_body.expect("else").kind_name(), "StatementsNode");

    // Bare `if` has null branches.
    let bare = Node::IfNode {
        flags: 0,
        span: span(),
        if_keyword_loc: None,
        predicate: Box::new(int(1)),
        then_keyword_loc: None,
        statements: None,
        subsequent: None,
        end_keyword_loc: None,
    };
    let view = owned(&bare, &pool).if_branch().expect("bare if");
    assert!(view.then_body.is_none() && view.else_body.is_none());
}

#[test]
fn while_until_cover_modifier_bit() {
    let (pool, _) = pool_with(&[]);
    let begun = Node::WhileNode {
        flags: loop_flags::BEGIN_MODIFIER,
        span: span(),
        keyword_loc: span(),
        do_keyword_loc: None,
        closing_loc: None,
        predicate: Box::new(int(1)),
        statements: Some(Box::new(stmts(vec![int(2)]))),
    };
    let view = owned(&begun, &pool).while_loop().expect("while");
    assert!(!view.is_until && view.begin_modifier);
    assert_eq!(view.body.expect("body").len(), 1);
    let until = Node::UntilNode {
        flags: 0,
        span: span(),
        keyword_loc: span(),
        do_keyword_loc: None,
        closing_loc: None,
        predicate: Box::new(int(1)),
        statements: None,
    };
    let view = owned(&until, &pool).while_loop().expect("until");
    assert!(view.is_until && !view.begin_modifier);
    assert!(view.body.is_none());
}

#[test]
fn lvar_accessors_cover_read_write_target() {
    let (pool, ids) = pool_with(&[b"x"]);
    let read = lvar_read(ids[0]);
    let view = owned(&read, &pool).lvar_read().expect("read");
    assert_eq!(view.name, b"x");
    assert_eq!(view.depth, 0);
    let write = Node::LocalVariableWriteNode {
        flags: 0,
        span: span(),
        name: ids[0],
        depth: 0,
        name_loc: span(),
        value: Box::new(int(1)),
        operator_loc: span(),
    };
    let view = owned(&write, &pool).lvar_write().expect("write");
    assert_eq!(view.name, b"x");
    assert_eq!(view.value.expect("value").kind_name(), "IntegerNode");
    let target = Node::LocalVariableTargetNode {
        flags: 0,
        span: span(),
        name: ids[0],
        depth: 0,
    };
    assert_eq!(
        owned(&target, &pool).lvar_target().expect("target").name,
        b"x"
    );
    // Missing pool entry fails closed.
    assert!(owned(&lvar_read(SymbolId(999)), &pool)
        .lvar_read()
        .is_none());
}

#[test]
fn logic_hash_assoc_and_splats() {
    let (pool, _) = pool_with(&[]);
    let and = Node::AndNode {
        flags: 0,
        span: span(),
        left: Box::new(int(1)),
        right: Box::new(int(2)),
        operator_loc: span(),
    };
    let (left, right) = owned(&and, &pool).logic().expect("and");
    assert_eq!(left.kind_name(), "IntegerNode");
    assert_eq!(right.kind_name(), "IntegerNode");
    let or = Node::OrNode {
        flags: 0,
        span: span(),
        left: Box::new(int(1)),
        right: Box::new(int(2)),
        operator_loc: span(),
    };
    assert!(owned(&or, &pool).logic().is_some());

    let hash = Node::HashNode {
        flags: 0,
        span: span(),
        opening_loc: span(),
        elements: vec![
            Node::AssocNode {
                flags: 0,
                span: span(),
                key: Box::new(int(1)),
                value: Box::new(int(2)),
                operator_loc: None,
            },
            Node::AssocSplatNode {
                flags: 0,
                span: span(),
                value: Some(Box::new(int(3))),
                operator_loc: span(),
            },
        ],
        closing_loc: span(),
    };
    let elements = owned(&hash, &pool).hash_elements().expect("hash");
    assert_eq!(elements.len(), 2);
    let (key, value) = elements[0].assoc_pair().expect("pair");
    assert_eq!(key.kind_name(), "IntegerNode");
    assert_eq!(value.kind_name(), "IntegerNode");
    assert!(elements[0].assoc_splat_value().is_none());
    let inner = elements[1].assoc_splat_value().expect("splat");
    assert_eq!(inner.expect("value").kind_name(), "IntegerNode");
    let bare_splat = Node::AssocSplatNode {
        flags: 0,
        span: span(),
        value: None,
        operator_loc: span(),
    };
    assert!(owned(&bare_splat, &pool)
        .assoc_splat_value()
        .expect("bare")
        .is_none());

    let khash = Node::KeywordHashNode {
        flags: 0,
        span: span(),
        elements: vec![],
    };
    assert_eq!(
        owned(&khash, &pool).hash_elements().expect("khash").len(),
        0
    );

    let splat = Node::SplatNode {
        flags: 0,
        span: span(),
        operator_loc: span(),
        expression: Some(Box::new(int(1))),
    };
    assert!(owned(&splat, &pool).splat_value().expect("splat").is_some());
    let bare = Node::SplatNode {
        flags: 0,
        span: span(),
        operator_loc: span(),
        expression: None,
    };
    assert!(owned(&bare, &pool).splat_value().expect("bare").is_none());
}

#[test]
fn case_when_distinguish_null_and_empty() {
    let (pool, _) = pool_with(&[]);
    let case = Node::CaseNode {
        flags: 0,
        span: span(),
        predicate: Some(Box::new(int(1))),
        conditions: vec![Node::WhenNode {
            flags: 0,
            span: span(),
            keyword_loc: span(),
            conditions: vec![int(1)],
            then_keyword_loc: None,
            statements: None,
        }],
        else_clause: Some(Box::new(Node::ElseNode {
            flags: 0,
            span: span(),
            else_keyword_loc: span(),
            statements: Some(Box::new(stmts(vec![]))),
            end_keyword_loc: None,
        })),
        case_keyword_loc: span(),
        end_keyword_loc: span(),
    };
    let view = owned(&case, &pool).case_view().expect("case");
    assert!(view.predicate.is_some() && view.else_body.is_some());
    assert_eq!(view.whens.len(), 1);
    let when = view.whens[0].when_view().expect("when");
    assert_eq!(when.conditions.len(), 1);
    assert!(when.body.is_none());

    // Empty statements node is Some(empty), distinct from null.
    let empty = Node::WhenNode {
        flags: 0,
        span: span(),
        keyword_loc: span(),
        conditions: vec![],
        then_keyword_loc: None,
        statements: Some(Box::new(stmts(vec![]))),
    };
    assert_eq!(
        owned(&empty, &pool)
            .when_view()
            .expect("when")
            .body
            .expect("body")
            .len(),
        0
    );
    // Bare `case` has no predicate or else.
    let bare = Node::CaseNode {
        flags: 0,
        span: span(),
        predicate: None,
        conditions: vec![],
        else_clause: None,
        case_keyword_loc: span(),
        end_keyword_loc: span(),
    };
    let view = owned(&bare, &pool).case_view().expect("bare");
    assert!(view.predicate.is_none() && view.else_body.is_none());
}

#[test]
fn string_interpolation_covers_embedded_forms() {
    let (pool, ids) = pool_with(&[b"name"]);
    let interp = Node::InterpolatedStringNode {
        flags: 0,
        span: span(),
        opening_loc: None,
        parts: vec![
            Node::StringNode {
                flags: 0,
                span: span(),
                opening_loc: None,
                content_loc: span(),
                closing_loc: None,
                unescaped: b"hi ".to_vec(),
            },
            Node::EmbeddedStatementsNode {
                flags: 0,
                span: span(),
                opening_loc: span(),
                statements: Some(Box::new(stmts(vec![lvar_read(ids[0])]))),
                closing_loc: span(),
            },
        ],
        closing_loc: None,
    };
    let parts = owned(&interp, &pool).string_parts().expect("parts");
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].kind_name(), "StringNode");
    assert_eq!(
        owned(parts[1].node, &pool)
            .embedded_body()
            .expect("body")
            .len(),
        1
    );
    assert!(owned(parts[1].node, &pool).embedded_var().is_none());

    // Empty `"#{}"` is a null subtree.
    let null = Node::EmbeddedStatementsNode {
        flags: 0,
        span: span(),
        opening_loc: span(),
        statements: None,
        closing_loc: span(),
    };
    assert!(owned(&null, &pool).embedded_body().is_none());
    let var = Node::EmbeddedVariableNode {
        flags: 0,
        span: span(),
        operator_loc: span(),
        variable: Box::new(lvar_read(ids[0])),
    };
    assert_eq!(
        owned(&var, &pool).embedded_var().expect("var").kind_name(),
        "LocalVariableReadNode"
    );
}

#[test]
fn block_lambda_yield_and_parameters() {
    let (pool, ids) = pool_with(&[b"a", b"b", b"c", b"d", b"e", b"f", b"g", b"blk", b"l"]);
    let params = Node::ParametersNode {
        flags: 0,
        span: span(),
        requireds: vec![Node::RequiredParameterNode {
            flags: 0,
            span: span(),
            name: ids[0],
        }],
        optionals: vec![Node::OptionalParameterNode {
            flags: 0,
            span: span(),
            name: ids[1],
            name_loc: span(),
            operator_loc: span(),
            value: Box::new(int(1)),
        }],
        rest: Some(Box::new(Node::RestParameterNode {
            flags: 0,
            span: span(),
            name: Some(ids[2]),
            name_loc: None,
            operator_loc: span(),
        })),
        posts: vec![],
        keywords: vec![
            Node::RequiredKeywordParameterNode {
                flags: 0,
                span: span(),
                name: ids[3],
                name_loc: span(),
            },
            Node::OptionalKeywordParameterNode {
                flags: 0,
                span: span(),
                name: ids[4],
                name_loc: span(),
                value: Box::new(int(2)),
            },
        ],
        keyword_rest: Some(Box::new(Node::KeywordRestParameterNode {
            flags: 0,
            span: span(),
            name: Some(ids[5]),
            name_loc: None,
            operator_loc: span(),
        })),
        block: Some(Box::new(Node::BlockParameterNode {
            flags: 0,
            span: span(),
            name: Some(ids[7]),
            name_loc: None,
            operator_loc: span(),
        })),
    };
    let parts = owned(&params, &pool).parameters_view().expect("params");
    assert_eq!(parts.requireds.len(), 1);
    assert_eq!(parts.requireds[0].required_param_name().expect("req"), b"a");
    let (name, _) = parts.optionals[0].optional_param().expect("opt");
    assert_eq!(name, b"b");
    assert_eq!(
        parts
            .rest
            .expect("rest")
            .rest_param_name()
            .expect("rest name")
            .expect("named"),
        b"c"
    );
    assert_eq!(parts.keywords.len(), 2);
    let kw = parts.keywords[0].keyword_param().expect("kw");
    assert_eq!(kw.name, b"d");
    assert!(kw.default.is_none());
    let kw = parts.keywords[1].keyword_param().expect("kw");
    assert_eq!(kw.name, b"e");
    assert!(kw.default.is_some());
    assert_eq!(
        parts
            .keyword_rest
            .expect("kwrest")
            .keyword_rest_name()
            .expect("kr")
            .expect("named"),
        b"f"
    );
    assert_eq!(
        parts
            .block
            .expect("blk")
            .block_param_name()
            .expect("bp")
            .expect("named"),
        b"blk"
    );
    // Anonymous rest/kwrest/block read as inner None.
    let anon = Node::RestParameterNode {
        flags: 0,
        span: span(),
        name: None,
        name_loc: None,
        operator_loc: span(),
    };
    assert!(owned(&anon, &pool)
        .rest_param_name()
        .expect("anon")
        .is_none());

    let block = Node::BlockNode {
        flags: 0,
        span: span(),
        locals: vec![ids[8]],
        parameters: Some(Box::new(Node::BlockParametersNode {
            flags: 0,
            span: span(),
            parameters: Some(Box::new(params)),
            locals: vec![Node::BlockLocalVariableNode {
                flags: 0,
                span: span(),
                name: ids[8],
            }],
            opening_loc: None,
            closing_loc: None,
        })),
        body: Some(Box::new(stmts(vec![int(1)]))),
        opening_loc: span(),
        closing_loc: span(),
    };
    let view = owned(&block, &pool).block_view().expect("block");
    assert_eq!(view.locals, vec![b"l".to_vec()]);
    let bp = view.params.expect("block params");
    let inner = bp.block_param_view().expect("bp view");
    assert!(inner.params.is_some());
    assert_eq!(inner.block_locals.len(), 1);
    assert_eq!(
        inner.block_locals[0].block_local_name().expect("local"),
        b"l"
    );
    assert!(view.body.is_some());

    let lambda = Node::LambdaNode {
        flags: 0,
        span: span(),
        locals: vec![],
        operator_loc: span(),
        opening_loc: span(),
        closing_loc: span(),
        parameters: None,
        body: None,
    };
    let view = owned(&lambda, &pool).lambda_view().expect("lambda");
    assert!(view.params.is_none() && view.body.is_none());

    let yld = Node::YieldNode {
        flags: 0,
        span: span(),
        keyword_loc: span(),
        lparen_loc: None,
        arguments: Some(Box::new(Node::ArgumentsNode {
            flags: 0,
            span: span(),
            arguments: vec![int(1)],
        })),
        rparen_loc: None,
    };
    assert!(owned(&yld, &pool)
        .yield_view()
        .expect("yield")
        .args
        .is_some());
    let bare = Node::YieldNode {
        flags: 0,
        span: span(),
        keyword_loc: span(),
        lparen_loc: None,
        arguments: None,
        rparen_loc: None,
    };
    assert!(owned(&bare, &pool)
        .yield_view()
        .expect("bare")
        .args
        .is_none());

    let numbered = Node::NumberedParametersNode {
        flags: 0,
        span: span(),
        maximum: 2,
    };
    assert_eq!(owned(&numbered, &pool).numbered_max(), Some(2));
    let it = Node::ItLocalVariableReadNode {
        flags: 0,
        span: span(),
    };
    assert_eq!(owned(&it, &pool).it_read(), Some(()));
    let bare_arg = Node::BlockArgumentNode {
        flags: 0,
        span: span(),
        expression: None,
        operator_loc: span(),
    };
    assert!(owned(&bare_arg, &pool).block_arg().expect("arg").is_none());
}

#[test]
fn block_param_noblock_forms() {
    let (pool, ids) = pool_with(&[b"blk", b"nil"]);
    let plain = Node::BlockParameterNode {
        flags: 0,
        span: span(),
        name: Some(ids[0]),
        name_loc: None,
        operator_loc: span(),
    };
    assert!(!owned(&plain, &pool).block_param_noblock());
    let flagged = Node::BlockParameterNode {
        flags: 8,
        span: span(),
        name: Some(ids[0]),
        name_loc: None,
        operator_loc: span(),
    };
    assert!(owned(&flagged, &pool).block_param_noblock());
    let named_nil = Node::BlockParameterNode {
        flags: 0,
        span: span(),
        name: Some(ids[1]),
        name_loc: None,
        operator_loc: span(),
    };
    assert!(owned(&named_nil, &pool).block_param_noblock());
    assert!(!owned(&int(1), &pool).block_param_noblock());
}

#[test]
fn def_class_module_sclass_cover_paths() {
    let (pool, ids) = pool_with(&[b"foo", b"recv", b"Foo", b"Bar", b"Baz"]);
    let def = Node::DefNode {
        flags: 0,
        span: span(),
        name: ids[0],
        name_loc: span(),
        receiver: Some(Box::new(lvar_read(ids[1]))),
        parameters: Some(Box::new(Node::ParametersNode {
            flags: 0,
            span: span(),
            requireds: vec![],
            optionals: vec![],
            rest: None,
            posts: vec![],
            keywords: vec![],
            keyword_rest: None,
            block: None,
        })),
        body: Some(Box::new(stmts(vec![int(1)]))),
        locals: vec![],
        def_keyword_loc: span(),
        operator_loc: None,
        lparen_loc: None,
        rparen_loc: None,
        equal_loc: None,
        end_keyword_loc: None,
    };
    let view = owned(&def, &pool).def_view().expect("def");
    assert_eq!(view.name, b"foo");
    assert!(view.receiver.is_some() && view.params.is_some() && view.body.is_some());

    // Plain `class Foo`.
    let class = Node::ClassNode {
        flags: 0,
        span: span(),
        locals: vec![],
        class_keyword_loc: span(),
        constant_path: Box::new(Node::ConstantReadNode {
            flags: 0,
            span: span(),
            name: ids[2],
        }),
        inheritance_operator_loc: None,
        superclass: None,
        body: None,
        end_keyword_loc: span(),
        name: ids[2],
    };
    let view = owned(&class, &pool).class_view().expect("class");
    assert_eq!(view.name, b"Foo");
    assert!(view.cpath_is_read && view.cpath_parent.is_none());

    // Scoped `class Foo::Bar < Baz`.
    let scoped = Node::ClassNode {
        flags: 0,
        span: span(),
        locals: vec![],
        class_keyword_loc: span(),
        constant_path: Box::new(Node::ConstantPathNode {
            flags: 0,
            span: span(),
            parent: Some(Box::new(Node::ConstantReadNode {
                flags: 0,
                span: span(),
                name: ids[2],
            })),
            name: Some(ids[3]),
            delimiter_loc: span(),
            name_loc: span(),
        }),
        inheritance_operator_loc: None,
        superclass: Some(Box::new(Node::ConstantReadNode {
            flags: 0,
            span: span(),
            name: ids[4],
        })),
        body: None,
        end_keyword_loc: span(),
        name: ids[3],
    };
    let view = owned(&scoped, &pool).class_view().expect("scoped");
    assert!(!view.cpath_is_read);
    assert!(view.cpath_parent.is_some() && view.superclass.is_some());

    // Rooted `::Foo` has no parent but is still a path.
    let rooted = Node::ConstantPathNode {
        flags: 0,
        span: span(),
        parent: None,
        name: Some(ids[2]),
        delimiter_loc: span(),
        name_loc: span(),
    };
    let path = owned(&rooted, &pool).const_path().expect("rooted");
    assert_eq!(path.name, b"Foo");
    assert!(path.parent.is_none());

    // Nameless path: `const_path` fails, `constant_path_parts` keeps empty.
    let nameless = Node::ConstantPathNode {
        flags: 0,
        span: span(),
        parent: None,
        name: None,
        delimiter_loc: span(),
        name_loc: span(),
    };
    assert!(owned(&nameless, &pool).const_path().is_none());
    let (parent, name) = owned(&nameless, &pool)
        .constant_path_parts()
        .expect("parts");
    assert!(parent.is_none() && name.is_empty());

    // Non-path constant path rejects class/module views.
    let bad = Node::ClassNode {
        flags: 0,
        span: span(),
        locals: vec![],
        class_keyword_loc: span(),
        constant_path: Box::new(Node::SelfNode {
            flags: 0,
            span: span(),
        }),
        inheritance_operator_loc: None,
        superclass: None,
        body: None,
        end_keyword_loc: span(),
        name: ids[2],
    };
    assert!(owned(&bad, &pool).class_view().is_none());

    let module = Node::ModuleNode {
        flags: 0,
        span: span(),
        locals: vec![],
        module_keyword_loc: span(),
        constant_path: Box::new(Node::ConstantReadNode {
            flags: 0,
            span: span(),
            name: ids[2],
        }),
        body: None,
        end_keyword_loc: span(),
        name: ids[2],
    };
    let view = owned(&module, &pool).module_view().expect("module");
    assert!(view.cpath_is_read);

    let sclass = Node::SingletonClassNode {
        flags: 0,
        span: span(),
        locals: vec![],
        class_keyword_loc: span(),
        operator_loc: span(),
        expression: Box::new(Node::SelfNode {
            flags: 0,
            span: span(),
        }),
        body: None,
        end_keyword_loc: span(),
    };
    let view = owned(&sclass, &pool).sclass_view().expect("sclass");
    assert_eq!(view.expression.kind_name(), "SelfNode");
    assert!(view.body.is_none());
}

#[test]
fn var_and_const_accessors_cover_reads_writes_targets() {
    let (pool, ids) = pool_with(&[b"@x", b"@@x", b"$x", b"$&", b"A", b"B"]);
    let read = Node::InstanceVariableReadNode {
        flags: 0,
        span: span(),
        name: ids[0],
    };
    assert_eq!(owned(&read, &pool).ivar_read().expect("ivar"), b"@x");
    assert_eq!(
        owned(&read, &pool).instance_var_read_name().expect("dup"),
        b"@x"
    );
    let write = Node::InstanceVariableWriteNode {
        flags: 0,
        span: span(),
        name: ids[0],
        name_loc: span(),
        value: Box::new(int(1)),
        operator_loc: span(),
    };
    assert_eq!(
        owned(&write, &pool).ivar_write().expect("write").name,
        b"@x"
    );

    let cvar = Node::ClassVariableReadNode {
        flags: 0,
        span: span(),
        name: ids[1],
    };
    assert_eq!(owned(&cvar, &pool).cvar_read().expect("cvar"), b"@@x");
    assert_eq!(
        owned(&cvar, &pool).class_var_read_name().expect("dup"),
        b"@@x"
    );
    let cvar_write = Node::ClassVariableWriteNode {
        flags: 0,
        span: span(),
        name: ids[1],
        name_loc: span(),
        value: Box::new(int(1)),
        operator_loc: span(),
    };
    assert_eq!(
        owned(&cvar_write, &pool).cvar_write().expect("write").name,
        b"@@x"
    );

    let gvar = Node::GlobalVariableReadNode {
        flags: 0,
        span: span(),
        name: ids[2],
    };
    assert_eq!(owned(&gvar, &pool).gvar_read().expect("gvar"), b"$x");
    assert_eq!(
        owned(&gvar, &pool).global_var_read_name().expect("dup"),
        b"$x"
    );
    let gvar_write = Node::GlobalVariableWriteNode {
        flags: 0,
        span: span(),
        name: ids[2],
        name_loc: span(),
        value: Box::new(int(1)),
        operator_loc: span(),
    };
    assert!(owned(&gvar_write, &pool).gvar_write().is_some());

    let backref = Node::BackReferenceReadNode {
        flags: 0,
        span: span(),
        name: ids[3],
    };
    assert_eq!(
        owned(&backref, &pool).backref_name().expect("backref"),
        b"$&"
    );
    let numbered = Node::NumberedReferenceReadNode {
        flags: 0,
        span: span(),
        number: 1,
    };
    assert_eq!(owned(&numbered, &pool).numbered_ref_number(), Some(1));

    let const_read = Node::ConstantReadNode {
        flags: 0,
        span: span(),
        name: ids[4],
    };
    assert_eq!(owned(&const_read, &pool).const_read().expect("const"), b"A");
    assert_eq!(
        owned(&const_read, &pool).constant_read_name().expect("dup"),
        b"A"
    );
    let const_write = Node::ConstantWriteNode {
        flags: 0,
        span: span(),
        name: ids[4],
        name_loc: span(),
        value: Box::new(int(1)),
        operator_loc: span(),
    };
    assert_eq!(
        owned(&const_write, &pool)
            .const_write()
            .expect("write")
            .name,
        b"A"
    );

    // Like Prism itself, the write target is a plain `ConstantPathNode`.
    let path_write = Node::ConstantPathWriteNode {
        flags: 0,
        span: span(),
        target: Box::new(Node::ConstantPathNode {
            flags: 0,
            span: span(),
            parent: Some(Box::new(Node::ConstantReadNode {
                flags: 0,
                span: span(),
                name: ids[4],
            })),
            name: Some(ids[5]),
            delimiter_loc: span(),
            name_loc: span(),
        }),
        operator_loc: span(),
        value: Box::new(int(1)),
    };
    let view = owned(&path_write, &pool)
        .const_path_write()
        .expect("path write");
    assert_eq!(view.name, b"B");
    assert!(view.parent.is_some());

    // Targets.
    let ivar_target = Node::InstanceVariableTargetNode {
        flags: 0,
        span: span(),
        name: ids[0],
    };
    assert_eq!(
        owned(&ivar_target, &pool).ivar_target_name().expect("t"),
        b"@x"
    );
    let cvar_target = Node::ClassVariableTargetNode {
        flags: 0,
        span: span(),
        name: ids[1],
    };
    assert_eq!(
        owned(&cvar_target, &pool).cvar_target_name().expect("t"),
        b"@@x"
    );
    let gvar_target = Node::GlobalVariableTargetNode {
        flags: 0,
        span: span(),
        name: ids[2],
    };
    assert_eq!(
        owned(&gvar_target, &pool).gvar_target_name().expect("t"),
        b"$x"
    );
    let const_target = Node::ConstantTargetNode {
        flags: 0,
        span: span(),
        name: ids[4],
    };
    assert_eq!(
        owned(&const_target, &pool).const_target_name().expect("t"),
        b"A"
    );
    let path_target = Node::ConstantPathTargetNode {
        flags: 0,
        span: span(),
        parent: None,
        name: Some(ids[5]),
        delimiter_loc: span(),
        name_loc: span(),
    };
    let (parent, name) = owned(&path_target, &pool).const_path_target().expect("pt");
    assert!(parent.is_none());
    assert_eq!(name, b"B");

    // Index and call targets.
    let index = Node::IndexTargetNode {
        flags: 0,
        span: span(),
        receiver: Box::new(int(1)),
        opening_loc: span(),
        arguments: Some(Box::new(Node::ArgumentsNode {
            flags: 0,
            span: span(),
            arguments: vec![int(2)],
        })),
        closing_loc: span(),
        block: None,
    };
    let view = owned(&index, &pool).index_target().expect("index");
    assert_eq!(view.receiver.kind_name(), "IntegerNode");
    assert!(view.args.is_some());
    let target = Node::CallTargetNode {
        flags: 0,
        span: span(),
        receiver: Box::new(int(1)),
        call_operator_loc: span(),
        name: ids[4],
        message_loc: span(),
    };
    let view = owned(&target, &pool).call_target().expect("call target");
    assert_eq!(view.receiver.kind_name(), "IntegerNode");
    assert_eq!(view.name, b"A");
}

#[test]
fn super_and_forwarding_super_gating() {
    let (pool, _) = pool_with(&[]);
    // Empty `super()` carries no args.
    let empty = Node::SuperNode {
        flags: 0,
        span: span(),
        keyword_loc: span(),
        lparen_loc: None,
        arguments: None,
        rparen_loc: None,
        block: None,
    };
    let view = owned(&empty, &pool).super_view().expect("super");
    assert!(view.args.is_none());
    // Plain positional args pass through.
    let plain = Node::SuperNode {
        flags: 0,
        span: span(),
        keyword_loc: span(),
        lparen_loc: None,
        arguments: Some(Box::new(Node::ArgumentsNode {
            flags: 0,
            span: span(),
            arguments: vec![int(1)],
        })),
        rparen_loc: None,
        block: None,
    };
    assert_eq!(
        owned(&plain, &pool)
            .super_view()
            .expect("plain")
            .args
            .expect("args")
            .len(),
        1
    );
    // `...` forwarding rides along like the FFI path.
    let fwd = Node::SuperNode {
        flags: 0,
        span: span(),
        keyword_loc: span(),
        lparen_loc: None,
        arguments: Some(Box::new(Node::ArgumentsNode {
            flags: 0,
            span: span(),
            arguments: vec![Node::ForwardingArgumentsNode {
                flags: 0,
                span: span(),
            }],
        })),
        rparen_loc: None,
        block: None,
    };
    assert!(owned(&fwd, &pool).super_view().is_some());
    // Splat, keyword hash and block forms stay gated.
    for gated in [
        Node::SuperNode {
            flags: 0,
            span: span(),
            keyword_loc: span(),
            lparen_loc: None,
            arguments: Some(Box::new(Node::ArgumentsNode {
                flags: 0,
                span: span(),
                arguments: vec![Node::SplatNode {
                    flags: 0,
                    span: span(),
                    operator_loc: span(),
                    expression: Some(Box::new(int(1))),
                }],
            })),
            rparen_loc: None,
            block: None,
        },
        Node::SuperNode {
            flags: 0,
            span: span(),
            keyword_loc: span(),
            lparen_loc: None,
            arguments: Some(Box::new(Node::ArgumentsNode {
                flags: 0,
                span: span(),
                arguments: vec![Node::KeywordHashNode {
                    flags: 0,
                    span: span(),
                    elements: vec![],
                }],
            })),
            rparen_loc: None,
            block: None,
        },
        Node::SuperNode {
            flags: 0,
            span: span(),
            keyword_loc: span(),
            lparen_loc: None,
            arguments: None,
            rparen_loc: None,
            block: Some(Box::new(Node::BlockArgumentNode {
                flags: 0,
                span: span(),
                expression: None,
                operator_loc: span(),
            })),
        },
    ] {
        assert!(owned(&gated, &pool).super_view().is_none());
    }
    // Forwarding super carries an optional block.
    let bare = Node::ForwardingSuperNode {
        flags: 0,
        span: span(),
        block: None,
    };
    assert!(owned(&bare, &pool)
        .forwarding_super()
        .expect("fwd")
        .is_none());
    assert!(owned(&int(1), &pool).forwarding_super().is_none());
}

#[test]
fn alias_undef_defined_implicit_parentheses() {
    let (pool, _) = pool_with(&[]);
    let alias = Node::AliasMethodNode {
        flags: 0,
        span: span(),
        new_name: Box::new(Node::SymbolNode {
            flags: 0,
            span: span(),
            opening_loc: None,
            value_loc: None,
            closing_loc: None,
            unescaped: b"bar".to_vec(),
        }),
        old_name: Box::new(Node::SymbolNode {
            flags: 0,
            span: span(),
            opening_loc: None,
            value_loc: None,
            closing_loc: None,
            unescaped: b"foo".to_vec(),
        }),
        keyword_loc: span(),
    };
    let (new_name, old_name) = owned(&alias, &pool).alias_pair().expect("alias");
    assert_eq!(new_name.symbol_lit().expect("new"), b"bar");
    assert_eq!(old_name.symbol_lit().expect("old"), b"foo");
    assert!(owned(&alias, &pool).undef_list().is_none());

    let undef = Node::UndefNode {
        flags: 0,
        span: span(),
        names: vec![Node::SymbolNode {
            flags: 0,
            span: span(),
            opening_loc: None,
            value_loc: None,
            closing_loc: None,
            unescaped: b"foo".to_vec(),
        }],
        keyword_loc: span(),
    };
    let names = owned(&undef, &pool).undef_list().expect("undef");
    assert_eq!(names.len(), 1);
    assert!(owned(&undef, &pool).alias_pair().is_none());

    let defined = Node::DefinedNode {
        flags: 0,
        span: span(),
        lparen_loc: None,
        value: Box::new(int(1)),
        rparen_loc: None,
        keyword_loc: span(),
    };
    assert_eq!(
        owned(&defined, &pool)
            .defined_value()
            .expect("defined")
            .kind_name(),
        "IntegerNode"
    );
    let implicit = Node::ImplicitNode {
        flags: 0,
        span: span(),
        value: Box::new(int(1)),
    };
    assert_eq!(
        owned(&implicit, &pool)
            .implicit_value()
            .expect("implicit")
            .kind_name(),
        "IntegerNode"
    );
    let parens = Node::ParenthesesNode {
        flags: 0,
        span: span(),
        body: Some(Box::new(stmts(vec![int(1)]))),
        opening_loc: span(),
        closing_loc: span(),
    };
    assert!(owned(&parens, &pool)
        .parentheses_body()
        .expect("parens")
        .is_some());
    // Empty `()` is Some(None).
    let unit = Node::ParenthesesNode {
        flags: 0,
        span: span(),
        body: None,
        opening_loc: span(),
        closing_loc: span(),
    };
    assert!(owned(&unit, &pool)
        .parentheses_body()
        .expect("unit")
        .is_none());
}

#[test]
fn begin_rescue_ensure_modifier() {
    let (pool, ids) = pool_with(&[b"E"]);
    let begin = Node::BeginNode {
        flags: 0,
        span: span(),
        begin_keyword_loc: None,
        statements: Some(Box::new(stmts(vec![int(1)]))),
        rescue_clause: Some(Box::new(Node::RescueNode {
            flags: 0,
            span: span(),
            keyword_loc: span(),
            exceptions: vec![Node::ConstantReadNode {
                flags: 0,
                span: span(),
                name: ids[0],
            }],
            operator_loc: None,
            reference: Some(Box::new(lvar_read(ids[0]))),
            then_keyword_loc: None,
            statements: Some(Box::new(stmts(vec![int(2)]))),
            subsequent: None,
        })),
        else_clause: Some(Box::new(Node::ElseNode {
            flags: 0,
            span: span(),
            else_keyword_loc: span(),
            statements: Some(Box::new(stmts(vec![int(3)]))),
            end_keyword_loc: None,
        })),
        ensure_clause: Some(Box::new(Node::EnsureNode {
            flags: 0,
            span: span(),
            ensure_keyword_loc: span(),
            statements: Some(Box::new(stmts(vec![int(4)]))),
            end_keyword_loc: span(),
        })),
        end_keyword_loc: None,
    };
    let view = owned(&begin, &pool).begin_view().expect("begin");
    assert!(!view.bare);
    assert_eq!(view.statements.expect("stmts").len(), 1);
    let clause = view
        .rescue_clause
        .expect("rescue")
        .rescue_view()
        .expect("view");
    assert_eq!(clause.exceptions.len(), 1);
    assert!(clause.reference.is_some());
    assert_eq!(clause.statements.expect("stmts").len(), 1);
    assert!(clause.subsequent.is_none());
    let ensure = view.ensure_clause.expect("ensure");
    assert_eq!(
        ensure
            .ensure_view()
            .expect("view")
            .statements
            .expect("s")
            .len(),
        1
    );
    assert!(view.else_clause.is_some());

    // Bare `begin` with no clauses.
    let bare = Node::BeginNode {
        flags: 0,
        span: span(),
        begin_keyword_loc: None,
        statements: Some(Box::new(stmts(vec![int(1)]))),
        rescue_clause: None,
        else_clause: None,
        ensure_clause: None,
        end_keyword_loc: None,
    };
    let view = owned(&bare, &pool).begin_view().expect("bare");
    assert!(view.bare);

    let modifier = Node::RescueModifierNode {
        flags: 0,
        span: span(),
        expression: Box::new(int(1)),
        keyword_loc: span(),
        rescue_expression: Box::new(int(2)),
    };
    let view = owned(&modifier, &pool)
        .rescue_modifier_view()
        .expect("modifier");
    assert_eq!(view.expression.kind_name(), "IntegerNode");
    assert_eq!(view.rescue_expression.kind_name(), "IntegerNode");
}

#[test]
fn multi_write_and_target_forms() {
    let (pool, ids) = pool_with(&[b"a", b"b", b"c"]);
    let write = Node::MultiWriteNode {
        flags: 0,
        span: span(),
        lefts: vec![Node::LocalVariableTargetNode {
            flags: 0,
            span: span(),
            name: ids[0],
            depth: 0,
        }],
        rest: Some(Box::new(Node::SplatNode {
            flags: 0,
            span: span(),
            operator_loc: span(),
            expression: Some(Box::new(Node::LocalVariableTargetNode {
                flags: 0,
                span: span(),
                name: ids[1],
                depth: 0,
            })),
        })),
        rights: vec![Node::LocalVariableTargetNode {
            flags: 0,
            span: span(),
            name: ids[2],
            depth: 0,
        }],
        lparen_loc: None,
        rparen_loc: None,
        operator_loc: span(),
        value: Box::new(Node::ArrayNode {
            flags: 0,
            span: span(),
            elements: vec![int(1)],
            opening_loc: None,
            closing_loc: None,
        }),
    };
    let view = owned(&write, &pool).multi_write_view().expect("multi");
    assert_eq!(view.lefts.len(), 1);
    assert!(view.lefts[0].lvar_target().is_some());
    assert_eq!(view.rest.expect("rest").kind_name(), "SplatNode");
    assert_eq!(view.rights.len(), 1);
    assert_eq!(view.value.kind_name(), "ArrayNode");

    // Nested target and implicit rest.
    let nested = Node::MultiWriteNode {
        flags: 0,
        span: span(),
        lefts: vec![
            Node::MultiTargetNode {
                flags: 0,
                span: span(),
                lefts: vec![Node::LocalVariableTargetNode {
                    flags: 0,
                    span: span(),
                    name: ids[0],
                    depth: 0,
                }],
                rest: None,
                rights: vec![],
                lparen_loc: None,
                rparen_loc: None,
            },
            Node::LocalVariableTargetNode {
                flags: 0,
                span: span(),
                name: ids[2],
                depth: 0,
            },
        ],
        rest: Some(Box::new(Node::ImplicitRestNode {
            flags: 0,
            span: span(),
        })),
        rights: vec![],
        lparen_loc: None,
        rparen_loc: None,
        operator_loc: span(),
        value: Box::new(int(1)),
    };
    let view = owned(&nested, &pool).multi_write_view().expect("nested");
    assert_eq!(view.lefts[0].kind_name(), "MultiTargetNode");
    let inner = view.lefts[0].multi_target_view().expect("target");
    assert_eq!(inner.lefts.len(), 1);
    assert!(inner.rest.is_none());
    assert_eq!(view.rest.expect("rest").kind_name(), "ImplicitRestNode");
}
