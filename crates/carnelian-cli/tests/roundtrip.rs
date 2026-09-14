//! Writer certification: `reference` emits the pinned C golden through the
//! CLI, then the Rust reader/writer round-trips it to identical bytes.
//! (`verify` compares real codegen output; see `p1_verify.rs`.)

use std::process::Command;

const SNIPPETS: &[(&str, &str)] = &[
    ("empty", ""),
    ("puts_int", "puts 1\n"),
    ("arith", "puts 1 + 2 * 3\n"),
    ("int64_over_i32", "x = 3000000000\nputs x\n"),
    ("bigint_over_i64", "x = 99999999999999999999999\nputs x\n"),
    ("float", "x = 1.5\nputs x\n"),
    ("string", "puts \"hello\"\n"),
    ("symbol", "puts :sym\n"),
    ("array", "a = [1, 2, 3]\nputs a\n"),
    ("hash", "h = {a: 1, \"b\" => 2}\nputs h\n"),
    ("if_else", "if true then puts 1 else puts 2 end\n"),
    ("while_loop", "i = 0\nwhile i < 3 do i += 1 end\nputs i\n"),
    ("def_call", "def add(a, b)\n  a + b\nend\nputs add(1, 2)\n"),
    (
        "class_def",
        "class Foo\n  def bar\n    42\n  end\nend\nputs Foo.new.bar\n",
    ),
    ("block", "[1, 2, 3].each { |x| puts x }\n"),
    ("rescue", "begin\n  foo\nrescue\n  bar\nend\n"),
    ("interp", "name = \"w\"\nputs \"hi #{name}\"\n"),
    ("splat", "a = [1, 2, 3]\nb = [*a, 4]\nputs b\n"),
    ("kwargs", "def f(a:, b: 2)\n  a + b\nend\nputs f(a: 1)\n"),
    ("lambda", "f = ->(x) { x * 2 }\nputs f.call(21)\n"),
    ("const", "X = 1\nputs X\n"),
    ("logic", "a = true && false || true\nputs a\n"),
    ("case", "case 1\nwhen 1 then puts 1\nelse puts 2\nend\n"),
];

fn carnelian() -> Command {
    Command::new(env!("CARGO_BIN_EXE_carnelian"))
}

#[test]
fn reference_and_verify_are_byte_identical() {
    let binary = env!("CARGO_BIN_EXE_carnelian");
    assert!(!binary.is_empty());
    for (name, source) in SNIPPETS {
        let dir = tempfile::tempdir().expect("tempdir");
        let input = dir.path().join(format!("{name}.rb"));
        let golden = dir.path().join(format!("{name}.mrb"));
        std::fs::write(&input, source).expect("write snippet");

        let reference = carnelian()
            .arg("reference")
            .arg(&input)
            .arg("-o")
            .arg(&golden)
            .output()
            .expect("run reference");
        assert!(
            reference.status.success(),
            "{name}: reference failed: {}",
            String::from_utf8_lossy(&reference.stderr)
        );
        let bytes = std::fs::read(&golden).expect("read golden");
        assert!(
            bytes.starts_with(b"RITE0400"),
            "{name}: not a RITE0400 binary"
        );

        // Determinism: a second run must produce identical bytes.
        let golden2 = dir.path().join(format!("{name}.2.mrb"));
        let again = carnelian()
            .arg("reference")
            .arg(&input)
            .arg("-o")
            .arg(&golden2)
            .output()
            .expect("run reference again");
        assert!(again.status.success(), "{name}: second reference failed");
        assert_eq!(bytes, std::fs::read(&golden2).expect("read golden2"));

        // Round-trip through the Rust reader/writer must be identical.
        let reemitted = carnelian_compiler::roundtrip(&bytes).expect("round-trip parses");
        assert_eq!(
            bytes, reemitted,
            "{name}: writer diverged from the C golden"
        );
    }
}

#[test]
fn cli_exit_codes() {
    let dir = tempfile::tempdir().expect("tempdir");

    // `compile --frontend prism` emits the program (exit 0).
    let input = dir.path().join("ok.rb");
    let output = dir.path().join("ok.mrb");
    std::fs::write(&input, "puts 1\n").expect("write");
    let compile = carnelian()
        .arg("compile")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .output()
        .expect("run compile");
    assert_eq!(
        compile.status.code(),
        Some(0),
        "compile failed: {}",
        String::from_utf8_lossy(&compile.stderr)
    );
    assert!(output.exists());

    // Unavailable frontends are a usage error (exit 2).
    let owned = carnelian()
        .arg("compile")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .arg("--frontend")
        .arg("owned")
        .output()
        .expect("run owned compile");
    assert_eq!(owned.status.code(), Some(2));

    // Broken source fails compilation with exit 1.
    let bad = dir.path().join("bad.rb");
    let bad_out = dir.path().join("bad.mrb");
    std::fs::write(&bad, "def (\n").expect("write");
    let bad_compile = carnelian()
        .arg("compile")
        .arg(&bad)
        .arg("-o")
        .arg(&bad_out)
        .output()
        .expect("run bad compile");
    assert_eq!(bad_compile.status.code(), Some(1));

    // Broken source fails the C reference with exit 1.
    let bad = dir.path().join("bad.rb");
    let bad_out = dir.path().join("bad.mrb");
    std::fs::write(&bad, "def (\n").expect("write");
    let reference = carnelian()
        .arg("reference")
        .arg(&bad)
        .arg("-o")
        .arg(&bad_out)
        .output()
        .expect("run bad reference");
    assert_eq!(reference.status.code(), Some(1));

    // `--pins` prints the tuple and exits 0.
    let pins = carnelian().arg("--pins").output().expect("run --pins");
    assert_eq!(pins.status.code(), Some(0));
    let text = String::from_utf8_lossy(&pins.stdout);
    assert!(text.contains("mruby-compiler2 0.5.0"));
    assert!(text.contains("Prism 1.9.0"));
    assert!(text.contains("RITE0400"));
}

#[test]
fn locked_pins_match_pins_md() {
    let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock"))
        .expect("read workspace Cargo.lock");
    for (name, version) in [
        ("mruby-compiler2-sys", "0.5.0"),
        ("ruby-prism", "1.9.0"),
        ("ruby-prism-sys", "1.9.0"),
    ] {
        let entry = format!("name = \"{name}\"\nversion = \"{version}\"");
        assert!(
            lock.contains(&entry),
            "Cargo.lock must pin {name} ={version} per PINS.md"
        );
    }
}
