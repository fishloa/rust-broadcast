// SIGNAL — rust-broadcast WASM analyzer. Vanilla JS, no framework, no CDN.
// Loads the wasm-pack `--target web` output from ./pkg/dvb_demo.js and
// drives every panel from the single JSON object `analyze()` returns.

import init, { analyze } from './pkg/dvb_demo.js';

// Palette mirrored from style.css custom properties — canvas 2D needs literal
// color strings, it cannot read CSS custom properties directly.
const COLOR = {
  green: '#4ade80',
  teal: '#34d9c4',
  amber: '#f5b942',
  red: '#ff5c5c',
  blue: '#6ea8ff',
  magenta: '#ff6ec7',
  grid: 'rgba(120, 200, 170, 0.14)',
  zero: 'rgba(245, 185, 66, 0.35)',
  axisText: 'rgba(140, 220, 190, 0.55)',
  emptyText: 'rgba(140, 220, 190, 0.35)',
  legendText: 'rgba(215, 236, 223, 0.8)',
};

// ── Init ─────────────────────────────────────────────────────────────────

let wasmReady = false;

(async () => {
  try {
    await init();
    wasmReady = true;
  } catch (e) {
    showError(`Failed to load WASM module: ${e}`);
  }
})();

// ── DOM refs ─────────────────────────────────────────────────────────────

const dropzone  = document.getElementById('dropzone');
const fileInput = document.getElementById('file-input');
const loading   = document.getElementById('loading');
const errBanner = document.getElementById('error-banner');
const results   = document.getElementById('results');
const brandLed  = document.getElementById('brand-led');

const statsList = document.getElementById('stats-list');

const pidmapSection = document.getElementById('pidmap-section');
const pidmapBody    = document.getElementById('pidmap-body');

const timingSection   = document.getElementById('timing-section');
const pcrCanvas       = document.getElementById('pcr-canvas');
const pcrChartLabel   = document.getElementById('pcr-chart-label');
const driftCanvas     = document.getElementById('drift-canvas');
const driftChartLabel = document.getElementById('drift-chart-label');
const timingNote      = document.getElementById('timing-note');

const servicesSection = document.getElementById('services-section');
const servicesBody    = document.getElementById('services-body');

const tablesSection = document.getElementById('tables-section');
const tablesBody    = document.getElementById('tables-body');

const conformanceSection = document.getElementById('conformance-section');
const conformanceStats   = document.getElementById('conformance-stats');
const conformanceGroups  = document.getElementById('conformance-groups');

const scte35Section  = document.getElementById('scte35-section');
const scte35Timeline = document.getElementById('scte35-timeline');
const scte35Note     = document.getElementById('scte35-note');

const jsonSection = document.getElementById('json-section');
const jsonPre     = document.getElementById('json-pre');
const jsonToggle  = document.getElementById('json-toggle');

// ── Drop zone events ───────────────────────────────────────────────────────

dropzone.addEventListener('dragover', e => {
  e.preventDefault();
  dropzone.classList.add('drag-over');
});

dropzone.addEventListener('dragleave', () => {
  dropzone.classList.remove('drag-over');
});

dropzone.addEventListener('drop', e => {
  e.preventDefault();
  dropzone.classList.remove('drag-over');
  const file = e.dataTransfer?.files?.[0];
  if (file) processFile(file);
});

dropzone.addEventListener('keydown', e => {
  if (e.key === 'Enter' || e.key === ' ') {
    e.preventDefault();
    fileInput.click();
  }
});

fileInput.addEventListener('change', () => {
  const file = fileInput.files?.[0];
  if (file) processFile(file);
});

// ── Raw JSON toggle ────────────────────────────────────────────────────────

jsonToggle.addEventListener('click', () => {
  const expanded = jsonToggle.getAttribute('aria-expanded') === 'true';
  jsonToggle.setAttribute('aria-expanded', String(!expanded));
  jsonToggle.textContent = expanded ? 'show' : 'hide';
  jsonPre.classList.toggle('hidden', expanded);
});

