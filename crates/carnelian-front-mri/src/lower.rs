//! MRI lowering: `lib-ruby-parser` tree to the owned AST (P4-A).
//!
//! Covers the 3.1.2 grammar 1:1. Scope data stays blank here (`depth`
//! zero, `locals` empty, `NumberedParametersNode.maximum` zero); the scope
//! pass fills them afterwards. A bare-word call with no receiver, args,
//! parens or block keeps the `VARIABLE_CALL` flag so the scope pass can
//! rewrite it to a local read when the name resolves.

use carnelian_ast::{
    arguments_node_flags, array_node_flags, call_node_flags, integer_base_flags,
    keyword_hash_node_flags, limbs_to_decimal, loop_flags, regular_expression_flags, Integer, Node,
    Span, SymbolId, SymbolPool,
};
use lib_ruby_parser::nodes::*;
use lib_ruby_parser::Loc;
use lib_ruby_parser::Node as Mri;

type Pool = SymbolPool;

/// Lower an MRI tree to its owned tree plus symbol pool.
pub fn lower(root: &Mri) -> (Node, SymbolPool) {
    let mut pool = SymbolPool::new();
    let node = conv(root, &mut pool);
    (node, pool)
}

/// Byte offsets of an MRI location, saturating on huge inputs.
fn span(loc: &Loc) -> Span {
    Span {
        start: u32::try_from(loc.begin).unwrap_or(u32::MAX),
        end: u32::try_from(loc.end).unwrap_or(u32::MAX),
    }
}

/// Optional MRI location to an optional span.
fn ospan(loc: &Option<Loc>) -> Option<Span> {
    loc.as_ref().map(span)
}

/// Full-expression span of an MRI node.
fn espan(node: &Mri) -> Span {
    span(node.expression())
}

/// Required span with an expression-span fallback (unreachable in valid trees).
fn reqspan(loc: &Option<Loc>, fallback: Span) -> Span {
    ospan(loc).unwrap_or(fallback)
}

/// Intern a UTF-8 name.
fn sym(pool: &mut Pool, name: &str) -> SymbolId {
    pool.intern(name.as_bytes())
}

/// Intern raw bytes (string/symbol payloads).
fn raw_sym(pool: &mut Pool, bytes: &[u8]) -> SymbolId {
    pool.intern(bytes)
}

/// Wrap lowered statements; every sequence position is a `StatementsNode`.
fn seq(body: Vec<Node>, span: Span) -> Box<Node> {
    Box::new(Node::StatementsNode {
        flags: 0,
        span,
        body,
    })
}

/// Lower an optional body, always `StatementsNode`-wrapped when present.
fn body_of(node: Option<&Mri>, pool: &mut Pool) -> Option<Box<Node>> {
    node.map(|inner| seq_one(inner, pool))
}

/// Lower one body node: paren-less `Begin` spreads, anything else wraps.
fn seq_one(node: &Mri, pool: &mut Pool) -> Box<Node> {
    match node {
        Mri::Begin(inner) if inner.begin_l.is_none() && inner.end_l.is_none() => seq(
            inner
                .statements
                .iter()
                .map(|stmt| conv(stmt, pool))
                .collect(),
            espan(node),
        ),
        other => {
            let span = espan(other);
            seq(vec![conv(other, pool)], span)
        }
    }
}

/// Lower a body that merges into an enclosing statement list (no wrapper).
fn flat_body(node: Option<&Mri>, pool: &mut Pool) -> Vec<Node> {
    match node {
        None => Vec::new(),
        Some(Mri::Begin(inner)) if inner.begin_l.is_none() && inner.end_l.is_none() => inner
            .statements
            .iter()
            .map(|stmt| conv(stmt, pool))
            .collect(),
        Some(other) => vec![conv(other, pool)],
    }
}

/// `then` keyword when the separator spans exactly four bytes (`then`).
fn then_loc(loc: &Loc) -> Option<Span> {
    (loc.end.saturating_sub(loc.begin) == 4).then(|| span(loc))
}

/// `do` keyword when the separator spans exactly two bytes (`do`).
fn do_loc(loc: &Loc) -> Option<Span> {
    (loc.end.saturating_sub(loc.begin) == 2).then(|| span(loc))
}

