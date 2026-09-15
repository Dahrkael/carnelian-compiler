//! Static site writer: self-contained offline `dist/`.
//!
//! `emit_static` renders `index.html` (editor + AST + bytecode + console
//! tabs, pins footer from `PINS.md`), `app.js` (vanilla, no deps, curated
//! examples inlined) and copies `examples/*.rb`. The wasm module is built
//! separately by the generator binary (`cargo` + `wasm-bindgen` CLI).

use std::fs;
use std::path::{Path, PathBuf};

use crate::disasm::disassemble;
use crate::{compile_source, execute};
use carnelian_compiler::read_rite;

const PAGE_TEMPLATE: &str = include_str!("page.html");
const APP_TEMPLATE: &str = include_str!("app.js");

/// One curated example with host-baked outputs.
#[derive(Debug, Clone)]
pub struct ExampleInfo {
    /// File stem (`hello`).
    pub name: String,
    /// Ruby source.
    pub source: String,
    /// Expected captured stdout.
    pub stdout: String,
    /// Expected return `inspect`.
    pub result: String,
    /// Compiled RITE bytes as lowercase hex (host reference for verify).
    pub hex: String,
}

/// Pins footer rows (`field`, `pin`) parsed from `PINS.md`.
#[derive(Debug, Clone)]
pub struct SiteManifest {
    /// Rows of the compatibility tuple table.
    pub pins: Vec<(String, String)>,
    /// Curated examples in filename order.
    pub examples: Vec<ExampleInfo>,
}

/// Crate directory for locating `PINS.md` and `examples/` at build time.
pub fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Parse the compatibility tuple table out of `PINS.md`.
pub fn read_pins() -> Result<Vec<(String, String)>, String> {
    let path = crate_dir().join("../../PINS.md");
    let text = fs::read_to_string(&path).map_err(|error| format!("read PINS.md: {error}"))?;
    let mut rows = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with('|') || !line.ends_with('|') {
            continue;
        }
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if cells.len() < 4 {
            continue;
        }
        let (field, pin) = (cells[1], cells[2]);
        if field == "Field" || field.is_empty() || field.starts_with("---") {
            continue;
        }
        rows.push((field.to_string(), pin.to_string()));
    }
    if rows.is_empty() {
        return Err("no pins parsed from PINS.md".to_string());
    }
    Ok(rows)
}

/// Compile and execute every `examples/*.rb`, baking host outputs.
pub fn collect_examples() -> Result<Vec<ExampleInfo>, String> {
    let dir = crate_dir().join("examples");
    let mut names: Vec<String> = fs::read_dir(&dir)
        .map_err(|error| format!("read examples dir: {error}"))?
        .map(|entry| entry.map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_, _>>()
        .map_err(|error| format!("list examples: {error}"))?;
    names.retain(|name| name.ends_with(".rb"));
    names.sort();
    if names.is_empty() {
        return Err("no examples/*.rb found".to_string());
    }
    let mut out = Vec::new();
    for name in names {
        let source =
            fs::read_to_string(dir.join(&name)).map_err(|error| format!("read {name}: {error}"))?;
        let bytes = compile_source(&source)
            .map_err(|diags| format!("example {name} does not compile: {diags:?}"))?;
        let outcome =
            execute(&bytes).map_err(|error| format!("example {name} does not execute: {error}"))?;
        let model = read_rite(&bytes).map_err(|error| format!("read back {name}: {error:?}"))?;
        let _ = disassemble(&model);
        out.push(ExampleInfo {
            name: name.strip_suffix(".rb").unwrap_or(&name).to_string(),
            source,
            stdout: outcome.stdout,
            result: outcome.result,
            hex: to_hex(&bytes),
        });
    }
    Ok(out)
}

fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Escape a string for JSON double quotes.
pub fn json_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
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

/// Render the curated manifest as a JSON array literal.
pub fn manifest_json(examples: &[ExampleInfo]) -> String {
    let mut out = String::from("[");
    for (i, example) in examples.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"name\":\"{}\",\"source\":\"{}\",\"stdout\":\"{}\",\"result\":\"{}\",\"hex\":\"{}\"}}",
            json_escape(&example.name),
            json_escape(&example.source),
            json_escape(&example.stdout),
            json_escape(&example.result),
            example.hex,
        ));
    }
    out.push(']');
    out
}

fn pins_footer(pins: &[(String, String)]) -> String {
    let mut out = String::new();
    for (field, pin) in pins {
        out.push_str(&format!(
            "<span><b>{}</b>: {}</span>",
            html_escape(field),
            html_escape(pin)
        ));
    }
    out
}

fn html_escape(text: &str) -> String {
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

/// Emit `index.html`, `app.js` and `examples/*.rb` into `out_dir`.
pub fn emit_static(out_dir: &Path, manifest: &SiteManifest) -> Result<(), String> {
    fs::create_dir_all(out_dir).map_err(|error| format!("create out dir: {error}"))?;
    fs::create_dir_all(out_dir.join("examples"))
        .map_err(|error| format!("create examples dir: {error}"))?;
    let examples_json = manifest_json(&manifest.examples);
    let default_source = manifest
        .examples
        .first()
        .map(|example| example.source.clone())
        .unwrap_or_default();
    let page = PAGE_TEMPLATE
        .replace("{{PINS_FOOTER}}", &pins_footer(&manifest.pins))
        .replace("{{DEFAULT_SOURCE}}", &html_escape(&default_source));
    let app = APP_TEMPLATE.replace("{{EXAMPLES_JSON}}", &examples_json);
    fs::write(out_dir.join("index.html"), page).map_err(|error| format!("write index: {error}"))?;
    fs::write(out_dir.join("app.js"), app).map_err(|error| format!("write app.js: {error}"))?;
    for example in &manifest.examples {
        fs::write(
            out_dir.join(format!("examples/{}.rb", example.name)),
            &example.source,
        )
        .map_err(|error| format!("write example {}: {error}", example.name))?;
    }
    Ok(())
}

/// Collect pins plus examples in one step.
pub fn collect_manifest() -> Result<SiteManifest, String> {
    Ok(SiteManifest {
        pins: read_pins()?,
        examples: collect_examples()?,
    })
}
