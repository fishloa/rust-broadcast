# dvb-stream 0.11.0

_Released 2026-11-22._

Minor for a 0.x crate. No API change of its own: the crate builds against the wave's epochs (`broadcast-common` 14, `dvb-si` 16, `dvb-t2mi` 16, `mpeg-ts` 0.10, `rtp-packet` 0.9), which is the breaking part. The work is the audit-23 test pass over the adapters.

## Breaking changes

- Builds against new caret epochs of workspace siblings. A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; consumers must resolve a single copy of each at the jiff 1.0 epoch, and the `rtp_packet`/section types this crate takes are those versions' types.

## Tests

- **The RTP payload shape, the UDP T2-MI stream and the T2-MI resync counters are pinned.** Each `rtp_ts_payload` condition is exercised in isolation (an empty payload cannot index it, a whole-packet payload without a sync byte is not unwrapped, and a sync byte at a non-multiple length is left to the resync path), `UdpT2miStream` is constructed and drained over loopback with real T2-MI packets carrying a junk prefix, and `T2miEventStream::resync_stats` is asserted against a dirty input instead of only a clean one (audit 23 findings 125-127, #3433/#3434/#3435).
- **The receive-buffer fallback and `check_udp`'s refusals are pinned.** A refused `SO_RCVBUF` request goes through an injected socket factory (the bind is retried without it and returns the second result, not the first error), and `check_udp` accepts a matching-family multicast join, refuses a cross-family IPv6 group and refuses a join its own options reject (audit 23 findings 128-129, #3436/#3437).

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #4022 (test bullets under a heading).

---

Published from tag `dvb-stream-v0.11.0`.
