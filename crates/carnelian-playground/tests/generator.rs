//! Generator determinism and static output checks.
//!
//! Two emissions into fresh directories must be byte-identical; the page
//! must be self-contained (no CDN, no fetch) and carry the pins footer.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use carnelian_playground::emit::{collect_manifest, emit_static};

fn fresh_dir(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("carnelian-playground-{tag}-{}", std::process::id()));
    if dir.exists() {
        fs::remove_dir_all(&dir).expect("clear temp dir");
    }
    dir
}

fn tree_bytes(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current).expect("read dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let key = path
                    .strip_prefix(dir)
                    .expect("under dir")
                    .to_string_lossy()
                    .into_owned();
                out.insert(key, fs::read(&path).expect("read file"));
            }
        }
    }
    out
}

#[test]
fn two_runs_are_byte_identical() {
    let manifest = collect_manifest().expect("collect");
    let first = fresh_dir("first");
    let second = fresh_dir("second");
    emit_static(&first, &manifest).expect("emit first");
    emit_static(&second, &manifest).expect("emit second");
    let a = tree_bytes(&first);
    let b = tree_bytes(&second);
    assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>());
    for (key, bytes) in &a {
        assert_eq!(bytes, &b[key], "identical bytes for {key}");
    }
    fs::remove_dir_all(&first).ok();
    fs::remove_dir_all(&second).ok();
}

#[test]
fn page_is_self_contained() {
    let manifest = collect_manifest().expect("collect");
    assert!(!manifest.pins.is_empty(), "pins footer parsed");
    assert!(!manifest.examples.is_empty(), "examples collected");
    let dir = fresh_dir("page");
    emit_static(&dir, &manifest).expect("emit");
    let page = fs::read_to_string(dir.join("index.html")).expect("read index");
    let app = fs::read_to_string(dir.join("app.js")).expect("read app.js");
    for token in ["{{", "}}", "http://", "https://", "cdn", "fetch("] {
        assert!(!page.contains(token), "index free of {token}");
    }
    for token in ["https://", "cdn", "fetch("] {
        assert!(!app.contains(token), "app.js free of {token}");
    }
    assert!(page.contains("RITE"), "pins footer mentions format");
    assert!(app.contains("EXAMPLES"), "curated manifest inlined");
    assert!(app.contains("playground.js"), "wasm glue referenced");
    for example in &manifest.examples {
        let copied =
            fs::read_to_string(dir.join(format!("examples/{}.rb", example.name))).expect("copy");
        assert_eq!(copied, example.source, "example {} copied", example.name);
        assert!(
            app.contains(&example.hex),
            "host bytes baked for {}",
            example.name
        );
    }
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn ast_and_disasm_smoke() {
    let (node, pool) =
        carnelian_playground::parse_and_lower("puts \"hello\"\n").expect("parse hello");
    let json = carnelian_playground::ast_to_json(&node, &pool);
    assert!(json.contains("ProgramNode"), "root kind: {json}");
    assert!(json.contains("CallNode"), "call visible: {json}");
    assert!(json.contains("puts"), "method detail: {json}");
    let bytes = carnelian_playground::compile_source("puts \"hello\"\n").expect("compile");
    let model = carnelian_compiler::read_rite(&bytes).expect("read back");
    let disasm = carnelian_playground::disassemble(&model);
    assert!(disasm.contains("SSEND"), "send shown:\n{disasm}");
    assert!(disasm.contains("RETURN"), "return shown:\n{disasm}");
}
