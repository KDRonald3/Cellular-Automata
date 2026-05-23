---
name: ca-rule-pdfs
description: Generate two PDFs of 1D CA simulation grids, one per reference HTML file, 4 rules per page with two ICs each
metadata:
  type: project
---

# CA Rule PDFs

## Summary

Generate two PDFs, one per reference HTML file, each containing 1D elementary cellular automaton simulation grids for the rule numbers listed in that file. Each rule is shown with a label and two side-by-side simulation grids (one per initial condition). 4 rules per page, 4 pages per PDF (16 rules each).

## Goals

- Produce `multiples_of_17_rules.pdf` for the 16 multiples-of-17 rules
- Produce `weighted_sum_rules.pdf` for the 16 4-bit weighted-sum rules
- Each rule appears with its number, 8-bit binary expansion, and two simulation grids side by side
- Output saved to the repo root (`c:\Users\kouat\Research\Cellular Automata\`)

## Non-goals

- Converting the reference HTML files themselves to PDF (screen-capture / print)
- Interactive or animated output
- Rules outside each file's defined 16-rule set

## Rule sets

**`multiples_of_17_rules.pdf`** — source: `multiples_of_17_binary_presentable.html`

0, 17, 34, 51, 68, 85, 102, 119, 136, 153, 170, 187, 204, 221, 238, 255

**`weighted_sum_rules.pdf`** — source: `Weighted Sum Reference.html` (weights 192, 48, 12, 3)

0, 3, 12, 15, 48, 51, 60, 63, 192, 195, 204, 207, 240, 243, 252, 255

## Simulation parameters

| Parameter    | Value      |
|-------------|------------|
| Width       | 40         |
| Generations | 50         |
| Boundary    | Wraparound |
| Alignment   | Center     |

The grid rendered is 40 cells wide × 51 rows tall (initial row + 50 generations).

## Initial conditions

Two ICs per rule, shown side by side:

| IC  | Seed   | Fill |
|-----|--------|------|
| IC1 | `1011` | 0    |
| IC2 | `0100` | 1    |

**For `multiples_of_17_rules.pdf` only:** seeds are read right-to-left (reversed):

| IC  | Seed   | Fill |
|-----|--------|------|
| IC1 | `1101` | 0    |
| IC2 | `0010` | 1    |

The seed bits are placed centered in the 40-cell row; all remaining cells take the fill value.

## PDF layout

- Paper: Letter (8.5 × 11 in)
- 4 rules per page → 4 pages per PDF
- Per rule block:
  - Label row: `Rule N  —  XXXXXXXX` (rule number and 8-bit binary, e.g. `Rule 30  —  00011110`)
  - Two grids side by side: IC1 on the left, IC2 on the right
- Cell size: **6 px** → each grid is 240 × 306 px (40 × 6 wide, 51 × 6 tall)

## Output files

```
c:\Users\kouat\Research\Cellular Automata\multiples_of_17_rules.pdf
c:\Users\kouat\Research\Cellular Automata\weighted_sum_rules.pdf
```

## Affected code

- [cellular_automata/src/lib.rs](../../cellular_automata/src/lib.rs) — CA simulation library (reference; not modified)
- [cellular_automata/src/svg.rs](../../cellular_automata/src/svg.rs) — SVG generation (reference; not modified)
- New: `gen_rule_pdfs.py` at repo root — standalone Python script; implements CA simulation, grid rendering, and PDF assembly

## Decisions made

**Cell size: 6 px.**
Width 40 at 6 px = 240 px per grid; two grids plus margins fit comfortably across a Letter page, and the pattern is readable at that scale.

**Bit-inversion for multiples-of-17: reverse the seed string.**
"Invert position from right to left" is interpreted as reversing the seed binary string — `1011` → `1101`, `0100` → `0010`. This is the most natural reading of reading the bit string right-to-left.

**Implementation: standalone Python script using `reportlab`.**
The CA simulation is a few lines of bitwise Python; no Rust compilation or FFI is needed. Cells are drawn directly as rectangles on the reportlab canvas — no SVG intermediary, no headless browser, no `svglib` dependency. Only `reportlab` is required (`pip install reportlab`).

## Open questions

None — all parameters confirmed.