// Re-draw the charts on resize — the backing store is sized from the
// container's current pixel width for crisp HiDPI rendering.
let resizeHandle = null;
window.addEventListener('resize', () => {
  if (!lastTiming) return;
  clearTimeout(resizeHandle);
  resizeHandle = setTimeout(() => renderTiming(lastTiming), 120);
});
let lastTiming = null;

// ── File processing ────────────────────────────────────────────────────────

async function processFile(file) {
  if (!wasmReady) {
    showError('WASM module is not ready yet — please wait a moment and try again.');
    return;
  }

  hideError();
  clearResults();
  loading.classList.remove('hidden');

  try {
    const arrayBuf = await file.arrayBuffer();
    const bytes = new Uint8Array(arrayBuf);

    // Run on next tick so the browser can paint the loading indicator.
    await new Promise(r => setTimeout(r, 0));

    const jsonStr = analyze(bytes);
    const result = JSON.parse(jsonStr);

    results.classList.remove('hidden');
    renderStats(result, file.name, bytes.length);
    renderPidMap(result.pid_map ?? []);
    renderServices(result.services ?? []);
    renderTables(result.tables ?? []);
    renderConformance(result.conformance);
    renderScte35(result.scte35);
    lastTiming = result.timing;
    timingSection.classList.remove('hidden');
    renderTiming(result.timing);
    renderJson(jsonStr);
  } catch (e) {
    results.classList.add('hidden');
    showError(`Analysis failed: ${e}`);
  } finally {
    loading.classList.add('hidden');
  }
}

// ── Summary ────────────────────────────────────────────────────────────────

function renderStats(result, filename, byteLen) {
  const tables = result.tables ?? [];
  const services = result.services ?? [];
  const scte35Count = result.scte35?.events?.length ?? 0;
  const parseErrors = result.parse_errors ?? 0;
  const crcErrors = result.crc_errors ?? 0;

  statsList.innerHTML = '';
  const items = [
    ['File', filename],
    ['Size', formatBytes(byteLen)],
    ['Packets', result.packets_fed ?? 0],
    ['PIDs', (result.pid_map ?? []).length],
    ['SI sections', tables.length],
    ['Services', services.length],
    ['SCTE-35', scte35Count],
    ['Parse err', parseErrors],
    ['CRC err', crcErrors],
  ];
  for (const [label, value] of items) {
    const isErr = (label === 'Parse err' || label === 'CRC err') && Number(value) > 0;
    const div = document.createElement('div');
    div.innerHTML = `<dt>${esc(label)}</dt><dd${isErr ? ' class="warn"' : ''}>${esc(String(value))}</dd>`;
    statsList.appendChild(div);
  }
}

// ── PID map ──────────────────────────────────────────────────────────────

function pidHex(pid) {
  return '0x' + pid.toString(16).toUpperCase().padStart(4, '0');
}

function badgeClassFor(kind) {
  return 'badge-' + kind.toLowerCase().replace(/[^a-z0-9]+/g, '-');
}

function renderPidMap(pidMap) {
  pidmapBody.innerHTML = '';
  if (pidMap.length === 0) {
    pidmapSection.classList.add('hidden');
    return;
  }
  for (const e of pidMap) {
    const tr = document.createElement('tr');
    tr.innerHTML = `
      <td class="pid-cell">${esc(String(e.pid))} <span class="pid-hex">${esc(pidHex(e.pid))}</span></td>
      <td><span class="badge ${badgeClassFor(e.kind)}">${esc(e.kind)}</span></td>
      <td>${e.stream_type ? esc(e.stream_type) : '—'}</td>
      <td class="num">${esc(String(e.packets))}</td>
      <td>${e.has_pcr ? '<span class="dot-on" title="carries PCR"></span>' : '<span class="dot-off">—</span>'}</td>
    `;
    pidmapBody.appendChild(tr);
  }
  pidmapSection.classList.remove('hidden');
}

// ── Services / SI tables (unchanged shape, restyled) ──────────────────────

