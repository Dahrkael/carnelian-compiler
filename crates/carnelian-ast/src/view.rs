//! Handler-facing node access shared by all frontends.
//!
//! The backend is written against `BackendNode`, implemented by the FFI
//! frontend for borrowed nodes and by `&Node` for owned ones. Default methods
//! return `None`; tranches override more of them. Children travel by value
//! (fresh wrappers on the FFI side, references on the owned side), so
//! recursion needs no lifetimes beyond the node itself.

use crate::AstNode;

/// Integer literal resolved by the frontend (mirrors `gen_pm_integer` inputs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegerLit {
    /// Fits in `i64`.
    I64(i64),
    /// Decimal digits without sign; `new_litbint` takes the sign separately.
    Bigint {
        /// Decimal digits.
        digits: Vec<u8>,
        /// Negative source literal.
        negative: bool,
    },
}

/// Trivial literal kinds (`true`, `false`, `nil`, `self`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimpleLit {
    True,
    False,
    Nil,
    SelfValue,
}

/// Local variable reference (`read`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LvarRef {
    /// Variable name bytes.
    pub name: Vec<u8>,
    /// Scope depth (`depth + for_depth` is applied by the frontend).
    pub depth: u32,
}

/// Local variable write target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LvarWrite<N> {
    /// Variable name bytes.
    pub name: Vec<u8>,
    /// Scope depth.
    pub depth: u32,
    /// Right-hand side (`None` for parameter targets).
    pub value: Option<N>,
}

/// Method call parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallView<N> {
    /// Method name bytes.
    pub name: Vec<u8>,
    /// Explicit receiver (`None` for implicit `self`).
    pub receiver: Option<N>,
    /// Arguments node (`None` when absent).
    pub args: Option<N>,
    /// Block argument (`None` when absent).
    pub block: Option<N>,
    /// `&.` safe navigation.
    pub safe_nav: bool,
    /// Attribute write (`foo.bar = x` diverts to the assign path).
    pub attr_write: bool,
}

/// `if`/`unless` parts. `None` is a null subtree; an empty vector is an
/// empty statements node (both occur in real parses).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IfView<N> {
    /// Condition (`None` only for the constant-folded path).
    pub predicate: Option<N>,
    /// Then branch statements.
    pub then_body: Option<Vec<N>>,
    /// Else clause node (`ElseNode` or nested `IfNode`).
    pub else_body: Option<N>,
    /// Inverted condition (`unless`).
    pub is_unless: bool,
}

/// `while`/`until` parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhileView<N> {
    /// Condition.
    pub predicate: Option<N>,
    /// Body statements.
    pub body: Option<Vec<N>>,
    /// `until` instead of `while`.
    pub is_until: bool,
    /// `begin...end while` modifier form.
    pub begin_modifier: bool,
}

/// `for` parts (`ForNode`). The frontend opens no scope of its own (Prism
/// has none); the backend opens the invisible child scope itself, so there
/// is no `locals` field here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForView<N> {
    /// Loop variable (`LocalVariableWriteNode` or `MultiTargetNode`).
    pub index: N,
    /// Collection expression.
    pub collection: N,
    /// Body statements (`None` for a null subtree).
    pub statements: Option<Vec<N>>,
}

/// `case` parts. `whens` holds the `WhenNode` children in order;
/// `else_body` is the `ElseNode` wrapper (`None` when absent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseView<N> {
    /// Subject (`None` for a bare `case`).
    pub predicate: Option<N>,
    /// `when` clauses in order.
    pub whens: Vec<N>,
    /// `else` clause node.
    pub else_body: Option<N>,
}

/// `when` parts. `body` is `None` for a null statements subtree and
/// `Some` (possibly empty) for a statements node, mirroring `IfView`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhenView<N> {
    /// Match conditions in order (`SplatNode` allowed).
    pub conditions: Vec<N>,
    /// Body statements.
    pub body: Option<Vec<N>>,
}

/// `case/in` parts (`CaseMatchNode`). `conditions` holds the `InNode`
/// children in order; `else_body` is the `ElseNode` wrapper (`None` when
/// absent), mirroring `CaseView`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseMatchView<N> {
    /// Subject (`None` for a bare `case`).
    pub predicate: Option<N>,
    /// `in` clauses in order.
    pub conditions: Vec<N>,
    /// `else` clause node.
    pub else_body: Option<N>,
}

/// `in` clause parts (`InNode`). `body` is `None` for a null statements
/// subtree and `Some` (possibly empty) for a statements node, mirroring
/// `WhenView`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InView<N> {
    /// Pattern expression (possibly an `if`/`unless` guard wrapper).
    pub pattern: N,
    /// Body statements.
    pub body: Option<Vec<N>>,
}

