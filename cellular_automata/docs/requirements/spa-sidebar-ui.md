# Requirements: SPA Sidebar UI & Human-Readable Run Nomenclature

## Summary

Replace the current two-page HTML export system (a table-based `index.html` plus
separate per-run pages) with a single-page application. A collapsible burger-menu
sidebar lists all saved runs grouped by rule number; clicking a run loads its canvas
in-place without navigating away. Run display names follow a human-readable format
derived from the simulation parameters, and the sidebar search matches against those
names.

## Goals

- One persistent layout: sidebar always available, canvas fills the rest of the viewport.
- Runs browsable without page navigation — clicking a sidebar entry swaps the canvas.
- Sidebar search works with natural language queries like `200 generations of rule 102 padded 1 before 1`.
- Initial condition visible and searchable in human-readable form, including fill value
  and alignment (e.g. `padded 1 before 1`).
- Rust export code extended to carry initial condition, padding fill, and padding
  alignment through to the manifest and the per-run meta blob.

## Non-goals

- Renaming the `.html` files on disk — technical filenames stay as-is.
- Changing how the canvas renders (cell size slider, border toggle, bit-packing) — that
  code is untouched.
- Adding new simulation parameters or changing the Rust simulation core.
- Server-side components — everything runs client-side from `file://` or a static server.

## Users / use cases

Single user (researcher) browsing saved runs. Typical flow:

1. Open `runs/index.html` in a browser.
2. Burger button reveals the sidebar.
3. Expand the rule group of interest (e.g. Rule 102).
4. Click a run entry — canvas loads immediately, metadata updates in the main area.
5. Optionally type in the search box to narrow the list (e.g. `wrap-around 1`).

## Behavior

### Layout

- **Main area**: canvas + controls (cell size slider, border toggle) — same as today.
- **Sidebar**: slides in from the left, overlays the canvas (does not push/reflow it).
- **Burger button** (`☰`): fixed position, always visible, toggles the sidebar.

### Sidebar contents

- Runs are grouped by rule number.
- Groups are listed in **descending** rule-number order (Rule 255 first, Rule 0 last).
- Each group header shows the rule number and is **collapsible**.
- Within a group, entries are listed in **descending** chronological order (newest first).
- Each entry displays the **human-readable display name** (see Nomenclature section).
- Clicking an entry loads that run's canvas without navigating away.
- A search/filter input at the top of the sidebar narrows visible entries by matching
  the search terms against the human-readable display name of every entry.

### Canvas loading

When a sidebar entry is clicked:

1. Fetch the corresponding `.html` file (path from the manifest).
2. Parse the `<script id="meta" type="application/json">` block to get `w`, `h`, `cs`,
   `borders`, `bw`, and `tiles`.
3. For each tile entry, fetch and parse the matching `<script id="bits*"
   type="application/octet-stream">` base-64 blob.
4. Re-render using the existing canvas drawing logic (already in each run page — this
   logic will be lifted into the SPA shell).
5. Update the metadata display (rule, generations, boundary, initial condition, timestamp).

Individual run HTML files remain self-contained and navigable directly; the SPA just
uses them as data sources via `fetch`.

### Nomenclature (display name)

Format: `[generations] generations of rule [rule number] [boundary label] padded [fill] [align] [initial condition]`

**Boundary labels:**

| `BoundaryMode`  | Label in name    |
|-----------------|------------------|
| `ZeroPadded`    | *(omitted)*      |
| `Wrap`          | `wrapped around` |

`ZeroPadded` is omitted because "padded" already appears in the initial-condition
descriptor immediately after; writing both would read as "padded padded …".

**Initial condition descriptor** — three parts derived at export time:

| Part | Source | Example values |
|------|--------|----------------|
| `padded [fill]` | `PaddingFill` | `padded 0`, `padded 1` |
| `[align]` | `PaddingAlign` | `before`, `after`, `centered` |
| `[decimal]` | seed binary → base-10 | `1`, `3`, `42` |

The decimal is computed from `rows[0..width]`:
1. Find the first `1` and the last `1` in the row.
2. Slice that range: `rows[first_one..=last_one]`.
3. Interpret as a big-endian binary number (leftmost = MSB).
4. Convert to base-10 string.
5. If no `1` is present, the value is `0`.

**Full examples:**

| Parameters | Display name |
|---|---|
| Rule 102, 200 gen, ZeroPadded, fill 0, before, seed=`1` | `200 generations of rule 102 padded 0 before 1` |
| Rule 102, 200 gen, ZeroPadded, fill 1, before, seed=`1` | `200 generations of rule 102 padded 1 before 1` |
| Rule 102, 200 gen, Wrap, fill 0, centered, seed=`1` | `200 generations of rule 102 wrapped around padded 0 centered 1` |
| Rule 119, 20000 gen, ZeroPadded, fill 0, centered, seed=`1` | `20000 generations of rule 119 padded 0 centered 1` |
| Rule 3, 200 gen, ZeroPadded, fill 1, after, seed=`11`→`3` | `200 generations of rule 3 padded 1 after 3` |

