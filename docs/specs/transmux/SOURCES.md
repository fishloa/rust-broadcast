# Sources used for transmux box documentation

## Explicit statement

All field tables in this documentation set are traced to the sources listed below.
ISO/IEC 14496-14 and ISO/IEC 14496-15 are mentioned only as the normative home of codec-config
boxes that could NOT be transcribed from a free source (they appear as GAP entries in COVERAGE.md).
No copy — authorized or unauthorized — of those standards was downloaded, fetched, or consulted.

**Exception:** ISO/IEC 14496-12 §8.8 (Movie Fragments) was used as a normative source for
fragmentation-control boxes (`mvex`, `trex`, `moof`, `mfhd`, `traf`, `tfhd`, `trun`) per
explicit owner direction in the project brief.  These are transcribed in `fragment-boxes.md`.

---

## Sources actually used

### 1. Apple QuickTime File Format specification
- **URL base:** `https://developer.apple.com/documentation/quicktime-file-format/` (SPA, redirect from legacy archive URL)
- **Machine-readable markdown endpoint (actually fetched):**  
  `https://developer.apple.com/tutorials/data/documentation/quicktime-file-format/<Slug>.md`  
  This endpoint returns valid `text/markdown` content for each atom page.  
  Verified working as of 2026-06-29.
- **License:** Apple developer documentation, publicly accessible without authentication.
- **What it covered:** All init-segment boxes whose layout Apple defines:  
  `ftyp`, `moov` (container), `mvhd`, `trak` (container), `tkhd`, `mdia` (container),
  `mdhd`, `hdlr`, `minf` (container, video and sound variants), `vmhd`, `smhd`,
  `dinf` (container), `dref` (outer atom structure), `stbl` (container), `stsd`,
  `stts`, `stsc`, `stsz`, `stco` / `co64`.
- **Specific pages fetched:**
  - `/Movie_header_atom.md` — mvhd field table
  - `/Track_header_atom.md` — tkhd field table
  - `/Media_header_atom.md` — mdhd field table
  - `/Handler_reference_atom.md` — hdlr field table
  - `/Video_media_information_atom.md` — minf (video) container structure
  - `/Sound_media_information_atom.md` — minf (sound) container structure
  - `/Video_media_information_header_atom.md` — vmhd field table
  - `/Sound_media_information_header_atom.md` — smhd field table
  - `/Data_information_atom.md` — dinf container description
  - `/Media_data_reference_atom.md` — dref outer container (no sub-fields transcribed; field page was empty)
  - `/Sample_description_atom.md` — stsd field table
  - `/Sample_table_atom.md` — stbl container listing
  - `/Time-to-sample_atom.md` — stts field table
  - `/Sample-to-chunk_atom.md` — stsc field table
  - `/Sample_size_atom.md` — stsz field table
  - `/Chunk_offset_atom.md` — stco field table
  - `/File_type_compatibility_atom.md` — ftyp field table

### 2. 3GPP TS 26.244 V19.1.0 (2025-12) — 3GPP file format (3GP)
- **URL:** `https://www.3gpp.org/ftp/Specs/archive/26_series/26.244/26244-j10.zip`  
  (3GPP FTP server — 3GPP publishes all specs free of charge)
- **File extracted:** `26244-j10.docx` (Word document, 185 KB)
- **License:** 3GPP open-access technical specification.
- **What it covered:**
  - Section 13.2: `styp` (Segment Type Box) — identical format to `ftyp`, just box type differs
  - Section 13.4: `sidx` (Segment Index Box) — full C-style syntax with all field semantics
  - Section 13.5: `tfdt` (Track Fragment Decode Time Box) — full C-style syntax with field semantics
  - Conformance profile §5.4.10 (Adaptive Streaming): which boxes are required in init segment
    (`moov` + `mvex`) and media segment (`moof` + `mdat`)
  - Note: the spec cites ISO/IEC 14496-12 [7] for `moof`, `mfhd`, `traf`, `tfhd`, `trun`,
    `trex`, `mvex`, and base containers — it does NOT reproduce their field-level syntax.

### 3. W3C ISO BMFF Byte Stream Format specification
- **URL:** `https://www.w3.org/TR/mse-byte-stream-format-isobmff/`  
  (W3C Group Note, published July 2024 — free)
- **What it covered:**
  - Init segment structure: required sequence `ftyp` → `moov` (with `mvex`).
  - Media segment structure: optional `styp` → `moof` (with at least one `traf` containing `tfdt`) → one or more `mdat`.
  - Error conditions for malformed segments (e.g. missing `mvex`, missing `tfdt`, missing `traf`).
  - Movie-fragment relative addressing requirement (`default-base-is-moof`).
  - Random access point types (SAP type 1 or 2).

