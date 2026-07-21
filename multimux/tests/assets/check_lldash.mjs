// Headless dash.js low-latency playback check for multimux issue #721
// (multimux/tests/lldash_dashjs.rs is the only caller). Not published --
// test-only harness.
//
// Loads the vendored dash.all.min.js (see dash.all.min.js.NOTICE for
// version/license) directly into a blank Playwright page -- no HTML file or
// static-file server needed, since the ONLY network requests the page makes
// are cross-origin fetches to the real multimux LL-DASH origin the Rust
// caller already started, and that origin sends a permissive
// `Access-Control-Allow-Origin: *` (multimux::origin::add_response_headers)
// on every response, manifest and segment alike.
//
// Usage: node check_lldash.mjs <manifestUrl> <minCurrentTimeSecs> <timeoutMs>
// Prints exactly one JSON object to stdout:
//   {
//     ok: bool,                 // did playback advance far enough with no fatal error and at least one finite live-latency sample?
//     reason: string|null,      // why `ok` is false, when it is
//     currentTime: number,      // video.currentTime at the end of the poll loop
//     liveLatencySamples: number[], // every finite getCurrentLiveLatency() reading observed
//     fatalError: string|null,  // dash.js MediaPlayer ERROR event payload, if any fired
//   }
// Exit code 0 whenever the JSON was produced (`ok` may still be false --
// that's a measured result, not a harness failure); non-zero only if the
// harness itself couldn't run (browser launch failure, etc.) -- the caller
// treats a non-zero exit as "couldn't run", never as "measured failure".

import { chromium } from 'playwright';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const dashjsSource = readFileSync(path.join(here, 'dash.all.min.js'), 'utf8');

const [, , manifestUrl, minCurrentTimeArg, timeoutMsArg] = process.argv;
if (!manifestUrl) {
  console.error('usage: node check_lldash.mjs <manifestUrl> <minCurrentTimeSecs> <timeoutMs>');
  process.exit(2);
}
const minCurrentTime = Number(minCurrentTimeArg ?? '2.0');
const timeoutMs = Number(timeoutMsArg ?? '20000');
const pollIntervalMs = 200;

const browser = await chromium.launch();
try {
  const page = await browser.newPage();
  page.on('console', (msg) => {
    // Surface page console errors on our own stderr for debugging a failed
    // run -- never parsed, never affects the JSON result.
    if (msg.type() === 'error') {
      process.stderr.write(`[page console] ${msg.text()}\n`);
    }
  });

  await page.setContent('<!doctype html><html><body><video id="v" muted playsinline></video></body></html>');
  await page.addScriptTag({ content: dashjsSource });

  await page.evaluate((url) => {
    const video = document.getElementById('v');
    video.muted = true;
    const player = dashjs.MediaPlayer().create();
    // Tuned low-latency catch-up: dash.js 5.x auto-detects low-latency mode
    // from the MPD's own <ServiceDescription>/<Latency> signalling
    // (`applyServiceDescription`, default true) rather than an explicit
    // `lowLatencyEnabled` flag (removed since dash.js v3/v4) -- our
        // LlDashPackager-built MPD carries that element, so no override is
    // needed here beyond tuning the target delay/catch-up aggressiveness.
    player.updateSettings({
      streaming: {
        delay: { liveDelay: 0.6 },
        liveCatchup: { enabled: true, mode: 'liveCatchupModeDefault' },
      },
    });
    window.__probe = { fatalError: null };
    player.on(dashjs.MediaPlayer.events.ERROR, (e) => {
      window.__probe.fatalError = (e && e.error && (e.error.message || JSON.stringify(e.error))) || JSON.stringify(e);
    });
    window.__player = player;
    window.__video = video;
    player.initialize(video, url, true);
  }, manifestUrl);

  const liveLatencySamples = [];
  let last = { currentTime: 0, fatalError: null, liveLatency: null };
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    last = await page.evaluate(() => {
      const v = window.__video;
      const p = window.__player;
      let liveLatency = null;
      try {
        const l = p.getCurrentLiveLatency();
        if (typeof l === 'number' && Number.isFinite(l)) liveLatency = l;
      } catch {
        // getCurrentLiveLatency throws before the stream is initialized --
        // not yet a sample.
      }
      return {
        currentTime: v ? v.currentTime : 0,
        fatalError: window.__probe ? window.__probe.fatalError : null,
        liveLatency,
      };
    });
    if (last.liveLatency !== null) liveLatencySamples.push(last.liveLatency);
    if (last.fatalError) break;
    if (last.currentTime >= minCurrentTime && liveLatencySamples.length > 0) break;
    await new Promise((r) => setTimeout(r, pollIntervalMs));
  }

  let ok = true;
  let reason = null;
  if (last.fatalError) {
    ok = false;
    reason = `dash.js fatal ERROR event: ${last.fatalError}`;
  } else if (last.currentTime < minCurrentTime) {
    ok = false;
    reason = `currentTime only reached ${last.currentTime}s (< required ${minCurrentTime}s) within ${timeoutMs}ms`;
  } else if (liveLatencySamples.length === 0) {
    ok = false;
    reason = 'no finite getCurrentLiveLatency() sample observed';
  }

  console.log(JSON.stringify({
    ok,
    reason,
    currentTime: last.currentTime,
    liveLatencySamples,
    fatalError: last.fatalError,
  }));
} finally {
  await browser.close();
}
