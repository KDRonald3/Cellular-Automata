use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write as IoWrite};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use axum::{
    Router,
    extract::{DefaultBodyLimit, Path as AxumPath, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json,
};
use tokio::sync::Semaphore;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use cellular_automata::{
    BoundaryMode, InitialRow, OutputKind, PaddingAlign, PaddingFill,
    RenderOptions, SimConfig, SimulationResult,
    run, save_result,
};
use serde::{Deserialize, Serialize};

// ── Constants ─────────────────────────────────────────────────────────────────

const MAX_EXPORT_CELLS: usize = 200_000_000;
const MAX_EXPORT_HEIGHT: usize = 32_000;
const MAX_SESSIONS: usize = 1_000;
const MAX_WIDTH: usize = 10_000;
const MAX_GENERATIONS: usize = 100_000;
const MAX_REQUEST_BYTES: usize = 64 * 1024;
/// Upper bound on cells retained across *all* in-memory session runs. A
/// single run can be up to 500 M cells, so MAX_SESSIONS alone would allow
/// hundreds of GB of retained grids; this caps the total instead.
const MAX_TOTAL_SESSION_CELLS: usize = 2_000_000_000;
/// Simulations are CPU-bound; without a cap each request gets its own
/// blocking thread (tokio allows hundreds), so a burst of large requests
/// can exhaust CPU and memory before the session cap is even checked.
const MAX_CONCURRENT_SIMULATIONS: usize = 4;

const MANIFEST_FILE: &str = "manifest.tsv";
const INDEX_FILE: &str = "index.html";
const GALLERY_HTML_HEAD: &str = include_str!("gallery.html.template");
const GALLERY_HTML_TAIL: &str = "\n</script>\n<script src=\"gallery.js\"></script>\n</body></html>\n";
const GALLERY_JS: &str = include_str!("gallery.js");

// ── Gallery / manifest state ──────────────────────────────────────────────────

/// Process-wide lock serialising manifest reads + writes so concurrent saves
/// and deletes don't tear `manifest.tsv` or `index.html`.
static MANIFEST_LOCK: Mutex<()> = Mutex::new(());
static NEXT_SAVE_JOB_ID: AtomicU64 = AtomicU64::new(1);

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
            id:         parts[0].to_string(),
            rule:       parts[1].to_string(),
            width:      parts[2].to_string(),
            generations: parts[3].to_string(),
            boundary:   parts[4].to_string(),
            status:     parts[5].to_string(),
            progress:   parts[6].to_string(),
            timestamp:  parts[7].to_string(),
            filename:   parts[8].to_string(),
            fill:  parts.get(9).copied().unwrap_or("").to_string(),
            align: parts.get(10).copied().unwrap_or("").to_string(),
            ic:    parts.get(11).copied().unwrap_or("").to_string(),
        });
    }
    Ok(entries)
}

fn append_manifest(
    dir: &Path,
    entry: &ManifestEntry,
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

    writeln!(
        f,
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        sanitize_tsv(&entry.id),
        sanitize_tsv(&entry.rule),
        sanitize_tsv(&entry.width),
        sanitize_tsv(&entry.generations),
        sanitize_tsv(&entry.boundary),
        sanitize_tsv(&entry.status),
        sanitize_tsv(&entry.progress),
        sanitize_tsv(&entry.timestamp),
        sanitize_tsv(&entry.filename),
        sanitize_tsv(&entry.fill),
        sanitize_tsv(&entry.align),
        sanitize_tsv(&entry.ic),
    )?;

    Ok(())
}

