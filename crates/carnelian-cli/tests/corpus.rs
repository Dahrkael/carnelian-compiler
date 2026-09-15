//! Shared parity corpus (P3.3): the `SNIPPETS`/`GATED` tables factored out of
//! the P1/P2/roundtrip certification files, plus the synthetic generators.
//!
//! Tables are byte-identical copies, renamed per origin file. Test files share
//! this module via `#[path = "corpus.rs"]`; behavior is unchanged.

// Data module: each test target uses only its own tables.
#![allow(dead_code)]

/// `SNIPPETS` table from `p1_verify.rs`.
pub const P1_SNIPPETS: &[(&str, &str)] = &[
    ("empty", ""),
    ("puts_int", "puts 1\n"),
    ("arith", "puts 1 + 2 * 3\n"),
    ("neg_one", "puts -1\n"),
    ("sub_fusion", "puts 10 - 3\n"),
    ("int8", "puts 300\n"),
    ("int16", "puts 70000\n"),
    ("int64", "x = 3000000000\nputs x\n"),
    ("bigint", "puts 99999999999999999999999\n"),
    ("neg_bigint", "puts -99999999999999999999999\n"),
    ("bigint_hex", "puts 0xFFFFFFFFFFFFFFFFFF\n"),
    ("bigint_oct", "puts 0o7777777777777777777777\n"),
    (
        "bigint_bin",
        "puts 0b1111111111111111111111111111111111111111111111111111111111111111111111111\n",
    ),
    (
        "bigint_huge_hex",
        "puts 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF\n",
    ),
    ("neg_bigint_hex", "puts -0xFFFFFFFFFFFFFFFFFF\n"),
    (
        "bigint_hex_underscores",
        "puts 0xFF_FF_FF_FF_FF_FF_FF_FF_FF\n",
    ),
    ("float", "puts 1.5\n"),
    ("neg_float", "puts -2.5\n"),
    ("neg_zero_float", "puts -0.0\n"),
    ("string", "puts \"hello\"\n"),
    ("string_escape", "puts \"a\\nb\"\n"),
    ("string_empty", "puts \"\"\n"),
    ("symbol", "puts :sym\n"),
    ("nil_lit", "puts nil\n"),
    ("true_lit", "puts true\n"),
    ("self_lit", "puts self\n"),
    ("if_else", "if true then puts 1 else puts 2 end\n"),
    ("if_no_else", "x = nil\nif x then puts 1 end\nputs 2\n"),
    ("ternary", "puts(true ? 1 : 2)\n"),
    ("unless_mod", "puts 1 unless false\n"),
    ("logic_and", "puts(true && false)\n"),
    ("logic_or", "puts(false || 2)\n"),
    ("compare", "puts(1 < 2)\nputs(1 == 1)\n"),
    ("array", "puts [1, 2]\n"),
    ("array_empty", "puts []\n"),
    ("array_nested", "puts [[1]]\n"),
    ("lvars", "x = 1 + 2\nputs x * x\n"),
    (
        "while_loop",
        "i = 0\nwhile i < 3 do i = i + 1 end\nputs i\n",
    ),
    ("while_empty", "i = 0\nwhile i < 0 do end\nputs i\n"),
    (
        "until_loop",
        "i = 0\nuntil i > 2 do i = i + 1 end\nputs i\n",
    ),
    ("nil_check", "x = nil\nputs x.nil?\n"),
    ("if_empty_then", "x = nil\nif x then end\nputs 1\n"),
    ("if_empty_then_val", "x = nil\nif x then end\n"),
    (
        "if_empty_else",
        "y = nil\nif y then puts 1 else end\nputs 2\n",
    ),
];

