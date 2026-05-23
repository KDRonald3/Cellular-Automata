# Library Conversion — Requirements

## Summary

Convert the existing binary-only `cellular_automata` crate into a reusable Rust library that another project can depend on and call. The library exposes a single `run()` entry point that takes the same input knobs the current UI has, plus an `OutputKind` parameter, and produces one of five outputs: in-memory structured data, JSON, SVG, an HTML gallery export, or a launched interactive UI window. A thin binary is retained so `cargo run` still opens the app the way it does today. Speed is a first-class non-functional requirement, on par with correctness.

## Goals

- The crate compiles as both a library (`src/lib.rs`) and a binary (`src/main.rs`). External projects can add it as a path or git dependency and `use cellular_automata::…`.
- A single function `run(config, initial, render, output) -> Result<Output, Error>` covers every use case. All inputs the current UI exposes are reachable through this function.
- Five output modes are supported through the `OutputKind` enum: `Structured`, `Json`, `Svg`, `Html { dir }`, `Ui`.
- The structured output gives the caller a Rust-native value (`SimulationResult`) they can iterate, slice, transform, or serialize themselves.
- The HTML output preserves the existing "gallery of runs" behavior — `manifest.tsv` + regenerated `index.html` — at a directory the caller chooses.
- Errors are returned as `Result`, never panicked, with messages a beginner can act on.
- Documentation has two layers: (1) rustdoc comments on every public item, and (2) a standalone `DOCUMENTATION.md` at the repo root that reads as a tutorial for a high-school student with some programming experience.
- The library is fast. Simulation throughput must not regress vs. the current binary; structured output must not introduce per-row heap allocations.

## Non-goals

- **No new simulation features.** Same elementary 1D CA (8-bit rule numbers, 1D row, boundary modes already in `sim.rs`). Not adding 2D, totalistic, multi-color, or non-binary states.
- **No async API.** `run()` is synchronous and blocking. The UI mode blocks until the window closes.
- **No CLI tool.** The binary's only job is to launch the UI; it is not a configurable command-line interface.
- **No feature flags.** Every dependency (`iced`, `base64`, `chrono`, plus the new `serde`/`serde_json` for JSON) is on by default. We are optimizing for a beginner's experience, not for binary-size savings.
- **No crates.io publishing in this scope.** The library lives in this repo and is consumed as a path or git dependency.
- **No WASM target.**
- **No builder pattern.** The caller constructs struct literals directly — explicit, greppable, and the user asked for it.

## Users and use cases

The primary consumer is the user's own other Rust project. Four representative call sites:

1. **"I want the raw cells to crunch in Rust."** Caller passes `OutputKind::Structured`, gets back a `SimulationResult` whose `rows: Vec<u8>` they iterate to compute densities, find gliders, slice for plotting, etc.
2. **"I want a figure for a paper."** Caller passes `OutputKind::Svg`, gets back a `String` they write to disk and drop into LaTeX/Inkscape.
3. **"I want a shareable web preview."** Caller passes `OutputKind::Html { dir }`, gets back the `PathBuf` of the generated file. The `runs/` gallery at `dir` keeps accumulating.
4. **"Let me look at it interactively."** Caller passes `OutputKind::Ui`, an iced window opens at the configured initial cell size, the caller's thread blocks until it's closed.

## Public API

### Module layout

```
src/
  lib.rs        # public surface: run(), Output, Error, re-exports
  sim.rs        # unchanged — simulation core, re-exported
  export.rs     # HTML export, refactored to accept a target dir
  svg.rs        # NEW — SVG generation with run-length encoding
  json.rs       # NEW — JSON serialization (or fold into lib.rs)
  ui.rs         # NEW — iced App moved out of main.rs
  main.rs       # thin: parse defaults, call run(..., OutputKind::Ui)
```

### Types

