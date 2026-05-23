//! JSON serialization for a simulation result.
//!
//! The on-disk shape is a top-level object with `config` and `rows` fields.
//! `rows` is a 2D array (one inner array per generation), even though the
//! library's in-memory representation is a flat `Vec<u8>` — the conversion
//! lives in [`to_json`] so callers don't pay for it unless they ask for JSON.

use serde::Serialize;

use crate::{SimConfig, SimulationResult};

#[derive(Serialize)]
struct JsonView<'a> {
    config: &'a SimConfig,
    /// Row-major 2D view over the flat backing buffer. Each inner slice is
    /// `config.width` cells long.
    rows: Vec<&'a [u8]>,
}

/// Serialize a [`SimulationResult`] as pretty-printed JSON.
///
/// The output looks like:
///
/// ```json
/// {
///   "config": {
///     "rule": 30,
///     "width": 21,
///     "generations": 5,
///     "boundary": "Wrap"
///   },
///   "rows": [
///     [0, 0, 0, 1, 0, 0, 0],
///     [0, 0, 1, 1, 1, 0, 0]
///   ]
/// }
/// ```
///
/// For very large runs (a 1000-wide x 1M-row grid is roughly 2 GB of JSON)
/// prefer [`OutputKind::Structured`](crate::OutputKind::Structured) and
/// serialize a subset yourself.
pub fn to_json(result: &SimulationResult) -> String {
    let rows_2d: Vec<&[u8]> = if result.width == 0 {
        Vec::new()
    } else {
        result.rows.chunks(result.width).collect()
    };
    let view = JsonView {
        config: &result.config,
        rows: rows_2d,
    };
    // The shadow type's `Serialize` impl is total over any valid input, so
    // `serde_json::to_string_pretty` cannot fail here.
    serde_json::to_string_pretty(&view).expect("SimulationResult serialization is infallible")
}
