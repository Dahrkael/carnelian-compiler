//! P2.5 certification (rescue/ensure, splat, kwargs, masgn): `verify` compares
//! `compile --frontend prism` against the pinned C golden, byte for byte, in
//! both strip modes. Exit `0` required. Pattern matching and the still-gated
//! assignment targets are locked as diagnostics below, never as diverging
//! bytes.

use std::process::Command;

const SNIPPETS: &[(&str, &str)] = &[
    // rescue/ensure/else/modifier
    ("begin_bare", "begin\n  1\nend\n"),
    ("begin_bare_empty", "begin\nend\n"),
    ("begin_bare_empty_expr", "x = begin\nend\nputs x\n"),
    ("rescue_bare", "begin\n  1\nrescue\n  2\nend\n"),
    (
        "rescue_typed",
        "begin\n  1\nrescue get_exception\n  2\nrescue other_exception\n  3\nend\n",
    ),
    (
        "rescue_typed_const",
        "begin\n  1\nrescue TypeError\n  2\nend\n",
    ),
    (
        "rescue_typed_const_path",
        "begin\n  1\nrescue Foo::Bar\n  2\nend\n",
    ),
    (
        "rescue_typed_const_ref",
        "begin\n  1\nrescue TypeError => e\n  e\nend\n",
    ),
    (
        "rescue_typed_const_multi",
        "begin\n  1\nrescue TypeError, ArgumentError\n  2\nend\n",
    ),
    ("rescue_else", "begin\n  1\nrescue\n  2\nelse\n  3\nend\n"),
    (
        "rescue_else_ensure",
        "begin\n  1\nrescue\n  2\nelse\n  3\nensure\n  4\nend\n",
    ),
    ("ensure_only", "begin\n  1\nensure\n  3\nend\n"),
    (
        "rescue_reference",
        "begin\n  1\nrescue get_a => e\n  e\nrescue get_b => f\n  f\nend\n",
    ),
    ("rescue_empty_body", "x = begin\n  1\nrescue\nend\nputs x\n"),
    ("begin_empty_body", "begin\nrescue\n  2\nend\nputs 1\n"),
    (
        "rescue_nested",
        "begin\n  begin\n    1\n  rescue\n    2\n  end\nrescue\n  3\nend\n",
    ),
    (
        "rescue_splat_classes",
        "begin\n  1\nrescue *get_list\n  2\nend\n",
    ),
    (
        "rescue_retry",
        "begin\n  1\nrescue get_a\n  retry\nrescue get_b\n  2\nend\n",
    ),
    ("rescue_modifier_assign", "x = 1 rescue 2\nputs x\n"),
    ("rescue_modifier_stmt", "1 rescue 2\nputs 3\n"),
    ("begin_expr", "x = begin\n  1\nrescue\n  2\nend\nputs x\n"),
    ("begin_expr_bare", "x = begin\n  10\nend\nputs x\n"),
    // call splats
    ("call_splat_only", "a = [1, 2]\nf(*a)\n"),
    ("call_splat_lead", "a = [1, 2]\nf(1, *a)\n"),
    ("call_splat_tail", "a = [1, 2]\nf(*a, 3)\n"),
    ("call_splat_multi", "a = [1]\nb = [2]\nf(*a, *b)\n"),
    ("call_splat_kw", "a = [1]\nf(*a, k: 2)\n"),
    // array splats
    ("array_splat_only", "a = [1, 2]\nx = [*a]\nputs x\n"),
    ("array_splat_lead", "a = [1, 2]\nx = [*a, 3]\nputs x\n"),
    ("array_splat_mid", "a = [1, 2]\nx = [0, *a, 3]\nputs x\n"),
    (
        "array_splat_multi",
        "a = [1]\nb = [2]\nx = [*a, 0, *b]\nputs x\n",
    ),
    ("array_splat_noval", "a = [1]\n[*a, 2]\nputs 3\n"),
    ("splat_standalone", "a = [1, 2]\nx = *a\nputs x\n"),
    // keyword arguments
    ("kwargs_single", "f(k: 1)\n"),
    ("kwargs_mixed", "f(1, k: 2)\n"),
    ("kwargs_multi", "f(a: 1, b: 2, c: 3)\n"),
    ("kwargs_double_splat", "h = {a: 1}\nf(**h)\n"),
    ("kwargs_double_mixed", "h = {a: 1}\nf(1, **h, b: 2)\n"),
    ("kwargs_double_then_kw", "h = {a: 1}\nf(**h, b: 2)\n"),
    ("kwargs_noval", "f(k: 1)\nputs 3\n"),
    // yield argument transport (positional, keyword, splat)
    ("yield_bare", "def m\n  yield\nend\nm { 1 }\n"),
    (
        "yield_pos",
        "def m\n  yield 1, 2\nend\nm { |a, b| puts a }\n",
    ),
    ("yield_kw", "def m\n  yield k: 1\nend\nm { 1 }\n"),
    ("yield_splat", "def m\n  yield *a\nend\nm { |x| puts x }\n"),
    ("yield_double_splat", "def m\n  yield **h\nend\nm { 1 }\n"),
    (
        "yield_maxargs",
        "def m\n  yield 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15\nend\nm { |a| a }\n",
    ),
    // multiple assignment
    ("masgn_array", "a, b = [1, 2]\nputs a\nputs b\n"),
    ("masgn_list", "a, b = 1, 2\nputs a\n"),
    ("masgn_valued", "a, b = 1, 2\n"),
    ("masgn_valued_splat", "a, *b = foo, 2, 3\n"),
    ("masgn_valued_call", "a, b = get_values\n"),
    (
        "masgn_splat_middle",
        "a, *b, c = [1, 2, 3, 4]\nputs a\nputs c\n",
    ),
    ("masgn_splat_first", "*a, b = [1, 2, 3]\nputs b\n"),
    ("masgn_splat_only", "*a = [1, 2]\nputs a\n"),
    ("masgn_implicit_rest", "a, = [1, 2]\nputs a\n"),
    (
        "masgn_rest_post",
        "a, *b, c, d = [1, 2, 3, 4, 5]\nputs a\nputs d\n",
    ),
    ("masgn_rights_only", "*_, a, b = [1, 2, 3]\nputs a\n"),
    ("masgn_more_lefts", "a, b, c = [1]\nputs a\nputs b\n"),
    ("masgn_more_elems", "a, b = [1, 2, 3]\nputs a\n"),
    ("masgn_swap", "a = 1\nb = 2\na, b = b, a\nputs a\n"),
    ("masgn_nested", "(a, b), c = [1, 2], 3\nputs a\n"),
    (
        "masgn_nested_pairs",
        "(a, b), (c, d) = [1, 2], [3, 4]\nputs a\n",
    ),
    ("masgn_splat_rhs", "c = [1, 2, 3]\na, b = *c\nputs a\n"),
    (
        "masgn_mixed_splat_rhs",
        "c = [2, 3]\na, b = 1, *c\nputs a\n",
    ),
    ("masgn_index_target", "a = [0]\na[0], b = 1, 2\nputs a[0]\n"),
    (
        "masgn_index_multi",
        "h = {}\nh[1, 2], x = 3, 4\nputs x\n",
    ),
    (
        "masgn_index_splat",
        "a = [0]\na[*[0]], x = 1, 2\nputs x\n",
    ),
    ("masgn_ivar_target", "a, @b = 1, 2\nputs @b\n"),
    ("masgn_ivar_rest", "a, *@b = 1, 2, 3\nputs @b[0]\n"),
    ("masgn_cvar_target", "@@a, @@b = 1, 2\nputs @@a\n"),
    ("masgn_gvar_target", "$a, $b = 1, 2\nputs $a\n"),
    ("masgn_const_target", "X, Y = 1, 2\nputs X\n"),
    (
        "masgn_call_target",
        "class Box25\n  attr_accessor :x, :y\nend\nb = Box25.new\nb.x, b.y = 1, 2\nputs b.x\n",
    ),
    (
        "masgn_self_call_target",
        "class Box25s\n  attr_accessor :x\n  def init\n    self.x, @y = 1, 2\n    puts @y\n  end\nend\nBox25s.new.init\n",
    ),
    (
        "masgn_nested_ivar",
        "(@a, @b), c = [1, 2], 3\nputs @a\nputs c\n",
    ),
];