function renderServices(services) {
  servicesBody.innerHTML = '';
  if (services.length === 0) {
    servicesSection.classList.add('hidden');
    return;
  }
  for (const svc of services) {
    const tr = document.createElement('tr');
    tr.innerHTML = `
      <td>${esc(String(svc.service_id))}</td>
      <td>${esc(svc.service_name || '—')}</td>
      <td>${esc(svc.provider_name || '—')}</td>
      <td>${esc(svc.service_type || '—')}</td>
    `;
    servicesBody.appendChild(tr);
  }
  servicesSection.classList.remove('hidden');
}

function renderTables(tables) {
  tablesBody.innerHTML = '';
  if (tables.length === 0) {
    tablesSection.classList.add('hidden');
    return;
  }
  for (const entry of tables) {
    const kind = Object.keys(entry.section)[0] ?? 'unknown';
    const tr = document.createElement('tr');
    const prettySection = JSON.stringify(entry.section[kind] ?? entry.section, null, 2);

    tr.innerHTML = `
      <td class="pid-cell">${esc(String(entry.pid))} <span class="pid-hex">${esc(pidHex(entry.pid))}</span></td>
      <td><code>${esc(kind)}</code></td>
      <td class="json-cell">
        <details>
          <summary>expand</summary>
          <pre>${esc(prettySection)}</pre>
        </details>
      </td>
    `;
    tablesBody.appendChild(tr);
  }
  tablesSection.classList.remove('hidden');
}

// ── TR 101 290 conformance ─────────────────────────────────────────────────

function renderConformance(conf) {
  conformanceStats.innerHTML = '';
  conformanceGroups.innerHTML = '';

  const inSync = !!conf?.stats?.in_sync;
  brandLed.classList.toggle('led-lost', !inSync);

  const items = [
    ['Packets', conf?.stats?.packets ?? 0],
    ['Events', conf?.stats?.events ?? 0],
    ['Sync', inSync ? 'LOCKED' : 'LOST'],
  ];
  for (const [label, value] of items) {
    const isWarn = label === 'Sync' && !inSync;
    const div = document.createElement('div');
    div.innerHTML = `<dt>${esc(label)}</dt><dd${isWarn ? ' class="warn"' : ''}>${esc(String(value))}</dd>`;
    conformanceStats.appendChild(div);
  }

  const groups = conf?.by_priority ?? [];
  if (groups.length === 0) {
    conformanceGroups.innerHTML =
      '<p class="conformance-clean">No TR 101 290 indicators raised for this capture.</p>';
    conformanceSection.classList.remove('hidden');
    return;
  }

  for (const group of groups) {
    const priorityWord = group.priority.split(' ')[0].toLowerCase();
    const wrap = document.createElement('div');
    wrap.className = 'priority-group';

    const heading = document.createElement('div');
    heading.className = `priority-heading priority-${priorityWord}`;
    heading.innerHTML = `<span class="dot-on"></span>${esc(group.priority)} (${group.indicators.length})`;
    wrap.appendChild(heading);

    for (const ind of group.indicators) {
      const row = document.createElement('div');
      row.className = 'indicator-row';
      const pids = (ind.pids ?? []).map(p => `${p} (${pidHex(p)})`).join(', ');
      const details = (ind.sample_details ?? []).map(d => `<div>${esc(d)}</div>`).join('');
      row.innerHTML = `
        <div class="indicator-name">${esc(ind.indicator)}</div>
        <div class="indicator-count">${esc(String(ind.count))}</div>
        <div class="indicator-clause">${esc(ind.clause)}</div>
        ${pids ? `<div class="indicator-pids">PIDs: ${esc(pids)}</div>` : ''}
        ${details ? `<div class="indicator-detail">${details}</div>` : ''}
      `;
      wrap.appendChild(row);
    }
    conformanceGroups.appendChild(wrap);
  }
  conformanceSection.classList.remove('hidden');
}

// ── SCTE-35 splice timeline ────────────────────────────────────────────────

