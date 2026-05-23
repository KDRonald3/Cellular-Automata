# `samples` as a thin binary consumer of the `cellular_automata` library

## Summary

The `samples` project currently duplicates all source code from the
`cellular_automata` GitHub repository directly into its `src/` directory.
The goal is to remove that duplication: `samples` should be a minimal
Rust binary crate that declares `cellular_automata` as a Cargo git
dependency and delegates all real behaviour to it, exactly as `main.rs`
already intends ("All real behaviour lives in the library").

The runtime experience for the user does not change — same iced window,
same Rule 30 defaults, same Export HTML button, same `runs/` directory.

---

## Goals

- `samples/Cargo.toml` lists `cellular_automata` as a `git` dependency
  pointing at `https://github.com/KDRonald3/cellular_automata`.
- `samples/src/` contains only `main.rs`. All other `.rs` files are
  deleted because they are owned by the library repo, not this one.
- `cargo run` in `samples/` opens the same interactive iced simulator
  as it does today.
- `cargo build` and `cargo check` pass with no errors or warnings.

---

## Non-goals

- No changes to the `cellular_automata` library repo itself.
- No new features or UI changes.
- No changes to the runtime defaults (rule, width, generations, cell
  size, border toggle, export directory).
- No publishing to crates.io — the dependency stays a private git ref.

---

## Current state (what needs to change)

| File | Action |
|------|--------|
| [Cargo.toml](../../Cargo.toml) | Replace inline deps + rename package to `samples`; add `cellular_automata` git dep |
| [src/lib.rs](../../src/lib.rs) | **Delete** — owned by the library repo |
| [src/sim.rs](../../src/sim.rs) | **Delete** — owned by the library repo |
| [src/ui.rs](../../src/ui.rs) | **Delete** — owned by the library repo |
| [src/export.rs](../../src/export.rs) | **Delete** — owned by the library repo |
| [src/json.rs](../../src/json.rs) | **Delete** — owned by the library repo |
| [src/svg.rs](../../src/svg.rs) | **Delete** — owned by the library repo |
| [src/main.rs](../../src/main.rs) | **Keep as-is** — already uses `cellular_automata::` imports |

---

## Target `Cargo.toml`

```toml
[package]
name = "samples"
version = "0.1.0"
edition = "2024"

[dependencies]
cellular_automata = { git = "https://github.com/KDRonald3/cellular_automata" }
```

The `git` dependency without a `rev` or `branch` key tracks the default
branch (`master`). If the library repo ever gains breaking changes on
master, pin with `rev = "<commit-sha>"`.

---

## Behavior after the change

`src/main.rs` is unchanged. It already:

1. Imports `run`, `BoundaryMode`, `InitialRow`, `OutputKind`,
   `PaddingAlign`, `PaddingFill`, `RenderOptions`, `SimConfig` from
   `cellular_automata`.
2. Builds the default config (Rule 30, 401 wide, 200 generations,
   zero-padded boundary, 4 px cells, borders on).
3. Calls `run(config, initial, render, OutputKind::Ui)`, which opens
   the iced window and blocks until the user closes it.
4. Returns `ExitCode::SUCCESS` or prints the error and returns
   `ExitCode::FAILURE`.

Nothing in `main.rs` needs to change.

---

## Edge cases & constraints

- **Cargo.lock**: after the change, `cargo build` will fetch the library
  from GitHub and regenerate `Cargo.lock`. The old lock file entries for
  `cellular_automata`'s modules become irrelevant; Cargo handles this
  automatically.
- **Offline builds**: a git dependency requires network access on the
  first fetch. Subsequent builds use the local Cargo cache.
- **Library API stability**: `main.rs` uses the public API
  (`run`, `SimConfig`, `InitialRow`, etc.). If the library changes those
  types, `main.rs` will need matching updates — but that is out of scope
  here.
- **Package name**: the binary crate should be renamed from
  `cellular_automata` back to `samples` in `Cargo.toml` to avoid a name
  collision with the library crate it depends on.

---

## Decisions made

- **Git dependency, not path dependency** — the library lives in a
  separate GitHub repo, so a `git` dep is the right mechanism. A path
  dep would require both repos to be cloned side-by-side.
- **No `rev` pin by default** — the current master of the library is
  working and equivalent to the code that was manually copied in. Pinning
  can be added later if stability requires it.
- **Delete, don't archive** — the six duplicate source files are deleted
  outright. Git history preserves them if they are ever needed again.

---

## Out of scope / deferred

- Exploring other `OutputKind` modes (Json, Svg, Html) as separate
  `samples` binaries.
- Adding example scripts or notebooks to `samples/`.
- Publishing `cellular_automata` to crates.io.