/// Lower one MRI node to its owned shape.
#[allow(clippy::too_many_lines)]
fn conv(node: &Mri, pool: &mut Pool) -> Node {
    match node {
        Mri::Alias(inner) => {
            let whole = espan(node);
            let to = conv(&inner.to, pool);
            let from = conv(&inner.from, pool);
            let keyword_loc = span(&inner.keyword_l);
            if matches!(&*inner.to, Mri::Gvar(_)) {
                Node::AliasGlobalVariableNode {
                    flags: 0,
                    span: whole,
                    new_name: Box::new(to),
                    old_name: Box::new(from),
                    keyword_loc,
                }
            } else {
                Node::AliasMethodNode {
                    flags: 0,
                    span: whole,
                    new_name: Box::new(to),
                    old_name: Box::new(from),
                    keyword_loc,
                }
            }
        }
        Mri::And(inner) => Node::AndNode {
            flags: 0,
            span: espan(node),
            left: Box::new(conv(&inner.lhs, pool)),
            right: Box::new(conv(&inner.rhs, pool)),
            operator_loc: span(&inner.operator_l),
        },
        Mri::AndAsgn(inner) => lower_logic_write(
            &inner.recv,
            true,
            &inner.operator_l,
            conv(&inner.value, pool),
            node,
            pool,
        ),
        Mri::Arg(inner) => Node::RequiredParameterNode {
            flags: 0,
            span: espan(node),
            name: sym(pool, &inner.name),
        },
        Mri::Args(inner) => {
            let mut shadows = Vec::new();
            let slots = build_params(&inner.args, pool, &mut shadows);
            // Standalone args cannot carry `;`-locals; drop if ever present.
            drop(shadows);
            params_node(slots, espan(node))
        }
        Mri::Array(inner) => {
            let elements: Vec<Node> = inner.elements.iter().map(|el| conv(el, pool)).collect();
            let mut flags = 0;
            if inner.elements.iter().any(|el| matches!(el, Mri::Splat(_))) {
                flags |= array_node_flags::CONTAINS_SPLAT;
            }
            Node::ArrayNode {
                flags,
                span: espan(node),
                elements,
                opening_loc: ospan(&inner.begin_l),
                closing_loc: ospan(&inner.end_l),
            }
        }
        Mri::ArrayPattern(inner) => lower_array_pattern(
            &inner.elements,
            None,
            false,
            &inner.begin_l,
            &inner.end_l,
            node,
            pool,
        ),
        Mri::ArrayPatternWithTail(inner) => lower_array_pattern(
            &inner.elements,
            None,
            // A trailing comma is an implicit rest (`[a,]` matches longer
            // arrays, like Prism's `ImplicitRestNode`).
            true,
            &inner.begin_l,
            &inner.end_l,
            node,
            pool,
        ),
        Mri::BackRef(inner) => Node::BackReferenceReadNode {
            flags: 0,
            span: espan(node),
            name: sym(pool, &inner.name),
        },
        Mri::Begin(inner) => lower_begin(inner, node, pool),
        Mri::Block(inner) => lower_block(inner, node, pool),
        Mri::Blockarg(inner) => Node::BlockParameterNode {
            flags: 0,
            span: espan(node),
            name: inner.name.as_deref().map(|name| sym(pool, name)),
            name_loc: ospan(&inner.name_l),
            operator_loc: span(&inner.operator_l),
        },
        Mri::BlockPass(inner) => Node::BlockArgumentNode {
            flags: 0,
            span: espan(node),
            expression: inner
                .value
                .as_deref()
                .map(|value| Box::new(conv(value, pool))),
            operator_loc: span(&inner.operator_l),
        },
        Mri::Break(inner) => Node::BreakNode {
            flags: 0,
            span: espan(node),
            arguments: call_args(&inner.args, pool),
            keyword_loc: span(&inner.keyword_l),
        },
        Mri::Case(inner) => Node::CaseNode {
            flags: 0,
            span: espan(node),
            predicate: inner.expr.as_deref().map(|expr| Box::new(conv(expr, pool))),
            conditions: inner.when_bodies.iter().map(|w| conv(w, pool)).collect(),
            else_clause: else_node(
                inner.else_body.as_deref(),
                inner.else_l.as_ref(),
                span(&inner.end_l),
                pool,
            ),
            case_keyword_loc: span(&inner.keyword_l),
            end_keyword_loc: span(&inner.end_l),
        },
        Mri::CaseMatch(inner) => Node::CaseMatchNode {
            flags: 0,
            span: espan(node),
            predicate: Some(Box::new(conv(&inner.expr, pool))),
            conditions: inner.in_bodies.iter().map(|w| conv(w, pool)).collect(),
            else_clause: else_node(
                inner.else_body.as_deref(),
                inner.else_l.as_ref(),
                span(&inner.end_l),
                pool,
            ),
            case_keyword_loc: span(&inner.keyword_l),
            end_keyword_loc: span(&inner.end_l),
        },
        Mri::Casgn(inner) => lower_casgn(inner, node, pool),
        Mri::Cbase(_) => Node::ConstantPathNode {
            flags: 0,
            span: espan(node),
            parent: None,
            name: None,
            delimiter_loc: espan(node),
            name_loc: espan(node),
        },
        Mri::Class(inner) => {
            let whole = espan(node);
            Node::ClassNode {
                flags: 0,
                span: whole,
                locals: Vec::new(),
                class_keyword_loc: span(&inner.keyword_l),
                constant_path: Box::new(conv(&inner.name, pool)),
                inheritance_operator_loc: ospan(&inner.operator_l),
                superclass: inner
                    .superclass
                    .as_deref()
                    .map(|sup| Box::new(conv(sup, pool))),
                body: body_of(inner.body.as_deref(), pool),
                end_keyword_loc: span(&inner.end_l),
                name: const_name(&inner.name, pool),
            }
        }
        Mri::Complex(inner) => Node::ImaginaryNode {
            flags: 0,
            span: espan(node),
            numeric: Box::new(lower_complex(&inner.value, espan(node))),
        },
        Mri::Const(inner) => lower_const(inner, node, pool),
        Mri::ConstPattern(inner) => lower_const_pattern(inner, node, pool),
        Mri::CSend(inner) => lower_send_parts(
            Some(&inner.recv),
            &inner.method_name,
            &inner.args,
            Some(span(&inner.dot_l)),
            inner.selector_l.as_ref(),
            inner.begin_l.as_ref(),
            inner.end_l.as_ref(),
            inner.operator_l.as_ref(),
            None,
            espan(node),
            call_node_flags::SAFE_NAVIGATION,
            pool,
        ),
        Mri::Cvar(inner) => Node::ClassVariableReadNode {
            flags: 0,
            span: espan(node),
            name: sym(pool, &inner.name),
        },
        Mri::Cvasgn(inner) => match &inner.value {
            Some(value) => Node::ClassVariableWriteNode {
                flags: 0,
                span: espan(node),
                name: sym(pool, &inner.name),
                name_loc: span(&inner.name_l),
                value: Box::new(conv(value, pool)),
                operator_loc: reqspan(&inner.operator_l, span(&inner.name_l)),
            },
            None => Node::ClassVariableReadNode {
                flags: 0,
                span: espan(node),
                name: sym(pool, &inner.name),
            },
        },
        Mri::Def(inner) => lower_def(
            &inner.name,
            &inner.name_l,
            None,
            None,
            inner.args.as_deref(),
            inner.body.as_deref(),
            &inner.keyword_l,
            inner.assignment_l.as_ref(),
            inner.end_l.as_ref(),
            node,
            pool,
        ),
        Mri::Defined(inner) => Node::DefinedNode {
            flags: 0,
            span: espan(node),
            lparen_loc: ospan(&inner.begin_l),
            value: Box::new(conv(&inner.value, pool)),
            rparen_loc: ospan(&inner.end_l),
            keyword_loc: span(&inner.keyword_l),
        },
        Mri::Defs(inner) => lower_def(
            &inner.name,
            &inner.name_l,
            Some(&inner.definee),
            Some(span(&inner.operator_l)),
            inner.args.as_deref(),
            inner.body.as_deref(),
            &inner.keyword_l,
            inner.assignment_l.as_ref(),
            inner.end_l.as_ref(),
            node,
            pool,
        ),
        Mri::Dstr(inner) => {
            let span = espan(node);
            let parts: Vec<Node> = inner.parts.iter().map(|p| interp_part(p, pool)).collect();
            if parts_all_plain(&inner.parts) {
                Node::StringNode {
                    flags: 0,
                    span,
                    opening_loc: None,
                    content_loc: inner.parts.first().map(espan).unwrap_or(span),
                    closing_loc: None,
                    unescaped: concat_str_bytes(&inner.parts),
                }
            } else {
                Node::InterpolatedStringNode {
                    flags: 0,
                    span,
                    opening_loc: ospan(&inner.begin_l),
                    parts,
                    closing_loc: ospan(&inner.end_l),
                }
            }
        }
        Mri::Dsym(inner) => {
            let span = espan(node);
            Node::InterpolatedSymbolNode {
                flags: 0,
                span,
                opening_loc: ospan(&inner.begin_l),
                parts: inner.parts.iter().map(|p| interp_part(p, pool)).collect(),
                closing_loc: ospan(&inner.end_l),
            }
        }
        Mri::EFlipFlop(inner) => Node::FlipFlopNode {
            flags: 0,
            span: espan(node),
            left: inner.left.as_deref().map(|l| Box::new(conv(l, pool))),
            right: inner.right.as_deref().map(|r| Box::new(conv(r, pool))),
            operator_loc: span(&inner.operator_l),
        },
        Mri::EmptyElse(_) => Node::ElseNode {
            flags: 0,
            span: espan(node),
            else_keyword_loc: espan(node),
            statements: None,
            end_keyword_loc: Some(espan(node)),
        },
        Mri::Encoding(_) => Node::SourceEncodingNode {
            flags: 0,
            span: espan(node),
        },
        Mri::Ensure(inner) => lower_ensure_begin(inner, None, node, pool),
        Mri::Erange(inner) => Node::RangeNode {
            flags: range_flags_exclude(),
            span: espan(node),
            left: inner.left.as_deref().map(|l| Box::new(conv(l, pool))),
            right: inner.right.as_deref().map(|r| Box::new(conv(r, pool))),
            operator_loc: span(&inner.operator_l),
        },
        Mri::False(_) => Node::FalseNode {
            flags: 0,
            span: espan(node),
        },
        Mri::File(_) => Node::SourceFileNode {
            flags: 0,
            span: espan(node),
            filepath: Vec::new(),
        },
        Mri::FindPattern(inner) => lower_find_pattern(inner, node, pool),
        Mri::Float(inner) => Node::FloatNode {
            flags: 0,
            span: espan(node),
            value: parse_float(&inner.value),
        },
        Mri::For(inner) => Node::ForNode {
            flags: 0,
            span: espan(node),
            index: Box::new(target(&inner.iterator, pool)),
            collection: Box::new(conv(&inner.iteratee, pool)),
            statements: body_of(inner.body.as_deref(), pool),
            for_keyword_loc: span(&inner.keyword_l),
            in_keyword_loc: span(&inner.operator_l),
            do_keyword_loc: do_loc(&inner.begin_l),
            end_keyword_loc: span(&inner.end_l),
        },
        Mri::ForwardArg(_) => Node::ForwardingParameterNode {
            flags: 0,
            span: espan(node),
        },
        Mri::ForwardedArgs(_) => Node::ForwardingArgumentsNode {
            flags: 0,
            span: espan(node),
        },
        Mri::Gvar(inner) => Node::GlobalVariableReadNode {
            flags: 0,
            span: espan(node),
            name: sym(pool, &inner.name),
        },
        Mri::Gvasgn(inner) => match &inner.value {
            Some(value) => Node::GlobalVariableWriteNode {
                flags: 0,
                span: espan(node),
                name: sym(pool, &inner.name),
                name_loc: span(&inner.name_l),
                value: Box::new(conv(value, pool)),
                operator_loc: reqspan(&inner.operator_l, span(&inner.name_l)),
            },
            None => Node::GlobalVariableReadNode {
                flags: 0,
                span: espan(node),
                name: sym(pool, &inner.name),
            },
        },
        Mri::Hash(inner) => {
            let span = espan(node);
            Node::HashNode {
                flags: 0,
                span,
                opening_loc: reqspan(&inner.begin_l, span),
                elements: inner.pairs.iter().map(|p| hash_element(p, pool)).collect(),
                closing_loc: reqspan(&inner.end_l, span),
            }
        }
        Mri::HashPattern(inner) => lower_hash_pattern(
            &inner.elements,
            None,
            &inner.begin_l,
            &inner.end_l,
            node,
            pool,
        ),
        Mri::Heredoc(inner) => {
            let whole = espan(node);
            if parts_all_plain(&inner.parts) {
                Node::StringNode {
                    flags: 0,
                    span: whole,
                    opening_loc: None,
                    content_loc: span(&inner.heredoc_body_l),
                    closing_loc: None,
                    unescaped: concat_str_bytes(&inner.parts),
                }
            } else {
                Node::InterpolatedStringNode {
                    flags: 0,
                    span: whole,
                    opening_loc: None,
                    parts: inner.parts.iter().map(|p| interp_part(p, pool)).collect(),
                    closing_loc: None,
                }
            }
        }
        Mri::If(inner) => lower_if(inner, node, pool),
        Mri::IfGuard(inner) => lower_guard(true, &inner.cond, &inner.keyword_l, node, pool),
        Mri::IFlipFlop(inner) => Node::FlipFlopNode {
            flags: 0,
            span: espan(node),
            left: inner.left.as_deref().map(|l| Box::new(conv(l, pool))),
            right: inner.right.as_deref().map(|r| Box::new(conv(r, pool))),
            operator_loc: span(&inner.operator_l),
        },
        Mri::IfMod(inner) => lower_if_mod(inner, node, pool),
        Mri::IfTernary(inner) => {
            let whole = espan(node);
            Node::IfNode {
                flags: 0,
                span: whole,
                if_keyword_loc: None,
                predicate: Box::new(conv(&inner.cond, pool)),
                then_keyword_loc: Some(span(&inner.question_l)),
                statements: Some(seq_one(&inner.if_true, pool)),
                subsequent: Some(Box::new(Node::ElseNode {
                    flags: 0,
                    span: whole,
                    else_keyword_loc: span(&inner.colon_l),
                    statements: Some(seq_one(&inner.if_false, pool)),
                    end_keyword_loc: Some(whole),
                })),
                end_keyword_loc: None,
            }
        }
        Mri::Index(inner) => index_read(
            &inner.recv,
            &inner.indexes,
            &inner.begin_l,
            &inner.end_l,
            node,
            pool,
        ),
        Mri::IndexAsgn(inner) => match &inner.value {
            Some(value) => index_write(
                &inner.recv,
                &inner.indexes,
                &inner.begin_l,
                &inner.end_l,
                inner.operator_l.as_ref(),
                conv(value, pool),
                node,
                pool,
            ),
            None => target(node, pool),
        },
        Mri::InPattern(inner) => {
            let mut pattern = conv(&inner.pattern, pool);
            if let Some(guard) = &inner.guard {
                pattern = wrap_guard(pattern, guard, pool);
            }
            Node::InNode {
                flags: 0,
                span: espan(node),
                pattern: Box::new(pattern),
                statements: body_of(inner.body.as_deref(), pool),
                in_loc: span(&inner.keyword_l),
                then_loc: then_loc(&inner.begin_l),
            }
        }
        Mri::Int(inner) => {
            let (value, flags) = convert_int(&inner.value);
            Node::IntegerNode {
                flags,
                span: espan(node),
                value,
            }
        }
        Mri::Irange(inner) => Node::RangeNode {
            flags: 0,
            span: espan(node),
            left: inner.left.as_deref().map(|l| Box::new(conv(l, pool))),
            right: inner.right.as_deref().map(|r| Box::new(conv(r, pool))),
            operator_loc: span(&inner.operator_l),
        },
        Mri::Ivar(inner) => Node::InstanceVariableReadNode {
            flags: 0,
            span: espan(node),
            name: sym(pool, &inner.name),
        },
        Mri::Ivasgn(inner) => match &inner.value {
            Some(value) => Node::InstanceVariableWriteNode {
                flags: 0,
                span: espan(node),
                name: sym(pool, &inner.name),
                name_loc: span(&inner.name_l),
                value: Box::new(conv(value, pool)),
                operator_loc: reqspan(&inner.operator_l, span(&inner.name_l)),
            },
            None => Node::InstanceVariableReadNode {
                flags: 0,
                span: espan(node),
                name: sym(pool, &inner.name),
            },
        },
        Mri::Kwarg(inner) => Node::RequiredKeywordParameterNode {
            flags: 0,
            span: espan(node),
            name: sym(pool, &inner.name),
            name_loc: span(&inner.name_l),
        },
        Mri::Kwargs(inner) => lower_kwargs(&inner.pairs, espan(node), pool),
        Mri::KwBegin(inner) => lower_kwbegin(inner, node, pool),
        Mri::Kwnilarg(inner) => Node::NoKeywordsParameterNode {
            flags: 0,
            span: espan(node),
            operator_loc: Span {
                start: espan(node).start,
                end: span(&inner.name_l).start,
            },
            keyword_loc: span(&inner.name_l),
        },
        Mri::Kwoptarg(inner) => Node::OptionalKeywordParameterNode {
            flags: 0,
            span: espan(node),
            name: sym(pool, &inner.name),
            name_loc: span(&inner.name_l),
            value: Box::new(conv(&inner.default, pool)),
        },
        Mri::Kwrestarg(inner) => Node::KeywordRestParameterNode {
            flags: 0,
            span: espan(node),
            name: inner.name.as_deref().map(|name| sym(pool, name)),
            name_loc: ospan(&inner.name_l),
            operator_loc: span(&inner.operator_l),
        },
        Mri::Kwsplat(inner) => Node::AssocSplatNode {
            flags: 0,
            span: espan(node),
            value: Some(Box::new(conv(&inner.value, pool))),
            operator_loc: span(&inner.operator_l),
        },
        Mri::Lambda(_) => Node::LambdaNode {
            flags: 0,
            span: espan(node),
            locals: Vec::new(),
            operator_loc: espan(node),
            opening_loc: espan(node),
            closing_loc: espan(node),
            parameters: None,
            body: None,
        },
        Mri::Line(_) => Node::SourceLineNode {
            flags: 0,
            span: espan(node),
        },
        Mri::Lvar(inner) => Node::LocalVariableReadNode {
            flags: 0,
            span: espan(node),
            name: sym(pool, &inner.name),
            depth: 0,
        },
        Mri::Lvasgn(inner) => match &inner.value {
            Some(value) => Node::LocalVariableWriteNode {
                flags: 0,
                span: espan(node),
                name: sym(pool, &inner.name),
                depth: 0,
                name_loc: span(&inner.name_l),
                value: Box::new(conv(value, pool)),
                operator_loc: reqspan(&inner.operator_l, span(&inner.name_l)),
            },
            None => Node::LocalVariableReadNode {
                flags: 0,
                span: espan(node),
                name: sym(pool, &inner.name),
                depth: 0,
            },
        },
        Mri::Masgn(inner) => lower_masgn(inner, node, pool),
        Mri::MatchAlt(inner) => Node::AlternationPatternNode {
            flags: 0,
            span: espan(node),
            left: Box::new(conv(&inner.lhs, pool)),
            right: Box::new(conv(&inner.rhs, pool)),
            operator_loc: span(&inner.operator_l),
        },
        Mri::MatchAs(inner) => Node::CapturePatternNode {
            flags: 0,
            span: espan(node),
            value: Box::new(conv(&inner.value, pool)),
            target: Box::new(target(&inner.as_, pool)),
            operator_loc: span(&inner.operator_l),
        },
        Mri::MatchCurrentLine(inner) => lower_match_current_line(&inner.re, node, pool),
        Mri::MatchNilPattern(inner) => Node::NoKeywordsParameterNode {
            flags: 0,
            span: espan(node),
            operator_loc: span(&inner.operator_l),
            keyword_loc: span(&inner.name_l),
        },
        Mri::MatchPattern(inner) => Node::MatchRequiredNode {
            flags: 0,
            span: espan(node),
            value: Box::new(conv(&inner.value, pool)),
            pattern: Box::new(conv(&inner.pattern, pool)),
            operator_loc: span(&inner.operator_l),
        },
        Mri::MatchPatternP(inner) => Node::MatchPredicateNode {
            flags: 0,
            span: espan(node),
            value: Box::new(conv(&inner.value, pool)),
            pattern: Box::new(conv(&inner.pattern, pool)),
            operator_loc: span(&inner.operator_l),
        },
        Mri::MatchRest(inner) => Node::SplatNode {
            flags: 0,
            span: espan(node),
            operator_loc: span(&inner.operator_l),
            expression: inner
                .name
                .as_deref()
                .map(|name| Box::new(target(name, pool))),
        },
        Mri::MatchVar(inner) => Node::LocalVariableTargetNode {
            flags: 0,
            span: espan(node),
            name: sym(pool, &inner.name),
            depth: 0,
        },
        Mri::MatchWithLvasgn(inner) => {
            let whole = espan(node);
            let receiver = conv(&inner.re, pool);
            let arg = conv(&inner.value, pool);
            let arg_span = espan(&inner.value);
            Node::MatchWriteNode {
                flags: 0,
                span: whole,
                call: Box::new(Node::CallNode {
                    flags: 0,
                    span: whole,
                    receiver: Some(Box::new(receiver)),
                    call_operator_loc: None,
                    name: sym(pool, "=~"),
                    message_loc: Some(span(&inner.operator_l)),
                    opening_loc: None,
                    arguments: Some(Box::new(Node::ArgumentsNode {
                        flags: 0,
                        span: arg_span,
                        arguments: vec![arg],
                    })),
                    closing_loc: None,
                    equal_loc: None,
                    block: None,
                }),
                targets: Vec::new(),
            }
        }
        Mri::Mlhs(inner) => {
            lower_mlhs_target(&inner.items, &inner.begin_l, &inner.end_l, node, pool)
        }
        Mri::Module(inner) => Node::ModuleNode {
            flags: 0,
            span: espan(node),
            locals: Vec::new(),
            module_keyword_loc: span(&inner.keyword_l),
            constant_path: Box::new(conv(&inner.name, pool)),
            body: body_of(inner.body.as_deref(), pool),
            end_keyword_loc: span(&inner.end_l),
            name: const_name(&inner.name, pool),
        },
        Mri::Next(inner) => Node::NextNode {
            flags: 0,
            span: espan(node),
            arguments: call_args(&inner.args, pool),
            keyword_loc: span(&inner.keyword_l),
        },
        Mri::Nil(_) => Node::NilNode {
            flags: 0,
            span: espan(node),
        },
        Mri::NthRef(inner) => Node::NumberedReferenceReadNode {
            flags: 0,
            span: espan(node),
            number: inner.name.parse().unwrap_or(0),
        },
        Mri::Numblock(inner) => lower_numblock(inner, node, pool),
        Mri::OpAsgn(inner) => lower_op_write(
            &inner.recv,
            &inner.operator,
            &inner.operator_l,
            conv(&inner.value, pool),
            node,
            pool,
        ),
        Mri::Optarg(inner) => Node::OptionalParameterNode {
            flags: 0,
            span: espan(node),
            name: sym(pool, &inner.name),
            name_loc: span(&inner.name_l),
            operator_loc: span(&inner.operator_l),
            value: Box::new(conv(&inner.default, pool)),
        },
        Mri::Or(inner) => Node::OrNode {
            flags: 0,
            span: espan(node),
            left: Box::new(conv(&inner.lhs, pool)),
            right: Box::new(conv(&inner.rhs, pool)),
            operator_loc: span(&inner.operator_l),
        },
        Mri::OrAsgn(inner) => lower_logic_write(
            &inner.recv,
            false,
            &inner.operator_l,
            conv(&inner.value, pool),
            node,
            pool,
        ),
        Mri::Pair(inner) => Node::AssocNode {
            flags: 0,
            span: espan(node),
            key: Box::new(conv(&inner.key, pool)),
            value: Box::new(conv(&inner.value, pool)),
            operator_loc: Some(span(&inner.operator_l)),
        },
        Mri::Pin(inner) => lower_pin(inner, node, pool),
        Mri::Postexe(inner) => Node::PostExecutionNode {
            flags: 0,
            span: espan(node),
            statements: body_of(inner.body.as_deref(), pool),
            keyword_loc: span(&inner.keyword_l),
            opening_loc: span(&inner.begin_l),
            closing_loc: span(&inner.end_l),
        },
        Mri::Preexe(inner) => Node::PreExecutionNode {
            flags: 0,
            span: espan(node),
            statements: body_of(inner.body.as_deref(), pool),
            keyword_loc: span(&inner.keyword_l),
            opening_loc: span(&inner.begin_l),
            closing_loc: span(&inner.end_l),
        },
        Mri::Procarg0(inner) => {
            lower_destructured(&inner.args, &inner.begin_l, &inner.end_l, node, pool)
        }
        Mri::Rational(inner) => {
            let (numerator, denominator) = parse_rational(&inner.value);
            Node::RationalNode {
                flags: 0,
                span: espan(node),
                numerator,
                denominator,
            }
        }
        Mri::Redo(_) => Node::RedoNode {
            flags: 0,
            span: espan(node),
        },
        Mri::Regexp(inner) => lower_regexp(inner, node, pool),
        Mri::RegOpt(_) => Node::MissingNode {
            flags: 0,
            span: espan(node),
        },
        Mri::Rescue(inner) => lower_rescue_standalone(inner, node, pool),
        Mri::RescueBody(inner) => lower_rescue_body_node(inner, None, espan(node), pool),
        Mri::Restarg(inner) => Node::RestParameterNode {
            flags: 0,
            span: espan(node),
            name: inner.name.as_deref().map(|name| sym(pool, name)),
            name_loc: ospan(&inner.name_l),
            operator_loc: span(&inner.operator_l),
        },
        Mri::Retry(_) => Node::RetryNode {
            flags: 0,
            span: espan(node),
        },
        Mri::Return(inner) => Node::ReturnNode {
            flags: 0,
            span: espan(node),
            keyword_loc: span(&inner.keyword_l),
            arguments: call_args(&inner.args, pool),
        },
        Mri::SClass(inner) => Node::SingletonClassNode {
            flags: 0,
            span: espan(node),
            locals: Vec::new(),
            class_keyword_loc: span(&inner.keyword_l),
            operator_loc: span(&inner.operator_l),
            expression: Box::new(conv(&inner.expr, pool)),
            body: body_of(inner.body.as_deref(), pool),
            end_keyword_loc: span(&inner.end_l),
        },
        Mri::Self_(_) => Node::SelfNode {
            flags: 0,
            span: espan(node),
        },
        Mri::Send(inner) => lower_send_parts(
            inner.recv.as_deref(),
            &inner.method_name,
            &inner.args,
            inner.dot_l.as_ref().map(span),
            inner.selector_l.as_ref(),
            inner.begin_l.as_ref(),
            inner.end_l.as_ref(),
            inner.operator_l.as_ref(),
            None,
            espan(node),
            0,
            pool,
        ),
        Mri::Shadowarg(inner) => Node::BlockLocalVariableNode {
            flags: 0,
            span: espan(node),
            name: sym(pool, &inner.name),
        },
        Mri::Splat(inner) => Node::SplatNode {
            flags: 0,
            span: espan(node),
            operator_loc: span(&inner.operator_l),
            expression: inner.value.as_deref().map(|v| Box::new(conv(v, pool))),
        },
        Mri::Str(inner) => {
            let span = espan(node);
            Node::StringNode {
                flags: 0,
                span,
                opening_loc: ospan(&inner.begin_l),
                content_loc: str_content(&inner.begin_l, &inner.end_l, span),
                closing_loc: ospan(&inner.end_l),
                unescaped: inner.value.raw.clone(),
            }
        }
        Mri::Super(inner) => {
            let (args, block) = split_block_pass(&inner.args, pool);
            Node::SuperNode {
                flags: 0,
                span: espan(node),
                keyword_loc: span(&inner.keyword_l),
                lparen_loc: ospan(&inner.begin_l),
                arguments: call_args(args, pool),
                rparen_loc: ospan(&inner.end_l),
                block,
            }
        }
        Mri::Sym(inner) => {
            let span = espan(node);
            Node::SymbolNode {
                flags: 0,
                span,
                opening_loc: ospan(&inner.begin_l),
                value_loc: Some(str_content(&inner.begin_l, &inner.end_l, span)),
                closing_loc: ospan(&inner.end_l),
                unescaped: inner.name.raw.clone(),
            }
        }
        Mri::True(_) => Node::TrueNode {
            flags: 0,
            span: espan(node),
        },
        Mri::Undef(inner) => Node::UndefNode {
            flags: 0,
            span: espan(node),
            names: inner.names.iter().map(|n| conv(n, pool)).collect(),
            keyword_loc: span(&inner.keyword_l),
        },
        Mri::UnlessGuard(inner) => lower_guard(false, &inner.cond, &inner.keyword_l, node, pool),
        Mri::Until(inner) => lower_loop(
            false,
            &inner.cond,
            inner.body.as_deref(),
            &inner.keyword_l,
            inner.begin_l.as_ref(),
            inner.end_l.as_ref(),
            node,
            pool,
        ),
        Mri::UntilPost(inner) => lower_loop(
            false,
            &inner.cond,
            Some(&inner.body),
            &inner.keyword_l,
            None,
            None,
            node,
            pool,
        ),
        Mri::When(inner) => Node::WhenNode {
            flags: 0,
            span: espan(node),
            keyword_loc: span(&inner.keyword_l),
            conditions: inner.patterns.iter().map(|p| conv(p, pool)).collect(),
            then_keyword_loc: then_loc(&inner.begin_l),
            statements: body_of(inner.body.as_deref(), pool),
        },
        Mri::While(inner) => lower_loop(
            true,
            &inner.cond,
            inner.body.as_deref(),
            &inner.keyword_l,
            inner.begin_l.as_ref(),
            inner.end_l.as_ref(),
            node,
            pool,
        ),
        Mri::WhilePost(inner) => lower_loop(
            true,
            &inner.cond,
            Some(&inner.body),
            &inner.keyword_l,
            None,
            None,
            node,
            pool,
        ),
        Mri::XHeredoc(inner) => lower_xheredoc(
            &inner.parts,
            &inner.heredoc_body_l,
            &inner.heredoc_end_l,
            node,
            pool,
        ),
        Mri::Xstr(inner) => {
            let whole = espan(node);
            if parts_all_plain(&inner.parts) {
                Node::XStringNode {
                    flags: 0,
                    span: whole,
                    opening_loc: span(&inner.begin_l),
                    content_loc: parts_span(&inner.parts, whole),
                    closing_loc: span(&inner.end_l),
                    unescaped: concat_str_bytes(&inner.parts),
                }
            } else {
                Node::InterpolatedXStringNode {
                    flags: 0,
                    span: whole,
                    opening_loc: span(&inner.begin_l),
                    parts: inner.parts.iter().map(|p| interp_part(p, pool)).collect(),
                    closing_loc: span(&inner.end_l),
                }
            }
        }
        Mri::Yield(inner) => Node::YieldNode {
            flags: 0,
            span: espan(node),
            keyword_loc: span(&inner.keyword_l),
            lparen_loc: ospan(&inner.begin_l),
            arguments: call_args(&inner.args, pool),
            rparen_loc: ospan(&inner.end_l),
        },
        Mri::ZSuper(_) => Node::ForwardingSuperNode {
            flags: 0,
            span: espan(node),
            block: None,
        },
    }
}

