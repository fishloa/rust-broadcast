# SMPTE ST 378M-2004 — MXF Operational Pattern 1A (Single Item, Single Package)

_Source: SMPTE 378M-2004 (Archived 2010), 10 pages, approved 2004-09-22._

OP1a is the **baseline** generalized operational pattern: a single playable
essence container with a single item of continuously decodable material.
All other generalized OPs are a superset of OP1a.

## Implementation status for this crate (read first, issue #937)

This crate types the OP1a structural metadata Sets — Material/Source
Package, Timeline/Event/Static Track, Sequence, SourceClip,
TimecodeComponent, FillerComponent — plus the OP1a Operational Pattern UL
identification helpers (`op1a` module, §4 below), validated against a real
`ffmpeg`-muxed OP1a file (`tests/fixtures/op1a_mpeg2_pcm.mxf`,
`tests/fixture_real_op1a.rs`).

It does **not** implement two things §6.5's minimum-file list below
requires:

- **`EssenceDescriptor`.** No typed Descriptor exists in this crate at all
  (F.2-F.6 are identified-but-generic, see `docs/st377-1.md`'s Scope
  section), and the real fixture's actual descriptors (an MPEG Video
  Descriptor and a Wave Audio Descriptor) are registered by essence-
  container-mapping specs *outside* ST 377-1, so typing ST 377-1's own
  generic Descriptor Sets would not even cover them.
- **A file assembler.** Nothing computes Partition byte offsets,
  `HeaderByteCount`/`IndexByteCount`, or emits a `RandomIndexPack` that
  points at real Partition offsets — see the crate root docs for detail.

So: this crate can parse and losslessly round-trip the structural metadata
of a real OP1a file's Header Metadata, but cannot build (or fully resolve)
a complete, playable OP1a file end to end.

---

> **Spec tables moved.** The transcribed ST 378M clauses (§4.2 pattern definition, §5.1/Table 1 constraints, §6 package
> constraints, §6.3/Table 2 OP1a UL, §6.4 qualifiers, §6.5 minimum implementation, §7 essence containers, §4.3 package
> relationship) are (c) SMPTE and live only in the private companion repo:
> `private/specs-verified/smpte/st377-1/st378-op1a.md` (old path `st377-1/docs/st378-op1a.md`, see
> `docs/specs/PRIVATE-MAP.md`).
