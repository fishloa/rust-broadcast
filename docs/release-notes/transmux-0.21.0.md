# transmux 0.21.0 — 2026-07-30

**Minor (breaking for 0.x caret compatibility).**

See the [lockstep 9.1.0 release note](v9.1.0.md) for the full summary.

## Breaking changes

- `EncryptConfig` gains a `constant_iv_senc: ConstantIvSenc` field (#783). Every struct-literal construction must add it. `ConstantIvSenc::Emit` (default) emits a `senc` box for constant-IV CBCS; `ConstantIvSenc::Omit` preserves the old `tenc`-only shape. `TrackEncryption::new` gains a fourth argument.
- All remaining un-annotated public enums now carry `#[non_exhaustive]` (#806): `Addressing`, `MediaKind`, `SgpdEntry`, `PreloadHintType`, `SampleEntryVariant`, `StblChild`, `MpdType`, `ColourType`, `MpegAudioLayer`, `StreamType`, `VvcNalUnitType`, `SmoothStreamType`, `FormatArg`, `CliError`, `Output`. Each now needs a wildcard match arm.

## Additive

- `InputDegradation` enum + `DemuxEvent::InputDegraded` (#778): `StreamingTsDemux` emits `TransportError` on TEI and `ContinuityGap` on real CC loss.
- `RtpPacket` public type exported from `transmux::rtp`.

## Diagnosing a mixed-version graph

`transmux` 0.21 is a minor under 0.x, which is a **breaking** caret bucket
(`^0.20` and `^0.21` do not unify). A graph holding both has two copies of every
transmux type: a `Sample` from one is not the `Sample` the other's function wants,
trait impls bind to the wrong copy, and methods look like they do not exist.

One command names every path that pulls each copy:

    cargo tree -i transmux

Run it with `-d` to list only the duplicated packages:

    cargo tree -d

The fix is always the same: raise every crate in the cascade table in
[v9.1.0.md](v9.1.0.md) to the version listed there, in one commit. Mixing them
is not a configuration you can make work.
