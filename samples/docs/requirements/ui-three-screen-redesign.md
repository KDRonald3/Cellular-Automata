# UI Three-Screen Redesign

## Summary

Restructure the current single-page web UI into three distinct screens. The **Home screen** becomes a minimal entry point for browsing and opening saved runs. A **New Simulation screen** handles parameter input and tracks session runs. A unified **viewer treatment** gives the canvas maximum space on both screens. Navigation between screens is explicit and directional.

## Goals

- Home screen is free of simulation controls and session-run state.
- New Simulation screen holds all input fields and the current session list in one sidebar.
- Opening any simulation (saved or session) produces a consistent layout: one horizontal bar above the canvas, canvas fills the rest of the main area.
- Per-run save action works correctly (current save is broken — must be fixed).

## Non-goals

- Sidebar export section (output directory, format selectors) — do not add.
- "Saved runs/filename" confirmation banner — do not add.
- Multi-user support, auth, or WASM.
- Changes to the server API routes or on-disk format.

## Screens

### 1. Home Screen

The current `web_ui.html` page, simplified:

- **Saved-runs sidebar** stays intact — this is how users browse and open old simulations.
- **Session section** (`#session-section`, "No runs yet — hit Run") is removed entirely.
- **Run form bar** (`#run-form-bar`) is removed entirely.
- A **"New Simulation" button** is added prominently in the topbar or main empty-state area, linking to the New Simulation screen.
- **Viewer treatment** (see below) applies when a saved run is clicked.

Affected elements in [src/web_ui.html](src/web_ui.html):
- Remove `#session-section` (~[L269–276](src/web_ui.html#L269))
- Remove `#run-form-bar` (~[L314–364](src/web_ui.html#L314))
- Add New Simulation button to topbar or empty state

### 2. New Simulation Screen

A second screen (same single HTML file, shown/hidden via JS routing):

- **Sidebar** contains, top to bottom:
  1. All input fields: initial row, rule, generations, width, cell size, boundary, padding, fill, borders checkbox, Run button.
  2. Below the fields: session runs list (runs completed in the current server session). Each entry has the run name and a **Save** button that correctly persists to disk.
- **Back-link** in the topbar returns to Home.
- **Main area**: empty state when no session run is selected; viewer treatment when one is clicked.
- Sidebar remains open while a run is being viewed — canvas fills the area to the right of the sidebar.

### 3. Viewer Treatment (both screens)

Replaces the current viewer layout (`.viewer-head` + `.meta-row` + `.controls` + `.canvas-card`):

- **Single horizontal bar** above the canvas containing:
  - Title (same styling as current `.rule-tag`: monospace, small, uppercase, accent color — not a large h1)
  - Meta cells: rule, width, generations, boundary, fill, align
  - Cell size control (range slider + number input)
  - Borders toggle checkbox
  - Filename
- **Canvas** fills all remaining vertical space below the bar and all horizontal space to the right of any open sidebar.
- The decorative canvas card padding/border is removed (canvas is flush or minimal).

## Affected Code

- [src/web_ui.html](src/web_ui.html) — all HTML, CSS, and JS changes live here
- [src/server.rs:408–444](src/server.rs#L408) — `post_save_run` handler: investigate and fix the broken save

## Known Bug

`POST /api/runs/{id}/save` does not work. Root cause unknown — needs investigation before the save button can be wired up on the New Simulation screen. The fix is in scope for this work.

## Decisions Made

- **Canvas fills main area, not full window** — the form sidebar stays open on the New Simulation screen, so "fills the window horizontally" means fills the area to the right of the sidebar, not edge-to-edge.
- **Title styled like rule-tag, not h1** — compact mono/uppercase/accent treatment so it fits in the horizontal bar without dominating it.
- **Per-run save survives the redesign** — the sidebar export section is removed, but saving an individual session run to disk remains a core action.
- **Single HTML file, JS routing** — no new server routes needed; screens are toggled in the browser.

## Open Questions

- Save bug root cause: whether it is a frontend wiring issue or a backend handler bug needs to be established before the session list save button can be finalized.
