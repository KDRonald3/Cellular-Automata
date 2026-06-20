//! Simulation core for elementary 1D cellular automata.
//!
//! This module holds the cell update rule, the row stepping function, and a
//! helper for building an initial row from a short seed. It is independent of
//! any output format — the higher-level library uses these primitives to run a
//! full simulation and then hands the resulting grid to the various output
//! backends (structured, JSON, SVG, HTML, UI).

use serde::{Deserialize, Serialize};

/// How the cells at the left and right edges of the row see their off-grid
/// neighbours.
///
/// In an elementary CA each cell's next state depends on three inputs: its
/// left neighbour, itself, and its right neighbour. The leftmost and
/// rightmost cells have no real neighbour on one side, so the boundary mode
/// decides what to use instead.
///
/// - [`Padded`](BoundaryMode::Padded): pretend the missing neighbour
///   is `0`. Patterns can grow off the edge and disappear forever.
/// - [`Wrap`](BoundaryMode::Wrap): glue the two edges together so the row
///   becomes a circle. Patterns that fall off the right edge re-enter on the
///   left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoundaryMode {
    Padded,
    Wrap,
}

impl BoundaryMode {
    /// Every variant, in display order. Handy for UI pickers.
    pub const ALL: [BoundaryMode; 2] = [BoundaryMode::Padded, BoundaryMode::Wrap];
}

impl std::fmt::Display for BoundaryMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BoundaryMode::Padded => write!(f, "Padded"),
            BoundaryMode::Wrap => write!(f, "Wrap-around"),
        }
    }
}

/// Where the seed pattern sits inside the initial row when the seed is
/// narrower than the row width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaddingAlign {
    /// Seed is placed at the left edge; the remaining cells are filled.
    After,
    /// Seed is placed at the right edge; the remaining cells are filled
    /// before it.
    Before,
    /// Seed is centered horizontally.
    Center,
}

impl PaddingAlign {
    pub const ALL: [PaddingAlign; 3] = [PaddingAlign::After, PaddingAlign::Before, PaddingAlign::Center];
}

impl std::fmt::Display for PaddingAlign {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PaddingAlign::After => write!(f, "Padding after"),
            PaddingAlign::Before => write!(f, "Padding before"),
            PaddingAlign::Center => write!(f, "Centered"),
        }
    }
}

/// What value the cells outside the seed take in the initial row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaddingFill {
    Zero,
    One,
}

impl PaddingFill {
    pub const ALL: [PaddingFill; 2] = [PaddingFill::Zero, PaddingFill::One];

    /// Returns the raw `u8` (0 or 1) for this fill choice.
    pub fn value(self) -> u8 {
        match self {
            PaddingFill::Zero => 0,
            PaddingFill::One => 1,
        }
    }
}

impl std::fmt::Display for PaddingFill {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PaddingFill::Zero => write!(f, "Fill 0s"),
            PaddingFill::One => write!(f, "Fill 1s"),
        }
    }
}

/// An elementary cellular automaton rule, identified by Wolfram's 0-255
/// numbering.
///
/// The 8-bit rule number names a lookup table of size 8: one entry for each
/// of the eight possible (left, center, right) neighbourhoods. Bit `i` of the
/// number is the next state when the neighbourhood is the binary
/// representation of `i`.
#[derive(Debug, Clone)]
pub struct Rule {
    /// The 0-255 Wolfram rule number.
    pub number: u8,
    table: [u8; 8],
}

impl Rule {
    /// Build a rule from its Wolfram number (0-255).
    pub fn new(number: u8) -> Self {
        let mut table = [0u8; 8];
        for pattern in 0u8..8 {
            table[pattern as usize] = (number >> pattern) & 1;
        }
        Rule { number, table }
    }

    /// Apply the rule to a single (left, center, right) triple.
    #[inline(always)]
    pub fn apply(&self, left: u8, center: u8, right: u8) -> u8 {
        let idx = ((left & 1) << 2) | ((center & 1) << 1) | (right & 1);
        // idx is 0-7 by construction; skip the bounds check.
        unsafe { *self.table.get_unchecked(idx as usize) }
    }

    /// Read-only view of the 8-entry lookup table.
    pub fn table(&self) -> &[u8; 8] {
        &self.table
    }
}

/// An owning simulation runner. Kept for callers who want a single struct
/// that holds the inputs, owns the result, and exposes a `.rows()` view.
///
/// Most library callers will use [`run`](crate::run) with
/// [`OutputKind::Structured`](crate::OutputKind::Structured) instead, which
/// returns a [`SimulationResult`](crate::SimulationResult) with a flat
/// row-major buffer (cheaper to allocate and easier to hand off to SIMD/FFI
/// code).
#[derive(Debug, Clone)]
pub struct CellularAutomaton {
    pub rule: Rule,
    pub width: usize,
    pub generations: usize,
    pub initial: Vec<u8>,
    pub boundary: BoundaryMode,
    rows: Vec<Vec<u8>>,
}

