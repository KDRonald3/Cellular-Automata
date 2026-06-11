use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

use crate::sim::{BoundaryMode, PaddingAlign, PaddingFill};

// To keep exported HTML viewable for very large runs, the embedded preview is
// split into stacked canvases so each stays within safe dimensions. These caps
// bound the per-canvas size; stride stays at 1 (full fidelity).
const MAX_EXPORT_CELLS: usize = 200_000_000; // per-canvas cap, ~25 MB packed (~34 MB base64)
const MAX_EXPORT_HEIGHT: usize = 32_000; // per-canvas height cap

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
    pub border_width: f32,
    pub padding_fill: PaddingFill,
    pub padding_align: PaddingAlign,
}

/// Write a stand-alone HTML file for one CA run. Does not touch the manifest
/// or gallery index — callers (e.g. the web server) handle those themselves.
pub(crate) fn export_job(input: &ExportInput, dir: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;

    let now = chrono::Local::now();
    let ts_file  = now.format("%Y%m%d_%H%M%S").to_string();
    let ts_human = now.format("%Y-%m-%d %H:%M:%S").to_string();

    let boundary_short = match input.boundary {
        BoundaryMode::ZeroPadded => "padded",
        BoundaryMode::Wrap => "wraparound",
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

    Ok(path)
}

fn render_run_html(input: &ExportInput, exported_at: &str, ic: &str) -> String {
    let rows = input.rows;
    let width = input.width;
    let height = rows.len().checked_div(width).unwrap_or(0);
    // Clamp border_width to a finite value before embedding in JSON.
    let safe_bw: f32 = if input.border_width.is_finite() {
        input.border_width.max(0.0)
    } else {
        0.0
    };
    let fill_str = fill_label(input.padding_fill);
    let align_str = align_label(input.padding_align);
    let cs = (1600usize / width.max(1)).clamp(1, 16);

    let tile_max_rows_by_cells = MAX_EXPORT_CELLS.checked_div(width).unwrap_or(1).max(1);
    let tile_max_rows = tile_max_rows_by_cells
        .min(MAX_EXPORT_HEIGHT / cs.max(1))
        .max(1);
    let num_tiles = height.div_ceil(tile_max_rows);

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

    let mut bit_cursor: usize = 0;
    for rendered_y in 0..rendered_h {
        let orig_y = start_row + rendered_y;
        let row = &rows[orig_y * width..(orig_y + 1) * width];
        let mut x = 0;

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
pub(crate) fn compute_ic(row: &[u8]) -> String {
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
    fn compute_ic_all_zero_returns_zero() {
        assert_eq!(compute_ic(&[0, 0, 0, 0, 0]), "0");
        assert_eq!(compute_ic(&[]), "0");
    }

    #[test]
    fn compute_ic_single_one() {
        assert_eq!(compute_ic(&[0, 0, 1, 0, 0]), "1");
        assert_eq!(compute_ic(&[1]), "1");
    }

    #[test]
    fn compute_ic_binary_to_decimal() {
        assert_eq!(compute_ic(&[0, 1, 1, 0]), "3");
        assert_eq!(compute_ic(&[1, 0, 1, 1]), "11");
        assert_eq!(compute_ic(&[0, 0, 1, 1, 0, 0]), "3");
    }

    #[test]
    fn compute_ic_padded_seed_strips_leading_trailing_zeros() {
        assert_eq!(compute_ic(&[0, 0, 1, 0, 1, 0, 0]), "5");
    }

    #[test]
    fn compute_ic_large_value() {
        let row = vec![1u8; 8];
        assert_eq!(compute_ic(&row), "255");
        let r2 = vec![1u8, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(compute_ic(&r2), "1");
        let r3 = vec![1u8, 0, 0, 0, 0, 0, 0, 1];
        assert_eq!(compute_ic(&r3), "129");
    }
}