/// One-line match parts (`MatchPredicateNode` for `in`, `MatchRequiredNode`
/// for `=>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchView<N> {
    /// Matched value.
    pub value: N,
    /// Pattern expression.
    pub pattern: N,
}

/// Alternation pattern parts (`AlternationPatternNode`, `left | right`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlternationView<N> {
    /// Left alternative.
    pub left: N,
    /// Right alternative.
    pub right: N,
}

/// Capture pattern parts (`CapturePatternNode`, `pattern => target`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureView<N> {
    /// Inner pattern matched first.
    pub value: N,
    /// Capture target (`LocalVariableTargetNode`).
    pub target: N,
}

/// Array pattern parts (`ArrayPatternNode`). `rest` is the `SplatNode`
/// (`None` when absent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrayPatternView<N> {
    /// Leading constant (`Const[...]` form, `None` otherwise).
    pub constant: Option<N>,
    /// Elements before the rest.
    pub requireds: Vec<N>,
    /// Rest element.
    pub rest: Option<N>,
    /// Elements after the rest.
    pub posts: Vec<N>,
}

/// Hash pattern parts (`HashPatternNode`). `rest` is the `AssocSplatNode`
/// or `NoKeywordsParameterNode` (`**nil`, `None` when absent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashPatternView<N> {
    /// Leading constant (`Const[...]` form, `None` otherwise).
    pub constant: Option<N>,
    /// `AssocNode` elements in order.
    pub elements: Vec<N>,
    /// Rest element.
    pub rest: Option<N>,
}

/// Find pattern parts (`FindPatternNode`, `*pre, mid, *post`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindPatternView<N> {
    /// Leading constant (`Const(...)` form, `None` otherwise).
    pub constant: Option<N>,
    /// Leading rest (`SplatNode`).
    pub left: N,
    /// Middle elements searched for in the array.
    pub requireds: Vec<N>,
    /// Trailing rest (`SplatNode`).
    pub right: N,
}

/// Guard wrapper parts (`pattern if cond`, `pattern unless cond`). The
/// reference walks the `if`/`unless` node as the pattern itself, with the
/// statements holding the inner pattern and the predicate holding the
/// guard condition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardView<N> {
    /// Inner pattern (first statement of the wrapper).
    pub inner: N,
    /// Guard condition.
    pub condition: N,
    /// `unless` instead of `if`.
    pub is_unless: bool,
}

/// Range parts (`RangeNode`). Either side is `None` for an open-ended
/// range (`..3`, `1..`); `exclude_end` is the `...` flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeView<N> {
    /// Left operand (`None` for a beginless range).
    pub left: Option<N>,
    /// Right operand (`None` for an endless range).
    pub right: Option<N>,
    /// `...` instead of `..`.
    pub exclude_end: bool,
}

/// Regexp literal parts (`RegularExpressionNode`): the source bytes plus
/// the raw flags word. Node-specific bits share the word with the generic
/// `NEWLINE`/`STATIC_LITERAL` bits, so the backend masks them like
/// `RangeView::exclude_end` does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegexpView {
    /// Pattern source bytes (unescaped).
    pub unescaped: Vec<u8>,
    /// Raw Prism flags word.
    pub flags: u16,
}

/// Interpolated regexp parts (`InterpolatedRegularExpressionNode`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterpRegexpView<N> {
    /// String and embedded parts in order.
    pub parts: Vec<N>,
    /// Raw Prism flags word.
    pub flags: u16,
}

/// Program parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramView<N> {
    /// Top-level local names in order.
    pub locals: Vec<Vec<u8>>,
    /// Statement list.
    pub body: N,
}

/// `begin` parts. `statements` distinguishes a null body (`None`) from an
/// empty statements node (`Some` with an empty vector); the two emit
/// different bytes (`gen_begin`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeginView<N> {
    /// Body statements.
    pub statements: Option<Vec<N>>,
    /// No `rescue`, `else` or `ensure` clause.
    pub bare: bool,
    /// First `rescue` clause.
    pub rescue_clause: Option<N>,
    /// `else` clause.
    pub else_clause: Option<N>,
    /// `ensure` clause.
    pub ensure_clause: Option<N>,
}

/// One `rescue` clause (`RescueNode`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RescueView<N> {
    /// Exception classes (`SplatNode` allowed).
    pub exceptions: Vec<N>,
    /// `=> e` target (`None` when absent).
    pub reference: Option<N>,
    /// Clause body (`None` for a null subtree).
    pub statements: Option<Vec<N>>,
    /// Next `rescue` clause.
    pub subsequent: Option<N>,
}

