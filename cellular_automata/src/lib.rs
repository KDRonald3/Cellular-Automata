//! # `cellular_automata`
//!
//! Run, render, and explore elementary 1D cellular automata.
//!
//! An *elementary cellular automaton* is a row of binary cells that updates
//! in lock-step generation after generation. Each cell looks at three inputs
//! — its left neighbour, itself, and its right neighbour — and picks its
//! next value from an 8-entry lookup table named by a single 0-255 number
//! (Wolfram's "rule number"). Stacking the rows vertically produces the
//! familiar triangular patterns of rule 30, rule 90, rule 110, and friends.
//!
//! This crate gives you one function — [`run`] — and asks you to pick what
//! kind of output you want. The same inputs flow into all five output
//! modes, so the typical usage is "tweak the config, change the
//! [`OutputKind`], call `run` again."
//!
//! ## Example
//!
//! ```no_run
//! use cellular_automata::{
//!     run, BoundaryMode, InitialRow, OutputKind, RenderOptions, SimConfig,
//! };
//!
//! let config = SimConfig {
//!     rule: 30,
//!     width: 401,
//!     generations: 200,
//!     boundary: BoundaryMode::ZeroPadded,
//! };
//! let initial = InitialRow::FromSeed {
//!     seed: vec![1],
//!     align: cellular_automata::PaddingAlign::Center,
//!     fill: cellular_automata::PaddingFill::Zero,
//! };
//! let render = RenderOptions::default();
//!
//! // Get the cells in memory for further analysis:
//! let cellular_automata::Output::Structured(result) =
//!     run(config.clone(), initial.clone(), render, OutputKind::Structured).unwrap()
//! else { unreachable!() };
//! assert_eq!(result.rows.len(), (config.generations + 1) * config.width);
//!
//! // Or get the same simulation as a string of pretty JSON:
//! let cellular_automata::Output::Json(json) =
//!     run(config, initial, render, OutputKind::Json).unwrap()
//! else { unreachable!() };
//! assert!(json.contains("\"rule\""));
//! ```
//!
//! ## The five output modes
//!
//! - [`OutputKind::Structured`] — returns a [`SimulationResult`] with the
//!   full grid in a flat `Vec<u8>`. The fastest option for in-process
//!   analysis: one allocation, zero per-row heap traffic.
//! - [`OutputKind::Json`] — returns a pretty-printed JSON string. Useful
//!   for piping into other tools or saving a reproducible snapshot.
//! - [`OutputKind::Svg`] — returns a self-contained SVG string. Drop it
//!   into a paper, a slide, or Inkscape.
//! - [`OutputKind::Html { dir }`] — writes a stand-alone HTML page with an
//!   interactive cell-size slider and a borders toggle. Good for sharing.
//!
//! ## Performance notes
//!
//! The simulation core uses an in-place row stepper
//! ([`sim::step_row_into`]) and a single contiguous `Vec<u8>` for the whole
//! grid. There is no per-row threading and no per-row allocation; long runs
//! at width ~1000 scale linearly. The release benchmark
//! `sim::tests::bench_2m_x_1000` is the regression gate — keep it honest.

pub mod export;
pub mod json;
pub mod sim;
pub mod svg;

pub use sim::{
    check_explicit_row, make_initial_row, step_row, step_row_into, BoundaryMode,
    CellularAutomaton, ExplicitRowError, PaddingAlign, PaddingFill, Rule,
};

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// Simulation-level inputs: which rule, how wide, how many generations, and
/// how the row's edges connect.
///
/// `width` must be greater than zero. The total grid size in cells is
/// `(generations + 1) * width`; for very long runs prefer
/// [`OutputKind::Structured`] over the textual outputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimConfig {
    /// Wolfram rule number, 0-255.
    pub rule: u8,
    /// Number of cells in each row. Must be >= 1.
    pub width: usize,
    /// How many extra rows to compute after the initial row. The result
    /// contains `generations + 1` rows in total.
    pub generations: usize,
    /// How the leftmost and rightmost cells see their off-grid neighbours.
    pub boundary: BoundaryMode,
}

