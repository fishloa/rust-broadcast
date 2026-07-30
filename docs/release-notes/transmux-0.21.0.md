# transmux 0.21.0 — 2026-07-30

**Minor (breaking for 0.x caret compatibility).**

See the [lockstep 9.1.0 release note](v9.1.0.md) for the full summary.

## Breaking changes

- `EncryptConfig` gains a `constant_iv_senc: ConstantIvSenc` field (#783). Every struct-literal construction must add it. `ConstantIvSenc::Emit` (default) emits a `senc` box for constant-IV CBCS; `ConstantIvSenc::Omit` preserves the old `tenc`-only shape. `TrackEncryption::new` gains a fourth argument.
- All remaining un-annotated public enums now carry `#[non_exhaustive]` (#806): `Addressing`, `MediaKind`, `SgpdEntry`, `PreloadHintType`, `SampleEntryVariant`, `StblChild`, `MpdType`, `ColourType`, `MpegAudioLayer`, `StreamType`, `VvcNalUnitType`, `SmoothStreamType`, `FormatArg`, `CliError`, `Output`. Each now needs a wildcard match arm.

## Additive

- `InputDegradation` enum + `DemuxEvent::InputDegraded` (#778): `StreamingTsDemux` emits `TransportError` on TEI and `ContinuityGap` on real CC loss.
- `RtpPacket` public type exported from `transmux::rtp`.