/// `...` range flag.
fn range_flags_exclude() -> u16 {
    carnelian_ast::range_flags::EXCLUDE_END
}

/// Content span of a quoted scalar: after the opener, before the closer.
fn str_content(begin_l: &Option<Loc>, end_l: &Option<Loc>, span: Span) -> Span {
    let start = begin_l.as_ref().map_or(span.start, |loc| {
        u32::try_from(loc.end).unwrap_or(span.start)
    });
    let end = end_l
        .as_ref()
        .map_or(span.end, |loc| u32::try_from(loc.begin).unwrap_or(span.end));
    Span {
        start: start.min(end),
        end,
    }
}

/// Span covering interpolation parts, falling back to the enclosing span.
fn parts_span(parts: &[Mri], fallback: Span) -> Span {
    match (parts.first(), parts.last()) {
        (Some(first), Some(last)) => Span {
            start: espan(first).start,
            end: espan(last).end,
        },
        _ => fallback,
    }
}

/// True when every part is a plain `Str` (no interpolation).
fn parts_all_plain(parts: &[Mri]) -> bool {
    parts.iter().all(|part| matches!(part, Mri::Str(_)))
}

/// Concatenated bytes of plain `Str` parts.
fn concat_str_bytes(parts: &[Mri]) -> Vec<u8> {
    let mut out = Vec::new();
    for part in parts {
        if let Mri::Str(inner) = part {
            out.extend_from_slice(&inner.value.raw);
        }
    }
    out
}

/// Lower call arguments, computing the `ArgumentsNode` presence flags.
fn lower_call_args(items: &[Mri], pool: &mut Pool) -> (Vec<Node>, u16) {
    let mut out = Vec::with_capacity(items.len());
    let mut flags = 0;
    let mut splats = 0;
    for item in items {
        match item {
            Mri::Kwargs(inner) => {
                flags |= arguments_node_flags::CONTAINS_KEYWORDS;
                if inner
                    .pairs
                    .iter()
                    .any(|pair| matches!(pair, Mri::Kwsplat(_)))
                {
                    flags |= arguments_node_flags::CONTAINS_KEYWORD_SPLAT;
                }
                out.push(lower_kwargs(&inner.pairs, espan(item), pool));
            }
            Mri::Kwsplat(inner) => {
                flags |= arguments_node_flags::CONTAINS_KEYWORDS
                    | arguments_node_flags::CONTAINS_KEYWORD_SPLAT;
                let whole = espan(item);
                out.push(Node::KeywordHashNode {
                    flags: 0,
                    span: whole,
                    elements: vec![Node::AssocSplatNode {
                        flags: 0,
                        span: whole,
                        value: Some(Box::new(conv(&inner.value, pool))),
                        operator_loc: span(&inner.operator_l),
                    }],
                });
            }
            Mri::Splat(inner) => {
                flags |= arguments_node_flags::CONTAINS_SPLAT;
                splats += 1;
                out.push(Node::SplatNode {
                    flags: 0,
                    span: espan(item),
                    operator_loc: span(&inner.operator_l),
                    expression: inner.value.as_deref().map(|v| Box::new(conv(v, pool))),
                });
            }
            Mri::BlockPass(inner) => out.push(Node::BlockArgumentNode {
                flags: 0,
                span: espan(item),
                expression: inner.value.as_deref().map(|v| Box::new(conv(v, pool))),
                operator_loc: span(&inner.operator_l),
            }),
            Mri::ForwardedArgs(inner) => {
                flags |= arguments_node_flags::CONTAINS_FORWARDING;
                out.push(Node::ForwardingArgumentsNode {
                    flags: 0,
                    span: span(&inner.expression_l),
                });
            }
            Mri::ForwardArg(inner) => {
                flags |= arguments_node_flags::CONTAINS_FORWARDING;
                out.push(Node::ForwardingArgumentsNode {
                    flags: 0,
                    span: span(&inner.expression_l),
                });
            }
            other => out.push(conv(other, pool)),
        }
    }
    if splats > 1 {
        flags |= arguments_node_flags::CONTAINS_MULTIPLE_SPLATS;
    }
    (out, flags)
}

/// Optional `ArgumentsNode` for a call argument list (empty means absent).
fn call_args(items: &[Mri], pool: &mut Pool) -> Option<Box<Node>> {
    if items.is_empty() {
        return None;
    }
    let span = Span {
        start: espan(&items[0]).start,
        end: espan(&items[items.len() - 1]).end,
    };
    let (arguments, flags) = lower_call_args(items, pool);
    Some(Box::new(Node::ArgumentsNode {
        flags,
        span,
        arguments,
    }))
}

/// Trailing `&block` of an argument list becomes the call's `block` field
/// (Prism models it outside `ArgumentsNode`); anything else stays inline.
fn split_block_pass<'a>(args: &'a [Mri], pool: &mut Pool) -> (&'a [Mri], Option<Box<Node>>) {
    match args.last() {
        Some(Mri::BlockPass(inner)) => {
            let block = Node::BlockArgumentNode {
                flags: 0,
                span: espan(&args[args.len() - 1]),
                expression: inner.value.as_deref().map(|v| Box::new(conv(v, pool))),
                operator_loc: span(&inner.operator_l),
            };
            (&args[..args.len() - 1], Some(Box::new(block)))
        }
        _ => (args, None),
    }
}

/// Setter call: `=` operator with a trailing-`=` method name.
fn is_setter(name: &str, operator_l: Option<&Loc>) -> bool {
    operator_l.is_some() && name.len() > 1 && name.as_bytes()[name.len() - 1] == b'='
}

/// Shared `Send`/`CSend` lowering to `CallNode`.
#[allow(clippy::too_many_arguments)]
fn lower_send_parts(
    recv: Option<&Mri>,
    name: &str,
    args: &[Mri],
    dot: Option<Span>,
    selector_l: Option<&Loc>,
    begin_l: Option<&Loc>,
    end_l: Option<&Loc>,
    operator_l: Option<&Loc>,
    block: Option<Box<Node>>,
    whole: Span,
    extra_flags: u16,
    pool: &mut Pool,
) -> Node {
    // A literal block wins; otherwise a trailing `&block` fills the field.
    let (args, block) = match block {
        Some(_) => (args, block),
        None => {
            let (rest, pass) = split_block_pass(args, pool);
            (rest, pass)
        }
    };
    let setter = is_setter(name, operator_l);
    let mut flags = extra_flags;
    let (method, message_loc, equal_loc) = if setter {
        flags |= call_node_flags::ATTRIBUTE_WRITE;
        // Prism keeps the `=` on the name (`bar=`), flagged as a write.
        (name, ospan(&selector_l.copied()), operator_l.map(span))
    } else {
        if recv.is_none() && args.is_empty() && begin_l.is_none() && operator_l.is_none() {
            flags |= call_node_flags::VARIABLE_CALL;
        }
        (name, ospan(&selector_l.copied()), None)
    };
    Node::CallNode {
        flags,
        span: whole,
        receiver: recv.map(|recv| Box::new(conv(recv, pool))),
        call_operator_loc: dot,
        name: sym(pool, method),
        message_loc,
        opening_loc: ospan(&begin_l.copied()),
        arguments: call_args(args, pool),
        closing_loc: ospan(&end_l.copied()),
        equal_loc,
        block,
    }
}

/// `foo[args]` reads as `[]` calls.
fn index_read(
    recv: &Mri,
    indexes: &[Mri],
    begin_l: &Loc,
    end_l: &Loc,
    node: &Mri,
    pool: &mut Pool,
) -> Node {
    let whole = espan(node);
    let (indexes, block) = split_block_pass(indexes, pool);
    Node::CallNode {
        flags: 0,
        span: whole,
        receiver: Some(Box::new(conv(recv, pool))),
        call_operator_loc: None,
        name: sym(pool, "[]"),
        message_loc: Some(Span {
            start: u32::try_from(begin_l.begin).unwrap_or(u32::MAX),
            end: u32::try_from(end_l.end).unwrap_or(u32::MAX),
        }),
        opening_loc: Some(span(begin_l)),
        arguments: call_args(indexes, pool),
        closing_loc: Some(span(end_l)),
        equal_loc: None,
        block,
    }
}

/// `foo[args] = value` writes as `[]=` calls.
#[allow(clippy::too_many_arguments)]
fn index_write(
    recv: &Mri,
    indexes: &[Mri],
    begin_l: &Loc,
    end_l: &Loc,
    operator_l: Option<&Loc>,
    value: Node,
    node: &Mri,
    pool: &mut Pool,
) -> Node {
    let whole = espan(node);
    let mut arguments: Vec<Node> = indexes.iter().map(|index| conv(index, pool)).collect();
    let list_span = Span {
        start: arguments
            .first()
            .map(Node::span)
            .map_or(whole.start, |s| s.start),
        end: value.span().end.max(
            arguments
                .last()
                .map(Node::span)
                .map_or(whole.end, |s| s.end),
        ),
    };
    arguments.push(value);
    Node::CallNode {
        flags: call_node_flags::ATTRIBUTE_WRITE,
        span: whole,
        receiver: Some(Box::new(conv(recv, pool))),
        call_operator_loc: None,
        name: sym(pool, "[]="),
        message_loc: Some(Span {
            start: u32::try_from(begin_l.begin).unwrap_or(u32::MAX),
            end: u32::try_from(end_l.end).unwrap_or(u32::MAX),
        }),
        opening_loc: Some(span(begin_l)),
        arguments: Some(Box::new(Node::ArgumentsNode {
            flags: 0,
            span: list_span,
            arguments,
        })),
        closing_loc: Some(span(end_l)),
        equal_loc: operator_l.map(span),
        block: None,
    }
}

/// Block or lambda parameters: `BlockParametersNode` when pipes are present.
fn block_params(args: Option<&Mri>, pool: &mut Pool) -> Option<Box<Node>> {
    let inner = args?;
    match inner {
        Mri::Args(args) => {
            let mut shadows = Vec::new();
            let slots = build_params(&args.args, pool, &mut shadows);
            let parameters = if slots.is_empty() {
                None
            } else {
                Some(params_box(slots, span(&args.expression_l)))
            };
            Some(Box::new(Node::BlockParametersNode {
                flags: 0,
                span: span(&args.expression_l),
                parameters,
                locals: shadows,
                opening_loc: ospan(&args.begin_l),
                closing_loc: ospan(&args.end_l),
            }))
        }
        // Destructured-only pipes still nest under block parameters.
        Mri::Procarg0(_) | Mri::Mlhs(_) => Some(Box::new(Node::BlockParametersNode {
            flags: 0,
            span: espan(inner),
            parameters: Some(params_box(single_required(conv(inner, pool)), espan(inner))),
            locals: Vec::new(),
            opening_loc: None,
            closing_loc: None,
        })),
        other => Some(Box::new(Node::BlockParametersNode {
            flags: 0,
            span: espan(other),
            parameters: Some(params_box(single_required(conv(other, pool)), espan(other))),
            locals: Vec::new(),
            opening_loc: None,
            closing_loc: None,
        })),
    }
}