/// How to construct the initial row.
///
/// Choose [`Explicit`](InitialRow::Explicit) when you already have the full
/// row pre-computed. Choose [`FromSeed`](InitialRow::FromSeed) when you have
/// a short pattern and want the library to pad it out to `config.width`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InitialRow {
    /// A complete row whose length equals `config.width`. Each entry must
    /// be `0` or `1`; any other value produces
    /// [`Error::NonBinaryCellValue`].
    Explicit(Vec<u8>),
    /// A short seed that gets aligned inside a `config.width`-cell row, with
    /// the cells outside the seed filled with `fill`. `seed.len()` must be
    /// less than or equal to `config.width`.
    FromSeed {
        seed: Vec<u8>,
        align: PaddingAlign,
        fill: PaddingFill,
    },
}

/// Rendering knobs shared by the visual outputs (SVG, HTML).
///
/// `cell_size` is the size of each cell in pixels. `show_borders` draws a
/// grid between cells when `cell_size > 1`; with `cell_size == 1` the flag
/// is ignored because a 1-pixel cell has nowhere to put a border.
/// `border_width` controls the thickness of that grid — *fractional values
/// are supported* (e.g. `0.5`, `1.5`) and are clamped to `[0.0, cell_size]`
/// at render time. Sub-pixel widths are anti-aliased by the SVG / canvas
/// renderers; integer widths stay crisp.
///
/// The SVG output additionally draws a thin black frame around the whole
/// image *regardless* of `show_borders` — that frame is SVG-specific and
/// is meant to give a figure dropped into LaTeX a visible outline by
/// default. HTML does not get the same outer frame.
///
/// [`RenderOptions::default()`] turns inter-cell borders off and uses a
/// 4-pixel cell size with `border_width: 1.0`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RenderOptions {
    pub cell_size: u32,
    pub show_borders: bool,
    /// Thickness of the inter-cell grid in pixels (or canvas units in the
    /// UI). Fractional values are allowed; the renderer anti-aliases
    /// sub-pixel widths. Clamped to `[0.0, cell_size]` when borders are
    /// drawn; ignored when `show_borders` is `false` or `cell_size <= 1`.
    /// Values >= `cell_size` hide the cell content entirely — usually
    /// not what you want.
    pub border_width: f32,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions {
            cell_size: 4,
            show_borders: false,
            border_width: 1.0,
        }
    }
}

/// Which output kind to produce from [`run`].
#[derive(Debug, Clone)]
pub enum OutputKind {
    /// Return the cells as a [`SimulationResult`].
    Structured,
    /// Return a pretty-printed JSON string with the cells nested as a 2D
    /// array.
    Json,
    /// Return a self-contained SVG string. White background, black cells.
    Svg,
    /// Write an HTML page (and update the `manifest.tsv` + `index.html`
    /// gallery) in `dir`. The directory is created if it does not exist.
    Html { dir: PathBuf },
}

/// What [`run`] returns. The variant always matches the [`OutputKind`] the
/// caller asked for.
#[derive(Debug)]
pub enum Output {
    Structured(SimulationResult),
    Json(String),
    Svg(String),
    /// Path of the freshly written HTML file inside the caller-chosen
    /// directory.
    Html(PathBuf),
}

/// The complete cell grid produced by a simulation, plus the inputs that
/// produced it.
///
/// `rows` is stored row-major in a single allocation: cell `(y, x)` lives at
/// `rows[y * width + x]`. The length is exactly `(generations + 1) * width`,
/// and every byte is `0` or `1`. Iterate or slice it however you like; pass
/// it to FFI, SIMD code, or your own renderer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationResult {
    pub config: SimConfig,
    pub width: usize,
    pub generations: usize,
    /// Flat row-major grid; see the struct-level doc for the layout.
    pub rows: Vec<u8>,
}

