#!/usr/bin/env python3
"""Generate CA rule PDFs per docs/requirements/ca-rule-pdfs.md."""

import os
from reportlab.lib.pagesizes import LETTER
from reportlab.pdfgen import canvas as pdf_canvas

OUTPUT_DIR = os.path.dirname(os.path.abspath(__file__))

RULES_WEIGHTED_SUM = [
    0, 3, 12, 15, 48, 51, 60, 63,
    192, 195, 204, 207, 240, 243, 252, 255,
]

RULES_MULTIPLES_17 = [
    0, 17, 34, 51, 68, 85, 102, 119,
    136, 153, 170, 187, 204, 221, 238, 255,
]

WIDTH  = 40
GENS   = 50
CS     = 3    # points per cell; 3 pt is the largest that fits 4 rules on Letter

MARGIN    = 36   # pt (0.5 in)
LABEL_H   = 16   # pt
GRID_W    = WIDTH * CS        # 120 pt
GRID_H    = (GENS + 1) * CS  # 153 pt  (initial row + 50 generations)
GRID_GAP  = 20   # pt between the two per-rule grids
BLOCK_GAP = 12   # pt between consecutive rule blocks
BLOCK_H   = LABEL_H + GRID_H  # 169 pt per rule block


def simulate(rule_num, seed, fill):
    row = [fill] * WIDTH
    offset = (WIDTH - len(seed)) // 2
    for i, bit in enumerate(seed):
        row[offset + i] = bit
    grid = [row[:]]
    for _ in range(GENS):
        prev = row
        row = []
        for i in range(WIDTH):
            nbr = (prev[(i - 1) % WIDTH] << 2) | (prev[i] << 1) | prev[(i + 1) % WIDTH]
            row.append((rule_num >> nbr) & 1)
        grid.append(row[:])
    return grid


def draw_grid(c, x, y, grid):
    """Draw a CA grid; y is the PDF y-coordinate of the TOP edge."""
    rows = len(grid)
    c.setFillColorRGB(1, 1, 1)
    c.rect(x, y - rows * CS, GRID_W, rows * CS, fill=1, stroke=0)
    c.setFillColorRGB(0, 0, 0)
    for ri, row in enumerate(grid):
        cy = y - (ri + 1) * CS
        for ci, cell in enumerate(row):
            if cell:
                c.rect(x + ci * CS, cy, CS, CS, fill=1, stroke=0)
    c.setStrokeColorRGB(0, 0, 0)
    c.setLineWidth(0.4)
    c.rect(x, y - rows * CS, GRID_W, rows * CS, fill=0, stroke=1)


def generate_pdf(path, rules, ic1_seed, ic2_seed):
    page_w, page_h = LETTER  # 612 x 792 pt
    two_w = 2 * GRID_W + GRID_GAP
    x1 = MARGIN + (page_w - 2 * MARGIN - two_w) / 2
    x2 = x1 + GRID_W + GRID_GAP
    y_top = page_h - MARGIN  # 756 pt

    total_pages = (len(rules) + 3) // 4
    c = pdf_canvas.Canvas(path, pagesize=LETTER)

    for page_i, chunk_start in enumerate(range(0, len(rules), 4)):
        chunk = rules[chunk_start:chunk_start + 4]
        y = y_top

        for rule_num in chunk:
            label = "Rule {}    {:08b}".format(rule_num, rule_num)
            c.setFont("Courier-Bold", 10)
            c.setFillColorRGB(0.1, 0.1, 0.1)
            label_w = c.stringWidth(label, "Courier-Bold", 10)
            c.drawString(x1 + (two_w - label_w) / 2, y - LABEL_H + 3, label)

            grid_top = y - LABEL_H
            draw_grid(c, x1, grid_top, simulate(rule_num, ic1_seed, 0))
            draw_grid(c, x2, grid_top, simulate(rule_num, ic2_seed, 1))

            y -= BLOCK_H + BLOCK_GAP

        c.setFont("Helvetica", 8)
        c.setFillColorRGB(0.5, 0.5, 0.5)
        c.drawCentredString(page_w / 2, MARGIN / 2, "{} / {}".format(page_i + 1, total_pages))
        c.showPage()

    c.save()
    print("Saved: {}".format(path))


if __name__ == "__main__":
    generate_pdf(
        os.path.join(OUTPUT_DIR, "weighted_sum_rules.pdf"),
        RULES_WEIGHTED_SUM,
        ic1_seed=[1, 0, 1, 1],
        ic2_seed=[0, 1, 0, 0],
    )
    generate_pdf(
        os.path.join(OUTPUT_DIR, "multiples_of_17_rules.pdf"),
        RULES_MULTIPLES_17,
        ic1_seed=[1, 1, 0, 1],  # 1011 reversed
        ic2_seed=[0, 0, 1, 0],  # 0100 reversed
    )
