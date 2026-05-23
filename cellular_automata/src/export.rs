use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

use crate::sim::{BoundaryMode, PaddingAlign, PaddingFill};
use crate::{RenderOptions, SimulationResult};

fn is_safe_filename(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
}

/// Process-wide lock around the manifest append + index regeneration phase.
/// `run(..., OutputKind::Html { .. })` is documented as reentrant, but
/// `regenerate_index` is a read-then-write that races with concurrent
/// writes; serializing the gallery-update phase keeps `index.html` and
/// `manifest.tsv` consistent without blocking the simulation work that
/// runs before this point.
static MANIFEST_LOCK: Mutex<()> = Mutex::new(());

// To keep exported HTML viewable for very large runs, the embedded preview is
// split into stacked canvases so each stays within safe dimensions. These caps
// bound the per-canvas size; stride stays at 1 (full fidelity).
const MAX_EXPORT_CELLS: usize = 200_000_000; // per-canvas cap, ~25 MB packed (~34 MB base64)
const MAX_EXPORT_HEIGHT: usize = 32_000; // per-canvas height cap

const MANIFEST_FILE: &str = "manifest.tsv";
const INDEX_FILE: &str = "index.html";

pub(crate) struct ExportInput<'a> {
    pub job_id: u64,
    pub rule: u8,
    pub width: usize,
    pub generations: usize,
    pub progress: usize,
    pub boundary: BoundaryMode,
    pub status: &'a str,
    /// Flat row-major grid: cell (y, x) lives at `rows[y * width + x]`.
    pub rows: &'a [u8],
    pub show_borders: bool,
    /// Initial inter-cell border width baked into the exported page.
    /// Fractional values are allowed; the embedded JS clamps the result
    /// to `cs` on every render so it stays valid as the user changes the
    /// cell size with the slider.
    pub border_width: f32,
    pub padding_fill: PaddingFill,
    pub padding_align: PaddingAlign,
}

pub(crate) fn export_job(input: &ExportInput, dir: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;

    let now = chrono::Local::now();
    let ts_file = now.format("%Y%m%d_%H%M%S").to_string();
    let ts_human = now.format("%Y-%m-%d %H:%M:%S").to_string();

    let boundary_short = match input.boundary {
        BoundaryMode::ZeroPadded => "zp",
        BoundaryMode::Wrap => "wr",
    };

    let filename = format!(
        "rule{:03}_w{}_g{}_{}_{}_job{}.html",
        input.rule, input.width, input.generations, boundary_short, ts_file, input.job_id
    );
    let path = dir.join(&filename);

    let ic = if input.width == 0 || input.rows.is_empty() {
        "0".to_string()
    } else {
        compute_ic(&input.rows[0..input.width])
    };

    let html = render_run_html(input, &ts_human, &ic);
    fs::write(&path, html)?;

    // Serialise the manifest+index phase so concurrent calls into the
    // same directory (allowed by the public API contract) can't tear
    // either file. A poisoned mutex here only happens if another thread
    // panicked mid-write; we recover and keep going.
    let _guard = MANIFEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    append_manifest(dir, input, &ts_human, &filename, &ic)?;
    regenerate_index(dir)?;

    Ok(path)
}

