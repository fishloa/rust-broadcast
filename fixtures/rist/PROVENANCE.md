# RIST fixture provenance (`fixtures/rist/`)

## `rist-simple-loss25pct-loopback.pcap`

A genuine network capture of a **librist** (VideoLAN's reference RIST
implementation) RIST Simple Profile sender/receiver session running with
librist's built-in loss simulator enabled, captured on macOS loopback
(`lo0`). Generated for issue #741 (rist-runtime ARQ engine) — TR-06-1
describes the wire formats but the crate had no fixture exercising real
packet loss + retransmission.

### Source

- **Repository:** `https://code.videolan.org/rist/librist.git`
- **Version/tag:** `v0.2.20`
- **Commit:** `4f45ef8f78983892d52ccd52d9f675435b23738f` (2026-07-30, "Merge branch
  'fix/news-0220-trim' into 'master'") — `v0.2.20` and `master` HEAD coincide
  at this commit.
- **Licence:** BSD-2-Clause. Quoted verbatim from librist's `COPYING` at this
  commit:

  > Copyright © 2019-2020, VideoLAN and librist authors
  > All rights reserved.
  >
  > Redistribution and use in source and binary forms, with or without
  > modification, are permitted provided that the following conditions are
  > met:
  >
  > 1. Redistributions of source code must retain the above copyright notice,
  >    this list of conditions and the following disclaimer.
  >
  > 2. Redistributions in binary form must reproduce the above copyright
  >    notice, this list of conditions and the following disclaimer in the
  >    documentation and/or other materials provided with the distribution.
  >
  > THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS
  > IS" AND ANY EXPRESS OR IMPLIED WARRANTIES [...] (full disclaimer applies)

  Permissive 2-clause BSD — redistributing a *capture of the software's
  network output* (not its source/binary) is clearly within scope; the
  licence text is quoted here for the record of what was verified, per the
  #741 assessment's precondition that the licence be checked before using
  librist as a fixture source.

### Build

```
brew install meson ninja        # meson 1.11.2, ninja 1.13.2
git clone https://code.videolan.org/rist/librist.git
cd librist && git checkout v0.2.20
meson setup build --buildtype=release
ninja -C build
```

Host: macOS 26.5.2 (Darwin 25.5.0), arm64, Apple clang 21.0.0
(`clang-2100.1.1.101`, target `arm64-apple-darwin25.5.0`). No patches applied
to librist source.

### Capture command

librist's own loss-simulation harness is `test/rist/test_send_receive.c`,
built as `build/test/rist/test_send_receive`. It is also what librist's own
meson test suite uses for its "packet loss" test cases (`test/rist/meson.build`
line 287: `test('Simple profile unicast packet loss 25%', test_send_receive,
args: ['0', 'rist://@127.0.0.1:3234', 'rist://127.0.0.1:3234', '25'], ...)`).
We ran that exact invocation directly, with `tcpdump` capturing loopback
alongside it:

```
tcpdump -i lo0 -w capture.pcap -U udp &
./build/test/rist/test_send_receive 0 'rist://@127.0.0.1:3234' \
    'rist://127.0.0.1:3234' 25
# test binary printed "OK" and exited 0
pkill -f 'tcpdump -i lo0 -w capture.pcap'
```

- `profile=0` → RIST **Simple Profile** (the profile `rist-runtime`/TR-06-1
  target).
- `losspercent` arg `25` → librist's `argv[4] * 10` = `250`; librist's own
  loss-percentage unit is **tenths of a percent** compared against
  `rand() % 1001` (`src/udp.c` lines 166-175, 259-267: `if (compare <=
  loss_percentage) { /* drop */ }`), so `250` = a nominal **25% independent
  drop probability per packet**, applied identically on the sender's egress
  and the receiver's egress (both `simulate_loss=true`), which is why the
  effective loss and retransmit-request rate below is higher than a naive
  25% (loss can occur on either leg's simulated send). This confirms the
  #741 assessment: no `tc netem`/network-namespace scripts were needed —
  librist's ARQ test harness drops packets **in-process** via this PRNG gate
  before the syscall, which is why loopback capture alone is sufficient to
  observe genuine retransmissions.
- The harness sends 16000 payloads (~1316-byte MPEG-TS-shaped UDP payloads,
  188-byte-multiple, `0x47` sync byte) at roughly 2000 pkt/s (`usleep(500)`
  between sends) for ~8 seconds, then the receiver validates content and the
  process reports `OK`/exit 0 if enough packets arrived intact — confirming
  librist's own correctness check passed on this run, i.e. loss was masked
  by successful retransmission, not just tolerated data loss.

### Extraction

The full capture (`tcpdump -i lo0 -w ... -U udp`, no host/port filter beyond
`udp`) was 17646 packets / 13 MB over ~26.6 s wall-clock (includes ~10 s
handshake settle + unrelated local mDNS broadcast noise). It was reduced to
this fixture with `tshark`/`editcap` (Wireshark 4.6.7, installed via
`brew install wireshark`):

```
tshark -r full-capture.pcap -Y "udp.port==3234 || udp.port==3235" \
    -w filtered.pcap
editcap -A "<t0>" -B "<t0 + 0.4s>" filtered.pcap window.pcap   # 400ms slice
editcap -F pcap window.pcap rist-simple-loss25pct-loopback.pcap  # classic pcap
```

- Port **3234** = RTP data (even port `P`); port **3235** = RTCP feedback
  (`P+1`) — this observed port pairing matches TR-06-1 §5.1.1's unicast port
  assignment rule (`docs/tr-06-1-simple-profile.md` §2.1) exactly; it is not
  asserted from the spec, it is what librist actually put on the wire.
- The 400 ms window was chosen because it already contains a rich, verified
  sample of every packet class needed (below) at ~1/33rd the size of the
  full capture (498 KB vs 13 MB) — there was no need to keep the whole
  session to get genuine loss+retransmit behaviour.
- Converted from pcapng back to classic pcap format (`editcap -F pcap`) for
  broadest Rust-crate compatibility.

SHA-256 of the committed file:
`98e0f9f6ca9b5f79407f4d86ca4fceac790e59ff3324b8706dda7844c19e7936`

### What the fixture contains (verified with `tshark`, counts against the
committed file itself, not the pre-trim capture)

- 685 packets total, link-type `NULL` (BSD loopback, 4-byte AF header — NOT
  Ethernet), 0.399 s of wall-clock capture.
- **617 RTP data packets** on UDP port 3234, split by the SSRC LSB
  retransmission marker described in TR-06-1 §5.3.3
  (`docs/tr-06-1-simple-profile.md` §4.3):
  - **458** with SSRC `0xd5615604` (LSB=0, original transmissions)
  - **159** with SSRC `0xd5615605` (LSB=1, **retransmissions** — same 31
    upper bits as the original-flow SSRC, exactly as the spec states)
- **68 RTCP APP packets** (`PT=204`) on UDP port 3235, name field
  `"RIST"` (`0x52495354`), by `Subtype`:
  - **55× Subtype=0 — Range-Based Retransmission Request** (TR-06-1
    §4.2.2/§5.3.2.2)
  - 7× Subtype=2 — RTT Echo Request (§5.2.6)
  - 6× Subtype=3 — RTT Echo Response (§5.2.6)
- **0× Generic NACK (bitmask, `PT=205`, `FMT=1`, §5.3.2.1/RFC 4585 §6.2)** —
  this build of librist's Simple Profile implementation only emits the
  Range-Based request type. TR-06-1 §4.2 permits this: "RIST senders shall
  support both [request types]. RIST receivers may implement either one, or
  both." librist's receiver-side choice is range-based only. **This fixture
  therefore does not exercise Generic NACK parsing** — if `rist-runtime`
  needs a bitmask-format fixture too, it will need a different source (a
  RIST implementation that emits Generic NACK, or a hand-built wire-format
  vector per TR-06-1 Appendix A, already transcribed in
  `docs/tr-06-1-simple-profile.md`).
- Standard compound-RTCP framing throughout: every RTCP packet observed is
  `SR/RR + SDES + [APP]`, matching §3 of the transcription (3 Sender
  Reports, 65 Receiver Reports, 68 SDES blocks).

### Verified NACK → retransmission correlation (not just "clean traffic with
some NACKs present" — an actual request-then-response pair, byte-traced)

Frame 15 (RTCP, UDP 3235→52318, `t=0.0058s` into the window) is a Range-Based
Retransmission Request. Its raw APP payload bytes are:

```
80 cc 00 05 d5 61 56 04 52 49 53 54 05 45 00 00 06 02 00 00 06 06 00 00
```

Decoded per the §4.2.2 layout in `docs/tr-06-1-simple-profile.md`:
- `80 cc` → V=2,P=0,Subtype=0; PT=0xCC=204 (APP)
- `00 05` → Length=5 → (5+1)×4 = 24 bytes total (header+SSRC+name+3 range
  fields)
- `d5 61 56 04` → SSRC of media source = `0xd5615604`
- `52 49 53 54` → name = "RIST"
- Three 32-bit range-request fields, each `Additional=0` (single-packet
  requests, not contiguous runs):
  - `05 45 00 00` → Start=0x0545=**1349**, Additional=0
  - `06 02 00 00` → Start=0x0602=**1538**, Additional=0
  - `06 06 00 00` → Start=0x0606=**1542**, Additional=0

The very next RTP-data packets on port 3234 bearing the retransmission SSRC
(`0xd5615605`) are frames 21/22/23, at `t=0.00954/0.00955/0.00957s` (4-5 ms
after the request):

| Frame | RTP seq | SSRC | Δt from NACK |
|---|---|---|---|
| 21 | 1349 | `0xd5615605` | +3.7 ms |
| 22 | 1538 | `0xd5615605` | +3.7 ms |
| 23 | 1542 | `0xd5615605` | +3.8 ms |

All three requested sequence numbers (1349, 1538, 1542) are retransmitted,
in the order requested, with no unrelated retransmitted sequence numbers
between them — this is a genuine request→response pair, reproducible by
anyone re-running `tshark -r rist-simple-loss25pct-loopback.pcap -Y
"frame.number==15" -x` and `tshark -r rist-simple-loss25pct-loopback.pcap -d
udp.port==3234,rtp -Y "rtp.ssrc==0xd5615605 && frame.number>15"` against the
committed file. This is one of 55 Range-Based requests and 159
retransmissions in the fixture; it is quoted here as a hand-verifiable
worked example, not the only instance of the correlation.

### Reproducibility

Because librist's loss simulator seeds `srand((unsigned int)time(NULL))`
(`test/rist/test_send_receive.c` line 138) at one-second resolution, and RIST
Simple Profile has no bonding/handshake randomness beyond that, re-running
the exact capture command will **not** byte-reproduce this file (different
PRNG draws → different loss pattern each run), but will reproduce a fixture
with the same structural properties (RTP on P, RTCP on P+1, Range-Based
requests, SSRC-LSB-marked retransmissions, `OK`/exit 0 on completion) with
overwhelming probability at a 25% configured loss rate over 16000 packets.

### Deferred (out of this task's stated scope)

The original brief's step 6 ("add a test in `rist-runtime/tests/` parsing
the real NACKs from the capture and asserting against our types") was not
done here — this pass was scoped to `rist-runtime/docs/` and
`fixtures/rist/` only, and `rist-runtime/tests/` falls outside that. The
byte-level worked example above (frame 15's exact FCI bytes, and the exact
`tshark` filter expressions used to derive it) should be sufficient input
for that follow-up without needing to redo the capture/analysis.
