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

/// Handler-facing node access. See the module docs.
pub trait BackendNode: AstNode + Sized {
    /// Integer literal value.
    fn integer_lit(&self) -> Option<IntegerLit> {
        None
    }

    /// Float literal value.
    fn float_lit(&self) -> Option<f64> {
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

    /// Positional call arguments of an arguments node. `None` when the node
    /// is absent or carries splat/keyword/forwarding forms.
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

    /// Interpolated string parts (`InterpolatedStringNode`).
    fn string_parts(&self) -> Option<Vec<Self>> {
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

    /// Local variable target (`LocalVariableTargetNode`).
    fn lvar_target(&self) -> Option<LvarRef> {
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

    /// Whether an arguments node carries `...` forwarding.
    fn args_forwarding(&self) -> bool {
        false
    }
}
