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

/// Program parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramView<N> {
    /// Top-level local names in order.
    pub locals: Vec<Vec<u8>>,
    /// Statement list.
    pub body: N,
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
}