/// `Block{call}` lowering; blocks attach to calls, `super`, or lambdas.
fn lower_block(inner: &Block, node: &Mri, pool: &mut Pool) -> Node {
    let whole = espan(node);
    let params = block_params(inner.args.as_deref(), pool);
    let body = body_of(inner.body.as_deref(), pool);
    match &*inner.call {
        Mri::Lambda(lam) => Node::LambdaNode {
            flags: 0,
            span: whole,
            locals: Vec::new(),
            operator_loc: span(&lam.expression_l),
            opening_loc: span(&inner.begin_l),
            closing_loc: span(&inner.end_l),
            parameters: params,
            body,
        },
        Mri::Send(send) => lower_send_parts(
            send.recv.as_deref(),
            &send.method_name,
            &send.args,
            send.dot_l.as_ref().map(span),
            send.selector_l.as_ref(),
            send.begin_l.as_ref(),
            send.end_l.as_ref(),
            send.operator_l.as_ref(),
            Some(block_node(params, body, inner, whole)),
            whole,
            0,
            pool,
        ),
        Mri::CSend(send) => lower_send_parts(
            Some(&send.recv),
            &send.method_name,
            &send.args,
            Some(span(&send.dot_l)),
            send.selector_l.as_ref(),
            send.begin_l.as_ref(),
            send.end_l.as_ref(),
            send.operator_l.as_ref(),
            Some(block_node(params, body, inner, whole)),
            whole,
            call_node_flags::SAFE_NAVIGATION,
            pool,
        ),
        Mri::Super(send) => Node::SuperNode {
            flags: 0,
            span: whole,
            keyword_loc: span(&send.keyword_l),
            lparen_loc: ospan(&send.begin_l),
            arguments: call_args(&send.args, pool),
            rparen_loc: ospan(&send.end_l),
            block: Some(block_node(params, body, inner, whole)),
        },
        Mri::ZSuper(zsuper) => Node::ForwardingSuperNode {
            flags: 0,
            span: span(&zsuper.expression_l),
            block: Some(block_node(params, body, inner, whole)),
        },
        // Unreachable in valid trees: the block is dropped, the call kept.
        other => conv(other, pool),
    }
}

/// Shared `BlockNode` shell for an attached block.
fn block_node(
    params: Option<Box<Node>>,
    body: Option<Box<Node>>,
    inner: &Block,
    whole: Span,
) -> Box<Node> {
    Box::new(Node::BlockNode {
        flags: 0,
        span: whole,
        locals: Vec::new(),
        parameters: params,
        body,
        opening_loc: span(&inner.begin_l),
        closing_loc: span(&inner.end_l),
    })
}

/// Positional/keyword slots collected from a flat MRI `Args` list.
#[derive(Default)]
struct ParamSlots {
    requireds: Vec<Node>,
    optionals: Vec<Node>,
    rest: Option<Box<Node>>,
    posts: Vec<Node>,
    keywords: Vec<Node>,
    keyword_rest: Option<Box<Node>>,
    block: Option<Box<Node>>,
}

impl ParamSlots {
    /// True when no parameter slot is filled.
    fn is_empty(&self) -> bool {
        self.requireds.is_empty()
            && self.optionals.is_empty()
            && self.rest.is_none()
            && self.posts.is_empty()
            && self.keywords.is_empty()
            && self.keyword_rest.is_none()
            && self.block.is_none()
    }
}

/// Collect MRI argument items into owned parameter slots. Required items
/// after a rest land in `posts`; `;`-locals accumulate separately.
fn build_params(items: &[Mri], pool: &mut Pool, shadows: &mut Vec<Node>) -> ParamSlots {
    build_params_refs(&items.iter().collect::<Vec<&Mri>>(), pool, shadows)
}

/// Splice paren-less `Procarg0` (`|x|` block pipes) into the item list;
/// parenthesized destructuring stays a `MultiTargetNode` item.
fn build_params_refs(items: &[&Mri], pool: &mut Pool, shadows: &mut Vec<Node>) -> ParamSlots {
    let mut expanded: Vec<&Mri> = Vec::with_capacity(items.len());
    for item in items {
        match item {
            Mri::Procarg0(inner) if inner.begin_l.is_none() => {
                for nested in &inner.args {
                    expanded.push(nested);
                }
            }
            other => expanded.push(other),
        }
    }
    let mut slots = ParamSlots::default();
    let mut seen_rest = false;
    for item in expanded {
        match item {
            Mri::Arg(inner) => {
                let param = Node::RequiredParameterNode {
                    flags: 0,
                    span: espan(item),
                    name: sym(pool, &inner.name),
                };
                if seen_rest {
                    slots.posts.push(param);
                } else {
                    slots.requireds.push(param);
                }
            }
            Mri::Optarg(inner) => slots.optionals.push(Node::OptionalParameterNode {
                flags: 0,
                span: espan(item),
                name: sym(pool, &inner.name),
                name_loc: span(&inner.name_l),
                operator_loc: span(&inner.operator_l),
                value: Box::new(conv(&inner.default, pool)),
            }),
            Mri::Restarg(inner) => {
                seen_rest = true;
                slots.rest = Some(Box::new(Node::RestParameterNode {
                    flags: 0,
                    span: espan(item),
                    name: inner.name.as_deref().map(|name| sym(pool, name)),
                    name_loc: ospan(&inner.name_l),
                    operator_loc: span(&inner.operator_l),
                }));
            }
            Mri::Kwarg(inner) => slots.keywords.push(Node::RequiredKeywordParameterNode {
                flags: 0,
                span: espan(item),
                name: sym(pool, &inner.name),
                name_loc: span(&inner.name_l),
            }),
            Mri::Kwoptarg(inner) => slots.keywords.push(Node::OptionalKeywordParameterNode {
                flags: 0,
                span: espan(item),
                name: sym(pool, &inner.name),
                name_loc: span(&inner.name_l),
                value: Box::new(conv(&inner.default, pool)),
            }),
            Mri::Kwrestarg(inner) => {
                slots.keyword_rest = Some(Box::new(Node::KeywordRestParameterNode {
                    flags: 0,
                    span: espan(item),
                    name: inner.name.as_deref().map(|name| sym(pool, name)),
                    name_loc: ospan(&inner.name_l),
                    operator_loc: span(&inner.operator_l),
                }));
            }
            Mri::Kwnilarg(inner) => {
                slots.keyword_rest = Some(Box::new(Node::NoKeywordsParameterNode {
                    flags: 0,
                    span: espan(item),
                    operator_loc: Span {
                        start: espan(item).start,
                        end: span(&inner.name_l).start,
                    },
                    keyword_loc: span(&inner.name_l),
                }));
            }
            Mri::Blockarg(inner) => {
                slots.block = Some(Box::new(Node::BlockParameterNode {
                    flags: 0,
                    span: espan(item),
                    name: inner.name.as_deref().map(|name| sym(pool, name)),
                    name_loc: ospan(&inner.name_l),
                    operator_loc: span(&inner.operator_l),
                }));
            }
            Mri::ForwardArg(inner) => {
                slots.keyword_rest = Some(Box::new(Node::ForwardingParameterNode {
                    flags: 0,
                    span: span(&inner.expression_l),
                }));
            }
            Mri::Procarg0(inner) => {
                let target =
                    lower_destructured(&inner.args, &inner.begin_l, &inner.end_l, item, pool);
                if seen_rest {
                    slots.posts.push(target);
                } else {
                    slots.requireds.push(target);
                }
            }
            Mri::Mlhs(inner) => {
                let target =
                    lower_mlhs_target(&inner.items, &inner.begin_l, &inner.end_l, item, pool);
                if seen_rest {
                    slots.posts.push(target);
                } else {
                    slots.requireds.push(target);
                }
            }
            Mri::Shadowarg(inner) => shadows.push(Node::BlockLocalVariableNode {
                flags: 0,
                span: espan(item),
                name: sym(pool, &inner.name),
            }),
            // Unreachable in valid trees: keep the node as a required slot.
            other => {
                let param = conv(other, pool);
                if seen_rest {
                    slots.posts.push(param);
                } else {
                    slots.requireds.push(param);
                }
            }
        }
    }
    slots
}

/// `ParametersNode` from collected slots.
fn params_node(slots: ParamSlots, span: Span) -> Node {
    Node::ParametersNode {
        flags: 0,
        span,
        requireds: slots.requireds,
        optionals: slots.optionals,
        rest: slots.rest,
        posts: slots.posts,
        keywords: slots.keywords,
        keyword_rest: slots.keyword_rest,
        block: slots.block,
    }
}

/// Boxed `ParametersNode` from collected slots.
fn params_box(slots: ParamSlots, span: Span) -> Box<Node> {
    Box::new(params_node(slots, span))
}

/// Slots holding a single required node (defensive positions).
fn single_required(node: Node) -> ParamSlots {
    ParamSlots {
        requireds: vec![node],
        ..ParamSlots::default()
    }
}

/// Destructured parameter (`|(a, b)|`, `def f((a, b))`) to `MultiTargetNode`.
fn lower_destructured(
    items: &[Mri],
    begin_l: &Option<Loc>,
    end_l: &Option<Loc>,
    node: &Mri,
    pool: &mut Pool,
) -> Node {
    // A rest item fills the `rest` field (like `split_mlhs`), never `lefts`.
    let at = items
        .iter()
        .position(|item| matches!(item, Mri::Restarg(_)));
    let (lefts, rest, rights) = match at {
        None => (
            items
                .iter()
                .map(|item| destructure_item(item, pool))
                .collect(),
            None,
            Vec::new(),
        ),
        Some(index) => {
            let lefts = items[..index]
                .iter()
                .map(|item| destructure_item(item, pool))
                .collect();
            let rest = match &items[index] {
                Mri::Restarg(inner) => Some(Box::new(Node::SplatNode {
                    flags: 0,
                    span: espan(&items[index]),
                    operator_loc: span(&inner.operator_l),
                    expression: inner.name.as_deref().map(|name| {
                        Box::new(Node::RequiredParameterNode {
                            flags: 0,
                            span: ospan(&inner.name_l).unwrap_or_else(|| espan(&items[index])),
                            name: sym(pool, name),
                        })
                    }),
                })),
                _ => None,
            };
            let rights = items[index + 1..]
                .iter()
                .map(|item| destructure_item(item, pool))
                .collect();
            (lefts, rest, rights)
        }
    };
    Node::MultiTargetNode {
        flags: 0,
        span: espan(node),
        lefts,
        rest,
        rights,
        lparen_loc: ospan(begin_l),
        rparen_loc: ospan(end_l),
    }
}

/// One item of a destructured parameter list.
fn destructure_item(item: &Mri, pool: &mut Pool) -> Node {
    match item {
        Mri::Arg(inner) => Node::RequiredParameterNode {
            flags: 0,
            span: espan(item),
            name: sym(pool, &inner.name),
        },
        Mri::Mlhs(inner) => {
            lower_mlhs_target(&inner.items, &inner.begin_l, &inner.end_l, item, pool)
        }
        Mri::Procarg0(inner) => {
            lower_destructured(&inner.args, &inner.begin_l, &inner.end_l, item, pool)
        }
        Mri::Optarg(inner) => Node::OptionalParameterNode {
            flags: 0,
            span: espan(item),
            name: sym(pool, &inner.name),
            name_loc: span(&inner.name_l),
            operator_loc: span(&inner.operator_l),
            value: Box::new(conv(&inner.default, pool)),
        },
        // Unreachable in valid trees: keep the lowered node as the part.
        other => conv(other, pool),
    }
}

/// `mlhs` items split around the splat into a `MultiTargetNode`.
fn lower_mlhs_target(
    items: &[Mri],
    begin_l: &Option<Loc>,
    end_l: &Option<Loc>,
    node: &Mri,
    pool: &mut Pool,
) -> Node {
    let (lefts, rest, rights) = split_mlhs(items, pool);
    Node::MultiTargetNode {
        flags: 0,
        span: espan(node),
        lefts,
        rest,
        rights,
        lparen_loc: ospan(begin_l),
        rparen_loc: ospan(end_l),
    }
}

/// Split assignment targets at the first splat (`*b` in masgn, `*b` as
/// `Restarg` in destructured parameters).
fn split_mlhs(items: &[Mri], pool: &mut Pool) -> (Vec<Node>, Option<Box<Node>>, Vec<Node>) {
    let at = items
        .iter()
        .position(|item| matches!(item, Mri::Splat(_) | Mri::Restarg(_)));
    match at {
        None => (
            items.iter().map(|item| target(item, pool)).collect(),
            None,
            Vec::new(),
        ),
        Some(index) => {
            let lefts = items[..index]
                .iter()
                .map(|item| target(item, pool))
                .collect();
            let rest = match &items[index] {
                Mri::Splat(inner) => Some(Box::new(Node::SplatNode {
                    flags: 0,
                    span: espan(&items[index]),
                    operator_loc: span(&inner.operator_l),
                    expression: inner
                        .value
                        .as_deref()
                        .map(|value| Box::new(target(value, pool))),
                })),
                // Destructured parameters name a plain parameter here.
                Mri::Restarg(inner) => Some(Box::new(Node::SplatNode {
                    flags: 0,
                    span: espan(&items[index]),
                    operator_loc: span(&inner.operator_l),
                    expression: inner.name.as_deref().map(|name| {
                        Box::new(Node::RequiredParameterNode {
                            flags: 0,
                            span: ospan(&inner.name_l).unwrap_or_else(|| espan(&items[index])),
                            name: sym(pool, name),
                        })
                    }),
                })),
                // Unreachable: position found a splat above.
                other => Some(Box::new(conv(other, pool))),
            };
            let rights = items[index + 1..]
                .iter()
                .map(|item| match item {
                    Mri::Splat(inner) => Node::SplatNode {
                        flags: 0,
                        span: espan(item),
                        operator_loc: span(&inner.operator_l),
                        expression: inner
                            .value
                            .as_deref()
                            .map(|value| Box::new(target(value, pool))),
                    },
                    other => target(other, pool),
                })
                .collect();
            (lefts, rest, rights)
        }
    }
}

