# `cellular_automata` — a friendly guide

Welcome. This guide explains how to use the `cellular_automata` library
from your own Rust project. It assumes you have written *some* Rust before
— you know what `Vec<u8>` is, you've called `cargo run`, you don't need
ownership re-explained — but it does not assume you have built a lot of
libraries or that you have ever touched a cellular automaton.

If you just want to play with the visualiser, run `cargo run --release` in
this folder and skip ahead to the [UI mode section](#ui-mode-cargo-run).

---

## 1. What is a cellular automaton?

A *cellular automaton* (plural: *automata*) is one of the simplest things
that can compute. You start with a row of cells; each cell is either `0`
(off, drawn white) or `1` (on, drawn black). Then you apply a rule, over
and over, to generate a new row. Stack the rows on top of each other and
you get a picture.

This crate handles the simplest possible kind: **elementary 1D cellular
automata**.

- **1D** — every row is a single line, not a grid. The rows are 1
  dimensional even though the picture you draw of many rows looks 2D.
- **Binary** — every cell is `0` or `1`. No greys, no colours.
- **Elementary** — to decide what a cell becomes next, the rule looks at
  three inputs: the cell's left neighbour, the cell itself, and the cell's
  right neighbour. Three binary inputs means eight possible neighbourhoods,
  so the rule is just a lookup table with 8 entries.

That 8-entry table can be encoded as a single byte. The bits of the byte
tell you the next state for each neighbourhood. For example, rule 30 in
binary is `00011110`. Read right-to-left, that's the next state for
neighbourhoods `000`, `001`, `010`, `011`, `100`, `101`, `110`, `111`.

This numbering is called the *Wolfram rule number*, and it goes from 0
(boring — everything dies) to 255 (boring — everything turns on). The
interesting rules are scattered in between: 30 looks chaotic, 90 draws
a Sierpinski triangle, 110 is famously Turing-complete.

### What runs to picture conversion looks like

Imagine `width = 7`, `generations = 4`, an initial row with a single `1`
in the middle, rule 30, and `boundary = ZeroPadded`. The library
simulates and stacks:

```
row 0 (initial):   0 0 0 1 0 0 0
row 1:             0 0 1 1 1 0 0
row 2:             0 1 1 0 0 1 0
row 3:             1 1 0 1 1 1 1
row 4:             1 0 0 1 0 0 0
```

When rendered with a few pixels per cell, the `1`s are black, the `0`s are
white, and you get the recognisable rule-30 picture.

---

## 2. Installing the library

The crate currently lives in this repository — there is no crates.io
release. Add it to your `Cargo.toml` as either a *path dependency* (if it
sits next to your project on disk) or a *git dependency* (if it lives in a
remote repo).

```toml
# path:
[dependencies]
cellular_automata = { path = "../cellular_automata" }

# or git:
[dependencies]
cellular_automata = { git = "https://github.com/your-user/cellular_automata" }
```

Then in your code:

```rust
use cellular_automata::{
    run, BoundaryMode, InitialRow, OutputKind, PaddingAlign, PaddingFill,
    RenderOptions, SimConfig,
};
```

All the dependencies (`iced`, `serde`, `base64`, etc.) turn on by default.
There are no feature flags to fiddle with.

---

## 3. The four-and-a-half output modes

Everything goes through one function:

```rust
pub fn run(
    config: SimConfig,
    initial: InitialRow,
    render: RenderOptions,
    output: OutputKind,
) -> Result<Output, Error>;
```

The first three arguments describe the simulation and how it should look.
The fourth tells the library which kind of artifact to produce. The
returned `Output` enum always matches the `OutputKind` you passed in.

### 3a. `OutputKind::Structured` — cells in RAM

Use this when you want the cells in your own Rust code: counting
densities, looking for gliders, slicing for plotting, feeding into a GPU.

```rust
use cellular_automata::*;

let config = SimConfig { rule: 30, width: 401, generations: 200,
                          boundary: BoundaryMode::ZeroPadded };
let initial = InitialRow::FromSeed {
    seed: vec![1],
    align: PaddingAlign::Center,
    fill: PaddingFill::Zero,
};
let render = RenderOptions { cell_size: 4, show_borders: false, border_width: 1.0 };

let Output::Structured(result) =
    run(config, initial, render, OutputKind::Structured).unwrap()
else { unreachable!() };

// One cell per byte, row-major: cell (y, x) is at rows[y * width + x].
let on_count: usize = result.rows.iter().map(|&b| b as usize).sum();
println!("{} black cells out of {}", on_count, result.rows.len());
```

`SimulationResult.rows` is a single `Vec<u8>` — one allocation, contiguous
in memory, ready to hand off to SIMD code or write to disk as raw bytes.

### 3b. `OutputKind::Json` — a portable snapshot

```rust
let Output::Json(s) = run(config, initial, render, OutputKind::Json).unwrap()
else { unreachable!() };
std::fs::write("rule30.json", s).unwrap();
```

The JSON is pretty-printed and looks like:

```json
{
  "config": { "rule": 30, "width": 401, "generations": 200,
              "boundary": "ZeroPadded" },
  "rows": [
    [0, 0, 0, /* ... */, 1, 0, 0, 0],
    [0, 0, 0, /* ... */, 1, 1, 0, 0]
  ]
}
```

JSON is *verbose*. A 1000-wide × 1 000 000-row run blows past 2 GB. If you
hit those scales, do your processing through `OutputKind::Structured` and
only serialize what you actually need.

### 3c. `OutputKind::Svg` — a figure for a paper

```rust
let Output::Svg(s) = run(config, initial, render, OutputKind::Svg).unwrap()
else { unreachable!() };
std::fs::write("rule30.svg", s).unwrap();
```

The SVG is self-contained — no scripts, no external references. It is
vector, so LaTeX, Inkscape, and slide tools handle it as a proper figure.

The library uses **horizontal run-length encoding**: consecutive black
cells in the same row collapse into one `<rect>`. Rule-30-style outputs
shrink by an order of magnitude or more compared to one rect per cell.
There is no hard cap, but SVGs with more than ~50 million cells trigger
a one-line warning to stderr because most editors will struggle to open
them.

The SVG output is static: no interactive widgets, no embedded controls.
For interactivity, use the HTML output below.

### 3d. `OutputKind::Html { dir }` — a shareable web preview

```rust
use std::path::PathBuf;
let Output::Html(path) = run(
    config, initial, render,
    OutputKind::Html { dir: PathBuf::from("./my_runs") },
).unwrap()
else { unreachable!() };
println!("wrote {}", path.display());
```

The library writes three things into `dir`:

1. A new `rule{:03}_w{}_g{}_{boundary}_{ts}_job{N}.html` file with an
   interactive cell-size slider, a borders toggle, and embedded bit-packed
   cell data. Each file also embeds `fill`, `align`, and `ic` (initial
   condition as a decimal string) in its meta JSON block.
2. A line appended to `manifest.tsv` (created on first export). The TSV
   has 12 columns: `id`, `rule`, `width`, `generations`, `boundary`,
   `status`, `progress`, `timestamp`, `filename`, `fill`, `align`, `ic`.
3. A regenerated `index.html` — a single-page application with a collapsible
   sidebar that lists runs grouped by rule number. Clicking a run loads its
   canvas in-place without page navigation. A search box filters entries by
   their human-readable display name, e.g.
   `200 generations of rule 102 padded 0 before 1`.

The human-readable name format is:

```
[generations] generations of rule [rule] [boundary] padded [fill] [align] [ic]
```

where boundary is omitted for `ZeroPadded` and is `wrapped around` for
`Wrap`, and `ic` is the decimal representation of the seed bits (leading
and trailing zeros stripped).

Old exports that pre-date the new fields fall back to showing the technical
filename in the sidebar. If you keep exporting into the same directory, the
`index.html` accumulates every run you have ever made. Stale entries (HTML
files that have been deleted) are filtered out automatically on the next
page load.

The directory is created if it does not exist. The library never creates
directories anywhere else.

### 3e. `OutputKind::Ui` — the interactive app

```rust
run(config, initial, render, OutputKind::Ui).unwrap();
// blocks until the user closes the window
```

Opens the iced application, pre-populated with the values from `config`,
`initial`, and `render`. The user can change anything in the form and
click Run; the simulation happens on a background thread so the UI stays
responsive.

#### UI mode threading constraints

- **Main thread only.** On macOS and on Windows with some graphics
  backends, iced *must* run on the main thread. Call `run` with
  `OutputKind::Ui` from `main()`, not from a worker thread.
- **Once per process.** iced's runtime cannot be started a second time
  in the same process. If you have already opened the UI and the user
  closed it, you cannot re-open it without restarting the program.

---

## 4. The input structs, field by field

### `SimConfig`

```rust
pub struct SimConfig {
    pub rule: u8,
    pub width: usize,
    pub generations: usize,
    pub boundary: BoundaryMode,
}
```

- `rule` — the Wolfram rule number, `0..=255`. There are exactly 256
  elementary CAs; this picks one.
- `width` — how many cells in each row. Must be at least 1. (`width == 0`
  returns `Error::WidthZero`.)
- `generations` — how many *additional* rows to compute after the initial
  row. The result has `generations + 1` rows.
- `boundary` — see below.

### `BoundaryMode`

```rust
pub enum BoundaryMode { ZeroPadded, Wrap }
```

The leftmost cell has no real left neighbour; the rightmost cell has no
real right neighbour. `BoundaryMode` decides what to pretend they look
at:

- `ZeroPadded` — the missing neighbour is `0`. Patterns can fall off the
  edge and disappear forever.
- `Wrap` — the row wraps around into a circle. Patterns that fall off
  the right edge re-enter on the left.

### `InitialRow`

```rust
pub enum InitialRow {
    Explicit(Vec<u8>),
    FromSeed { seed: Vec<u8>, align: PaddingAlign, fill: PaddingFill },
}
```

Two ways to build the first row.

- `Explicit(v)` — you provide the whole row. `v.len()` must equal
  `config.width`, and every entry must be `0` or `1`. Anything else
  returns an `Error::InitialRowLengthMismatch` or
  `Error::NonBinaryCellValue`.
- `FromSeed { seed, align, fill }` — you provide a short pattern and
  the library pads it to `config.width`. `seed.len()` must be `<=
  config.width`. `align` decides where the seed sits; `fill` says what
  to put in the empty cells.

### `PaddingAlign`

```rust
pub enum PaddingAlign { After, Before, Center }
```

Where the seed sits inside the padded row. Suppose `width = 9` and the
seed is `1 0 1`:

| align    | result            |
|----------|-------------------|
| `After`  | `1 0 1 _ _ _ _ _ _` |
| `Before` | `_ _ _ _ _ _ 1 0 1` |
| `Center` | `_ _ _ 1 0 1 _ _ _` |

(The `_` cells get the `fill` value below.)

### `PaddingFill`

```rust
pub enum PaddingFill { Zero, One }
```

What value goes in the cells outside the seed. `Zero` means the seed
sits on a sea of white; `One` means it sits on a sea of black.

### `RenderOptions`

```rust
pub struct RenderOptions {
    pub cell_size: u32,
    pub show_borders: bool,
    pub border_width: f32,
}
```

- `cell_size` — how big each cell is in pixels (for SVG/HTML) or canvas
  units (for UI). The HTML page also lets the user change this
  interactively after the file is written.
- `show_borders` — draws a thin grid between cells when
  `cell_size > 1`. With `cell_size == 1` there is no room for a border,
  so the flag is ignored.
- `border_width` — how thick that grid is, in pixels (or canvas units
  in the UI). **Fractional values are supported** — try `0.5` for a
  hairline grid or `1.5` for something a touch heavier than 1 px.
  Sub-pixel widths are anti-aliased by the renderer (SVG, canvas, iced
  all handle them natively); integer widths stay crisp. Clamped to
  `[0.0, cell_size]` at render time. Ignored when `show_borders` is
  `false`.

The SVG output additionally draws a fixed 0.1-unit black frame around
the whole image **regardless of `show_borders`**, so a figure dropped
into LaTeX always has a visible hairline outline without overwhelming
the content. The HTML and UI outputs do not include this outer frame —
toggle `show_borders` if you want internal grid lines there.

`RenderOptions` implements `Default`: 4-pixel cells, no inter-cell
borders, `border_width: 1.0`. Useful as a starting point when you want to
override one field:

```rust
let render = RenderOptions { show_borders: true, border_width: 0.5,
                              ..RenderOptions::default() };
```

---

## 5. Common patterns

### Save an SVG figure for a paper

```rust
use cellular_automata::*;
use std::fs;

fn rule_figure(rule: u8, name: &str) {
    let config = SimConfig { rule, width: 401, generations: 200,
                              boundary: BoundaryMode::ZeroPadded };
    let initial = InitialRow::FromSeed {
        seed: vec![1],
        align: PaddingAlign::Center,
        fill: PaddingFill::Zero,
    };
    let render = RenderOptions { cell_size: 3, show_borders: false, border_width: 1.0 };
    let Output::Svg(s) =
        run(config, initial, render, OutputKind::Svg).unwrap()
    else { unreachable!() };
    fs::write(name, s).unwrap();
}

rule_figure(30, "rule30.svg");
rule_figure(90, "rule90.svg");
rule_figure(110, "rule110.svg");
```

### Build a gallery of HTML runs

Point every call at the same directory and the gallery in
`index.html` grows:

```rust
use std::path::PathBuf;
let dir = PathBuf::from("./my_runs");
for rule in [30u8, 54, 90, 110] {
    let config = SimConfig { rule, width: 401, generations: 200,
                              boundary: BoundaryMode::ZeroPadded };
    let initial = InitialRow::FromSeed {
        seed: vec![1],
        align: PaddingAlign::Center,
        fill: PaddingFill::Zero,
    };
    let render = RenderOptions { cell_size: 4, show_borders: true, border_width: 1.5 };
    run(config, initial, render, OutputKind::Html { dir: dir.clone() }).unwrap();
}
// open ./my_runs/index.html in your browser
```

### Iterate the cells in Rust

```rust
let Output::Structured(result) =
    run(config, initial, render, OutputKind::Structured).unwrap()
else { unreachable!() };

let width = result.width;
for (y, row) in result.rows.chunks(width).enumerate() {
    let on = row.iter().filter(|&&b| b == 1).count();
    println!("row {}: {} on cells", y, on);
}
```

### Read back a JSON snapshot

`SimConfig`, `InitialRow`, `RenderOptions`, and `SimulationResult` all
implement `serde::Deserialize`, so you can round-trip a JSON file back
into the structured form:

```rust
let s = std::fs::read_to_string("rule30.json").unwrap();
let result: cellular_automata::SimulationResult =
    serde_json::from_str(&s).unwrap();
println!("rule {} reloaded with {} rows", result.config.rule,
         result.rows.len() / result.width);
```

---

## 6. Performance notes and limits

Some sharp edges worth knowing about.

- **Memory.** The result grid uses one byte per cell. A 1000-wide ×
  10 000 000-generation run is 10 GB. The library's binary refuses runs
  above 8 GiB up front with a friendly error; the library itself does
  not enforce a cap, so plan accordingly.
- **SVG size.** Even with run-length encoding, an SVG of more than ~50M
  cells will not open well in most editors. The library prints a
  one-line warning at that scale but still produces the file. Consider
  exporting fewer generations or using `OutputKind::Structured` and
  rendering the part you care about yourself.
- **JSON size.** Pretty-printed JSON of a 1000 × 1M run is roughly
  2 GB. Mostly useful for snapshots up to a few million cells.
- **No threading inside the simulation.** A previous version parallelised
  the row stepper with rayon and it was a measured regression: at width
  ~1000 the thread-dispatch overhead dwarfed the actual compute and
  starved every other thread on the machine. The current sequential
  in-place stepper is the regression gate (see
  `sim::tests::bench_2m_x_1000`). Please do not "fix" this.
- **`run` is reentrant for non-UI outputs.** Multiple threads can call
  `run(..., OutputKind::Structured | Json | Svg | Html { .. })` at the
  same time. Only `OutputKind::Ui` is single-shot per process.

---

## 7. Error reference

`run` returns `Result<Output, Error>`. The variants:

| Variant | Why it fires | How to fix |
|---|---|---|
| `WidthZero` | `config.width == 0`. | Pick a width of 1 or more. |
| `InitialRowLengthMismatch { expected, got }` | You passed `InitialRow::Explicit(v)` but `v.len() != config.width`. | Either resize `v` or change `config.width`. |
| `SeedWiderThanWidth { width, seed_len }` | The seed in `InitialRow::FromSeed` is longer than `config.width`. | Shorten the seed or widen the config. |
| `NonBinaryCellValue { position, value }` | A cell in `InitialRow::Explicit` was not 0 or 1. | Replace the offending byte with 0 or 1. |
| `HtmlExportFailed(io_err)` | Filesystem error during `OutputKind::Html`. | Check the directory is writable and the disk has space. |
| `UiLaunchFailed(msg)` | iced refused to start. | Usually a graphics-driver or main-thread issue; see the iced docs. |

---

## 8. UI mode (`cargo run`)

The crate also ships a thin binary that just opens the UI with sensible
defaults. From this folder:

```
cargo run --release
```

The release build is *much* faster for long runs — the simulator goes
from a few million cells per second in debug to hundreds of millions in
release.

The interactive UI exports HTML into `./runs/` by default. That is the
same directory the older binary used, so old galleries keep working.

---

## 9. Glossary

- **Cell.** One square on the grid. A 0 or a 1.
- **Row.** All the cells in one generation, side by side.
- **Generation.** One step of the simulation. Generation 0 is the
  initial row; generation 1 is the row computed from generation 0; and
  so on.
- **Rule.** The 8-entry lookup table that maps a (left, center, right)
  triple to the next cell value. Wolfram's numbering compresses this
  table into a single byte.
- **Boundary mode.** How the leftmost and rightmost cells see their
  off-grid neighbours.
- **Row-major.** A way of flattening a 2D grid into a 1D array: row 0
  first, then row 1, then row 2, and so on. Cell `(y, x)` lives at
  index `y * width + x`. The opposite is *column-major*, where you
  store column 0 first. The library uses row-major because that is the
  natural memory order when you stream rows from the simulator.
- **Run-length encoding (RLE).** Replacing a run of identical values
  with a single record that says "this value repeats N times." The SVG
  output uses horizontal RLE on black cells.
- **Wolfram numbering.** The 0–255 naming scheme for elementary CA
  rules invented by Stephen Wolfram in the 1980s. Bit `i` of the rule
  number is the next state when the neighbourhood is the binary form
  of `i`.