fn render_run_html(input: &ExportInput, exported_at: &str, ic: &str) -> String {
    let rows = input.rows;
    let width = input.width;
    let height = if width == 0 { 0 } else { rows.len() / width };
    // Clamp border_width to a finite value before embedding in JSON.
    // f32::INFINITY or NaN would format as "inf"/"NaN", both invalid JSON.
    let safe_bw: f32 = if input.border_width.is_finite() {
        input.border_width.max(0.0)
    } else {
        0.0
    };
    let fill_str = fill_label(input.padding_fill);
    let align_str = align_label(input.padding_align);
    // Full-fidelity HTML preview: slice into stacked canvases so each stays
    // within safe dimensions while retaining stride == 1.
    // Export cell size: scale so the canvas lands near 1600px wide, matching
    // the in-app default view. Capped at 16 to prevent absurdly large canvases
    // for very narrow automata. cs=1 means no grid lines (large automata).
    let cs = (1600usize / width.max(1)).clamp(1, 16);

    let tile_max_rows_by_cells = if width == 0 {
        1
    } else {
        (MAX_EXPORT_CELLS / width).max(1)
    };
    // When cs > 1 each row is cs pixels tall, so cap tile height in rows
    // so the canvas height stays within MAX_EXPORT_HEIGHT pixels.
    let tile_max_rows = tile_max_rows_by_cells
        .min(MAX_EXPORT_HEIGHT / cs.max(1))
        .max(1);
    let num_tiles = if height == 0 {
        0
    } else {
        (height + tile_max_rows - 1) / tile_max_rows
    };

    let mut tile_meta_entries: Vec<String> = Vec::with_capacity(num_tiles);
    let mut tile_scripts = String::new();
    for tile_idx in 0..num_tiles {
        let start = tile_idx * tile_max_rows;
        let end = (start + tile_max_rows).min(height);
        let packed = pack_bits_range(rows, width, start, end);
        let b64 = STANDARD.encode(&packed);
        let id = format!("bits{}", tile_idx);
        tile_meta_entries.push(format!(
            "{{\"id\":\"{}\",\"start\":{},\"end\":{}}}",
            id, start, end
        ));
        tile_scripts.push_str(&format!(
            "<script id=\"{}\" type=\"application/octet-stream\">{}</script>\n",
            id, b64
        ));
    }
    let tiles_meta_json = tile_meta_entries.join(",");

    let title = format!(
        "Rule {} - {}x{} - job {}",
        input.rule, width, height, input.job_id
    );

    let mut html = String::with_capacity(tile_scripts.len() + 4096);
    html.push_str(&format!(
        "<!doctype html><html lang=\"en\"><head>\n\
         <meta charset=\"utf-8\">\n\
         <title>{}</title>\n\
         <style>\n\
         body {{ font-family: system-ui, sans-serif; margin: 20px; background: #fafafa; color: #222; }}\n\
         h1 {{ font-size: 18px; margin: 4px 0 12px 0; }}\n\
         dl {{ display: grid; grid-template-columns: auto 1fr; gap: 2px 12px; font-size: 13px; max-width: 420px; margin: 0 0 12px 0; }}\n\
         dt {{ color: #666; }}\n\
         canvas {{ image-rendering: pixelated; image-rendering: crisp-edges; border: 1px solid #ccc; display: block; margin-top: 12px; background: white; width: 100%; max-width: 1600px; height: auto; }}\n\
         a.back {{ font-size: 13px; color: #06c; text-decoration: none; }}\n\
         a.back:hover {{ text-decoration: underline; }}\n\
         </style>\n\
         </head><body>\n\
         <p><a class=\"back\" href=\"index.html\">&lt; back to index</a></p>\n\
         <h1>{}</h1>\n\
         <dl>\n\
           <dt>job id</dt><dd>{}</dd>\n\
           <dt>rule</dt><dd>{}</dd>\n\
           <dt>width</dt><dd>{}</dd>\n\
           <dt>generations</dt><dd>{} / {}</dd>\n\
           <dt>boundary</dt><dd>{}</dd>\n\
           <dt>status</dt><dd>{}</dd>\n\
           <dt>exported</dt><dd>{}</dd>\n\
           <dt>preview</dt><dd>{}</dd>\n\
         </dl>\n\
         <div style=\"margin:8px 0 4px 0;display:flex;align-items:center;gap:16px;font-size:13px;\">\n\
           <label style=\"display:flex;align-items:center;gap:6px;\">Cell size\n\
             <input id=\"cs-range\" type=\"range\" min=\"1\" max=\"16\" style=\"width:120px;vertical-align:middle;\">\n\
             <input id=\"cs-num\" type=\"number\" min=\"1\" max=\"64\" style=\"width:44px;padding:2px 4px;font-size:13px;\">px\n\
           </label>\n\
           <label style=\"display:flex;align-items:center;gap:4px;\"><input id=\"b-check\" type=\"checkbox\"> Borders</label>\n\
         </div>\n\
         <div id=\"tile-note\" style=\"font-size:12px;color:#555;margin-top:2px;\"></div>\n\
         <div id=\"tiles\"></div>\n\
         <script id=\"meta\" type=\"application/json\">{{\"w\":{},\"h\":{},\"cs\":{},\"borders\":{},\"bw\":{},\"tiles\":[{}],\"fill\":\"{}\",\"align\":\"{}\",\"ic\":\"{}\"}}</script>\n\
         {}",
        html_escape(&title),
        html_escape(&title),
        input.job_id,
        input.rule,
        input.width,
        input.progress,
        input.generations,
        // Defence in depth: escape every interpolated string even if today's
        // Display impls and library callers only produce safe constants.
        html_escape(&input.boundary.to_string()),
        html_escape(input.status),
        html_escape(exported_at),
        if num_tiles <= 1 {
            "full fidelity (single canvas)".to_string()
        } else {
            format!(
                "full fidelity: {} tiles ({} rows each, last shorter)",
                num_tiles, tile_max_rows
            )
        },
        width.max(1),
        height.max(1),
        cs,
        input.show_borders,
        safe_bw,
        tiles_meta_json,
        fill_str,
        align_str,
        ic,
        tile_scripts,
    ));
    html.push_str(
        "<script>\n\
        (function(){\n\
          const meta = JSON.parse(document.getElementById('meta').textContent);\n\
          if (!meta || meta.w === 0 || meta.h === 0) return;\n\
          const host   = document.getElementById('tiles');\n\
          const noteEl = document.getElementById('tile-note');\n\
          const tiles  = (meta.tiles || []).map(t => {\n\
            const b64 = (document.getElementById(t.id)?.textContent || '').trim();\n\
            if (!b64) return null;\n\
            return { bin: atob(b64), tileH: Math.max(0, (t.end||0)-(t.start||0)) };\n\
          }).filter(t => t && t.tileH > 0);\n\
          let curCs      = meta.cs || 1;\n\
          let curBorders = !!meta.borders;\n\
          function render(cs, borders) {\n\
            host.innerHTML = '';\n\
            let first = true;\n\
            for (const { bin, tileH } of tiles) {\n\
              const cv  = document.createElement('canvas');\n\
              cv.width  = meta.w * cs;\n\
              cv.height = tileH  * cs;\n\
              cv.style.display   = 'block';\n\
              cv.style.marginTop = first ? '12px' : '0';\n\
              cv.style.border    = 'none';\n\
              first = false;\n\
              const ctx = cv.getContext('2d');\n\
              if (cs === 1) {\n\
                cv.style.imageRendering = 'pixelated';\n\
                cv.style.imageRendering = 'crisp-edges';\n\
                const img = ctx.createImageData(meta.w, tileH);\n\
                for (let y = 0; y < tileH; y++) {\n\
                  for (let x = 0; x < meta.w; x++) {\n\
                    const i = y * meta.w + x;\n\
                    const bit = (bin.charCodeAt(i >> 3) >> (7 - (i & 7))) & 1;\n\
                    const p = i * 4, v = bit ? 0 : 255;\n\
                    img.data[p]=v; img.data[p+1]=v; img.data[p+2]=v; img.data[p+3]=255;\n\
                  }\n\
                }\n\
                ctx.putImageData(img, 0, 0);\n\
              } else {\n\
                cv.style.width    = cv.width  + 'px';\n\
                cv.style.height   = cv.height + 'px';\n\
                cv.style.maxWidth = 'none';\n\
                if (borders) {\n\
                  const rawBw = (typeof meta.bw === 'number') ? meta.bw : 1;\n\
                  const bw = Math.max(0, Math.min(rawBw, cs));\n\
                  const cellInset = Math.max(0, cs - bw);\n\
                  ctx.fillStyle = '#4d4d4d';\n\
                  ctx.fillRect(0, 0, cv.width, cv.height);\n\
                  for (let y = 0; y < tileH; y++) {\n\
                    for (let x = 0; x < meta.w; x++) {\n\
                      const i = y * meta.w + x;\n\
                      const bit = (bin.charCodeAt(i >> 3) >> (7 - (i & 7))) & 1;\n\
                      ctx.fillStyle = bit ? '#000000' : '#ffffff';\n\
                      ctx.fillRect(x*cs+bw, y*cs+bw, cellInset, cellInset);\n\
                    }\n\
                  }\n\
                } else {\n\
                  ctx.fillStyle = '#ffffff';\n\
                  ctx.fillRect(0, 0, cv.width, cv.height);\n\
                  ctx.fillStyle = '#000000';\n\
                  for (let y = 0; y < tileH; y++) {\n\
                    for (let x = 0; x < meta.w; x++) {\n\
                      const i = y * meta.w + x;\n\
                      const bit = (bin.charCodeAt(i >> 3) >> (7 - (i & 7))) & 1;\n\
                      if (bit) ctx.fillRect(x*cs, y*cs, cs, cs);\n\
                    }\n\
                  }\n\
                }\n\
              }\n\
              host.appendChild(cv);\n\
            }\n\
            noteEl.textContent = tiles.length > 1\n\
              ? 'Rendered ' + tiles.length + ' stacked canvases (full fidelity, stride 1).'\n\
              : '';\n\
          }\n\
          const csRange = document.getElementById('cs-range');\n\
          const csNum   = document.getElementById('cs-num');\n\
          const bCheck  = document.getElementById('b-check');\n\
          csRange.value  = curCs;\n\
          csNum.value    = curCs;\n\
          bCheck.checked = curBorders;\n\
          function applyCs(v) {\n\
            curCs = Math.max(1, parseInt(v) || 1);\n\
            csRange.value = Math.min(curCs, 16);\n\
            csNum.value   = curCs;\n\
            render(curCs, curBorders);\n\
          }\n\
          csRange.addEventListener('input',  () => applyCs(csRange.value));\n\
          csNum.addEventListener('change',   () => applyCs(csNum.value));\n\
          bCheck.addEventListener('change',  () => { curBorders = bCheck.checked; render(curCs, curBorders); });\n\
          render(curCs, curBorders);\n\
        })();\n\
        </script>\n\
        </body></html>\n",
    );

    html
}

