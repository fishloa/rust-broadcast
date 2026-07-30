# ISO/IEC 13818-1 (H.222.0) — T-STD Buffer Model (§2.4.2)

Transcribed from `private/specs/itu_t_h222_0_202308_mpeg2_systems.pdf`, PDF pages
39–43 (ITU-T H.222.0 v9, 2023-08). These are the values &rsquo;d by
`dvb-conformance`'s T-STD buffer model.

**Provenance.** PDF pages 39–43 extracted via markitdown; table cells verified
against pypdf `extract_text()` line-by-line on the individual pages. All buffer
sizes and rates cited below appear directly in the spec text (not inferred from
surrounding equations).

## §2.4.2.1 — General (PDF p. 39, symbol definitions)

| Symbol | Definition |
|--------|-----------|
| TBn | Transport buffer for elementary stream n |
| TBSn | Size of TBn, measured in bytes |
| TBsys | Transport buffer for system information for the program being decoded |
| TBSsys | Size of TBsys, measured in bytes |
| Rxn | Rate at which data are removed from TBn |
| Rxsys | Rate at which data are removed from TBsys |
| MBn | Multiplexing buffer for video elementary stream n |
| EBn | Elementary stream buffer for video elementary stream n |
| Bn | Main buffer for audio elementary stream n / system data |
| Bsys | Main buffer for system data |

## §2.4.2.4 — Buffering (PDF pp. 42–43)

### Transport buffer sizes

> The transport buffer size is fixed at **512 bytes**.
>
> — H.222.0 §2.4.2.4, PDF page 42 line 20

This applies to both **TBn** (per elementary stream) and **TBsys** (system
information). The symbol TBSsys is defined at PDF page 39 line 23.

### Transport buffer leak rates

Defined per stream type, PDF page 42 lines 6–11:

| Stream type | Rate | Spec text |
|-------------|------|-----------|
| Video | `Rxn = 1.2 × Rmax[profile, level]` | Profile/level-dependent (§2.4.2.4) |
| Audio (ISO/IEC 13818-7 ADTS) | See channel table below | §2.4.2.4 |
| Other audio | `Rxn = 2 × 10⁶ bits per second` | PDF page 42 line 6 |
| Systems data | `Rxn = 1 × 10⁶ bits per second` | PDF page 42 line 10 |

**Rxsys** is defined at PDF page 39 line 35 as "the rate at which data are
removed from TBsys." No separate numeric value is stated in the spec; TBsys is
the transport buffer for system data, so **Rxsys = 1 × 10⁶ bps** (the systems-
data Rxn) is the only rate consistent with §2.4.2.4.

### ISO/IEC 13818-7 ADTS audio rates (PDF page 40, Table)

| Channels | Rxn [bit/s] |
|----------|-------------|
| 1–2 | 2 000 000 |
| 3–8 | 5 529 600 |
| 9–12 | 8 294 400 |
| 13–48 | 33 177 600 |

### Main buffer sizes (PDF page 43)

| Buffer | Size | Spec text |
|--------|------|-----------|
| Bn (audio, ADTS 1–2 ch) | 3 584 bytes | PDF page 43 line 22 |
| Bn (audio, other) | `BSn = BSmux + BSdec + BSoh = 3584 bytes` | PDF page 43 line 31 |
| Bsys | **1 536 bytes** | PDF page 43 line 37: "The main buffer Bsys for system data is of size BSsys = 1536 bytes" |
| BSmux (audio) | 736 bytes | PDF page 43 line 34 |
| BSdec + BSoh (audio, max) | ≤ 2 848 bytes | PDF page 43 line 32–33 |

### Multiplexing and Elementary stream buffers (PDF pages 42–43)

**MBSn** (video only):
- Low/Main level: `MBSn = BSmux + BSoh + VBVmax[profile,level] − vbv_buffer_size`
- High-1440/High level: `MBSn = BSmux + BSoh`
- `BSoh = (1/750) seconds × Rmax[profile, level]`
- `BSmux = 0.004 seconds × Rmax[profile, level]`

**EBSn**: equal to `vbv_buffer_size` from the video sequence header (§2.4.2.4).

### Scope of dvb-conformance implementation

| Buffer | Implemented | Notes |
|--------|------------|-------|
| TBn (512 B) | Yes | Occupancy tracked for empty/delay checks; overflow deferred (needs Rxn) |
| TBsys (512 B, 1 Mbit/s) | Yes | Overflow, empty-interval, data-delay checks |
| Bsys (1536 B) | Deferred | Not yet modelled (needs descriptor parsing for section flow) |
| MBn / EBn / Bn | Deferred | Codec-dependent sizes from descriptors |

## Conformance thresholds (not from H.222.0)

The delay and empty-interval thresholds used by indicators 3.9 and 3.10 are
specified by **ETSI TR 101 290 v1.4.1 Table 5.0c**, not by H.222.0:

| Constant | Value | Source |
|----------|-------|--------|
| DATA_DELAY_LIMIT_SECS | 1 s | TR 101 290 indicator 3.10 |
| TB_EMPTY_INTERVAL_SECS | 1 s | TR 101 290 indicator 3.9 |
| TB_SYS_EMPTY_INTERVAL_SECS | 1 s | TR 101 290 indicator 3.9 |

The still-picture 60 s delay threshold (also indicator 3.10) is **not
implemented**: detecting still-picture PIDs requires PMT stream_type
parsing which is deferred pending descriptor support.