/// `SNIPPETS` table from `p2_verify.rs`.
pub const P2_SNIPPETS: &[(&str, &str)] = &[
    ("hash_sym_rocket", "x = {a: 1, \"b\" => 2}\nputs x\n"),
    ("hash_empty", "x = {}\nputs x\n"),
    ("hash_empty_call", "puts({})\n"),
    ("hash_nested", "x = {a: {b: 1}}\nputs x\n"),
    ("hash_splat", "y = {b: 2}\nx = {a: 1, **y}\nputs x\n"),
    ("hash_splat_middle", "x = {a: 1, **nil, b: 2}\nputs x\n"),
    ("hash_splat_first", "x = {**nil, a: 1}\nputs x\n"),
    ("hash_multi_splat", "x = {**nil, a: 1, **nil}\nputs x\n"),
    ("hash_noval_splat", "{a: 1, **nil}\nputs 1\n"),
    ("hash_in_call", "puts({a: 1})\n"),
    ("hash_mixed_keys", "x = {1 => \"a\", :s => 2}\nputs x\n"),
    ("hash_noval", "{a: 1}\nputs 2\n"),
    (
        "case_basic",
        "x = 2\ncase x\nwhen 1 then puts 1\nwhen 2 then puts 2\nelse puts 3\nend\n",
    ),
    (
        "case_no_else",
        "x = 1\ncase x\nwhen 1 then puts 1\nend\nputs 2\n",
    ),
    (
        "case_empty_when",
        "x = 1\ncase x\nwhen 1 then\nelse puts 2\nend\nputs 3\n",
    ),
    (
        "case_no_predicate",
        "x = 1\ncase\nwhen x == 1 then puts 1\nelse puts 2\nend\n",
    ),
    (
        "case_valued",
        "x = 2\ny = case x\nwhen 1 then 10\nwhen 2 then 20\nelse 30\nend\nputs y\n",
    ),
    (
        "case_valued_empty",
        "x = 2\ny = case x\nwhen 1 then\nwhen 2 then 20\nend\nputs y\n",
    ),
    (
        "case_multi_cond",
        "x = 1\ncase x\nwhen 1, 2 then puts \"low\"\nelse puts \"high\"\nend\n",
    ),
    (
        "case_splat_when",
        "a = [1, 2]\nx = 1\ncase x\nwhen *a then puts 1\nelse puts 2\nend\n",
    ),
    (
        "case_in_call",
        "puts(case 1\nwhen 1 then \"one\"\nelse \"other\"\nend)\n",
    ),
    ("interp_basic", "name = \"bob\"\nputs \"hi #{name}\"\n"),
    ("interp_leading", "puts \"#{1} hi\"\n"),
    ("interp_only", "x = 1\nputs \"#{x}\"\n"),
    ("interp_nested", "x = \"a\"\nputs \"o#{\"i#{x}\"}e\"\n"),
    ("interp_noval", "\"hi #{1 + 2}\"\nputs 1\n"),
    ("interp_empty_embexpr", "puts \"#{}\"\n"),
    ("interp_noval_empty", "\"#{}\"\nputs 1\n"),
    ("interp_multi", "x = 1\ny = \"a\"\nputs \"a#{x}b#{y}c\"\n"),
    ("interp_side_effect", "\"hi #{puts 1}\"\nputs 2\n"),
];

/// `GATED` table from `p2_verify.rs`.
pub const P2_GATED: &[(&str, &str, &str)] = &[
    (
        "case_match_gated",
        "x = 1\ncase x\nin 1 then puts 1\nend\n",
        "CaseMatchNode",
    ),
    (
        "interp_symbol_gated",
        "x = 1\nputs :\"s#{x}\"\n",
        "InterpolatedSymbolNode",
    ),
];

