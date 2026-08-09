# Fixture provenance — `fixtures/atsc3/`

## `slt-lls-2019-01-07.bin` (363 bytes)

**Source**: `junhuac/libatsc3`
(`https://github.com/junhuac/libatsc3`) — "ATSC 3.0 NGBP Open Source
Library - Parse LMT, LLS and other signaling, object delivery via
ROUTE, video playback of MMT and DASH." The bytes come from the
hex-encoded string literal returned by `_get_2019_01_07_slt_route_dash()`
in `src/atsc3_lls_test.c`, at commit
`a0e82a81d1ba15a4aebdb85008d34008344cc870` (2019-02-06,
"updates for ip and port selective filtering, adding in LLS parsing for
all test"):
`https://github.com/junhuac/libatsc3/blob/a0e82a81d1ba15a4aebdb85008d34008344cc870/src/atsc3_lls_test.c`.
Fetched 2026-08-08.

**Licence**: repository root `LICENSE` — **MIT License**, copyright (c)
2019 jjustman (confirmed via the GitHub API `license` field: `mit`).
MIT permits copying/redistribution with the copyright notice retained;
that notice is reproduced here: "Copyright (c) 2019 jjustman", MIT
License.

**Why this is a genuine capture, not a synthetic sample**: the
function's own name (`_get_2019_01_07_slt_route_dash`) encodes a
capture date, distinguishing it from the file's other, undated helper
`__get_test_slt()` used elsewhere in the same test file. The decoded
content (below) carries real-world markers a hand-built happy-path
fixture would have no reason to include: `slsMajorProtocolVersion`/
`slsMinorProtocolVersion` attributes actually present on the wire (our
crate defaults these when absent — see `atsc3/src/slt.rs` — so their
presence here is only explainable by a real encoder emitting them),
a `bsid="0"` (an encoder default/placeholder value, not a value anyone
constructing a clean example would pick), and generic non-branded
service names (`NATNL`/`NATN2`, i.e. "national" test channels 45.1/45.2)
consistent with an ATSC 3.0 pilot/lab broadcast rather than a real
commercial affiliate — plausible for the January 2019 timeframe (ATSC
3.0 field trials were underway; Phoenix/Cleveland-area pilots and
industry lab tests were both active then), though the specific source
site cannot be confirmed beyond what the repository itself records.

**Byte layout** (`LLS_table()` common envelope, A/331 §6.2 Table 6.1 —
see `atsc3/docs/a331-signalling.md`):

| Offset | Bytes | Field | Value |
|---|---|---|---|
| 0 | `01` | `LLS_table_id` | `1` = SLT |
| 1 | `01` | `LLS_group_id` | `1` |
| 2 | `00` | `group_count_minus1` | `0` (group_count = 1) |
| 3 | `15` | `LLS_table_version` | `0x15` = 21 |
| 4..363 | `1f 8b 08 08 …` | payload | gzip (RFC 1952) — magic `1f 8b`, matches |

**Decompression verified**: `gzip.decompress()` (Python stdlib) on the
359-byte payload succeeds and yields well-formed UTF-8 XML — the exact
path `b"payload-bytes"` (the byte string the pre-existing hand-invented
test used) could never exercise, since it is neither valid gzip nor
valid XML. Decompressed content (`SLT` root, A/331 §6.3 Table 6.2):

```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<SLT xmlns="tag:atsc.org,2016:XMLSchemas/ATSC3/Delivery/SLT/1.0/" bsid="0">
   <Service serviceId="11" majorChannelNo="45" minorChannelNo="1" serviceCategory="1" shortServiceName="NATNL" sltSvcSeqNum="0">
      <BroadcastSvcSignaling slsProtocol="1" slsMajorProtocolVersion="1" slsMinorProtocolVersion="0" slsDestinationIpAddress="239.255.1.1" slsDestinationUdpPort="49152" slsSourceIpAddress="192.168.59.62"/>
   </Service>
   <Service serviceId="12" majorChannelNo="45" minorChannelNo="2" serviceCategory="1" shortServiceName="NATN2" sltSvcSeqNum="0">
      <BroadcastSvcSignaling slsProtocol="1" slsMajorProtocolVersion="1" slsMinorProtocolVersion="0" slsDestinationIpAddress="239.255.1.2" slsDestinationUdpPort="49153" slsSourceIpAddress="192.168.59.62"/>
   </Service>
</SLT>
```

This parses cleanly with `atsc3::slt::Slt::parse` (two `Service`
elements, `slsProtocol="1"` = ROUTE, both services `LinearAv` category)
— see `atsc3/tests/fixture_slt.rs`. Note `sltSvcSeqNum` on each
`Service` is a real attribute this crate does not yet model (deferred,
per `atsc3/src/slt.rs`'s own doc comment listing `sltSvcSeqNum` among
"not yet modeled here" — this real fixture confirms it is a genuine,
in-the-wild attribute worth eventually adding, not a documentation
guess).

## `route-*.bin` (2020-11-05 real ATSC 3.0 ROUTE capture — LCT/ALC + SLS)

**Source**: `kansonkong/libatsc3` (`https://github.com/kansonkong/libatsc3`)
— the actively-maintained continuation of the same `libatsc3` project as
the SLT fixture above (same author, same MIT terms; see "Licence"
below). Files under `src/test/testdata/2020-11-17-signalling/`, at
commit `6cf0074e3daffd8e07174ff483bf2416a5485137` (2021-03-02, "Recover
missing merge from signed_lls, initial support for CMS verification of
SLS…"):
`https://github.com/kansonkong/libatsc3/blob/6cf0074e3daffd8e07174ff483bf2416a5485137/src/test/testdata/2020-11-17-signalling/ROUTE_SLS1.pcap`.
Fetched 2026-08-09.

**Licence**: repository root `LICENSE` — **MIT License**, "Copyright
(c) 2022 jjustman - jjustman@ngbp.org" (confirmed via the GitHub API
`license` field: `mit`, and by fetching and reading the LICENSE file
text directly). `kansonkong/libatsc3` is not a GitHub fork of
`junhuac/libatsc3` (`fork: false` for both), but its commit history is
authored and merged directly by Jason Justman (`jjustman`, company
"NGBP, LLC.") — the same person the earlier SLT fixture's copyright
notice names — continuing the same codebase years later (test-harness
paths, tooling, and code style are identical). MIT permits
redistribution with the copyright notice retained; reproduced here:
"Copyright (c) 2022 jjustman - jjustman@ngbp.org", MIT License.

**What was extracted**: the four `.bin` files below are individual UDP
payloads (i.e. exactly what would be the payload of an ALC/LCT packet
on the wire — Ethernet/IP/UDP headers stripped) pulled from
`ROUTE_SLS1.pcap` (9,573,960 bytes, not committed here — a raw Ethernet
`pcap` capture, `tcpdump`-verified `[udp sum ok]` on every extracted
frame, TTL=4, source `10.12.79.120` → destination `239.1.120.120:49152`,
captured 2020-11-05 20:01:24–25 UTC) using `scapy` to reassemble IP/UDP
and hand-parse the LCT header (RFC 5651 §5.1) to bucket packets by
`(TSI, TOI)`. The S-TSID recovered from the same capture (see below)
independently confirms the two media TSIs (3000, 3003) this session
uses, and the FDT-Instance XML confirms the TOI=0 FDT convention
(`flute.md`) — i.e. every structural claim below is corroborated from
two independent angles (the wire bytes AND the signalling XML that
describes them), not asserted from the bytes alone.

### `route-fdt-instance-2020-11-05.bin` (439 bytes)

One complete, unfragmented ALC/LCT packet: TSI=0, TOI=0 (frame 174 of
`ROUTE_SLS1.pcap`). Byte-exact decode of the LCT header (RFC 5651 §5.1)
and both header extensions:

| Field | Value | Notes |
|---|---|---|
| `V` / `C` / `PSI` | 1 / 0 / `10` | LCT v1; CCI=32 bits; PSI bit `X`=SPI=1 (source-data FEC Payload ID format, `alc.md` §"PSI bits") |
| `S` / `O` / `H` | 1 / 1 / 0 | TSI=32 bits, TOI=32 bits |
| `A` / `B` | 0 / 1 | Close-Object flag set — sender signalling end-of-object, consistent with this being a single-packet (non-fragmented) FDT-Instance |
| `HDR_LEN` | 9 (36 bytes) | fixed(4)+CCI(4)+TSI(4)+TOI(4)+extensions(20) = 36 |
| `CP` (Codepoint) | 4 | ROUTE Table A.3.6 value 4 = "Signed Package Mode" (`a331-route.md` §4) |
| TSI / TOI | 0 / 0 | Matches the FLUTE "TOI=0 reserved for FDT Instances" convention (`flute.md`) |
| Ext 1: EXT_FTI | HET=64, HEL=4 (16-byte ext, 14-byte HEC) | HEC = `00 00 00 00 01 8f 00 00 00 00 00 00 00 00`. Bytes 4-5 of the HEC decode as a 16-bit big-endian value `0x018F` = **399**, which is exactly the byte length of the FDT-Instance XML payload carried in *this same packet* (measured independently below) — strong corroborating evidence this is (or includes) a Transfer-Length-bearing Common FEC OTI field (RFC 5052 §6.2.4). The *exact* bit-packing of the remaining bytes (which is Reserved vs. FEC Instance ID vs. scheme-specific) is FEC-scheme-defined per RFC 5052 §6.3 and not independently confirmed here against a vendored FEC-scheme spec (RFC 5445 Compact No-Code is not vendored in this repo — same caveat `alc.md`/`norm.md` already carry) — reported as unresolved rather than guessed. |
| Ext 2: EXT_FDT | HET=192 (fixed-length) | Content = `20 00 00` → `V` (FLUTE version, high nibble) = **2**, FDT Instance ID (20 bits) = **0**. Exact byte-for-byte match to `flute.md`'s EXT_FDT layout (§"EXT_FDT — FDT Instance Header"). |
| FEC Payload ID | `00 00 00 00` | 32-bit `start_offset` = 0 (ROUTE §3.1 Compact No-Code source-flow format, `a331-route.md` §3.1) — consistent with a single, unfragmented object. |
| Payload | 399 bytes, `<?xml version="1.0" encoding="UTF-8"?>\r\n<FDT-Instance …>` | A real RFC 6726 §3.4.2 FDT-Instance: `Expires="4294967295"`, ATSC extension `afdt:efdtVersion="74"`, one `File` child: `TOI="458826" Content-Location="sls" Content-Length="6758" Content-Type="multipart/signed"`. |

### `route-media-video-fragment-2020-11-05.bin` / `route-media-audio-fragment-2020-11-05.bin` (1444 bytes each)

Two more complete ALC/LCT packets from the same capture (frames 1 and
17), one per media LCT channel: TSI=3000 (video) and TSI=3003 (audio),
both TOI=6034, `CP`=128 (ROUTE Table A.3.6: indirected through the
S-TSID's `SrcFlow.Payload@codePoint`, confirmed below to be `formatId=1`
File Mode, `frag=0`). Both carry an EXT_FTI (HET=64, HEL=4) whose 14-byte
HEC content is reported here **without** an asserted field decode — the
values (`00 00 00 06 ff f3 00 00 00 00 00 00 00 00` for TSI=3000;
`00 00 00 00 80 ee 00 00 00 00 00 00 00 00` for TSI=3003) don't fit the
same Transfer-Length-in-bytes-4-5 pattern the FDT packet showed, and
whether that's because they encode something else (e.g. these are
`rt="true"` real-time flows, where A/331 permits Transfer-Length = 0 /
unknown-streaming) is not established here — flagged as unresolved
rather than guessed. Both carry a 32-bit Compact-No-Code `start_offset`
FEC Payload ID (video: 107008; audio: 26752) followed by opaque
DASH-segment bytes (payload not decoded — correctly opaque to this
crate's scope). Sampling further consecutive packets on each TSI shows
the `start_offset` incrementing by exactly this packet's payload size
(1408 bytes) with no gaps — i.e. sequential, loss-free streaming in this
capture, independent confirmation this is a real, low-level-verified
capture rather than hand-built bytes.

### `route-sls-signed-package-2020-11-05.bin` (6758 bytes)

**Not** an ALC/LCT packet — this is the FEC-domain-reassembled content
of TOI=458826 (all 5 fragments carried on TSI=0, reassembled in
`start_offset` order; final size matches the FDT's declared
`Content-Length="6758"` exactly). A real `multipart/signed`
(`application/pkcs7-signature`) MIME package containing, in order: an
MBMS `metadataEnvelope` (referencing `mpd.xml`/`stsid.xml`/`usbd.xml`),
a DASH `mpd.xml`, the **S-TSID** (`application/route-s-tsid+xml`,
`stsid.xml`) — an `<RS>`/`<LS>` pair for `tsi="3000"`/`tsi="3003"` that
independently confirms the two media TSIs seen on the wire above, each
with an `EFDT`, `ContentInfo.MediaInfo` (`contentType="video"`/`"audio"`)
and `Payload codePoint="128" formatId="1" frag="0"` — and the **USBD**
(`application/route-usd+xml`, `usbd.xml`, a `BundleDescriptionROUTE`
with `BroadcastAppService`/`BasePattern` entries), plus a trailing
PKCS#7 signature part. **No `RepairFlow` element appears in the S-TSID**,
which is independently consistent with the wire capture: every ALC/LCT
packet across both `ROUTE_SLS1.pcap` and `ROUTE_SLS2.pcap` (8,400 and
6,485 frames respectively) has the LCT PSI "SPI" bit = 1 (source-data
format; `alc.md` §"PSI bits") — i.e. **this capture contains no
repair-flow (FEC-repair) packets at all**. This resolves the S-TSID/USBD
gap noted below as "not obtained" in an earlier pass of this file, but
does **not** provide the repair-flow example that would exercise the
RaptorQ FEC Payload ID path (`a331-route.md` §3.2) — that remains
unobtained; see below.

## What was not obtained

- **A repair-flow (FEC-repair) ALC/LCT packet.** Both `ROUTE_SLS1.pcap`
  and `ROUTE_SLS2.pcap` from the same capture session were scanned in
  full (8,400 + 6,485 frames) and every single packet on the ROUTE
  destination has PSI/SPI=1 (source-data format) — this lab session
  evidently ran without FEC repair enabled. No other permissively
  licensed, real ROUTE repair-flow capture was found. This is a genuine
  gap, not a decision to skip it: the RaptorQ FEC Payload ID layout
  (`a331-route.md` §3.2, `SBN`+`Encoding Symbol ID`) remains
  fixture-unverified.
- A second, independent real SLT capture (a distinct broadcaster's SLT,
  beyond `slt-lls-2019-01-07.bin` above) was not pursued — the ROUTE
  capture above already newly supplies independent S-TSID/USBD/MPD
  fixtures, which was the higher-leverage gap per #943 milestone 2/3.
