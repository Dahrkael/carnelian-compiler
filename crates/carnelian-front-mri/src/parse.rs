//! MRI parsing wrapper (P4): source bytes to diagnostics plus tree.
//!
//! Syntax past the 3.1.2 grammar ceiling fails here with a clear
//! diagnostic; the backend never sees it.

use lib_ruby_parser::nodes::{Block, Class, Def, Defs, Lvar, Module, Numblock, SClass, Send};
use lib_ruby_parser::traverse::visitor::{visit_def, visit_send, Visitor};
use lib_ruby_parser::{Loc, Node as Mri, Parser, ParserOptions};

/// One parse diagnostic with byte offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDiagnostic {
    /// Message text.
    pub message: String,
    /// Start byte offset.
    pub start: u32,
    /// End byte offset.
    pub end: u32,
}

/// Parsed program: error list plus the MRI tree on success.
pub struct Parsed {
    result: lib_ruby_parser::ParserResult,
    extra: Vec<ParseDiagnostic>,
}

impl Parsed {
    /// Parser errors (warnings do not fail compilation).
    pub fn errors(&self) -> Vec<ParseDiagnostic> {
        let mut out: Vec<ParseDiagnostic> = self
            .result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.is_error() && !is_tolerated(diagnostic))
            .map(|diagnostic| ParseDiagnostic {
                message: diagnostic.render_message(),
                start: diagnostic.loc.begin as u32,
                end: diagnostic.loc.end as u32,
            })
            .collect();
        out.extend(self.extra.iter().cloned());
        out
    }

    /// Root of the MRI tree (`None` for empty input).
    pub fn root(&self) -> Option<&Mri> {
        self.result.ast.as_deref()
    }
}

/// Parse Ruby source with the pinned `lib-ruby-parser`.
pub fn parse(source: &[u8]) -> Parsed {
    let options = ParserOptions {
        buffer_name: "(eval)".to_string(),
        decoder: None,
        record_tokens: false,
    };
    let result = Parser::new(source.to_vec(), options).do_parse();
    let mut extra = Vec::new();
    if let Some(ast) = result.ast.as_deref() {
        extra.extend(find_it_param(ast));
    }
    Parsed { result, extra }
}

/// Errors the reference tolerates: `def t(foo = foo)` carries a
/// `CircularArgumentReference` diagnostic in both parsers (Prism:
/// `PM_ERR_PARAMETER_CIRCULAR`), yet the pinned reference compiles it, so
/// we demote it to a warning to stay byte-identical.
fn is_tolerated(diagnostic: &lib_ruby_parser::Diagnostic) -> bool {
    matches!(
        diagnostic.message,
        lib_ruby_parser::DiagnosticMessage::CircularArgumentReference { .. }
    )
}

/// Bare `it` reads inside blocks (`it` is Ruby 3.4; the 3.1 grammar reads
/// a plain send, which would silently diverge from the reference).
fn find_it_param(root: &Mri) -> Vec<ParseDiagnostic> {
    let mut scan = ItScan {
        blocks: Vec::new(),
        found: Vec::new(),
    };
    scan.visit(root);
    scan.found
        .iter()
        .map(|loc| ParseDiagnostic {
            message: "it block parameter requires Ruby 3.4 (grammar ceiling is 3.1.2)".to_string(),
            start: loc.begin as u32,
            end: loc.end as u32,
        })
        .collect()
}

/// Block stack: whether the block declares an explicit `it` parameter
/// (which shadows the implicit 3.4 one for its whole subtree).
struct ItScan {
    blocks: Vec<bool>,
    found: Vec<Loc>,
}

impl ItScan {
    /// Whether an argument list binds `it` explicitly (any named
    /// parameter form, not just `Arg`).
    fn declares_it(args: &Option<Box<Mri>>) -> bool {
        let Some(args) = args.as_deref() else {
            return false;
        };
        let Mri::Args(list) = args else {
            return false;
        };
        Self::binds_it_list(&list.args)
    }