/// `SNIPPETS` table from `p23_blocks.rs`.
pub const P23_SNIPPETS: &[(&str, &str)] = &[
    ("block_args", "puts [1, 2].map { |x| x + 1 }\n"),
    ("block_do", "[1].each do |x|\nputs x\nend\n"),
    ("block_noval", "[1].each { |x| puts x }\nputs 1\n"),
    ("block_noargs", "puts [1].map { 42 }\n"),
    ("block_empty", "puts [1].map { }\n"),
    ("block_empty_params", "[1].each { || puts 1 }\n"),
    ("block_multi", "[1, 2].each { |a, b| puts a }\n"),
    ("block_rest", "[1, 2].each { |*a| puts a }\n"),
    ("block_shadow", "x = 1\n[1].each { |x| puts x }\nputs x\n"),
    (
        "block_nested",
        "[1].each { |a| [2].each { |b| puts a + b } }\n",
    ),
    ("block_capture", "x = 10\n[1].each { puts x }\n"),
    ("block_write_capture", "x = 1\n[1].each { x = 2 }\nputs x\n"),
    ("lambda_basic", "f = -> { 1 }\nputs f.call\n"),
    ("lambda_args", "f = ->(x) { x + 1 }\nputs f.call(2)\n"),
    ("lambda_rest", "f = ->(*a) { puts a }\nf.call(1, 2)\n"),
    ("sym_proc", "puts [1, 2].map(&:to_s)\n"),
    ("numbered", "puts [1, 2].map { _1 + 1 }\n"),
    ("it_block", "puts [1, 2].map { it + 1 }\n"),
    ("semi_local", "[1].each { |x; y| y = x + 1\nputs y }\n"),
    ("block_destructure", "[1].each { |(a, b)| puts a }\n"),
    (
        "block_destructure_rest",
        "[1].each { |a, (b, c)| puts b }\n",
    ),
    ("block_optional", "[1].each { |x = 1| puts x }\n"),
    ("block_post", "[1, 2, 3].each { |a, *b, c| puts c }\n"),
    ("block_keyword", "[1].each { |x: 1| puts x }\n"),
    ("block_param", "[1].each { |&b| puts b }\n"),
    (
        "block_mixed",
        "[1].each { |a, b = 1, *c, d, e:, **f, &g| puts a }\n",
    ),
    (
        "lambda_optional",
        "f = ->(a, b = 1) { a + b }\nputs f.call(1)\n",
    ),
    (
        "lambda_keyword",
        "f = ->(a:, b: 2) { a + b }\nputs f.call(a: 1)\n",
    ),
    (
        "lambda_rest_block",
        "f = ->(*a, &b) { a }\nputs f.call(1)\n",
    ),
    (
        "lambda_mixed",
        "f = ->(a, *b, c, d: 1, &e) { puts a }\nf.call(1, 2, 3)\n",
    ),
];

/// `GATED` table from `p23_blocks.rs`.
pub const P23_GATED: &[(&str, &str, &str)] = &[
    ("yield_naked", "yield\n", "Invalid yield"),
    ("yield_args", "yield 1\n", "Invalid yield"),
];

/// `SNIPPETS` table from `p24_defclass.rs`.
pub const P24_SNIPPETS: &[(&str, &str)] = &[
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
    (
        "def_destructure_nested",
        "def foo(a, (b, (c, d)))\n  a + b + c + d\nend\nputs foo(1, [2, [3, 4]])\n",
    ),
    (
        "def_destructure_nested_wide",
        "def foo((a, (b, c, d)))\n  a\nend\nputs foo([1, [2, 3, 4]])\n",
    ),
    (
        "def_destructure_nested_deep",
        "def foo((a, (b, (c, d))))\n  a + b + c + d\nend\nputs foo([1, [2, [3, 4]]])\n",
    ),
    (
        "def_destructure_nested_post",
        "def foo(x, *y, (a, (b, c)))\n  a + b + c\nend\nputs foo(1, 2, [3, [4, 5]])\n",
    ),
    (
        "def_destructure_nested_rest",
        "def foo((a, (b, *c)))\n  b\nend\nputs foo([1, [2, 3, 4]])\n",
    ),
    ("def_endless_optional", "def foo(a = 1, &b) = a\nputs foo\n"),
    (
        "def_mixed",
        "def foo(a, b = 1, *c, d, e:, f: 2, **g, &h)\n  [a, b, c, d, e, f, g]\nend\nputs foo(1, 2, 3, 4, e: 5, x: 6) { 7 }\n",
    ),
    ("def_forwarding", "def foo(...)\n  1\nend\nputs foo(1, 2)\n"),
];

