# rmt-flute 0.4.0 — renamed from `dvb-flute`

**No code change.** This release renames the crate and nothing else. Same
parsers, same API, same behaviour.

## Why

`dvb-flute` implemented **no DVB standard**. Every format in it is IETF RMT
(Reliable Multicast Transport):

| Component | Spec |
|---|---|
| `LctHeader` | RFC 5651 — Layered Coding Transport |
| `AlcPacket`, `FecPayloadId128` | RFC 5775 — Asynchronous Layered Coding |
| `ExtFdt`, `ExtCenc` | RFC 6726 — FLUTE |
| `NormCommonHeader`, `NormData`/`NormCmd`/`NormFeedback` | RFC 5740 — NORM |

The crate's own `description` and `keywords` never mentioned DVB — only the
name did. DVB is one *consumer* of these formats, alongside 3GPP MBMS/eMBMS
and ATSC 3.0 ROUTE.

The name became a concrete problem rather than a cosmetic one when ATSC 3.0
work was planned: A/331 Annex A defines ROUTE as a profile-and-delta on
RFC 5651/5775/6726, so a future `atsc3-route` crate would have depended on a
`dvb-*` crate for something neither ATSC nor DVB. Fixing the name before that
dependency existed was far cheaper than after.

## Migration

```diff
-dvb-flute = "0.3"
+rmt-flute = "0.4"
```

```diff
-use dvb_flute::{LctHeader, AlcPacket};
+use rmt_flute::{LctHeader, AlcPacket};
```

Nothing else changes — no type, function or feature was renamed.

## Yanks

All `dvb-flute` versions (0.1.0, 0.1.1, 0.2.0, 0.3.0, 0.3.1) are **yanked**.
No compatibility shim is published, matching the approach taken when
`smpte2038`/`dvb-smpte2038` became `st291`.

There were **zero reverse dependencies** on crates.io when the rename was
made, so no published crate is affected. Anyone with an existing `Cargo.lock`
pinning a yanked version continues to build; new resolutions must move to
`rmt-flute`.

## Version choice

0.4.0 continues the existing line rather than restarting at 0.1.0. The code is
five releases in, adversarially audited, fuzzed, and backed by real committed
fixtures — publishing it as `0.1.0` would signal "new and unproven", which is
not true. This matches the workspace's prior renames: `smpte2038` → `st291`
continued at 0.2.0, and `ll-hls-runtime` → `hls-runtime` continued at 0.4.0.

The bump is minor rather than patch because a rename is breaking for
consumers, and for a 0.x crate the minor position is the breaking axis.
