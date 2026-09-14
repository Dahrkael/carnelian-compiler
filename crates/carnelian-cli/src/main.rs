//! `carnelian` developer CLI.
//!
//! Certification runs through this binary: `reference` emits the pinned C
//! golden, `compile --frontend prism` runs the Rust codegen, and `verify`
//! compares both byte for byte in stripped and unstripped modes.

use std::io::Read;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

const PINS: &str = include_str!("../../../PINS.md");

#[derive(Debug, Parser)]
#[command(name = "carnelian", version, about = "Ruby -> RITE compiler")]
struct Cli {
    /// Print the compatibility tuple and exit.
    #[arg(long)]
    pins: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Compile Ruby source with the Rust codegen.
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
        #[arg(long, default_value = "prism")]
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
    /// Compare `compile` against `reference`, byte for byte, in both strip
    /// modes (both must match the same flags-`0` golden).
    Verify {
        /// Source file (`-` reads stdin).
        input: PathBuf,
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

fn first_divergence(a: &[u8], b: &[u8]) -> Option<usize> {
    for (index, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        if x != y {
            return Some(index);
        }
    }
    if a.len() != b.len() {
        return Some(a.len().min(b.len()));
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
    // Both modes must match the same flags-`0` golden (see agents/progress.md).
    for stripped in [false, true] {
        let opts = carnelian_compiler::CompileOptions {
            stripped,
            filename: None,
        };
        let parsed = carnelian_front_prism::parse(source.as_bytes());
        let errors = parsed.errors();
        if !errors.is_empty() {
            for diagnostic in &errors {
                eprintln!(
                    "error: {} ({}:{})",
                    diagnostic.message, diagnostic.start, diagnostic.end
                );
            }
            return 1;
        }
        let compiled = match carnelian_compiler::compile_prism(parsed.root(), &opts) {
            Ok(bytes) => bytes,
            Err(diagnostics) => {
                eprint!("{diagnostics}");
                return 1;
            }
        };
        match first_divergence(&reference, &compiled) {
            None => {
                println!(
                    "verify: identical ({} bytes, stripped={stripped})",
                    reference.len()
                );
            }
            Some(offset) => {
                let a = reference.get(offset).copied().unwrap_or(0);
                let b = compiled.get(offset).copied().unwrap_or(0);
                eprintln!(
                    "divergence at offset {offset} (stripped={stripped}): reference=0x{a:02x} carnelian=0x{b:02x} (len {} vs {})",
                    reference.len(),
                    compiled.len()
                );
                return 3;
            }
        }
    }
    0
}

fn cmd_compile(input: &PathBuf, output: &PathBuf, strip: bool, frontend: &str) -> i32 {
    if frontend != "prism" {
        eprintln!("error: frontend '{frontend}' is unavailable in P1 (only 'prism')");
        return 2;
    }
    let source = match read_input(input) {
        Ok(source) => source,
        Err(message) => {
            eprintln!("error: {message}");
            return 2;
        }
    };
    let opts = carnelian_compiler::CompileOptions {
        stripped: strip,
        filename: None,
    };
    let parsed = carnelian_front_prism::parse(source.as_bytes());
    let errors = parsed.errors();
    if !errors.is_empty() {
        for diagnostic in &errors {
            eprintln!(
                "error: {} ({}:{})",
                diagnostic.message, diagnostic.start, diagnostic.end
            );
        }
        return 1;
    }
    match carnelian_compiler::compile_prism(parsed.root(), &opts) {
        Ok(bytes) => {
            if let Err(err) = std::fs::write(output, &bytes) {
                eprintln!("error: cannot write {}: {err}", output.display());
                return 1;
            }
            println!("compile: {} bytes -> {}", bytes.len(), output.display());
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
            strip,
            frontend,
        }) => cmd_compile(input, output, *strip, frontend),
        Some(Command::Reference { input, output }) => cmd_reference(input, output),
        Some(Command::Verify { input }) => cmd_verify(input),
    };
    std::process::exit(code);
}
