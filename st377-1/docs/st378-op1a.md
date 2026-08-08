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

## 1. Operational pattern definition (§4.2)

Two orthogonal dimensions:

| Dimension | OP1a value |
|---|---|
| **Item complexity** | Single Item — one Material Package SourceClip whose duration equals the File Package Sequence duration |
| **Package complexity** | Single Package — the Material Package references exactly one File Package |

## 2. General constraints (§5.1, Table 1)

| Constraint | Value |
|---|---|
| File Kind | MXF |
| Operational Pattern | 1a (Single Item Single Package) |
| Role | Continuous recording, exchange of A/V material as a single entity |
| Essence | Single Essence Container; OPQ qualifiers apply (§6.4) |
| Material Packages | **1** |
| Number of Material Package SourceClips | **1** |
| Top-level File Packages | **1** |
| Number of Essence container Types | **1** |
| Lower-level Source Packages | 0 or more |
| Partition limits | None |
| Body Partitions | Decoder required (encoder optional — §7.2.5) |
| Index Tables | Optional |
| Editing Support | None |
| Streaming Support | Per Operational Pattern Qualifiers (§6.4) |

## 3. Package constraints (§6.2)

1. The **material package shall have one SourceClip per track**.
2. The **file package shall have one track per essence element** in the
   essence container.
3. All tracks/sequences in both the material package and file package
   **shall have the same duration**.
4. The material package may define a different start time to the file
   package tracks (to allow a timecode offset on playout), but the
   *duration* of the material package and the file package **shall remain
   identical**.
5. Source packages, where present, define historical editing context.

## 4. OP1a Universal Label (§6.3, Table 2)

The Operational Pattern Identification UL in the Partition Pack and Preface:

| Byte(s) | Description | Value |
|---|---|---|
| 1–12 | MXF OP UL prefix (defined in ST 377M OP section) | `06.0E.2B.34.04.01.01.01.0D.01.02.01` |
| 13 | Item Complexity | `0x01` |
| 14 | Essence container Complexity | `0x01` |
| 15 | Qualifiers (application-dependent, see §6.4) | `0x0N` (N = qualifier bits) |
| 16 | Reserved | `0x00` |

Full OP1a UL (without qualifiers):
`06.0E.2B.34.04.01.01.01.0D.01.02.01.01.01.00.00`

## 5. Operational Pattern Qualifiers (§6.4, byte 15)

Each bit of byte 15 **shall** be correctly set per ST 377M to reflect the
essence container status:

| Bit | §6.4 | Meaning when set to 0 |
|---|---|---|
| bit 1 | §6.4.1 | Essence container is **internal** to the file |
| bit 2 | §6.4.2 | Essence container is **streamable** |
| bit 3 | §6.4.3 | Essence container has a **single** essence track |

When the respective condition is not met (e.g. external essence, not
streamable, multi-track), the corresponding bit is 1.

## 6. Minimum implementation (§6.5)

All ST 377M constraints apply unless overridden. The minimum OP1a file
contains:

### Header metadata
- **1** Preface set
- **1 or more** Identification sets
- **1** Content Storage set
- **1** Essence Container Data set

### Material Package (1)
- sets for the timecode track
- sets for each picture track (as required by the essence container)
- sets for each sound track (as required by the essence container)
- sets for each data track (as required by the essence container)

### File Package (1)
- sets for the timecode track
- sets for each picture track (as required by the essence container)
- sets for each sound track (as required by the essence container)
- sets for each data track (as required by the essence container)

The "sets" per track means: **Timeline Track → Sequence → SourceClip** (for
the Material Package) or **Timeline Track → Sequence → SourceClip +
EssenceDescriptor** (for the File Package).

NOTE — Descriptive metadata is optional but recommended.

## 7. Essence container issues (§7)

### 7.1 Essence container identification
The essence container UL is defined by the essence container spec
(e.g. MPEG, JPEG 2000, uncompressed). Recorded in:
- Preface → Essence Containers property (batch)
- all Partition Packs → Essence Container property
- the appropriate Essence Container Data set

### 7.2 Requirements

| §7.2.x | Rule |
|---|---|
| 7.2.1 Number of elements | No constraint — may be zero (metadata-only file permitted) |
| 7.2.2 Interleaving | For streaming: interleave over limited duration (typically 1 frame) |
| 7.2.3 Continuity | Continuous decoding of contiguous elements with no processing; descriptor properties constant for track duration |
| 7.2.4 Track count | Defined by picture/sound/data essence elements in the container |
| 7.2.5 Body partitions | **Optional** for encoder, **required** for decoder to handle |

## 8. Material Package ↔ File Package relationship (§4.3, Figure 2)

```
Material Package
  └─ Timeline Track (defines origin)
       └─ Sequence (defines duration)
            └─ SourceClip ──→ references File Package + track
                              (SourcePackageID, SourceTrackID)
                              (duration == File Package Sequence duration)
                              (start position may differ)

File Package (= Source Package = "Physical Package")
  └─ Timeline Track (defines origin)
       └─ Sequence (defines duration)
            ├─ SourceClip(s) ──→ may reference lower-level Source Packages
            └─ EssenceDescriptor (e.g. MPEG, CDCI, Wave)
                 └─ describes the essence container element
```

The Material Package SourceClip's `SourcePackageID` and `SourceTrackID`
identify the single File Package and the specific track containing the
essence. The duration of the Material Package SourceClip equals the File
Package Sequence duration.
