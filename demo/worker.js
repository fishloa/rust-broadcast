// SIGNAL — dedicated Web Worker running the WASM `analyze()` call off the
// main thread (GitHub issue #928: "handle a large file without locking the
// UI thread"). `analyze()` is a single synchronous, CPU-bound call (the
// whole point of shipping one WASM function rather than a chunked API —
// every crate underneath already streams the input packet-by-packet), so
// moving *where* it runs, rather than *how* it runs, is what keeps the tab
// scrollable/interactive while a large capture is being analyzed. A module
// worker can `import` the wasm-bindgen `--target web` output exactly like the
// main thread does.

import init, { analyze } from './pkg/dvb_demo.js';

let wasmReady = false;
let initError = null;

const readyPromise = init()
  .then(() => {
    wasmReady = true;
  })
  .catch(e => {
    initError = String(e);
  });

self.onmessage = async event => {
  const { bytes } = event.data;
  await readyPromise;

  if (!wasmReady) {
    self.postMessage({ error: `Failed to load WASM module: ${initError}` });
    return;
  }

  try {
    const json = analyze(new Uint8Array(bytes));
    self.postMessage({ json });
  } catch (e) {
    self.postMessage({ error: `Analysis failed: ${e}` });
  }
};