/// Anything that can go wrong inside [`run`]. The library never panics on
/// bad inputs.
#[derive(Debug)]
pub enum Error {
    /// `config.width == 0`. Pick a positive width.
    WidthZero,
    /// `InitialRow::Explicit(v)` was passed but `v.len() != config.width`.
    InitialRowLengthMismatch { expected: usize, got: usize },
    /// `InitialRow::FromSeed { seed, .. }` was passed but `seed` is wider
    /// than `config.width`. Shorten the seed or widen the config.
    SeedWiderThanWidth { width: usize, seed_len: usize },
    /// A cell in an [`InitialRow::Explicit`] row was something other than 0
    /// or 1.
    NonBinaryCellValue { position: usize, value: u8 },
    /// HTML export failed at the filesystem layer (permission denied, disk
    /// full, etc.).
    HtmlExportFailed(std::io::Error),
    /// The OS refused to allocate the backing buffer for the requested
    /// `(generations + 1) * width` cells. Reduce either dimension.
    AllocationFailed { bytes_requested: usize },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::WidthZero => write!(f, "width must be greater than 0"),
            Error::InitialRowLengthMismatch { expected, got } => write!(
                f,
                "initial row length {got} does not match config.width {expected}"
            ),
            Error::SeedWiderThanWidth { width, seed_len } => write!(
                f,
                "seed length {seed_len} is wider than config.width {width}"
            ),
            Error::NonBinaryCellValue { position, value } => write!(
                f,
                "cell at position {position} has value {value}; only 0 or 1 are allowed"
            ),
            Error::HtmlExportFailed(e) => write!(f, "HTML export failed: {e}"),
            Error::AllocationFailed { bytes_requested } => write!(
                f,
                "could not allocate {bytes_requested} bytes for the simulation grid; \
                 reduce generations or width"
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::HtmlExportFailed(e) => Some(e),
            _ => None,
        }
    }
}

/// Run a cellular automaton and produce the requested kind of output.
///
/// The same inputs flow into every output mode — change the
/// [`OutputKind`] and call again. See the [crate-level docs](crate) for an
/// end-to-end example.
///
/// # Errors
///
/// Returns [`Error::WidthZero`] if `config.width == 0`,
/// [`Error::InitialRowLengthMismatch`] /
/// [`Error::NonBinaryCellValue`] / [`Error::SeedWiderThanWidth`] for bad
/// initial rows, and [`Error::HtmlExportFailed`] if `OutputKind::Html`
/// could not write to disk.
pub fn run(
    config: SimConfig,
    initial: InitialRow,
    render: RenderOptions,
    output: OutputKind,
) -> Result<Output, Error> {
    if config.width == 0 {
        return Err(Error::WidthZero);
    }

    let (initial_row, padding_align, padding_fill) = build_initial_row(&config, initial)?;

    match output {
        OutputKind::Structured => {
            let result = simulate(config, initial_row)?;
            Ok(Output::Structured(result))
        }
        OutputKind::Json => {
            let result = simulate(config, initial_row)?;
            Ok(Output::Json(json::to_json(&result)))
        }
        OutputKind::Svg => {
            let result = simulate(config, initial_row)?;
            Ok(Output::Svg(svg::to_svg(&result, &render)))
        }
        OutputKind::Html { dir } => {
            let result = simulate(config, initial_row)?;
            let path = export_html(&result, &render, &dir, padding_fill, padding_align)
                .map_err(Error::HtmlExportFailed)?;
            Ok(Output::Html(path))
        }
    }
}

/// Save an already-computed [`SimulationResult`] to an HTML file in `dir`.
/// Equivalent to calling `run(..., OutputKind::Html { dir })` but skips
/// re-running the simulation.
pub fn save_result(
    result: &SimulationResult,
    render: &RenderOptions,
    dir: &std::path::Path,
    padding_fill: PaddingFill,
    padding_align: PaddingAlign,
) -> Result<PathBuf, Error> {
    export_html(result, render, dir, padding_fill, padding_align)
        .map_err(Error::HtmlExportFailed)
}

fn build_initial_row(
    config: &SimConfig,
    initial: InitialRow,
) -> Result<(Vec<u8>, PaddingAlign, PaddingFill), Error> {
    match initial {
        InitialRow::Explicit(v) => match sim::check_explicit_row(config.width, &v) {
            Ok(()) => Ok((v, PaddingAlign::Center, PaddingFill::Zero)),
            Err(ExplicitRowError::LengthMismatch { expected, got }) => {
                Err(Error::InitialRowLengthMismatch { expected, got })
            }
            Err(ExplicitRowError::NonBinary { position, value }) => {
                Err(Error::NonBinaryCellValue { position, value })
            }
        },
        InitialRow::FromSeed { seed, align, fill } => {
            if seed.len() > config.width {
                return Err(Error::SeedWiderThanWidth {
                    width: config.width,
                    seed_len: seed.len(),
                });
            }
            let row = make_initial_row(config.width, &seed, align, fill.value());
            Ok((row, align, fill))
        }
    }
}

