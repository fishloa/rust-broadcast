# WebRTC fixture provenance

Fixtures for `webrtc-runtime`'s `media` feature (ICE / DTLS-SRTP / SRTP). See
issue context: the `media` feature was implemented against a live browser
only (interop, happy-path, nothing committed/repeatable) — these are the
spec-vector fixtures that should have existed from day one.

## SRTP/SRTCP — RFC 3711 Appendix B (`rfc3711-appendix-b.md`)

| Field | Value |
|---|---|
| Source | IETF RFC 3711, "The Secure Real-time Transport Protocol (SRTP)", Baugher/McGrew/Naslund/Carrara/Norrman, March 2004. |
| Exact URL | https://www.rfc-editor.org/rfc/rfc3711.txt |
| Section | Appendix B: Test Vectors — B.2 "AES-CM Test Vectors" (line ~2887 of the plain-text RFC), B.3 "Key Derivation Test Vectors" (line ~2919). |
| Retrieved | 2026-08-09, via `curl https://www.rfc-editor.org/rfc/rfc3711.txt` (plain-text `.txt` rendering, not passed through any summarizing tool, so every hex digit below is copied verbatim from the fetched text file). |
| Licence | RFC 3711's own "Full Copyright Statement" (verbatim, lines 3089–3097 of the fetched text): "Copyright (C) The Internet Society (2004). This document is subject to the rights, licenses and restrictions contained in BCP 78 and except as set forth therein, the authors retain all their rights." Per the IETF Trust's standing "Copyright" policy for RFC document text (in effect since the IETF Trust's formation and referenced by BCP 78), RFC text may be reproduced, copied, published, and distributed, in whole or in part, without restriction of any kind, provided the copyright notice is included. This project already treats RFC text as freely redistributable (`specs/` holds RFCs as the "freely-redistributable text specs" bucket per `CLAUDE.md`); this is the same basis. |
| Verification | Independently cross-checked: the B.3 master key/salt/derived-key values transcribed here are byte-identical to the ones asserted in `rtc-srtp-0.20.0`'s own internal test (`~/.cargo/registry/.../rtc-srtp-0.20.0/src/key_derivation.rs`, `test_valid_session_keys`) — two independent transcriptions of the same RFC section agree. |

## STUN — RFC 5769 (`rfc5769-stun-vectors.md`)

| Field | Value |
|---|---|
| Source | IETF RFC 5769, "Test Vectors for Session Traversal Utilities for NAT (STUN)", Denis-Courmont, April 2010. |
| Exact URL | https://www.rfc-editor.org/rfc/rfc5769.txt |
| Section | §2 "Test Vectors" (§2.1 Sample Request, §2.2 Sample IPv4 Response, §2.3 Sample IPv6 Response, §2.4 Sample Request with Long-Term Authentication) and Appendix A "Source Code for Test Vectors" (the `req[]`/`respv4[]`/`respv6[]`/`reqltc[]` C byte arrays), which is the byte-exact source used for the fixture bytes below (hand-copying hex pairs from the prose tables risks a transposition; the Appendix A C arrays are the RFC's own machine-checkable restatement of the same bytes). |
| Retrieved | 2026-08-09, via `curl https://www.rfc-editor.org/rfc/rfc5769.txt`. |
| Licence | RFC 5769's Copyright Notice (verbatim, lines 39–52 of the fetched text): "Copyright (c) 2010 IETF Trust and the persons identified as the document authors. All rights reserved. This document is subject to BCP 78 and the IETF Trust's Legal Provisions Relating to IETF Documents (http://trustee.ietf.org/license-info) ... Code Components extracted from this document must include Simplified BSD License text as described in Section 4.e of the Trust Legal Provisions and are provided without warranty as described in the Simplified BSD License." The test-vector byte arrays transcribed here are exactly such "Code Components" (literal C source in Appendix A) — explicitly BSD-licensed for extraction and reuse. |

## Target 3 — real ICE/DTLS pcap captures

**No fixture obtained.** Searched nine sources; every candidate either had
no matching capture, or the capture existed under a licence that fails the
project's hard bar. Summary (full detail in the task report):

| Source | Licence found | Verdict |
|---|---|---|
| Wireshark SampleCaptures wiki | GPL (`wiki.wireshark.org/License`: "available under the GNU General Public License... By contributing to this site, you agree to the terms of the GNU GPL") | FAIL — GPL |
| Wireshark git `test/captures/` (incl. `snakeoil-dtls.pcap`) | GPL-2.0 (`COPYING`: "GNU GENERAL PUBLIC LICENSE / Version 2") | FAIL — GPL-2.0, no per-file override found |
| tcpdump/libpcap | BSD-3-Clause | N/A — no ICE/DTLS sample-capture set exists there |
| pion/ice, pion/dtls, pion/webrtc | MIT ("Copyright (c) 2026 The Pion community") | FAIL — licence is fine but no real `.pcap`/`.pcapng` exists; `pion/dtls`'s `testdata/seed/*.raw` are small synthetic session-resumption byte dumps, not network captures |
| Chromium/WebRTC test resources | Chromium itself BSD, but the `chrome-webrtc-resources` data directory's own README requires non-Google contributors to source their own copies (no explicit redistribution grant for the data) | FAIL — no matching capture, and the access note is itself a red flag even if one existed |
| coturn | Genuine BSD-3-Clause `LICENSE` text (GitHub's licence-detector just doesn't recognize it — see coturn issue #1382) | Licence clears the bar; no test-capture pcaps exist in the repo to apply it to |
| Academic/CC0 datasets (Zenodo, IEEE DataPort) | — | FAIL — none found matching ICE/DTLS/WebRTC |
| Zeek/Bro `testing/btest/Traces/tls/{webrtc-stun,dtls1_0,dtls1_2}.pcap` | Repo is BSD-3-Clause, but Zeek's own testing docs state trace files "come from a variety of places... may carry their own licenses" | FAIL — per-file provenance explicitly disclaimed as uncertain by the project itself; unstated per-file terms are not permission |
| IETF ICE/DTLS interop suites | — | FAIL — no publicly indexed pcap repository found |

Conclusion: the repos with permissive licences (pion, coturn) don't have real
ICE/DTLS/SRTP captures committed; the repos that do have such captures
(Wireshark, Zeek) are GPL or explicitly provenance-ambiguous per their own
docs. This is a genuine "nothing found" result, not an omission — no bytes
were fabricated or borrowed under a licence that doesn't clear the bar.

**Superseded below** — the search above was for a *pre-existing* capture.
The same problem is solved by generating our own, the way `rist-runtime`'s
librist fixture was: this crate already has a working ICE/DTLS/SRTP stack
(`media` feature) proven against a real browser (issues #740/#743), so
running that stack against Chrome and capturing the loopback traffic
produces a genuine ICE/DTLS/SRTP session — Chrome's, an independent
implementation, not ours. No third-party capture-repository licence
question even arises: the licence that matters is Chrome's own EULA over
*running the browser*, and Google's own automation flags
(`--use-fake-ui-for-media-stream` etc.) exist precisely to make this kind
of scripted, no-license-question local testing possible. Chrome's output
bytes (the pcap) are not Google's copyrighted "Chrome" — they're a record
of the standard IETF ICE/DTLS/SRTP protocol runs Chrome performed against
our socket, same as any other packet capture of any protocol exchange.

## `whip-ice-dtls-srtp-loopback.pcap` — generated capture (target 1)

A genuine loopback packet capture of a real Google Chrome
`RTCPeerConnection` publishing a fake-device audio track over WHIP-lite to
this crate's own `webrtc_runtime::media::MediaTransport`
(`examples/whip_media_smoke.rs`), captured with `tcpdump` on `lo0`. It
contains, in order: the WHIP-lite HTTP/SDP offer-answer exchange (plain
TCP, not part of what this fixture is *for*, but left in because it's what
actually happened), a full ICE Binding-request/response check exchange in
both directions (STUN, RFC 5389/8489, with real USERNAME/MESSAGE-INTEGRITY/
ICE-CONTROLLING/ICE-CONTROLLED/PRIORITY/FINGERPRINT attributes), a complete
DTLS 1.2 handshake (RFC 6347, with the `use_srtp` extension, RFC 5764) run
between Chrome's real DTLS stack and this crate's `rtc-dtls`-backed
`MediaTransport`, and ~100 real SRTP packets (RFC 3711) carrying Chrome's
fake-audio-device Opus stream, decrypted live by `MediaTransport` during
the capture (the run's own stdout log confirms a decrypted plaintext RTP
packet, parsed by this workspace's own `rtp-packet` crate).

### Source

- **Generator:** Google Chrome 151.0.7922.76 (macOS arm64,
  `/Applications/Google Chrome.app`), driven headlessly
  (`--headless=new`) by a local static HTML/JS page (not committed — it is
  ~50 lines of `RTCPeerConnection`/`getUserMedia`/`fetch` boilerplate, no
  redistribution question because none of Google's/Chromium's code is
  copied into it).
- **Chrome launch flags** (the standard WebRTC test-automation set, per
  Chromium's own documented `--use-fake-ui-for-media-stream` testing
  convention and this repo's pre-existing doc comment in
  `whip_media_smoke.rs`):
  ```
  --headless=new --disable-gpu \
    --use-fake-ui-for-media-stream --use-fake-device-for-media-stream \
    --force-webrtc-ip-handling-policy=default_public_and_private_interfaces \
    --user-data-dir=<throwaway profile dir> --no-first-run
  ```
  `--use-fake-device-for-media-stream` is what makes the audio track real
  bytes rather than requiring an actual microphone (a deterministic fake
  sine/noise-ish signal Chromium itself generates) — no third-party sample
  media is captured or embedded; the SRTP payload bytes are opaque
  ciphertext regardless of what's inside them.
- **Peer under test (our own code):** `cargo run -p webrtc-runtime --features
  media --example whip_media_smoke`, built with the pinned newer-than-MSRV
  toolchain the `media` feature requires (rustc 1.94.0 — see the crate
  README's MSRV note; MSRV 1.86 doesn't build `rtc-dtls`'s `rcgen` dep).
- **Host:** macOS 26.5.2 (Darwin 25.5.0), arm64. `tcpdump` 4.99.1 (Apple
  version 158), `tshark`/Wireshark 4.6.7 used only for verification
  (protocol-hierarchy stats and field dumps below), not to write the
  fixture.
- **Licence basis:** Chrome is Google-proprietary (not itself
  redistributed here — only the network bytes it emitted while running
  locally), but this fixture needs no third-party-repository licence
  clearance at all, unlike the search above: no third-party source code or
  data is copied into the repo, only a capture of a standard-protocol
  network exchange this project's own process participated in. The
  `whip_media_smoke.rs` example being captured is this repo's own MIT OR
  Apache-2.0 code.

### Build / run

```bash
# media feature needs rustc >= 1.88; MSRV toolchain (1.86) cannot build it.
rustup toolchain install 1.94.0
cargo +1.94.0 build -p webrtc-runtime --features media --example whip_media_smoke --locked
```

### Capture command

```bash
tcpdump -i lo0 -w lo0.pcap -U 'udp or (tcp port 8787)' &
tcpdump -i en0 -w en0.pcap -U 'udp or (tcp port 8787)' &   # belt-and-braces; ended up empty, see below
cargo +1.94.0 run -p webrtc-runtime --features media --example whip_media_smoke --locked &
python3 -m http.server 8000 --bind 127.0.0.1 &             # serves the driver HTML
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
  --headless=new --disable-gpu \
  --use-fake-ui-for-media-stream --use-fake-device-for-media-stream \
  --force-webrtc-ip-handling-policy=default_public_and_private_interfaces \
  --user-data-dir=<throwaway> --no-first-run \
  http://127.0.0.1:8000/whip_test.html
```

Chrome's ICE candidates named the host's real LAN IP (`192.168.16.28`, en0)
rather than `127.0.0.1`, because
`--force-webrtc-ip-handling-policy=default_public_and_private_interfaces`
disables Chrome's usual mDNS-obfuscation of local-network candidates so a
non-browser peer can actually connect. Despite the candidate address being
en0's IP, the OS delivered the media traffic entirely over `lo0` (sending
to an IP address owned by one of the host's own interfaces loops back at
the routing layer rather than actually egressing) — confirmed empirically:
`en0.pcap` captured zero packets on the negotiated UDP port (only
unrelated background mDNS/QUIC/SSDP broadcast noise), while `lo0.pcap`
captured the full STUN/DTLS/SRTP exchange on that port. Only `lo0.pcap`
(renamed `whip-ice-dtls-srtp-loopback.pcap`) is committed.

### Run result (from the example's own stdout, this run)

```
[smoke] UDP media socket bound at 127.0.0.1:59668
[smoke] received SDP offer (1938 bytes)
[smoke] remote offered 4 candidate(s)
[smoke] added remote candidate: ... 192.168.16.28 51044 typ host ...
[smoke] ICE state changed: Checking
[smoke] ICE state changed: Connected
[smoke] DTLS handshake complete with 192.168.16.28:51044
[smoke] DECRYPTED inbound SRTP packet from 192.168.16.28:51044: 32 bytes plaintext payload
[smoke] RTP header (parsed by workspace rtp-packet crate): marker=true pt=111 seq=28377 ts=363515917 ssrc=0x4c9df50b csrc_count=0
[smoke] SUCCESS: decrypted a real inbound SRTP packet via MediaTransport.
```

### Verification (byte-level, hand-checkable — the RIST-fixture standard)

- **File identity:** classic (non-pcapng) libpcap, little-endian, magic
  `a1 b2 c3 d4` at offset 0, `DLT_NULL` (loopback) link type (`00 00 00 00`
  at offset 20) — confirmed both by `file`/`xxd` on the raw bytes and by
  `capinfos`: 146 packets, 23,865 bytes, capture duration 2.14s
  (2026-08-09 09:56:16.421989–09:56:18.563983 UTC+1), SHA-256
  `859d82f1bd303fe63e251fd807221caf0fbedf6884f3b69e5da99ed46b04bfcc`.
- **Protocol hierarchy** (`tshark -r whip-ice-dtls-srtp-loopback.pcap -q -z
  io,phs`): `tcp > http > sdp` (2 frames — the WHIP POST/201 exchange),
  `udp > stun` (7 frames), `udp > dtls`/`dtlsv1.2` (9 frames), `udp > rtp`
  (104 frames — Wireshark's own SRTP heuristic dissector, which classifies
  RTP-shaped-but-undecryptable-without-the-key payloads as `SRTP` when it
  detects an RTP header on a flow it can't decrypt: 104 of them appear as
  plain `rtp` in the `-z io,phs` hierarchy and as `SRTP` in a per-packet
  `-T fields` dump, both meaning the same thing here — ciphertext with a
  valid RTP-shaped header, exactly what encrypted SRTP looks like on the
  wire before/without key material).
- **First STUN packet (frame 27), read with `tshark -V`:** `Message Type:
  0x0001 (Binding Request)`, `Message Cookie: 2112a442` (the RFC 5389
  magic cookie), attributes `USERNAME: zk41pUeB:X4I6`, `ICE-CONTROLLING`
  (tie-breaker `d6471fb7fffd3f2b`), `USE-CANDIDATE`, `PRIORITY`, and a
  20-byte `MESSAGE-INTEGRITY` HMAC-SHA1 — real ICE, not a stub.
- **A request from our own server, frame 28**, is the mirror-image check
  (full ICE runs bidirectional checks, not ice-lite): same USERNAME
  reversed (`X4I6:zk41pUeB`), `ICE-CONTROLLED` instead of
  `ICE-CONTROLLING`, `MESSAGE-INTEGRITY`, and `FINGERPRINT` (CRC-32
  verified good by Wireshark) — proof this crate's own `rtc-ice`-backed
  gatherer, not just Chrome's, put a real authenticated Binding request on
  the wire.
- **DTLS record types** (`tshark -Y dtls -T fields -e
  dtls.record.content_type -e dtls.handshake.type`): a full DTLS 1.2
  flight — ClientHello (1) x2, ServerHello (2) + Certificate (11) +
  ServerKeyExchange (12) + ServerHelloDone (14) in one flight, ClientKeyExchange
  (16), ChangeCipherSpec (content type 20) on both sides, and Finished —
  i.e. a real ECDHE handshake with self-signed certs and DTLS-SRTP keying
  export (RFC 5764), not a truncated stub.
- **HTTP layer** (`tshark -Y http -T fields -e http.request.method -e
  http.response.code`): `OPTIONS`→204 (CORS preflight), `POST`→201 — the
  WHIP-lite SDP offer/answer exchange that bootstrapped the whole session.
- Automated re-verification of the STUN layer lives in
  `webrtc-runtime/tests/whip_smoke_pcap_stun.rs` (feature `media`): it
  walks this exact file with a hand-written classic-pcap reader (no new
  `pcap` dependency — none existed in the workspace already) and decodes
  every STUN message with `rtc_stun::message::Message` (the same type
  `media::gather` uses in production), asserting the request/response
  pair, USERNAME/MESSAGE-INTEGRITY, and the ICE-CONTROLLING XOR
  ICE-CONTROLLED role split found above; a second test spot-checks that
  later non-STUN UDP payloads on the flow are DTLS-record-shaped and then
  RTP-shaped (the SRTP tail), i.e. that the fixture's byte progression
  matches the STUN→DTLS→SRTP story this document claims.

## RIST — VSF TR-06-1 Appendix A (already committed, cross-referenced here)

The byte-exact worked NACK example (bitmask + range formats) from VSF
TR-06-1:2020 Appendix A is already transcribed in
`rist-runtime/docs/tr-06-1-simple-profile.md` and already has committed
tests in `rist-runtime/tests/spec_vectors.rs` (`appendix_a_bitmask_nack`,
`appendix_a_range_nack`, and the cross-verification tests) — landed in an
earlier PR (#920). Not re-done here; see the task report for the bite-proof
re-run.

## RIST ARQ fixture (issue #741) — librist netns feasibility

**Licence — verified BSD-2-Clause.** `code.videolan.org/rist/librist`'s
`COPYING` (fetched via the `deepin-community/librist` upstream mirror,
since the primary GitLab host sits behind a bot-wall that blocked direct
fetch): "Copyright © 2019-2020, VideoLAN and librist authors. All rights
reserved. Redistribution and use in source and binary forms, with or
without modification, are permitted provided that the following conditions
are met: 1. Redistributions of source code must retain..." — two
conditions, no endorsement clause: textbook BSD-2-Clause. The task brief's
claim is correct.

**The netns/`tc netem` premise does not hold — librist doesn't use them.**
A full repo search (0 hits for `netem`/`netns`/`qdisc`; `.gitlab-ci.yml` is
a plain meson build→test pipeline with no network jobs) found librist's own
ARQ/loss testing is done **in-process**, not via OS-level network
emulation: `test/rist/test_send_receive.c` takes a loss-percentage argument
and drops packets with a PRNG (`sender_ctx->simulate_loss = true;
sender_ctx->loss_percentage = N`), all over loopback — no kernel network
namespace, no `tc`, nothing Linux-specific at all.

**macOS feasibility: yes, and no netns workaround is even needed.** Because
the loss injection is librist's own software simulator rather than a
kernel-level mechanism, building librist (portable C, macOS is a supported
target) and running `ristsender`/`ristreceiver` or the `test_send_receive`
harness directly on this machine produces a genuine RIST ARQ/retransmission
capture with real packet loss — natively, no netns, no Docker required.

Side finding on the netns/`tc netem` approach in general (for any *other*
future fixture that does need real kernel-level loss injection): Docker
Desktop's Linux VM supports `tc netem` with `--cap-add=NET_ADMIN` (no
`--privileged` needed) on Intel Macs, but is currently broken on Apple
Silicon — `tc qdisc ... netem` fails with `RTNETLINK answers: Operation not
supported` (`docker/for-mac#7138`, open since January 2024, no documented
fix). On Apple Silicon the fallback would be a real Linux VM (Lima/UTM/
Multipass with a virtualized NIC), not Docker Desktop. Not needed for this
fixture, but worth recording since it will recur.

No librist capture was actually generated in this task (fixture-sourcing
scope only; building/running librist is implementation work) — this
section is the feasibility assessment the task asked for.