/// `expr rescue expr` (`RescueModifierNode`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RescueModifierView<N> {
    /// Protected expression.
    pub expression: N,
    /// Fallback expression.
    pub rescue_expression: N,
}

/// `ensure` clause (`EnsureNode`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsureView<N> {
    /// Ensure body (`None` for a null subtree).
    pub statements: Option<Vec<N>>,
}

/// Multiple assignment (`MultiWriteNode`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiWriteView<N> {
    /// Targets before the splat.
    pub lefts: Vec<N>,
    /// Rest target (`SplatNode` or `ImplicitRestNode`).
    pub rest: Option<N>,
    /// Targets after the splat.
    pub rights: Vec<N>,
    /// Right-hand side.
    pub value: N,
}

/// Multiple assignment target (`MultiTargetNode`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiTargetView<N> {
    /// Targets before the splat.
    pub lefts: Vec<N>,
    /// Rest target (`SplatNode` or `ImplicitRestNode`).
    pub rest: Option<N>,
    /// Targets after the splat.
    pub rights: Vec<N>,
}

/// Block or lambda parts (`BlockNode`, `LambdaNode` share the layout).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockView<N> {
    /// Block locals (`locals` field).
    pub locals: Vec<Vec<u8>>,
    /// Parameters node (`BlockParametersNode`, `NumberedParametersNode`,
    /// `ItParametersNode`, or absent for an empty block).
    pub params: Option<N>,
    /// Body node (`StatementsNode`, `BeginNode`, or absent).
    pub body: Option<N>,
}

/// Lambda parts (same layout as blocks, distinct opcode).
pub type LambdaView<N> = BlockView<N>;

/// `yield` parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YieldView<N> {
    /// Arguments node (`ArgumentsNode`, absent for bare `yield`).
    pub args: Option<N>,
}

/// `BlockParametersNode` parts (`|...;...|` wrapper).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockParamView<N> {
    /// Inner `ParametersNode` (absent for `||` or `|;local|` forms).
    pub params: Option<N>,
    /// `;` block locals (`BlockLocalVariableNode` children).
    pub block_locals: Vec<N>,
}

/// `ParametersNode` parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamsView<N> {
    /// Mandatory positional parameters.
    pub requireds: Vec<N>,
    /// Optional positional parameters.
    pub optionals: Vec<N>,
    /// Rest parameter (`RestParameterNode` or `ImplicitRestNode`).
    pub rest: Option<N>,
    /// Post (post-rest mandatory) parameters.
    pub posts: Vec<N>,
    /// Keyword parameters.
    pub keywords: Vec<N>,
    /// Keyword rest or forwarding parameter.
    pub keyword_rest: Option<N>,
    /// Block parameter (`BlockParameterNode`).
    pub block: Option<N>,
}

/// Method definition parts (`DefNode`). `params` holds the `ParametersNode`
/// (`None` for a parameterless `def`); the backend ports `lambda_body`
/// (`blk=0`) for the full optional/rest/post/keyword/block layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefView<N> {
    /// Method name bytes.
    pub name: Vec<u8>,
    /// Explicit receiver (`def self.foo`, `None` for plain `def`).
    pub receiver: Option<N>,
    /// Parameter list (`None` for `def f` without parentheses).
    pub params: Option<N>,
    /// Body node (`None` for an empty body).
    pub body: Option<N>,
    /// Method-scope locals in order (includes parameters).
    pub locals: Vec<Vec<u8>>,
}

/// Keyword parameter parts: `default` is `None` for a required keyword
/// (`a:`) and `Some` for one with a default (`a: 1`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeywordParamView<N> {
    /// Keyword name bytes.
    pub name: Vec<u8>,
    /// Default value (`None` for a required keyword).
    pub default: Option<N>,
}

/// Class parts (`ClassNode`). The constant path is either a plain read
/// (`cpath_is_read`) or a scoped path with an optional parent (`None` parent
/// is the rooted `::Foo` form).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassView<N> {
    /// Class name bytes.
    pub name: Vec<u8>,
    /// True for `class Foo` (emits `LOADNIL` for the outer object).
    pub cpath_is_read: bool,
    /// Parent object for `class Foo::Bar` (`None` for plain or rooted).
    pub cpath_parent: Option<N>,
    /// Superclass expression (`None` for an implicit superclass).
    pub superclass: Option<N>,
    /// Body node (`None` for an empty body).
    pub body: Option<N>,
    /// Class-body locals in order.
    pub locals: Vec<Vec<u8>>,
}

