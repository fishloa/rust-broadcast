// Headless-Chrome WHIP publish harness for multimux issue #740
// (multimux/tests/whip_ingest.rs). Not published; test-only.
//
// Drives a real `RTCPeerConnection` (fake video capture device, forced to
// H.264 so it stays within `multimux::source::whip`'s documented "video
// only" scope -- see that module's doc for why Opus audio is out of scope),
// completes vanilla (non-trickle) ICE gathering, POSTs the SDP offer to a
// real WHIP endpoint, applies the answer, then holds the connection open for
// `holdMs` so the server has real time to segment the media -- proving an
// actual browser's SRTP-encrypted RTP was decrypted and depayloaded by this
// workspace's own code, not a mocked peer agreeing with itself.
//
// Usage: node whip_publish.mjs <whipUrl> <connectTimeoutMs> <holdMs>
// Prints one JSON object to stdout: { ok, connectionState, bytesSent, error }.

import { chromium } from 'playwright';
import http from 'node:http';

const [, , whipUrl, connectTimeoutMsArg, holdMsArg] = process.argv;
const connectTimeoutMs = parseInt(connectTimeoutMsArg, 10);
const holdMs = parseInt(holdMsArg, 10);

// `navigator.mediaDevices` is only exposed in a secure context: an
// `about:blank` page (Playwright's default) has an opaque origin and does
// NOT count, even though it's on the "trustworthy" list for other purposes.
// `http://127.0.0.1` *is* a secure context (loopback is trustworthy per the
// W3C Secure Contexts spec) without needing TLS, so this spins up a trivial
// local page to navigate to first -- no relation to the WHIP server itself.
const blankPageServer = http.createServer((_req, res) => {
  res.writeHead(200, { 'Content-Type': 'text/html' });
  res.end('<!doctype html><title>whip harness</title>');
});
await new Promise((resolve) => blankPageServer.listen(0, '127.0.0.1', resolve));
const blankPageUrl = `http://127.0.0.1:${blankPageServer.address().port}/`;

const browser = await chromium.launch({
  headless: true,
  args: [
    '--use-fake-ui-for-media-stream',
    '--use-fake-device-for-media-stream',
    '--disable-webrtc-hw-encoding',
    '--no-sandbox',
  ],
});

let result;
try {
  const page = await browser.newPage();
  page.on('console', (msg) => console.error('[page]', msg.text()));
  page.on('pageerror', (err) => console.error('[pageerror]', err));
  await page.goto(blankPageUrl);

  result = await page.evaluate(
    async ({ whipUrl, connectTimeoutMs, holdMs }) => {
      try {
        const stream = await navigator.mediaDevices.getUserMedia({ video: true });
        const track = stream.getVideoTracks()[0];

        const pc = new RTCPeerConnection({ iceServers: [] });
        const transceiver = pc.addTransceiver(track, { direction: 'sendonly' });

        const caps = RTCRtpSender.getCapabilities('video');
        const h264 = (caps?.codecs ?? []).filter((c) => /H264/i.test(c.mimeType));
        if (h264.length === 0) {
          return { ok: false, error: 'browser reports no H264 video codec capability' };
        }
        transceiver.setCodecPreferences(h264);

        const offer = await pc.createOffer();
        await pc.setLocalDescription(offer);

        // Vanilla (non-trickle) ICE: wait for gathering to finish so the
        // offer POSTed below already carries every candidate -- the server
        // side (`multimux::source::whip`) implements no trickle-ICE PATCH.
        await new Promise((resolve) => {
          if (pc.iceGatheringState === 'complete') {
            resolve();
            return;
          }
          const onChange = () => {
            if (pc.iceGatheringState === 'complete') {
              pc.removeEventListener('icegatheringstatechange', onChange);
              resolve();
            }
          };
          pc.addEventListener('icegatheringstatechange', onChange);
        });

        const resp = await fetch(whipUrl, {
          method: 'POST',
          headers: { 'Content-Type': 'application/sdp' },
          body: pc.localDescription.sdp,
        });
        if (!resp.ok) {
          return {
            ok: false,
            error: `WHIP POST failed: ${resp.status} ${await resp.text()}`,
          };
        }
        const answerSdp = await resp.text();
        await pc.setRemoteDescription({ type: 'answer', sdp: answerSdp });

        const deadline = Date.now() + connectTimeoutMs;
        let connectionState = pc.connectionState;
        let bytesSent = 0;
        while (Date.now() < deadline) {
          connectionState = pc.connectionState;
          if (connectionState === 'connected') break;
          if (connectionState === 'failed' || connectionState === 'closed') break;
          await new Promise((r) => setTimeout(r, 100));
        }
        if (connectionState !== 'connected') {
          return { ok: false, connectionState, error: 'never reached connectionState=connected' };
        }

        // Held open so the server-side segmenter has real time to close a
        // segment before this test asks for one.
        const holdDeadline = Date.now() + holdMs;
        while (Date.now() < holdDeadline) {
          const stats = await pc.getStats();
          stats.forEach((report) => {
            if (report.type === 'outbound-rtp' && report.kind === 'video') {
              bytesSent = report.bytesSent ?? bytesSent;
            }
          });
          await new Promise((r) => setTimeout(r, 200));
        }

        return { ok: bytesSent > 0, connectionState, bytesSent };
      } catch (e) {
        return { ok: false, error: String((e && e.stack) || e) };
      }
    },
    { whipUrl, connectTimeoutMs, holdMs },
  );
} finally {
  await browser.close();
  blankPageServer.close();
}

console.log(JSON.stringify(result));
