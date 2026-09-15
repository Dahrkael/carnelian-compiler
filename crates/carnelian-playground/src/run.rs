//! Execution on the pinned mrubyedge fork with captured output.
//!
//! The fork prints `puts`/`p` via `println!` under its `wasi` feature,
//! which never reaches a browser console. The playground instead replaces
//! both methods after `VM::open` with capturing natives backed by a
//! thread-local buffer, so native and wasm runs share one code path and
//! the console shows captured stdout plus the return-value `inspect`.
//! Bigints stay gated fail-closed before load (see `pipeline::gate_bigint`).

use std::cell::RefCell;

use mrubyedge::yamrb::helpers::{mrb_define_cmethod, mrb_funcall};
use mrubyedge::yamrb::value::Value;
use mrubyedge::yamrb::vm::VM;

use crate::pipeline::gate_bigint;

thread_local! {
    /// Captured `puts`/`p` lines for the running VM.
    static CAPTURE: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Captured stdout plus the inspected return value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOutcome {
    /// Lines written by `puts`/`p`, each newline-terminated.
    pub stdout: String,
    /// `inspect` of the program return value.
    pub result: String,
}

fn render_to_string(vm: &mut VM, value: &Value, method: &str) -> String {
    match mrb_funcall(vm, Some(value.clone()), method, &[]) {
        Ok(rendered) => String::try_from(&rendered).unwrap_or_else(|_| "(unprintable)".to_string()),
        Err(_) => "(unprintable)".to_string(),
    }
}

/// One `puts` value: arrays splat one line per element like MRI (depth
/// cap turns cyclic structures into `[...] ` instead of overflowing).
fn puts_value(vm: &mut VM, value: &Value, out: &mut String, depth: u32) {
    if depth > 32 {
        out.push_str("[...]\n");
        return;
    }
    if let Value::Object(object) = value {
        if let Ok(elements) = Vec::<Value>::try_from(object.as_ref()) {
            for element in &elements {
                puts_value(vm, element, out, depth + 1);
            }
            return;
        }
    }
    out.push_str(&render_to_string(vm, value, "to_s"));
    out.push('\n');
}

/// Capturing `puts`: one line per argument via `to_s`.
fn capture_puts(vm: &mut VM, args: &[Option<Value>]) -> Result<Value, mrubyedge::Error> {
    let mut out = String::new();
    if args.iter().all(Option::is_none) {
        out.push('\n');
    }
    for arg in args.iter().flatten() {
        puts_value(vm, arg, &mut out, 0);
    }
    CAPTURE.with(|cell| cell.borrow_mut().push_str(&out));
    Ok(Value::Nil)
}

/// Capturing `p`: one `inspect` line per argument.
fn capture_p(vm: &mut VM, args: &[Option<Value>]) -> Result<Value, mrubyedge::Error> {
    let mut out = String::new();
    for arg in args.iter().flatten() {
        out.push_str(&render_to_string(vm, arg, "inspect"));
        out.push('\n');
    }
    CAPTURE.with(|cell| cell.borrow_mut().push_str(&out));
    Ok(Value::Nil)
}

/// Install the capturing natives over the fork's `println!` versions.
fn install_capture(vm: &mut VM) {
    let object_class = vm.object_class.clone();
    mrb_define_cmethod(vm, object_class.clone(), "puts", Box::new(capture_puts));
    mrb_define_cmethod(vm, object_class, "p", Box::new(capture_p));
}

/// Execute compiled RITE bytes; bigints and load errors fail closed.
pub fn execute(bytes: &[u8]) -> Result<ExecOutcome, String> {
    gate_bigint(bytes)?;
    let mut rite = mrubyedge::rite::load(bytes).map_err(|error| format!("load: {error:?}"))?;
    let mut vm = VM::open(&mut rite);
    install_capture(&mut vm);
    CAPTURE.with(|cell| cell.borrow_mut().clear());
    let value = vm.run().map_err(|error| format!("run: {error}"))?;
    let stdout = CAPTURE.with(|cell| cell.borrow_mut().clone());
    let result = render_to_string(&mut vm, &value, "inspect");
    Ok(ExecOutcome { stdout, result })
}
