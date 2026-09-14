//! Build script: generates the borrowed-node `kind`/`flags` dispatch from the
//! shared Prism `config.json` snapshot (single source of truth with
//! `carnelian-ast`). Output `$OUT_DIR/kind_generated.rs`.

use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct Node {
    name: String,
}

#[derive(Debug, Deserialize)]
struct Config {
    nodes: Vec<Node>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let config_path = manifest
        .join("..")
        .join("carnelian-ast")
        .join("config")
        .join("prism-1.9.0-config.json");
    println!("cargo:rerun-if-changed={}", config_path.display());

    let file = std::fs::File::open(&config_path)?;
    let config: Config = serde_json::from_reader(file)?;

    let out_dir = std::env::var_os("OUT_DIR").expect("OUT_DIR is set");
    let dest = Path::new(&out_dir).join("kind_generated.rs");
    let mut out = String::new();
    out.push_str("// Generated from prism-1.9.0 config.json. Do not edit.\n");
    out.push_str("pub fn node_kind_name(node: &ruby_prism::Node<'_>) -> &'static str {\n");
    out.push_str("    match node {\n");
    for node in &config.nodes {
        out.push_str(&format!(
            "        ruby_prism::Node::{n} {{ .. }} => \"{n}\",\n",
            n = node.name
        ));
    }
    out.push_str("    }\n}\n");
    out.push_str("pub fn node_flags(node: &ruby_prism::Node<'_>) -> u16 {\n");
    out.push_str("    match node {\n");
    for node in &config.nodes {
        out.push_str(&format!(
            "        ruby_prism::Node::{n} {{ .. }} => node.as_{s}().expect(\"matched kind\").flags(),\n",
            n = node.name,
            s = to_snake(&node.name)
        ));
    }
    out.push_str("    }\n}\n");
    out.push_str(
        "pub fn clone_node<'pr>(node: &ruby_prism::Node<'pr>) -> ruby_prism::Node<'pr> {\n",
    );
    out.push_str("    match node {\n");
    for node in &config.nodes {
        out.push_str(&format!(
            "        ruby_prism::Node::{n} {{ parser, pointer, marker }} => ruby_prism::Node::{n} {{ parser: *parser, pointer: *pointer, marker: *marker }},\n",
            n = node.name
        ));
    }
    out.push_str("    }\n}\n");
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
