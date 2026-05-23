//! Self-contained SVG generation for a simulation result.
//!
//! The SVG is laid out as one [`RenderOptions::cell_size`]-pixel square per
//! cell. To keep the file size tractable for long runs, consecutive black
//! cells in the same row are coalesced into a single `<rect>` (horizontal
//! run-length encoding). White cells are represented implicitly by the white
//! background, so a typical rule-30 output produces an order of magnitude
//! fewer rects than `width * generations`.
//!
//! A hairline (0.1-unit) black outer frame is always drawn around the
//! image, regardless of [`RenderOptions::show_borders`], so a figure
//! dropped into LaTeX always shows a visible outline without being a
//! visually heavy black border. The width of the *inter-cell* grid is
//! controlled separately by [`RenderOptions::border_width`].
//!
//! Output is plain ASCII SVG with no embedded scripts or external references
//! — ready to drop into LaTeX, Inkscape, or a static web page.
//!
//! [`RenderOptions::cell_size`]: crate::RenderOptions::cell_size
//! [`RenderOptions::show_borders`]: crate::RenderOptions::show_borders
//! [`RenderOptions::border_width`]: crate::RenderOptions::border_width

use std::fmt::Write as _;

use crate::{RenderOptions, SimulationResult};

/// Above this many cells we print a one-line warning to stderr. The SVG is
/// still produced — the warning just nudges the caller toward
/// [`OutputKind::Structured`](crate::OutputKind::Structured) for very large
/// runs.
const LARGE_SVG_THRESHOLD: u128 = 50_000_000;