fn sanitize_tsv(s: &str) -> String {
    s.chars()
        .map(|c| if c == '\t' || c == '\n' || c == '\r' { ' ' } else { c })
        .collect()
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
            '<'  => out.push_str("\\u003c"),
            '>'  => out.push_str("\\u003e"),
            '/'  => out.push_str("\\u002f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn regenerate_index(dir: &Path) -> io::Result<()> {
    let entries = read_manifest(dir)?;
    let valid: Vec<&ManifestEntry> = entries.iter()
        .filter(|e| dir.join(&e.filename).exists())
        .collect();

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

    // Write gallery.js next to index.html so the <script src="gallery.js">
    // tag resolves when the gallery is opened from disk. Always overwrite so
    // the on-disk copy tracks the version compiled into this binary.
    fs::write(dir.join("gallery.js"), GALLERY_JS)?;

    let html = format!("{}{}{}", GALLERY_HTML_HEAD, baked, GALLERY_HTML_TAIL);
    fs::write(dir.join(INDEX_FILE), html)?;
    Ok(())
}

fn save_result_if_unique(
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
        compute_ic_with_fill(&result.rows[0..result.width], fill)
    };
    let rule_str     = result.config.rule.to_string();
    let width_str    = result.width.to_string();
    let gens_str     = result.generations.to_string();
    let boundary_str = result.config.boundary.to_string();
    let fill_str     = fill_label(fill);
    let align_str    = align_label(align);

    let _guard = MANIFEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());

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

    let path = save_result(result, opts, dir, fill, align)
        .map_err(|e| io::Error::other(e.to_string()))?;

    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let now = chrono::Local::now();
    let ts_human = now.format("%Y-%m-%d %H:%M:%S").to_string();
    let job_id = NEXT_SAVE_JOB_ID.fetch_add(1, Ordering::Relaxed);

    let entry = ManifestEntry {
        id:         job_id.to_string(),
        rule:       rule_str,
        width:      width_str,
        generations: gens_str,
        boundary:   boundary_str,
        status:     "Done".to_string(),
        progress:   result.generations.to_string(),
        timestamp:  ts_human,
        filename,
        fill:       fill_str.to_string(),
        align:      align_str.to_string(),
        ic,
    };
    append_manifest(dir, &entry)?;
    regenerate_index(dir)?;

    Ok(path)
}