/// Module parts (`ModuleNode`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleView<N> {
    /// Module name bytes.
    pub name: Vec<u8>,
    /// True for `module Foo` (emits `LOADNIL` for the outer object).
    pub cpath_is_read: bool,
    /// Parent object for `module Foo::Bar` (`None` for plain or rooted).
    pub cpath_parent: Option<N>,
    /// Body node (`None` for an empty body).
    pub body: Option<N>,
    /// Module-body locals in order.
    pub locals: Vec<Vec<u8>>,
}

/// Singleton class parts (`SingletonClassNode`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SclassView<N> {
    /// Subject expression (`class << expr`).
    pub expression: N,
    /// Body node (`None` for an empty body).
    pub body: Option<N>,
    /// Body locals in order.
    pub locals: Vec<Vec<u8>>,
}

/// Plain variable or constant write target (`name = value`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarWrite<N> {
    /// Variable or constant name bytes.
    pub name: Vec<u8>,
    /// Right-hand side.
    pub value: N,
}

/// Constant path write target (`Parent::Name = value`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstPathWrite<N> {
    /// Parent object (`None` for the rooted `::Name` form).
    pub parent: Option<N>,
    /// Constant name bytes.
    pub name: Vec<u8>,
    /// Right-hand side.
    pub value: N,
}

/// Constant path read (`Parent::Name`, `::Name` when parent is `None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstPathRead<N> {
    /// Parent object (`None` for the rooted form).
    pub parent: Option<N>,
    /// Constant name bytes.
    pub name: Vec<u8>,
}

/// Index assignment target (`recv[args] = value`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexTargetView<N> {
    /// Receiver expression.
    pub receiver: N,
    /// `ArgumentsNode` (`None` for a bare `recv[] = value`).
    pub args: Option<N>,
}

/// Call (attribute) assignment target (`recv.name = value`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallTargetView<N> {
    /// Receiver expression.
    pub receiver: N,
    /// Attribute name bytes (without the `=`).
    pub name: Vec<u8>,
}

/// `*OperatorWrite` on a scalar (`x += v`, `@x -= v`, `$g *= v`, `@@c /= v`,
/// `C %= v`): the name, the right-hand side, and the binary operator symbol
/// bytes. `depth` is the local depth for `LocalVariableOperatorWriteNode`
/// and `0` otherwise (the backend adds `for_depth` for locals, mirroring
/// `gen_assignment`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpWriteView<N> {
    /// Variable or constant name bytes.
    pub name: Vec<u8>,
    /// Scope depth (locals only).
    pub depth: u32,
    /// Right-hand side.
    pub value: N,
    /// Binary operator symbol bytes (e.g. `+`).
    pub binary_operator: Vec<u8>,
}

/// `*OrWrite`/`*AndWrite` on a scalar (`x ||= v`, `@x &&= v`, ...): the same
/// layout without the operator (the backend picks `JMPIF`/`JMPNOT` by kind).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicWriteView<N> {
    /// Variable or constant name bytes.
    pub name: Vec<u8>,
    /// Scope depth (locals only).
    pub depth: u32,
    /// Right-hand side.
    pub value: N,
}

/// `Call*Write` (`obj.foo += v`, `obj.foo ||= v`, `obj.foo &&= v`): the
/// receiver, the read/write names, the right-hand side, and safe navigation.
/// `binary_operator` is `Some` for operator writes and `None` for `||=` and
/// `&&=` (the backend picks the jump by kind there).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallWriteView<N> {
    /// Receiver expression (`None` for an implicit-`self` call).
    pub receiver: Option<N>,
    /// Getter name bytes.
    pub read_name: Vec<u8>,
    /// Setter name bytes (with the `=`).
    pub write_name: Vec<u8>,
    /// Binary operator symbol bytes (`None` for `||=`/`&&=`).
    pub binary_operator: Option<Vec<u8>>,
    /// Right-hand side.
    pub value: N,
    /// `&.` safe navigation.
    pub safe_nav: bool,
}

/// `Index*Write` (`a[i] += v`, `a[i] ||= v`, `a[i] &&= v`): the receiver, the
/// `ArgumentsNode`, the right-hand side, and the optional operator (same
/// `Some`/`None` split as calls).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexWriteView<N> {
    /// Receiver expression (`None` fails closed; writes always carry one).
    pub receiver: Option<N>,
    /// `ArgumentsNode` (`None` for a bare `recv[] op= value`).
    pub args: Option<N>,
    /// Right-hand side.
    pub value: N,
    /// Binary operator symbol bytes (`None` for `||=`/`&&=`).
    pub binary_operator: Option<Vec<u8>>,
}

