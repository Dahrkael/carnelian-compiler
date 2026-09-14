# carnelian-compiler — Plan de ejecución

## 0. Resumen

Compilador standalone **Ruby → bytecode RITE (mruby) escrito en Rust**,
byte-idéntico a una referencia C pinneada (`mruby-compiler2` + Prism), con
librería + CLI, y capaz de correr en `wasm32-unknown-unknown`. Se usará luego
en motorpg, pero motorpg es irrelevante para el diseño salvo por los requisitos
de wasm (sin libc, sin `setjmp`, sin FFI en el artefacto final).

- **Nombre:** `carnelian-compiler`.
- **Estado inicial:** consumir `ruby-prism` por FFI, asegurar el output
  (byte-idéntico), y después convertir a AST owned.

### Objetivos

1. Backend Rust que genere RITE byte-idéntico a la referencia pinneada.
2. Frontend de Fase 1 = Prism (vía `ruby-prism` FFI).
3. Fase 2: convertir el backend/entrada a AST owned (sin C, sin FFI).
4. Librería + CLI; la certificación se hace ejecutando el CLI.
5. Determinismo total y viabilidad wasm32.

### No-objetivos (inicial)

- Ejecutar bytecode (eso es mrubyedge).
- UI, editor, motorpg.
- Soporte de `mruby-compiler` bison (mainline). La referencia es la ruta
  **Prism** (`mruby-compiler2`).

---

## 1. Contrato de referencia y pins

Cada release de carnelian declara una **tupla de compatibilidad**. La
byte-identidad solo se certifica contra esa tupla.

| Campo | Pin inicial | Notas |
|---|---|---|
| Compilador C | `mruby-compiler2 0.5.0` | ruta Prism (PicoRuby/FemtoRuby) |
| Parser C | `Prism 1.9.0` | incluida en el vendor de compiler2 |
| Formato | `RITE0400` | mrubyedge rechaza cualquier otro header |
| Config build | `MRC_TARGET_MRUBYC` (`PICORB_VM_MRUBYC`), `MRBC_ALLOC_LIBC`, `MRB_NO_PRESYM`, `MRB_INT64=1`, `PRISM_BUILD_MINIMAL`, `PRISM_XALLOCATOR` | espejo exacto del `build.rs` |
| Int | 64-bit | `MRC_INT_BIT = 64` |

**Proceso de bump upstream (documentado, no improvisado):**

1. Diff de `config.yml` (nodos/campos/flags) y de `codegen.c`/`dump.c`.
2. Portar diferencias por tramos (ver §6).
3. Re-certificar byte-identidad (corpus completo, ambos modos).
4. Actualizar pins + changelog de compatibilidad + bump de versión.

`PINS.md` en la raíz del repo como fuente de verdad legible por humanos y por
un test de CI que falla si la versión de referencia ≠ pin declarado.

---

## 2. Arquitectura

```
                    Fase 1                    Fase 2                 Fase 3 (opcional)
                 ┌───────────┐            ┌──────────────┐        ┌──────────────┐
 Ruby source ───► │ ruby-prism│──borrowed─►│ owned AST    │        │ Prism-Rust   │
                 │  (C, FFI) │            │ (generado)   │        │ (frontend)   │
                 └───────────┘            └──────┬───────┘        └──────┬───────┘
                                                 │  lowering             │
                                                 ▼                       │
                                          ┌──────────────────────────────────┐
                                          │ backend codegen (Rust puro)      │
                                          │  ports de codegen.c              │
                                          └───────────────┬──────────────────┘
                                                          ▼
                                                   irep model
                                                          │
                                                          ▼
                                          writer RITE (port de dump.c) ─► bytes
```

Reglas:

- El **backend no conoce C**. En Fase 1 lo alimenta un módulo de acceso al
  árbol `ruby-prism` (FFI, **dev/CLI only**).
- El artefacto final (librería shipping / wasm) **no incluye FFI**: se compila
  sin los frontends C.