fn delete_saved_run(dir: &Path, filename: &str) -> io::Result<()> {
    if !is_safe_filename(filename) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "unsafe filename"));
    }
    let _guard = MANIFEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    if let Err(e) = fs::remove_file(dir.join(filename))
        && e.kind() != io::ErrorKind::NotFound
    {
        return Err(e);
    }
    let manifest_path = dir.join(MANIFEST_FILE);
    if manifest_path.exists() {
        let entries = read_manifest(dir)?;
        let mut f = OpenOptions::new()
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

// ── Tile packing ──────────────────────────────────────────────────────────────

fn is_safe_filename(name: &str) -> bool {
    if name.is_empty()
        || name.len() > 200
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
        // Reject names with no alphanumeric at all — this excludes "." and
        // ".." (which `Path::join` would resolve to the runs dir or its
        // parent) without restricting any real exported filename.
        || !name.bytes().any(|b| b.is_ascii_alphanumeric())
    {
        return false;
    }
    // Windows maps reserved device names (optionally with an extension,
    // e.g. "CON.html") to devices; reading one can block the request task.
    let stem = name.split('.').next().unwrap_or(name);
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL",
        "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
        "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    !RESERVED.iter().any(|r| stem.eq_ignore_ascii_case(r))
}

/// True if a `Host` header names this machine. Requests from a hostile
/// website via DNS rebinding carry the attacker's hostname here, so anything
/// other than a loopback name is rejected before reaching a handler.
fn host_is_local(host: &str) -> bool {
    let name = if let Some(rest) = host.strip_prefix('[') {
        // IPv6 literal: "[::1]:3000" → "::1"
        match rest.split_once(']') {
            Some((ip, _)) => ip,
            None => return false,
        }
    } else {
        host.rsplit_once(':').map_or(host, |(n, _)| n)
    };
    name.eq_ignore_ascii_case("localhost") || name == "127.0.0.1" || name == "::1"
}

async fn require_local_host(req: Request, next: Next) -> Response {
    let host_ok = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .is_some_and(host_is_local);
    if !host_ok {
        return (StatusCode::FORBIDDEN, "invalid Host header").into_response();
    }
    next.run(req).await
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

/// IC for naming purposes, fill-aware: with a fill of 1 the row is mostly
/// ones, so the raw binary value says nothing about the seed. Bitwise-NOT
/// the row first so the IC describes the pattern against its background;
/// with a fill of 0 this is plain `compute_ic`.
fn compute_ic_with_fill(row: &[u8], fill: PaddingFill) -> String {
    match fill {
        PaddingFill::Zero => compute_ic(row),
        PaddingFill::One => {
            let notted: Vec<u8> = row.iter().map(|&b| (b & 1) ^ 1).collect();
            compute_ic(&notted)
        }
    }
}

fn compute_ic(row: &[u8]) -> String {
    let first = row.iter().position(|&b| b & 1 == 1);
    let last = row.iter().rposition(|&b| b & 1 == 1);
    let (first, last) = match (first, last) {
        (Some(f), Some(l)) => (f, l),
        _ => return "0".to_string(),
    };
    let bits = &row[first..=last];
    let mut digits: Vec<u8> = vec![0];
    for &bit in bits {
        let mut carry = 0u8;
        for d in digits.iter_mut() {
            let v = *d * 2 + carry;
            *d = v % 10;
            carry = v / 10;
        }
        if carry > 0 {
            digits.push(carry);
        }
        if bit & 1 == 1 {
            let mut carry = 1u8;
            for d in digits.iter_mut() {
                let v = *d + carry;
                *d = v % 10;
                carry = v / 10;
            }
            if carry > 0 {
                digits.push(carry);
            }
        }
    }
    digits.iter().rev().map(|&d| (b'0' + d) as char).collect()
}

// ── Session state ─────────────────────────────────────────────────────────────

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
struct SessionRun {
    result: Arc<SimulationResult>,
    render: RenderOptions,
    padding_fill: PaddingFill,
    padding_align: PaddingAlign,
}

#[derive(Clone)]
struct AppState {
    session: Arc<Mutex<HashMap<u64, SessionRun>>>,
    runs_dir: PathBuf,
    /// Limits concurrent CPU-bound simulations; excess requests queue.
    sim_permits: Arc<Semaphore>,
}

// ── Serde types ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct RunRequest {
    seed: String,
    rule: u8,
    generations: usize,
    width: usize,
    cell_size: u32,
    boundary: String,
    align: String,
    fill: String,
    show_borders: bool,
    border_width: Option<f32>,
}

#[derive(Serialize)]
struct TileMeta {
    id: String,
    start: usize,
    end: usize,
}

#[derive(Serialize)]
struct TilePayload {
    id: String,
    b64: String,
}

#[derive(Serialize)]
struct CanvasMeta {
    w: usize,
    h: usize,
    cs: usize,
    borders: bool,
    bw: f32,
    fill: String,
    align: String,
    ic: String,
    tiles: Vec<TileMeta>,
}

#[derive(Serialize)]
struct RunResponse {
    id: u64,
    rule: u8,
    width: usize,
    generations: usize,
    boundary: String,
    fill: String,
    align: String,
    ic: String,
    meta: CanvasMeta,
    tiles: Vec<TilePayload>,
}

#[derive(Serialize)]
struct SaveResponse {
    filename: String,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_boundary(s: &str) -> Option<BoundaryMode> {
    match s {
        "ZeroPadded" => Some(BoundaryMode::ZeroPadded),
        "Wrap" => Some(BoundaryMode::Wrap),
        _ => None,
    }
}

fn parse_align(s: &str) -> Option<PaddingAlign> {
    match s {
        "Before" => Some(PaddingAlign::Before),
        "After" => Some(PaddingAlign::After),
        "Center" => Some(PaddingAlign::Center),
        _ => None,
    }
}

fn parse_fill(s: &str) -> Option<PaddingFill> {
    match s {
        "Zero" => Some(PaddingFill::Zero),
        "One" => Some(PaddingFill::One),
        _ => None,
    }
}

fn fill_label(f: PaddingFill) -> &'static str {
    match f {
        PaddingFill::Zero => "0",
        PaddingFill::One => "1",
    }
}

fn align_label(a: PaddingAlign) -> &'static str {
    match a {
        PaddingAlign::Before => "before",
        PaddingAlign::After => "after",
        PaddingAlign::Center => "centered",
    }
}

fn boundary_label(b: BoundaryMode) -> &'static str {
    match b {
        BoundaryMode::ZeroPadded => "Padded",
        BoundaryMode::Wrap => "Wrap-around",
    }
}