fn simulate(config: SimConfig, initial_row: Vec<u8>) -> Result<SimulationResult, Error> {
    let rule = Rule::new(config.rule);
    let width = config.width;
    let generations = config.generations;
    let total_rows = generations.saturating_add(1);
    let total_cells = total_rows.saturating_mul(width);

    // try_reserve_exact lets a too-large config fail with a friendly
    // Error::AllocationFailed instead of an OS-level abort or a panic
    // from Vec::with_capacity.
    let mut flat: Vec<u8> = Vec::new();
    flat.try_reserve_exact(total_cells).map_err(|_| Error::AllocationFailed {
        bytes_requested: total_cells,
    })?;
    flat.extend_from_slice(&initial_row);

    let mut prev = initial_row;
    let mut next = vec![0u8; width];

    for _ in 0..generations {
        sim::step_row_into(&rule, &prev, &mut next, config.boundary, 0);
        // SAFETY-NOTE: `flat` was reserved up front for the full grid;
        // `extend_from_slice` is bounded by `next.len() == width` per
        // iteration and never reallocates.
        flat.extend_from_slice(&next);
        std::mem::swap(&mut prev, &mut next);
    }

    Ok(SimulationResult {
        config,
        width,
        generations,
        rows: flat,
    })
}

/// Monotonic per-process counter used to fill the `job_id` slot in the HTML
/// export. Restarts at 1 on each process launch — persistence across runs
/// is out of scope, callers who need stable IDs can rename the produced
/// files themselves.
static NEXT_JOB_ID: AtomicU64 = AtomicU64::new(1);

