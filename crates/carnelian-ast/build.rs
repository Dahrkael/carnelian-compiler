//! Build script: reads the Prism `config.json` snapshot and emits owned AST types.
//!
//! Input `config/prism-1.9.0-config.json` is a byte copy of the file shipped
//! inside `ruby-prism 1.9.0` on crates.io (`vendor/prism-1.9.0/config.json`).
//! Output `$OUT_DIR/node_generated.rs` is included by `src/lib.rs`.
//!
//! Mapping (owned, no FFI, no `unsafe`, no `repr(C)`):
//! `node` -> `Box<Node>`, `node?` -> `Option<Box<Node>>`,
//! `node[]` -> `Vec<Node>`, `string` -> `Vec<u8>`,
//! `constant` -> `SymbolId`, `constant[]` -> `Vec<SymbolId>`,
//! `location` -> `Span`, `integer` -> `Integer`, flags -> `u16`.

use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
enum FieldType {
    #[serde(rename = "node")]
    Node,
    #[serde(rename = "node?")]
    OptionalNode,
    #[serde(rename = "node[]")]
    NodeList,
    #[serde(rename = "string")]
    String,
    #[serde(rename = "constant")]
    Constant,
    #[serde(rename = "constant?")]
    OptionalConstant,
    #[serde(rename = "constant[]")]
    ConstantList,
    #[serde(rename = "location")]
    Location,
    #[serde(rename = "location?")]
    OptionalLocation,
    #[serde(rename = "uint8")]
    UInt8,
    #[serde(rename = "uint32")]
    UInt32,
    #[serde(rename = "integer")]
    Integer,
    #[serde(rename = "double")]
    Double,
}

#[derive(Debug, Deserialize)]
struct NodeField {
    name: String,
    #[serde(rename = "type")]
    field_type: FieldType,
}

#[derive(Debug, Deserialize)]
struct FlagValue {
    name: String,
}

#[derive(Debug, Deserialize)]
struct Flags {
    name: String,
    #[serde(default)]
    values: Vec<FlagValue>,
}

#[derive(Debug, Deserialize)]
struct Node {
    name: String,

    // Kept for schema parity with Prism; per-flag accessors arrive with P2.
    #[allow(dead_code)]
    flags: Option<String>,
    #[serde(default)]
    fields: Vec<NodeField>,
}

#[derive(Debug, Deserialize)]
struct Config {
    nodes: Vec<Node>,
    flags: Vec<Flags>,
}

fn rust_field_name(name: &str) -> String {
    name.to_owned()
}

