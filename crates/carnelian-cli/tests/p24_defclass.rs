//! P2.4 certification (def/class tranche): `verify` compares
//! `compile --frontend prism` against the pinned C golden, byte for byte, in
//! both strip modes. Exit `0` required. Operator writes and match-reference
//! reads belong to other tranches and are locked as diagnostics below, never
//! as diverging bytes.

use std::process::Command;

const SNIPPETS: &[(&str, &str)] = &[
    ("def_call", "def foo\n  1\nend\nputs foo\n"),
    ("def_args", "def add(a, b)\n  a + b\nend\nputs add(1, 2)\n"),
    ("def_nested", "def outer\n  def inner\n    1\n  end\n  inner\nend\nputs outer\n"),
    ("def_endless", "def foo = 42\nputs foo\n"),
    ("def_self", "def self.foo\n  1\nend\nputs foo\n"),
    ("def_three_args", "def foo(a, b, c)\n  a + b + c\nend\nputs foo(1, 2, 3)\n"),
    ("class_methods", "class Foo\n  def bar\n    1\n  end\nend\nputs 1\n"),
    (
        "class_superclass",
        "class Foo < Object\n  def bar\n    1\n  end\nend\nputs 1\n",
    ),
    (
        "class_const",
        "class Foo\n  X = 1\n  def get\n    X\n  end\nend\nputs 1\n",
    ),
    ("module_basic", "module M\n  def foo\n    1\n  end\nend\nputs 1\n"),
    (
        "module_include",
        "module M\nend\nclass C\n  include M\nend\nputs C.include?(M)\n",
    ),
    (
        "sclass_self",
        "class << self\n  def foo\n    1\n  end\nend\nputs foo\n",
    ),
    ("const_write", "X = 1\nputs X\n"),
    (
        "const_path",
        "class Foo\nend\nFoo::Bar = 1\nputs Foo::Bar\n",
    ),
    ("const_rooted", "::Top = 1\nputs ::Top\n"),
    ("ivar_toplevel", "@x = 1\nputs @x\n"),
    (
        "ivar_methods",
        "class Foo\n  def set(v)\n    @x = v\n  end\n  def get\n    @x\n  end\nend\nputs 1\n",
    ),
    (
        "ivar_interp",
        "puts \"hi #@v\"\n",
    ),
    (
        "cvar_methods",
        "class Foo\n  @@x = 1\n  def get\n    @@x\n  end\nend\nputs 1\n",
    ),
    ("gvar_toplevel", "$x = 1\nputs $x\n"),
    (
        "super_explicit",
        "class A\n  def foo(a)\n    a + 1\n  end\nend\nclass B < A\n  def foo(a)\n    super(a)\n  end\nend\nputs B.new.foo(1)\n",
    ),
    (
        "super_bare",
        "class A\n  def foo(a)\n    a + 1\n  end\nend\nclass B < A\n  def foo(a)\n    super\n  end\nend\nputs B.new.foo(1)\n",
    ),
    (
        "super_empty",
        "class A\n  def foo\n    1\n  end\nend\nclass B < A\n  def foo\n    super()\n  end\nend\nputs B.new.foo\n",
    ),
    ("def_optional", "def foo(a = 1)\n  a\nend\nputs foo\nputs foo(2)\n"),
    (
        "def_optional_multi",
        "def foo(a, b = 1, c = 2)\n  a + b + c\nend\nputs foo(1)\nputs foo(1, 2, 3)\n",
    ),
    ("def_rest", "def foo(*a)\n  a\nend\nputs foo(1, 2)\n"),
    ("def_rest_named", "def foo(a, *b)\n  a\nend\nputs foo(1, 2, 3)\n"),
    ("def_rest_anon", "def foo(*)\n  1\nend\nputs foo(1, 2)\n"),
    ("def_post", "def foo(a, *b, c)\n  c\nend\nputs foo(1, 2, 3)\n"),
    ("def_kw_required", "def foo(a:)\n  a\nend\nputs foo(a: 1)\n"),
    ("def_kw_optional", "def foo(a: 1)\n  a\nend\nputs foo\nputs foo(a: 2)\n"),
    (
        "def_kw_mixed",
        "def foo(a:, b: 2)\n  a + b\nend\nputs foo(a: 1)\n",
    ),
    ("def_kw_rest", "def foo(**h)\n  h\nend\nputs foo(a: 1)\n"),
    ("def_kw_rest_anon", "def foo(**)\n  1\nend\nputs foo(a: 1)\n"),
    ("def_block", "def foo(&b)\n  b.call(1)\nend\nfoo { |x| puts x }\n"),
    ("def_block_anon", "def foo(&)\n  1\nend\nputs foo { 1 }\n"),
    ("def_destructure", "def foo((a, b))\n  a + b\nend\nputs foo([1, 2])\n"),
    (
        "def_destructure_rest",
        "def foo(a, (b, c))\n  a + b + c\nend\nputs foo(1, [2, 3])\n",
    ),
    (
        "def_destructure_splat",
        "def foo((a, *b))\n  a\nend\nputs foo([1, 2, 3])\n",
    ),
    ("def_endless_optional", "def foo(a = 1, &b) = a\nputs foo\n"),
    (
        "def_mixed",
        "def foo(a, b = 1, *c, d, e:, f: 2, **g, &h)\n  [a, b, c, d, e, f, g]\nend\nputs foo(1, 2, 3, 4, e: 5, x: 6) { 7 }\n",
    ),
    ("def_forwarding", "def foo(...)\n  1\nend\nputs foo(1, 2)\n"),
];

/// Deferred syntax: compilation must fail with a diagnostic, and must never
/// emit bytes that diverge from the reference.
const GATED: &[(&str, &str, &str)] = &[
    (
        "def_nested_destructure_gated",
        "def foo(a, (b, (c, d)))\n  a\nend\nputs foo(1, [2, [3, 4]])\n",
        "DefNode",
    ),
    ("ivar_op_gated", "@x = 1\n@x += 1\nputs @x\n", "InstanceVariableOperatorWriteNode"),
    (
        "ivar_or_gated",
        "@x = 1\n@x ||= 2\nputs @x\n",
        "InstanceVariableOrWriteNode",
    ),
    (
        "cvar_or_gated",
        "@@x = 1\n@@x ||= 2\nputs @@x\n",
        "ClassVariableOrWriteNode",
    ),
    ("const_or_gated", "X = 1\nX ||= 2\nputs X\n", "ConstantOrWriteNode"),
    (
        "super_splat_gated",
        "class A\n  def foo(a)\n    a\n  end\nend\nclass B < A\n  def foo(a)\n    super(*a)\n  end\nend\n",
        "super",
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
fn p24_corpus_is_byte_identical() {
    for (name, source) in SNIPPETS {
        check_identical(name, source);
    }
}

#[test]
fn p24_deferred_syntax_is_gated() {
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