/// Assignment target: writes become `*Target` nodes, `depth` blank.
fn target(node: &Mri, pool: &mut Pool) -> Node {
    match node {
        Mri::Lvasgn(inner) => Node::LocalVariableTargetNode {
            flags: 0,
            span: espan(node),
            name: sym(pool, &inner.name),
            depth: 0,
        },
        Mri::Lvar(inner) => Node::LocalVariableTargetNode {
            flags: 0,
            span: espan(node),
            name: sym(pool, &inner.name),
            depth: 0,
        },
        Mri::Ivasgn(inner) => Node::InstanceVariableTargetNode {
            flags: 0,
            span: espan(node),
            name: sym(pool, &inner.name),
        },
        Mri::Ivar(inner) => Node::InstanceVariableTargetNode {
            flags: 0,
            span: espan(node),
            name: sym(pool, &inner.name),
        },
        Mri::Cvasgn(inner) => Node::ClassVariableTargetNode {
            flags: 0,
            span: espan(node),
            name: sym(pool, &inner.name),
        },
        Mri::Cvar(inner) => Node::ClassVariableTargetNode {
            flags: 0,
            span: espan(node),
            name: sym(pool, &inner.name),
        },
        Mri::Gvasgn(inner) => Node::GlobalVariableTargetNode {
            flags: 0,
            span: espan(node),
            name: sym(pool, &inner.name),
        },
        Mri::Gvar(inner) => Node::GlobalVariableTargetNode {
            flags: 0,
            span: espan(node),
            name: sym(pool, &inner.name),
        },
        Mri::Casgn(inner) => match &inner.scope {
            None => Node::ConstantTargetNode {
                flags: 0,
                span: espan(node),
                name: sym(pool, &inner.name),
            },
            Some(scope) => Node::ConstantPathTargetNode {
                flags: 0,
                span: espan(node),
                parent: scope_parent(scope, pool),
                name: Some(sym(pool, &inner.name)),
                delimiter_loc: reqspan(&inner.double_colon_l, span(&inner.name_l)),
                name_loc: span(&inner.name_l),
            },
        },
        Mri::Const(inner) => match &inner.scope {
            None => Node::ConstantTargetNode {
                flags: 0,
                span: espan(node),
                name: sym(pool, &inner.name),
            },
            Some(scope) => Node::ConstantPathTargetNode {
                flags: 0,
                span: espan(node),
                parent: scope_parent(scope, pool),
                name: Some(sym(pool, &inner.name)),
                delimiter_loc: reqspan(&inner.double_colon_l, span(&inner.name_l)),
                name_loc: span(&inner.name_l),
            },
        },
        Mri::Send(inner) => {
            Node::CallTargetNode {
                flags: 0,
                span: espan(node),
                receiver: Box::new(inner.recv.as_deref().map_or_else(
                    || Node::MissingNode {
                        flags: 0,
                        span: espan(node),
                    },
                    |recv| conv(recv, pool),
                )),
                call_operator_loc: inner
                    .dot_l
                    .as_ref()
                    .map(span)
                    .unwrap_or_else(|| espan(node)),
                // Prism keeps the `=` suffix on write-target names.
                name: sym(pool, &inner.method_name),
                message_loc: ospan(&inner.selector_l).unwrap_or_else(|| espan(node)),
            }
        }
        Mri::Index(inner) => index_target(
            &inner.recv,
            &inner.indexes,
            &inner.begin_l,
            &inner.end_l,
            node,
            pool,
        ),
        Mri::IndexAsgn(inner) => index_target(
            &inner.recv,
            &inner.indexes,
            &inner.begin_l,
            &inner.end_l,
            node,
            pool,
        ),
        Mri::Mlhs(inner) => {
            lower_mlhs_target(&inner.items, &inner.begin_l, &inner.end_l, node, pool)
        }
        Mri::MatchVar(inner) => Node::LocalVariableTargetNode {
            flags: 0,
            span: espan(node),
            name: sym(pool, &inner.name),
            depth: 0,
        },
        Mri::Splat(inner) => Node::SplatNode {
            flags: 0,
            span: espan(node),
            operator_loc: span(&inner.operator_l),
            expression: inner
                .value
                .as_deref()
                .map(|value| Box::new(target(value, pool))),
        },
        // Unreachable in valid trees: keep the lowered node as the target.
        other => conv(other, pool),
    }
}

/// Index assignment target.
fn index_target(
    recv: &Mri,
    indexes: &[Mri],
    begin_l: &Loc,
    end_l: &Loc,
    node: &Mri,
    pool: &mut Pool,
) -> Node {
    Node::IndexTargetNode {
        flags: 0,
        span: espan(node),
        receiver: Box::new(conv(recv, pool)),
        opening_loc: span(begin_l),
        arguments: call_args(indexes, pool),
        closing_loc: span(end_l),
        block: None,
    }
}

/// `masgn` to `MultiWriteNode`, splitting the `mlhs` around the splat.
fn lower_masgn(inner: &Masgn, node: &Mri, pool: &mut Pool) -> Node {
    let whole = espan(node);
    let (lefts, rest, rights, lparen_loc, rparen_loc) = match &*inner.lhs {
        Mri::Mlhs(mlhs) => {
            let (lefts, rest, rights) = split_mlhs(&mlhs.items, pool);
            (
                lefts,
                rest,
                rights,
                ospan(&mlhs.begin_l),
                ospan(&mlhs.end_l),
            )
        }
        // Unreachable in valid trees: single target on the left.
        other => (vec![target(other, pool)], None, Vec::new(), None, None),
    };
    Node::MultiWriteNode {
        flags: 0,
        span: whole,
        lefts,
        rest,
        rights,
        lparen_loc,
        rparen_loc,
        operator_loc: span(&inner.operator_l),
        value: Box::new(conv(&inner.rhs, pool)),
    }
}

/// `Numblock` lowering: numbered parameters stay blank (`maximum` zero).
fn lower_numblock(inner: &Numblock, node: &Mri, pool: &mut Pool) -> Node {
    let whole = espan(node);
    let block = Box::new(Node::BlockNode {
        flags: 0,
        span: whole,
        locals: Vec::new(),
        parameters: Some(Box::new(Node::NumberedParametersNode {
            flags: 0,
            span: span(&inner.expression_l),
            maximum: 0,
        })),
        body: Some(seq_one(&inner.body, pool)),
        opening_loc: span(&inner.begin_l),
        closing_loc: span(&inner.end_l),
    });
    match &*inner.call {
        Mri::Send(send) => lower_send_parts(
            send.recv.as_deref(),
            &send.method_name,
            &send.args,
            send.dot_l.as_ref().map(span),
            send.selector_l.as_ref(),
            send.begin_l.as_ref(),
            send.end_l.as_ref(),
            send.operator_l.as_ref(),
            Some(block),
            whole,
            0,
            pool,
        ),
        Mri::CSend(send) => lower_send_parts(
            Some(&send.recv),
            &send.method_name,
            &send.args,
            Some(span(&send.dot_l)),
            send.selector_l.as_ref(),
            send.begin_l.as_ref(),
            send.end_l.as_ref(),
            send.operator_l.as_ref(),
            Some(block),
            whole,
            call_node_flags::SAFE_NAVIGATION,
            pool,
        ),
        // Unreachable in valid trees: numbered block on `super`/lambda.
        other => conv(other, pool),
    }
}

/// Shared `Def`/`Defs` lowering; `Defs` carries the receiver plus `.` loc.
#[allow(clippy::too_many_arguments)]
fn lower_def(
    name: &str,
    name_l: &Loc,
    receiver: Option<&Mri>,
    operator_loc: Option<Span>,
    args: Option<&Mri>,
    body: Option<&Mri>,
    keyword_l: &Loc,
    assignment_l: Option<&Loc>,
    end_l: Option<&Loc>,
    node: &Mri,
    pool: &mut Pool,
) -> Node {
    let (parameters, lparen_loc, rparen_loc) = match args {
        None => (None, None, None),
        Some(Mri::Args(inner)) => {
            let mut shadows = Vec::new();
            let slots = build_params(&inner.args, pool, &mut shadows);
            // `;`-locals cannot appear in method params; drop if ever present.
            drop(shadows);
            (
                Some(params_box(slots, span(&inner.expression_l))),
                ospan(&inner.begin_l),
                ospan(&inner.end_l),
            )
        }
        // Unreachable in valid trees: keep the node as whole parameters.
        Some(other) => {
            let span = espan(other);
            (
                Some(Box::new(Node::ParametersNode {
                    flags: 0,
                    span,
                    requireds: vec![conv(other, pool)],
                    optionals: Vec::new(),
                    rest: None,
                    posts: Vec::new(),
                    keywords: Vec::new(),
                    keyword_rest: None,
                    block: None,
                })),
                None,
                None,
            )
        }
    };
    Node::DefNode {
        flags: 0,
        span: espan(node),
        name: sym(pool, name),
        name_loc: span(name_l),
        receiver: receiver.map(|recv| Box::new(conv(recv, pool))),
        parameters,
        body: body_of(body, pool),
        locals: Vec::new(),
        def_keyword_loc: span(keyword_l),
        operator_loc,
        lparen_loc,
        rparen_loc,
        equal_loc: assignment_l.map(span),
        end_keyword_loc: end_l.map(span),
    }
}

/// Constant read or `::` path.
fn lower_const(inner: &Const, node: &Mri, pool: &mut Pool) -> Node {
    match &inner.scope {
        None => Node::ConstantReadNode {
            flags: 0,
            span: espan(node),
            name: sym(pool, &inner.name),
        },
        Some(scope) => Node::ConstantPathNode {
            flags: 0,
            span: espan(node),
            parent: scope_parent(scope, pool),
            name: Some(sym(pool, &inner.name)),
            delimiter_loc: reqspan(&inner.double_colon_l, span(&inner.name_l)),
            name_loc: span(&inner.name_l),
        },
    }
}

/// Constant write or `::` path write.
fn lower_casgn(inner: &Casgn, node: &Mri, pool: &mut Pool) -> Node {
    match (&inner.scope, &inner.value) {
        (None, Some(value)) => Node::ConstantWriteNode {
            flags: 0,
            span: espan(node),
            name: sym(pool, &inner.name),
            name_loc: span(&inner.name_l),
            value: Box::new(conv(value, pool)),
            operator_loc: reqspan(&inner.operator_l, span(&inner.name_l)),
        },
        (Some(scope), Some(value)) => Node::ConstantPathWriteNode {
            flags: 0,
            span: espan(node),
            target: Box::new(Node::ConstantPathNode {
                flags: 0,
                span: espan(node),
                parent: scope_parent(scope, pool),
                name: Some(sym(pool, &inner.name)),
                delimiter_loc: reqspan(&inner.double_colon_l, span(&inner.name_l)),
                name_loc: span(&inner.name_l),
            }),
            operator_loc: reqspan(&inner.operator_l, span(&inner.name_l)),
            value: Box::new(conv(value, pool)),
        },
        // Unreachable in valid trees outside assignment targets: read shape.
        (None, None) => Node::ConstantReadNode {
            flags: 0,
            span: espan(node),
            name: sym(pool, &inner.name),
        },
        (Some(scope), None) => Node::ConstantPathNode {
            flags: 0,
            span: espan(node),
            parent: scope_parent(scope, pool),
            name: Some(sym(pool, &inner.name)),
            delimiter_loc: reqspan(&inner.double_colon_l, span(&inner.name_l)),
            name_loc: span(&inner.name_l),
        },
    }
}

/// Parent of a `::` path; leading `::` (`Cbase`) maps to no parent.
fn scope_parent(scope: &Mri, pool: &mut Pool) -> Option<Box<Node>> {
    match scope {
        Mri::Cbase(_) => None,
        other => Some(Box::new(conv(other, pool))),
    }
}

/// Trailing constant name of a class/module path for the `name` symbol.
fn const_name(path: &Mri, pool: &mut Pool) -> SymbolId {
    match path {
        Mri::Const(inner) => sym(pool, &inner.name),
        Mri::Casgn(inner) => sym(pool, &inner.name),
        // Unreachable in valid trees: class names are constants.
        _ => raw_sym(pool, b""),
    }
}

/// `op=` writes to the matching `*OperatorWrite` node.
fn lower_op_write(
    recv: &Mri,
    operator: &str,
    operator_l: &Loc,
    value: Node,
    node: &Mri,
    pool: &mut Pool,
) -> Node {
    let whole = espan(node);
    let binary_operator = sym(pool, operator);
    let binary_operator_loc = span(operator_l);
    match recv {
        Mri::Lvasgn(inner) => Node::LocalVariableOperatorWriteNode {
            flags: 0,
            span: whole,
            name_loc: span(&inner.name_l),
            binary_operator_loc,
            value: Box::new(value),
            name: sym(pool, &inner.name),
            binary_operator,
            depth: 0,
        },
        Mri::Ivasgn(inner) => Node::InstanceVariableOperatorWriteNode {
            flags: 0,
            span: whole,
            name: sym(pool, &inner.name),
            name_loc: span(&inner.name_l),
            binary_operator_loc,
            value: Box::new(value),
            binary_operator,
        },
        Mri::Cvasgn(inner) => Node::ClassVariableOperatorWriteNode {
            flags: 0,
            span: whole,
            name: sym(pool, &inner.name),
            name_loc: span(&inner.name_l),
            binary_operator_loc,
            value: Box::new(value),
            binary_operator,
        },
        Mri::Gvasgn(inner) => Node::GlobalVariableOperatorWriteNode {
            flags: 0,
            span: whole,
            name: sym(pool, &inner.name),
            name_loc: span(&inner.name_l),
            binary_operator_loc,
            value: Box::new(value),
            binary_operator,
        },
        Mri::Casgn(inner) => match &inner.scope {
            None => Node::ConstantOperatorWriteNode {
                flags: 0,
                span: whole,
                name: sym(pool, &inner.name),
                name_loc: span(&inner.name_l),
                binary_operator_loc,
                value: Box::new(value),
                binary_operator,
            },
            Some(scope) => Node::ConstantPathOperatorWriteNode {
                flags: 0,
                span: whole,
                target: Box::new(Node::ConstantPathNode {
                    flags: 0,
                    span: espan(recv),
                    parent: scope_parent(scope, pool),
                    name: Some(sym(pool, &inner.name)),
                    delimiter_loc: reqspan(&inner.double_colon_l, span(&inner.name_l)),
                    name_loc: span(&inner.name_l),
                }),
                binary_operator_loc,
                value: Box::new(value),
                binary_operator,
            },
        },
        Mri::Send(inner) => Node::CallOperatorWriteNode {
            flags: 0,
            span: whole,
            receiver: inner.recv.as_deref().map(|recv| Box::new(conv(recv, pool))),
            call_operator_loc: inner.dot_l.as_ref().map(span),
            message_loc: ospan(&inner.selector_l),
            read_name: sym(pool, &inner.method_name),
            write_name: sym(pool, format!("{}=", inner.method_name).as_str()),
            binary_operator,
            binary_operator_loc,
            value: Box::new(value),
        },
        Mri::Index(inner) => index_operator_write(
            &inner.recv,
            &inner.indexes,
            &inner.begin_l,
            &inner.end_l,
            binary_operator,
            binary_operator_loc,
            value,
            whole,
            pool,
        ),
        Mri::IndexAsgn(inner) => index_operator_write(
            &inner.recv,
            &inner.indexes,
            &inner.begin_l,
            &inner.end_l,
            binary_operator,
            binary_operator_loc,
            value,
            whole,
            pool,
        ),
        // Unreachable in valid trees: keep the receiver, drop the operator.
        other => conv(other, pool),
    }
}