### 4. AOM AV1 ISOBMFF Binding specification
- **URL:** `https://aomediacodec.github.io/av1-isobmff/`  
  (Alliance for Open Media, publicly available)
- **What it covered:**
  - `av01` sample entry syntax (extends `VisualSampleEntry`)
  - `av1C` (`AV1CodecConfigurationBox`) — complete `AV1CodecConfigurationRecord` bit-field layout
    (marker, version, seq\_profile, seq\_level\_idx\_0, seq\_tier\_0, high\_bitdepth, twelve\_bit,
    monochrome, chroma\_subsampling\_x/y, chroma\_sample\_position, initial\_presentation\_delay)

### 5. ISO/IEC 14496-12:2015 — ISO Base Media File Format (expanded)
- **Status:** Used as normative source per owner direction.
- **Additional sections used in this round:**
  - §8.3.3: `tref` (Track Reference Box)
  - §8.5.2: `btrt` (Bit Rate Box) — class BitRateBox
  - §8.6.1.3: `ctts` (Composition Time to Sample Box)
  - §8.6.2: `stss` (Sync Sample Box)
  - §8.6.4: `sdtp` (Independent and Disposable Samples Box)
  - §8.6.5: `edts` (Edit Box)
  - §8.6.6: `elst` (Edit List Box)
  - §8.7.7: `subs` (Sub-Sample Information Box)
  - §8.7.8: `saiz` (Sample Auxiliary Information Sizes Box)
  - §8.7.9: `saio` (Sample Auxiliary Information Offsets Box)
  - §8.8.9: `mfra` (Movie Fragment Random Access Box)
  - §8.8.10: `tfra` (Track Fragment Random Access Box)
  - §8.8.11: `mfro` (Movie Fragment Random Access Offset Box)
  - §8.9.2: `sbgp` (Sample to Group Box)
  - §8.9.3: `sgpd` (Sample Group Description Box)
  - §8.12.1: `sinf` (Protection Scheme Information Box)
  - §8.12.2: `frma` (Original Format Box)
  - §8.12.5: `schm` (Scheme Type Box)
  - §8.12.6: `schi` (Scheme Information Box)
  - §8.15.3: `rinf` (Restricted Scheme Information Box)
  - §8.15.4: `stvi` (Stereo Video Box / stereoscopic video)
  - §8.16.5: `prft` (Producer Reference Time Box)
  - §12.1.4: `pasp` (Pixel Aspect Ratio Box), `clap` (Clean Aperture Box)
  - §12.1.5: `colr` (Colour Information Box)
  - §6.2.3: Box Order rules (Table 1 containment hierarchy)
- **What it covered (cumulative):**
  - `mvex`, `mehd`, `trex`, `moof`, `mfhd`, `traf`, `tfhd`, `trun` (from earlier round)
  - `tref`, `btrt`, `ctts`, `stss`, `sdtp`, `edts`, `elst`, `subs`, `saiz`, `saio`
  - `mfra`, `tfra`, `mfro`, `sbgp`, `sgpd`
  - `sinf`, `frma`, `schm`, `schi`, `rinf`, `stvi`
  - `prft`, `pasp`, `clap`, `colr`
  - Box Order rules + Table 1 hierarchy (27 new boxes this round)

### 6. MP4 Registration Authority (MP4RA)
- **URL:** `https://mp4ra.org/registered-types/boxes`  
  (MP4RA — the authoritative fourCC registry, free)
- **What it covered:** Enumeration and descriptions of all registered box fourCCs;
  confirmation that all target box types are registered and their defining spec is noted as `[ISO]`.

---

## Unreachable or blocked hosts

| URL | Status | Notes |
|-----|--------|-------|
| `https://www.3gpp.org/ftp/Specs/archive/26_series/26.244/` | **200 OK** | Directory listing worked; ZIP download worked |
| `https://developer.apple.com/library/archive/documentation/QuickTime/QTFF/QTFFChap2/qtff2.html` | **301** → SPA | Legacy URL redirects to SPA; raw content inaccessible via curl; markdown API used instead |
| `https://web.archive.org/…` | **Blocked** | Claude Code cannot access web.archive.org |
| `https://www.etsi.org/deliver/etsi_ts/126200_126299/126244/…` | **403** | Blocked; used 3GPP FTP instead |