    /// `it` bound anywhere in a parameter item list, through
    /// destructuring wrappers.
    fn binds_it_list(items: &[Mri]) -> bool {
        items.iter().any(|item| {
            let name = match item {
                Mri::Arg(inner) => Some(inner.name.as_str()),
                Mri::Optarg(inner) => Some(inner.name.as_str()),
                Mri::Restarg(inner) => inner.name.as_deref(),
                Mri::Blockarg(inner) => inner.name.as_deref(),
                Mri::Kwarg(inner) => Some(inner.name.as_str()),
                Mri::Kwoptarg(inner) => Some(inner.name.as_str()),
                Mri::Kwrestarg(inner) => inner.name.as_deref(),
                Mri::Procarg0(inner) => return Self::binds_it_list(&inner.args),
                Mri::Mlhs(inner) => return Self::binds_it_list(&inner.items),
                _ => None,
            };
            name == Some("it")
        })
    }
}

impl ItScan {
    /// Whether the innermost block leaves `it` implicit (no explicit
    /// `|it|`-family parameter shadowing it).
    fn implicit_it(&self) -> bool {
        matches!(self.blocks.last(), Some(false))
    }
}

impl Visitor for ItScan {
    fn on_block(&mut self, node: &Block) {
        // The call evaluates outside the block; the signature and body
        // run inside it.
        self.visit(&node.call);
        let declares = Self::declares_it(&node.args);
        self.blocks.push(declares);
        if let Some(args) = node.args.as_deref() {
            self.visit(args);
        }
        if let Some(body) = node.body.as_deref() {
            self.visit(body);
        }
        self.blocks.pop();
    }

    fn on_numblock(&mut self, node: &Numblock) {
        self.visit(&node.call);
        self.blocks.push(false);
        self.visit(&node.body);
        self.blocks.pop();
    }

    fn on_send(&mut self, node: &Send) {
        // A bare `it` call where 3.4 would read the implicit parameter.
        if self.implicit_it()
            && node.recv.is_none()
            && node.args.is_empty()
            && node.method_name == "it"
        {
            self.found.push(node.expression_l);
        }
        visit_send(self, node);
    }

    fn on_lvar(&mut self, node: &Lvar) {
        // Same rule for parser-bound reads: an explicit binder anywhere
        // outside (block param, assignment, def param, `for` index) loses
        // to the innermost implicit `it` under 3.4, diverging from the
        // plain local read MRI semantics produce.
        if self.implicit_it() && node.name == "it" {
            self.found.push(node.expression_l);
        }
    }

    // Methodist scopes seal off outer blocks, but their outer-evaluated
    // parts (superclass, `sclass` subject, singleton definee) keep the
    // enclosing stack; only signatures and bodies start sealed.
    fn on_def(&mut self, node: &Def) {
        let saved = std::mem::take(&mut self.blocks);
        visit_def(self, node);
        self.blocks = saved;
    }
    fn on_defs(&mut self, node: &Defs) {
        self.visit(&node.definee);
        let saved = std::mem::take(&mut self.blocks);
        if let Some(args) = node.args.as_deref() {
            self.visit(args);
        }
        if let Some(body) = node.body.as_deref() {
            self.visit(body);
        }
        self.blocks = saved;
    }
    fn on_class(&mut self, node: &Class) {
        self.visit(&node.name);
        if let Some(superclass) = node.superclass.as_deref() {
            self.visit(superclass);
        }
        let saved = std::mem::take(&mut self.blocks);
        if let Some(body) = node.body.as_deref() {
            self.visit(body);
        }
        self.blocks = saved;
    }
    fn on_module(&mut self, node: &Module) {
        self.visit(&node.name);
        let saved = std::mem::take(&mut self.blocks);
        if let Some(body) = node.body.as_deref() {
            self.visit(body);
        }
        self.blocks = saved;
    }
    fn on_s_class(&mut self, node: &SClass) {
        self.visit(&node.expr);
        let saved = std::mem::take(&mut self.blocks);
        if let Some(body) = node.body.as_deref() {
            self.visit(body);
        }
        self.blocks = saved;
    }
}
