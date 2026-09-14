//! P2.6 certification (alias/undef and `defined?` tranche): `verify`
//! compares `compile --frontend prism` against the pinned C golden, byte for
//! byte, in both strip modes. Exit `0` required. Pattern matching (`case/in`,
//! `in`/`=>`), `BEGIN`/`END` and flip-flop belong to later work and are
//! locked as diagnostics below, never as diverging bytes. `defined?`
//! back-reference reads and chain links that hit still-gated call forms stay
//! gated for the same reason.

use std::process::Command;

const SNIPPETS: &[(&str, &str)] = &[
    ("alias_bare_noval", "alias bar foo\nputs 1\n"),
    ("alias_bare_valued", "alias bar foo\n"),
    ("alias_sym_noval", "alias :bar :foo\nputs 1\n"),
    ("alias_sym_valued", "alias :bar :foo\n"),
    ("alias_mixed", "alias bar :foo\nputs 1\n"),
    ("undef_bare_noval", "undef foo\nputs 1\n"),
    ("undef_bare_valued", "undef foo\n"),
    ("undef_multi_bare", "undef foo, bar\nputs 1\n"),
    ("undef_triple_sym", "undef :a, :b, :c\nputs 1\n"),
    (
        "alias_in_if",
        "x = 1\nif x == 1 then alias bar foo\nelse alias baz qux\nend\nputs 1\n",
    ),
    (
        "undef_in_if",
        "x = 1\nif x == 1 then undef foo\nend\nputs 1\n",
    ),
    ("alias_then_undef", "alias bar foo\nundef bar\nputs 1\n"),
    (
        "alias_valued_if",
        "x = 1\ny = if x == 1 then alias bar foo\nelse alias baz qux\nend\nputs y\n",
    ),
    (
        "alias_undef_in_case",
        "x = 2\ny = case x\nwhen 1 then alias bar foo\nwhen 2 then undef bar\nelse puts 3\nend\nputs y\n",
    ),
    (
        "undef_in_while",
        "x = 1\nwhile x == 2 do undef foo\nend\nputs 1\n",
    ),
    ("defined_lvar", "x = 1\nputs defined?(x)\n"),
    ("defined_method", "puts defined?(foo)\n"),
    ("defined_method_args", "puts defined?(foo(bar))\n"),
    ("defined_const", "puts defined?(A)\n"),
    ("defined_const_path", "puts defined?(A::B)\n"),
    ("defined_const_path_toplevel", "puts defined?(::A)\n"),
    ("defined_ivar", "puts defined?(@x)\n"),
    ("defined_gvar", "puts defined?($x)\n"),
    ("defined_cvar", "puts defined?(@@x)\n"),
    ("defined_self", "puts defined?(self)\n"),
    ("defined_nil", "puts defined?(nil)\n"),
    ("defined_true", "puts defined?(true)\n"),
    ("defined_literal", "puts defined?(1)\n"),
    ("defined_string", "puts defined?(\"s\")\n"),
    ("defined_symbol", "puts defined?(:sym)\n"),
    ("defined_array", "puts defined?([1, 2])\n"),
    ("defined_hash", "puts defined?({a: 1})\n"),
    ("defined_range", "puts defined?(1..2)\n"),
    ("defined_yield", "puts defined?(yield)\n"),
    ("defined_super", "puts defined?(super)\n"),
    ("defined_assignment", "puts defined?(x = 1)\n"),
    ("defined_ivar_assignment", "puts defined?(@x = 1)\n"),
    ("defined_op_assignment", "puts defined?(x += 1)\n"),
    ("defined_or_assignment", "puts defined?(x ||= 1)\n"),
    ("defined_method_on", "puts defined?(x.foo)\n"),
    ("defined_method_on_args", "puts defined?(x.foo(1))\n"),
    ("defined_method_on_safe", "puts defined?(x&.foo)\n"),
    ("defined_method_on_self", "puts defined?(self.foo)\n"),
    ("defined_index_call", "puts defined?(x[1])\n"),
    ("defined_chain", "puts defined?(x.foo.bar)\n"),
    ("defined_chain_long", "puts defined?(x.foo.bar.baz)\n"),
    ("defined_const_path_on_expr", "puts defined?(x::A)\n"),
    ("defined_parens", "puts defined?((x))\n"),
    ("defined_parens_literal", "puts defined?((1))\n"),
    ("defined_begin_empty", "puts defined?(begin; end)\n"),
    ("defined_array_parts", "puts defined?([x, 1])\n"),
    ("defined_hash_parts", "puts defined?({a: x})\n"),
    ("defined_splat_arg", "puts defined?(foo(*a))\n"),
    ("defined_literal_block", "puts defined?(foo { })\n"),
    ("defined_logic", "puts defined?(x && y)\n"),
    ("defined_nested", "puts defined?(defined?(x))\n"),
    ("defined_receiver_const", "puts defined?(A.foo)\n"),
    ("defined_receiver_const_path", "puts defined?(A::B.foo)\n"),
    ("defined_receiver_ivar", "puts defined?(@x.foo)\n"),
    ("defined_receiver_gvar", "puts defined?($x.foo)\n"),
    ("defined_receiver_cvar", "puts defined?(@@x.foo)\n"),
    ("defined_receiver_chain", "puts defined?(Foo.bar.baz)\n"),
    ("defined_begin_rescue", "puts defined?(begin; 1; rescue; 2; end)\n"),
    ("defined_backref", "puts defined?($&)\n"),
    ("defined_numbered_ref", "puts defined?($1)\n"),
    ("defined_match_rest", "puts defined?($~)\nputs defined?($+)\nputs defined?($`)\n"),
    ("backref_match", "puts $&\n"),
    ("backref_prematch", "puts $`\n"),
    ("backref_postmatch", "puts $'\n"),
    ("backref_last_group", "puts $+\n"),
    ("backref_numbered", "puts $1\n"),
    ("backref_match_var", "puts $~\n"),
    ("backref_assign", "x = $&\nputs x\n"),
    ("backref_large_number", "puts $99999999999\n"),
];