/// Explicit `super` call (`SuperNode` with plain positional arguments,
/// plus `...` forwarding which rides `gen_values`).
/// `args` is `None` for `super()` and `Some` (possibly empty) otherwise;
/// splat/keyword forms and block arguments are gated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuperView<N> {
    /// Plain positional arguments (`None` for empty `super()`).
    pub args: Option<Vec<N>>,
    /// Explicit block (`super() { }` literal or `super(&block)` argument).
    pub block: Option<N>,
}

/// Handler-facing node access. See the module docs.
pub trait BackendNode: AstNode + Clone + Sized {
    /// Integer literal value.
    fn integer_lit(&self) -> Option<IntegerLit> {
        None
    }

    /// Float literal value.
    fn float_lit(&self) -> Option<f64> {
        None
    }

    /// Rational literal numerator and denominator (`RationalNode`).
    fn rational(&self) -> Option<(IntegerLit, IntegerLit)> {
        None
    }

    /// Imaginary literal numeric child (`ImaginaryNode`).
    fn imaginary(&self) -> Option<Self> {
        None
    }

    /// String literal bytes (unescaped).
    fn string_lit(&self) -> Option<Vec<u8>> {
        None
    }

    /// Symbol literal bytes (unescaped).
    fn symbol_lit(&self) -> Option<Vec<u8>> {
        None
    }

    /// Method call parts.
    fn call(&self) -> Option<CallView<Self>> {
        None
    }

    /// Call arguments of an arguments node, including splat and `...`
    /// forwarding forms (`gen_values` handles them); `None` when the node
    /// is absent. Keyword hashes ride along for the caller to split off.
    fn call_args(&self) -> Option<Vec<Self>> {
        None
    }

    /// `if`/`unless` parts.
    fn if_branch(&self) -> Option<IfView<Self>> {
        None
    }

    /// Array literal elements (`None` for splat forms).
    fn array_elements(&self) -> Option<Vec<Self>> {
        None
    }

    /// `while`/`until` parts.
    fn while_loop(&self) -> Option<WhileView<Self>> {
        None
    }

    /// `for` parts (`ForNode`).
    fn for_view(&self) -> Option<ForView<Self>> {
        None
    }

    /// Statement list.
    fn statements(&self) -> Option<Vec<Self>> {
        None
    }

    /// `else` branch statements (`None` when the node is not an `else`).
    fn else_body(&self) -> Option<Vec<Self>> {
        None
    }

    /// Program root.
    fn program(&self) -> Option<ProgramView<Self>> {
        None
    }

    /// Local variable read.
    fn lvar_read(&self) -> Option<LvarRef> {
        None
    }

    /// Local variable write.
    fn lvar_write(&self) -> Option<LvarWrite<Self>> {
        None
    }

    /// `&&`/`||` operands in order.
    fn logic(&self) -> Option<(Self, Self)> {
        None
    }

    /// `true`/`false`/`nil`/`self` marker.
    fn simple_lit(&self) -> Option<SimpleLit> {
        None
    }

    /// Hash literal elements (`AssocNode`/`AssocSplatNode` children).
    /// Shared by `HashNode` and `KeywordHashNode` like `gen_hash`.
    fn hash_elements(&self) -> Option<Vec<Self>> {
        None
    }

    /// Association key and value (`AssocNode`).
    fn assoc_pair(&self) -> Option<(Self, Self)> {
        None
    }

    /// Splatted hash value (`AssocSplatNode`); inner `None` is a bare `**`.
    fn assoc_splat_value(&self) -> Option<Option<Self>> {
        None
    }

    /// Splatted value (`SplatNode`); inner `None` is a bare `*`.
    fn splat_value(&self) -> Option<Option<Self>> {
        None
    }

    /// `case` parts.
    fn case_view(&self) -> Option<CaseView<Self>> {
        None
    }

    /// `when` parts.
    fn when_view(&self) -> Option<WhenView<Self>> {
        None
    }

    /// `case/in` parts.
    fn case_match_view(&self) -> Option<CaseMatchView<Self>> {
        None
    }

    /// `in` clause parts.
    fn in_view(&self) -> Option<InView<Self>> {
        None
    }

    /// `expr in pattern` parts.
    fn match_predicate_view(&self) -> Option<MatchView<Self>> {
        None
    }

    /// `expr => pattern` parts.
    fn match_required_view(&self) -> Option<MatchView<Self>> {
        None
    }

    /// Alternation pattern parts.
    fn alternation_view(&self) -> Option<AlternationView<Self>> {
        None
    }

    /// Capture pattern parts.
    fn capture_view(&self) -> Option<CaptureView<Self>> {
        None
    }

    /// Array pattern parts.
    fn array_pattern_view(&self) -> Option<ArrayPatternView<Self>> {
        None
    }

