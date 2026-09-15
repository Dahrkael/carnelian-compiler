//! Playground site generator (native binary).
//!
//! `cargo run -p carnelian-playground --bin playground -- --out dist/`
//! emits a self-contained offline `dist/`: editor, AST and bytecode
//! viewers plus an execution console. The wasm module is built with the
//! plain `cargo` + `wasm-bindgen` CLI pair (no trunk/wasm-pack).

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use carnelian_playground::emit::{collect_manifest, emit_static};

/// Usage text.
const USAGE: &str = "usage: playground [--out dist/] [--no-wasm] [--release]";

fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
        .expect("crate lives under the workspace")
}

fn target_dir(root: &Path) -> PathBuf {
    if let Ok(dir) = env::var("CARGO_TARGET_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    root.join("target")
}

fn run_command(program: &str, args: &[&str], cwd: &Path) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .status()
        .map_err(|error| format!("run {program}: {error} (is it installed?)"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} {args:?} failed with {status}"))
    }
}

/// Build the wasm module and glue into `out_dir` via the installed CLI pair.
fn build_wasm(out_dir: &Path, release: bool) -> Result<(), String> {
    let root = workspace_root();
    let profile = if release { "release" } else { "debug" };
    let mut args = vec![
        "build",
        "-p",
        "carnelian-playground",
        "--lib",
        "--target",
        "wasm32-unknown-unknown",
    ];
    if release {
        args.push("--release");
    }
    run_command("cargo", &args, &root)?;
    let wasm = target_dir(&root)
        .join("wasm32-unknown-unknown")
        .join(profile)
        .join("carnelian_playground.wasm");
    if !wasm.exists() {
        return Err(format!("wasm artifact missing: {}", wasm.display()));
    }
    let out = out_dir.to_string_lossy().into_owned();
    let input = wasm.to_string_lossy().into_owned();
    run_command(
        "wasm-bindgen",
        &[
            input.as_str(),
            "--target",
            "web",
            "--out-dir",
            out.as_str(),
            "--out-name",
            "playground",
        ],
        &root,
    )
}

fn main() {
    let mut out_dir = PathBuf::from("dist");
    let mut with_wasm = true;
    let mut release = false;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => match args.next() {
                Some(dir) => out_dir = PathBuf::from(dir),
                None => {
                    eprintln!("{USAGE}");
                    std::process::exit(2);
                }
            },
            "--no-wasm" => with_wasm = false,
            "--release" => release = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                return;
            }
            _ => {
                eprintln!("{USAGE}");
                std::process::exit(2);
            }
        }
    }
    let manifest = match collect_manifest() {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!("playground: {error}");
            std::process::exit(1);
        }
    };
    if let Err(error) = emit_static(&out_dir, &manifest) {
        eprintln!("playground: {error}");
        std::process::exit(1);
    }
    if with_wasm {
        if let Err(error) = build_wasm(&out_dir, release) {
            eprintln!("playground: wasm step failed: {error}");
            eprintln!("playground: static files are emitted; retry or pass --no-wasm");
            std::process::exit(1);
        }
    }
    println!(
        "playground: wrote {} ({} examples, wasm={with_wasm})",
        out_dir.display(),
        manifest.examples.len()
    );
}