/// `&&=`/`||=` writes to the matching `*AndWrite`/`*OrWrite` node.
fn lower_logic_write(
    recv: &Mri,
    is_and: bool,
    operator_l: &Loc,
    value: Node,
    node: &Mri,
    pool: &mut Pool,
) -> Node {
    let whole = espan(node);
    let operator_loc = span(operator_l);
    if is_and {
        match recv {
            Mri::Lvasgn(inner) => Node::LocalVariableAndWriteNode {
                flags: 0,
                span: whole,
                name_loc: span(&inner.name_l),
                operator_loc,
                value: Box::new(value),
                name: sym(pool, &inner.name),
                depth: 0,
            },
            Mri::Ivasgn(inner) => Node::InstanceVariableAndWriteNode {
                flags: 0,
                span: whole,
                name: sym(pool, &inner.name),
                name_loc: span(&inner.name_l),
                operator_loc,
                value: Box::new(value),
            },
            Mri::Cvasgn(inner) => Node::ClassVariableAndWriteNode {
                flags: 0,
                span: whole,
                name: sym(pool, &inner.name),
                name_loc: span(&inner.name_l),
                operator_loc,
                value: Box::new(value),
            },
            Mri::Gvasgn(inner) => Node::GlobalVariableAndWriteNode {
                flags: 0,
                span: whole,
                name: sym(pool, &inner.name),
                name_loc: span(&inner.name_l),
                operator_loc,
                value: Box::new(value),
            },
            Mri::Casgn(inner) => {
                logic_const_write(inner, recv, true, operator_loc, value, whole, pool)
            }
            Mri::Send(inner) => Node::CallAndWriteNode {
                flags: 0,
                span: whole,
                receiver: inner.recv.as_deref().map(|recv| Box::new(conv(recv, pool))),
                call_operator_loc: inner.dot_l.as_ref().map(span),
                message_loc: ospan(&inner.selector_l),
                read_name: sym(pool, &inner.method_name),
                write_name: sym(pool, format!("{}=", inner.method_name).as_str()),
                operator_loc,
                value: Box::new(value),
            },
            Mri::Index(inner) => index_logic_write(
                &inner.recv,
                &inner.indexes,
                &inner.begin_l,
                &inner.end_l,
                true,
                operator_loc,
                value,
                whole,
                pool,
            ),
            Mri::IndexAsgn(inner) => index_logic_write(
                &inner.recv,
                &inner.indexes,
                &inner.begin_l,
                &inner.end_l,
                true,
                operator_loc,
                value,
                whole,
                pool,
            ),
            // Unreachable in valid trees: keep the receiver, drop the operator.
            other => conv(other, pool),
        }
    } else {
        match recv {
            Mri::Lvasgn(inner) => Node::LocalVariableOrWriteNode {
                flags: 0,
                span: whole,
                name_loc: span(&inner.name_l),
                operator_loc,
                value: Box::new(value),
                name: sym(pool, &inner.name),
                depth: 0,
            },
            Mri::Ivasgn(inner) => Node::InstanceVariableOrWriteNode {
                flags: 0,
                span: whole,
                name: sym(pool, &inner.name),
                name_loc: span(&inner.name_l),
                operator_loc,
                value: Box::new(value),
            },
            Mri::Cvasgn(inner) => Node::ClassVariableOrWriteNode {
                flags: 0,
                span: whole,
                name: sym(pool, &inner.name),
                name_loc: span(&inner.name_l),
                operator_loc,
                value: Box::new(value),
            },
            Mri::Gvasgn(inner) => Node::GlobalVariableOrWriteNode {
                flags: 0,
                span: whole,
                name: sym(pool, &inner.name),
                name_loc: span(&inner.name_l),
                operator_loc,
                value: Box::new(value),
            },
            Mri::Casgn(inner) => {
                logic_const_write(inner, recv, false, operator_loc, value, whole, pool)
            }
            Mri::Send(inner) => Node::CallOrWriteNode {
                flags: 0,
                span: whole,
                receiver: inner.recv.as_deref().map(|recv| Box::new(conv(recv, pool))),
                call_operator_loc: inner.dot_l.as_ref().map(span),
                message_loc: ospan(&inner.selector_l),
                read_name: sym(pool, &inner.method_name),
                write_name: sym(pool, format!("{}=", inner.method_name).as_str()),
                operator_loc,
                value: Box::new(value),
            },
            Mri::Index(inner) => index_logic_write(
                &inner.recv,
                &inner.indexes,
                &inner.begin_l,
                &inner.end_l,
                false,
                operator_loc,
                value,
                whole,
                pool,
            ),
            Mri::IndexAsgn(inner) => index_logic_write(
                &inner.recv,
                &inner.indexes,
                &inner.begin_l,
                &inner.end_l,
                false,
                operator_loc,
                value,
                whole,
                pool,
            ),
            // Unreachable in valid trees: keep the receiver, drop the operator.
            other => conv(other, pool),
        }
    }
}

/// Constant `&&=`/`||=` writes, splitting dotted paths from bare names.
fn logic_const_write(
    inner: &Casgn,
    recv: &Mri,
    is_and: bool,
    operator_loc: Span,
    value: Node,
    whole: Span,
    pool: &mut Pool,
) -> Node {
    match &inner.scope {
        None => {
            let name = sym(pool, &inner.name);
            let name_loc = span(&inner.name_l);
            if is_and {
                Node::ConstantAndWriteNode {
                    flags: 0,
                    span: whole,
                    name,
                    name_loc,
                    operator_loc,
                    value: Box::new(value),
                }
            } else {
                Node::ConstantOrWriteNode {
                    flags: 0,
                    span: whole,
                    name,
                    name_loc,
                    operator_loc,
                    value: Box::new(value),
                }
            }
        }
        Some(scope) => {
            let target = Box::new(Node::ConstantPathNode {
                flags: 0,
                span: espan(recv),
                parent: scope_parent(scope, pool),
                name: Some(sym(pool, &inner.name)),
                delimiter_loc: reqspan(&inner.double_colon_l, span(&inner.name_l)),
                name_loc: span(&inner.name_l),
            });
            if is_and {
                Node::ConstantPathAndWriteNode {
                    flags: 0,
                    span: whole,
                    target,
                    operator_loc,
                    value: Box::new(value),
                }
            } else {
                Node::ConstantPathOrWriteNode {
                    flags: 0,
                    span: whole,
                    target,
                    operator_loc,
                    value: Box::new(value),
                }
            }
        }
    }
}

/// Index `op=` writes.
#[allow(clippy::too_many_arguments)]
fn index_operator_write(
    recv: &Mri,
    indexes: &[Mri],
    begin_l: &Loc,
    end_l: &Loc,
    binary_operator: SymbolId,
    binary_operator_loc: Span,
    value: Node,
    whole: Span,
    pool: &mut Pool,
) -> Node {
    Node::IndexOperatorWriteNode {
        flags: 0,
        span: whole,
        receiver: Some(Box::new(conv(recv, pool))),
        call_operator_loc: None,
        opening_loc: span(begin_l),
        arguments: call_args(indexes, pool),
        closing_loc: span(end_l),
        block: None,
        binary_operator,
        binary_operator_loc,
        value: Box::new(value),
    }
}

/// Index `&&=`/`||=` writes.
#[allow(clippy::too_many_arguments)]
fn index_logic_write(
    recv: &Mri,
    indexes: &[Mri],
    begin_l: &Loc,
    end_l: &Loc,
    is_and: bool,
    operator_loc: Span,
    value: Node,
    whole: Span,
    pool: &mut Pool,
) -> Node {
    let receiver = Some(Box::new(conv(recv, pool)));
    let arguments = call_args(indexes, pool);
    if is_and {
        Node::IndexAndWriteNode {
            flags: 0,
            span: whole,
            receiver,
            call_operator_loc: None,
            opening_loc: span(begin_l),
            arguments,
            closing_loc: span(end_l),
            block: None,
            operator_loc,
            value: Box::new(value),
        }
    } else {
        Node::IndexOrWriteNode {
            flags: 0,
            span: whole,
            receiver,
            call_operator_loc: None,
            opening_loc: span(begin_l),
            arguments,
            closing_loc: span(end_l),
            block: None,
            operator_loc,
            value: Box::new(value),
        }
    }
}

/// Parenthesized groups stay `ParenthesesNode`; bare sequences spread.
fn lower_begin(inner: &Begin, node: &Mri, pool: &mut Pool) -> Node {
    let whole = espan(node);
    match &inner.begin_l {
        Some(begin) => {
            let body = match inner.statements.as_slice() {
                [] => None,
                // A single statement still nests in `StatementsNode`
                // (Prism shape; `defined?` unwraps exactly that).
                [single] => {
                    let stmt = conv(single, pool);
                    let span = stmt.span();
                    Some(seq(vec![stmt], span))
                }
                _ => Some(seq(
                    inner
                        .statements
                        .iter()
                        .map(|stmt| conv(stmt, pool))
                        .collect(),
                    whole,
                )),
            };
            Node::ParenthesesNode {
                flags: 0,
                span: whole,
                body,
                opening_loc: span(begin),
                closing_loc: reqspan(&inner.end_l, whole),
            }
        }
        None => match inner.statements.as_slice() {
            [] => Node::StatementsNode {
                flags: 0,
                span: whole,
                body: Vec::new(),
            },
            [single] => conv(single, pool),
            _ => Node::StatementsNode {
                flags: 0,
                span: whole,
                body: inner
                    .statements
                    .iter()
                    .map(|stmt| conv(stmt, pool))
                    .collect(),
            },
        },
    }
}

/// Trailing keyword arguments to `KeywordHashNode` with `SYMBOL_KEYS` check.
fn lower_kwargs(pairs: &[Mri], span: Span, pool: &mut Pool) -> Node {
    let elements: Vec<Node> = pairs.iter().map(|pair| hash_element(pair, pool)).collect();
    let mut flags = 0;
    if !elements.is_empty()
        && elements.iter().all(|el| {
            matches!(el, Node::AssocNode { key, .. } if matches!(**key, Node::SymbolNode { .. }))
        })
    {
        flags |= keyword_hash_node_flags::SYMBOL_KEYS;
    }
    Node::KeywordHashNode {
        flags,
        span,
        elements,
    }
}

/// One hash element: pairs stay `AssocNode`, `**` becomes `AssocSplatNode`.
fn hash_element(pair: &Mri, pool: &mut Pool) -> Node {
    match pair {
        Mri::Pair(inner) => Node::AssocNode {
            flags: 0,
            span: espan(pair),
            key: Box::new(conv(&inner.key, pool)),
            value: Box::new(conv(&inner.value, pool)),
            operator_loc: Some(span(&inner.operator_l)),
        },
        Mri::Kwsplat(inner) => Node::AssocSplatNode {
            flags: 0,
            span: espan(pair),
            value: Some(Box::new(conv(&inner.value, pool))),
            operator_loc: span(&inner.operator_l),
        },
        // Unreachable in valid trees: keep the lowered node as the element.
        other => conv(other, pool),
    }
}

/// `else` branch: absent stays absent, empty stays an empty `ElseNode`,
/// `elsif` chains nest directly as the subsequent node.
fn else_node(
    body: Option<&Mri>,
    else_l: Option<&Loc>,
    end: Span,
    pool: &mut Pool,
) -> Option<Box<Node>> {
    match body {
        None => else_l.map(|loc| {
            Box::new(Node::ElseNode {
                flags: 0,
                span: Span {
                    start: span(loc).start,
                    end: end.end,
                },
                else_keyword_loc: span(loc),
                statements: None,
                end_keyword_loc: Some(end),
            })
        }),
        Some(Mri::EmptyElse(inner)) => Some(Box::new(Node::ElseNode {
            flags: 0,
            span: span(&inner.expression_l),
            else_keyword_loc: else_l
                .map(span)
                .unwrap_or_else(|| span(&inner.expression_l)),
            statements: None,
            end_keyword_loc: Some(end),
        })),
        Some(Mri::If(_)) => body.map(|inner| Box::new(conv(inner, pool))),
        Some(other) => {
            let whole = espan(other);
            Some(Box::new(Node::ElseNode {
                flags: 0,
                span: whole,
                else_keyword_loc: else_l.map(span).unwrap_or(whole),
                statements: Some(seq_one(other, pool)),
                end_keyword_loc: Some(end),
            }))
        }
    }
}

/// Statement `if`: bare `unless` (empty `if_true`) maps to `UnlessNode`.
/// `unless...else` keeps `if` shape (no source to tell them apart).
fn lower_if(inner: &If, node: &Mri, pool: &mut Pool) -> Node {
    let whole = espan(node);
    let predicate = Box::new(conv(&inner.cond, pool));
    match (&inner.if_true, &inner.if_false) {
        (None, Some(if_false)) => Node::UnlessNode {
            flags: 0,
            span: whole,
            keyword_loc: span(&inner.keyword_l),
            predicate,
            then_keyword_loc: then_loc(&inner.begin_l),
            statements: Some(seq_one(if_false, pool)),
            else_clause: None,
            end_keyword_loc: ospan(&inner.end_l),
        },
        _ => Node::IfNode {
            flags: 0,
            span: whole,
            if_keyword_loc: Some(span(&inner.keyword_l)),
            predicate,
            then_keyword_loc: then_loc(&inner.begin_l),
            statements: inner.if_true.as_deref().map(|then| seq_one(then, pool)),
            subsequent: else_node(
                inner.if_false.as_deref(),
                inner.else_l.as_ref(),
                ospan(&inner.end_l).unwrap_or(whole),
                pool,
            ),
            end_keyword_loc: ospan(&inner.end_l),
        },
    }
}

/// Modifier `if`/`unless`: the keyword stays, locs stay empty otherwise.
fn lower_if_mod(inner: &IfMod, node: &Mri, pool: &mut Pool) -> Node {
    let whole = espan(node);
    let predicate = Box::new(conv(&inner.cond, pool));
    match (&inner.if_true, &inner.if_false) {
        (None, Some(if_false)) => Node::UnlessNode {
            flags: 0,
            span: whole,
            keyword_loc: span(&inner.keyword_l),
            predicate,
            then_keyword_loc: None,
            statements: Some(seq_one(if_false, pool)),
            else_clause: None,
            end_keyword_loc: None,
        },
        (Some(if_true), if_false) => Node::IfNode {
            flags: 0,
            span: whole,
            if_keyword_loc: Some(span(&inner.keyword_l)),
            predicate,
            then_keyword_loc: None,
            statements: Some(seq_one(if_true, pool)),
            subsequent: if_false.as_deref().map(|els| {
                Box::new(Node::ElseNode {
                    flags: 0,
                    span: espan(els),
                    else_keyword_loc: espan(els),
                    statements: Some(seq_one(els, pool)),
                    end_keyword_loc: Some(whole),
                })
            }),
            end_keyword_loc: None,
        },
        // Unreachable in valid trees: empty modifier keeps the predicate.
        (None, None) => Node::IfNode {
            flags: 0,
            span: whole,
            if_keyword_loc: Some(span(&inner.keyword_l)),
            predicate,
            then_keyword_loc: None,
            statements: None,
            subsequent: None,
            end_keyword_loc: None,
        },
    }
}

/// Pattern guard alone (defensive): modifier shape with no statements.
fn lower_guard(is_if: bool, cond: &Mri, keyword_l: &Loc, node: &Mri, pool: &mut Pool) -> Node {
    let whole = espan(node);
    let predicate = Box::new(conv(cond, pool));
    if is_if {
        Node::IfNode {
            flags: 0,
            span: whole,
            if_keyword_loc: Some(span(keyword_l)),
            predicate,
            then_keyword_loc: None,
            statements: None,
            subsequent: None,
            end_keyword_loc: None,
        }
    } else {
        Node::UnlessNode {
            flags: 0,
            span: whole,
            keyword_loc: span(keyword_l),
            predicate,
            then_keyword_loc: None,
            statements: None,
            else_clause: None,
            end_keyword_loc: None,
        }
    }
}

/// Wrap a pattern in its `if`/`unless` guard, mirroring Prism modifiers.
fn wrap_guard(pattern: Node, guard: &Mri, pool: &mut Pool) -> Node {
    let whole = pattern.span();
    match guard {
        Mri::IfGuard(inner) => Node::IfNode {
            flags: 0,
            span: whole,
            if_keyword_loc: Some(span(&inner.keyword_l)),
            predicate: Box::new(conv(&inner.cond, pool)),
            then_keyword_loc: None,
            statements: Some(Box::new(Node::StatementsNode {
                flags: 0,
                span: whole,
                body: vec![pattern],
            })),
            subsequent: None,
            end_keyword_loc: None,
        },
        Mri::UnlessGuard(inner) => Node::UnlessNode {
            flags: 0,
            span: whole,
            keyword_loc: span(&inner.keyword_l),
            predicate: Box::new(conv(&inner.cond, pool)),
            then_keyword_loc: None,
            statements: Some(Box::new(Node::StatementsNode {
                flags: 0,
                span: whole,
                body: vec![pattern],
            })),
            else_clause: None,
            end_keyword_loc: None,
        },
        // Unreachable in valid trees: keep the bare pattern.
        _ => pattern,
    }
}