```rust
pub struct SimConfig {
    pub rule: u8,
    pub width: usize,
    pub generations: usize,
    pub boundary: BoundaryMode,        // re-exported from sim.rs
}

pub enum InitialRow {
    /// Caller hands in a full row of length == width. Each entry must be 0 or 1.
    Explicit(Vec<u8>),
    /// Library builds the row from a short seed pattern padded out to width.
    FromSeed {
        seed: Vec<u8>,
        align: PaddingAlign,           // re-exported from sim.rs
        fill: PaddingFill,             // re-exported from sim.rs
    },
}

pub struct RenderOptions {
    /// Pixel size of each cell in SVG/HTML; sets the initial zoom in UI mode.
    pub cell_size: u32,
    /// Whether grid lines are drawn between cells (SVG/HTML/UI).
    pub show_borders: bool,
}

pub enum OutputKind {
    Structured,
    Json,
    Svg,
    Html { dir: PathBuf },
    Ui,
}

pub enum Output {
    Structured(SimulationResult),
    Json(String),
    Svg(String),
    Html(PathBuf),                     // path of the file written
    Ui,                                // unit-like; the window has closed
}

pub struct SimulationResult {
    pub config: SimConfig,
    pub width: usize,
    pub generations: usize,
    /// Flat row-major grid: cell (y, x) lives at `rows[y * width + x]`.
    /// Length is exactly `(generations + 1) * width`. Values are 0 or 1.
    pub rows: Vec<u8>,
}

pub enum Error {
    WidthZero,
    InitialRowLengthMismatch { expected: usize, got: usize },
    SeedWiderThanWidth { width: usize, seed_len: usize },
    NonBinaryCellValue { position: usize, value: u8 },
    HtmlExportFailed(std::io::Error),
    UiLaunchFailed(String),
}
```

### Entry point

```rust
pub fn run(
    config: SimConfig,
    initial: InitialRow,
    render: RenderOptions,
    output: OutputKind,
) -> Result<Output, Error>;
```

### JSON shape

```json
{
  "config": {
    "rule": 30,
    "width": 21,
    "generations": 5,
    "boundary": "Wrap"
  },
  "rows": [
    [0,0,0,1,0,0,0,...],
    [0,0,1,1,1,0,0,...],
    ...
  ]
}
```

Row-major 2D array. Verbose for very large runs — the doc tells callers to prefer `Structured` for those.

## Affected code

