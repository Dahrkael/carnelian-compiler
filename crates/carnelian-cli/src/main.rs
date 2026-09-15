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
        #[cfg_attr(feature = "prism", arg(long, default_value = "prism"))]
        #[cfg_attr(not(feature = "prism"), arg(long, default_value = "mri"))]
        frontend: String,
    },
    /// Emit the pinned C reference golden (`mruby-compiler2 0.5.0`, dev only).
    #[cfg(feature = "reference")]
    Reference {
        /// Source file (`-` reads stdin).
        input: PathBuf,
        /// Output `.mrb` path.
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Compare `compile` against `reference`, byte for byte, in both strip
    /// modes (both must match the same flags-`0` golden).
    #[cfg(feature = "reference")]
    Verify {
        /// Source file (`-` reads stdin).
        input: PathBuf,
        /// Frontend selector.
        #[cfg_attr(feature = "prism", arg(long, default_value = "prism"))]
        #[cfg_attr(not(feature = "prism"), arg(long, default_value = "mri"))]
        frontend: String,
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

#[cfg(feature = "reference")]
fn reference_bytes(source: &str) -> Result<Vec<u8>, String> {
    // SAFETY: single-threaded dev-only use of the pinned C compiler.
    unsafe {
        let mut context = mruby_compiler2_sys::MRubyCompiler2Context::new();
        context
            .compile(source)
            .map_err(|err| format!("reference compile failed: {err}"))
    }
}

#[cfg(feature = "reference")]
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

#[cfg(feature = "reference")]
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

fn check_frontend(frontend: &str) -> Result<(), i32> {
    if frontend == "mri" {
        return Ok(());
    }
    #[cfg(feature = "prism")]
    if frontend == "prism" || frontend == "owned" {
        return Ok(());
    }
    #[cfg(feature = "prism")]
    {
        eprintln!("error: unknown frontend '{frontend}' (expected 'prism', 'owned' or 'mri')");
        Err(2)
    }
    #[cfg(not(feature = "prism"))]
    {
        eprintln!(
            "error: unknown frontend '{frontend}' (expected 'mri'; this build has no prism support, use --frontend mri)"
        );
        Err(2)
    }
}

#[cfg(feature = "prism")]
fn parse_errors_text(errors: &[carnelian_front_prism::ParseDiagnostic]) -> String {
    let mut text = String::new();
    for diagnostic in errors {
        text.push_str(&format!(
            "error: {} ({}:{})\n",
            diagnostic.message, diagnostic.start, diagnostic.end
        ));
    }
    text
}

fn mri_parse_errors_text(errors: &[carnelian_front_mri::ParseDiagnostic]) -> String {
    let mut text = String::new();
    for diagnostic in errors {
        text.push_str(&format!(
            "error: {} ({}:{})\n",
            diagnostic.message, diagnostic.start, diagnostic.end
        ));
    }
    text
}

/// Compile source with the selected frontend: `prism` uses the borrowed
/// FFI tree directly, `owned` lowers it to the owned AST first, `mri`
/// parses with the pure-Rust grammar and runs its end-to-end pipeline.
/// All feed the same generic backend, so bytes must agree.
fn compile_source(
    frontend: &str,
    source: &str,
    opts: &carnelian_compiler::CompileOptions,
) -> Result<Vec<u8>, String> {
    match frontend {
        #[cfg(feature = "prism")]
        "prism" => {
            let parsed = carnelian_front_prism::parse(source.as_bytes());
            let errors = parsed.errors();
            if !errors.is_empty() {
                return Err(parse_errors_text(&errors));
            }
            carnelian_compiler::compile_tree(parsed.root(), opts)
                .map_err(|diagnostics| format!("{diagnostics}"))
        }
        #[cfg(feature = "prism")]
        "owned" => {
            let parsed = carnelian_front_prism::parse(source.as_bytes());
            let errors = parsed.errors();
            if !errors.is_empty() {
                return Err(parse_errors_text(&errors));
            }
            let (node, pool) = carnelian_front_prism::lower(parsed.root());
            let owned = carnelian_ast::Owned {
                node: &node,
                pool: &pool,
            };
            carnelian_compiler::compile_tree(owned, opts)
                .map_err(|diagnostics| format!("{diagnostics}"))
        }
        "mri" => {
            let parsed = carnelian_front_mri::parse(source.as_bytes());
            let errors = parsed.errors();
            if !errors.is_empty() {
                return Err(mri_parse_errors_text(&errors));
            }
            carnelian_front_mri::compile(source, opts)
                .map_err(|diagnostics| format!("{diagnostics}"))
        }
        _ => unreachable!("frontend checked by the caller"),
    }
}

#[cfg(feature = "reference")]
fn cmd_verify(input: &PathBuf, frontend: &str) -> i32 {
    if let Err(code) = check_frontend(frontend) {
        return code;
    }
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
        let compiled = match compile_source(frontend, &source, &opts) {
            Ok(bytes) => bytes,
            Err(text) => {
                eprint!("{text}");
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
    if let Err(code) = check_frontend(frontend) {
        return code;
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
    match compile_source(frontend, &source, &opts) {
        Ok(bytes) => {
            if let Err(err) = std::fs::write(output, &bytes) {
                eprintln!("error: cannot write {}: {err}", output.display());
                return 1;
            }
            println!("compile: {} bytes -> {}", bytes.len(), output.display());
            0
        }
        Err(text) => {
            eprint!("{text}");
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
                #[cfg(feature = "reference")]
                eprintln!("error: missing subcommand (compile|reference|verify) or --pins");
                #[cfg(not(feature = "reference"))]
                eprintln!("error: missing subcommand (compile) or --pins");
                2
            }
        }
        Some(Command::Compile {
            input,
            output,
            strip,
            frontend,
        }) => cmd_compile(input, output, *strip, frontend),
        #[cfg(feature = "reference")]
        Some(Command::Reference { input, output }) => cmd_reference(input, output),
        #[cfg(feature = "reference")]
        Some(Command::Verify { input, frontend }) => cmd_verify(input, frontend),
    };
    std::process::exit(code);
}