impl CellularAutomaton {
    pub fn new(
        rule: Rule,
        width: usize,
        generations: usize,
        initial: Vec<u8>,
        boundary: BoundaryMode,
    ) -> Self {
        CellularAutomaton {
            rule,
            width,
            generations,
            initial,
            boundary,
            rows: Vec::new(),
        }
    }

    /// Returns the simulated rows. Empty until [`run`](Self::run) has been
    /// called.
    pub fn rows(&self) -> &[Vec<u8>] {
        &self.rows
    }

    /// Run the simulation to completion, filling `rows`.
    pub fn run(&mut self) {
        let width = self.width;
        if width == 0 {
            self.rows = Vec::new();
            return;
        }

        let row0 = self.initial.clone();
        let total_rows = self.generations + 1;
        let mut rows: Vec<Vec<u8>> = Vec::with_capacity(total_rows);
        rows.push(row0);

        for g in 0..self.generations {
            let next = step_row(&self.rule, &rows[g], self.boundary, 0);
            rows.push(next);
        }

        self.rows = rows;
    }
}

/// Build an initial row of length `width` from a short seed pattern, Padded
/// with `fill` on whichever side(s) `align` dictates.
///
/// Every byte in the result is `0` or `1`. Bytes in `seed` other than `0` are
/// treated as `1`. If `seed.len() > width` the trailing seed bytes are
/// silently dropped; library callers go through the validated
/// [`InitialRow::FromSeed`](crate::InitialRow::FromSeed) path which rejects
/// that case explicitly.
pub fn make_initial_row(width: usize, seed: &[u8], align: PaddingAlign, fill: u8) -> Vec<u8> {
    let mut row = vec![fill & 1; width];
    if width == 0 || seed.is_empty() {
        return row;
    }
    let len = seed.len().min(width);
    // When seed overflows width, Before right-truncates (keep last `len` cells so the
    // seed's value stays right-aligned); After and Center left-truncate.
    let seed_skip = match align {
        PaddingAlign::Before => seed.len().saturating_sub(width),
        PaddingAlign::After | PaddingAlign::Center => 0,
    };
    let start = match align {
        PaddingAlign::Before => width - len,
        PaddingAlign::After => 0,
        PaddingAlign::Center => (width.saturating_sub(len)) / 2,
    };
    for (i, v) in seed.iter().skip(seed_skip).take(len).enumerate() {
        row[start + i] = if *v != 0 { 1 } else { 0 };
    }
    row
}

/// Validate that `row` is a legal explicit initial row for a configuration
/// of the given `width`. Used by [`run`](crate::run) when the caller hands
/// in an [`InitialRow::Explicit`](crate::InitialRow::Explicit) value.
///
/// Returns the offending position and value if any cell is not `0` or `1`,
/// or `Err` describing the mismatch if `row.len() != width`.
pub fn check_explicit_row(width: usize, row: &[u8]) -> Result<(), ExplicitRowError> {
    if row.len() != width {
        return Err(ExplicitRowError::LengthMismatch {
            expected: width,
            got: row.len(),
        });
    }
    for (i, &v) in row.iter().enumerate() {
        if v > 1 {
            return Err(ExplicitRowError::NonBinary { position: i, value: v });
        }
    }
    Ok(())
}

/// Reasons [`check_explicit_row`] rejects an initial row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplicitRowError {
    LengthMismatch { expected: usize, got: usize },
    NonBinary { position: usize, value: u8 },
}

/// # Advanced
///
/// Write the next generation row into `next`. `prev` and `next` must have the
/// same length; this function does not allocate and does not use threads.
///
/// The previous per-row `rayon::into_par_iter` version was disastrously
/// expensive for long runs: with width ~1000 and millions of generations, the
/// thread-dispatch overhead per row dwarfed the actual compute and prevented
/// any other thread (including the UI) from getting CPU time. The sequential
/// in-place implementation is the regression gate — see
/// `sim::tests::bench_2m_x_1000`.
pub fn step_row_into(rule: &Rule, prev: &[u8], next: &mut [u8], boundary: BoundaryMode, padded: u8) {
    let width = prev.len();
    debug_assert_eq!(width, next.len());
    if width == 0 {
        return;
    }
    let fill = padded & 1;
    // Shared interior: cell i (1..width-1) sees the window prev[i-1..=i+1].
    // Iterating `windows(3)` zipped with `next[1..]` keeps the inner loop
    // free of per-cell bounds checks, unlike indexed `prev[i - 1]` access.
    let step_interior = |next: &mut [u8]| {
        for (n, w) in next[1..width - 1].iter_mut().zip(prev.windows(3)) {
            *n = rule.apply(w[0], w[1], w[2]);
        }
    };
    match boundary {
        BoundaryMode::Padded => {
            if width == 1 {
                next[0] = rule.apply(fill, prev[0], fill);
            } else {
                // Peel first and last so the inner loop has no branch per cell.
                next[0] = rule.apply(fill, prev[0], prev[1]);
                step_interior(next);
                next[width - 1] = rule.apply(prev[width - 2], prev[width - 1], fill);
            }
        }
        BoundaryMode::Wrap => {
            if width == 1 {
                let c = prev[0];
                next[0] = rule.apply(c, c, c);
                return;
            }
            next[0] = rule.apply(prev[width - 1], prev[0], prev[1]);
            step_interior(next);
            next[width - 1] = rule.apply(prev[width - 2], prev[width - 1], prev[0]);
        }
    }
}