    /// Hash pattern parts.
    fn hash_pattern_view(&self) -> Option<HashPatternView<Self>> {
        None
    }

    /// Find pattern parts.
    fn find_pattern_view(&self) -> Option<FindPatternView<Self>> {
        None
    }

    /// Pinned variable (`PinnedVariableNode`): the `^name` operand.
    fn pinned_var(&self) -> Option<Self> {
        None
    }

    /// Pinned expression (`PinnedExpressionNode`): the `^(expr)` operand.
    fn pinned_expr(&self) -> Option<Self> {
        None
    }

    /// Guard wrapper parts (`pattern if cond` / `pattern unless cond`).
    fn guard_view(&self) -> Option<GuardView<Self>> {
        None
    }

    /// Interpolated string parts (`InterpolatedStringNode`).
    fn string_parts(&self) -> Option<Vec<Self>> {
        None
    }

    /// Interpolated symbol parts (`InterpolatedSymbolNode`).
    fn interp_symbol(&self) -> Option<Vec<Self>> {
        None
    }

    /// Backtick literal bytes (`XStringNode`, unescaped).
    fn xstring(&self) -> Option<Vec<u8>> {
        None
    }

    /// Interpolated backtick parts (`InterpolatedXStringNode`).
    fn interp_xstring(&self) -> Option<Vec<Self>> {
        None
    }

    /// Regexp literal source and flags (`RegularExpressionNode`).
    fn regexp(&self) -> Option<RegexpView> {
        None
    }

    /// Interpolated regexp parts and flags
    /// (`InterpolatedRegularExpressionNode`).
    fn interp_regexp(&self) -> Option<InterpRegexpView<Self>> {
        None
    }

    /// Embedded statements body (`EmbeddedStatementsNode`); `None` is a
    /// null statements subtree, `Some` (possibly empty) is the node.
    fn embedded_body(&self) -> Option<Vec<Self>> {
        None
    }

    /// Embedded variable read (`EmbeddedVariableNode`).
    fn embedded_var(&self) -> Option<Self> {
        None
    }

    /// Block parts (`BlockNode`).
    fn block_view(&self) -> Option<BlockView<Self>> {
        None
    }

    /// Lambda parts (`LambdaNode`).
    fn lambda_view(&self) -> Option<LambdaView<Self>> {
        None
    }

    /// `yield` parts (`YieldNode`); outer `None` is not a yield, inner
    /// `None` is a bare `yield` without arguments.
    fn yield_view(&self) -> Option<YieldView<Self>> {
        None
    }

    /// `BlockParametersNode` parts.
    fn block_param_view(&self) -> Option<BlockParamView<Self>> {
        None
    }

    /// `ParametersNode` parts.
    fn parameters_view(&self) -> Option<ParamsView<Self>> {
        None
    }

    /// Required parameter name (`RequiredParameterNode`).
    fn required_param_name(&self) -> Option<Vec<u8>> {
        None
    }

    /// Rest parameter name (`RestParameterNode`); outer `None` is not a
    /// rest node, inner `None` is an anonymous `*`.
    fn rest_param_name(&self) -> Option<Option<Vec<u8>>> {
        None
    }

    /// Optional parameter name and default (`OptionalParameterNode`).
    fn optional_param(&self) -> Option<(Vec<u8>, Self)> {
        None
    }

    /// Keyword parameter name and default (`RequiredKeywordParameterNode`
    /// yields `default: None`, `OptionalKeywordParameterNode` yields the
    /// default expression).
    fn keyword_param(&self) -> Option<KeywordParamView<Self>> {
        None
    }

    /// Keyword-rest name (`KeywordRestParameterNode`); outer `None` is not
    /// such a node, inner `None` is an anonymous `**`.
    fn keyword_rest_name(&self) -> Option<Option<Vec<u8>>> {
        None
    }

    /// Block parameter name (`BlockParameterNode`); outer `None` is not
    /// such a node, inner `None` is an anonymous `&`.
    fn block_param_name(&self) -> Option<Option<Vec<u8>>> {
        None
    }

    /// True for `&nil` (the method accepts no block, `MRC_ARGS_NOBLOCK`).
    fn block_param_noblock(&self) -> bool {
        false
    }

    /// Block-local name (`BlockLocalVariableNode`).
    fn block_local_name(&self) -> Option<Vec<u8>> {
        None
    }

    /// Numbered parameters maximum (`NumberedParametersNode`).
    fn numbered_max(&self) -> Option<u8> {
        None
    }

