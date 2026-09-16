//! Real-world corpus runner (dev only; requires the `reference` feature).
//!
//! Walks a directory of `.rb` files (see `tools/fetch_corpus.sh`), compiles
//! each with the pinned C reference and with our frontends, and classifies
//! every (file, frontend, strip-mode) row as identical, ref_reject,
//! our_gate, diverge or unreadable. `--check` enforces
//! `corpus/baseline.tsv`; `--update` rewrites it. `--coverage` adds
//! node-kind and opcode reachability to the report.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::{compile_source, first_divergence, reference_bytes};

#[cfg(feature = "prism")]
use carnelian_ast::Visit;

/// Opcode names in discriminant order (`OP_NOP == 0` .. `OP_STOP == 118`).
/// Dev-only copy of the playground table; anchored by debug asserts below.
const OP_NAMES: [&str; 119] = [
    "NOP",
    "MOVE",
    "LOADL",
    "LOADI8",
    "LOADINEG",
    "LOADI__1",
    "LOADI_0",
    "LOADI_1",
    "LOADI_2",
    "LOADI_3",
    "LOADI_4",
    "LOADI_5",
    "LOADI_6",
    "LOADI_7",
    "LOADI16",
    "LOADI32",
    "LOADSYM",
    "LOADNIL",
    "LOADSELF",
    "LOADTRUE",
    "LOADFALSE",
    "GETGV",
    "SETGV",
    "GETSV",
    "SETSV",
    "GETIV",
    "SETIV",
    "GETCV",
    "SETCV",
    "GETCONST",
    "SETCONST",
    "GETMCNST",
    "SETMCNST",
    "GETUPVAR",
    "SETUPVAR",
    "GETIDX",
    "GETIDX0",
    "SETIDX",
    "JMP",
    "JMPIF",
    "JMPNOT",
    "JMPNIL",
    "JMPUW",
    "EXCEPT",
    "RESCUE",
    "RAISEIF",
    "MATCHERR",
    "SSEND",
    "SSEND0",
    "SSENDB",
    "SEND",
    "SEND0",
    "SENDB",
    "CALL",
    "BLKCALL",
    "SUPER",
    "ARGARY",
    "ENTER",
    "KEY_P",
    "KEYEND",
    "KARG",
    "RETURN",
    "RETURN_BLK",
    "RETSELF",
    "RETNIL",
    "RETTRUE",
    "RETFALSE",
    "BREAK",
    "BLKPUSH",
    "ADD",
    "ADDI",
    "SUB",
    "SUBI",
    "ADDILV",
    "SUBILV",
    "MUL",
    "DIV",
    "EQ",
    "LT",
    "LE",
    "GT",
    "GE",
    "ARRAY",
    "ARRAY2",
    "ARYCAT",
    "ARYPUSH",
    "ARYSPLAT",
    "AREF",
    "ASET",
    "APOST",
    "INTERN",
    "SYMBOL",
    "STRING",
    "STRCAT",
    "HASH",
    "HASHADD",
    "HASHCAT",
    "LAMBDA",
    "BLOCK",
    "METHOD",
    "RANGE_INC",
    "RANGE_EXC",
    "OCLASS",
    "CLASS",
    "MODULE",
    "EXEC",
    "DEF",
    "TDEF",
    "SDEF",
    "ALIAS",
    "UNDEF",
    "SCLASS",
    "TCLASS",
    "DEBUG",
    "ERR",
    "EXT1",
    "EXT2",
    "EXT3",
    "STOP",
];

fn check_op_names() {
    use carnelian_compiler::opcode;
    for (byte, name) in [
        (opcode::OP_NOP, "NOP"),
        (opcode::OP_JMP, "JMP"),
        (opcode::OP_SEND, "SEND"),
        (opcode::OP_ENTER, "ENTER"),
        (opcode::OP_RETURN, "RETURN"),
        (opcode::OP_STOP, "STOP"),
    ] {
        assert_eq!(OP_NAMES[byte as usize], name);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Status {
    Identical,
    RefReject,
    OurGate,
    Diverge,
    Unreadable,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Identical => "identical",
            Status::RefReject => "ref_reject",
            Status::OurGate => "our_gate",
            Status::Diverge => "diverge",
            Status::Unreadable => "unreadable",
        }
    }

    fn from_str(text: &str) -> Option<Status> {
        match text {
            "identical" => Some(Status::Identical),
            "ref_reject" => Some(Status::RefReject),
            "our_gate" => Some(Status::OurGate),
            "diverge" => Some(Status::Diverge),
            "unreadable" => Some(Status::Unreadable),
            _ => None,
        }
    }
}