/// Later-tranche syntax: compilation must fail with a diagnostic, and must
/// never emit bytes that diverge from the reference.
const GATED: &[(&str, &str, &str)] = &[
    (
        "masgn_const_path_target_gated",
        "class Foo25\nend\nFoo25::A, b = 1, 2\n",
        "ConstantPathTargetNode",
    ),
    ("for_gated", "for i in [1] do\n  puts i\nend\n", "ForNode"),
    (
        "case_match_gated",
        "x = 1\ncase x\nin 1 then puts 1\nend\n",
        "CaseMatchNode",
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
fn p25_corpus_is_byte_identical() {
    for (name, source) in SNIPPETS {
        check_identical(name, source);
    }
}

#[test]
fn p25_splat_flush_paths() {
    // A splat after a long positional run flushes the pending values into an
    // array (`ARRAY`), then the splat concatenates (`ARYCAT`).
    let args: Vec<String> = (0..20).map(|index| index.to_string()).collect();
    let source = format!("a = [1]\nf({}, *a)\n", args.join(", "));
    check_identical("splat_flush_positional", &source);

    // A splat followed by more values flushes, then appends them
    // (`ARYPUSH`).
    let source = "a = [1]\nf(*a, 0, 1, 2, 3)\n";
    check_identical("splat_flush_trailing", source);

    // A splat after a long run flushes and concatenates a wide array.
    let items: Vec<String> = (0..70).map(|index| index.to_string()).collect();
    let source = format!("x = [{}, *[99]]\nputs x\n", items.join(", "));
    check_identical("splat_flush_wide", &source);

    // Crossing `GEN_VAL_STACK_MAX` (99) flushes plain values mid-list.
    let args: Vec<String> = (0..99).map(|index| index.to_string()).collect();
    let source = format!("f({})\n", args.join(", "));
    check_identical("values_stack_limit_flush", &source);

    // More keywords than the small-hash limit pack into a variable hash.
    let kwargs: Vec<String> = (0..15).map(|index| format!("k{index}: {index}")).collect();
    let source = format!("f(1, {})\n", kwargs.join(", "));
    check_identical("kwargs_limit_flush", &source);
}

#[test]
fn p25_deferred_syntax_is_gated() {
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