    /// Block argument (`BlockArgumentNode`); outer `None` is not a block
    /// argument, inner `None` is a bare `&`.
    fn block_arg(&self) -> Option<Option<Self>> {
        None
    }

    /// `it` read (`ItLocalVariableReadNode`).
    fn it_read(&self) -> Option<()> {
        None
    }

    /// Method definition (`DefNode`); `None` for complex parameter forms.
    fn def_view(&self) -> Option<DefView<Self>> {
        None
    }

    /// Class definition (`ClassNode`).
    fn class_view(&self) -> Option<ClassView<Self>> {
        None
    }

    /// Module definition (`ModuleNode`).
    fn module_view(&self) -> Option<ModuleView<Self>> {
        None
    }

    /// Singleton class (`SingletonClassNode`).
    fn sclass_view(&self) -> Option<SclassView<Self>> {
        None
    }

    /// Constant read name (`ConstantReadNode`).
    fn const_read(&self) -> Option<Vec<u8>> {
        None
    }

    /// Constant write (`ConstantWriteNode`).
    fn const_write(&self) -> Option<VarWrite<Self>> {
        None
    }

    /// Constant path read (`ConstantPathNode`).
    fn const_path(&self) -> Option<ConstPathRead<Self>> {
        None
    }

    /// Constant path write (`ConstantPathWriteNode`).
    fn const_path_write(&self) -> Option<ConstPathWrite<Self>> {
        None
    }

    /// Instance variable read name (`InstanceVariableReadNode`).
    fn ivar_read(&self) -> Option<Vec<u8>> {
        None
    }

    /// Instance variable write (`InstanceVariableWriteNode`).
    fn ivar_write(&self) -> Option<VarWrite<Self>> {
        None
    }

    /// Class variable read name (`ClassVariableReadNode`).
    fn cvar_read(&self) -> Option<Vec<u8>> {
        None
    }

    /// Class variable write (`ClassVariableWriteNode`).
    fn cvar_write(&self) -> Option<VarWrite<Self>> {
        None
    }

    /// Global variable read name (`GlobalVariableReadNode`).
    fn gvar_read(&self) -> Option<Vec<u8>> {
        None
    }

    /// Global variable write (`GlobalVariableWriteNode`).
    fn gvar_write(&self) -> Option<VarWrite<Self>> {
        None
    }

    /// Explicit `super` call (`SuperNode`); `None` for complex arguments.
    fn super_view(&self) -> Option<SuperView<Self>> {
        None
    }

    /// Bare `super` without arguments (`ForwardingSuperNode`); inner `Some`
    /// carries an explicit block (gated), `None` is plain pass-through.
    fn forwarding_super(&self) -> Option<Option<Self>> {
        None
    }

    /// Local variable target (`LocalVariableTargetNode`).
    fn lvar_target(&self) -> Option<LvarRef> {
        None
    }

    /// `alias` new and old names (`AliasMethodNode`).
    fn alias_pair(&self) -> Option<(Self, Self)> {
        None
    }

    /// `undef` name list (`UndefNode`).
    fn undef_list(&self) -> Option<Vec<Self>> {
        None
    }

    /// `defined?` operand (`DefinedNode`).
    fn defined_value(&self) -> Option<Self> {
        None
    }

    /// `return` operand (`ReturnNode`); outer `None` is not a return,
    /// inner `None` is a bare `return`.
    fn return_args(&self) -> Option<Option<Self>> {
        None
    }

    /// `break` operand (`BreakNode`); outer `None` is not a break, inner
    /// `None` is a bare `break`.
    fn break_args(&self) -> Option<Option<Self>> {
        None
    }

    /// `next` operand (`NextNode`); outer `None` is not a next, inner
    /// `None` is a bare `next`.
    fn next_args(&self) -> Option<Option<Self>> {
        None
    }

    /// Range parts (`RangeNode`).
    fn range_view(&self) -> Option<RangeView<Self>> {
        None
    }

    /// Implicit value (`ImplicitNode`).
    fn implicit_value(&self) -> Option<Self> {
        None
    }

    /// Match-write `=~` call (`MatchWriteNode`); targets occupy LVAR
    /// slots but emit no stores, so only the call emits code.
    fn match_write(&self) -> Option<Self> {
        None
    }

    /// Parentheses body (`ParenthesesNode`); `Some(None)` for empty `()`.
    fn parentheses_body(&self) -> Option<Option<Self>> {
        None
    }

    /// `begin` parts (`BeginNode`).
    fn begin_view(&self) -> Option<BeginView<Self>> {
        None
    }

    /// One `rescue` clause (`RescueNode`).
    fn rescue_view(&self) -> Option<RescueView<Self>> {
        None
    }