struct Row {
    path: String,
    frontend: String,
    stripped: bool,
    status: Status,
    detail: String,
}

/// First line with control characters flattened, capped for TSV stability.
fn one_line(text: &str) -> String {
    let line: String = text
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c == '\t' || c == '\r' || c == '\n' {
                ' '
            } else {
                c
            }
        })
        .collect();
    const CAP: usize = 240;
    if line.len() > CAP {
        let mut cut = CAP;
        while !line.is_char_boundary(cut) {
            cut -= 1;
        }
        format!("{}…", &line[..cut])
    } else {
        line
    }
}

fn collect_rb(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries =
        std::fs::read_dir(dir).map_err(|err| format!("cannot read {}: {err}", dir.display()))?;
    let mut children: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| format!("cannot list {}: {err}", dir.display()))?;
        children.push(entry.path());
    }
    children.sort();
    for child in children {
        if child.is_dir() {
            collect_rb(&child, out)?;
        } else if child.extension().and_then(|ext| ext.to_str()) == Some("rb") {
            out.push(child);
        }
    }
    Ok(())
}

fn rel_slash(dir: &Path, path: &Path) -> String {
    path.strip_prefix(dir)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn strip_debug(bytes: &[u8]) -> Result<Vec<u8>, String> {
    carnelian_compiler::without_debug(bytes).map_err(|err| format!("{err}"))
}

#[cfg(feature = "prism")]
struct Kinds(BTreeSet<&'static str>);

#[cfg(feature = "prism")]
impl carnelian_ast::Visit for Kinds {
    fn visit(&mut self, node: &carnelian_ast::Node) {
        self.0.insert(node.kind_name());
        carnelian_ast::visit_children(self, node);
    }
}

fn collect_opcodes(irep: &carnelian_compiler::Irep, into: &mut BTreeSet<u8>) {
    let mut pc = 0;
    while pc < irep.iseq.len() {
        into.insert(irep.iseq[pc]);
        match carnelian_compiler::opcode::decode_at(&irep.iseq, pc) {
            Some((_, next)) if next > pc => pc = next,
            _ => pc += 1,
        }
    }
    for child in &irep.reps {
        collect_opcodes(child, into);
    }
}

pub struct Options {
    pub dir: PathBuf,
    pub frontends: Vec<String>,
    pub baseline: PathBuf,
    pub check: bool,
    pub update: bool,
    pub report: Option<PathBuf>,
    pub coverage: bool,
}

/// Parse `--frontends` (`mri`, `prism`, comma list, or `all`).
pub fn parse_frontends(raw: &str) -> Result<Vec<String>, i32> {
    let mut out = Vec::new();
    for name in raw.split(',') {
        let name = name.trim();
        if name == "all" {
            out.push("mri".to_owned());
            out.push("prism".to_owned());
        } else if name == "mri" || name == "prism" {
            out.push(name.to_owned());
        } else {
            eprintln!("error: unknown frontend '{name}' (expected 'mri', 'prism' or 'all')");
            return Err(2);
        }
    }
    out.sort();
    out.dedup();
    if out.is_empty() {
        eprintln!("error: no frontends selected");
        return Err(2);
    }
    for frontend in &out {
        if crate::check_frontend(frontend).is_err() {
            return Err(2);
        }
    }
    Ok(out)
}

type BaseKey = (String, String, bool);

fn load_baseline(baseline: &Path) -> Result<BTreeMap<BaseKey, (Status, String)>, String> {
    let text = std::fs::read_to_string(baseline)
        .map_err(|err| format!("cannot read {}: {err}", baseline.display()))?;
    let mut map = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(4, '\t');
        let (Some(path), Some(frontend), Some(stripped), Some(rest)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(format!(
                "{}:{}: malformed row",
                baseline.display(),
                index + 1
            ));
        };
        let mut rest = rest.splitn(2, '\t');
        let (Some(status), Some(detail)) = (rest.next(), rest.next()) else {
            return Err(format!(
                "{}:{}: malformed row",
                baseline.display(),
                index + 1
            ));
        };
        let Some(status) = Status::from_str(status) else {
            return Err(format!("{}:{}: bad status", baseline.display(), index + 1));
        };
        let stripped = match stripped {
            "true" => true,
            "false" => false,
            _ => return Err(format!("{}:{}: bad mode", baseline.display(), index + 1)),
        };
        map.insert(
            (path.to_owned(), frontend.to_owned(), stripped),
            (status, detail.to_owned()),
        );
    }
    Ok(map)
}

pub fn cmd_corpus(opts: Options) -> i32 {
    check_op_names();
    if opts.check && opts.update {
        eprintln!("error: --check and --update are exclusive");
        return 2;
    }
    let mut files = Vec::new();
    if let Err(message) = collect_rb(&opts.dir, &mut files) {
        eprintln!("error: {message}");
        return 2;
    }
    if files.is_empty() {
        eprintln!(
            "error: no .rb files under {} (run tools/fetch_corpus.sh first?)",
            opts.dir.display()
        );
        return 2;
    }

    let mut rows: Vec<Row> = Vec::new();
    let mut kinds: BTreeSet<&'static str> = BTreeSet::new();
    let mut opcodes: BTreeSet<u8> = BTreeSet::new();
    #[cfg(feature = "prism")]
    let coverage_note: Option<String> = None;
    #[cfg(not(feature = "prism"))]
    let coverage_note: Option<String> = opts
        .coverage
        .then(|| "node kinds need the `prism` feature".to_owned());

    for path in &files {
        let rel = rel_slash(&opts.dir, path);
        let source = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) => {
                for frontend in &opts.frontends {
                    for stripped in [false, true] {
                        rows.push(Row {
                            path: rel.clone(),
                            frontend: frontend.clone(),
                            stripped,
                            status: Status::Unreadable,
                            detail: one_line(&format!("{err}")),
                        });
                    }
                }
                continue;
            }
        };
        let reference = match reference_bytes(&source) {
            Ok(bytes) => match strip_debug(&bytes) {
                Ok(stripped) => Some(stripped),
                Err(message) => {
                    eprintln!("error: cannot strip reference output for {rel}: {message}");
                    return 1;
                }
            },
            Err(message) => {
                let detail = one_line(&message);
                for frontend in &opts.frontends {
                    for stripped in [false, true] {
                        rows.push(Row {
                            path: rel.clone(),
                            frontend: frontend.clone(),
                            stripped,
                            status: Status::RefReject,
                            detail: detail.clone(),
                        });
                    }
                }
                None
            }
        };
        let Some(reference) = reference else {
            continue;
        };
        for frontend in &opts.frontends {
            for stripped in [false, true] {
                let copts = carnelian_compiler::CompileOptions {
                    stripped,
                    filename: None,
                };
                match compile_source(frontend, &source, &copts) {
                    Err(text) => rows.push(Row {
                        path: rel.clone(),
                        frontend: frontend.clone(),
                        stripped,
                        status: Status::OurGate,
                        detail: one_line(&text),
                    }),
                    Ok(bytes) => {
                        let compiled = match strip_debug(&bytes) {
                            Ok(stripped) => stripped,
                            Err(_) => {
                                rows.push(Row {
                                    path: rel.clone(),
                                    frontend: frontend.clone(),
                                    stripped,
                                    status: Status::Diverge,
                                    detail: "unreadable-output".to_owned(),
                                });
                                continue;
                            }
                        };
                        match first_divergence(&reference, &compiled) {
                            None => {
                                if opts.coverage {
                                    if let Ok(model) = carnelian_compiler::read_rite(&compiled) {
                                        collect_opcodes(&model.root, &mut opcodes);
                                    }
                                }
                                rows.push(Row {
                                    path: rel.clone(),
                                    frontend: frontend.clone(),
                                    stripped,
                                    status: Status::Identical,
                                    detail: format!("{} bytes", compiled.len()),
                                });
                            }
                            Some(offset) => {
                                let a = reference.get(offset).copied().unwrap_or(0);
                                let b = compiled.get(offset).copied().unwrap_or(0);
                                rows.push(Row {
                                    path: rel.clone(),
                                    frontend: frontend.clone(),
                                    stripped,
                                    status: Status::Diverge,
                                    detail: format!(
                                        "offset {offset} ref=0x{a:02x} ours=0x{b:02x} len {} vs {}",
                                        reference.len(),
                                        compiled.len()
                                    ),
                                });
                            }
                        }
                    }
                }
            }
        }
        #[cfg(feature = "prism")]
        if opts.coverage {
            let parsed = carnelian_front_prism::parse(source.as_bytes());
            if parsed.errors().is_empty() {
                let (node, _pool) = carnelian_front_prism::lower(parsed.root());
                let mut visitor = Kinds(BTreeSet::new());
                visitor.visit(&node);
                kinds.extend(visitor.0);
            }
        }
    }

    // Summary + gate histogram to stdout.
    let mut counts: BTreeMap<Status, usize> = BTreeMap::new();
    let mut gates: BTreeMap<String, usize> = BTreeMap::new();
    let mut refgates: BTreeMap<String, usize> = BTreeMap::new();
    for row in &rows {
        *counts.entry(row.status).or_default() += 1;
        match row.status {
            Status::OurGate => *gates.entry(row.detail.clone()).or_default() += 1,
            Status::RefReject => *refgates.entry(row.detail.clone()).or_default() += 1,
            _ => {}
        }
    }
    // One histogram entry per file (modes/frontends share the same gate).
    println!(
        "corpus: {} files, {} rows ({} frontends)",
        files.len(),
        rows.len(),
        opts.frontends.join("+")
    );
    for status in [
        Status::Identical,
        Status::RefReject,
        Status::OurGate,
        Status::Diverge,
        Status::Unreadable,
    ] {
        println!(
            "  {}: {}",
            status.as_str(),
            counts.get(&status).unwrap_or(&0)
        );
    }
    let mut gates: Vec<(String, usize)> = gates.into_iter().collect();
    gates.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    println!("  top our_gate markers:");
    for (marker, count) in gates.iter().take(15) {
        println!("    {count:>4}  {marker}");
    }
    if gates.len() > 15 {
        println!("    … and {} more", gates.len() - 15);
    }

    if let Some(path) = &opts.report {
        if let Err(message) = write_report(
            path,
            &opts,
            &rows,
            &gates,
            &refgates,
            opts.coverage.then_some((&kinds, &opcodes, &coverage_note)),
        ) {
            eprintln!("error: {message}");
            return 1;
        }
        println!("corpus: report -> {}", path.display());
    }

    if opts.update {
        if let Err(message) = write_baseline(&opts.baseline, &rows) {
            eprintln!("error: {message}");
            return 1;
        }
        println!("corpus: baseline -> {}", opts.baseline.display());
        return 0;
    }

    if opts.check {
        let expected = match load_baseline(&opts.baseline) {
            Ok(map) => map,
            Err(message) => {
                eprintln!("error: {message} (run with --update first?)");
                return 2;
            }
        };
        let mut actual: BTreeMap<BaseKey, (Status, String)> = BTreeMap::new();
        for row in &rows {
            actual.insert(
                (row.path.clone(), row.frontend.clone(), row.stripped),
                (row.status, row.detail.clone()),
            );
        }
        let mut mismatches = 0;
        for (key, (status, detail)) in &actual {
            match expected.get(key) {
                Some((want_status, want_detail))
                    if *want_status == *status && *want_detail == *detail =>
                {
                    continue;
                }
                Some((want_status, want_detail)) => {
                    println!(
                        "- {} [{} stripped={}] changed: was {} [{}], now {} [{}]",
                        key.0,
                        key.1,
                        key.2,
                        want_status.as_str(),
                        want_detail,
                        status.as_str(),
                        detail
                    );
                    mismatches += 1;
                }
                None => {
                    println!(
                        "- {} [{} stripped={}] new row: {} [{}]",
                        key.0,
                        key.1,
                        key.2,
                        status.as_str(),
                        detail
                    );
                    mismatches += 1;
                }
            }
        }
        for key in expected.keys() {
            if !actual.contains_key(key) {
                println!(
                    "- {} [{} stripped={}] vanished from corpus",
                    key.0, key.1, key.2
                );
                mismatches += 1;
            }
        }
        if mismatches > 0 {
            eprintln!("corpus: {mismatches} baseline mismatches");
            return 1;
        }
        println!("corpus: baseline matches");
    }
    0
}