fn export_html(
    result: &SimulationResult,
    render: &RenderOptions,
    dir: &std::path::Path,
    padding_fill: PaddingFill,
    padding_align: PaddingAlign,
) -> std::io::Result<PathBuf> {
    let job_id = NEXT_JOB_ID.fetch_add(1, Ordering::Relaxed);
    let status_str = "Done";
    let input = export::ExportInput {
        job_id,
        rule: result.config.rule,
        width: result.width,
        generations: result.generations,
        progress: result.generations,
        boundary: result.config.boundary,
        status: status_str,
        rows: &result.rows,
        cell_size: render.cell_size,
        show_borders: render.show_borders,
        border_width: render.border_width,
        padding_fill,
        padding_align,
    };
    export::export_job(&input, dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule30_config() -> (SimConfig, InitialRow) {
        let config = SimConfig {
            rule: 30,
            width: 21,
            generations: 5,
            boundary: BoundaryMode::ZeroPadded,
        };
        let initial = InitialRow::FromSeed {
            seed: vec![1],
            align: PaddingAlign::Center,
            fill: PaddingFill::Zero,
        };
        (config, initial)
    }

    #[test]
    fn structured_run_produces_expected_grid_size() {
        let (config, initial) = rule30_config();
        let render = RenderOptions { cell_size: 4, show_borders: false, border_width: 1.0 };
        let Output::Structured(r) = run(config.clone(), initial, render, OutputKind::Structured).unwrap()
        else { panic!() };
        assert_eq!(r.rows.len(), (config.generations + 1) * config.width);
        for &b in &r.rows {
            assert!(b == 0 || b == 1);
        }
    }

    #[test]
    fn width_zero_is_rejected() {
        let config = SimConfig {
            rule: 30,
            width: 0,
            generations: 1,
            boundary: BoundaryMode::ZeroPadded,
        };
        let initial = InitialRow::Explicit(Vec::new());
        let render = RenderOptions { cell_size: 1, show_borders: false, border_width: 1.0 };
        assert!(matches!(
            run(config, initial, render, OutputKind::Structured),
            Err(Error::WidthZero)
        ));
    }

    #[test]
    fn explicit_length_mismatch_is_rejected() {
        let config = SimConfig {
            rule: 30,
            width: 4,
            generations: 1,
            boundary: BoundaryMode::ZeroPadded,
        };
        let initial = InitialRow::Explicit(vec![0, 1, 1]);
        let render = RenderOptions { cell_size: 1, show_borders: false, border_width: 1.0 };
        let err = run(config, initial, render, OutputKind::Structured).unwrap_err();
        assert!(matches!(
            err,
            Error::InitialRowLengthMismatch { expected: 4, got: 3 }
        ));
    }

    #[test]
    fn explicit_non_binary_is_rejected() {
        let config = SimConfig {
            rule: 30,
            width: 3,
            generations: 1,
            boundary: BoundaryMode::ZeroPadded,
        };
        let initial = InitialRow::Explicit(vec![0, 2, 1]);
        let render = RenderOptions { cell_size: 1, show_borders: false, border_width: 1.0 };
        let err = run(config, initial, render, OutputKind::Structured).unwrap_err();
        assert!(matches!(
            err,
            Error::NonBinaryCellValue { position: 1, value: 2 }
        ));
    }

    #[test]
    fn seed_wider_than_width_is_rejected() {
        let config = SimConfig {
            rule: 30,
            width: 3,
            generations: 1,
            boundary: BoundaryMode::ZeroPadded,
        };
        let initial = InitialRow::FromSeed {
            seed: vec![1, 1, 1, 1],
            align: PaddingAlign::Center,
            fill: PaddingFill::Zero,
        };
        let render = RenderOptions { cell_size: 1, show_borders: false, border_width: 1.0 };
        let err = run(config, initial, render, OutputKind::Structured).unwrap_err();
        assert!(matches!(
            err,
            Error::SeedWiderThanWidth { width: 3, seed_len: 4 }
        ));
    }

    #[test]
    fn generations_zero_is_valid() {
        let config = SimConfig {
            rule: 30,
            width: 4,
            generations: 0,
            boundary: BoundaryMode::Wrap,
        };
        let initial = InitialRow::Explicit(vec![1, 0, 0, 0]);
        let render = RenderOptions { cell_size: 1, show_borders: false, border_width: 1.0 };
        let Output::Structured(r) = run(config, initial, render, OutputKind::Structured).unwrap()
        else { panic!() };
        assert_eq!(r.rows, vec![1, 0, 0, 0]);
    }

    #[test]
    fn json_contains_config_and_rows() {
        let (config, initial) = rule30_config();
        let render = RenderOptions { cell_size: 4, show_borders: false, border_width: 1.0 };
        let Output::Json(s) = run(config, initial, render, OutputKind::Json).unwrap()
        else { panic!() };
        assert!(s.contains("\"rule\""));
        assert!(s.contains("\"rows\""));
        assert!(s.contains("\"ZeroPadded\""));
    }

    #[test]
    fn svg_contains_svg_root() {
        let (config, initial) = rule30_config();
        let render = RenderOptions { cell_size: 4, show_borders: false, border_width: 1.0 };
        let Output::Svg(s) = run(config, initial, render, OutputKind::Svg).unwrap()
        else { panic!() };
        assert!(s.contains("<svg"));
        assert!(s.contains("</svg>"));
    }

    #[test]
    fn svg_always_has_outer_frame() {
        // The outer 1-pixel black stroke is drawn regardless of
        // show_borders so SVGs always have a visible outline.
        for borders in [false, true] {
            let (config, initial) = rule30_config();
            let render = RenderOptions { cell_size: 4, show_borders: borders, border_width: 1.0 };
            let Output::Svg(s) = run(config, initial, render, OutputKind::Svg).unwrap()
            else { panic!() };
            assert!(
                s.contains("stroke=\"#000000\""),
                "expected outer frame stroke (borders={borders})"
            );
        }
    }

    #[test]
    fn render_options_default_has_borders_off() {
        let r = RenderOptions::default();
        assert!(!r.show_borders);
        assert_eq!(r.cell_size, 4);
        assert_eq!(r.border_width, 1.0);
    }

    #[test]
    fn fractional_border_width_renders_in_svg() {
        let (config, initial) = rule30_config();
        let render = RenderOptions { cell_size: 4, show_borders: true, border_width: 0.5 };
        let Output::Svg(s) = run(config, initial, render, OutputKind::Svg).unwrap()
        else { panic!() };
        // With bw = 0.5 the rect at row 0, col 0 sits at (0.5, 0.5).
        assert!(s.contains("x=\"0.5\""));
        assert!(s.contains("y=\"0.5\""));
        // Fractional widths flip the SVG-level shape-rendering hint.
        assert!(s.contains("shape-rendering=\"auto\""));
    }
}
