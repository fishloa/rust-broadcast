// rust-dvb WASM demo — vanilla JS, no framework
// Loads the wasm-pack `--target web` output from ./pkg/dvb_demo.js
// and drives the UI.

import init, { parse_ts } from './pkg/dvb_demo.js';

// ── Init ────────────────────────────────────────────────────────────────────

let wasmReady = false;

(async () => {
  try {
    await init();
    wasmReady = true;
  } catch (e) {
    showError(`Failed to load WASM module: ${e}`);
  }
})();

// ── DOM refs ─────────────────────────────────────────────────────────────────

const dropzone   = document.getElementById('dropzone');
const fileInput  = document.getElementById('file-input');
const loading    = document.getElementById('loading');
const errBanner  = document.getElementById('error-banner');

const statsSection   = document.getElementById('stats-section');
const statsList      = document.getElementById('stats-list');

const servicesSection = document.getElementById('services-section');
const servicesBody    = document.getElementById('services-body');

const tablesSection  = document.getElementById('tables-section');
const tablesBody     = document.getElementById('tables-body');

const jsonSection    = document.getElementById('json-section');
const jsonPre        = document.getElementById('json-pre');
const jsonToggle     = document.getElementById('json-toggle');

// ── Drop zone events ─────────────────────────────────────────────────────────

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

fileInput.addEventListener('change', () => {
  const file = fileInput.files?.[0];
  if (file) processFile(file);
});

// ── Raw JSON toggle ───────────────────────────────────────────────────────────

jsonToggle.addEventListener('click', () => {
  const expanded = jsonToggle.getAttribute('aria-expanded') === 'true';
  jsonToggle.setAttribute('aria-expanded', String(!expanded));
  jsonToggle.textContent = expanded ? 'Show' : 'Hide';
  jsonPre.classList.toggle('hidden', expanded);
});

// ── File processing ───────────────────────────────────────────────────────────

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

    const jsonStr = parse_ts(bytes);
    const result  = JSON.parse(jsonStr);

    renderStats(result, file.name, bytes.length);
    renderServices(result.services ?? []);
    renderTables(result.tables ?? []);
    renderJson(jsonStr);
  } catch (e) {
    showError(`Parse failed: ${e}`);
  } finally {
    loading.classList.add('hidden');
  }
}

// ── Renderers ─────────────────────────────────────────────────────────────────

function renderStats(result, filename, byteLen) {
  const tables    = result.tables   ?? [];
  const services  = result.services ?? [];

  // Count unique table names from the section key (the camelCase variant name).
  const byKind = new Map();
  for (const entry of tables) {
    const kind = Object.keys(entry.section)[0] ?? 'unknown';
    byKind.set(kind, (byKind.get(kind) ?? 0) + 1);
  }

  statsList.innerHTML = '';
  const items = [
    ['File',           filename],
    ['Size',           formatBytes(byteLen)],
    ['Packets fed',    result.packets_fed ?? '—'],
    ['Sections',       tables.length],
    ['Services',       services.length],
    ['CRC errors',     result.crc_errors ?? 0],
    ['Parse errors',   result.parse_errors ?? 0],
  ];
  for (const [label, value] of items) {
    const div = document.createElement('div');
    div.innerHTML = `<dt>${esc(String(label))}</dt><dd>${esc(String(value))}</dd>`;
    statsList.appendChild(div);
  }
  statsSection.classList.remove('hidden');
}

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
    const tr   = document.createElement('tr');
    const prettySection = JSON.stringify(entry.section[kind] ?? entry.section, null, 2);

    tr.innerHTML = `
      <td>${esc(String(entry.pid))}</td>
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

function renderJson(jsonStr) {
  try {
    jsonPre.textContent = JSON.stringify(JSON.parse(jsonStr), null, 2);
  } catch {
    jsonPre.textContent = jsonStr;
  }
  jsonSection.classList.remove('hidden');
  // Reset toggle to collapsed state on new parse.
  jsonToggle.setAttribute('aria-expanded', 'false');
  jsonToggle.textContent = 'Show';
  jsonPre.classList.add('hidden');
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function showError(msg) {
  errBanner.textContent = msg;
  errBanner.classList.remove('hidden');
}

function hideError() {
  errBanner.textContent = '';
  errBanner.classList.add('hidden');
}

function clearResults() {
  for (const el of [statsSection, servicesSection, tablesSection, jsonSection]) {
    el.classList.add('hidden');
  }
  statsList.innerHTML = '';
  servicesBody.innerHTML = '';
  tablesBody.innerHTML = '';
  jsonPre.textContent = '';
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