fn compute_tile_data(
    result: &SimulationResult,
    render: &RenderOptions,
    padding_fill: PaddingFill,
    padding_align: PaddingAlign,
) -> (CanvasMeta, Vec<TilePayload>) {
    let width = result.width;
    let height = result.rows.len().checked_div(width).unwrap_or(0);
    // Honor the caller's requested cell size as the initial zoom; 0 means
    // "auto" (fit ~1600 px). The viewer can always re-zoom client-side.
    let cs = if render.cell_size == 0 {
        (1600usize.checked_div(width).unwrap_or(1)).clamp(1, 16)
    } else {
        (render.cell_size as usize).clamp(1, 64)
    };

    let tile_max_rows_by_cells = MAX_EXPORT_CELLS.checked_div(width).unwrap_or(1).max(1);
    let tile_max_rows = tile_max_rows_by_cells
        .min(MAX_EXPORT_HEIGHT / cs.max(1))
        .max(1);
    let num_tiles = height.div_ceil(tile_max_rows);

    let ic = if width == 0 || result.rows.is_empty() {
        "0".to_string()
    } else {
        compute_ic_with_fill(&result.rows[0..width], padding_fill)
    };

    let mut tile_metas = Vec::with_capacity(num_tiles);
    let mut tile_payloads = Vec::with_capacity(num_tiles);

    for tile_idx in 0..num_tiles {
        let start = tile_idx * tile_max_rows;
        let end = (start + tile_max_rows).min(height);
        let packed = pack_bits_range(&result.rows, width, start, end);
        let b64 = STANDARD.encode(&packed);
        let id = format!("tile{}", tile_idx);
        tile_metas.push(TileMeta { id: id.clone(), start, end });
        tile_payloads.push(TilePayload { id, b64 });
    }

    let meta = CanvasMeta {
        w: width.max(1),
        h: height.max(1),
        cs,
        borders: render.show_borders,
        bw: render.border_width.max(0.0),
        fill: fill_label(padding_fill).to_string(),
        align: align_label(padding_align).to_string(),
        ic,
        tiles: tile_metas,
    };

    (meta, tile_payloads)
}

fn unprocessable(msg: impl std::fmt::Display) -> Response {
    (StatusCode::UNPROCESSABLE_ENTITY, msg.to_string()).into_response()
}

/// Turn serde's deserialization message into something a person filling in
/// the form can act on. Falls back to the (prefix-stripped) original text
/// for shapes we don't recognise.
fn friendly_json_error(raw: &str) -> String {
    let msg = raw
        .strip_prefix("Failed to deserialize the JSON body into the target type: ")
        .unwrap_or(raw);
    let msg = match msg.rfind(" at line ") {
        Some(pos) => &msg[..pos],
        None => msg,
    };
    let msg = msg
        .replace("expected u8", "expected a whole number between 0 and 255")
        .replace("expected usize", "expected a non-negative whole number")
        .replace("expected u32", "expected a non-negative whole number")
        .replace("invalid type: null,", "the field is empty;");
    format!("Invalid input — {msg}")
}

// ── Route handlers ────────────────────────────────────────────────────────────