fn pack_bits_range(rows: &[u8], width: usize, start_row: usize, end_row: usize) -> Vec<u8> {
    if width == 0 {
        return Vec::new();
    }
    let total_rows = rows.len().checked_div(width).unwrap_or(0);
    if start_row >= end_row || start_row >= total_rows {
        return Vec::new();
    }
    let clamped_end = end_row.min(total_rows);
    let rendered_h = clamped_end.saturating_sub(start_row);
    let total_bits = width.saturating_mul(rendered_h);
    let out_len = total_bits.div_ceil(8);
    let mut out = vec![0u8; out_len];

    // The naive version does one branch + shift + write per cell. We can
    // do quite a lot better by accumulating 8 cells into a register and
    // writing one byte at a time whenever the destination is
    // byte-aligned; this is the common case in practice because most rows
    // produce a stream of contiguous bits starting at the tile's first
    // byte. Misaligned spans (e.g. when `width % 8 != 0`) fall back to
    // the bit-at-a-time path, which is still safe and correct.
    let mut bit_cursor: usize = 0;
    for rendered_y in 0..rendered_h {
        let orig_y = start_row + rendered_y;
        let row = &rows[orig_y * width..(orig_y + 1) * width];
        let mut x = 0;

        // Byte-aligned 8-cell stride: turns each group of 8 cells into a
        // single OR-and-store, eliminating the per-cell branch and shift.
        while x + 8 <= width && bit_cursor & 7 == 0 {
            let byte_val: u8 = ((row[x] & 1) << 7)
                | ((row[x + 1] & 1) << 6)
                | ((row[x + 2] & 1) << 5)
                | ((row[x + 3] & 1) << 4)
                | ((row[x + 4] & 1) << 3)
                | ((row[x + 5] & 1) << 2)
                | ((row[x + 6] & 1) << 1)
                | (row[x + 7] & 1);
            out[bit_cursor >> 3] = byte_val;
            bit_cursor += 8;
            x += 8;
        }

        // Tail / misaligned cells: bit-at-a-time fallback.
        while x < width {
            if row[x] & 1 == 1 {
                let byte = bit_cursor >> 3;
                let shift = 7 - (bit_cursor & 7);
                out[byte] |= 1 << shift;
            }
            bit_cursor += 1;
            x += 1;
        }
    }

    out
}

fn append_manifest(
    dir: &Path,
    input: &ExportInput,
    timestamp: &str,
    filename: &str,
    ic: &str,
) -> io::Result<()> {
    let path = dir.join(MANIFEST_FILE);
    let exists = path.exists();
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;

    if !exists {
        writeln!(
            f,
            "id\trule\twidth\tgenerations\tboundary\tstatus\tprogress\ttimestamp\tfilename\tfill\talign\tic"
        )?;
    }

    // Sanitize any value that might contain a tab or newline; the TSV
    // format has no escaping and a stray separator would shift every
    // subsequent column. Library callers only produce safe constants
    // today, but defensive sanitisation keeps the file parseable if a
    // future caller passes something unusual.
    let safe_status = sanitize_tsv(input.status);
    let safe_filename = sanitize_tsv(filename);
    let safe_timestamp = sanitize_tsv(timestamp);
    let safe_boundary = sanitize_tsv(&input.boundary.to_string());
    writeln!(
        f,
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        input.job_id,
        input.rule,
        input.width,
        input.generations,
        safe_boundary,
        safe_status,
        input.progress,
        safe_timestamp,
        safe_filename,
        fill_label(input.padding_fill),
        align_label(input.padding_align),
        sanitize_tsv(ic),
    )?;

    Ok(())
}

fn sanitize_tsv(s: &str) -> String {
    s.chars()
        .map(|c| if c == '\t' || c == '\n' || c == '\r' { ' ' } else { c })
        .collect()
}