- Frontera del backend: el AST de Prism. En Fase 1 prestado; en Fase 2 owned.
  Para no duplicar handlers, el backend se escribe contra un **trait de acceso
  de nodo** fino (`AstNode`/visitantes) que ambos implementan. Es un adaptador
  delgado, no un IR nuevo.

### Cómo decidir sobre el trait de acceso

- **Recomendado:** trait fino (una implementación para `ruby_prism::Node`, otra
  para owned). Evita reescribir 131 handlers en Fase 2.
- Alternativa: escribir handlers contra `ruby-prism` y re-portarlos a owned en
  Fase 2 (rework mecánico grande). Se descarta salvo que el trait resulte
  molesto.

---

## 3. Estructura del workspace

```
carnelian-compiler/
├── Cargo.toml                     # workspace
├── PINS.md                        # tupla de compatibilidad + changelog
├── crates/
│   ├── carnelian-ast/             # tipos owned generados desde config.yml (+ generator)
│   │   ├── build.rs               # lee config.json, emite node.rs/flags/visitor
│   │   └── src/                   # glue: Node enum helpers, interning, Integer
│   ├── carnelian-astgen/          # (opcional) binario de desarrollo para inspeccionar el generado
│   ├── carnelian-compiler/        # BACKEND + irep + writer RITE (Rust puro)
│   ├── carnelian-front-prism/     # Fase 1: adaptador ruby-prism (feature-gated, FFI)
│   ├── carnelian-front-mri/       # Fase 3: adaptador lib-ruby-parser (Rust puro)
│   └── carnelian-cli/             # binario `carnelian`
└── tests/                         # integración: invoca el CLI y compara bytes
```

Features de `carnelian-compiler`: `front-prism` (FFI), `front-mri`,
`front-owned` (por defecto, puro). El perfil `shipping`/`web` no activa
`front-prism`.

Dependencias pinneadas:

- `ruby-prism = "=1.9.0"`, `ruby-prism-sys = "=1.9.0"` (dev/CLI, feature
  `front-prism`).
- `mruby-compiler2-sys = "=0.5.0"` (dev-dependency del harness de certificación
  / comando `reference`).
- `lib-ruby-parser = "=4.0.6+ruby-3.1.2"` (Fase 3).

---

## 4. Generador de AST owned (P0)

El generador se adapta de `ruby/prism/rust/ruby-prism/build.rs`, que ya lee
`config.json` (derivado de `config.yml`) y emite Rust. Diferencias a aplicar:

- **Owned, no FFI:** `node?` → `Option<Box<Node>>`, `node[]` → `Vec<Node>`,
  `string` → `Box<[u8]>`/`String`, `constant`/`constant[]` → símbolos internados
  (`u32` en un pool propio), `location`/`location?` → offsets `u32` (o
  `Option<Range>`), `integer` → modelo de entero (ver §4.1), flags → `u16` +
  getters.
- **Dos salidas del mismo `config.yml`:** (a) tipos owned; (b) descripción de
  nodos para el lowering FFI→owned (o visitantes).
- Mantener nombres y campos **1:1** con Prism para que el diff upstream sea
  trivial y el mapeo con `codegen.c` no cambie.
- Sin `unsafe`, sin `repr(C)`.

### 4.1 Pendiente de diseño

- `integer`: Prism usa entero de precisión arbitraria. mruby con `MRC_INT64`
  usa `i64`; overflow → literal en pool (`LOADL`). Definir: owned guarda `i64`
  + posible fallback string, o modela bigint. Resolver contra `new_lit_int` de
  `codegen.c`.
- Interning de `constant` y de símbolos: layout y orden deben coincidir con el
  orden que produce `dump.c` para el pool de syms (afecta byte-identidad).

---

## 5. CLI y API (P0/P1)

### Librería

```rust
pub struct CompileOptions { pub stripped: bool, pub filename: Option<String> }
pub fn compile(source: &str, opts: &CompileOptions) -> Result<Vec<u8>, Diagnostics>;
```

`Diagnostics` con offsets (Prism `location`) y mensajes equivalentes a
`mrc_diagnostic_list` (errores/warnings de parser).

