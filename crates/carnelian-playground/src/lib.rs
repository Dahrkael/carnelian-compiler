//! Shared playground pipeline: parse, compile, gate, inspect.
//!
//! Used by the native generator, the in-crate tests and the wasm module,
//! so browser and CLI behavior match by construction.

pub mod ast_json;
pub mod disasm;
pub mod emit;
pub mod pipeline;
pub mod run;

pub use ast_json::{ast_to_json, debug_fallback};
pub use disasm::disassemble;
pub use pipeline::{compile_source, gate_bigint, has_bigint, parse_and_lower, Diag};
pub use run::{execute, ExecOutcome};

#[cfg(target_arch = "wasm32")]
use emit::json_escape;

#[cfg(target_arch = "wasm32")]
fn diags_json(diags: &[Diag]) -> String {
    let mut out = String::from("[");
    for (i, diag) in diags.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"message\":\"{}\",\"start\":{},\"end\":{}}}",
            json_escape(&diag.message),
            diag.start,
            diag.end
        ));
    }
    out.push(']');
    out
}

/// Compile for the browser: diagnostics plus AST, disassembly and size.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn pg_compile(source: &str) -> String {
    let (node, pool) = match parse_and_lower(source) {
        Ok(pair) => pair,
        Err(diags) => {
            return format!(
                "{{\"ok\":false,\"diags\":{},\"ast\":null,\"ast_debug\":\"\",\"disasm\":\"\",\"byte_len\":0}}",
                diags_json(&diags)
            );
        }
    };
    let ast = ast_to_json(&node, &pool);
    let ast_debug = json_escape(&debug_fallback(&node));
    let (disasm, byte_len) = match compile_source(source) {
        Ok(bytes) => match carnelian_compiler::read_rite(&bytes) {
            Ok(model) => (json_escape(&disassemble(&model)), bytes.len()),
            Err(error) => (format!("unreadable RITE: {error:?}"), 0),
        },
        Err(diags) => {
            return format!(
                "{{\"ok\":false,\"diags\":{},\"ast\":null,\"ast_debug\":\"{}\",\"disasm\":\"\",\"byte_len\":0}}",
                diags_json(&diags),
                ast_debug
            );
        }
    };
    format!(
        "{{\"ok\":true,\"diags\":[],\"ast\":{ast},\"ast_debug\":\"{ast_debug}\",\"disasm\":\"{disasm}\",\"byte_len\":{byte_len}}}"
    )
}

/// Raw compiled bytes for the in-browser verify comparison. Intentionally
/// ungated: bytes and views never reach the VM, so bigints are harmless.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn pg_bytes(source: &str) -> Vec<u8> {
    compile_source(source).unwrap_or_default()
}

/// Compile and execute for the browser console.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn pg_run(source: &str) -> String {
    let bytes = match compile_source(source) {
        Ok(bytes) => bytes,
        Err(diags) => {
            return format!(
                "{{\"ok\":false,\"error\":\"compile: {}\"}}",
                json_escape(&format!("{diags:?}"))
            );
        }
    };
    match execute(&bytes) {
        Ok(outcome) => format!(
            "{{\"ok\":true,\"stdout\":\"{}\",\"result\":\"{}\"}}",
            json_escape(&outcome.stdout),
            json_escape(&outcome.result)
        ),
        Err(error) => format!("{{\"ok\":false,\"error\":\"{}\"}}", json_escape(&error)),
    }
}