/// Allocating variant of [`step_row_into`]: returns a freshly allocated next
/// row instead of writing into a caller-owned buffer.
pub fn step_row(rule: &Rule, prev: &[u8], boundary: BoundaryMode, padded: u8) -> Vec<u8> {
    let mut next = vec![0u8; prev.len()];
    step_row_into(rule, prev, &mut next, boundary, padded);
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_30_table() {
        let r = Rule::new(30);
        assert_eq!(r.apply(1, 1, 1), 0);
        assert_eq!(r.apply(1, 1, 0), 0);
        assert_eq!(r.apply(1, 0, 1), 0);
        assert_eq!(r.apply(1, 0, 0), 1);
        assert_eq!(r.apply(0, 1, 1), 1);
        assert_eq!(r.apply(0, 1, 0), 1);
        assert_eq!(r.apply(0, 0, 1), 1);
        assert_eq!(r.apply(0, 0, 0), 0);
    }

    /// Headless smoke benchmark for the sequential simulation path.
    /// Runs the same inner loop the GUI worker uses, just with timing. Handy
    /// for verifying the right binary is being built and how fast rule 30
    /// really goes at width 1000. Invoke with:
    ///   cargo test --release sim::tests::bench_2m_x_1000 -- --nocapture --ignored
    #[test]
    #[ignore]
    fn bench_2m_x_1000() {
        let width: usize = 1000;
        let generations: usize = 2_000_000;
        let rule = Rule::new(30);

        let mut row0 = vec![0u8; width];
        row0[width / 2] = 1;

        let total_cells = (generations + 1) * width;
        let mut flat: Vec<u8> = Vec::with_capacity(total_cells);
        flat.extend_from_slice(&row0);

        let mut prev = row0;
        let mut next = vec![0u8; width];

        let start = std::time::Instant::now();
        for _ in 0..generations {
            step_row_into(&rule, &prev, &mut next, BoundaryMode::Wrap, 0);
            flat.extend_from_slice(&next);
            std::mem::swap(&mut prev, &mut next);
        }
        let elapsed = start.elapsed();

        let gens_per_sec = generations as f64 / elapsed.as_secs_f64();
        eprintln!(
            "bench_2m_x_1000: {} gens x {} width in {:.2?} ({:.0} gens/s, {:.1} Mcells/s, flat={} MiB)",
            generations,
            width,
            elapsed,
            gens_per_sec,
            (generations as f64 * width as f64) / elapsed.as_secs_f64() / 1e6,
            flat.len() / (1024 * 1024),
        );
        assert_eq!(flat.len(), total_cells);
    }

    #[test]
    fn rows_have_fixed_width() {
        let mut ca = CellularAutomaton::new(
            Rule::new(30),
            21,
            5,
            {
                let mut v = vec![0u8; 21];
                v[10] = 1;
                v
            },
            BoundaryMode::Padded,
        );
        ca.run();
        assert_eq!(ca.rows().len(), 6);
        for row in ca.rows() {
            assert_eq!(row.len(), 21);
        }
    }

    #[test]
    fn check_explicit_row_accepts_binary_with_matching_length() {
        assert!(check_explicit_row(4, &[0, 1, 1, 0]).is_ok());
    }

    #[test]
    fn check_explicit_row_rejects_length_mismatch() {
        let err = check_explicit_row(4, &[0, 1, 1]).unwrap_err();
        assert!(matches!(err, ExplicitRowError::LengthMismatch { expected: 4, got: 3 }));
    }

    #[test]
    fn check_explicit_row_rejects_non_binary() {
        let err = check_explicit_row(3, &[0, 2, 1]).unwrap_err();
        assert!(matches!(err, ExplicitRowError::NonBinary { position: 1, value: 2 }));
    }
}