### CLI `carnelian`

- `carnelian compile <file.rb> -o <out.mrb> [--strip] [--frontend owned|prism|mri]`
- `carnelian compile -` (stdin)
- `carnelian reference <file.rb> -o <ref.mrb>` — **solo build de dev**, usa
  `mruby-compiler2-sys` (C) para generar el golden.
- `carnelian verify <file.rb> [--strip]` — compila con ambos y compara bytes;
  imprime el primer offset divergente. **Solo dev.**
- `carnelian --version --pins` — imprime la tupla de compatibilidad.
- Códigos de salida: 0 ok, 1 error de compilación (diagnósticos), 2 error de
  uso, 3 divergencia de bytes (verify).

**La certificación se hace invocando el CLI** (subprocess) desde los tests de
integración y comparando ficheros `.mrb`.

---

## 6. Backend: qué portar exactamente (P1)

Port fiel, en el mismo orden, de:

1. **Modelo irep** (`mrc_irep.h`, `mrc_irep_pool_type.h`): `nlocals`, `nregs`,
   `clen`, `flags`, `iseq`, `pool`, `syms`, `reps`, `lv`, `debug_info`, catch
   handlers.
2. **Scope/registros** (`mrc_codegen_scope`, `codegen.c:122`): `sp`, `nlocals`,
   `nregs`, `pc`, `lastpc`, `lastlabel`, `ainfo`, `aspec`, `loop`, pools, `lv`,
   `for_depth`.
3. **Emisión** (`gen_B/S/W`, `genop_0/1/2/3/2S/2SS/W`, `emit_*`,
   `push/pop/cursp`).
4. **Peephole** (la clave de byte-identidad): `gen_move` (`codegen.c:850+`),
   `genjmp2` (`:783`), `gen_int` (`:837`), `mrc_last_insn`, `rewind_pc`,
   `addr_pc`, `mrc_prev_pc`, `no_peephole`.
5. **Pool/syms/lv**: `new_lit_int`, interning de símbolos, `scope_new` (`:452`)
   y la lectura de `program->locals`, `cls/module/sclass/lambda->locals`.
6. **Tramos de nodos** (131 de 152 tipos): dispatch por `nint(tree)` con
   `CAST(...)`.
7. **Errores**: sustituir `MRC_TRY/MRC_CATCH/MRC_THROW` (setjmp) por `Result`
   propagado. `codegen_error` → `Err(Diagnostic)`.
8. **Catch tables**: `catch_handler_set`, `MRC_CATCH_RESCUE`/`MRC_CATCH_ENSURE`
   (rescue/ensure).
9. **Debug info**: `debug.c` — filenames, line table, DBG section (para
   `compile` con DBG) y nada para `--strip`.
10. **Writer RITE** (`dump.c`): secciones `IREP`, `DEBUG`, `EOF`; big-endian en
    el blob; `write_irep_header/iseq/pool/syms`, `write_irep_record`,
    `write_debug_record`, footer. Los offsets se calculan en un pase previo
    (patrón de `dump.c`).

**Determinismo:** todo el backend es single-thread y sin maps de orden no
determinista (usar `Vec`/`BTreeMap`/insertion-order donde el C usa arrays).
Cualquier `HashMap` que afecte orden de pool/syms es bug.

---

## 7. Fases, hitos y criterios de salida

### P0 — Esqueleto, pins, generador, writer

- Workspace, `PINS.md`, CI con test de pin.
- `carnelian-ast`: generador desde `config.yml` → owned types + visitor.
- `carnelian-compiler`: modelo irep + **writer RITE**.
- `carnelian-cli`: `compile` (stub), `reference`, `verify`, `--pins`.
- **Salida:** *round-trip* byte-exacto: compilar con el CLI `reference`,
  parsear con `mrubyedge::rite`, reemitir con el writer, comparar; 20+
  snippets.

### P1 — Motor de codegen sobre `ruby-prism` FFI