fn fill_label(f: PaddingFill) -> &'static str {
    match f { PaddingFill::Zero => "0", PaddingFill::One => "1" }
}

fn align_label(a: PaddingAlign) -> &'static str {
    match a {
        PaddingAlign::Before => "before",
        PaddingAlign::After  => "after",
        PaddingAlign::Center => "centered",
    }
}

/// Converts the initial row to a decimal string.
/// Finds the first and last `1`, slices that range, interprets as
/// big-endian binary via pure-Rust string arithmetic, returns "0" if
/// no `1` is present.
fn compute_ic(row: &[u8]) -> String {
    let first = row.iter().position(|&b| b & 1 == 1);
    let last  = row.iter().rposition(|&b| b & 1 == 1);
    let (first, last) = match (first, last) {
        (Some(f), Some(l)) => (f, l),
        _ => return "0".to_string(),
    };
    let bits = &row[first..=last];
    let mut digits: Vec<u8> = vec![0]; // little-endian decimal digits
    for &bit in bits {
        let mut carry = 0u8;
        for d in digits.iter_mut() {
            let v = *d * 2 + carry;
            *d = v % 10;
            carry = v / 10;
        }
        if carry > 0 { digits.push(carry); }
        if bit & 1 == 1 {
            let mut carry = 1u8;
            for d in digits.iter_mut() {
                let v = *d + carry;
                *d = v % 10;
                carry = v / 10;
            }
            if carry > 0 { digits.push(carry); }
        }
    }
    digits.iter().rev().map(|&d| (b'0' + d) as char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Naive bit-packing reference; mirrors the pre-optimisation
    /// implementation so we can confirm the fast path produces the same
    /// byte stream the embedded JS already knows how to decode.
    fn pack_bits_reference(rows: &[u8], width: usize, start: usize, end: usize) -> Vec<u8> {
        if width == 0 || start >= end {
            return Vec::new();
        }
        let total_rows = rows.len() / width;
        let end = end.min(total_rows);
        if start >= end {
            return Vec::new();
        }
        let h = end - start;
        let total_bits = width * h;
        let mut out = vec![0u8; total_bits.div_ceil(8)];
        for y in 0..h {
            for x in 0..width {
                if rows[(start + y) * width + x] & 1 == 1 {
                    let idx = y * width + x;
                    out[idx >> 3] |= 1 << (7 - (idx & 7));
                }
            }
        }
        out
    }

    #[test]
    fn pack_bits_range_matches_reference_aligned_widths() {
        for &width in &[1usize, 2, 7, 8, 9, 15, 16, 31, 32, 33, 64, 1000] {
            let h = 5;
            let mut rows = vec![0u8; width * h];
            for (i, b) in rows.iter_mut().enumerate() {
                *b = ((i * 37) % 3 == 0) as u8;
            }
            let got = pack_bits_range(&rows, width, 0, h);
            let want = pack_bits_reference(&rows, width, 0, h);
            assert_eq!(got, want, "width={width}");
        }
    }

    #[test]
    fn pack_bits_range_handles_partial_tile() {
        let width = 13;
        let h = 4;
        let rows: Vec<u8> = (0..(width * h) as u8).map(|n| n & 1).collect();
        let got = pack_bits_range(&rows, width, 1, 3);
        let want = pack_bits_reference(&rows, width, 1, 3);
        assert_eq!(got, want);
    }

    #[test]
    fn sanitize_tsv_strips_tabs_and_newlines() {
        assert_eq!(sanitize_tsv("a\tb\nc\rd"), "a b c d");
        assert_eq!(sanitize_tsv("normal text"), "normal text");
    }

    #[test]
    fn compute_ic_all_zero_returns_zero() {
        assert_eq!(compute_ic(&[0, 0, 0, 0, 0]), "0");
        assert_eq!(compute_ic(&[]), "0");
    }

    #[test]
    fn compute_ic_single_one() {
        // A single 1 anywhere → decimal 1.
        assert_eq!(compute_ic(&[0, 0, 1, 0, 0]), "1");
        assert_eq!(compute_ic(&[1]), "1");
    }

    #[test]
    fn compute_ic_binary_to_decimal() {
        // [1,1] = 0b11 = 3
        assert_eq!(compute_ic(&[0, 1, 1, 0]), "3");
        // [1,0,1,1] = 0b1011 = 11
        assert_eq!(compute_ic(&[1, 0, 1, 1]), "11");
        // seed "11" → bits are [1,1] → decimal 3
        assert_eq!(compute_ic(&[0, 0, 1, 1, 0, 0]), "3");
    }

    #[test]
    fn compute_ic_padded_seed_strips_leading_trailing_zeros() {
        // [0,0,1,0,1,0,0] → slice [1,0,1] = 0b101 = 5
        assert_eq!(compute_ic(&[0, 0, 1, 0, 1, 0, 0]), "5");
    }

    #[test]
    fn compute_ic_large_value() {
        // 8-bit 1s = 0xFF = 255
        let row = vec![1u8; 8];
        assert_eq!(compute_ic(&row), "255");
        // Trailing zeros are stripped: [1,0,0,0,0,0,0,0] → slice=[1] → decimal 1.
        let r2 = vec![1u8, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(compute_ic(&r2), "1");
        // [1,0,0,0,0,0,0,1] → slice all 8 bits → 0b10000001 = 129
        let r3 = vec![1u8, 0, 0, 0, 0, 0, 0, 1];
        assert_eq!(compute_ic(&r3), "129");
    }
}

struct ManifestEntry {
    id: String,
    rule: String,
    width: String,
    generations: String,
    boundary: String,
    status: String,
    progress: String,
    timestamp: String,
    filename: String,
    fill: String,
    align: String,
    ic: String,
}

fn read_manifest(dir: &Path) -> io::Result<Vec<ManifestEntry>> {
    let path = dir.join(MANIFEST_FILE);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let f = File::open(&path)?;
    let reader = BufReader::new(f);
    let mut entries = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        if i == 0 || line.trim().is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 9 {
            continue;
        }
        entries.push(ManifestEntry {
            id: parts[0].to_string(),
            rule: parts[1].to_string(),
            width: parts[2].to_string(),
            generations: parts[3].to_string(),
            boundary: parts[4].to_string(),
            status: parts[5].to_string(),
            progress: parts[6].to_string(),
            timestamp: parts[7].to_string(),
            filename: parts[8].to_string(),
            fill:  parts.get(9).copied().unwrap_or("").to_string(),
            align: parts.get(10).copied().unwrap_or("").to_string(),
            ic:    parts.get(11).copied().unwrap_or("").to_string(),
        });
    }
    Ok(entries)
}

fn json_str_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"'  => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // HTML-safe escapes: prevent `</script>` or `<!--` from breaking
            // out of the enclosing <script type="application/json"> element.
            '<'  => out.push_str("\\u003c"),
            '>'  => out.push_str("\\u003e"),
            '/'  => out.push_str("\\u002f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

// Gallery: IBM Plex typography, warm-paper palette, grid-push sidebar (default closed),
// ascending rule / IC / gens sort, separate gallery.js written alongside index.html.
const GALLERY_HTML_HEAD: &str = include_str!("gallery.html.template");
const GALLERY_HTML_TAIL: &str = "\n</script>\n<script src=\"gallery.js\"></script>\n</body></html>\n";
const GALLERY_JS:        &str = include_str!("gallery.js");

// (old SPA_HEAD / SPA_SCRIPT inline constants deleted; see GALLERY_HTML_HEAD / GALLERY_JS above)
#[allow(dead_code)]
const _DELETED_SPA_HEAD: &str = concat!(
    "<!doctype html><html lang=\"en\"><head>\n",
    "<meta charset=\"utf-8\">\n",
    "<title>Cellular Automata runs</title>\n",
    "<style>\n",
    "html,body{margin:0;padding:0;height:100%;}\n",
    "body{font-family:system-ui,sans-serif;background:#fafafa;color:#222;overflow-x:hidden;}\n",
    "#burger{position:fixed;top:10px;left:10px;z-index:200;font-size:18px;",
    "background:#fff;border:1px solid #bbb;border-radius:4px;padding:3px 8px;cursor:pointer;line-height:1;}\n",
    "#sidebar{position:fixed;top:0;left:-292px;width:280px;height:100vh;",
    "background:#fff;border-right:1px solid #ddd;z-index:100;overflow-y:auto;",
    "transition:left 0.2s ease;display:flex;flex-direction:column;}\n",
    "body.sidebar-open #sidebar{left:0;}\n",
    "#search{margin:50px 8px 8px;padding:6px 8px;font-size:13px;",
    "border:1px solid #ccc;border-radius:3px;box-sizing:border-box;width:calc(100% - 16px);}\n",
    "#run-list details summary{cursor:pointer;padding:6px 10px;font-size:13px;font-weight:600;",
    "background:#f5f5f5;border-bottom:1px solid #e0e0e0;user-select:none;list-style:none;}\n",
    "#run-list details summary::-webkit-details-marker{display:none;}\n",
    "#run-list details summary:hover{background:#eee;}\n",
    ".run-entry{padding:5px 10px 5px 16px;font-size:12px;cursor:pointer;",
    "border-bottom:1px solid #f0f0f0;line-height:1.4;}\n",
    ".run-entry:hover{background:#f0f4ff;}\n",
    ".run-entry.active{background:#dde8ff;font-weight:500;}\n",
    "#main{position:fixed;top:0;left:0;right:0;bottom:0;overflow-y:auto;",
    "padding:14px 16px 16px 52px;box-sizing:border-box;}\n",
    "#canvas-meta{display:grid;grid-template-columns:auto 1fr;gap:2px 12px;",
    "font-size:13px;max-width:480px;margin:0 0 10px 0;}\n",
    "#canvas-meta dt{color:#666;}\n",
    "#controls{margin:8px 0 4px 0;display:flex;align-items:center;gap:16px;font-size:13px;}\n",
    "#tile-note{font-size:12px;color:#555;margin-top:2px;}\n",
    "canvas{image-rendering:pixelated;image-rendering:crisp-edges;",
    "border:1px solid #ccc;display:block;margin-top:12px;background:white;",
    "width:100%;max-width:1600px;height:auto;}\n",
    "#empty-msg{color:#888;padding:60px 0 0 0;font-size:14px;}\n",
    "</style>\n",
    "</head><body>\n",
    "<button id=\"burger\" title=\"Toggle sidebar\">&#9776;</button>\n",
    "<div id=\"sidebar\">\n",
    "  <input id=\"search\" placeholder=\"Filter runs\u{2026}\" autocomplete=\"off\">\n",
    "  <div id=\"run-list\"></div>\n",
    "</div>\n",
    "<div id=\"main\">\n",
    "  <div id=\"empty-msg\" style=\"display:none\">No runs exported yet.</div>\n",
    "  <dl id=\"canvas-meta\" style=\"display:none\"></dl>\n",
    "  <div id=\"controls\" style=\"display:none\">\n",
    "    <label style=\"display:flex;align-items:center;gap:6px;\">Cell size\n",
    "      <input id=\"cs-range\" type=\"range\" min=\"1\" max=\"16\" style=\"width:120px;vertical-align:middle;\">\n",
    "      <input id=\"cs-num\" type=\"number\" min=\"1\" max=\"64\" style=\"width:44px;padding:2px 4px;font-size:13px;\">px\n",
    "    </label>\n",
    "    <label style=\"display:flex;align-items:center;gap:4px;\"><input id=\"b-check\" type=\"checkbox\"> Borders</label>\n",
    "  </div>\n",
    "  <div id=\"tile-note\"></div>\n",
    "  <div id=\"tiles\"></div>\n",
    "</div>\n"
);

// (old inline SPA script — superseded by gallery.js; kept for reference only)
#[allow(dead_code)]
const SPA_SCRIPT: &str = concat!(
    "<script>\n",
    "(function(){\n",
    "  var curCs=4,curBorders=false,curMeta=null,curTiles=[],activeEl=null;\n",
    "  var burger=document.getElementById('burger');\n",
    "  var searchEl=document.getElementById('search');\n",
    "  var runList=document.getElementById('run-list');\n",
    "  var metaDl=document.getElementById('canvas-meta');\n",
    "  var ctrlEl=document.getElementById('controls');\n",
    "  var tilesEl=document.getElementById('tiles');\n",
    "  var noteEl=document.getElementById('tile-note');\n",
    "  var emptyMsg=document.getElementById('empty-msg');\n",
    "  var csRange=document.getElementById('cs-range');\n",
    "  var csNum=document.getElementById('cs-num');\n",
    "  var bCheck=document.getElementById('b-check');\n",
    "  function esc(s){return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/\"/g,'&quot;');}\n",
    "  burger.addEventListener('click',function(){document.body.classList.toggle('sidebar-open');});\n",
    "  function displayName(e){\n",
    "    if(!e.fill&&!e.align&&!e.ic) return e.filename;\n",
    "    var bnd=(e.boundary==='Wrap-around')?'wrapped around ':'';\n",
    "    return (e.generations+' generations of rule '+e.rule+' '+bnd+'padded '+e.fill+' '+e.align+' '+e.ic).replace(/\\s+/g,' ').trim();\n",
    "  }\n",
    "  function buildSidebar(entries){\n",
    "    var groups=new Map();\n",
    "    for(var i=0;i<entries.length;i++){\n",
    "      var e=entries[i]; var r=parseInt(e.rule,10);\n",
    "      if(!groups.has(r)) groups.set(r,[]);\n",
    "      groups.get(r).push(e);\n",
    "    }\n",
    "    var rules=Array.from(groups.keys()).sort(function(a,b){return b-a;});\n",
    "    runList.innerHTML='';\n",
    "    for(var ri=0;ri<rules.length;ri++){\n",
    "      var rule=rules[ri],ruleEntries=groups.get(rule);\n",
    "      var details=document.createElement('details');\n",
    "      details.open=true;\n",
    "      var summary=document.createElement('summary');\n",
    "      summary.textContent='Rule '+rule;\n",
    "      details.appendChild(summary);\n",
    "      for(var ei=0;ei<ruleEntries.length;ei++){\n",
    "        (function(e){\n",
    "          var div=document.createElement('div');\n",
    "          div.className='run-entry';\n",
    "          div.setAttribute('data-filename',e.filename);\n",
    "          div.textContent=displayName(e);\n",
    "          div.addEventListener('click',function(){loadRun(e,div);});\n",
    "          details.appendChild(div);\n",
    "        })(ruleEntries[ei]);\n",
    "      }\n",
    "      runList.appendChild(details);\n",
    "    }\n",
    "  }\n",
    "  searchEl.addEventListener('input',function(){\n",
    "    var term=searchEl.value.toLowerCase();\n",
    "    var dets=runList.querySelectorAll('details');\n",
    "    for(var i=0;i<dets.length;i++){\n",
    "      var divs=dets[i].querySelectorAll('.run-entry'),any=false;\n",
    "      for(var j=0;j<divs.length;j++){\n",
    "        var match=!term||divs[j].textContent.toLowerCase().includes(term);\n",
    "        divs[j].style.display=match?'':'none';\n",
    "        if(match) any=true;\n",
    "      }\n",
    "      dets[i].style.display=any?'':'none';\n",
    "    }\n",
    "  });\n",
    "  function renderCanvas(){\n",
    "    tilesEl.innerHTML=''; noteEl.textContent='';\n",
    "    if(!curMeta||curMeta.w===0) return;\n",
    "    var cs=curCs,borders=curBorders,meta=curMeta,tiles=curTiles,first=true;\n",
    "    for(var ti=0;ti<tiles.length;ti++){\n",
    "      var bin=tiles[ti].bin,tileH=tiles[ti].tileH;\n",
    "      var cv=document.createElement('canvas');\n",
    "      cv.width=meta.w*cs; cv.height=tileH*cs;\n",
    "      cv.style.display='block'; cv.style.marginTop=first?'12px':'0'; cv.style.border='none';\n",
    "      first=false;\n",
    "      var ctx=cv.getContext('2d');\n",
    "      if(cs===1){\n",
    "        cv.style.imageRendering='pixelated';\n",
    "        cv.style.imageRendering='crisp-edges';\n",
    "        var img=ctx.createImageData(meta.w,tileH);\n",
    "        for(var y=0;y<tileH;y++) for(var x=0;x<meta.w;x++){\n",
    "          var i=y*meta.w+x;\n",
    "          var bit=(bin.charCodeAt(i>>3)>>(7-(i&7)))&1;\n",
    "          var p=i*4,v=bit?0:255;\n",
    "          img.data[p]=v;img.data[p+1]=v;img.data[p+2]=v;img.data[p+3]=255;\n",
    "        }\n",
    "        ctx.putImageData(img,0,0);\n",
    "      } else {\n",
    "        cv.style.width=cv.width+'px'; cv.style.height=cv.height+'px'; cv.style.maxWidth='none';\n",
    "        if(borders){\n",
    "          var rawBw=(typeof meta.bw==='number')?meta.bw:1;\n",
    "          var bw=Math.max(0,Math.min(rawBw,cs)),ci=Math.max(0,cs-bw);\n",
    "          ctx.fillStyle='#4d4d4d'; ctx.fillRect(0,0,cv.width,cv.height);\n",
    "          for(var y=0;y<tileH;y++) for(var x=0;x<meta.w;x++){\n",
    "            var i=y*meta.w+x;\n",
    "            var bit=(bin.charCodeAt(i>>3)>>(7-(i&7)))&1;\n",
    "            ctx.fillStyle=bit?'#000000':'#ffffff';\n",
    "            ctx.fillRect(x*cs+bw,y*cs+bw,ci,ci);\n",
    "          }\n",
    "        } else {\n",
    "          ctx.fillStyle='#ffffff'; ctx.fillRect(0,0,cv.width,cv.height);\n",
    "          ctx.fillStyle='#000000';\n",
    "          for(var y=0;y<tileH;y++) for(var x=0;x<meta.w;x++){\n",
    "            var i=y*meta.w+x;\n",
    "            var bit=(bin.charCodeAt(i>>3)>>(7-(i&7)))&1;\n",
    "            if(bit) ctx.fillRect(x*cs,y*cs,cs,cs);\n",
    "          }\n",
    "        }\n",
    "      }\n",
    "      tilesEl.appendChild(cv);\n",
    "    }\n",
    "    noteEl.textContent=tiles.length>1?'Rendered '+tiles.length+' stacked canvases (full fidelity, stride 1).':'';\n",
    "  }\n",
    "  function applyCs(v){curCs=Math.max(1,parseInt(v)||1);csRange.value=Math.min(curCs,16);csNum.value=curCs;renderCanvas();}\n",
    "  csRange.addEventListener('input',function(){applyCs(csRange.value);});\n",
    "  csNum.addEventListener('change',function(){applyCs(csNum.value);});\n",
    "  bCheck.addEventListener('change',function(){curBorders=bCheck.checked;renderCanvas();});\n",
    "  function loadRun(e,entryEl){\n",
    "    if(activeEl) activeEl.classList.remove('active');\n",
    "    activeEl=entryEl; entryEl.classList.add('active');\n",
    "    document.body.classList.remove('sidebar-open');\n",
    "    tilesEl.innerHTML=''; noteEl.textContent='';\n",
    "    fetch(e.filename).then(function(r){if(!r.ok) throw new Error(); return r.text();})\n",
    "    .then(function(html){\n",
    "      var doc=new DOMParser().parseFromString(html,'text/html');\n",
    "      var meta=JSON.parse(doc.getElementById('meta').textContent);\n",
    "      var tiles=(meta.tiles||[]).map(function(t){\n",
    "        var el=doc.getElementById(t.id);\n",
    "        return {bin:atob((el?el.textContent:'').trim()),tileH:Math.max(0,(t.end||0)-(t.start||0))};\n",
    "      }).filter(function(t){return t.tileH>0;});\n",
    "      curMeta=meta; curTiles=tiles;\n",
    "      curCs=meta.cs||4; curBorders=!!meta.borders;\n",
    "      csRange.value=Math.min(curCs,16); csNum.value=curCs; bCheck.checked=curBorders;\n",
    "      var icLabel=(e.fill&&e.align&&e.ic)?('padded '+e.fill+' '+e.align+' '+e.ic):e.filename;\n",
    "      metaDl.innerHTML='<dt>rule</dt><dd>'+esc(e.rule)+'</dd>'\n",
    "        +'<dt>generations</dt><dd>'+esc(e.progress)+' / '+esc(e.generations)+'</dd>'\n",
    "        +'<dt>boundary</dt><dd>'+esc(e.boundary)+'</dd>'\n",
    "        +'<dt>initial condition</dt><dd>'+esc(icLabel)+'</dd>'\n",
    "        +'<dt>exported</dt><dd>'+esc(e.timestamp)+'</dd>';\n",
    "      emptyMsg.style.display='none';\n",
    "      metaDl.style.display='grid'; ctrlEl.style.display='';\n",
    "      renderCanvas();\n",
    "    }).catch(function(){\n",
    "      var isFile=location.protocol==='file:';\n",
    "      var msg=isFile\n",
    "        ?'Chrome blocks cross-file access over <code>file://</code>. Use Firefox, or serve locally:<br><code>python -m http.server</code>'\n",
    "        :'Could not load <code>'+esc(e.filename)+'</code>.';\n",
    "      tilesEl.innerHTML='<p style=\"color:#c00;margin:20px 0;font-size:13px\">'+msg+'</p>'\n",
    "        +'<p><a href=\"'+esc(e.filename)+'\" style=\"color:#06c;font-size:13px\">Open '+esc(e.filename)+' directly &#8594;</a></p>';\n",
    "    });\n",
    "  }\n",
    "  function parseTsv(text){\n",
    "    var lines=text.trim().split('\\n'),out=[];\n",
    "    for(var i=1;i<lines.length;i++){\n",
    "      var p=lines[i].split('\\t');\n",
    "      if(p.length<9) continue;\n",
    "      out.push({id:p[0],rule:p[1],width:p[2],generations:p[3],\n",
    "                boundary:p[4],status:p[5],progress:p[6],timestamp:p[7],filename:p[8],\n",
    "                fill:p[9]||'',align:p[10]||'',ic:p[11]||''});\n",
    "    }\n",
    "    out.reverse();\n",
    "    return out;\n",
    "  }\n",
    "  function autoLoadFromHash(){\n",
    "    var hash=decodeURIComponent(location.hash.slice(1));\n",
    "    if(!hash) return;\n",
    "    var div=runList.querySelector('[data-filename=\"'+hash.replace(/\"/g,'\\\\\"')+'\"]');\n",
    "    if(div) div.click();\n",
    "  }\n",
    "  function init(){\n",
    "    fetch('manifest.tsv').then(function(r){if(!r.ok) throw new Error(); return r.text();})\n",
    "    .then(function(text){\n",
    "      var entries=parseTsv(text);\n",
    "      if(entries.length===0){emptyMsg.style.display=''; return;}\n",
    "      buildSidebar(entries); autoLoadFromHash();\n",
    "    }).catch(function(){\n",
    "      var entries=JSON.parse(document.getElementById('baked').textContent||'[]');\n",
    "      if(entries.length===0){emptyMsg.style.display=''; return;}\n",
    "      buildSidebar(entries); autoLoadFromHash();\n",
    "    });\n",
    "  }\n",
    "  init();\n",
    "})();\n",
    "</script>\n",
    "</body></html>\n"
);

fn regenerate_index(dir: &Path) -> io::Result<()> {
    let entries = read_manifest(dir)?;
    let valid: Vec<&ManifestEntry> = entries.iter()
        .filter(|e| dir.join(&e.filename).exists())
        .collect();

    // Build baked JSON array for file:// fallback (gallery.js boot() reads this).
    let mut baked = String::from("[");
    let mut first = true;
    for e in valid.iter() {
        if !first { baked.push(','); }
        first = false;
        baked.push_str(&format!(
            "{{\"id\":\"{}\",\"rule\":\"{}\",\"width\":\"{}\",\"generations\":\"{}\",\
             \"boundary\":\"{}\",\"status\":\"{}\",\"progress\":\"{}\",\"timestamp\":\"{}\",\
             \"filename\":\"{}\",\"fill\":\"{}\",\"align\":\"{}\",\"ic\":\"{}\"}}",
            json_str_escape(&e.id),
            json_str_escape(&e.rule),
            json_str_escape(&e.width),
            json_str_escape(&e.generations),
            json_str_escape(&e.boundary),
            json_str_escape(&e.status),
            json_str_escape(&e.progress),
            json_str_escape(&e.timestamp),
            json_str_escape(&e.filename),
            json_str_escape(&e.fill),
            json_str_escape(&e.align),
            json_str_escape(&e.ic),
        ));
    }
    baked.push(']');

    // Write gallery.js next to index.html so the <script src="gallery.js"> tag resolves.
    // Only write if absent — contents are compile-time constants, so they never change.
    let gallery_js_path = dir.join("gallery.js");
    if !gallery_js_path.exists() {
        fs::write(&gallery_js_path, GALLERY_JS)?;
    }

    // index.html = HTML head+body template  +  baked JSON  +  closing tags + script ref
    let html = format!("{}{}{}", GALLERY_HTML_HEAD, baked, GALLERY_HTML_TAIL);
    fs::write(dir.join(INDEX_FILE), html)?;
    Ok(())
}

static NEXT_SAVE_JOB_ID: AtomicU64 = AtomicU64::new(1);

/// Save a simulation result to `dir` only if no identical run already exists in the manifest.
///
/// "Identical" means same rule, width, generations, boundary, padding fill, padding align,
/// and initial-condition decimal string. The dedup check and the manifest write happen under
/// the same [`MANIFEST_LOCK`] so there is no TOCTOU window between concurrent saves.
///
/// Returns `Err` with kind [`io::ErrorKind::AlreadyExists`] if a duplicate is found; the error
/// message starts with `"duplicate:"` followed by the existing filename so callers can surface
/// a 409 response without a separate manifest read.
pub fn save_result_if_unique(
    result: &SimulationResult,
    opts: &RenderOptions,
    dir: &Path,
    fill: PaddingFill,
    align: PaddingAlign,
) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;

    let ic = if result.width == 0 || result.rows.is_empty() {
        "0".to_string()
    } else {
        compute_ic(&result.rows[0..result.width])
    };
    let rule_str     = result.config.rule.to_string();
    let width_str    = result.width.to_string();
    let gens_str     = result.generations.to_string();
    let boundary_str = result.config.boundary.to_string();
    let fill_str     = fill_label(fill);
    let align_str    = align_label(align);

    let now = chrono::Local::now();
    let ts_file  = now.format("%Y%m%d_%H%M%S").to_string();
    let ts_human = now.format("%Y-%m-%d %H:%M:%S").to_string();
    let boundary_short = match result.config.boundary {
        BoundaryMode::ZeroPadded => "zp",
        BoundaryMode::Wrap => "wr",
    };
    let job_id = NEXT_SAVE_JOB_ID.fetch_add(1, Ordering::Relaxed);
    let filename = format!(
        "rule{:03}_w{}_g{}_{}_{}_job{}.html",
        result.config.rule, result.width, result.generations,
        boundary_short, ts_file, job_id
    );
    let path = dir.join(&filename);

    // Acquire the manifest lock before dedup check so the check + write are atomic.
    let _guard = MANIFEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    // Dedup check: reject if a run with identical parameters already exists.
    if let Ok(entries) = read_manifest(dir) {
        for e in &entries {
            if e.rule == rule_str
                && e.width == width_str
                && e.generations == gens_str
                && e.boundary == boundary_str
                && e.fill == fill_str
                && e.align == align_str
                && e.ic == ic
            {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("duplicate:{}", e.filename),
                ));
            }
        }
    }

    let input = ExportInput {
        job_id,
        rule: result.config.rule,
        width: result.width,
        generations: result.generations,
        progress: result.generations,
        boundary: result.config.boundary,
        status: "Done",
        rows: &result.rows,
        show_borders: opts.show_borders,
        border_width: opts.border_width,
        padding_fill: fill,
        padding_align: align,
    };

    let html = render_run_html(&input, &ts_human, &ic);
    fs::write(&path, html)?;
    append_manifest(dir, &input, &ts_human, &filename, &ic)?;
    regenerate_index(dir)?;

    Ok(path)
}

pub fn delete_saved_run(dir: &std::path::Path, filename: &str) -> io::Result<()> {
    if !is_safe_filename(filename) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "unsafe filename"));
    }
    let _guard = MANIFEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    if let Err(e) = fs::remove_file(dir.join(filename)) {
        if e.kind() != io::ErrorKind::NotFound {
            return Err(e);
        }
    }
    let manifest_path = dir.join(MANIFEST_FILE);
    if manifest_path.exists() {
        let entries = read_manifest(dir)?;
        let mut f = std::fs::OpenOptions::new()
            .write(true).create(true).truncate(true)
            .open(&manifest_path)?;
        writeln!(f, "id\trule\twidth\tgenerations\tboundary\tstatus\tprogress\ttimestamp\tfilename\tfill\talign\tic")?;
        for e in &entries {
            if e.filename != filename {
                writeln!(f, "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    e.id, e.rule, e.width, e.generations, e.boundary,
                    e.status, e.progress, e.timestamp, e.filename,
                    e.fill, e.align, e.ic)?;
            }
        }
    }
    regenerate_index(dir)
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

