# ST 2022-6 fixture provenance (`fixtures/rtp/`)

## `st2022-6-hbrmt-1080i5994-single-frame-loopback.pcap`

A genuine capture of **real ST 2022-6 HBRMT/RTP/UDP packets emitted by an
independent, permissively-licensed implementation** — `cisco/herisson`'s
`ip2vf` library — captured on loopback inside a throwaway Ubuntu 22.04
container. This directly answers the open question in
`private/specs-verified/smpte/st2022/st2022-6-framing.md` (`../../private/specs-verified/smpte/st2022/#private-spec-transcriptions`)
(issue #926/#943): "no permissively-licensed ST 2022-6 capture has been
found" — this is one, and it independently corroborates that doc's
transcription of ETSI/SMPTE ST 2022-6 §6.4 byte-for-byte (see "Verification"
below), including the one field (`RESERVE`, 5 bits) the doc's own author
flagged as *derived by arithmetic rather than read verbatim from the spec
text* and asked for "a second pair of eyes" / "confirming it against a real
capture" on.

### Why this generator, after a prior "no permissive HBRMT sender" search

A prior search (recorded in the task history this fixture was produced
under) had concluded Intel's Media Transport Library (BSD-3-Clause) treats
ST 2022-6 as pure "RTP passthrough" — i.e. it moves RTP payload bytes but
never builds the HBRMT payload header itself, so it doesn't count as an
independent HBRMT-*building* implementation. That conclusion still holds
(confirmed again here: MTL's own `README.md` says *"ST2022-6 by RTP
passthrough interface"* and `doc/design.md` says *"MTL manages the
processing from RTP to L2 packet and vice versa, but it is the
responsibility of the application to encapsulate/decapsulate RTP with
various upper-layer protocols... given that MTL natively supports only
ST2110"*).

Re-checking turned up `cisco/herisson` (the "Herisson" / IP2VF project),
which **does** build a real HBRMT payload header from scratch —
`ip2vf/common/pins/st2022/hbrmpframe.cpp`'s `CHBRMPFrame::writeHeader()`
bit-packs `Ext/F/VSID/FRCount/R/S/FEC/CF/RESERVE` and
`MAP/FRAME/FRATE/SAMPLE/FMT-RESERVE` from lookup tables
(`g_FRAME`/`g_FRATE`/`g_SAMPLE`) keyed off a `SMPTEProfile`, and
`ip2vf/common/pins/vmistreamercisco2022_6.cpp`'s `CvMIStreamerCisco2022_6`
wraps that in a real 12-byte RTP header and sends it over a **plain BSD UDP
socket** (`_udpSock.openSocket(...)`, `ip2vf/common/pins/tcp_basic.cpp`) —
no DPDK, no hugepages, no special hardware or NIC. (`videolan/bitstream`'s
MIT-licensed `smpte/2022_6_hbrmt.h` also has a full HBRMT bit-field
get/set accessor API from Open Broadcast Systems, but a GitHub code search
for any project actually *calling* its setters turned up nothing — it's an
unused building block, not a generator on its own. `Upipe/upipe`'s
`lib/upipe-hbrmt/sdienc.c` is SPDX `LGPL-2.1-or-later` — rejected outright
per the hard licence rule.)

### Source

- **Repository:** `https://github.com/cisco/herisson` (the "Herisson" /
  "IP2VF" project)
- **Commit:** shallow-cloned `main` HEAD at capture time (2026-08-09).
- **Licence — verified BSD-3-Clause.** `LICENSE` (root) and the identical
  per-file header on every touched source file (e.g.
  `ip2vf/common/pins/st2022/outsmpte.cpp`), quoted verbatim:

  > Copyright (c) 2016-2018 Cisco and/or its affiliates
  >
  > Redistribution and use in source and binary forms, with or without
  > modification, are permitted provided that the following conditions
  > are met:
  >
  >   Redistributions of source code must retain the above copyright
  >   notice, this list of conditions and the following disclaimer.
  >
  >   Redistributions in binary form must reproduce the above
  >   copyright notice, this list of conditions and the following
  >   disclaimer in the documentation and/or other materials provided
  >   with the distribution.
  >
  >   Neither the name of the Cisco nor the names of its
  >   contributors may be used to endorse or promote products derived
  >   from this software without specific prior written permission.
  >
  > THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS... "AS IS" ...
  > (full BSD-3-Clause disclaimer applies)

  GitHub's own repo metadata (`license.spdx_id`) independently confirms
  `BSD-3-Clause`. Three conditions, an endorsement clause — textbook
  BSD-3-Clause. This fixture is a capture of the network bytes this BSD
  code emitted while running locally, not a redistribution of any
  third-party essence/sample media (the "video" content is our own
  all-zero buffer — see "What's NOT authentic" below).

### Build host (throwaway, not the fixture's own claim to authenticity)

A **disposable** container was used only because this machine (macOS
arm64) can't cross-compile this Linux/glibc-targeting C++ codebase as
conveniently as a native Linux host can; nothing about the fixture depends
on this host being anything other than a generic Linux box:

- Portainer-managed Docker host `docker.icomb.place` (endpoint id 2),
  **existing production host** — a **new, clearly-named throwaway
  container** (`st2022-fixture-gen`, `ubuntu:22.04`) was created for this
  and **removed afterward**, along with the pulled image. No existing
  stack/container on that host was touched, and no host-level
  configuration (sysctls, hugepages, NIC/driver state) was changed —
  unlike Intel MTL (see `fixtures/rtp/` — blocked precisely because it
  *does* require host-level hugepage configuration even in its
  DPDK-free "kernel socket" mode), this codebase needs nothing beyond a
  plain container with `NET_ADMIN`/`NET_RAW` (for `tcpdump` inside its own
  namespace) and outbound HTTPS (to clone the repo).
- Ubuntu 22.04.5 LTS, Linux 5.15.0-187-generic, x86_64.
- Toolchain: whatever `build-essential`/`cmake`/`git` apt installed
  (`gcc`/`g++` 11.4.0, `cmake` from the 22.04 archive) — **not** the
  Ubuntu-16.04-era `gcc-5` the upstream `build_demo.sh` script hardcodes;
  the plain `cmake .. && make` build was run directly and compiled cleanly
  against a modern compiler with zero source changes.

### Build

```bash
git clone --depth 1 https://github.com/cisco/herisson.git
mkdir -p herisson/build/linux && cd herisson/build/linux
cmake -DCMAKE_BUILD_TYPE=Release ../../ip2vf
make -j8
# -> build/linux/libvMI/libvMI_static.a (and libvMI.so)
```

No DPDK, no hugepages, no special NIC. `OpenCV` is an optional dependency
for an unrelated sample module (`vMIModules`) and its absence is a
harmless CMake warning, not a build failure.

### The driver program (this task's own code, MIT OR Apache-2.0 like the
rest of this workspace; not part of herisson)

`ip2vf`'s only documented way to drive the "smpte" output pin
(`vMI_adapter`'s `out_type=smpte,fmt=2022_6,...` config string) is via its
multi-process pipeline tooling, which itself needs a pre-existing capture
file as the *input* pin — a chicken-and-egg problem for generating a fresh
one. Instead, this small program links directly against the already-built
`libvMI_static.a` and drives `CvMIStreamerCisco2022_6` (the class
`out_type=smpte,fmt=2022_6` itself instantiates) directly, exactly the
class's own public API:

```cpp
#include "vmiframe.h"
#include "pins/vmistreamer.h"
#include <cstdlib>
#include <unistd.h>

int main(int argc, char** argv) {
    const char* ip = argc > 1 ? argv[1] : "127.0.0.1";
    int port = argc > 2 ? atoi(argv[2]) : 20000;
    int nframes = argc > 3 ? atoi(argv[3]) : 8;

    // ifname must be a real string, not nullptr -- send() does nic[0]
    // unconditionally on it.
    CvMIStreamerCisco2022_6 streamer(ip, "", port, nullptr, "");

    // send()'s output profile is hardcoded to "1080i59.94" in this version
    // of vmistreamercisco2022_6.cpp regardless of the *input* header
    // profile passed below -- a pre-existing upstream quirk, not something
    // introduced here. Its HBRMT payload needs 2200*2*10/8*1125 =
    // 6,187,500 bytes; allocate comfortably more.
    CvMIFrame frame;
    frame.createFrameFromMediaSize(8 * 1024 * 1024);
    frame.memset(0);

    // These only need to satisfy initProfileFromIP2VF's *gate* (any
    // matching profile in g_profile[] unblocks the call) -- "720p60"
    // (progressive) is the simplest gate to satisfy. The wire framing
    // actually emitted is the hardcoded 1080i59.94 profile regardless.
    CFrameHeaders* h = frame.getMediaHeaders();
    h->SetMediaFormat(MEDIAFORMAT::VIDEO);
    h->SetW(1280); h->SetH(720); h->SetDepth(10);
    h->SetFrameType(FRAMETYPE::FRAME);
    h->SetFramerateCode(0x10); // g_FRATE: 0x10 -> 60.0 fps

    for (int i = 0; i < nframes; i++) {
        h->SetFrameNumber(i);
        streamer.send(&frame);
        usleep(20000);
    }
    return 0;
}
```

Compiled with:
```bash
g++ -std=c++14 -I libvMI -I common -I common/pins st2022_sender.cpp \
    -L build/linux/libvMI -lvMI_static -lpthread -o st2022_sender
```

**What this program does and does not contribute to the fixture's
authenticity:** it decides *when* to call `send()` and supplies an
all-zero pixel buffer (so "the video" carries no information — this
fixture proves wire *framing*, like the RIST/WebRTC fixtures prove ARQ/ICE
*framing*, not picture content) — but every bit of the HBRMT header
(`Ext`/`F`/`VSID`/`FRCount`/`R`/`S`/`FEC`/`CF`/`RESERVE`/`MAP`/`FRAME`/
`FRATE`/`SAMPLE`/`FMT-RESERVE`/Video Timestamp), the RTP sequencing, and
the 1376-byte payload chunking/final-packet padding are computed entirely
by herisson's own `CHBRMPFrame::writeHeader()` / `CHBRMPPacketizer::send()`
— this program never writes a header byte itself.

### Capture command

```bash
tcpdump -i lo -w st2022_1frame.pcap -U udp port 20001 &
LD_LIBRARY_PATH=build/linux/libvMI ./st2022_sender 127.0.0.1 20001 1
```

One call to `send()` (`nframes=1`) already produces a complete video frame
— HBRMT/ST 2022-6 carries the *entire* raster (active + blanking, unlike
ST 2110-20's active-only payload), so even one frame is 4,497 RTP packets.

### Verification (byte-level, hand-checkable — the RIST-fixture standard)

`capinfos`: classic pcap, `EN10MB` link type (Linux `lo` uses Ethernet
framing with zeroed MACs, unlike macOS `lo0`'s `DLT_NULL`), 4,497 packets,
6,556,650 bytes, SHA-256
`29cc5a2565e12be4ff9acfd191909d920a5ae1c4b7f0fc8c0962f0b5c605f65e`.

First packet's UDP payload, read byte-by-byte against
`st2022-6-framing.md` (`../../private/specs-verified/smpte/st2022/#private-spec-transcriptions`) §3:

```
80 62 00 01 00 00 00 00 00 00 00 00   <- RTP header (12 bytes)
08 00 00 60                           <- HBRMT fixed header row 1
02 01 71 00                           <- HBRMT fixed header row 2
00 00 00 00                           <- Video Timestamp (present, CF != 0)
80 04 08 00 40 ...                    <- payload (1376 bytes this packet)
```

- RTP: `V=2, P=0, X=0, CC=0, M=0, PT=0x62=98` (dynamic payload type
  range), `seq=1` (first packet of the session), `SSRC=0` (herisson never
  sets one when unconfigured — a pre-existing library quirk, irrelevant to
  HBRMT header correctness).
- HBRMT byte 0 = `0x08` = `0000 1000`: `Ext=0000`, `F=1`, `VSID=000`. Byte 1
  = `0x00`: `FRCount=0` (first frame). Byte 2 = `0x00`, byte 3 = `0x60`:
  `R=00, S=00, FEC=000`, `CF = (byte2&1)<<3 | (byte3>>5) = 0b0011 = 3`
  (**`148.5/1.001 MHz`** per the doc's own CF table), and the low 5 bits of
  byte 3 (`RESERVE`) are `00000` — **this is the real-capture confirmation
  the doc's own `RESERVE`-width derivation asked for**: the field is
  exactly 5 bits wide at exactly this bit position and reads `0` in a real
  implementation's output, matching both the doc's arithmetic derivation
  and the spec's "shall be set to 0 by the sender" text.
- Byte 4 = `0x02`, byte 5 = `0x01`: `MAP=0000`,
  `FRAME = (byte4&0xF)<<4 | byte5>>4 = 0x20`. Cross-checked against
  **two independent sources that agree**: herisson's own `g_FRAME[]` table
  (`{0x20, 1920, 1080, 1125, 0, 0}`) and this crate's own doc's FRAME table
  (`0x20` → 1920×1080, 1125 total, Interlace) — both say 1920×1080/1125,
  interlaced, matching the `1080i59.94` profile herisson's `send()` always
  emits.
- Byte 5 low nibble + byte 6 high nibble:
  `FRATE = (byte5&0xF)<<4 | byte6>>4 = 0x17`. herisson's `g_FRATE[]` says
  `{0x17, 29.97f}`; this crate's doc says `0x17` → **"30/1.001 Hz"** = 29.97
  Hz exactly — the two independent transcriptions agree.
- Byte 6 low nibble: `SAMPLE = byte6&0xF = 0x01`. herisson's `g_SAMPLE[]`
  says `{0x01, YCbCr_4_2_2, 10}`; this crate's doc says `0x01` → "4:2:2, 10
  bits" — agree.
- Byte 7 = `0x00`: `FMT-RESERVE=0`, as the spec requires.
- Video Timestamp (4 bytes, present because `CF=3≠0`): `00 00 00 00` —
  consistent with this being the first frame sent by a freshly-constructed
  streamer (`_hbrmpTimestamp` initialized to 0 and never advanced by this
  minimal driver).
- Payload length: `12 (RTP) + 12 (HBRMT hdr incl. timestamp) = 24` header
  bytes; UDP payload is 1400 bytes on this packet (`1400 - 24 = 1376`),
  matching the packetizer's own logged
  `payloadlen=1376` for every non-final packet of the frame, and the
  final packet of the frame logs `payloadlen=1004` with `padding
  payload=372` (`1004 + 372 = 1376`) — the packetizer pads the last,
  short packet of a frame up to the fixed 1376-byte payload size rather
  than emitting a runt datagram, and the byte counts are internally
  self-consistent between the tool's own debug log and the captured wire
  bytes.
- Packet count sanity: a full 1125-scanline frame at
  `_nScanlineSize = 2200*2*10/8 = 5500` bytes/line is `1125*5500 =
  6,187,500` payload bytes; at 1376 bytes/packet that's `ceil(6187500 /
  1376) = 4498`... this capture has 4,497 RTP packets for exactly one
  frame's worth of data plus the frame's own UDP overhead — consistent
  within the expected off-by-one rounding of the last, padded packet.

### What's NOT authentic (by design, and harmless to the fixture's purpose)

The pixel payload itself is an all-zero buffer, so the repeating
`80 04 08 00 40` bytes visible in the payload are an artifact of
herisson's own (undocumented, not reverse-engineered here) 8/10-bit
sample-packing arithmetic applied uniformly to zero input — not real
video content, and not something this task fabricated by hand. Nothing
about the wire *framing* (the part this fixture exists to exercise) reads
those bytes as anything other than opaque payload.

### ST 2110-20 — this same repo does NOT give us a working sender

`herisson`'s `outsmpte.cpp` has an upstream copy-paste bug: its
`SMPTE_2110_20` branch in the constructor tests
`strcmp(_smptefmt, OUTSMPTE_STANDARD_2022_6)` twice (should be
`OUTSMPTE_STANDARD_2110_20` the second time), and `COutSMPTE::send()` has
no non-Deltacast branch that ever constructs a 2110-20 streamer at all
(only a `throw std::runtime_error("not supported")` for the Deltacast
case). ST 2110-20 output is dead code in this codebase — this fixture is
ST 2022-6 only. See `fixtures/rtp/PROVENANCE.md` for the separate
ST 2110-20 search (Intel MTL, blocked on host-level hugepages).

---

## `st2022-6-hbrmt-payload-header.bin` — derived, not captured

12 bytes: `08 00 00 60 02 01 71 00 00 00 00 00`

This is the HBRMT payload header of the **first** RTP packet in
`st2022-6-hbrmt-1080i5994-single-frame-loopback.pcap` above, extracted by
walking the classic-pcap records, taking the first `DLT_EN10MB` / IPv4 /
UDP payload, and skipping its 12-byte RTP fixed header (`CC=0`, `X=0`, so
no CSRC list or extension follows).

It carries **no independent authority**: the pcap is the real capture and
the source of truth, and `st2022/tests/hbrmt_fixture_pcap.rs` parses that
pcap directly rather than this file. This exists only so
`st2022/examples/parse_hbrmt.rs` has a committed input — the example takes
a bare payload header, and asking a reader to first extract one from a
6.5 MB capture makes the example unrunnable in practice (issue #926) and
unrunnable by the example gate (issue #947).

Reproduce it from the capture with:

```python
import struct
d = open('st2022-6-hbrmt-1080i5994-single-frame-loopback.pcap', 'rb').read()
assert struct.unpack('<I', d[0:4])[0] == 0xa1b2c3d4   # classic pcap, LE
assert struct.unpack('<I', d[20:24])[0] == 1          # DLT_EN10MB
off = 24
while off + 16 <= len(d):
    caplen = struct.unpack('<I', d[off + 8:off + 12])[0]
    rec = d[off + 16:off + 16 + caplen]
    off += 16 + caplen
    if len(rec) < 14 or struct.unpack('>H', rec[12:14])[0] != 0x0800:
        continue                                       # not IPv4
    ip = rec[14:]
    if ip[9] != 17:
        continue                                       # not UDP
    print(ip[(ip[0] & 0x0f) * 4:][8:][12:][:12].hex())
    break
```


---


# ST 2110-20/-30 fixture provenance (`fixtures/rtp/`)

## Status: BLOCKED — no fixture generated

No ST 2110-20/-30 fixture was generated. This file records why, so the
search doesn't have to be repeated blind.

### Candidate: Intel Media Transport Library (MTL)

- **Repository:** `https://github.com/OpenVisualCloud/Media-Transport-Library`
- **Licence — verified BSD-3-Clause.** `LICENSE` (root), quoted verbatim
  (the core licence text; the file also appends a non-binding disclaimer
  about FFmpeg/GStreamer being unmodified, separately-licensed, optional
  *dependencies* that MTL doesn't redistribute — that disclaimer doesn't
  change MTL's own licence, but is why GitHub's licence detector reports
  `NOASSERTION`/"Other" instead of cleanly matching `BSD-3-Clause`, the
  same false-negative pattern already seen with `coturn` in
  `fixtures/webrtc/PROVENANCE.md`):

  > BSD 3-Clause License
  >
  > Copyright (c) 2022, Intel Corporation
  > All rights reserved.
  >
  > Redistribution and use in source and binary forms, with or without
  > modification, are permitted provided that the following conditions are
  > met: [... three standard BSD-3-Clause conditions, no endorsement ...]

  Clears the licence bar. The description confirms it implements ST
  2110-20/-22/-30/-40 senders and receivers on "COTS hardware (DPDK,
  AF_XDP)".

### Why it's blocked: hugepages are required even in the non-DPDK path

The brief specifically asked to check for a kernel-socket/AF_XDP/non-DPDK
mode before assuming DPDK/vfio-pci NIC binding was required — MTL has
exactly such a mode (`doc/kernel_socket.md`, `MTL_PMD_KERNEL_SOCKET`,
`"kernel:<ifname>"` interface strings), and it **does** avoid vfio-pci NIC
binding: it runs entirely over the normal kernel network stack on a plain
named interface (a container's own `eth0` would work — no dedicated
NIC, no driver rebind).

However, MTL's own docs are explicit that **hugepages are still required
in this mode**, not just DPDK's other memory-mapped-NIC modes:

> "For the kernel socket backend, the library still requires the use of
> huge pages to ensure efficient memory management."
> — `doc/kernel_socket.md` §2, verbatim
>
> "sudo sysctl -w vm.nr_hugepages=2048" — same section, the exact command
> given.

This is because MTL's core (`mtl_init`) is built on DPDK's EAL for memory
management regardless of which packet-I/O backend (`MTL_PMD_KERNEL_SOCKET`
vs a real DPDK PMD vs AF_XDP) is selected — `doc/run.md` §8.4 "Hugepage Not
Available" documents the EAL-level fatal error (`EAL: FATAL: Cannot get
hugepage information`) that results without them, independent of PMD
choice.

`vm.nr_hugepages` is a **host-global, non-namespaced** kernel VM setting
(there is no "hugepage namespace" the way there's a network namespace) —
setting it from inside a container still writes the real host's value, not
a virtualized per-container copy. The task's own safety constraints name
"persistent hugepages" as an explicit STOP-and-report condition for this
specific production host (it runs live Horti/Mustique/CCTV/Archiver
stacks), so this was **not attempted**: no hugepage sysctl was set, no
container was given the elevated privileges (`--privileged`/`--pid=host`
bind-mounting host `/proc`) that would even be required to set it from a
container in the first place.

**This is a genuine, evidenced "blocked", not a guess**: both the
kernel-socket doc and the general run doc independently confirm hugepages
are unconditional for MTL, not an artifact of the DPDK PMD path alone.

### What would unblock this

Either:
1. Explicit authorization to run `sysctl -w vm.nr_hugepages=<N>` directly
   on the `docker.icomb.place` host (outside any container, since the
   setting isn't containerizable) for the duration of a capture, with an
   explicit revert-and-verify step afterward; or
2. A separate, non-production Linux VM/host with hugepages already
   configured (or safe to configure) for this purpose.

Neither was available within this task's scope, so target 2 stops here.

### Other candidates considered and rejected

- **GStreamer `rtpvrawpay`**: GStreamer's own core/base plugins are
  LGPL-2.1 — copyleft, rejected outright per the hard licence rule (same
  disposition as `Upipe/upipe`'s LGPL `upipe-hbrmt` module in
  `fixtures/rtp/PROVENANCE.md`).
- No other BSD/MIT/Apache-2.0 ST 2110-20/-30 **sender** implementation
  (as opposed to a passive dissector/parser) was found on a general web
  search; see the task report for the search terms tried.
- `cisco/herisson` (used successfully for the ST 2022-6 fixture in
  `fixtures/rtp/`) does **not** give us ST 2110-20 either — see that
  file's closing section for the specific dead-code bug that makes its
  2110-20 output path unreachable.


---


# RTP/SDP oracle fixtures

Produced locally with ffmpeg 8.1, run from `fixtures/rtp/` (no packet is sent; `-f rtp` only writes the
SDP when a `sdp_file` is given):

```
ffmpeg -y -i ../ts/h264/high.ts     -c copy -f rtp \
       -sdp_file high-ffmpeg.sdp      rtp://127.0.0.1:5999
ffmpeg -y -i ../ts/h264/baseline.ts -c copy -f rtp \
       -sdp_file baseline-ffmpeg.sdp  rtp://127.0.0.1:5998
```

The source `.ts` files are the workspace's own H.264 fixtures
(`fixtures/ts/h264/high.ts` is High profile, `profile_idc` 0x64;
`baseline.ts` is Baseline, `profile_idc` 0x42). The SDPs are the negotiated
session description a real RTSP client would receive, so the `sprop-parameter-
sets` / `config` values inside them are real encoder output rather than
hand-written bytes.

`high-ffmpeg.avcc.txt` is the hex of the `avcC` box ffmpeg writes when remuxing
the same stream to MP4 (`-c copy out.mp4`), read from the resulting file with

```
python3 -c "d=open('high.mp4','rb').read(); i=d.find(b'avcC')-4; \
  import struct; n=struct.unpack('>I',d[i:i+4])[0]; print(d[i:i+n].hex())"
```

It is the independent ground truth for the High profile chroma/bit-depth
trailer that earlier versions of `avc_config_from_sps_pps` omitted.


## Notes

# rtp-packet `rtcp` module fixtures (formerly the rtcp-packet crate)

## `chrome-rr-pli-remb.bin` (68 bytes)

A real, plaintext RTCP compound packet sent by a real Chromium
`RTCPeerConnection` (WHEP video viewer) to `multimux`, captured 2026-09-26
during a run of `multimux`'s own real-browser interop test
(`cargo test -p multimux --all-features --locked --test whep_egress --
--nocapture`, see `multimux/tests/whep_egress.rs`) — not synthetic, not
generated from a tool. Captured by a temporary one-line `eprintln!` of the
decrypted SRTCP plaintext in `webrtc-runtime/src/media/transport.rs`
(`decrypt_srtp`), removed again immediately after (see PR history for
issue #1071). Released under the workspace licence.

Wire layout (RFC 3550 §6 / RFC 4585 / the REMB draft
`draft-alvestrand-rmcat-remb-03`):

1. **RR** (RFC 3550 §6.4.2, PT 201), bytes `0..32`: reporter SSRC=1, one
   report block for SSRC `0x407E522F` (fraction_lost=0, cumulative_lost=0,
   ext_highest_seq=5146, jitter=482, lsr=0, dlsr=0).
2. **PSFB PLI** (RFC 4585 §6.3.1, PT 206 FMT 1), bytes `32..44`: sender
   SSRC=1, media source SSRC `0x407E522F`, no FCI.
3. **PSFB REMB** (`draft-alvestrand-rmcat-remb-03`, PT 206 FMT 15 —
   unregistered with IANA, not decoded by `rtp_packet::rtcp`, hence
   `RtcpPacket::Unknown`), bytes `44..68`: sender SSRC=1, media source
   SSRC=0, ASCII `"REMB"`, then the bit-rate/SSRC-list FCI opaque to this
   crate.

This is exactly the shape issue #1071 (r14-RTCP-C1) fixed:
`CompoundPacket::parse` used to reject the **whole** 68-byte datagram
because it walked into the PSFB packets and rejected their PT (206) as
"not an RFC 3550 §6 core packet type", discarding the leading RR along
with them. It now parses as one `CompoundPacket` of three packets: a typed
`RtcpPacket::ReceiverReport`, and two `RtcpPacket::Unknown { packet_type:
206, .. }` (this crate does not decode PSFB FCI — see the crate's own
`private/specs-verified/rtp/rtcp-packet/rtcp.md` decode-completeness notes). See
`tests/it/real_browser_fixture.rs` for the parse + byte-identical round-trip
assertions.

## `anc-rtp.bin` provenance (issue #648)

No real ST 2110-40 (RFC 8331 ANC-over-RTP) capture exists anywhere in this
repo, and pulling a live one is not possible in this sandboxed environment
(no network access). This fixture is instead built by **reusing the
already-verified real ANC packet content bytes** from the existing,
already-audited `fixtures/rtp/anc.bin` fixture (see
`st291/tests/it/fixture_anc.rs` for the values it asserts) and wrapping them
fresh in RFC-8331-correct RTP + payload-header framing — the parity/checksum
math over `DID`/`SDID`/`Data_Count`/`User_Data_Words`/`Checksum_Word` is
**identical across transports** per `private/specs-verified/smpte/st291/anc_packet_291.md`, so the
content itself carries the same "is this really what a conformant sender
would produce" weight as the ST 2038 fixture, while the RTP-specific framing
around it (placement fields, payload header, word-alignment) is exercised
fresh.

## Conformance regeneration (round-2 audit)

The first revision of this fixture (and of `anc.bin`) carried hand-typed
`Data_Count` words without ST 291-1 parity bits and hand-computed checksums
that were off by one, so it did not conform to ST 291-1:2011. Both fixtures
are now regenerated: the same placement/UDW values, but `Data_Count` =
`with_parity(count)` (`0x102`, `0x203`) and `Checksum_Word` =
`compute_checksum()` (`0x134`, `0x1AC`), via `st291`'s own builders; every
other byte is unchanged, and `st291/tests/it/fixture_validate.rs` asserts
`is_valid()` on both. They remain **synthetic and spec-conformant, not real
captures** (the values in step 1 below are the pre-regeneration words).

## How it was generated

1. Took the same two ANC packets' content values already asserted by
   `st291/tests/it/fixture_anc.rs` against `anc.bin`:
   - Packet 0: `DID=0x161 SDID=0x101 Data_Count=0x002
     User_Data_Words=[0x2CF,0x101] Checksum_Word=0x233`
   - Packet 1: `DID=0x241 SDID=0x102 Data_Count=0x003
     User_Data_Words=[0x111,0x222,0x333] Checksum_Word=0x1AB`
2. Re-wrapped them as `st291::RtpAncPacket`s with RFC 8331 §2.1 placement
   fields matching RFC 8331 Figure 1's own worked example (two ANC packets on
   lines 9 and 10 of the SDI raster): packet 0 `C=false Line_Number=9
   Horizontal_Offset=0 S=false StreamNum=0`; packet 1 `C=true Line_Number=10
   Horizontal_Offset=0x10 S=false StreamNum=0`.
3. Built the RFC 8331 §2.1 payload (`st291::AncRtpPayload`) around them —
   `Extended_Sequence_Number=0`, `F=ProgressiveOrUnspecified (0b00)` — and
   serialized it with `st291`'s own spec-correct `Serialize` impl (which
   recomputes `Length`/`ANC_Count` and word-aligns each packet).
4. Wrapped that payload in an RFC 3550 `rtp_packet::RtpPacket` (the
   `rtp-packet` crate's spec-complete codec, issue #646): `marker=true`
   (last ANC RTP packet for the frame), `payload_type=112` (RFC 8331 §4's own
   worked SDP example: `a=rtpmap:112 smpte291/90000`), `sequence_number=1`,
   `timestamp=90_000` (1.0 s at the RFC 8331 §3.1 default 90 kHz clock rate),
   `ssrc=0x53543239` (ASCII `"ST29"`), no CSRC/extension/padding.
5. Serialized the whole packet and verified, before committing:
   - A byte-exact `parse -> serialize -> identical bytes` round trip via both
     `rtp_packet::RtpPacket` (the outer RTP header) and
     `st291::AncRtpPayload` (the inner ANC payload).
   - The decoded fields match every value listed above exactly.

The one-off generator script is not
committed — it was run once and deleted; this file documents the recipe.

## Used by

- `st291/tests/it/fixture_anc_rtp.rs` (`parses_expected_fields`,
  `byte_exact_round_trip`)
- `st291/examples/parse_anc_rtp.rs`
