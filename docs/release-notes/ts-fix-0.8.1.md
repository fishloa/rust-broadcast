# ts-fix 0.8.1

_Released 2026-11-10._

Patch release: stuffing, continuity, pad-rate and CLI fixes.

Version: 0.8.0 -> 0.8.1.

## Added
- `Error::InvalidPadRate` (the enum is `#[non_exhaustive]`) and `Stuffing::MAX_PACKETS_PER_INPUT`.

## Fixed
- Stuffing now runs before `restamp_pcr` and `honor_pcr_discontinuity` (it was last), so a PCR describes the byte position of the packet in the output instead of its position before nulls were inserted or dropped; `restamp_pcr(from_bitrate)` combined with `pad_to` or `drop_nulls` produced PCRs off by the stuffing ratio.
- `repair_continuity` keeps a legal duplicate of a repaired packet a duplicate (it carries the CC the original was given) instead of renumbering it into a second distinct packet.
- `repair_continuity` records the flagged packet's CC at a signalled discontinuity, so the following packet is checked against it (it was accepted with any CC).
- `Stuffing::pad_to` with a non-finite, below-1.0 or above-`Stuffing::MAX_PACKETS_PER_INPUT` rate fails `build()` with `Error::InvalidPadRate` instead of looping forever (`--pad-to inf`).
- `PidFilter::service` ignores PAT, PMT and CAT sections with `current_next_indicator = 0`.
- The CLI writes the trailing aligned packets of the input, and a capture shorter than five packets, instead of an empty file.

## Changed
- Internal: the service-extract filter state is boxed (clippy `large_enum_variant` after `SectionReassembler` grew a continuity-counter field).

## Dependencies
- `mpeg-ts` 0.7.0 -> 0.7.1.
- `mpeg-pes` 0.7.0 -> 0.7.1.
- `scte35-splice` 5.0 -> 5.0.1.

---

Published from tag `ts-fix-v0.8.1`.
