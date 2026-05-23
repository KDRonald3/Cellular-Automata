# Web UI Migration

## Summary

Replace the removed native `OutputKind::Ui` with a browser-based equivalent. A Rust HTTP server
(added to the `samples` crate) hosts the simulation API; the existing gallery frontend
(`gallery.html.template` + `gallery.js`) is extended with a parameter form, an in-memory run
panel, and destructive-delete support. Runs live in server memory until the user explicitly saves
or removes them. The library (`cellular_automata`) is called server-side — no WASM.

## Goals

- All parameters from the old native UI are accessible in the browser: Initial (seed string), Rule,
  Generations, Width, Cell px, Boundary, Padding, Fill, Borders.
- Clicking **Run** calls the HTTP API, runs the simulation, and renders the result on a canvas in
  the browser — no page reload.
- Multiple in-memory runs coexist in a session list (tabs / sidebar section), just like the old
  tab strip.
- **Save** writes the run to disk (HTML file + `manifest.tsv` + regenerated `index.html`) and
  promotes the run into the saved-runs gallery.
- **Remove** drops a run from the in-memory session without touching any file on disk.
- **Delete** on a saved gallery entry permanently removes the HTML file from disk, scrubs the row
  from `manifest.tsv`, and regenerates `index.html`.
- The existing gallery sidebar (saved runs, filter, hash deep-linking, canvas viewer) continues to
  work unchanged.

## Non-goals

- WebAssembly / client-side simulation — the library runs on the server only.
- Authentication or multi-user isolation — single local user, localhost only.
- Persisting in-memory runs across server restarts — session state is ephemeral.
- Undoing a Delete — deletion is immediate and permanent, no recovery path.
- Changing the on-disk format produced by the library's existing `export_job` — the server reuses
  it as-is.

## Users / use cases

Single researcher running the server locally. Typical flow:

1. Start the server (`cargo run`) — browser opens (or user navigates) to `http://localhost:<port>`.
2. Fill in parameters, click **Run** → canvas renders inline.
3. Adjust parameters, run again → new in-memory tab appears.
4. For a run worth keeping: click **Save** → file written to `runs/`, entry appears in gallery
   sidebar.
5. Discard an unwanted in-memory run: click **Remove**.
6. Delete an old saved run from the gallery: click **Delete** on its sidebar entry → file gone,
   manifest updated.

## Behavior

### HTTP API (server-side, Rust)

| Method   | Path                      | Action |
|----------|---------------------------|--------|
| `GET`    | `/`                       | Serve the modified `index.html` |
| `GET`    | `/gallery.js`             | Serve `gallery.js` |
| `GET`    | `/manifest.tsv`           | Serve `runs/manifest.tsv` (or empty 200) |
| `GET`    | `/runs/<filename>`        | Serve a saved HTML run file |
| `POST`   | `/api/run`                | Run simulation; return JSON with session ID + packed bit tiles |
| `POST`   | `/api/runs/<id>/save`     | Write HTML + update manifest; return `{ filename }` |
| `DELETE` | `/api/runs/<id>`          | Remove in-memory run from session |
| `DELETE` | `/api/saved/<filename>`   | Delete HTML file + scrub manifest + regenerate index |

`POST /api/run` request body (JSON):

```json
{
  "rule": 102,
  "width": 10,
  "generations": 10,
  "boundary": "ZeroPadded",
  "seed": "0000000001",
  "align": "Before",
  "fill": "Zero",
  "cell_size": 4,
  "show_borders": false,
  "border_width": 0.1
}
```

Response: JSON containing a session `id`, the packed-bit tile data (base64, same format the
existing export embeds), plus the metadata fields the gallery already renders (`w`, `h`, `cs`,
`borders`, `bw`, `tiles`, `fill`, `align`, `ic`).

In-memory runs are stored in a `Mutex<HashMap<u64, RunRecord>>` on the server. IDs are
monotonically assigned (reuse `NEXT_JOB_ID` from `lib.rs`).

### Frontend changes to `gallery.html.template` + `gallery.js`

**New: Run form** — inserted above `#content`, always visible:

- Text input: **Initial** (seed digits, e.g. `0000000001`)
- Number input: **Rule** (0–255)
- Number input: **Generations**
- Number input: **Width**
- Number input: **Cell px** (cell\_size)
- Select: **Boundary** (`Padded` / `Wrap-around`)
- Select: **Padding** (`Padding before` / `Padding after` / `Centered`)
- Select: **Fill** (`Fill 0s` / `Fill 1s`)
- Checkbox: **Borders**
- Button: **Run**

Defaults match the old binary's constants: seed `0000000001`, rule 102, 10 generations, width 10,
4 px cells, ZeroPadded, Before, Zero, no borders.

