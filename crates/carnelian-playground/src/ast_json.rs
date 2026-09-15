//! Owned-AST JSON for the playground viewer.
//!
//! A `Visit` walker emits one stable object per node (`kind`, byte span,
//! flags, short detail, children in traversal order). The raw `Debug`
//! dump stays available as a `<pre>` fallback.

use carnelian_ast::{visit_children, Node, SymbolPool, Visit};

/// One rendered node.
struct Frame {
    kind: &'static str,
    start: u32,
    end: u32,
    flags: u16,
    detail: String,
    children: Vec<String>,
}

/// Walker carrying the symbol pool for name details.
struct JsonWalker<'p> {
    pool: &'p SymbolPool,
    stack: Vec<Frame>,
    roots: Vec<String>,
}

impl<'p> JsonWalker<'p> {
    fn new(pool: &'p SymbolPool) -> Self {
        Self {
            pool,
            stack: Vec::new(),
            roots: Vec::new(),
        }
    }

    fn finish_frame(&mut self) -> String {
        let frame = self.stack.pop().expect("balanced walker stack");
        let mut out = String::from("{\"kind\":\"");
        out.push_str(frame.kind);
        out.push_str("\",\"span\":[");
        out.push_str(&frame.start.to_string());
        out.push(',');
        out.push_str(&frame.end.to_string());
        out.push_str("],\"flags\":");
        out.push_str(&frame.flags.to_string());
        out.push_str(",\"detail\":\"");
        out.push_str(&escape(&frame.detail));
        out.push_str("\",\"children\":[");
        out.push_str(&frame.children.join(","));
        out.push_str("]}");
        out
    }

    fn push_done(&mut self, rendered: String) {
        match self.stack.last_mut() {
            Some(parent) => parent.children.push(rendered),
            None => self.roots.push(rendered),
        }
    }
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Short scalar detail for common nodes; empty otherwise.
fn detail(node: &Node, pool: &SymbolPool) -> String {
    let name = |id: carnelian_ast::SymbolId| {
        pool.lookup(id)
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_else(|| format!("#{}", id.0))
    };
    match node {
        Node::IntegerNode { value, .. } => match value {
            carnelian_ast::Integer::I64(v) => v.to_string(),
            carnelian_ast::Integer::Fallback { raw } => {
                format!("bigint {}", String::from_utf8_lossy(raw))
            }
        },
        Node::FloatNode { value, .. } => value.to_string(),
        Node::StringNode { unescaped, .. } => {
            format!("{:?}", String::from_utf8_lossy(unescaped))
        }
        Node::SymbolNode { unescaped, .. } => {
            format!(":{}", String::from_utf8_lossy(unescaped))
        }
        Node::CallNode { name: id, .. } => name(*id),
        Node::LocalVariableReadNode { name: id, .. } => name(*id),
        Node::LocalVariableWriteNode { name: id, .. } => name(*id),
        Node::LocalVariableTargetNode { name: id, .. } => name(*id),
        Node::ConstantReadNode { name: id, .. } => name(*id),
        Node::ConstantWriteNode { name: id, .. } => name(*id),
        Node::InstanceVariableReadNode { name: id, .. } => name(*id),
        Node::InstanceVariableWriteNode { name: id, .. } => name(*id),
        Node::ClassVariableReadNode { name: id, .. } => name(*id),
        Node::GlobalVariableReadNode { name: id, .. } => name(*id),
        Node::DefNode { name: id, .. } => format!("def {}", name(*id)),
        Node::RequiredParameterNode { name: id, .. } => name(*id),
        _ => String::new(),
    }
}

impl Visit for JsonWalker<'_> {
    fn visit(&mut self, node: &Node) {
        let span = node.span();
        let frame = Frame {
            kind: node.kind_name(),
            start: span.start,
            end: span.end,
            flags: node.flags(),
            detail: detail(node, self.pool),
            children: Vec::new(),
        };
        self.stack.push(frame);
        visit_children(self, node);
        let rendered = self.finish_frame();
        self.push_done(rendered);
    }
}

/// Render the owned tree as stable JSON.
pub fn ast_to_json(node: &Node, pool: &SymbolPool) -> String {
    let mut walker = JsonWalker::new(pool);
    walker.visit(node);
    debug_assert_eq!(walker.roots.len(), 1);
    walker.roots.pop().unwrap_or_else(|| "{}".to_string())
}

/// Raw `Debug` fallback for the `<pre>` viewer.
pub fn debug_fallback(node: &Node) -> String {
    format!("{node:#?}")
}

/// Escape HTML special chars for `<pre>` embedding.
pub fn html_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}