function renderScte35(report) {
  scte35Timeline.innerHTML = '';
  scte35Note.classList.add('hidden');

  const events = report?.events ?? [];
  if (events.length === 0) {
    scte35Section.classList.add('hidden');
    return;
  }

  for (const ev of events) {
    const clear = ev.section?.clear;
    const cmdObj = clear?.command;
    const cmdKey = cmdObj ? Object.keys(cmdObj)[0] : 'encrypted';
    const cmdBody = cmdObj ? cmdObj[cmdKey] : ev.section;

    const li = document.createElement('li');
    li.innerHTML = `
      <div class="timeline-head">
        <span class="timeline-cmd">${esc(cmdKey)}</span>
        <span class="timeline-meta">PID ${esc(String(ev.pid))} (${esc(pidHex(ev.pid))}) &middot; packet #${esc(String(ev.packet_index))}</span>
      </div>
      <details class="timeline-fields">
        <summary>fields</summary>
        <pre>${esc(JSON.stringify(cmdBody, null, 2))}</pre>
      </details>
    `;
    scte35Timeline.appendChild(li);
  }

  const noteBits = [];
  if (report?.truncated) noteBits.push('timeline truncated at the collection cap');
  if (report?.parse_errors) noteBits.push(`${report.parse_errors} section(s) failed to parse`);
  if (noteBits.length) {
    scte35Note.textContent = noteBits.join(' · ');
    scte35Note.classList.remove('hidden');
  }

  scte35Section.classList.remove('hidden');
}

// ── Timing charts (canvas 2D, no libraries) ────────────────────────────────

function groupBy(arr, keyFn) {
  const m = new Map();
  for (const item of arr) {
    const k = keyFn(item);
    if (!m.has(k)) m.set(k, []);
    m.get(k).push(item);
  }
  return m;
}

function dominantKey(map) {
  let bestKey = null;
  let bestLen = -1;
  for (const [k, v] of map) {
    if (v.length > bestLen) {
      bestLen = v.length;
      bestKey = k;
    }
  }
  return bestKey;
}

// Decimal places are chosen from the axis *step* (the gap between adjacent
// gridlines), not each value's own magnitude — otherwise a narrow-range axis
// around e.g. 100ms renders every gridline as the identical "100.0".
function axisDecimals(step) {
  if (step >= 10) return 0;
  if (step >= 1) return 1;
  if (step >= 0.1) return 2;
  return 3;
}

function formatAxisNumber(v, decimals) {
  return v.toFixed(decimals);
}

/** Prepare a canvas's backing store for crisp HiDPI drawing. CSS `width:100%;
 * height:auto` on the element derives the displayed size from this
 * intrinsic aspect ratio — only the `width`/`height` IDL attributes (the
 * canvas coordinate system) are touched here, never `style`. */
function setupCanvas(canvas, cssHeight) {
  const cssWidth = canvas.parentElement.clientWidth || 320;
  const dpr = window.devicePixelRatio || 1;
  canvas.width = Math.max(1, Math.round(cssWidth * dpr));
  canvas.height = Math.max(1, Math.round(cssHeight * dpr));
  const ctx = canvas.getContext('2d');
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  return { ctx, width: cssWidth, height: cssHeight };
}

function drawEmptyMessage(canvas, message) {
  const { ctx, width, height } = setupCanvas(canvas, 180);
  ctx.clearRect(0, 0, width, height);
  ctx.fillStyle = COLOR.emptyText;
  ctx.font = '12px ui-monospace, monospace';
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.fillText(message, width / 2, height / 2);
}

