# New Simulation Screen — Design Overhaul

## Summary

Apply the visual design from the FLAT-BUFFER V3 mockup to the New Simulation screen. The home screen and its saved-runs sidebar are explicitly out of scope — they keep their current look. The viewer bar (above the canvas) gains a thin progress bar below the simulation title. No functionality changes; this is a pure design pass plus one new UI element (progress bar).

## Goals

- The New Simulation screen's sidebar matches the mockup's typography, color application, spacing, and control treatments exactly.
- The viewer bar follows the same design language with the addition of a progress bar.
- The home screen is visually unchanged.

## Non-goals

- Home screen redesign — sidebar, topbar, saved-run entries all stay as-is.
- Functional changes to how runs are submitted, saved, or displayed.
- Streaming/incremental simulation progress from the server — the progress bar is a client-side loading indicator only.
- Redesigning the session run entries in the sidebar (those stay as compact list items).

## Behavior

### Sidebar — New Simulation screen

**Section headers**
Labels like "SIMULATION" and any future section breaks are rendered as a small-caps/uppercase monospace label with a horizontal rule extending to the right edge of the sidebar:
```
SIMULATION ──────────────────────
```
The rule is the same color as the existing `--rule` variable. The label color matches the mockup's muted warm tone.

**Form controls**
- Field labels (INITIAL ROW, RULE (0-255), GENERATIONS, WIDTH, CELL SIZE PX, PADDING, FILL) remain small uppercase mono above each input, styled to match the mockup.
- Text and number inputs are full-width within the sidebar, minimal border, matching the mockup's flat/clean style.
- Dropdowns (PADDING, FILL) remain `<select>` elements, styled to match.

**Boundary control**
Replace the current `<select id="f-boundary">` with a two-button segmented toggle:
- Two buttons side by side: "Padded" and "Wrap-around"
- The active button has a filled black background with white text
- The inactive button has a transparent background with dark text and a border
- Toggling updates the underlying boundary value the same way the select did

**SHOW CELL BORDERS**
Checkbox + label styled to match the mockup: visible square checkbox, uppercase label.

**Run and Reset buttons**
- Replace the current "Run" button with "RUN SIMULATION →" — full-width, all-caps, dark (near-black) fill, white text, monospace, with a right arrow character
- Add a "RESET" button beside it (not full-width), outline style, resets all form fields to their default values:
  - Initial: `0000000001`
  - Rule: `102`
  - Generations: `10`
  - Width: `10`
  - Cell px: `4`
  - Boundary: Padded
  - Padding: Before seed
  - Fill: Fill 0s
  - Borders: unchecked

### Viewer bar — progress bar

A thin bar (~2–3px tall) sits directly below the simulation title (`#v-title`) within the viewer bar:

- **Hidden** when no run is active (no space taken).
- **Indeterminate animation** (sweeping highlight) while the run is computing (between clicking RUN SIMULATION and the result arriving).
- **Solid green** (matching the mockup's done-state green: `oklch(46% 0.12 145)`) once the canvas has fully rendered.
- Spans the full width of the title element, not the full bar width.
- Does not affect the height of the viewer bar — sits flush below the title text within its existing line.

## Affected code

- [src/web_ui.html](src/web_ui.html) — all changes are CSS and JS within this single file:
  - CSS: section-header rule treatment, segmented toggle styles, run/reset button styles, progress bar styles, input/label refinements
  - HTML: replace `#f-boundary` select with two-button toggle; add reset button beside run button; add progress bar element inside `#viewer-bar`
  - JS: boundary toggle logic (keep same `value` interface); reset button handler; progress bar show/animate/complete lifecycle tied to `runBtn` click → `applyLoaded`

## Decisions made

- **Progress bar is client-side only** — the server returns a complete result in one response; there is no streaming API. The bar animates while the fetch is in-flight and goes green when `applyLoaded` is called. We chose Option A (animated indeterminate → green) over a static always-green bar because it gives useful feedback during longer computations.
- **Scope is New Simulation screen only** — home screen design is intentionally left unchanged; mixing the two would require reconciling two different visual languages and is out of scope for this pass.
- **Segmented toggle replaces select for Boundary** — the mockup uses a two-button toggle and this is the only binary-choice field, making the toggle the right control. PADDING and FILL remain selects because they have three options each.
- **RESET defaults match the current HTML input defaults** — no server-side reset needed; JS resets DOM values directly.