/// `while`/`until`, statement and post-condition forms sharing one shape.
#[allow(clippy::too_many_arguments)]
fn lower_loop(
    is_while: bool,
    cond: &Mri,
    body: Option<&Mri>,
    keyword_l: &Loc,
    begin_l: Option<&Loc>,
    end_l: Option<&Loc>,
    node: &Mri,
    pool: &mut Pool,
) -> Node {
    let whole = espan(node);
    let post = matches!(node, Mri::WhilePost(_) | Mri::UntilPost(_));
    let mut flags = 0;
    if post {
        flags |= loop_flags::BEGIN_MODIFIER;
    }
    let predicate = Box::new(conv(cond, pool));
    let statements = body.map(|inner| seq_one(inner, pool));
    if is_while {
        Node::WhileNode {
            flags,
            span: whole,
            keyword_loc: span(keyword_l),
            do_keyword_loc: begin_l.and_then(do_loc),
            closing_loc: end_l.map(span),
            predicate,
            statements,
        }
    } else {
        Node::UntilNode {
            flags,
            span: whole,
            keyword_loc: span(keyword_l),
            do_keyword_loc: begin_l.and_then(do_loc),
            closing_loc: end_l.map(span),
            predicate,
            statements,
        }
    }
}

/// One rescue body plus its tail as a `RescueNode` chain link.
fn lower_rescue_body_node(
    inner: &RescueBody,
    subsequent: Option<Box<Node>>,
    whole: Span,
    pool: &mut Pool,
) -> Node {
    Node::RescueNode {
        flags: 0,
        span: whole,
        keyword_loc: span(&inner.keyword_l),
        exceptions: rescue_exceptions(inner.exc_list.as_deref(), pool),
        operator_loc: inner.assoc_l.as_ref().map(span),
        reference: inner
            .exc_var
            .as_deref()
            .map(|var| Box::new(target(var, pool))),
        then_keyword_loc: inner.begin_l.as_ref().and_then(then_loc),
        statements: body_of(inner.body.as_deref(), pool),
        subsequent,
    }
}

/// Exception list: `rescue` arrays flatten, bare nodes stand alone.
fn rescue_exceptions(list: Option<&Mri>, pool: &mut Pool) -> Vec<Node> {
    match list {
        None => Vec::new(),
        Some(Mri::Array(inner)) => inner.elements.iter().map(|el| conv(el, pool)).collect(),
        Some(other) => vec![conv(other, pool)],
    }
}

/// Split a statement `Rescue` into pre-statements and the clause chain.
/// Callers attach `else` themselves (the `end` span differs per site).
fn split_rescue(inner: &Rescue, pool: &mut Pool) -> (Vec<Node>, Option<Box<Node>>) {
    let pre = flat_body(inner.body.as_deref(), pool);
    let mut chain: Option<Box<Node>> = None;
    for body in inner.rescue_bodies.iter().rev() {
        let Mri::RescueBody(inner) = body else {
            // Unreachable in valid trees: rescue lists hold bodies.
            continue;
        };
        let node = lower_rescue_body_node(inner, chain, span(&inner.expression_l), pool);
        chain = Some(Box::new(node));
    }
    (pre, chain)
}

/// Statement rescue to `BeginNode` (mirrors Prism def/block rescue bodies).
/// Modifier rescue (`a rescue b`) to `RescueModifierNode`.
fn lower_rescue_standalone(inner: &Rescue, node: &Mri, pool: &mut Pool) -> Node {
    let whole = espan(node);
    if is_modifier_rescue(inner) {
        let body = match inner.rescue_bodies.first() {
            Some(Mri::RescueBody(inner)) => Some(inner),
            // Unreachable: modifiers carry one rescue body.
            _ => None,
        };
        return Node::RescueModifierNode {
            flags: 0,
            span: whole,
            expression: Box::new(
                inner
                    .body
                    .as_deref()
                    .map(|expr| conv(expr, pool))
                    .unwrap_or(Node::NilNode {
                        flags: 0,
                        span: whole,
                    }),
            ),
            keyword_loc: body.map_or(whole, |rb| span(&rb.keyword_l)),
            rescue_expression: Box::new(
                body.and_then(|rb| rb.body.as_deref())
                    .map(|expr| conv(expr, pool))
                    .unwrap_or(Node::NilNode {
                        flags: 0,
                        span: whole,
                    }),
            ),
        };
    }
    let (pre, chain) = split_rescue(inner, pool);
    let statements = if pre.is_empty() {
        None
    } else {
        Some(seq(pre, whole))
    };
    Node::BeginNode {
        flags: 0,
        span: whole,
        begin_keyword_loc: None,
        statements,
        rescue_clause: chain,
        else_clause: else_node(inner.else_.as_deref(), inner.else_l.as_ref(), whole, pool),
        ensure_clause: None,
        end_keyword_loc: None,
    }
}

/// Modifier rescue: one bare body, no list, no variable, no `begin` marker.
fn is_modifier_rescue(inner: &Rescue) -> bool {
    if inner.body.is_none() || inner.else_.is_some() || inner.else_l.is_some() {
        return false;
    }
    match inner.rescue_bodies.as_slice() {
        [Mri::RescueBody(single)] => {
            single.exc_list.is_none()
                && single.exc_var.is_none()
                && single.begin_l.is_none()
                && single.body.is_some()
        }
        _ => false,
    }
}

/// `Ensure` outside `KwBegin` (def bodies): `BeginNode` plus ensure clause.
fn lower_ensure_begin(inner: &Ensure, end: Option<Span>, node: &Mri, pool: &mut Pool) -> Node {
    let whole = espan(node);
    let end_or_whole = end.unwrap_or(whole);
    let (pre, rescue_clause, else_clause) = match &inner.body {
        Some(body) if matches!(&**body, Mri::Rescue(_)) => match &**body {
            Mri::Rescue(rescue) => {
                let (pre, chain) = split_rescue(rescue, pool);
                let els = else_node(
                    rescue.else_.as_deref(),
                    rescue.else_l.as_ref(),
                    end_or_whole,
                    pool,
                );
                (pre, chain, els)
            }
            // Unreachable: just matched `Rescue`.
            _ => (Vec::new(), None, None),
        },
        other => (flat_body(other.as_deref(), pool), None, None),
    };
    let statements = if pre.is_empty() {
        None
    } else {
        Some(seq(pre, whole))
    };
    Node::BeginNode {
        flags: 0,
        span: whole,
        begin_keyword_loc: None,
        statements,
        rescue_clause,
        else_clause,
        ensure_clause: Some(Box::new(Node::EnsureNode {
            flags: 0,
            span: whole,
            ensure_keyword_loc: span(&inner.keyword_l),
            statements: body_of(inner.ensure.as_deref(), pool),
            end_keyword_loc: end_or_whole,
        })),
        end_keyword_loc: end,
    }
}

/// Explicit `begin/end` with optional rescue, else and ensure clauses.
fn lower_kwbegin(inner: &KwBegin, node: &Mri, pool: &mut Pool) -> Node {
    let whole = espan(node);
    let mut pre = Vec::new();
    let mut rescue_clause = None;
    let mut else_clause = None;
    let mut ensure_clause = None;
    for child in &inner.statements {
        match child {
            Mri::Rescue(rescue) => {
                let (mut stmts, chain) = split_rescue(rescue, pool);
                pre.append(&mut stmts);
                rescue_clause = chain;
                else_clause = else_node(
                    rescue.else_.as_deref(),
                    rescue.else_l.as_ref(),
                    ospan(&inner.end_l).unwrap_or(whole),
                    pool,
                );
            }
            Mri::Ensure(ensure) => {
                let ensured = lower_ensure_begin(ensure, ospan(&inner.end_l), child, pool);
                match ensured {
                    Node::BeginNode {
                        statements,
                        rescue_clause: rescue,
                        else_clause: els,
                        ensure_clause: ensure,
                        ..
                    } => {
                        if let Some(stmts) = statements {
                            match *stmts {
                                Node::StatementsNode { body, .. } => pre.extend(body),
                                other => pre.push(other),
                            }
                        }
                        if rescue.is_some() {
                            rescue_clause = rescue;
                        }
                        if els.is_some() {
                            else_clause = els;
                        }
                        ensure_clause = ensure;
                    }
                    // Unreachable: `lower_ensure_begin` returns `BeginNode`.
                    other => pre.push(other),
                }
            }
            other => pre.push(conv(other, pool)),
        }
    }
    let statements = if pre.is_empty() {
        None
    } else {
        Some(seq(pre, whole))
    };
    Node::BeginNode {
        flags: 0,
        span: whole,
        begin_keyword_loc: ospan(&inner.begin_l),
        statements,
        rescue_clause,
        else_clause,
        ensure_clause,
        end_keyword_loc: ospan(&inner.end_l),
    }
}

/// Integer literal: `I64` when it fits, normalized decimal `Fallback`
/// otherwise (mirrors the Prism lowering, limb-free over the MRI text).
fn convert_int(text: &str) -> (Integer, u16) {
    let bytes = text.as_bytes();
    let (negative, rest) = match bytes.split_first() {
        Some((b'-', tail)) => (true, tail),
        Some((b'+', tail)) => (false, tail),
        _ => (false, bytes),
    };
    let (radix, raw, base) = if rest.len() > 2 && rest[0] == b'0' {
        match rest[1] {
            b'x' | b'X' => (16, &rest[2..], integer_base_flags::HEXADECIMAL),
            b'b' | b'B' => (2, &rest[2..], integer_base_flags::BINARY),
            b'o' | b'O' => (8, &rest[2..], integer_base_flags::OCTAL),
            b'd' | b'D' => (10, &rest[2..], integer_base_flags::DECIMAL),
            _ => legacy_radix(rest),
        }
    } else {
        legacy_radix(rest)
    };
    // Little-endian base-2^32 magnitude over the cleaned digits.
    let mut limbs: Vec<u32> = vec![0];
    for byte in raw.iter().filter(|b| **b != b'_') {
        let digit = digit_value(*byte);
        if digit >= radix {
            continue;
        }
        let mut carry = digit;
        for limb in limbs.iter_mut() {
            let current = u64::from(*limb) * u64::from(radix) + u64::from(carry);
            *limb = current as u32;
            carry = (current >> 32) as u32;
        }
        // The leftover carry always fits one limb (`radix <= 36`).
        if carry > 0 {
            limbs.push(carry);
        }
    }
    while limbs.len() > 1 && limbs.last() == Some(&0) {
        limbs.pop();
    }
    let limit: u128 = if negative {
        1u128 << 63
    } else {
        i64::MAX as u128
    };
    if limbs.len() <= 2 {
        let magnitude = limbs.iter().enumerate().fold(0u128, |acc, (index, limb)| {
            acc | (u128::from(*limb) << (32 * index))
        });
        if magnitude <= limit {
            let value = if negative {
                if magnitude == 1u128 << 63 {
                    i64::MIN
                } else {
                    -(magnitude as i64)
                }
            } else {
                magnitude as i64
            };
            return (Integer::I64(value), base);
        }
    }
    let mut digits = limbs_to_decimal(&limbs);
    if negative && digits != b"0" {
        let mut signed = vec![b'-'];
        signed.append(&mut digits);
        digits = signed;
    }
    (Integer::Fallback { raw: digits }, base)
}

/// Radix for prefix-less literals: legacy `0...` octal, else decimal.
fn legacy_radix(rest: &[u8]) -> (u32, &[u8], u16) {
    if rest.len() > 1
        && rest[0] == b'0'
        && rest
            .iter()
            .all(|b| (*b >= b'0' && *b <= b'7') || *b == b'_')
    {
        (8, rest, integer_base_flags::OCTAL)
    } else {
        (10, rest, integer_base_flags::DECIMAL)
    }
}

/// Digit value for radix digits (upper- and lowercase hex).
fn digit_value(byte: u8) -> u32 {
    match byte {
        b'0'..=b'9' => u32::from(byte - b'0'),
        b'a'..=b'f' => u32::from(byte - b'a') + 10,
        b'A'..=b'F' => u32::from(byte - b'A') + 10,
        _ => u32::MAX,
    }
}

/// Float literal text to `f64` (underscores stripped; `0.0` fallback).
fn parse_float(text: &str) -> f64 {
    text.replace('_', "").parse().unwrap_or(0.0)
}

/// Rational literal to numerator/denominator, reduced when small enough
/// for `u128` math, best-effort decimal shift beyond that.
fn parse_rational(text: &str) -> (Integer, Integer) {
    let body = text.strip_suffix('r').unwrap_or(text);
    let (mantissa, exp) = split_exponent(body);
    let (int_part, frac_part) = match mantissa.find('.') {
        Some(dot) => (&mantissa[..dot], &mantissa[dot + 1..]),
        None => (mantissa, ""),
    };
    let (negative, unsigned) = match int_part.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, int_part.strip_prefix('+').unwrap_or(int_part)),
    };
    let clean = |part: &str| {
        part.bytes()
            .filter(|b| *b != b'_')
            .filter(u8::is_ascii_digit)
            .collect::<Vec<u8>>()
    };
    let mut digits = clean(unsigned);
    let frac_digits = clean(frac_part);
    let frac_len = frac_digits.len() as i64;
    digits.extend_from_slice(&frac_digits);
    while digits.len() > 1 && digits.first() == Some(&b'0') {
        digits.remove(0);
    }
    if digits.is_empty() {
        digits = vec![b'0'];
    }
    let shift = exp.saturating_add(-frac_len);
    if let Ok(small) = decimal_u128(&digits, shift) {
        let (mut num, mut den) = small;
        let divisor = gcd_u128(num, den);
        num /= divisor;
        den /= divisor;
        let numerator = if negative {
            negate_integer(num)
        } else {
            u128_to_integer(num)
        };
        return (numerator, u128_to_integer(den));
    }
    // Best-effort path for huge literals: decimal shift as digit strings.
    let (num_text, den_text) = shift_decimal_text(&digits, shift);
    let numerator = if negative && num_text != b"0" {
        let mut signed = vec![b'-'];
        signed.extend_from_slice(&num_text);
        Integer::Fallback { raw: signed }
    } else {
        Integer::Fallback { raw: num_text }
    };
    (numerator, Integer::Fallback { raw: den_text })
}

/// Split a trailing decimal exponent (`1e3`, `1E-3`) from the mantissa.
fn split_exponent(body: &str) -> (&str, i64) {
    let bytes = body.as_bytes();
    let mut at = None;
    for (index, byte) in bytes.iter().enumerate().rev() {
        if *byte == b'e' || *byte == b'E' {
            at = Some(index);
            break;
        }
        if !byte.is_ascii_digit() && *byte != b'+' && *byte != b'-' && *byte != b'_' {
            break;
        }
    }
    match at {
        Some(index) => {
            let exp = body[index + 1..].replace('_', "").parse().unwrap_or(0);
            (&body[..index], exp)
        }
        None => (body, 0),
    }
}

/// Mantissa digits with a decimal shift as `(numerator, denominator)`.
fn decimal_u128(digits: &[u8], shift: i64) -> Result<(u128, u128), ()> {
    let text = core::str::from_utf8(digits).map_err(|_| ())?;
    let mantissa: u128 = text.parse().map_err(|_| ())?;
    if shift >= 0 {
        let factor = checked_pow10(shift.unsigned_abs())?;
        Ok((mantissa.checked_mul(factor).ok_or(())?, 1))
    } else {
        let factor = checked_pow10(shift.unsigned_abs())?;
        Ok((mantissa, factor))
    }
}

/// `10^exp`, failing past `u128`.
fn checked_pow10(exp: u64) -> Result<u128, ()> {
    let mut out: u128 = 1;
    for _ in 0..exp {
        out = out.checked_mul(10).ok_or(())?;
    }
    Ok(out)
}

/// Greatest common divisor for reduction.
fn gcd_u128(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let rest = a % b;
        a = b;
        b = rest;
    }
    a.max(1)
}

/// `u128` magnitude to the owned integer model.
fn u128_to_integer(value: u128) -> Integer {
    if let Ok(small) = i64::try_from(value) {
        Integer::I64(small)
    } else {
        Integer::Fallback {
            raw: value.to_string().into_bytes(),
        }
    }
}

