// Headless-Chrome WHIP-publish + WHEP-playback harness for multimux issue
// #743 (multimux/tests/whep_egress.rs). Not published; test-only.
//
// Drives TWO real `RTCPeerConnection`s in one headless browser page: one
// publishes a fake-video-device H.264 capture into multimux over WHIP
// (mirrors `whip_publish.mjs`), the other is a genuine WHEP *viewer* that
// POSTs an SDP offer to multimux's WHEP endpoint, applies the answer,
// attaches the received track to a real `<video>` element, and proves
// actual decode (not just packet receipt) by polling both
// `RTCRtpReceiver` stats (`framesDecoded`) and the `<video>` element's own
// `currentTime` advancing -- the same real-browser-decode bar
// `lldash_dashjs.rs` and `whip_ingest.rs` already hold themselves to.
//
// Usage: node whep_playback.mjs <whipUrl> <whepUrl> <connectTimeoutMs> <holdMs>
// Prints one JSON object to stdout:
//   { ok, error, publishConnectionState, viewerConnectionState,
//     bytesSent, bytesReceived, framesDecoded, videoCurrentTime }

import { chromium } from 'playwright';
import http from 'node:http';

const [, , whipUrl, whepUrl, connectTimeoutMsArg, holdMsArg] = process.argv;
const connectTimeoutMs = parseInt(connectTimeoutMsArg, 10);
const holdMs = parseInt(holdMsArg, 10);

// See `whip_publish.mjs`'s identical comment: a secure context is required
// for `navigator.mediaDevices`, and loopback HTTP qualifies.
const blankPageServer = http.createServer((_req, res) => {
  res.writeHead(200, { 'Content-Type': 'text/html' });
  res.end('<!doctype html><title>whep harness</title><video id="v" autoplay playsinline></video>');
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
    async ({ whipUrl, whepUrl, connectTimeoutMs, holdMs }) => {
      // Waits for non-trickle ICE gathering to finish -- neither multimux
      // WHIP ingest nor WHEP egress implements a trickle-ICE PATCH
      // endpoint, so every offer POSTed here must already carry every
      // candidate.
      const waitForIceGatheringComplete = (pc) =>
        new Promise((resolve) => {
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

      const waitForConnected = async (pc, timeoutMs) => {
        const deadline = Date.now() + timeoutMs;
        let state = pc.connectionState;
        while (Date.now() < deadline) {
          state = pc.connectionState;
          if (state === 'connected' || state === 'failed' || state === 'closed') break;
          await new Promise((r) => setTimeout(r, 100));
        }
        return state;
      };

      try {
        // --- Publisher (WHIP): identical shape to whip_publish.mjs ---
        const stream = await navigator.mediaDevices.getUserMedia({ video: true });
        const track = stream.getVideoTracks()[0];
        const publishPc = new RTCPeerConnection({ iceServers: [] });
        const senderTransceiver = publishPc.addTransceiver(track, { direction: 'sendonly' });
        const senderCaps = RTCRtpSender.getCapabilities('video');
        const senderH264 = (senderCaps?.codecs ?? []).filter((c) => /H264/i.test(c.mimeType));
        if (senderH264.length === 0) {
          return { ok: false, error: 'browser reports no H264 encode capability' };
        }
        senderTransceiver.setCodecPreferences(senderH264);

        const publishOffer = await publishPc.createOffer();
        await publishPc.setLocalDescription(publishOffer);
        await waitForIceGatheringComplete(publishPc);

        const publishResp = await fetch(whipUrl, {
          method: 'POST',
          headers: { 'Content-Type': 'application/sdp' },
          body: publishPc.localDescription.sdp,
        });
        if (!publishResp.ok) {
          return {
            ok: false,
            error: `WHIP POST failed: ${publishResp.status} ${await publishResp.text()}`,
          };
        }
        const publishAnswerSdp = await publishResp.text();
        await publishPc.setRemoteDescription({ type: 'answer', sdp: publishAnswerSdp });

        const publishConnectionState = await waitForConnected(publishPc, connectTimeoutMs);
        if (publishConnectionState !== 'connected') {
          return {
            ok: false,
            publishConnectionState,
            error: 'publisher never reached connectionState=connected',
          };
        }

        // Give multimux a moment to observe the publisher's first real IDR
        // (WHIP ingest's own deferred-avcC-capture gate) before a viewer
        // tries to negotiate against the route's track set.
        await new Promise((r) => setTimeout(r, 1000));

        // --- Viewer (WHEP) ---
        const viewerPc = new RTCPeerConnection({ iceServers: [] });
        viewerPc.addTransceiver('video', { direction: 'recvonly' });

        const videoEl = document.getElementById('v');
        const trackPromise = new Promise((resolve) => {
          viewerPc.addEventListener('track', (ev) => resolve(ev.track), { once: true });
        });

        const viewerOffer = await viewerPc.createOffer();
        await viewerPc.setLocalDescription(viewerOffer);
        await waitForIceGatheringComplete(viewerPc);

        const viewerResp = await fetch(whepUrl, {
          method: 'POST',
          headers: { 'Content-Type': 'application/sdp' },
          body: viewerPc.localDescription.sdp,
        });
        if (!viewerResp.ok) {
          return {
            ok: false,
            publishConnectionState,
            error: `WHEP POST failed: ${viewerResp.status} ${await viewerResp.text()}`,
          };
        }
        const viewerAnswerSdp = await viewerResp.text();
        await viewerPc.setRemoteDescription({ type: 'answer', sdp: viewerAnswerSdp });

        const viewerConnectionState = await waitForConnected(viewerPc, connectTimeoutMs);
        if (viewerConnectionState !== 'connected') {
          return {
            ok: false,
            publishConnectionState,
            viewerConnectionState,
            error: 'viewer never reached connectionState=connected',
          };
        }

        const remoteTrack = await trackPromise;
        videoEl.srcObject = new MediaStream([remoteTrack]);
        await videoEl.play().catch(() => {});

        // Held open so both sides have real time to exchange real media:
        // the publisher keeps encoding/sending, and the *viewer's own
        // browser decoder* has to actually decode what multimux's WHEP
        // egress packetised and SRTP-encrypted for real.
        const holdDeadline = Date.now() + holdMs;
        let bytesSent = 0;
        let bytesReceived = 0;
        let framesDecoded = 0;
        while (Date.now() < holdDeadline) {
          const senderStats = await publishPc.getStats();
          senderStats.forEach((report) => {
            if (report.type === 'outbound-rtp' && report.kind === 'video') {
              bytesSent = report.bytesSent ?? bytesSent;
            }
          });
          const receiverStats = await viewerPc.getStats();
          receiverStats.forEach((report) => {
            if (report.type === 'inbound-rtp' && report.kind === 'video') {
              bytesReceived = report.bytesReceived ?? bytesReceived;
              framesDecoded = report.framesDecoded ?? framesDecoded;
            }
          });
          await new Promise((r) => setTimeout(r, 200));
        }

        return {
          ok: bytesSent > 0 && bytesReceived > 0 && framesDecoded > 0 && videoEl.currentTime > 0,
          publishConnectionState,
          viewerConnectionState,
          bytesSent,
          bytesReceived,
          framesDecoded,
          videoCurrentTime: videoEl.currentTime,
        };
      } catch (e) {
        return { ok: false, error: String((e && e.stack) || e) };
      }
    },
    { whipUrl, whepUrl, connectTimeoutMs, holdMs },
  );
} finally {
  await browser.close();
  blankPageServer.close();
}

console.log(JSON.stringify(result));
