# dvb-si 16.0.0

_Released 2026-11-22._

Lockstep major (15.1 -> 16.0). Three things move at once: `num_enum` is gone from the public API, the `jiff` feature decodes jiff 1.x types, and the crate builds against `broadcast-common` 14 and `mpeg-ts` 0.10. The one new API is `epg::PresentEventTracker`, the EIT present/following tracker that `dvr-runtime` and `multimux` now use to mark programme boundaries.

## Breaking changes

- `num_enum` is no longer a dependency (#3963). `TableId`, `DescriptorTag` and `tables::sat::SatTableId` implement a hand-written `TryFrom<u8>` whose error is this crate's own `Error::InvalidValue` (naming `table_id` / `descriptor_tag` / `satellite_table_id` and the rejected value) instead of `num_enum::TryFromPrimitiveError<T>`. `TableId` and `SatTableId` gain `From<_> for u8`. Migration: match on `dvb_si::Error`; a reserved `table_id` is `Error::InvalidValue`, not a foreign wrapper.
- `jiff` 0.2 -> 1.0: the `jiff` feature's decoded times and durations (`jiff::Timestamp`, `jiff::SignedDuration`, the `epg` module) are jiff 1.x types. Migration: depend on `jiff` 1.x and move the crates that share these values with it.
- Builds against `broadcast-common` 14 and `mpeg-ts` 0.10 (the `ts` feature). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; a consumer must resolve a single copy of each.

## Added

- `epg::PresentEventTracker` follows one service's EIT present/following present event (ETSI EN 300 468 §5.2.4, Table 6) and reports each programme boundary as `PresentEventChange::Baseline` or `::Transition`. Only present/following *actual* sections for the tracked `service_id` count; the present event is the one whose `running_status` is Running (not the first in the loop), and a section naming the same `event_id` again changes nothing, so neither a repeated section nor a re-versioned copy of the same event rolls a programme (#3677). `feed_ts` queues up to `PRESENT_CHANGE_QUEUE_CAP` (16) changes for `poll_change`, counting older drops in `changes_dropped`; `on_section` reports a section's change directly (#3926). `EpgEvent` gained a crate-private constructor over `tables::eit::EitEvent` that shares the existing descriptor decoding.

## Fixed

- `PresentEventTracker::feed_ts` recovers packet alignment through `mpeg_ts::resync::TsResync` instead of cutting 188-byte packets from the first byte it ever saw. A stream that starts mid-packet, or uses 192-byte M2TS or 204-byte Reed-Solomon framing, now tracks its EIT sections instead of mis-framing every packet and silently reporting nothing (#3988).
- The tracker's docs cite the EIT layout and `running_status` transcriptions under `private/specs-verified/dvb/dvb-si/` (#3997).

## Changed

- The H.222 descriptor bit writer is an adapter over `broadcast_common::bits::BitBuf`; output bytes and errors are unchanged (#3961).
- The `wire_enum!` macro now comes from `broadcast_common`, with this crate's `Error` as its error type; generated enums and conversions are unchanged (#3989).
- README names `epg::PresentEventTracker` in the task recipes and in the `jiff` feature row (#4014).

## Tests

The 10 s DVB-T capture drives `feed_ts` at odd read sizes, starting mid-packet, and reframed as 192-byte M2TS and 204-byte Reed-Solomon packets, each reporting the one `Baseline`; re-signalling the capture's own following event as running reports exactly one `Transition` and the same section again none; the tracker's `Debug` names its service, tracked event and drop count (#3988, #4000). The rest of the section's tests pin the surviving mutants of the audit: section-length floors, error payloads, reserved-range arms, label literals, SAT byte vectors, DSM-CC round trips and the SI demux's version/CRC gate.

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #3988 (TS alignment), #3989 (`wire_enum!` one home), #3997 (transcription citations), #4000 (transition and `Debug` coverage), #4014 (README), #4017 (Breaking/Changed split).

---

Published from tag `dvb-si-v16.0.0`.
