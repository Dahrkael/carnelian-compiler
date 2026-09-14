//! P1 certification: `verify` compares `compile --frontend prism` against the
//! pinned C golden, byte for byte, in both strip modes. Exit `0` required.

use std::process::Command;

const SNIPPETS: &[(&str, &str)] = &[
    ("empty", ""),
    ("puts_int", "puts 1\n"),
    ("arith", "puts 1 + 2 * 3\n"),
    ("neg_one", "puts -1\n"),
    ("sub_fusion", "puts 10 - 3\n"),
    ("int8", "puts 300\n"),
    ("int16", "puts 70000\n"),
    ("int64", "x = 3000000000\nputs x\n"),
    ("bigint", "puts 99999999999999999999999\n"),
    ("float", "puts 1.5\n"),
    ("neg_float", "puts -2.5\n"),
    ("string", "puts \"hello\"\n"),
    ("string_escape", "puts \"a\\nb\"\n"),
    ("string_empty", "puts \"\"\n"),
    ("symbol", "puts :sym\n"),
    ("nil_lit", "puts nil\n"),
    ("true_lit", "puts true\n"),
    ("self_lit", "puts self\n"),
    ("if_else", "if true then puts 1 else puts 2 end\n"),
    ("if_no_else", "x = nil\nif x then puts 1 end\nputs 2\n"),
    ("ternary", "puts(true ? 1 : 2)\n"),
    ("unless_mod", "puts 1 unless false\n"),
    ("logic_and", "puts(true && false)\n"),
    ("logic_or", "puts(false || 2)\n"),
    ("compare", "puts(1 < 2)\nputs(1 == 1)\n"),
    ("array", "puts [1, 2]\n"),
    ("array_empty", "puts []\n"),
    ("array_nested", "puts [[1]]\n"),
    ("lvars", "x = 1 + 2\nputs x * x\n"),
    (
        "while_loop",
        "i = 0\nwhile i < 3 do i = i + 1 end\nputs i\n",
    ),
    (
        "until_loop",
        "i = 0\nuntil i > 2 do i = i + 1 end\nputs i\n",
    ),
    ("nil_check", "x = nil\nputs x.nil?\n"),
];

fn carnelian() -> Command {
    Command::new(env!("CARGO_BIN_EXE_carnelian"))
}

#[test]
fn p1_corpus_is_byte_identical() {
    for (name, source) in SNIPPETS {
        let dir = tempfile::tempdir().expect("tempdir");
        let input = dir.path().join(format!("{name}.rb"));
        std::fs::write(&input, source).expect("write snippet");

        // `verify` checks both strip modes against the same golden.
        let verify = carnelian()
            .arg("verify")
            .arg(&input)
            .output()
            .expect("run verify");
        assert_eq!(
            verify.status.code(),
            Some(0),
            "{name}: verify failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&verify.stdout),
            String::from_utf8_lossy(&verify.stderr)
        );

        // `compile` output must also match the golden file byte for byte.
        let golden = dir.path().join(format!("{name}.mrb"));
        let reference = carnelian()
            .arg("reference")
            .arg(&input)
            .arg("-o")
            .arg(&golden)
            .output()
            .expect("run reference");
        assert!(reference.status.success(), "{name}: reference failed");
        for strip in [false, true] {
            let out = dir.path().join(format!("{name}.{strip}.mrb"));
            let mut command = carnelian();
            command.arg("compile").arg(&input).arg("-o").arg(&out);
            if strip {
                command.arg("--strip");
            }
            let compiled = command.output().expect("run compile");
            assert_eq!(
                compiled.status.code(),
                Some(0),
                "{name} (strip={strip}): compile failed: {}",
                String::from_utf8_lossy(&compiled.stderr)
            );
            assert_eq!(
                std::fs::read(&golden).expect("read golden"),
                std::fs::read(&out).expect("read output"),
                "{name} (strip={strip}): bytes diverge"
            );
        }
    }
}