/// Left-out syntax: compilation must fail with a diagnostic, and must never
/// emit bytes that diverge from the reference.
const GATED: &[(&str, &str, &str)] = &[
    (
        "defined_chain_splat_gated",
        "puts defined?(x.foo(*a).bar)\n",
        "complex arguments",
    ),
    (
        "defined_chain_block_gated",
        "puts defined?(x.foo { }.bar)\n",
        "block argument",
    ),
    (
        "case_match_gated",
        "x = 1\ncase x\nin 1 then puts 1\nelse puts 2\nend\n",
        "CaseMatchNode",
    ),
    (
        "match_predicate_gated",
        "x = 1\nif x in 1 then puts 1\nend\nputs 2\n",
        "MatchPredicateNode",
    ),
    (
        "match_required_gated",
        "x = 1\nx => 1\nputs 1\n",
        "MatchRequiredNode",
    ),
    (
        "preexec_gated",
        "BEGIN { puts 1 }\nputs 2\n",
        "PreExecutionNode",
    ),
    (
        "postexec_gated",
        "END { puts 1 }\nputs 2\n",
        "PostExecutionNode",
    ),
    (
        "flipflop_gated",
        "if 1..2 then puts 1\nend\nputs 2\n",
        "FlipFlopNode",
    ),
    (
        "alias_dynamic_gated",
        "alias :\"#{1}\" :foo\nputs 1\n",
        "dynamic symbol",
    ),
    (
        "undef_dynamic_gated",
        "undef :\"#{1}\"\nputs 1\n",
        "dynamic symbol",
    ),
];

fn carnelian() -> Command {
    Command::new(env!("CARGO_BIN_EXE_carnelian"))
}

fn check_identical(name: &str, source: &str) {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join(format!("{name}.rb"));
    std::fs::write(&input, source).expect("write snippet");

    // `verify` checks both strip modes against the same golden.
    let verify = carnelian()
        .arg("verify")
        .arg(&input)
        .output()
        .expect("run verify");
    assert_eq!(
        verify.status.code(),
        Some(0),
        "{name}: verify failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&verify.stdout),
        String::from_utf8_lossy(&verify.stderr)
    );

    // `compile` output must also match the golden file byte for byte.
    let golden = dir.path().join(format!("{name}.mrb"));
    let reference = carnelian()
        .arg("reference")
        .arg(&input)
        .arg("-o")
        .arg(&golden)
        .output()
        .expect("run reference");
    assert!(reference.status.success(), "{name}: reference failed");
    for strip in [false, true] {
        let out = dir.path().join(format!("{name}.{strip}.mrb"));
        let mut command = carnelian();
        command.arg("compile").arg(&input).arg("-o").arg(&out);
        if strip {
            command.arg("--strip");
        }
        let compiled = command.output().expect("run compile");
        assert_eq!(
            compiled.status.code(),
            Some(0),
            "{name} (strip={strip}): compile failed: {}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        assert_eq!(
            std::fs::read(&golden).expect("read golden"),
            std::fs::read(&out).expect("read output"),
            "{name} (strip={strip}): bytes diverge"
        );
    }
}

#[test]
fn p26_corpus_is_byte_identical() {
    for (name, source) in SNIPPETS {
        check_identical(name, source);
    }
}

#[test]
fn p26_deferred_syntax_is_gated() {
    for (name, source, marker) in GATED {
        let dir = tempfile::tempdir().expect("tempdir");
        let input = dir.path().join(format!("{name}.rb"));
        std::fs::write(&input, source).expect("write snippet");
        let out = dir.path().join(format!("{name}.mrb"));
        let compiled = carnelian()
            .arg("compile")
            .arg(&input)
            .arg("-o")
            .arg(&out)
            .output()
            .expect("run compile");
        assert_eq!(
            compiled.status.code(),
            Some(1),
            "{name}: expected a gating diagnostic"
        );
        let stderr = String::from_utf8_lossy(&compiled.stderr);
        assert!(
            stderr.contains(marker),
            "{name}: diagnostic lacks {marker:?}: {stderr}"
        );
    }
}