/// Convert a simulation result to a self-contained SVG document.
///
/// `opts.cell_size` controls the size of each cell in user-space units (1 =
/// 1 SVG pixel). `opts.show_borders` draws a grid between cells when
/// `cell_size > 1`; with `cell_size == 1` borders would overwhelm the
/// content so the flag is ignored. `opts.border_width` controls the
/// thickness of that grid in pixels and is clamped to `cell_size - 1`.
///
/// A fixed 0.1-unit outer frame is always drawn around the image so the
/// figure has a visible outline by default without being a heavy border.
pub fn to_svg(result: &SimulationResult, opts: &RenderOptions) -> String {
    let width = result.width;
    let height = if width == 0 { 0 } else { result.rows.len() / width };
    let cs = opts.cell_size.max(1) as usize;
    let total_w = width * cs;
    let total_h = height * cs;

    let cell_count = (width as u128) * (height as u128);
    if cell_count > LARGE_SVG_THRESHOLD {
        eprintln!(
            "warning: SVG output covers {} cells; many editors will struggle to open it. \
             Consider OutputKind::Structured for analysis at this scale.",
            cell_count
        );
    }

    let with_borders = opts.show_borders && cs > 1;
    // Inter-cell border width in user-space units. Fractional values are
    // allowed; clamped to `[0.0, cs]` so we don't emit negative cell
    // widths. Values at or beyond cs hide the cell entirely.
    let bw: f32 = if with_borders {
        opts.border_width.max(0.0).min(cs as f32)
    } else {
        0.0
    };
    let bg = if with_borders { "#4d4d4d" } else { "#ffffff" };

    // Integer-aligned borders look best with `crispEdges`; fractional
    // borders need `auto` so the renderer anti-aliases instead of
    // snapping to whole pixels.
    let shape_rendering = if bw.fract() == 0.0 { "crispEdges" } else { "auto" };

    let mut s = String::with_capacity(256 + cell_count.min(8_000_000) as usize / 4);
    let _ = writeln!(s, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    let _ = writeln!(
        s,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" \
         viewBox=\"0 0 {} {}\" shape-rendering=\"{}\">",
        total_w, total_h, total_w, total_h, shape_rendering
    );
    let _ = writeln!(
        s,
        "<rect width=\"{}\" height=\"{}\" fill=\"{}\"/>",
        total_w, total_h, bg
    );

    if width == 0 || height == 0 {
        s.push_str("</svg>\n");
        return s;
    }

    if with_borders {
        // Cells start at offset `bw` from the top/left of their `cs x cs`
        // box, with size `cs - bw`. The dark background shows through the
        // remaining `bw` units as the grid line — exactly matching the
        // HTML export's borders-on rendering. Fractional `bw` produces
        // anti-aliased edges in the renderer.
        //
        // Hot path: use push_str + push_usize / push_f32_compact instead of
        // writeln! to avoid format-string machinery overhead per cell.
        let cs_f = cs as f32;
        let inset = (cs_f - bw).max(0.0);

        if bw.fract() == 0.0 {
            // All coordinates are integers — use the fast integer path.
            let bw_int = bw as usize;
            let inset_int = inset as usize;
            for y in 0..height {
                let row = &result.rows[y * width..(y + 1) * width];
                let y_px = y * cs + bw_int;
                for x in 0..width {
                    let color = if row[x] == 1 { "#000000" } else { "#ffffff" };
                    s.push_str("<rect x=\"");
                    push_usize(&mut s, x * cs + bw_int);
                    s.push_str("\" y=\"");
                    push_usize(&mut s, y_px);
                    s.push_str("\" width=\"");
                    push_usize(&mut s, inset_int);
                    s.push_str("\" height=\"");
                    push_usize(&mut s, inset_int);
                    s.push_str("\" fill=\"");
                    s.push_str(color);
                    s.push_str("\"/>\n");
                }
            }
        } else {
            // Fractional border: precompute constant strings; vary only x/y per cell.
            let mut inset_str = String::new();
            push_f32_compact(&mut inset_str, inset);
            let mut scratch = String::with_capacity(16);
            for y in 0..height {
                let row = &result.rows[y * width..(y + 1) * width];
                scratch.clear();
                push_f32_compact(&mut scratch, y as f32 * cs_f + bw);
                let y_str: &str = unsafe {
                    // SAFETY: `scratch` contains only ASCII digits and '.'/'-',
                    // all valid UTF-8. We take a reference that lives for the
                    // inner loop which does not modify `scratch`.
                    std::str::from_utf8_unchecked(scratch.as_bytes())
                };
                for x in 0..width {
                    let color = if row[x] == 1 { "#000000" } else { "#ffffff" };
                    s.push_str("<rect x=\"");
                    push_f32_compact(&mut s, x as f32 * cs_f + bw);
                    s.push_str("\" y=\"");
                    s.push_str(y_str);
                    s.push_str("\" width=\"");
                    s.push_str(&inset_str);
                    s.push_str("\" height=\"");
                    s.push_str(&inset_str);
                    s.push_str("\" fill=\"");
                    s.push_str(color);
                    s.push_str("\"/>\n");
                }
            }
        }
    } else {
        // White background already painted; emit one <rect> per maximal
        // horizontal run of black cells. Default SVG fill is black, so we
        // can omit the `fill` attribute for an even smaller file.
        //
        // The hot loop here can dominate runtime for large outputs (one
        // pass over every cell), so we sidestep the `write!`/`Formatter`
        // machinery and assemble the rects with `push_str` + a direct
        // integer-to-decimal helper. On rule-30 at width 1000 × 5000
        // this measurably reduces SVG generation time in release builds.
        for y in 0..height {
            let row = &result.rows[y * width..(y + 1) * width];
            let y_px = y * cs;
            let mut x = 0;
            while x < width {
                if row[x] == 1 {
                    let start = x;
                    while x < width && row[x] == 1 {
                        x += 1;
                    }
                    let run_len = x - start;
                    s.push_str("<rect x=\"");
                    push_usize(&mut s, start * cs);
                    s.push_str("\" y=\"");
                    push_usize(&mut s, y_px);
                    s.push_str("\" width=\"");
                    push_usize(&mut s, run_len * cs);
                    s.push_str("\" height=\"");
                    push_usize(&mut s, cs);
                    s.push_str("\"/>\n");
                } else {
                    x += 1;
                }
            }
        }
    }

    // Outer frame, always. Fixed at 0.1 units wide — a hairline that sits
    // entirely inside the viewBox (0.05 offset + (total - 0.1) size). The
    // explicit `shape-rendering="geometricPrecision"` on this element
    // overrides any parent `crispEdges` hint so the sub-pixel stroke
    // actually renders.
    const OUTER_FRAME_W: f32 = 0.1;
    if total_w > 0 && total_h > 0 {
        let half = OUTER_FRAME_W / 2.0;
        let _ = writeln!(
            s,
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" \
             fill=\"none\" stroke=\"#000000\" stroke-width=\"{}\" \
             shape-rendering=\"geometricPrecision\"/>",
            half,
            half,
            total_w as f32 - OUTER_FRAME_W,
            total_h as f32 - OUTER_FRAME_W,
            OUTER_FRAME_W
        );
    }

    s.push_str("</svg>\n");
    s
}

/// Push `v` to `s` as a compact decimal string (no trailing zeros after the decimal point).
/// Rounds to 2 decimal places; suitable for SVG coordinate attributes.
/// Always non-negative in our use case (x/y/width/height are never negative).
#[inline]
fn push_f32_compact(s: &mut String, v: f32) {
    if v.fract() == 0.0 {
        push_usize(s, v as usize);
    } else {
        let scaled = (v * 100.0).round() as u64;
        let int_part = (scaled / 100) as usize;
        let frac = (scaled % 100) as usize;
        push_usize(s, int_part);
        s.push('.');
        if frac % 10 == 0 {
            push_usize(s, frac / 10); // "50" → "5", "30" → "3"
        } else {
            if frac < 10 { s.push('0'); }
            push_usize(s, frac);
        }
    }
}

/// Push `n` to `s` as decimal digits without going through `fmt::Display`.
/// Around 5-10x faster than `write!(s, "{}", n)` on hot integer-only
/// formatting paths; the result is identical.
#[inline]
fn push_usize(s: &mut String, mut n: usize) {
    if n == 0 {
        s.push('0');
        return;
    }
    // usize::MAX on 64-bit is 20 digits.
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    // SAFETY: every byte we wrote is an ASCII digit (`b'0'..=b'9'`),
    // which is valid UTF-8. `i` is in-bounds because we only ever
    // decremented after writing.
    s.push_str(unsafe { std::str::from_utf8_unchecked(&buf[i..]) });
}

#[cfg(test)]
mod tests {
    use super::push_usize;

    #[test]
    fn push_usize_matches_display() {
        for n in [0usize, 1, 9, 10, 99, 100, 12345, 987654321, usize::MAX] {
            let mut got = String::new();
            push_usize(&mut got, n);
            assert_eq!(got, n.to_string(), "value {n}");
        }
    }
}