async fn get_index() -> impl IntoResponse {
    const HTML: &str = include_str!("web_ui.html");
    (
        [(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"))],
        HTML,
    )
}

async fn manifest_response(runs_dir: &Path) -> Response {
    let path = runs_dir.join(MANIFEST_FILE);
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => (
            [
                (header::CONTENT_TYPE, HeaderValue::from_static("text/tab-separated-values; charset=utf-8")),
                (header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")),
            ],
            content,
        ).into_response(),
        Err(_) => (StatusCode::OK, "").into_response(),
    }
}

async fn get_manifest(State(state): State<AppState>) -> Response {
    manifest_response(&state.runs_dir).await
}

async fn get_run_file(
    State(state): State<AppState>,
    AxumPath(filename): AxumPath<String>,
) -> Response {
    // The gallery index.html references these two by relative URL, so they
    // must resolve under /runs/ too. gallery.js is served from the embedded
    // constant so it can never go stale relative to this binary.
    if filename == "gallery.js" {
        return (
            [
                (header::CONTENT_TYPE, HeaderValue::from_static("text/javascript; charset=utf-8")),
                (header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")),
            ],
            GALLERY_JS,
        ).into_response();
    }
    if filename == MANIFEST_FILE {
        return manifest_response(&state.runs_dir).await;
    }
    if !is_safe_filename(&filename) || !filename.ends_with(".html") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let path = state.runs_dir.join(&filename);
    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8")),
                (header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")),
            ],
            bytes,
        ).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn post_run(
    State(state): State<AppState>,
    payload: Result<Json<RunRequest>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(j) => j,
        Err(rej) => return unprocessable(friendly_json_error(&rej.body_text())),
    };
    let boundary = match parse_boundary(&req.boundary) {
        Some(b) => b,
        None => return unprocessable(format!("unknown boundary {:?}", req.boundary)),
    };
    let align = match parse_align(&req.align) {
        Some(a) => a,
        None => return unprocessable(format!("unknown align {:?}", req.align)),
    };
    let fill = match parse_fill(&req.fill) {
        Some(f) => f,
        None => return unprocessable(format!("unknown fill {:?}", req.fill)),
    };

    if req.width == 0 || req.width > MAX_WIDTH {
        return unprocessable(format!("width must be between 1 and {MAX_WIDTH}"));
    }
    if req.generations > MAX_GENERATIONS {
        return unprocessable(format!("generations must be at most {MAX_GENERATIONS}"));
    }
    if req.width.saturating_mul(req.generations.saturating_add(1)) > 500_000_000 {
        return unprocessable("simulation too large: width × (generations + 1) exceeds 500 M cells");
    }

    let seed: Vec<u8> = req.seed
        .chars()
        .filter_map(|c| c.to_digit(10).map(|d| if d == 0 { 0u8 } else { 1u8 }))
        .collect();

    let config = SimConfig {
        rule: req.rule,
        width: req.width,
        generations: req.generations,
        boundary,
    };
    let initial = InitialRow::FromSeed { seed, align, fill };
    let render = RenderOptions {
        cell_size: req.cell_size,
        show_borders: req.show_borders,
        border_width: req.border_width.unwrap_or(1.0),
    };

    // Holding the permit across the blocking task bounds both CPU use and
    // the transient memory of simulations that are not yet session-tracked.
    let _permit = match state.sim_permits.clone().acquire_owned().await {
        Ok(p) => p,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    let result = match tokio::task::spawn_blocking(move || {
        run(config, initial, render, OutputKind::Structured)
    })
    .await
    {
        Ok(r) => r,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "simulation panicked").into_response(),
    };

    let result = match result {
        Ok(cellular_automata::Output::Structured(r)) => r,
        Err(e) => return unprocessable(e),
        _ => unreachable!(),
    };

    let (meta, tiles) = compute_tile_data(&result, &render, fill, align);
    let ic = meta.ic.clone();
    let boundary_str = boundary_label(boundary).to_string();
    let fill_str = fill_label(fill).to_string();
    let align_str = align_label(align).to_string();

    let id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);

    {
        let mut lock = state.session.lock().unwrap_or_else(|p| p.into_inner());
        if lock.len() >= MAX_SESSIONS {
            return (StatusCode::TOO_MANY_REQUESTS, "session limit reached").into_response();
        }
        let retained_cells: usize = lock.values().map(|s| s.result.rows.len()).sum();
        if retained_cells.saturating_add(result.rows.len()) > MAX_TOTAL_SESSION_CELLS {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                "session memory limit reached; remove some session runs and retry",
            )
                .into_response();
        }
        lock.insert(id, SessionRun {
            result: Arc::new(result),
            render,
            padding_fill: fill,
            padding_align: align,
        });
    }

    let response = RunResponse {
        id,
        rule: req.rule,
        width: req.width,
        generations: req.generations,
        boundary: boundary_str,
        fill: fill_str,
        align: align_str,
        ic,
        meta,
        tiles,
    };

    Json(response).into_response()
}

async fn post_save_run(
    State(state): State<AppState>,
    AxumPath(id_str): AxumPath<String>,
    headers: HeaderMap,
) -> Response {
    // A body-less POST is a CORS "simple request" that browsers send
    // cross-origin without preflight. Requiring a custom header forces a
    // preflight (which fails — no CORS layer), so only same-origin callers
    // and non-browser clients like curl can save.
    if !headers.contains_key("x-requested-with") {
        return (StatusCode::FORBIDDEN, "missing X-Requested-With header").into_response();
    }
    let id: u64 = match id_str.parse() {
        Ok(n) => n,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    let session_run = {
        let lock = state.session.lock().unwrap_or_else(|p| p.into_inner());
        lock.get(&id).cloned()
    };

    let sr = match session_run {
        Some(s) => s,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let runs_dir = state.runs_dir.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        save_result_if_unique(&sr.result, &sr.render, &runs_dir, sr.padding_fill, sr.padding_align)
    })
    .await;

    match outcome {
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        Ok(Err(e)) if e.kind() == io::ErrorKind::AlreadyExists => {
            let existing = e.to_string()
                .strip_prefix("duplicate:")
                .unwrap_or("")
                .to_string();
            (StatusCode::CONFLICT, Json(SaveResponse { filename: existing })).into_response()
        }
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        Ok(Ok(path)) => {
            let filename = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            Json(SaveResponse { filename }).into_response()
        }
    }
}