fn rust_type(field: &NodeField) -> &'static str {
    match field.field_type {
        FieldType::Node => "Box<Node>",
        FieldType::OptionalNode => "Option<Box<Node>>",
        FieldType::NodeList => "Vec<Node>",
        FieldType::String => "Vec<u8>",
        FieldType::Constant => "SymbolId",
        FieldType::OptionalConstant => "Option<SymbolId>",
        FieldType::ConstantList => "Vec<SymbolId>",
        FieldType::Location => "Span",
        FieldType::OptionalLocation => "Option<Span>",
        FieldType::UInt8 => "u8",
        FieldType::UInt32 => "u32",
        FieldType::Integer => "Integer",
        FieldType::Double => "f64",
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let config_path = manifest.join("config").join("prism-1.9.0-config.json");
    println!("cargo:rerun-if-changed={}", config_path.display());

    let file = std::fs::File::open(&config_path)?;
    let config: Config = serde_json::from_reader(file)?;

    let out_dir = std::env::var_os("OUT_DIR").expect("OUT_DIR is set");
    let dest = Path::new(&out_dir).join("node_generated.rs");
    let mut out = String::new();

    out.push_str("// Generated from prism-1.9.0 config.json. Do not edit.\n");
    out.push_str(&format!(
        "// Nodes: {}, flag groups: {}.\n",
        config.nodes.len(),
        config.flags.len()
    ));

    // Flag constants per group. Node-specific flags share the `u16` word
    // with the generic `NEWLINE` (bit 0) and `STATIC_LITERAL` (bit 1), so
    // group bit `n` is word bit `n + 2` (`pm_node_flags_t` in Prism).
    for group in &config.flags {
        out.push_str(&format!(
            "\n/// Flag bits for `{}` (Prism `u16` flags, 1:1 names).\n",
            group.name
        ));
        out.push_str(&format!("pub mod {} {{\n", to_snake(&group.name)));
        for (bit, value) in group.values.iter().enumerate() {
            out.push_str(&format!("    /// Prism flag `{}`.\n", value.name));
            out.push_str(&format!(
                "    pub const {}: u16 = 1 << {};\n",
                value.name,
                bit + 2
            ));
        }
        out.push_str("}\n");
    }

    // Node enum with owned struct variants.
    out.push_str("\n/// Owned Prism AST node (1:1 names and fields with Prism).\n");
    out.push_str("#[derive(Debug, Clone, PartialEq)]\n");
    out.push_str("pub enum Node {\n");
    for node in &config.nodes {
        out.push_str(&format!("    /// Prism `{}`.\n", node.name));
        if node.fields.is_empty() {
            out.push_str(&format!("    {} {{\n", node.name));
            out.push_str("        flags: u16,\n");
            out.push_str("        span: Span,\n");
            out.push_str("    },\n");
        } else {
            out.push_str(&format!("    {} {{\n", node.name));
            out.push_str("        flags: u16,\n");
            out.push_str("        span: Span,\n");
            for field in &node.fields {
                out.push_str(&format!(
                    "        {}: {},\n",
                    rust_field_name(&field.name),
                    rust_type(field)
                ));
            }
            out.push_str("    },\n");
        }
    }
    out.push_str("}\n");

    // Kind name + span + flags accessors (thin access core for the backend).
    out.push_str("\nimpl Node {\n");
    out.push_str("    /// Prism node kind name, 1:1 with `config.json`.\n");
    out.push_str("    #[must_use]\n");
    out.push_str("    pub fn kind_name(&self) -> &'static str {\n");
    out.push_str("        match self {\n");
    for node in &config.nodes {
        out.push_str(&format!(
            "            Node::{n} {{ .. }} => \"{n}\",\n",
            n = node.name
        ));
    }
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("\n    /// Node span (offsets into the source).\n");
    out.push_str("    #[must_use]\n");
    out.push_str("    pub const fn span(&self) -> Span {\n");
    out.push_str("        match *self {\n");
    for node in &config.nodes {
        out.push_str(&format!(
            "            Node::{n} {{ span, .. }} => span,\n",
            n = node.name
        ));
    }
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("\n    /// Raw Prism `u16` flags.\n");
    out.push_str("    #[must_use]\n");
    out.push_str("    pub const fn flags(&self) -> u16 {\n");
    out.push_str("        match *self {\n");
    for node in &config.nodes {
        out.push_str(&format!(
            "            Node::{n} {{ flags, .. }} => flags,\n",
            n = node.name
        ));
    }
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n");

    // Visitor with default child walk.
    out.push_str("\n/// Visitor over owned nodes with a default child walk.\n");
    out.push_str("pub trait Visit {\n");
    out.push_str("    /// Visit any node (dispatches to the typed method).\n");
    out.push_str("    fn visit(&mut self, node: &Node) {\n");
    out.push_str("        match node {\n");
    for node in &config.nodes {
        out.push_str(&format!(
            "            Node::{n} {{ .. }} => self.visit_{s}(node),\n",
            n = node.name,
            s = to_snake(&node.name)
        ));
    }
    out.push_str("        }\n");
    out.push_str("    }\n");
    for node in &config.nodes {
        out.push_str(&format!(
            "\n    /// Visit `{}` (default: walk owned children).\n",
            node.name
        ));
        out.push_str(&format!(
            "    fn visit_{}(&mut self, node: &Node) {{\n",
            to_snake(&node.name)
        ));
        out.push_str("        visit_children(self, node);\n");
        out.push_str("    }\n");
    }
    out.push_str("}\n");

    out.push_str("\n/// Walk direct owned children of `node`.\n");
    out.push_str("pub fn visit_children<V: Visit + ?Sized>(visitor: &mut V, node: &Node) {\n");
    out.push_str("    match node {\n");
    for node in &config.nodes {
        let child_fields: Vec<&NodeField> = node
            .fields
            .iter()
            .filter(|f| {
                matches!(
                    f.field_type,
                    FieldType::Node | FieldType::OptionalNode | FieldType::NodeList
                )
            })
            .collect();
        if child_fields.is_empty() {
            out.push_str(&format!("        Node::{} {{ .. }} => {{}}\n", node.name));
        } else {
            out.push_str(&format!("        Node::{} {{ ", node.name));
            for f in child_fields.iter() {
                out.push_str(&format!("{}, ", rust_field_name(&f.name)));
            }
            out.push_str(".. } => {\n");
            for f in child_fields {
                let name = rust_field_name(&f.name);
                match f.field_type {
                    FieldType::Node => {
                        out.push_str(&format!("            visitor.visit({name});\n"));
                    }
                    FieldType::OptionalNode => {
                        out.push_str(&format!(
                            "            if let Some(child) = {name} {{ visitor.visit(child); }}\n"
                        ));
                    }
                    FieldType::NodeList => {
                        out.push_str(&format!(
                            "            for child in {name} {{ visitor.visit(child); }}\n"
                        ));
                    }
                    _ => {}
                }
            }
            out.push_str("        }\n");
        }
    }
    out.push_str("    }\n");
    out.push_str("}\n");

    std::fs::write(&dest, out)?;
    Ok(())
}

fn to_snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(ch.to_lowercase().next().unwrap());
        } else {
            out.push(ch);
        }
    }
    out
}