`ExportInput` gains two new fields: `padding_fill: PaddingFill` and
`padding_align: PaddingAlign`. All three callers (`ui.rs`, `lib.rs`, `main.rs`) already
hold these values and pass them to `make_initial_row`; they just need to forward them
into `ExportInput` as well.

The manifest gains three new columns: `fill`, `align`, `initial_condition` (positions
9–11). The per-run meta JSON gains `"fill"`, `"align"`, and `"ic"` fields. Existing
manifest rows with fewer columns are handled gracefully — the SPA falls back to the
technical filename for old runs missing any of these fields.

Filenames on disk are **not** renamed.

## Affected code

| File | Change |
|------|--------|
| [src/export.rs](../../src/export.rs) | Add `padding_fill` and `padding_align` fields to `ExportInput`; compute `ic` string from `rows[0..width]`; add `"fill"`, `"align"`, `"ic"` to meta JSON; add three new columns to manifest TSV; update `regenerate_index` |
| [src/ui.rs](../../src/ui.rs) | Forward `padding_fill` and `padding_align` into `ExportInput` at [ui.rs:294](../../src/ui.rs#L294) |
| [src/lib.rs](../../src/lib.rs) | Forward `padding_fill` and `padding_align` into `ExportInput` at [lib.rs:426](../../src/lib.rs#L426) |
| [src/main.rs](../../src/main.rs) | Forward `padding_fill` and `padding_align` into `ExportInput` |
| [runs/index.html](../../runs/index.html) | Full rewrite: becomes the SPA shell with sidebar, canvas area, and all JS logic |
| Per-run HTML files (future exports) | `meta` JSON gains `"fill"`, `"align"`, `"ic"` fields; old files fall back to technical filename |
| [src/export.rs `INDEX_HEAD` / `INDEX_SCRIPT`](../../src/export.rs#L511) | Replaced with new SPA template |

## Edge cases & constraints

- **`file://` protocol**: `fetch` of sibling files works in most browsers when opened
  via `file://` if they're in the same directory. The existing fallback-to-baked-rows
  pattern is preserved for the sidebar manifest load.
- **Missing `fill`/`align`/`ic` fields** (old exports): display name falls back to the technical filename.
- **All-zero initial row**: initial condition displays as `0`.
- **Very large initial conditions**: for wide automata with a complex seed, the base-10
  number can be very large. The JS `BigInt` type handles arbitrary precision; the Rust
  side stores it as a string in the manifest to avoid overflow.
- **Concurrent exports**: the `MANIFEST_LOCK` mutex in [export.rs:17](../../src/export.rs#L17)
  already serialises manifest writes; the new column doesn't change that.
- **Sidebar on narrow viewports**: the overlay approach (sidebar covers canvas) means no
  layout reflow; acceptable for a research tool.

## Decisions made

- **Filenames stay technical** — human-readable names would require timestamps or job IDs
  for uniqueness anyway, making them nearly as opaque. Display labels are a cleaner
  separation of concerns.
- **Search on display name, not filename** — the entire point of the nomenclature is
  natural-language searchability; searching the human-readable label is the right target.
- **Sidebar overlays (doesn't push) the canvas** — avoids canvas resize/reflow on toggle,
  which would trigger a re-render at the new width.
- **Fill IS included in the name** — revised from an earlier position that fill would be
  omitted. "padded 1 before" is more informative and searchable than just the decimal.
- **`ExportInput` gains `padding_fill` and `padding_align`** — all three callers already
  hold these values (they pass them to `make_initial_row`); the change is purely additive.
- **`ic` derived from `rows[0..width]`** — no separate seed field needed; the initial
  row is already in `ExportInput.rows`.
- **`BigInt` for large initial conditions in JS** — `Number` is unsafe beyond 2^53;
  `BigInt` is universally supported in modern browsers and handles arbitrary widths
  correctly.
- **Per-run files remain self-contained** — the SPA fetches and parses them rather than
  splitting data into separate `.json`/`.bin` sidecar files, avoiding a format change
  that would break existing exports.

## Open questions

- None — all design decisions were resolved in the discovery conversation.

## Out of scope / deferred

- Dark mode or theming.
- Pagination of the sidebar (all runs load into the sidebar at once).
- Deleting or archiving runs from the UI.
- Sorting within a rule group by anything other than date.
