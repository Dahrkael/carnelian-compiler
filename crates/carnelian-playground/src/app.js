// Carnelian playground UI: vanilla JS, no dependencies, no network.
// Curated examples (with host-baked outputs) are inlined below.
const EXAMPLES = {{EXAMPLES_JSON}};

import init, * as pg from './playground.js';

const $ = (id) => document.getElementById(id);
let wasmReady = false;

function showTab(name) {
  for (const el of document.querySelectorAll('.tabs button')) {
    el.classList.toggle('active', el.dataset.tab === name);
  }
  for (const el of document.querySelectorAll('.tab')) {
    el.classList.toggle('active', el.id === 'tab-' + name);
  }
}

for (const el of document.querySelectorAll('.tabs button')) {
  el.addEventListener('click', () => showTab(el.dataset.tab));
}

// Byte offset (UTF-8) to UTF-16 code-unit index for textarea selection.
function byteToChar(source, byteOffset) {
  const enc = new TextEncoder();
  let bytes = 0;
  let i = 0;
  while (i < source.length) {
    const cp = source.codePointAt(i);
    const ch = String.fromCodePoint(cp);
    const len = enc.encode(ch).length;
    if (bytes + len > byteOffset) break;
    bytes += len;
    i += ch.length;
  }
  return i;
}

// Byte offset to 1-based line:col for diagnostics.
function byteToLineCol(source, byteOffset) {
  const enc = new TextEncoder();
  let bytes = 0, line = 1, col = 1;
  let i = 0;
  while (i < source.length && bytes < byteOffset) {
    const cp = source.codePointAt(i);
    const ch = String.fromCodePoint(cp);
    const len = enc.encode(ch).length;
    if (ch === '\n') { line += 1; col = 1; } else { col += 1; }
    bytes += len;
    i += ch.length;
  }
  return [line, col];
}

function printConsole(lines) {
  $('console').textContent = lines.join('\n');
  showTab('console');
}

function currentExample() {
  const name = $('example').value;
  return EXAMPLES.find((e) => e.name === name);
}

function fillExamples() {
  const sel = $('example');
  sel.textContent = '';
  const adHoc = document.createElement('option');
  adHoc.value = '';
  adHoc.textContent = '(ad-hoc)';
  sel.appendChild(adHoc);
  for (const e of EXAMPLES) {
    const opt = document.createElement('option');
    opt.value = e.name;
    opt.textContent = e.name;
    sel.appendChild(opt);
  }
  sel.addEventListener('change', () => {
    const ex = currentExample();
    if (ex) $('editor').value = ex.source;
  });
  if (EXAMPLES.length > 0) {
    sel.value = EXAMPLES[0].name;
    $('editor').value = EXAMPLES[0].source;
  }
}

function requireWasm() {
  if (!wasmReady) {
    printConsole(['wasm module not ready yet; serve dist/ over http and reload.']);
    return false;
  }
  return true;
}

function compileInBrowser(source) {
  return JSON.parse(pg.pg_compile(source));
}

$('btn-run').addEventListener('click', () => {
  if (!requireWasm()) return;
  const source = $('editor').value;
  const compiled = compileInBrowser(source);
  const lines = [];
  if (compiled.diags.length > 0) {
    for (const d of compiled.diags) {
      const [line, col] = byteToLineCol(source, d.start);
      lines.push(`error ${line}:${col}: ${d.message}`);
    }
    const first = compiled.diags[0];
    const ed = $('editor');
    ed.focus();
    ed.setSelectionRange(byteToChar(source, first.start), byteToChar(source, first.end));
    printConsole(lines);
    return;
  }
  const ran = JSON.parse(pg.pg_run(source));
  if (!ran.ok) {
    printConsole(['run failed: ' + ran.error]);
    return;
  }
  lines.push('--- stdout ---');
  lines.push(ran.stdout === '' ? '(empty)' : ran.stdout.replace(/\n$/, ''));
  lines.push('--- return ---');
  lines.push(ran.result);
  lines.push(`--- bytes: ${compiled.byte_len} ---`);
  $('disasm').textContent = compiled.disasm;
  renderAst(compiled.ast, compiled.ast_debug);
  printConsole(lines);
});

function renderAst(ast, astDebug) {
  const host = $('ast');
  host.textContent = '';
  const build = (node) => {
    const det = document.createElement('details');
    det.className = 'ast';
    det.open = true;
    const sum = document.createElement('summary');
    sum.textContent = `${node.kind} [${node.span[0]}, ${node.span[1]})` +
      (node.detail ? ` ${node.detail}` : '');
    det.appendChild(sum);
    for (const child of node.children || []) det.appendChild(build(child));
    return det;
  };
  try {
    host.appendChild(build(typeof ast === 'string' ? JSON.parse(ast) : ast));
  } catch (e) {
    host.textContent = 'AST render failed: ' + e;
  }
  $('ast-debug').textContent = astDebug;
  showTab('ast');
}

$('btn-ast').addEventListener('click', () => {
  if (!requireWasm()) return;
  const source = $('editor').value;
  const compiled = compileInBrowser(source);
  if (compiled.diags.length > 0) {
    $('btn-run').click();
    return;
  }
  renderAst(compiled.ast, compiled.ast_debug);
});

$('btn-dis').addEventListener('click', () => {
  if (!requireWasm()) return;
  const source = $('editor').value;
  const compiled = compileInBrowser(source);
  if (compiled.diags.length > 0) {
    $('btn-run').click();
    return;
  }
  $('disasm').textContent = compiled.disasm;
  showTab('bytecode');
});

function hexOf(bytes) {
  return Array.from(bytes).map((b) => b.toString(16).padStart(2, '0')).join('');
}

$('btn-verify').addEventListener('click', () => {
  if (!requireWasm()) return;
  const ex = currentExample();
  if (!ex) {
    printConsole(['verify needs a curated example; ad-hoc code has no host reference in-browser.']);
    return;
  }
  const source = $('editor').value;
  const compiled = compileInBrowser(source);
  if (compiled.diags.length > 0) {
    printConsole(['verify: source does not compile.']);
    return;
  }
  const localHex = hexOf(pg.pg_bytes(source));
  const lines = [`example: ${ex.name}`, `local bytes: ${compiled.byte_len}`];
  if (source !== ex.source) {
    lines.push('note: editor differs from the curated source; comparing against baked host bytes anyway.');
  }
  lines.push(localHex === ex.hex
    ? 'verify: byte-identical to host build'
    : 'verify: DIFFERS from host build');
  printConsole(lines);
});

fillExamples();

try {
  await init();
  wasmReady = true;
  $('wasm-state').textContent = 'wasm ready';
} catch (e) {
  $('wasm-state').textContent = 'wasm failed: serve dist/ via http (' + e + ')';
}
