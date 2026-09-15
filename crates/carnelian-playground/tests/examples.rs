//! Curated examples compile and execute with expected outputs.
//!
//! Each row is verified end to end on the MRI path: parse, lower, compile
//! to RITE, gate, load into the pinned executor and capture stdout plus
//! the return-value `inspect`.

use std::fs;

use carnelian_playground::{compile_source, execute};

fn example_path(name: &str) -> String {
    format!("{}/examples/{name}.rb", env!("CARGO_MANIFEST_DIR"))
}

/// Expected `(example, stdout, return inspect)` rows.
const EXPECTED: &[(&str, &str, &str)] = &[
    ("hello", "hello\n", "nil"),
    ("arith", "7\n", "nil"),
    ("vars", "42\n", "nil"),
    ("branches", "1\n", "nil"),
    ("loop", "3\n", "nil"),
    ("compare", "true\ntrue\n", "nil"),
    ("inspect", "42\n", "nil"),
];

#[test]
fn curated_examples_execute() {
    for (name, stdout, result) in EXPECTED {
        let source = fs::read_to_string(example_path(name)).expect("read example");
        let bytes = compile_source(&source).expect("example compiles");
        let outcome = execute(&bytes).expect("example executes");
        assert_eq!(outcome.stdout, *stdout, "stdout of {name}");
        assert_eq!(outcome.result, *result, "return of {name}");
    }
}

#[test]
fn puts_forms_match_mri_shapes() {
    // Multi-arg, array splat (nested too), nil and bare forms.
    for (source, stdout) in [
        ("puts 1, 2\n", "1\n2\n"),
        ("puts [1, 2]\n", "1\n2\n"),
        ("puts [[1, 2], 3]\n", "1\n2\n3\n"),
        ("puts nil\n", "\n"),
        ("puts\n", "\n"),
        ("p 1, 2\n", "1\n2\n"),
        ("p [1, 2]\n", "[1, 2]\n"),
        ("p\n", ""),
    ] {
        let bytes = compile_source(source).expect("form compiles");
        let outcome = execute(&bytes).expect("form executes");
        assert_eq!(outcome.stdout, stdout, "stdout of {source:?}");
        assert_eq!(outcome.result, "nil", "return of {source:?}");
    }
}

#[test]
fn bigint_literal_stays_gated() {
    let bytes = compile_source("puts 99999999999999999999999\n").expect("bigint compiles");
    let error = execute(&bytes).expect_err("bigint must not execute");
    assert!(error.contains("bigint"), "fail-closed diagnostic: {error}");
}

#[test]
fn malformed_input_reports_offsets() {
    let source = "def foo(\n";
    let diags = compile_source(source).expect_err("broken syntax fails");
    assert!(!diags.is_empty(), "at least one diagnostic");
    for diag in &diags {
        assert!(!diag.message.is_empty(), "message present");
        assert!(
            diag.end as usize <= source.len(),
            "offsets inside source: {diag:?}"
        );
    }
}

#[test]
fn return_inspect_without_puts() {
    let bytes = compile_source("1 + 2\n").expect("compiles");
    let outcome = execute(&bytes).expect("executes");
    assert_eq!(outcome.stdout, "");
    assert_eq!(outcome.result, "3");
}