async fn delete_session_run(
    State(state): State<AppState>,
    AxumPath(id_str): AxumPath<String>,
) -> Response {
    let id: u64 = match id_str.parse() {
        Ok(n) => n,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let mut lock = state.session.lock().unwrap_or_else(|p| p.into_inner());
    if lock.remove(&id).is_some() {
        StatusCode::NO_CONTENT.into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

async fn delete_saved_file(
    State(state): State<AppState>,
    AxumPath(filename): AxumPath<String>,
) -> Response {
    if !is_safe_filename(&filename) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let runs_dir = state.runs_dir.clone();
    let result = match tokio::task::spawn_blocking(move || delete_saved_run(&runs_dir, &filename))
        .await
    {
        Ok(r) => r,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) if e.kind() == io::ErrorKind::InvalidInput => {
            StatusCode::BAD_REQUEST.into_response()
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            StatusCode::NOT_FOUND.into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn build_router(runs_dir: PathBuf) -> Router {
    let state = AppState {
        session: Arc::new(Mutex::new(HashMap::new())),
        runs_dir,
        sim_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_SIMULATIONS)),
    };
    Router::new()
        .route("/", get(get_index))
        .route("/manifest.tsv", get(get_manifest))
        .route("/runs/:filename", get(get_run_file))
        .route("/api/run", post(post_run))
        .route("/api/runs/:id/save", post(post_save_run))
        .route("/api/runs/:id", delete(delete_session_run))
        .route("/api/saved/:filename", delete(delete_saved_file))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .layer(middleware::from_fn(require_local_host))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_filename_accepts_exported_names() {
        assert!(is_safe_filename("rule030_w401_g100_padded_20260611_120000_job1.html"));
        assert!(is_safe_filename("a-b_c.1.html"));
    }

    #[test]
    fn safe_filename_rejects_traversal_and_separators() {
        assert!(!is_safe_filename(""));
        assert!(!is_safe_filename("."));
        assert!(!is_safe_filename(".."));
        assert!(!is_safe_filename("..."));
        assert!(!is_safe_filename("a/b.html"));
        assert!(!is_safe_filename("a\\b.html"));
        assert!(!is_safe_filename("..\\x.html"));
        assert!(!is_safe_filename(&"a".repeat(201)));
    }

    #[test]
    fn safe_filename_rejects_windows_device_names() {
        assert!(!is_safe_filename("CON"));
        assert!(!is_safe_filename("con.html"));
        assert!(!is_safe_filename("Nul.html"));
        assert!(!is_safe_filename("COM1.html"));
        assert!(!is_safe_filename("lpt9"));
        // Similar-looking but legal names stay allowed.
        assert!(is_safe_filename("CONFIG.html"));
        assert!(is_safe_filename("COM10.html"));
    }

    #[test]
    fn compute_ic_with_fill_one_nots_the_row() {
        assert_eq!(compute_ic_with_fill(&[1, 1, 0, 1, 1], PaddingFill::One), "1");
        assert_eq!(compute_ic_with_fill(&[1, 1, 1], PaddingFill::One), "0");
        assert_eq!(compute_ic_with_fill(&[0, 1, 1, 0], PaddingFill::Zero), "3");
    }

    #[test]
    fn friendly_json_error_strips_serde_noise() {
        let raw = "Failed to deserialize the JSON body into the target type: \
                   rule: invalid value: integer `300`, expected u8 at line 1 column 22";
        assert_eq!(
            friendly_json_error(raw),
            "Invalid input — rule: invalid value: integer `300`, \
             expected a whole number between 0 and 255"
        );
        let raw2 = "Failed to deserialize the JSON body into the target type: \
                    generations: invalid type: null, expected usize at line 1 column 40";
        assert_eq!(
            friendly_json_error(raw2),
            "Invalid input — generations: the field is empty; \
             expected a non-negative whole number"
        );
    }

    #[test]
    fn host_header_check_accepts_loopback_only() {
        assert!(host_is_local("127.0.0.1:3000"));
        assert!(host_is_local("127.0.0.1"));
        assert!(host_is_local("localhost:3000"));
        assert!(host_is_local("LOCALHOST"));
        assert!(host_is_local("[::1]:3000"));
        assert!(!host_is_local("evil.example.com:3000"));
        assert!(!host_is_local("localhost.evil.com:3000"));
        assert!(!host_is_local("[2001:db8::1]:3000"));
        assert!(!host_is_local(""));
    }
}