/** series: [{ label, color, points: [{x, y}] }] */
function drawLineChart(canvas, series) {
  const { ctx, width, height } = setupCanvas(canvas, 180);
  ctx.clearRect(0, 0, width, height);

  const allPoints = series.flatMap(s => s.points);
  if (allPoints.length === 0) {
    drawEmptyMessage(canvas, 'no data');
    return;
  }

  const padR = 12;
  const padT = 14;
  const padB = 18;
  const gridRows = 4;

  let xMin = Math.min(...allPoints.map(p => p.x));
  let xMax = Math.max(...allPoints.map(p => p.x));
  let yMin = Math.min(...allPoints.map(p => p.y));
  let yMax = Math.max(...allPoints.map(p => p.y));
  if (xMin === xMax) xMax = xMin + 1;
  if (yMin === yMax) {
    yMin -= 1;
    yMax += 1;
  }
  const yPad = (yMax - yMin) * 0.12;
  yMin -= yPad;
  yMax += yPad;

  // Decimal places come from the axis *step*; the left gutter below is sized
  // from the actual rendered label width so extra decimals never clip.
  const yStep = (yMax - yMin) / gridRows;
  const decimals = axisDecimals(yStep);
  const yLabels = [];
  for (let i = 0; i <= gridRows; i++) {
    yLabels.push(formatAxisNumber(yMax - yStep * i, decimals));
  }

  // Size the left gutter from the widest label actually being drawn, so
  // labels are never clipped against the plot area.
  ctx.font = '10px ui-monospace, monospace';
  const maxLabelWidth = Math.max(...yLabels.map(l => ctx.measureText(l).width));
  const padL = Math.max(34, Math.ceil(maxLabelWidth) + 14);

  const plotW = width - padL - padR;
  const plotH = height - padT - padB;

  const xToPx = x => padL + ((x - xMin) / (xMax - xMin)) * plotW;
  const yToPx = y => padT + plotH - ((y - yMin) / (yMax - yMin)) * plotH;

  // Grid.
  ctx.strokeStyle = COLOR.grid;
  ctx.lineWidth = 1;
  for (let i = 0; i <= gridRows; i++) {
    const y = padT + (plotH / gridRows) * i;
    ctx.beginPath();
    ctx.moveTo(padL, Math.round(y) + 0.5);
    ctx.lineTo(padL + plotW, Math.round(y) + 0.5);
    ctx.stroke();
  }

  // Zero reference line, when in range (drift can be negative).
  if (yMin < 0 && yMax > 0) {
    ctx.strokeStyle = COLOR.zero;
    const y = Math.round(yToPx(0)) + 0.5;
    ctx.beginPath();
    ctx.moveTo(padL, y);
    ctx.lineTo(padL + plotW, y);
    ctx.stroke();
  }

  // Y-axis labels.
  ctx.fillStyle = COLOR.axisText;
  ctx.font = '10px ui-monospace, monospace';
  ctx.textAlign = 'right';
  ctx.textBaseline = 'middle';
  for (let i = 0; i <= gridRows; i++) {
    const y = padT + (plotH / gridRows) * i;
    ctx.fillText(yLabels[i], padL - 6, y);
  }

  // Series traces.
  for (const s of series) {
    if (s.points.length === 0) continue;
    ctx.strokeStyle = s.color;
    ctx.lineWidth = 1.4;
    ctx.beginPath();
    s.points.forEach((p, i) => {
      const px = xToPx(p.x);
      const py = yToPx(p.y);
      if (i === 0) ctx.moveTo(px, py);
      else ctx.lineTo(px, py);
    });
    ctx.stroke();
  }

  // Legend (only when there's more than one series to disambiguate).
  if (series.length > 1) {
    let lx = padL;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'alphabetic';
    ctx.font = '10px ui-monospace, monospace';
    for (const s of series) {
      ctx.fillStyle = s.color;
      ctx.fillRect(lx, padT - 10, 8, 8);
      ctx.fillStyle = COLOR.legendText;
      ctx.fillText(s.label, lx + 11, padT - 2);
      lx += ctx.measureText(s.label).width + 11 + 16;
    }
  }
}