**New: In-memory run panel** — a second sidebar section (or a tab strip above the canvas)
listing session runs. Each entry shows `#<id> rule <n>` and has:
- **Save** button → `POST /api/runs/<id>/save`; on success the run also appears in the saved
  gallery sidebar and the button greys out / changes to "Saved".
- **Remove** button → `DELETE /api/runs/<id>`; entry disappears from the session list.

Clicking a session run entry renders its canvas from the in-memory tile data (no fetch needed —
data already in JS).

**Modified: saved-runs sidebar** — each `.run-entry` gains a **Delete** button (icon or small
text). Clicking it calls `DELETE /api/saved/<filename>`, confirms success, removes the entry from
the sidebar DOM, and decrements the count badge. No confirmation dialog required.

**Canvas rendering** — unchanged; the existing `renderCanvas()` + `bitAt()` logic in `gallery.js`
handles both saved-run tiles (fetched from disk) and session-run tiles (from API response).

## Affected code

- [samples/src/main.rs](samples/src/main.rs) — replace the `run(..., OutputKind::Ui)` call with
  an HTTP server startup. Most logic moves into a new `server.rs` module in the same crate.
- [samples/Cargo.toml](samples/Cargo.toml) — add HTTP server dependency (e.g. `axum`) and its
  runtime (`tokio`). Adjust `[package]` as needed.
- [cellular_automata/src/export.rs](cellular_automata/src/export.rs) — `export_job` and
  `pack_bits_range` are reused as-is. A new thin helper may be needed to delete a manifest entry
  and regenerate the index without writing a full new run.
- [cellular_automata/src/gallery.html.template](cellular_automata/src/gallery.html.template) —
  add the run form markup, session-run panel, and Delete buttons. Keep all existing styles and
  structure.
- [cellular_automata/src/gallery.js](cellular_automata/src/gallery.js) — add `runForm` submit
  handler (`POST /api/run`), session list management, Save/Remove handlers, Delete handler on
  gallery entries. Existing `loadRun`, `renderCanvas`, `buildSidebar`, `parseTsv`, `init` are
  preserved.

## Edge cases & constraints

- **Seed wider than width**: validated server-side; API returns a 422 with an error message that
  the JS displays near the form (not a browser alert).
- **Width zero**: same — 422 with message.
- **Non-binary seed characters**: seed string is parsed digit-by-digit (non-zero → 1); invalid
  characters are silently treated as 0, matching the old binary's `filter_map` behavior
  ([main.rs:18](samples/src/main.rs#L18)).
- **Delete while viewing**: if the user deletes a saved run that is currently displayed in the
  canvas viewer, the viewer stays showing the already-rendered canvas (data is in JS memory).
  The sidebar entry disappears. No extra UX needed.
- **Concurrent saves to same directory**: already serialized by `MANIFEST_LOCK` in
  [export.rs:17](cellular_automata/src/export.rs#L17).
- **Large runs**: tile packing (`pack_bits_range`) and base64 are done server-side; the JSON
  response can be large. No streaming is needed for the expected interactive scale (≤ a few
  thousand generations). For very large runs the existing `MAX_EXPORT_CELLS` / `MAX_EXPORT_HEIGHT`
  caps still apply when saving.
- **Port**: default `127.0.0.1:3000`; can be overridden via a CLI argument or `PORT` env var.
- **`runs/` directory**: created on first save if it does not exist (same as existing
  `fs::create_dir_all` in `export_job`).

## Decisions made

- **HTTP server, not WASM**: the library has no existing WASM target and the user explicitly asked
  for HTTP. Server-side keeps the library's existing output pipeline (including HTML export)
  unchanged.
- **Reuse existing export machinery**: `export_job` already writes the HTML + manifest + index.
  The Save API call just forwards to it. No parallel export path.
- **No confirmation on Delete**: the user confirmed deletion should be immediate and permanent.
  A confirmation dialog was considered and rejected as unnecessary friction for a local tool.
- **Session state in memory, not a database**: runs are small and the tool is single-user. A
  `Mutex<HashMap>` is sufficient and avoids a persistence dependency.
- **Save button greys out after saving**: prevents double-saving the same in-memory run without
  adding a uniqueness check on the server.

## Decisions made (continued)

- **Auto-open browser on startup**: the server opens `http://localhost:<port>` in the default
  browser immediately after binding. Matches the old native app's behavior of showing the UI
  without a manual step.
- **Session runs are purely in-memory**: unsaved runs live only in the server process and are gone
  when the server closes. No drafts folder, no temp files, no recovery on restart. This is the
  intended behavior — the user explicitly wants them to disappear on close.