    /// `expr rescue expr` (`RescueModifierNode`).
    fn rescue_modifier_view(&self) -> Option<RescueModifierView<Self>> {
        None
    }

    /// `ensure` clause (`EnsureNode`).
    fn ensure_view(&self) -> Option<EnsureView<Self>> {
        None
    }

    /// Multiple assignment (`MultiWriteNode`).
    fn multi_write_view(&self) -> Option<MultiWriteView<Self>> {
        None
    }

    /// Multiple assignment target (`MultiTargetNode`).
    fn multi_target_view(&self) -> Option<MultiTargetView<Self>> {
        None
    }

    /// Instance variable target name (`InstanceVariableTargetNode`).
    fn ivar_target_name(&self) -> Option<Vec<u8>> {
        None
    }

    /// Class variable target name (`ClassVariableTargetNode`).
    fn cvar_target_name(&self) -> Option<Vec<u8>> {
        None
    }

    /// Global variable target name (`GlobalVariableTargetNode`).
    fn gvar_target_name(&self) -> Option<Vec<u8>> {
        None
    }

    /// Constant target name (`ConstantTargetNode`).
    fn const_target_name(&self) -> Option<Vec<u8>> {
        None
    }

    /// Constant path target (`ConstantPathTargetNode`): parent and name.
    fn const_path_target(&self) -> Option<(Option<Self>, Vec<u8>)> {
        None
    }

    /// Index assignment target (`IndexTargetNode`).
    fn index_target(&self) -> Option<IndexTargetView<Self>> {
        None
    }

    /// Call assignment target (`CallTargetNode`).
    fn call_target(&self) -> Option<CallTargetView<Self>> {
        None
    }

    /// Scalar operator write (`*OperatorWriteNode` on a local, ivar, gvar,
    /// cvar, or constant).
    fn op_write(&self) -> Option<OpWriteView<Self>> {
        None
    }

    /// Scalar `||=`/`&&=` write (`*OrWriteNode`/`*AndWriteNode` on a local,
    /// ivar, gvar, cvar, or constant).
    fn logic_write(&self) -> Option<LogicWriteView<Self>> {
        None
    }

    /// Call operator/`||=`/`&&=` write (`CallOperatorWriteNode`,
    /// `CallOrWriteNode`, `CallAndWriteNode`).
    fn call_write(&self) -> Option<CallWriteView<Self>> {
        None
    }

    /// Index operator/`||=`/`&&=` write (`IndexOperatorWriteNode`,
    /// `IndexOrWriteNode`, `IndexAndWriteNode`).
    fn index_write(&self) -> Option<IndexWriteView<Self>> {
        None
    }

    /// Whether an arguments node carries `...` forwarding.
    fn args_forwarding(&self) -> bool {
        false
    }

    /// Instance variable read name (`InstanceVariableReadNode`).
    fn instance_var_read_name(&self) -> Option<Vec<u8>> {
        None
    }

    /// Back-reference name (`BackReferenceReadNode`, e.g. `$&`).
    fn backref_name(&self) -> Option<Vec<u8>> {
        None
    }

    /// Numbered reference number (`NumberedReferenceReadNode`, e.g. `$1`).
    fn numbered_ref_number(&self) -> Option<u32> {
        None
    }

    /// Global variable read name (`GlobalVariableReadNode`).
    fn global_var_read_name(&self) -> Option<Vec<u8>> {
        None
    }

    /// Class variable read name (`ClassVariableReadNode`).
    fn class_var_read_name(&self) -> Option<Vec<u8>> {
        None
    }

    /// Constant read name (`ConstantReadNode`).
    fn constant_read_name(&self) -> Option<Vec<u8>> {
        None
    }

    /// Constant path parts (`ConstantPathNode`): parent and name.
    fn constant_path_parts(&self) -> Option<(Option<Self>, Vec<u8>)> {
        None
    }

    /// Raw positional arguments (`ArgumentsNode`) including splats and
    /// keyword hashes; also forwarding forms.
    fn raw_call_args(&self) -> Option<Vec<Self>> {
        None
    }

    /// Raw array elements (`ArrayNode`) including splats.
    fn raw_array_elements(&self) -> Option<Vec<Self>> {
        None
    }

    /// `__FILE__` path bytes (`SourceFileNode`).
    fn source_file(&self) -> Option<Vec<u8>> {
        None
    }

    /// `__LINE__` marker (`SourceLineNode` carries only flags and span).
    fn source_line(&self) -> Option<()> {
        None
    }

    /// `__ENCODING__` marker (`SourceEncodingNode` carries only flags and
    /// span).
    fn source_encoding(&self) -> Option<()> {
        None
    }
}