/// `GATED` table from `p24_defclass.rs`.
pub const P24_GATED: &[(&str, &str, &str)] = &[
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

/// `SNIPPETS` table from `p25_rescue.rs`.
pub const P25_SNIPPETS: &[(&str, &str)] = &[
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
    ("index_block_arg", "a = [0, 1]\nputs a[0, &b]\n"),
    // `...` forwarding calls
    (
        "call_forward",
        "def foo(...)\n  bar(...)\nend\ndef bar(*a)\n  a\nend\nputs foo(1, 2)\n",
    ),
    (
        "call_forward_args",
        "def foo(...)\n  bar(1, ...)\nend\ndef bar(*a)\n  a\nend\nputs foo(2)\n",
    ),
    (
        "call_forward_kwargs",
        "def foo(a, ...)\n  bar(...)\nend\ndef bar(*a)\n  a\nend\nputs foo(1, 2, k: 3)\n",
    ),
    (
        "call_forward_optional",
        "def foo(a = 1, ...)\n  bar(...)\nend\ndef bar(*a)\n  a\nend\nputs foo(2, 3)\n",
    ),
    (
        "call_forward_splat",
        "def foo(...)\n  bar(*a, ...)\nend\ndef bar(*a)\n  a\nend\na = [1]\nputs foo(2)\n",
    ),
    (
        "call_forward_block",
        "def foo(...)\n  [1].each { bar(...) }\nend\ndef bar(*a)\n  puts a[0]\nend\nfoo(9)\n",
    ),
    (
        "call_forward_self",
        "def foo(...)\n  self.bar(...)\nend\ndef bar(*a)\n  a\nend\nputs foo(1)\n",
    ),
    (
        "super_forward_args",
        "class A25f\n  def foo(*a)\n    a\n  end\nend\nclass B25f < A25f\n  def foo(...)\n    super(...)\n  end\nend\nputs B25f.new.foo(1)\n",
    ),
    (
        "super_forward_mixed",
        "class A25g\n  def foo(*a)\n    a\n  end\nend\nclass B25g < A25g\n  def foo(...)\n    super(1, ...)\n  end\nend\nputs B25g.new.foo(2)\n",
    ),
    (
        "super_forward_block",
        "class A25h\n  def foo(*a)\n    a\n  end\nend\nclass B25h < A25h\n  def foo(...)\n    [1].each { super(...) }\n  end\nend\nputs B25h.new.foo(1)\n",
    ),
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
    ("for_basic", "for i in [1, 2] do\n  puts i\nend\n"),
    ("for_no_do", "for i in [1]\n  puts i\nend\n"),
    (
        "for_multi_target",
        "for a, b in [[1, 2]] do\n  puts a\n  puts b\nend\n",
    ),
    (
        "for_nested",
        "for i in [1, 2] do\n  for j in [3] do\n    puts i + j\n  end\nend\n",
    ),
    (
        "for_lvar_collection",
        "x = [1, 2]\nfor i in x do\n  puts i\nend\n",
    ),
];

/// `GATED` table from `p25_rescue.rs`.
pub const P25_GATED: &[(&str, &str, &str)] = &[
    (
        "masgn_const_path_target_gated",
        "class Foo25\nend\nFoo25::A, b = 1, 2\n",
        "ConstantPathTargetNode",
    ),
    (
        "case_match_gated",
        "x = 1\ncase x\nin 1 then puts 1\nend\n",
        "CaseMatchNode",
    ),
    // Plain attribute writes gate on every frontend (no `gen_call_assign`
    // in the backend yet); the reference compiles them.
    (
        "index_write_gated",
        "a = [0]\na[0] = 1\n",
        "attribute assignment",
    ),
    (
        "attr_write_gated",
        "class Box25w\n  attr_accessor :x\nend\nb = Box25w.new\nb.x = 1\n",
        "attribute assignment",
    ),
];

/// `SNIPPETS` table from `p26_specials.rs`.
pub const P26_SNIPPETS: &[(&str, &str)] = &[
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
    ("defined_backref_recv", "puts defined?($&.foo)\n"),
    ("defined_numbered_recv", "puts defined?($1.bar)\n"),
    ("defined_backref_index", "puts defined?($&[0])\n"),
    ("defined_backref_chain", "puts defined?($&.foo.bar)\n"),
    ("defined_backref_splat", "puts defined?($&.foo(*a))\n"),
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

/// `GATED` table from `p26_specials.rs`.
pub const P26_GATED: &[(&str, &str, &str)] = &[
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

/// `SNIPPETS` table from `roundtrip.rs`.
pub const ROUNDTRIP_SNIPPETS: &[(&str, &str)] = &[
    ("empty", ""),
    ("puts_int", "puts 1\n"),
    ("arith", "puts 1 + 2 * 3\n"),
    ("int64_over_i32", "x = 3000000000\nputs x\n"),
    ("bigint_over_i64", "x = 99999999999999999999999\nputs x\n"),
    ("float", "x = 1.5\nputs x\n"),
    ("string", "puts \"hello\"\n"),
    ("symbol", "puts :sym\n"),
    ("array", "a = [1, 2, 3]\nputs a\n"),
    ("hash", "h = {a: 1, \"b\" => 2}\nputs h\n"),
    ("if_else", "if true then puts 1 else puts 2 end\n"),
    ("while_loop", "i = 0\nwhile i < 3 do i += 1 end\nputs i\n"),
    ("def_call", "def add(a, b)\n  a + b\nend\nputs add(1, 2)\n"),
    (
        "class_def",
        "class Foo\n  def bar\n    42\n  end\nend\nputs Foo.new.bar\n",
    ),
    ("block", "[1, 2, 3].each { |x| puts x }\n"),
    ("rescue", "begin\n  foo\nrescue\n  bar\nend\n"),
    ("interp", "name = \"w\"\nputs \"hi #{name}\"\n"),
    ("splat", "a = [1, 2, 3]\nb = [*a, 4]\nputs b\n"),
    ("kwargs", "def f(a:, b: 2)\n  a + b\nend\nputs f(a: 1)\n"),
    ("lambda", "f = ->(x) { x * 2 }\nputs f.call(21)\n"),
    ("const", "X = 1\nputs X\n"),
    ("logic", "a = true && false || true\nputs a\n"),
    ("case", "case 1\nwhen 1 then puts 1\nelse puts 2\nend\n"),
];

/// Synthetic sources shared by the P2/P2.5 limit-path tests.
pub fn synthetic_cases() -> Vec<(&'static str, String)> {
    let pairs: Vec<String> = (0..70).map(|index| format!("{index} => {index}")).collect();
    let mut cases = vec![
        (
            "hash_big",
            format!("x = {{{}}}\nputs x\n", pairs.join(", ")),
        ),
        (
            "splat_flush_trailing",
            "a = [1]\nf(*a, 0, 1, 2, 3)\n".to_owned(),
        ),
        (
            "splat_flush_wide",
            format!(
                "x = [{}, *[99]]\nputs x\n",
                (0..70)
                    .map(|index| index.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        (
            "values_stack_limit_flush",
            format!(
                "f({})\n",
                (0..99)
                    .map(|index| index.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        (
            "kwargs_limit_flush",
            format!(
                "f(1, {})\n",
                (0..15)
                    .map(|index| format!("k{index}: {index}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
    ];
    // `hash_wide` reuses the `hash_big` pairs with 63 locals first.
    let mut wide = String::new();
    for index in 0..63 {
        wide.push_str(&format!("v{index} = {index}\n"));
    }
    wide.push_str(&format!("x = {{{}}}\nputs x\n", pairs.join(", ")));
    cases.push(("hash_wide", wide));
    // `splat_flush_positional` reuses a 20-argument run before the splat.
    cases.push((
        "splat_flush_positional",
        format!(
            "a = [1]\nf({}, *a)\n",
            (0..20)
                .map(|index| index.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    ));
    cases
}

/// One synthetic source by name (used to keep existing tests unchanged).
pub fn synthetic_source(name: &str) -> String {
    synthetic_cases()
        .into_iter()
        .find(|(known, _)| *known == name)
        .map(|(_, source)| source)
        .expect("known synthetic case")
}
