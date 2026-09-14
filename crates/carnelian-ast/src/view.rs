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

/// Method definition parts (`DefNode`). `required_params` holds the required
/// positional names in order; any optional/rest/post/keyword/block form is
/// gated (the accessor returns `None` like `call_args` does for splats).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefView<N> {
    /// Method name bytes.
    pub name: Vec<u8>,
    /// Explicit receiver (`def self.foo`, `None` for plain `def`).
    pub receiver: Option<N>,
    /// Required positional parameter names in order.
    pub required_params: Vec<Vec<u8>>,
    /// Body node (`None` for an empty body).
    pub body: Option<N>,
    /// Method-scope locals in order (includes parameters).
    pub locals: Vec<Vec<u8>>,
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

/// Explicit `super` call (`SuperNode` with plain positional arguments).
/// `args` is `None` for `super()` and `Some` (possibly empty) otherwise;
/// splat/keyword/forwarding forms and block arguments are gated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuperView<N> {
    /// Plain positional arguments (`None` for empty `super()`).
    pub args: Option<Vec<N>>,
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
}