function renderTiming(timing) {
  timingNote.classList.add('hidden');
  if (!timing) {
    drawEmptyMessage(pcrCanvas, 'no data');
    drawEmptyMessage(driftCanvas, 'no data');
    return;
  }

  // PCR interval, for whichever PID carried the most PCR samples.
  const pcrByPid = groupBy(timing.pcr_samples ?? [], s => s.pid);
  const pcrPid = dominantKey(pcrByPid);
  if (pcrPid == null) {
    pcrChartLabel.textContent = 'PCR interval — no PCR observed';
    drawEmptyMessage(pcrCanvas, 'no PCR observed');
  } else {
    const samples = pcrByPid.get(pcrPid).slice().sort((a, b) => a.packet_index - b.packet_index);
    const points = [];
    for (let i = 1; i < samples.length; i++) {
      points.push({
        x: samples[i].packet_index,
        y: (samples[i].seconds - samples[i - 1].seconds) * 1000,
      });
    }
    pcrChartLabel.textContent = `PCR interval (ms) — PID ${pcrPid} (${pidHex(pcrPid)})`;
    drawLineChart(pcrCanvas, [{ label: 'interval', color: COLOR.green, points }]);
  }

  // PTS/DTS drift, for whichever PID carried the most timestamp samples.
  const tsByPid = groupBy(timing.pts_samples ?? [], s => s.pid);
  const tsPid = dominantKey(tsByPid);
  if (tsPid == null) {
    driftChartLabel.textContent = 'PTS / DTS drift — no PTS/DTS observed';
    drawEmptyMessage(driftCanvas, 'no PTS/DTS observed');
  } else {
    const samples = tsByPid.get(tsPid).slice().sort((a, b) => a.packet_index - b.packet_index);
    const ptsPoints = samples
      .filter(s => s.kind === 'pts')
      .map(s => ({ x: s.packet_index, y: s.drift_seconds * 1000 }));
    const dtsPoints = samples
      .filter(s => s.kind === 'dts')
      .map(s => ({ x: s.packet_index, y: s.drift_seconds * 1000 }));
    driftChartLabel.textContent = `PTS / DTS drift (ms) — PID ${tsPid} (${pidHex(tsPid)})`;
    drawLineChart(driftCanvas, [
      { label: 'pts', color: COLOR.green, points: ptsPoints },
      { label: 'dts', color: COLOR.amber, points: dtsPoints },
    ]);
  }

  const noteBits = [];
  if (timing.truncated) noteBits.push('sample collection truncated at the cap');
  if (timing.pes_parse_errors) noteBits.push(`${timing.pes_parse_errors} PES packet(s) failed to parse`);
  if (noteBits.length) {
    timingNote.textContent = noteBits.join(' · ');
    timingNote.classList.remove('hidden');
  }
}

// ── Raw JSON ───────────────────────────────────────────────────────────────

function renderJson(jsonStr) {
  try {
    jsonPre.textContent = JSON.stringify(JSON.parse(jsonStr), null, 2);
  } catch {
    jsonPre.textContent = jsonStr;
  }
  jsonSection.classList.remove('hidden');
  jsonToggle.setAttribute('aria-expanded', 'false');
  jsonToggle.textContent = 'show';
  jsonPre.classList.add('hidden');
}

// ── Helpers ────────────────────────────────────────────────────────────────

function showError(msg) {
  errBanner.textContent = msg;
  errBanner.classList.remove('hidden');
}

function hideError() {
  errBanner.textContent = '';
  errBanner.classList.add('hidden');
}

function clearResults() {
  results.classList.add('hidden');
  for (const el of [pidmapSection, timingSection, servicesSection, tablesSection, conformanceSection, scte35Section]) {
    el.classList.add('hidden');
  }
  statsList.innerHTML = '';
  pidmapBody.innerHTML = '';
  servicesBody.innerHTML = '';
  tablesBody.innerHTML = '';
  conformanceStats.innerHTML = '';
  conformanceGroups.innerHTML = '';
  scte35Timeline.innerHTML = '';
  jsonPre.textContent = '';
  brandLed.classList.remove('led-lost');
  lastTiming = null;
}

function formatBytes(n) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(2)} MB`;
}

function esc(str) {
  return str
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}