- [src/sim.rs](src/sim.rs) — unchanged behavior. Types (`BoundaryMode`, `PaddingAlign`, `PaddingFill`, `Rule`, `CellularAutomaton`) become re-exports from `lib.rs`. The `#[allow(dead_code)]` markers on the public API ([sim.rs:69](src/sim.rs#L69), [sim.rs:96](src/sim.rs#L96), [sim.rs:107](src/sim.rs#L107)) come off since the items become genuinely public. `make_initial_row` ([sim.rs:151](src/sim.rs#L151)) gets a length-check sibling for `Explicit` rows.
- [src/main.rs](src/main.rs) — most of the file moves to a new `src/ui.rs`: the `Job`, `JobShared`, `JobParams`, `Message`, `App`, `TileProgram` types ([main.rs:59-1015](src/main.rs#L59-L1015)) all relocate. `main.rs` shrinks to a tiny entry point that constructs default `SimConfig` / `InitialRow` / `RenderOptions` and calls `run(..., OutputKind::Ui)`.
- [src/export.rs](src/export.rs) — `export_job` ([export.rs:34](src/export.rs#L34)) is refactored so its target directory is a parameter rather than computed by `runs_dir()` ([export.rs:541](src/export.rs#L541)). **Every other behavior in this file is preserved byte-for-byte** — see the "HTML export — feature preservation" section below for the full enumeration. The `ExportInput` struct ([export.rs:21](src/export.rs#L21)) is internal; the public surface is a single function that takes a `SimulationResult`, a `RenderOptions`, and a `PathBuf`.
- [src/svg.rs](src/svg.rs) — **new.** Produces a `String` of self-contained SVG. Uses horizontal run-length encoding (one `<rect>` per run of consecutive same-color cells in a row) so large outputs stay tractable. Respects `RenderOptions.cell_size` and `RenderOptions.show_borders`.
- [src/json.rs](src/json.rs) — **new.** Wraps `serde_json::to_string_pretty` on a serializable shadow of `SimulationResult`. `BoundaryMode` serializes to its variant name (`"ZeroPadded"`, `"Wrap"`).
- [src/lib.rs](src/lib.rs) — **new.** Declares the modules, defines `run()`, `Output`, `Error`, `SimConfig`, `InitialRow`, `RenderOptions`, `OutputKind`. Re-exports the simulation types from `sim`. Carries the crate-level rustdoc.
- [Cargo.toml](Cargo.toml) — adds `[lib]` (or relies on default discovery via `src/lib.rs`), adds `serde` (with `derive`) and `serde_json` to `[dependencies]`. The `iced`/`base64`/`chrono` deps stay.

## HTML export — feature preservation

**Hard rule: the HTML export's current feature set is preserved exactly.** The only thing the library is allowed to change is *where* the files go (caller-provided directory instead of `runs_dir()`). Everything else — file format, in-page UI, manifest schema, index regeneration, filename convention — stays as it is in [src/export.rs](src/export.rs). A diff of two `.html` files produced before and after the refactor, given identical inputs, should differ only in fields derived from time (the timestamp).

Preserved in full:

**Per-run HTML file ([export.rs:62-271](src/export.rs#L62-L271))**

- **Page header metadata block:** job id, rule, width, generations (shown as `progress / generations`), boundary, status, exported timestamp, preview-mode string ("full fidelity (single canvas)" or "full fidelity: N tiles (M rows each, last shorter)"). Same labels, same order, same styling.
- **"&lt; back to index" link** at the top of the page pointing at `index.html`.
- **Interactive cell-size control** ([export.rs:140-146](src/export.rs#L140-L146)): a `<input type="range" min="1" max="16">` slider paired with a `<input type="number" min="1" max="64">` numeric input, both bidirectional, both updating the canvas live.
- **Interactive borders toggle** ([export.rs:145](src/export.rs#L145)): a `<input type="checkbox">` labeled "Borders" that re-renders all tiles when changed.
- **Initial cell size and initial borders state** are seeded from the `RenderOptions` the caller passed to `run()`, exactly as today's UI seeds them from `App.show_borders` and the computed `cs`.
- **Multi-canvas tiling** ([export.rs:73-105](src/export.rs#L73-L105)) using `MAX_EXPORT_CELLS = 200_000_000` and `MAX_EXPORT_HEIGHT = 32_000`. Each tile is one `<canvas>` stacked vertically with no gap. The tile-count note ("Rendered N stacked canvases…") still appears when N > 1.
- **Bit-packed base64 data** ([export.rs:273-300](src/export.rs#L273-L300)): one row's worth of bits packed MSB-first, base64-encoded, embedded as `<script id="bitsN" type="application/octet-stream">` per tile. Same packing layout.
- **JS rendering with two paths:**
  - `cs === 1`: `ctx.createImageData` fast path, white = 0 / black = 1, `image-rendering: pixelated; crisp-edges` ([export.rs:202-214](src/export.rs#L202-L214)). No grid lines (matches today's "cs=1 means no grid lines" comment).
  - `cs > 1`: per-cell `fillRect`. With borders on, draw `#4d4d4d` background then white/black cells inset by 1px on each side; without borders, white background then black cells edge-to-edge ([export.rs:215-242](src/export.rs#L215-L242)).
- **Page styling:** body font / margin / colors, `dl` grid, canvas `border: 1px solid #ccc; max-width: 1600px`, and the rest of the inline CSS at [export.rs:118-126](src/export.rs#L118-L126).

**Manifest ([export.rs:302-349](src/export.rs#L302-L349))**

- File name `manifest.tsv`.
- Column order and headers, exactly: `id`, `rule`, `width`, `generations`, `boundary`, `status`, `progress`, `timestamp`, `filename`.
- Header row written on first creation; append-only afterward.

**Index page ([export.rs:351-524](src/export.rs#L351-L524))**

- File name `index.html`, regenerated on every export.
- Live reload of `manifest.tsv` from the page (the "live manifest.tsv reload on index" feature from the recent commit).
- Stale-entry filtering (the "stale index entries" fix from the recent commit) preserved.
- Same column layout, same styling, same per-row link to the exported HTML.

**File naming and directory layout**

- Per-run filename: `rule{:03}_w{}_g{}_{boundary_short}_{ts}_job{}.html` ([export.rs:47-50](src/export.rs#L47-L50)) with `boundary_short = "zp" | "wr"` and `ts = "%Y%m%d_%H%M%S"`. Unchanged.
- Inside the target directory: the run HTML files alongside `manifest.tsv` and `index.html`. Unchanged.

**Inputs to the export (internal, not part of public API)**

The internal `ExportInput` keeps every field it has today — `job_id`, `rule`, `width`, `generations`, `progress`, `boundary`, `status`, `rows`, `show_borders`. When `run(..., OutputKind::Html { dir })` is called from the library, those fields are filled as follows:

- `job_id` — monotonically incremented within a single library-using process, starting at 1. Persistence across process restarts is out of scope; the user gets a fresh sequence per process. (Consumers who care about unique-across-time IDs can rename files themselves.)
- `progress` — always equal to `generations` (library runs go to completion synchronously).
- `status` — always `"Done"` (the library doesn't expose cancellation).
- `show_borders` — taken from `RenderOptions.show_borders`.
- Everything else — direct from `SimConfig` / `SimulationResult`.

Cancellation, in-progress exports, and the "Running"/"Cancelled" status values stay reachable only from the binary's UI mode, which still goes through the same export function with its own values for those fields.

## Edge cases and constraints

- **`width == 0`** → return `Error::WidthZero`. The current sim core silently returns empty rows ([sim.rs:132-135](src/sim.rs#L132-L135)); from the library that should be an explicit error.
- **`InitialRow::Explicit(v)` where `v.len() != config.width`** → `Error::InitialRowLengthMismatch`.
- **`InitialRow::Explicit(v)` containing values other than 0 or 1** → `Error::NonBinaryCellValue { position, value }`. Defensive — the current step function masks via `& 1` ([sim.rs:84-88](src/sim.rs#L84-L88)) and tolerates it, but for a high-schooler-facing library it's clearer to reject with a precise message.
- **`InitialRow::FromSeed { seed }` where `seed.len() > config.width`** → `Error::SeedWiderThanWidth`. Today the helper truncates silently ([sim.rs:156](src/sim.rs#L156)); make it explicit.
- **`generations == 0`** → valid. `rows` is just the initial row. Outputs render a single horizontal strip.
- **Very large SVGs.** Even with RLE, a 1000-wide × millions-of-generations run will produce an SVG no editor can open. SVG output of more than ~50M cells emits a warning in the rustdoc and a non-fatal note printed to stderr; it still runs. No hard cap — the user can decide.
- **Very large JSON.** A 1000-wide × 1M-row run is ~2 GB of JSON. Documented; no hard cap; the rustdoc points the caller at `Structured` for large workloads.
- **UI mode threading constraints.** `iced::Application::run` requires the main thread on macOS and Windows-with-some-graphics-backends. The library documents this — `OutputKind::Ui` must be invoked from `main()` or an equivalent main-thread context. Calling it a second time in the same process is not supported (iced limitation); the library documents this rather than enforcing it.
- **`runs/` target directory.** The library does *not* create directories outside the path the caller provides. If `dir` does not exist, it is created (mirroring today's `fs::create_dir_all` at [export.rs:36](src/export.rs#L36)). No default — the caller passes the path explicitly. The thin binary defaults its own call to `./runs/` so behavior on `cargo run` is unchanged.
- **Thread safety.** `SimulationResult` is `Send + Sync`. `run()` is reentrant for non-`Ui` outputs (multiple threads can run simulations concurrently). The `Ui` variant is not reentrant within a process.

## Performance requirements and design decisions

- **Flat row-major storage.** `SimulationResult.rows` is `Vec<u8>`, length `(generations + 1) * width`, not `Vec<Vec<u8>>`. **Why:** matches the internal representation used by the GUI's job storage ([main.rs:85-93](src/main.rs#L85-L93)), avoids millions of small heap allocations on long runs, gives the caller a contiguous buffer they can pass to SIMD / FFI / GPU code if they want to.
- **In-place stepping.** Simulation reuses `step_row_into` ([sim.rs:175](src/sim.rs#L175)), which writes the next row into a caller-owned buffer with no allocation per generation.
- **No per-row rayon.** The previous parallel-per-row implementation was a measured regression — at width ~1000, thread-dispatch overhead dwarfed the actual compute and starved the UI thread ([sim.rs:170-174](src/sim.rs#L170-L174)). **Why we record this:** it is exactly the kind of "obvious" optimization a future contributor would re-introduce without realizing it was already tried. The rustdoc on the simulation entry point should call it out so the lesson sticks.
- **SVG via run-length encoding.** Emit one `<rect>` per maximal horizontal run of same-color cells. **Why:** SVG is chosen by callers who want a *vector* output (figure embedding, Inkscape). A raster-fallback would defeat the point. RLE typically shrinks rect count by an order of magnitude or more on rule-30-style outputs and keeps the file editable.
- **HTML output preserves the existing export entirely.** See the dedicated "HTML export — feature preservation" section. The library is allowed to change *where* files land (caller-supplied directory replaces `runs_dir()` lookup) and nothing else.
- **Throughput target.** The release benchmark `bench_2m_x_1000` ([sim.rs:240](src/sim.rs#L240)) is the regression gate. Library refactor must not measurably slow it.

## Documentation requirements

Two artifacts, both written for a high-schooler with some programming experience.

**(1) Rustdoc on every public item.**
- Each struct, enum, variant, field, and function has a doc comment.
- The crate-level doc (`//!` in `lib.rs`) opens with a one-paragraph explanation of what an elementary cellular automaton is (rule number, neighborhood of three, generations stacked vertically), then walks through each `OutputKind` with a small copy-pasteable example.
- Doc-comments include `# Examples` blocks compiled as doctests where reasonable.
- Errors document what triggers them and how to fix the input.

**(2) `DOCUMENTATION.md` at the repo root.**
- A single long-form markdown guide, not a generated book.
- Structure (suggested, not prescribed): What a CA is → Installing the library → The four-and-a-half output modes → Walkthrough of every input struct field by field, with diagrams in ASCII where it helps → Common patterns ("save an SVG figure", "build a gallery of runs", "iterate the cells in Rust") → Performance notes and limits → Error reference → Glossary.
- Tone: assumes the reader has written *some* Rust before but is not fluent. Explains things like why we use `u8` for a binary cell value, what `Vec<u8>` means as a row, what row-major layout is.
- Every code snippet must compile.

## Decisions made (with rationale)

| Decision | Why |
|---|---|
| One `run()` function with an `OutputKind` parameter, not five separate top-level functions | The user's mental model is "pick what I want out and call the thing." Matches their stated API shape. |
| `SimulationResult.rows` is flat `Vec<u8>`, not nested | Performance (single allocation), matches GUI internals, callers can still build row views if they need them. |
| Returns `Result<Output, Error>`, never panics on bad input | The audience is a high-schooler — clear errors are much friendlier than `unwrap()` traces. |
| HTML keeps `manifest.tsv` + `index.html` gallery behavior, target dir is a required parameter | User picked option (b) — keep the gallery feature, but the library must not silently create folders in someone else's project. |
| No feature flags; all deps always on | Simpler for the audience; the size cost of `iced` is acceptable for a research tool. |
| Single thin `main.rs` stays so `cargo run` still works | Lowest-friction option; no workflow regression for the user. |
| SVG uses RLE rectangles instead of `<image>` fallback for big runs | Preserves the vector-output property that's the whole reason to pick SVG. |
| SVG output stays static — no JS controls, no `<foreignObject>` HTML inputs, no embedded bit-packed data | SVG is for figure-grade vector output (papers, Inkscape, LaTeX). Interactive features would render invisibly in those tools and pay a size cost even when callers only want a figure. The HTML output is the home for interactivity; SVG and HTML serve different use cases on purpose. |
| `runs/` directory does *not* default in the library API; the binary defaults to `./runs/` for itself | Library should have no hidden filesystem side effects; binary should match today's behavior. |
| Single `DOCUMENTATION.md` rather than mdBook | One file is easier to read end-to-end and easier to keep in sync. |
| `Explicit` initial rows are validated for length and binary-ness | Rejecting bad input loudly is more helpful than masking; the cost is one length check + one pass over the row. |

## Open questions

- **Crate name vs module name.** Stays `cellular_automata` (matches today's `Cargo.toml`). Confirm this is fine when used as `use cellular_automata::run;` from another project — long, but greppable.
- **`serde` derives on the public types.** Plan is to derive `Serialize`/`Deserialize` on `SimConfig`, `InitialRow`, `RenderOptions`, `SimulationResult` so callers can deserialize a saved config and replay. Not strictly required by the conversation, but free if we're already pulling in serde for JSON output.
- **Whether to expose `step_row_into` publicly.** It's the fastest in-place step function. Useful for advanced callers; cluttering for beginners. Default: keep it `pub` but mark it `# Advanced` in the docs.

## Out of scope / deferred

- crates.io publish flow, semver policy, CHANGELOG.
- WebAssembly build.
- Async / streaming API (yield rows as they're computed).
- Decimation / stride support on the structured output for memory-bound runs.
- 2D CAs, totalistic rules, multi-color cells.
- A real CLI binary with argument parsing.
- Concurrent multiple-`Ui` support.
