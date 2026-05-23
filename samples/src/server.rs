use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use axum::{
    Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json,
};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use cellular_automata::{
    BoundaryMode, InitialRow, OutputKind, PaddingAlign, PaddingFill,
    RenderOptions, SimConfig, SimulationResult,
    delete_saved_run, run, save_result_if_unique,
};
use serde::{Deserialize, Serialize};

// ── Tile packing (replicated from cellular_automata::export — kept private) ──

const MAX_EXPORT_CELLS: usize = 200_000_000;
const MAX_EXPORT_HEIGHT: usize = 32_000;
const MAX_SESSIONS: usize = 1_000;
const MAX_WIDTH: usize = 10_000;
const MAX_GENERATIONS: usize = 100_000;
const MAX_REQUEST_BYTES: usize = 64 * 1024;

fn is_safe_filename(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
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
    let height = if width == 0 { 0 } else { result.rows.len() / width };
    let cs = if width == 0 { 1 } else { (1600usize / width.max(1)).clamp(1, 16) };

    let tile_max_rows_by_cells = if width == 0 { 1 } else { (MAX_EXPORT_CELLS / width).max(1) };
    let tile_max_rows = tile_max_rows_by_cells
        .min(MAX_EXPORT_HEIGHT / cs.max(1))
        .max(1);
    let num_tiles = if height == 0 { 0 } else { (height + tile_max_rows - 1) / tile_max_rows };

    let ic = if width == 0 || result.rows.is_empty() {
        "0".to_string()
    } else {
        compute_ic(&result.rows[0..width])
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

// ── Route handlers ────────────────────────────────────────────────────────────

async fn get_index() -> impl IntoResponse {
    const HTML: &str = include_str!("web_ui.html");
    (
        [(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"))],
        HTML,
    )
}

async fn get_manifest(State(state): State<AppState>) -> Response {
    let path = state.runs_dir.join("manifest.tsv");
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => (
            [(header::CONTENT_TYPE, HeaderValue::from_static("text/tab-separated-values; charset=utf-8"))],
            content,
        ).into_response(),
        Err(_) => (StatusCode::OK, "").into_response(),
    }
}

async fn get_run_file(
    State(state): State<AppState>,
    Path(filename): Path<String>,
) -> Response {
    if !is_safe_filename(&filename) || !filename.ends_with(".html") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let path = state.runs_dir.join(&filename);
    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            [(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"))],
            bytes,
        ).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn post_run(
    State(state): State<AppState>,
    Json(req): Json<RunRequest>,
) -> Response {
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
    Path(id_str): Path<String>,
) -> Response {
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
    // Dedup check and save are atomic under MANIFEST_LOCK inside save_result_if_unique.
    let outcome = tokio::task::spawn_blocking(move || {
        save_result_if_unique(&sr.result, &sr.render, &runs_dir, sr.padding_fill, sr.padding_align)
    })
    .await;

    match outcome {
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::AlreadyExists => {
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
    Path(id_str): Path<String>,
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
    Path(filename): Path<String>,
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
        Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => {
            StatusCode::BAD_REQUEST.into_response()
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
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
        .with_state(state)
}