- `carnelian-front-prism`: adaptador delgado del árbol `ruby-prism`.
- Backend: scope, registros, emisión, peephole, pool/syms, debug, catch.
- **Salida:** byte-identidad en programas triviales (literales, aritmética,
  `puts`, `if`) en ambos modos (DBG/stripped), certificado vía CLI.

### P2 — Tramos de nodos hasta paridad completa

Por tramos, cada uno con corpus y byte-test:

1. literales, lvars, operadores/send, array/hash
2. `if/unless/while/until/case`, `and/or`, ternario
3. llamadas con args/bloques, `yield`, procs/lambdas
4. `def`/`class`/`module`/`sclass`, const/ivar/cvar/gvar, `super`
5. `rescue`/`ensure` (catch tables), splat/kwargs, masgn
6. pattern matching y especiales (`defined?`, `alias`, flip-flop, BEGIN/END)

- **Salida por tramo:** 100% byte-idéntico en su corpus + sin regresiones en
  tramos anteriores.

### P3 — Conversión a owned (sin FFI)

- Bajar `carnelian-ast` a owned; implementar lowering `ruby-prism` → owned.
- Migrar el backend al trait de acceso (o portar handlers) y **desactivar
  `front-prism` en el artefacto shipping**.
- **Salida:** los mismos bytes que P2, pero compilando sin C. Smoke wasm
  (`wasm32-unknown-unknown`) del `compile`.

### P4 — Frontend `lib-ruby-parser` (100% Rust)

- `carnelian-front-mri`: lowering AST MRI → owned **+ pase de scopes/upvars**:
  (a) locals ordenados por scope, (b) `depth` de cada local read/write (Prism lo
  trae en el nodo; MRI no), (c) `for_depth` y block-locals.
- Gatear sintaxis no soportada (gramática 3.1.2) con diagnóstico claro.
- **Salida:** paridad de comportamiento y de bytes en el subconjunto soportado;
  suite diferencial.

### P5 (opcional) — Prism-Rust

Reemplazar `lib-ruby-parser` por Prism portado. El backend no cambia (tras el
trait).

---

## 8. Certificación (vía CLI)

Dos niveles, ambos orquestando el binario `carnelian`:

- **Nivel 1 — comportamiento:** `carnelian compile` → ejecutar el `.mrb` en
  mrubyedge → comparar stdout/excepciones con el resultado de la referencia.
  (Un test helper, no dependencia de producción.)
- **Nivel 2 — bytes idénticos:** `carnelian verify <f>` compara `compile`
  (carnelian) vs `reference` (C pinneado) byte a byte, en `--strip` y sin él.
  Reporta offset y bytes divergentes.

Corpus:

- Snippets por tramo.
- Suite de tests de `mruby-compiler2`.
- Código real (p. ej. un `framework` Ruby, sin acoplarse a motorpg).
- Fuzz ligero: fragmentos generados/truncados.

CI:

- Matriz nativa + wasm smoke.
- Job de pin: falla si la versión de referencia ≠ `PINS.md`.
- Golden files versionados por tupla
  (`golden/{prism}-{mruby-compiler2}/...`).

---

## 9. Riesgos y mitigaciones

| Riesgo | Impacto | Mitigación |
|---|---|---|
| Peephole/registros divergen | byte no idéntico | port fiel + `verify` desde P1 |
| `integer` de precisión arbitraria | literales mal emitidos | resolver contra `new_lit_int` antes de P2.1 |
| Deriva upstream | tests rompen | proceso diff/re-pin (§1) |
| Scopes/upvars en P4 | bytecode inválido | contrato fijado por P1-P2 + tests diferenciales |
| Rework FFI→owned | coste | trait de acceso delgado |
| `lib-ruby-parser` 3.1.2 | sintaxis moderna | gatear + documentar; P5 cubre el resto |

---

## 10. Convenciones del repo

Ver `AGENTS.md`. En resumen: commits `+`/`-`/`*` con confirmación previa,
`cargo fmt` + `cargo clippy` sin violaciones, tests vía CLI, sin FFI en el
artefacto de distribución, y sin comentarios que documenten fixes.
