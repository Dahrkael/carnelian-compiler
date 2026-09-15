//! Writer certification: `reference` emits the pinned C golden through the
//! CLI, then the Rust reader/writer round-trips it to identical bytes.
//! (`verify` compares real codegen output; see `p1_verify.rs`.)

use std::process::Command;

#[cfg(feature = "reference")]
#[path = "corpus.rs"]
mod corpus;

#[cfg(feature = "reference")]
use corpus::ROUNDTRIP_SNIPPETS as SNIPPETS;

fn carnelian() -> Command {
    Command::new(env!("CARGO_BIN_EXE_carnelian"))
}

/// Bytes with the `DBG` section removed (cross-frontend parity ignores
/// debug info; the writer round-trip above stays byte-exact).
fn without_debug(bytes: &[u8]) -> Vec<u8> {
    carnelian_compiler::without_debug(bytes).expect("strip debug")
}

#[cfg(feature = "reference")]
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

    // `compile` with the default frontend emits the program (exit 0).
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

    // `compile --frontend owned` matches `prism` ignoring `DBG` bytes.
    let owned_out = dir.path().join("ok.owned.mrb");
    #[cfg(feature = "prism")]
    {
        let owned = carnelian()
            .arg("compile")
            .arg(&input)
            .arg("-o")
            .arg(&owned_out)
            .arg("--frontend")
            .arg("owned")
            .output()
            .expect("run owned compile");
        assert_eq!(
            owned.status.code(),
            Some(0),
            "owned compile failed: {}",
            String::from_utf8_lossy(&owned.stderr)
        );
        assert_eq!(
            without_debug(&std::fs::read(&output).expect("read prism output")),
            without_debug(&std::fs::read(&owned_out).expect("read owned output")),
            "owned diverges from prism"
        );
    }

    // `compile --frontend mri` succeeds (exit 0) and matches the default
    // frontend ignoring `DBG` bytes. In pure builds the default is mri itself,
    // so the cross-frontend comparison only runs with prism linked.
    let mri_out = dir.path().join("ok.mri.mrb");
    let mri = carnelian()
        .arg("compile")
        .arg(&input)
        .arg("-o")
        .arg(&mri_out)
        .arg("--frontend")
        .arg("mri")
        .output()
        .expect("run mri compile");
    assert_eq!(
        mri.status.code(),
        Some(0),
        "mri compile failed: {}",
        String::from_utf8_lossy(&mri.stderr)
    );
    #[cfg(feature = "prism")]
    assert_eq!(
        without_debug(&std::fs::read(&output).expect("read default output")),
        without_debug(&std::fs::read(&mri_out).expect("read mri output")),
        "mri diverges from prism"
    );

    // Unknown frontends are a usage error (exit 2).
    let unknown = carnelian()
        .arg("compile")
        .arg(&input)
        .arg("-o")
        .arg(&owned_out)
        .arg("--frontend")
        .arg("bogus")
        .output()
        .expect("run unknown compile");
    assert_eq!(unknown.status.code(), Some(2));

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
    #[cfg(feature = "reference")]
    {
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
    }

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
        ("lib-ruby-parser", "4.0.6+ruby-3.1.2"),
        ("lib-ruby-parser-ast", "0.55.0"),
    ] {
        let entry = format!("name = \"{name}\"\nversion = \"{version}\"");
        assert!(
            lock.contains(&entry),
            "Cargo.lock must pin {name} ={version} per PINS.md"
        );
    }
}
