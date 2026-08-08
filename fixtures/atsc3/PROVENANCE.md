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

## What was not obtained

No second, independent real capture (e.g. an S-TSID/USBD SLS XML, or a
distinct broadcaster's SLT) was pursued — issue #926's minimum bar is
one real fixture per table, and the ROUTE/DASH-labelled capture above
already exercises the full LLS envelope + gzip decompress + SLT parse
path this crate implements today. Obtaining S-TSID/USBD fixtures is
tracked separately under #943 milestone 2/3 (same acquisition, higher
downstream leverage — deferred here to stay in scope for #926).