/// Negated `u128` magnitude to the owned integer model.
fn negate_integer(value: u128) -> Integer {
    if value == 1u128 << 63 {
        Integer::I64(i64::MIN)
    } else if let Ok(small) = i64::try_from(value) {
        Integer::I64(-small)
    } else {
        let mut raw = vec![b'-'];
        raw.extend_from_slice(value.to_string().as_bytes());
        Integer::Fallback { raw }
    }
}

/// Defensive decimal shift cap (digit-string growth bound).
const MAX_DEC_SHIFT: i64 = 100_000;

/// Apply a decimal shift to digit strings, saturating pathological widths.
fn shift_decimal_text(digits: &[u8], shift: i64) -> (Vec<u8>, Vec<u8>) {
    let shift = shift.clamp(-MAX_DEC_SHIFT, MAX_DEC_SHIFT);
    if shift >= 0 {
        let mut num = digits.to_vec();
        num.extend(vec![b'0'; shift as usize]);
        (trim_zeros(num), vec![b'1'])
    } else {
        let mut den = vec![b'1'];
        den.extend(vec![b'0'; (-shift) as usize]);
        (trim_zeros(digits.to_vec()), den)
    }
}

/// Strip leading zeros, keeping one digit.
fn trim_zeros(mut digits: Vec<u8>) -> Vec<u8> {
    while digits.len() > 1 && digits.first() == Some(&b'0') {
        digits.remove(0);
    }
    if digits.is_empty() {
        vec![b'0']
    } else {
        digits
    }
}

/// Complex literal to an `ImaginaryNode` numeric payload.
fn lower_complex(text: &str, span: Span) -> Node {
    let body = text.strip_suffix('i').unwrap_or(text);
    if body.ends_with('r') {
        let (numerator, denominator) = parse_rational(body);
        Node::RationalNode {
            flags: 0,
            span,
            numerator,
            denominator,
        }
    } else if body.bytes().any(|b| b == b'.' || b == b'e' || b == b'E') {
        Node::FloatNode {
            flags: 0,
            span,
            value: parse_float(body),
        }
    } else {
        let (value, flags) = convert_int(body);
        let _ = flags;
        Node::IntegerNode {
            flags: 0,
            span,
            value,
        }
    }
}

/// One interpolation part: strings stay, `Begin` embeds, reads wrap.
fn interp_part(part: &Mri, pool: &mut Pool) -> Node {
    match part {
        Mri::Str(inner) => {
            let span = espan(part);
            Node::StringNode {
                flags: 0,
                span,
                opening_loc: None,
                content_loc: span,
                closing_loc: None,
                unescaped: inner.value.raw.clone(),
            }
        }
        Mri::Begin(inner) => {
            let whole = espan(part);
            let mut body: Vec<Node> = inner
                .statements
                .iter()
                .map(|stmt| conv(stmt, pool))
                .collect();
            if body.is_empty() {
                body.push(Node::NilNode {
                    flags: 0,
                    span: whole,
                });
            }
            Node::EmbeddedStatementsNode {
                flags: 0,
                span: whole,
                opening_loc: reqspan(&inner.begin_l, whole),
                statements: Some(seq(body, whole)),
                closing_loc: reqspan(&inner.end_l, whole),
            }
        }
        Mri::Lvar(_)
        | Mri::Ivar(_)
        | Mri::Cvar(_)
        | Mri::Gvar(_)
        | Mri::Const(_)
        | Mri::BackRef(_)
        | Mri::NthRef(_) => Node::EmbeddedVariableNode {
            flags: 0,
            span: espan(part),
            operator_loc: espan(part),
            variable: Box::new(conv(part, pool)),
        },
        // Unreachable in valid trees: keep the lowered node inline.
        other => conv(other, pool),
    }
}

/// Regexp option letters to `RegularExpressionFlags` bits.
fn regexp_flags(options: Option<&str>) -> u16 {
    let mut flags = 0;
    for byte in options.map(str::as_bytes).unwrap_or(b"") {
        flags |= match byte {
            b'i' => regular_expression_flags::IGNORE_CASE,
            b'x' => regular_expression_flags::EXTENDED,
            b'm' => regular_expression_flags::MULTI_LINE,
            b'o' => regular_expression_flags::ONCE,
            b'e' => regular_expression_flags::EUC_JP,
            b'n' => regular_expression_flags::ASCII_8BIT,
            b's' => regular_expression_flags::WINDOWS_31J,
            b'u' => regular_expression_flags::UTF_8,
            _ => 0,
        };
    }
    flags
}

/// MRI regexp options text.
fn regexp_options(options: &Option<Box<Mri>>) -> Option<String> {
    match options.as_deref() {
        Some(Mri::RegOpt(inner)) => inner.options.clone(),
        _ => None,
    }
}

/// Interpolated or plain regexp, mirroring Prism literal shapes.
fn lower_regexp(inner: &Regexp, node: &Mri, pool: &mut Pool) -> Node {
    let whole = espan(node);
    let flags = regexp_flags(regexp_options(&inner.options).as_deref());
    if parts_all_plain(&inner.parts) {
        Node::RegularExpressionNode {
            flags,
            span: whole,
            opening_loc: span(&inner.begin_l),
            content_loc: parts_span(&inner.parts, whole),
            closing_loc: span(&inner.end_l),
            unescaped: concat_str_bytes(&inner.parts),
        }
    } else {
        Node::InterpolatedRegularExpressionNode {
            flags,
            span: whole,
            opening_loc: span(&inner.begin_l),
            parts: inner.parts.iter().map(|p| interp_part(p, pool)).collect(),
            closing_loc: span(&inner.end_l),
        }
    }
}

/// Backtick heredoc: plain or interpolated `XString` shapes.
fn lower_xheredoc(parts: &[Mri], body_l: &Loc, end_l: &Loc, node: &Mri, pool: &mut Pool) -> Node {
    let whole = espan(node);
    if parts_all_plain(parts) {
        Node::XStringNode {
            flags: 0,
            span: whole,
            opening_loc: span(body_l),
            content_loc: span(body_l),
            closing_loc: span(end_l),
            unescaped: concat_str_bytes(parts),
        }
    } else {
        Node::InterpolatedXStringNode {
            flags: 0,
            span: whole,
            opening_loc: span(body_l),
            parts: parts.iter().map(|p| interp_part(p, pool)).collect(),
            closing_loc: span(end_l),
        }
    }
}

/// `if /re/` conditions to match-last-line nodes, copying the payload.
fn lower_match_current_line(re: &Mri, node: &Mri, pool: &mut Pool) -> Node {
    let whole = espan(node);
    match re {
        Mri::Regexp(inner) => {
            let flags = regexp_flags(regexp_options(&inner.options).as_deref());
            if parts_all_plain(&inner.parts) {
                Node::MatchLastLineNode {
                    flags,
                    span: whole,
                    opening_loc: span(&inner.begin_l),
                    content_loc: parts_span(&inner.parts, whole),
                    closing_loc: span(&inner.end_l),
                    unescaped: concat_str_bytes(&inner.parts),
                }
            } else {
                Node::InterpolatedMatchLastLineNode {
                    flags,
                    span: whole,
                    opening_loc: span(&inner.begin_l),
                    parts: inner.parts.iter().map(|p| interp_part(p, pool)).collect(),
                    closing_loc: span(&inner.end_l),
                }
            }
        }
        // Unreachable in valid trees: keep the lowered condition.
        other => conv(other, pool),
    }
}

/// `^var` pins to variables, `^(expr)` pins to parenthesized expressions.
fn lower_pin(inner: &Pin, node: &Mri, pool: &mut Pool) -> Node {
    let whole = espan(node);
    match &*inner.var {
        Mri::Begin(begin) => {
            let body = match begin.statements.as_slice() {
                [] => Node::StatementsNode {
                    flags: 0,
                    span: whole,
                    body: Vec::new(),
                },
                [single] => conv(single, pool),
                _ => Node::StatementsNode {
                    flags: 0,
                    span: espan(&inner.var),
                    body: begin
                        .statements
                        .iter()
                        .map(|stmt| conv(stmt, pool))
                        .collect(),
                },
            };
            Node::PinnedExpressionNode {
                flags: 0,
                span: whole,
                expression: Box::new(body),
                operator_loc: span(&inner.selector_l),
                lparen_loc: reqspan(&begin.begin_l, whole),
                rparen_loc: reqspan(&begin.end_l, whole),
            }
        }
        _ => Node::PinnedVariableNode {
            flags: 0,
            span: whole,
            variable: Box::new(conv(&inner.var, pool)),
            operator_loc: span(&inner.selector_l),
        },
    }
}

/// Rest-split array pattern to `ArrayPatternNode`.
fn lower_array_pattern(
    elements: &[Mri],
    constant: Option<Box<Node>>,
    implicit_rest: bool,
    begin_l: &Option<Loc>,
    end_l: &Option<Loc>,
    node: &Mri,
    pool: &mut Pool,
) -> Node {
    let whole = espan(node);
    let at = elements
        .iter()
        .position(|el| matches!(el, Mri::MatchRest(_)));
    let (requireds, rest, posts) = match at {
        None => (
            elements.iter().map(|el| conv(el, pool)).collect(),
            implicit_rest.then(|| {
                Box::new(Node::ImplicitRestNode {
                    flags: 0,
                    span: whole,
                })
            }),
            Vec::new(),
        ),
        Some(index) => {
            let requireds = elements[..index].iter().map(|el| conv(el, pool)).collect();
            let rest = match &elements[index] {
                Mri::MatchRest(inner) => Some(Box::new(Node::SplatNode {
                    flags: 0,
                    span: espan(&elements[index]),
                    operator_loc: span(&inner.operator_l),
                    expression: inner
                        .name
                        .as_deref()
                        .map(|name| Box::new(target(name, pool))),
                })),
                // Unreachable: position found a rest above.
                other => Some(Box::new(conv(other, pool))),
            };
            let posts = elements[index + 1..]
                .iter()
                .map(|el| conv(el, pool))
                .collect();
            (requireds, rest, posts)
        }
    };
    Node::ArrayPatternNode {
        flags: 0,
        span: whole,
        constant,
        requireds,
        rest,
        posts,
        opening_loc: ospan(begin_l),
        closing_loc: ospan(end_l),
    }
}

/// `*, mid, *` patterns to `FindPatternNode`.
fn lower_find_pattern(inner: &FindPattern, node: &Mri, pool: &mut Pool) -> Node {
    let whole = espan(node);
    let mut elements = inner.elements.iter();
    let left = elements
        .next()
        .map(|el| find_end(el, pool))
        .unwrap_or_else(|| Node::SplatNode {
            flags: 0,
            span: whole,
            operator_loc: whole,
            expression: None,
        });
    // Collect to split the trailing rest from the middle patterns.
    let tail: Vec<&Mri> = elements.collect();
    let (middle, right) = match tail.as_slice() {
        [] => (
            Vec::new(),
            Node::SplatNode {
                flags: 0,
                span: whole,
                operator_loc: whole,
                expression: None,
            },
        ),
        [..] => {
            let (last, rest) = tail
                .split_last()
                .map_or((None, &[][..]), |(l, r)| (Some(*l), r));
            let right = last
                .map(|el| find_end(el, pool))
                .unwrap_or(Node::SplatNode {
                    flags: 0,
                    span: whole,
                    operator_loc: whole,
                    expression: None,
                });
            (rest.iter().map(|el| conv(el, pool)).collect(), right)
        }
    };
    Node::FindPatternNode {
        flags: 0,
        span: whole,
        constant: None,
        left: Box::new(left),
        requireds: middle,
        right: Box::new(right),
        opening_loc: ospan(&inner.begin_l),
        closing_loc: ospan(&inner.end_l),
    }
}

/// One `*` end of a find pattern (rest name binds through targets).
fn find_end(element: &Mri, pool: &mut Pool) -> Node {
    match element {
        Mri::MatchRest(inner) => Node::SplatNode {
            flags: 0,
            span: espan(element),
            operator_loc: span(&inner.operator_l),
            expression: inner
                .name
                .as_deref()
                .map(|name| Box::new(target(name, pool))),
        },
        // Unreachable in valid trees: keep the lowered node as the end.
        other => conv(other, pool),
    }
}

/// Hash patterns to `HashPatternNode`; `**nil` is the no-keywords marker.
fn lower_hash_pattern(
    elements: &[Mri],
    constant: Option<Box<Node>>,
    begin_l: &Option<Loc>,
    end_l: &Option<Loc>,
    node: &Mri,
    pool: &mut Pool,
) -> Node {
    let whole = espan(node);
    let mut lowered = Vec::with_capacity(elements.len());
    let mut rest = None;
    for element in elements {
        match element {
            Mri::Pair(inner) => lowered.push(Node::AssocNode {
                flags: 0,
                span: espan(element),
                key: Box::new(conv(&inner.key, pool)),
                value: Box::new(conv(&inner.value, pool)),
                operator_loc: Some(span(&inner.operator_l)),
            }),
            Mri::MatchRest(inner) => {
                rest = Some(Box::new(Node::AssocSplatNode {
                    flags: 0,
                    span: espan(element),
                    value: inner
                        .name
                        .as_deref()
                        .map(|name| Box::new(target(name, pool))),
                    operator_loc: span(&inner.operator_l),
                }));
            }
            Mri::MatchNilPattern(inner) => {
                rest = Some(Box::new(Node::NoKeywordsParameterNode {
                    flags: 0,
                    span: espan(element),
                    operator_loc: span(&inner.operator_l),
                    keyword_loc: span(&inner.name_l),
                }));
            }
            // Shorthand `{a:}` is a bare `MatchVar` (key implicit from the
            // name); expand it to the `AssocNode` Prism parses.
            Mri::MatchVar(inner) => lowered.push(Node::AssocNode {
                flags: 0,
                span: espan(element),
                key: Box::new(Node::SymbolNode {
                    flags: 0,
                    span: span(&inner.name_l),
                    opening_loc: None,
                    value_loc: Some(span(&inner.name_l)),
                    closing_loc: None,
                    unescaped: inner.name.as_bytes().to_vec(),
                }),
                value: Box::new(Node::LocalVariableTargetNode {
                    flags: 0,
                    span: espan(element),
                    name: sym(pool, &inner.name),
                    depth: 0,
                }),
                operator_loc: None,
            }),
            // Unreachable in valid trees: keep the lowered node inline.
            other => lowered.push(conv(other, pool)),
        }
    }
    Node::HashPatternNode {
        flags: 0,
        span: whole,
        constant,
        elements: lowered,
        rest,
        opening_loc: ospan(begin_l),
        closing_loc: ospan(end_l),
    }
}

/// `Const(pattern)` sets the constant on the inner array/hash pattern.
fn lower_const_pattern(inner: &ConstPattern, node: &Mri, pool: &mut Pool) -> Node {
    let whole = espan(node);
    let constant = Some(Box::new(conv(&inner.const_, pool)));
    match conv(&inner.pattern, pool) {
        Node::ArrayPatternNode {
            requireds,
            rest,
            posts,
            opening_loc,
            closing_loc,
            ..
        } => Node::ArrayPatternNode {
            flags: 0,
            span: whole,
            constant,
            requireds,
            rest,
            posts,
            opening_loc,
            closing_loc,
        },
        Node::HashPatternNode {
            elements,
            rest,
            opening_loc,
            closing_loc,
            ..
        } => Node::HashPatternNode {
            flags: 0,
            span: whole,
            constant,
            elements,
            rest,
            opening_loc,
            closing_loc,
        },
        Node::FindPatternNode {
            left,
            requireds,
            right,
            opening_loc,
            closing_loc,
            ..
        } => Node::FindPatternNode {
            flags: 0,
            span: whole,
            constant,
            left,
            requireds,
            right,
            opening_loc,
            closing_loc,
        },
        // Unreachable in valid trees: keep the lowered pattern as-is.
        other => other,
    }
}
