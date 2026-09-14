//! `carnelian` developer CLI.
//!
//! Certification runs through this binary: `reference` emits the pinned C
//! golden, `verify` round-trips it through the pure-Rust writer and compares
//! bytes. `compile` is a P0 stub (no codegen yet).

use std::io::Read;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

const PINS: &str = include_str!("../../../PINS.md");

#[derive(Debug, Parser)]
#[command(
    name = "carnelian",
    version,
    about = "Ruby -> RITE compiler (P0 skeleton)"
)]
struct Cli {
    /// Print the compatibility tuple and exit.
    #[arg(long)]
    pins: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Compile Ruby source (P0 stub: reports unimplemented).
    Compile {
        /// Source file (`-` reads stdin).
        input: PathBuf,
        /// Output `.mrb` path.
        #[arg(short, long)]
        output: PathBuf,
        /// Omit the `DBG` section (accepted; codegen arrives in P1).
        #[arg(long)]
        strip: bool,
        /// Frontend selector.
        #[arg(long, default_value = "owned")]
        frontend: String,
    },
    /// Emit the pinned C reference golden (`mruby-compiler2 0.5.0`, dev only).
    Reference {
        /// Source file (`-` reads stdin).
        input: PathBuf,
        /// Output `.mrb` path.
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Round-trip `reference` through the Rust writer and compare bytes.
    Verify {
        /// Source file (`-` reads stdin).
        input: PathBuf,
        /// Accepted for P1 parity; the C reference emits flags `0`.
        #[arg(long)]
        strip: bool,
    },
}

fn read_input(path: &PathBuf) -> Result<String, String> {
    if path.as_os_str() == "-" {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|err| format!("cannot read stdin: {err}"))?;
        Ok(text)
    } else {
        std::fs::read_to_string(path)
            .map_err(|err| format!("cannot read {}: {err}", path.display()))
    }
}

fn reference_bytes(source: &str) -> Result<Vec<u8>, String> {
    // SAFETY: single-threaded dev-only use of the pinned C compiler.
    unsafe {
        let mut context = mruby_compiler2_sys::MRubyCompiler2Context::new();
        context
            .compile(source)
            .map_err(|err| format!("reference compile failed: {err}"))
    }
}

fn model_has_bigint(irep: &carnelian_compiler::Irep) -> bool {
    irep.pool
        .iter()
        .any(|entry| matches!(entry, carnelian_compiler::PoolValue::BigInt(_)))
        || irep.reps.iter().any(model_has_bigint)
}

fn first_divergence(a: &[u8], b: &[u8]) -> Option<usize> {
    let common = a.len().min(b.len());
    for (index, (x, y)) in a.iter().zip(b.iter()).enumerate().take(common) {
        if x != y {
            return Some(index);
        }
    }
    if a.len() != b.len() {
        return Some(common);
    }
    None
}

fn cmd_reference(input: &PathBuf, output: &PathBuf) -> i32 {
    let source = match read_input(input) {
        Ok(source) => source,
        Err(message) => {
            eprintln!("error: {message}");
            return 2;
        }
    };
    match reference_bytes(&source) {
        Ok(bytes) => {
            if let Err(err) = std::fs::write(output, &bytes) {
                eprintln!("error: cannot write {}: {err}", output.display());
                return 1;
            }
            println!("reference: {} bytes -> {}", bytes.len(), output.display());
            0
        }
        Err(message) => {
            eprintln!("error: {message}");
            1
        }
    }
}

fn cmd_verify(input: &PathBuf) -> i32 {
    let source = match read_input(input) {
        Ok(source) => source,
        Err(message) => {
            eprintln!("error: {message}");
            return 2;
        }
    };
    let reference = match reference_bytes(&source) {
        Ok(bytes) => bytes,
        Err(message) => {
            eprintln!("error: {message}");
            return 1;
        }
    };

    // Sanity: the reference must parse as RITE0400 under both readers.
    // `mrubyedge 2.0.0` misreads pool type 7 (`BIGINT`: u16 length instead of
    // the `dump.c` u8 length + verbatim bytes) and panics, so binaries with a
    // bigint pool entry skip that cross-check; the carnelian reader is exact.
    let model = match carnelian_compiler::read_rite(&reference) {
        Ok(model) => model,
        Err(err) => {
            eprintln!("error: reference output rejected by carnelian reader: {err}");
            return 1;
        }
    };
    let irep_count = if model_has_bigint(&model.root) {
        println!("verify: note: bigint pool entry present, skipping mrubyedge cross-check");
        model.root.reps.len() + 1
    } else {
        match mrubyedge::rite::load(&reference) {
            Ok(parsed) => parsed.irep.len(),
            Err(err) => {
                eprintln!("error: reference output rejected by mrubyedge::rite: {err:?}");
                return 1;
            }
        }
    };
    let reemitted = carnelian_compiler::write_rite(&model);

    match first_divergence(&reference, &reemitted) {
        None => {
            println!(
                "verify: identical ({} bytes, {} top-level ireps)",
                reference.len(),
                irep_count
            );
            0
        }
        Some(offset) => {
            let a = reference.get(offset).copied().unwrap_or(0);
            let b = reemitted.get(offset).copied().unwrap_or(0);
            eprintln!(
                "divergence at offset {offset}: reference=0x{a:02x} writer=0x{b:02x} (len {} vs {})",
                reference.len(),
                reemitted.len()
            );
            3
        }
    }
}

fn cmd_compile(input: &PathBuf, output: &PathBuf, frontend: &str) -> i32 {
    if frontend != "owned" {
        eprintln!("error: frontend '{frontend}' is unavailable in P0 (only 'owned')");
        return 2;
    }
    let source = match read_input(input) {
        Ok(source) => source,
        Err(message) => {
            eprintln!("error: {message}");
            return 2;
        }
    };
    let opts = carnelian_compiler::CompileOptions::default();
    match carnelian_compiler::compile(&source, &opts) {
        Ok(bytes) => {
            if let Err(err) = std::fs::write(output, &bytes) {
                eprintln!("error: cannot write {}: {err}", output.display());
                return 1;
            }
            0
        }
        Err(diagnostics) => {
            eprint!("{diagnostics}");
            1
        }
    }
}

fn main() {
    let cli = Cli::parse();
    if cli.pins {
        println!("{PINS}");
    }
    let code = match &cli.command {
        None => {
            if cli.pins {
                0
            } else {
                eprintln!("error: missing subcommand (compile|reference|verify) or --pins");
                2
            }
        }
        Some(Command::Compile {
            input,
            output,
            strip: _,
            frontend,
        }) => cmd_compile(input, output, frontend),
        Some(Command::Reference { input, output }) => cmd_reference(input, output),
        Some(Command::Verify { input, strip: _ }) => cmd_verify(input),
    };
    std::process::exit(code);
}
