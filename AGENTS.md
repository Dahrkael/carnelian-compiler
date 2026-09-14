# AGENTS.md

Guidelines for working on this repo.

## Commits

- Each message line starts with `+` (additions: features, methods, classes,
  files), `-` (removals: files, functions, features) or `*` (fixes: bugs,
  wrong paths).
- One line per distinct change; do not merge unrelated changes into one line.
- Lines are short and concrete. No explanations or justifications.
- Never commit without the user's explicit confirmation of the exact
  proposal. When work is done, list the proposed commits (grouping and
  message lines) and wait for approval; do not run `git commit` on your own
  initiative, even for changes you made yourself.
- Never commit changes or files you did not make or create yourself. Stage
  only your own work, even when using broad `git add` paths; if a foreign
  change is already staged by accident, unstage it before committing.

## Testing

- Never test with distribution (`shipping`/`web`) profiles. Use debug or
  release.
- Test through the CLI: compile a source file, then compare bytes against the
  pinned reference (`mruby-compiler2` + Prism) and/or run the result.
- Byte-identical output is certified against the reference tuple declared in
  `PINS.md`, in both modes: with DBG section and stripped.
- The pure-Rust path must build and pass its smoke tests for
  `wasm32-unknown-unknown`; the FFI frontend is dev/CLI only.
- Worktrees must use their own unique CARGO_TARGET_DIR to avoid collisions
  with other worktrees, {worktree root}/target is recommended.
- Never pin a reference version different from `PINS.md` in a test.

## Code style

- Code and comments in English only (identifiers, comments, log/error strings,
  CLI help text).
- Keep comments brief; explain intent, not mechanics.
- Do not document fixes or decisions in code comments. Architecture decisions
  live in `docs/architecture.md`; progress notes in `docs/progress.md`;
  compatibility pins in `PINS.md`.
- No `unsafe` outside the FFI frontend crate, and keep it contained there.
- No C, no libc and no setjmp in the shipping library path; errors propagate
  as `Result`/diagnostics.
- Deterministic output: no hash maps where iteration order can reach the
  emitted bytecode.
- Do not add fallbacks or backwards compatibility on your own, ask first.
- Rust code has to be linted using cargo clippy and it discovers no violations
- Rust code has to be formatted using cargo fmt

## Workflow

- Use ripgrep when available for searches
- Never run merges (including merge commits on pull) or delete branches
  without the user's explicit confirmation.
- Big features, big refactors or potentially breaking changes need to be done
  in a new worktree with a new "feature/{name_here}" branch created just for
  that under the /tmp folder. Once the work is done and everything is
  commited (with prior approval) the worktree has to be rebased from its
  parent branch and merged to its parent branch.
- If the parent branch has uncommited files push it to the stash, merge
  then pop the stash to restore the files.
- Stay on plan unless blocked by insurmountable need. Every deviation
  is recorded in agents/progress.md with cause. User decisions are not 
  deviations but keep them tracked too.
- After finishing a plan phase a reviewer subagent must be dispatched to 
  search for bugs, duplicated code, optimizations and architectural improvements.

## Editing source files

- Never edit existing source files with inline scripts (python/sed/awk).
  Use the Edit tool exclusively; it fails loudly on mismatches instead of
  silently no-op'ing.
- Never overwrite, move, or delete files whose origin you have not verified.
  If a name is unfamiliar, ask before touching it.

## Conventions

- The compiler emits RITE `RITE0400` only; any other version is rejected.
- Parser/compiler reference versions are pinned in `PINS.md`; changing them is
  a deliberate, reviewed event with a compatibility changelog entry.
- Frontends live behind cargo features and a thin node-access trait;
  the backend never sees C.
- Public errors are diagnostics with source offsets, not panics.