fn write_baseline(path: &Path, rows: &[Row]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("cannot create {}: {err}", parent.display()))?;
        }
    }
    let mut text = String::from(
        "# Real-world corpus baseline (generated by `carnelian corpus --update`).\n# path\tfrontend\tstripped\tstatus\tdetail\n",
    );
    for row in rows {
        text.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            row.path,
            row.frontend,
            row.stripped,
            row.status.as_str(),
            row.detail
        ));
    }
    std::fs::write(path, text).map_err(|err| format!("cannot write {}: {err}", path.display()))
}

#[allow(clippy::too_many_arguments)]
fn write_report(
    path: &Path,
    opts: &Options,
    rows: &[Row],
    gates: &[(String, usize)],
    refgates: &BTreeMap<String, usize>,
    coverage: Option<(&BTreeSet<&'static str>, &BTreeSet<u8>, &Option<String>)>,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("cannot create {}: {err}", parent.display()))?;
        }
    }
    let mut text = String::new();
    text.push_str("# Corpus report (generated by `carnelian corpus --report`)\n\n");
    text.push_str(&format!("- dir: `{}`\n", opts.dir.display()));
    text.push_str(&format!("- frontends: {}\n", opts.frontends.join(", ")));
    text.push_str(&format!("- rows: {}\n\n", rows.len()));
    text.push_str("## Status counts\n\n| status | rows |\n|---|---:|\n");
    for status in [
        Status::Identical,
        Status::RefReject,
        Status::OurGate,
        Status::Diverge,
        Status::Unreadable,
    ] {
        let count = rows.iter().filter(|row| row.status == status).count();
        text.push_str(&format!("| {} | {count} |\n", status.as_str()));
    }
    text.push_str("\n## Our-gate histogram\n\n| rows | marker |\n|---:|---|\n");
    for (marker, count) in gates {
        text.push_str(&format!("| {count} | `{marker}` |\n"));
    }
    let mut refgates: Vec<(&String, &usize)> = refgates.iter().collect();
    refgates.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    text.push_str("\n## Reference-reject histogram\n\n| rows | marker |\n|---:|---|\n");
    for (marker, count) in refgates.iter().take(25) {
        text.push_str(&format!("| {count} | `{marker}` |\n"));
    }
    text.push_str("\n## Non-identical rows\n\n| file | frontend | stripped | status | detail |\n|---|---|---|---|---|\n");
    for row in rows {
        if row.status != Status::Identical {
            text.push_str(&format!(
                "| `{}` | {} | {} | {} | {} |\n",
                row.path,
                row.frontend,
                row.stripped,
                row.status.as_str(),
                row.detail.replace('|', "/")
            ));
        }
    }
    if let Some((kinds, opcodes, note)) = coverage {
        text.push_str("\n## Coverage\n\n");
        if let Some(note) = note {
            text.push_str(&format!("{note}\n"));
        } else {
            text.push_str(&format!(
                "Node kinds reached: {}/{}\n\n",
                kinds.len(),
                carnelian_ast::ALL_NODE_KINDS.len()
            ));
            text.push_str("Unreached node kinds:\n\n");
            for kind in carnelian_ast::ALL_NODE_KINDS {
                if !kinds.contains(kind) {
                    text.push_str(&format!("- `{kind}`\n"));
                }
            }
            text.push_str(&format!(
                "\nOpcodes reached: {}/{}\n\n",
                opcodes.len(),
                OP_NAMES.len()
            ));
            text.push_str("Unreached opcodes:\n\n");
            for (index, name) in OP_NAMES.iter().enumerate() {
                if !opcodes.contains(&(index as u8)) {
                    text.push_str(&format!("- `{name}`\n"));
                }
            }
        }
    }
    std::fs::write(path, text).map_err(|err| format!("cannot write {}: {err}", path.display()))
}
