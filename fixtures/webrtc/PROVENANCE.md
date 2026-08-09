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
