/* Cellular Automata — runs gallery
 *
 * Sidebar push (default closed), rule groups ascending, runs sorted by
 * IC value ASC then generations ASC, live filter, hash-based deep linking,
 * canvas rendering with fetch fallback to in-JS CA computation.
 *
 * IC stored as decimal integer (e.g. "1", "5", "255") — computeRun() converts
 * via BigInt().toString(2) to get seed bits.
 */

(function () {
  'use strict';

  // ----- DOM refs -----
  const body      = document.body;
  const toggleBtn = document.getElementById('toggle');
  const filterEl  = document.getElementById('filter');
  const listEl    = document.getElementById('run-list');
  const countEl   = document.getElementById('total-count');

  const resizer = document.getElementById('resizer');
  const vTag   = document.getElementById('v-tag');
  const vTitle = document.getElementById('v-title');
  const vMeta  = document.getElementById('v-meta');
  const vFile  = document.getElementById('v-file');
  const vDims  = document.getElementById('v-dims');
  const vPre   = document.getElementById('v-pre');

  const tilesEl = document.getElementById('tiles');
  const noteEl  = document.getElementById('tile-note');

  const csRange = document.getElementById('cs-range');
  const csNum   = document.getElementById('cs-num');
  const bCheck  = document.getElementById('b-check');

  // ----- State -----
  let curCs = 4;
  let curBorders = false;
  let curMeta = null;
  let curTiles = [];
  let activeEl = null;

  const esc = (s) => String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');

  // ----- Sidebar toggle -----
  toggleBtn.addEventListener('click', () => {
    body.classList.toggle('sidebar-open');
  });

  // ----- Sidebar resize -----
  const SIDEBAR_MIN = 180;
  const SIDEBAR_MAX = 640;
  const savedW = localStorage.getItem('ca-sidebar-w');
  if (savedW) document.documentElement.style.setProperty('--sidebar-w', savedW + 'px');

  resizer.addEventListener('mousedown', (e) => {
    e.preventDefault();
    const startX = e.clientX;
    const startW = parseInt(getComputedStyle(document.documentElement).getPropertyValue('--sidebar-w'), 10) || 304;
    body.classList.add('resizing');

    const onMove = (ev) => {
      const w = Math.max(SIDEBAR_MIN, Math.min(SIDEBAR_MAX, startW + ev.clientX - startX));
      document.documentElement.style.setProperty('--sidebar-w', w + 'px');
    };
    const onUp = () => {
      body.classList.remove('resizing');
      const w = parseInt(getComputedStyle(document.documentElement).getPropertyValue('--sidebar-w'), 10);
      localStorage.setItem('ca-sidebar-w', w);
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
    };
    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
  });

  // ----- Naming -----
  function displayName(e) {
    if (!e.fill && !e.align && !e.ic) return e.filename;
    const bnd = (e.boundary === 'Wrap-around' || e.boundary === 'Wrap') ? 'wrap-around' : 'padded';
    return (e.generations + ' generations of rule ' + e.rule + ' ' + bnd + ' fill ' + e.fill + ' ' + e.align + ' ' + e.ic).replace(/\s+/g, ' ').trim();
  }
  function shortName(e) {
    const bnd = (e.boundary === 'Wrap-around' || e.boundary === 'Wrap') ? 'wrap-around' : 'padded';
    return 'rule ' + e.rule + ' ' + e.generations + ' gens · ' + bnd + ' fill ' + e.fill + ' ' + e.align + ' ' + e.ic;
  }

  // ----- Sort -----
  // Within a rule: ASC by IC (decimal numeric), then ASC by generations.
  function sortRuns(entries) {
    return entries.slice().sort((a, b) => {
      const icA = parseInt(a.ic || '0', 10) || 0;
      const icB = parseInt(b.ic || '0', 10) || 0;
      if (icA !== icB) return icA - icB;
      const gA = parseInt(a.generations || '0', 10) || 0;
      const gB = parseInt(b.generations || '0', 10) || 0;
      if (gA !== gB) return gA - gB;
      return String(a.timestamp || '').localeCompare(String(b.timestamp || ''));
    });
  }

  // ----- Sidebar build -----
  function buildSidebar(entries) {
    const groups = new Map();
    for (const e of entries) {
      const r = parseInt(e.rule, 10);
      if (!groups.has(r)) groups.set(r, []);
      groups.get(r).push(e);
    }
    const rules = Array.from(groups.keys()).sort((a, b) => a - b);

    listEl.innerHTML = '';
    let total = 0;
    for (const rule of rules) {
      const ruleEntries = sortRuns(groups.get(rule));
      total += ruleEntries.length;

      const det = document.createElement('details');
      det.className = 'rule-group';
      det.open = true;

      const sum = document.createElement('summary');
      sum.innerHTML =
        '<span class="chev" aria-hidden="true"></span>' +
        '<span class="rule-label">Rule</span>' +
        '<span class="rule-num">' + esc(String(rule).padStart(3, '0')) + '</span>' +
        '<span class="rule-count">' + ruleEntries.length + '</span>';
      det.appendChild(sum);

      const grpBody = document.createElement('div');
      grpBody.className = 'group-body';
      for (const e of ruleEntries) {
        const row = document.createElement('div');
        row.className = 'run-entry';
        row.setAttribute('data-filename', e.filename);
        row.innerHTML =
          '<span class="run-name">' + esc(shortName(e)) + '</span>' +
          '<span class="run-meta">#' + esc(e.id) + '</span>';
        row.addEventListener('click', () => loadRun(e, row));
        grpBody.appendChild(row);
      }
      det.appendChild(grpBody);
      listEl.appendChild(det);
    }
    countEl.textContent = total;
  }

  // ----- Filter -----
  filterEl.addEventListener('input', () => {
    const term = filterEl.value.toLowerCase().trim();
    const groups = listEl.querySelectorAll('details.rule-group');
    for (const g of groups) {
      const items = g.querySelectorAll('.run-entry');
      let any = false;
      for (const it of items) {
        const match = !term || it.textContent.toLowerCase().includes(term);
        it.style.display = match ? '' : 'none';
        if (match) any = true;
      }
      g.style.display = any ? '' : 'none';
      if (term && any) g.open = true;
    }
  });

  // ----- Canvas render -----
  function renderCanvas() {
    tilesEl.innerHTML = '';
    noteEl.textContent = '';
    if (!curMeta || curMeta.w === 0) return;
    const cs = curCs, borders = curBorders, meta = curMeta, tiles = curTiles;
    let first = true;
    for (const t of tiles) {
      const bin = t.bin;
      const tileH = t.tileH;
      const cv = document.createElement('canvas');
      cv.width  = meta.w * cs;
      cv.height = tileH  * cs;
      cv.style.display = 'block';
      first = false;
      const ctx = cv.getContext('2d');

      if (cs === 1) {
        const img = ctx.createImageData(meta.w, tileH);
        for (let y = 0; y < tileH; y++) {
          for (let x = 0; x < meta.w; x++) {
            const i = y * meta.w + x;
            const bit = bitAt(bin, i);
            const p = i * 4, v = bit ? 0 : 255;
            img.data[p]=v; img.data[p+1]=v; img.data[p+2]=v; img.data[p+3]=255;
          }
        }
        ctx.putImageData(img, 0, 0);
      } else {
        cv.style.width    = cv.width  + 'px';
        cv.style.height   = cv.height + 'px';
        cv.style.maxWidth = 'none';
        if (borders) {
          const rawBw = (typeof meta.bw === 'number') ? meta.bw : 1;
          const bw = Math.max(0, Math.min(rawBw, cs));
          const ci = Math.max(0, cs - bw);
          ctx.fillStyle = '#d9d3c4';
          ctx.fillRect(0, 0, cv.width, cv.height);
          for (let y = 0; y < tileH; y++) {
            for (let x = 0; x < meta.w; x++) {
              const i = y * meta.w + x;
              const bit = bitAt(bin, i);
              ctx.fillStyle = bit ? '#1a1816' : '#ffffff';
              ctx.fillRect(x*cs+bw, y*cs+bw, ci, ci);
            }
          }
        } else {
          ctx.fillStyle = '#ffffff';
          ctx.fillRect(0, 0, cv.width, cv.height);
          ctx.fillStyle = '#1a1816';
          for (let y = 0; y < tileH; y++) {
            for (let x = 0; x < meta.w; x++) {
              const i = y * meta.w + x;
              if (bitAt(bin, i)) ctx.fillRect(x*cs, y*cs, cs, cs);
            }
          }
        }
      }
      tilesEl.appendChild(cv);
    }
    if (tiles.length > 1) {
      noteEl.textContent = 'Rendered ' + tiles.length + ' stacked canvases (full fidelity, stride 1).';
    }
  }

  function bitAt(bin, i) {
    return (bin.charCodeAt(i >> 3) >> (7 - (i & 7))) & 1;
  }

  function applyCs(v) {
    curCs = Math.max(1, parseInt(v, 10) || 1);
    csRange.value = Math.min(curCs, 16);
    csNum.value = curCs;
    renderCanvas();
  }
  csRange.addEventListener('input',  () => applyCs(csRange.value));
  csNum.addEventListener('change',   () => applyCs(csNum.value));
  bCheck.addEventListener('change',  () => { curBorders = bCheck.checked; renderCanvas(); });

  // ----- Meta panel -----
  function paintMeta(e) {
    // All plain-text values are pre-escaped; the Status cell wraps the
    // escaped value in a trusted <span>, so it stays HTML-safe.
    const cells = [
      ['Rule',          esc(String(e.rule))],
      ['Width',         esc(String(e.width))],
      ['Generations',   esc((e.progress || '0') + ' / ' + e.generations)],
      ['Boundary',      esc(e.boundary)],
      ['Initial cond.', esc(e.ic || '—')],
      ['Padding',       esc((e.fill || '—') + ' ' + (e.align || ''))],
      ['Status',        '<span class="pill done">' + esc(e.status) + '</span>'],
      ['Exported',      esc(e.timestamp)]
    ];
    vMeta.innerHTML = cells.map(([k, v]) =>
      '<div class="meta-cell"><dt>' + esc(k) + '</dt><dd>' + v + '</dd></div>'
    ).join('');
    vTag.textContent = 'RULE ' + String(e.rule).padStart(3, '0');
    // Escape the display name first so user-controlled fields can't inject
    // HTML; only our own <span> tags (wrapping digit groups) are trusted.
    vTitle.innerHTML = esc(displayName(e)).replace(/(\d+)/g, '<span class="num">$1</span>');
    vFile.textContent = e.filename;
    vDims.textContent = e.width + ' × ' + (parseInt(e.generations, 10) + 1) + '  ·  ' + (e.boundary || '').toLowerCase();
    vPre.textContent  = 'Preview · rule ' + e.rule;
  }

  // ----- Load run -----
  function loadRun(e, entryEl) {
    if (activeEl) activeEl.classList.remove('active');
    activeEl = entryEl; entryEl.classList.add('active');
    body.classList.add('has-run');
    tilesEl.innerHTML = ''; noteEl.textContent = '';
    paintMeta(e);

    fetch(e.filename)
      .then(r => { if (!r.ok) throw new Error('not found'); return r.text(); })
      .then(html => {
        const doc = new DOMParser().parseFromString(html, 'text/html');
        const meta = JSON.parse(doc.getElementById('meta').textContent);
        const tiles = (meta.tiles || []).map(t => {
          const el = doc.getElementById(t.id);
          return { bin: atob((el ? el.textContent : '').trim()),
                   tileH: Math.max(0, (t.end || 0) - (t.start || 0)) };
        }).filter(t => t.tileH > 0);
        applyLoaded(meta, tiles);
      })
      .catch(() => {
        const isFile = location.protocol === 'file:';
        const msg = isFile
          ? 'Chrome blocks cross-file access over <code>file://</code>. Use Firefox, or serve locally: <code>python -m http.server</code>'
          : 'Could not load <code>' + esc(e.filename) + '</code>.';
        tilesEl.innerHTML =
          '<div class="err-box">' + msg +
          '<br><br><a href="' + esc(e.filename) + '">Open ' + esc(e.filename) + ' directly &rarr;</a></div>';
      });
  }

  function applyLoaded(meta, tiles) {
    curMeta = meta; curTiles = tiles;
    curCs = meta.cs || 4;
    curBorders = !!meta.borders;
    csRange.value = Math.min(curCs, 16);
    csNum.value = curCs;
    bCheck.checked = curBorders;
    renderCanvas();
  }

  // ----- Manifest TSV / hash deep-link -----
  function parseTsv(text) {
    const lines = text.trim().split('\n'); const out = [];
    for (let i = 1; i < lines.length; i++) {
      const p = lines[i].split('\t');
      if (p.length < 9) continue;
      out.push({ id:p[0], rule:p[1], width:p[2], generations:p[3],
                 boundary:p[4], status:p[5], progress:p[6], timestamp:p[7], filename:p[8],
                 fill:p[9]||'', align:p[10]||'', ic:p[11]||'' });
    }
    return out;
  }

  function autoLoadFromHash() {
    const hash = decodeURIComponent(location.hash.slice(1));
    if (!hash) return;
    // Avoid building a CSS selector by string concatenation — CSS special
    // characters in the hash (e.g. `]`, `\`) would break querySelector and
    // throw. Compare via getAttribute instead.
    const items = listEl.querySelectorAll('[data-filename]');
    const div = Array.from(items).find(el => el.getAttribute('data-filename') === hash);
    if (div) div.click();
  }

  function init() {
    fetch('manifest.tsv')
      .then(r => { if (!r.ok) throw new Error(); return r.text(); })
      .then(text => {
        const entries = parseTsv(text);
        boot(entries);
      })
      .catch(() => {
        try {
          const entries = JSON.parse(document.getElementById('baked').textContent || '[]');
          boot(entries);
        } catch {
          boot([]);
        }
      });
  }

  function boot(entries) {
    if (!entries.length) {
      countEl.textContent = '0';
      return;
    }
    buildSidebar(entries);
    autoLoadFromHash();
  }

  init();
})();
