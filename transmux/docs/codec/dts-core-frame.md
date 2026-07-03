# DTS Coherent Acoustics — core substream frame header

Source: **ETSI TS 102 114 V1.6.1 (2019-08)**, §5.3 (Synchronization) and §5.4
(Frame header), Tables 5-1, 5-2, 5-4, 5-5, 5-7. This is the **elementary-stream**
layout the TS demuxer needs to frame-sync a DTS PES payload and recover
sample-rate / channel-count / samples-per-frame / frame-byte-size. The ISOBMFF
`ddts`/`DTSSpecificBox` side (built from these values on the mux path) is in
[`dts-isobmff-etsi102114.md`](dts-isobmff-etsi102114.md); TS carriage / PMT
signalling (stream_type `0x82`/`0x85`/`0x8A`) is in that doc's Annex F section.

## §5.3 Synchronization

A DTS bit stream is a sequence of audio frames, each beginning with a 32-bit
sync word. For a **core substream** the sync word is `0x7FFE8001` (big-endian,
16-bit core). (A core frame embedded in an extension substream uses `0x02B09261`;
a core inside EXSS is out of scope here — TS carriage of the standalone core is
`0x7FFE8001`.) The 6 bits after SYNC (FTYPE + SHORT) are `0b111111` (`0x3F`) for
normal frames — an optional extra synchronization check.

## §5.4.2 Core frame header — Table 5-1 (bit-stream header, in order, MSB-first)

| Field   | Bits | Notes |
|---------|-----:|-------|
| SYNC    | 32   | `0x7FFE8001` |
| FTYPE   | 1    | Table 5-2: `1` = normal frame, `0` = termination frame |
| SHORT   | 5    | deficit sample count; `31` for a normal frame |
| CPF     | 1    | CRC present flag (should be 0; if 1, a 16-bit HCRC follows the header block below) |
| NBLKS   | 7    | (NBLKS+1) PCM sample blocks; **samples per channel = 32 × (NBLKS+1)**. Valid 5..=127 (0..=4 invalid) |
| FSIZE   | 14   | **frame byte size = FSIZE + 1** (primary + extension). Valid 95..=16383 (0..=94 invalid) |
| AMODE   | 6    | Table 5-4: channel arrangement → channel count (CHS) |
| SFREQ   | 4    | Table 5-5: core sampling frequency |
| RATE    | 5    | Table 5-7: targeted bit rate (0b01110 = open/variable) |
| FixedBit| 1    | |
| DYNF    | 1    | dynamic range coeff present |
| TIMEF   | 1    | time-stamp present |
| AUXF    | 1    | auxiliary data present |
| HDCD    | 1    | |
| EXT_AUDIO_ID | 3 | |
| EXT_AUDIO | 1  | |
| ASPF    | 1    | audio sync-word insertion flag |
| LFF     | 2    | **low-frequency effects flag**: `0` = none, `1`/`2` = LFE present (adds 1 channel) |
| HFLAG   | 1    | |
| HCRC    | 16   | present **only if CPF == 1** |
| …       |      | primary-audio-coding header (FILTS, VERNUM, …) follows — not needed for framing |

Everything up to and including HFLAG is fixed-position; the demuxer only needs
SYNC…SFREQ (+ LFF for LFE) to frame and describe the stream. Bits are packed
MSB-first into a big-endian byte stream, so after the 4 sync bytes:
`FTYPE(1) SHORT(5) CPF(1) NBLKS[6](1)` = byte 4; `NBLKS[0:5](6) FSIZE[13:12](2)`
= byte 5; etc. — walk it with a big-endian bit reader.

## Table 5-2: FTYPE

| FTYPE | Frame type |
|------:|------------|
| 1 | Normal frame |
| 0 | Termination frame (partial; carries SHORT deficit) |

## Table 5-4: AMODE → channel count (CHS)

| AMODE | CHS | Arrangement |
|------:|----:|-------------|
| 0b000000 | 1 | A (mono) |
| 0b000001 | 2 | A+B (dual mono) |
| 0b000010 | 2 | L+R (stereo) |
| 0b000011 | 2 | (L+R)+(L−R) (sum-difference) |
| 0b000100 | 2 | LT+RT (left/right total) |
| 0b000101 | 3 | C+L+R |
| 0b000110 | 3 | L+R+S |
| 0b000111 | 4 | C+L+R+S |
| 0b001000 | 4 | L+R+SL+SR |
| 0b001001 | 5 | C+L+R+SL+SR |
| 0b001010 | 6 | CL+CR+L+R+SL+SR |
| 0b001011 | 6 | C+L+R+LR+RR+OV |
| 0b001100 | 6 | CF+CR+LF+RF+LR+RR |
| 0b001101 | 7 | CL+C+CR+L+R+SL+SR |
| 0b001110 | 8 | CL+CR+L+R+SL1+SL2+SR1+SR2 |
| 0b001111 | 8 | CL+C+CR+L+R+SL+S+SR |
| 0b010000–0b111111 | — | user defined |

Total channels = CHS + (1 if LFF ∈ {1,2} else 0).

## Table 5-5: SFREQ → core sampling frequency (Hz)

| SFREQ | Hz | | SFREQ | Hz |
|------:|---:|-|------:|---:|
| 0b0000 | invalid | | 0b1000 | 44100 |
| 0b0001 | 8000  | | 0b1001 | invalid |
| 0b0010 | 16000 | | 0b1010 | invalid |
| 0b0011 | 32000 | | 0b1011 | 12000 |
| 0b0100 | invalid | | 0b1100 | 24000 |
| 0b0101 | invalid | | 0b1101 | 48000 |
| 0b0110 | 11025 | | 0b1110 | invalid |
| 0b0111 | 22050 | | 0b1111 | invalid |

## Table 5-7: RATE → targeted bit rate (kbit/s), partial

`0..=0b01101` → 32,56,64,96,112,128,192,224,256,320,384,448,512,576; higher codes
add up to 4096; `0b11101` = open (variable). Only needed for the `ddts`
`max_bitrate`/`avg_bitrate` hint on the mux side.

## Demux derivation (what TS→IR needs)

For each core frame found at a `0x7FFE8001` sync:
- **frame length (bytes)** = `FSIZE + 1` → split the PES payload into frames.
- **sample_rate (Hz)** = Table 5-5[SFREQ] (reject invalid codes).
- **samples per frame** = `32 × (NBLKS + 1)` → sample duration @ 90 kHz =
  `samples_per_frame × 90000 / sample_rate` (per-frame interpolation, issue #556).
- **channel_count** = Table 5-4[AMODE].CHS + LFE(LFF).
- Feed these into `DtsSpecificBox` (`ddts`) for the fMP4 sample entry — see the
  derivation below and [`dts-isobmff-etsi102114.md`](dts-isobmff-etsi102114.md)
  §E.2.2.3. The core-only stream uses the `dtsc` sample-entry FourCC.

## `into_ddts` — building a core-only `DtsSpecificBox` from the core header

For a standalone DTS **core** substream (the `dtsc` case — no extensions),
ETSI TS 102 114 §E.2.2.3.2, Tables E-2/E-3/E-5:

| `DtsSpecificBox` field | Value from core header |
|---|---|
| `dts_sampling_frequency` | Table 5-5[SFREQ] (Hz) |
| `frame_duration` | code for samples/channel = 32×(NBLKS+1): 512→0, 1024→1, 2048→2, 4096→3 (256 → treat as 0 / smallest; only the four listed are valid `ddts` codes) |
| `stream_construction` | **1** (core substream only → codingname `dtsc`, Table E-2) |
| `core_lfe_present` | `LFF != 0` |
| `core_layout` | Table E-3 from AMODE (see below); `31` (= "use ChannelLayout") for arrangements not in the table |
| `core_size` | `FSIZE + 1` (core AU byte size) |
| `stereo_downmix` | `false` (not valid for mono/stereo; no embedded downmix info in the core header) |
| `representation_type` | `0` |
| `channel_layout` | `0` when `core_layout != 31` (core config carried by CoreLayout); otherwise the Table E-5 mask sum for the channels present |
| `pcm_sample_depth` | `16` |
| `max_bitrate` / `avg_bitrate` | Table 5-7[RATE] × 1000 (bits/s); `0` if RATE is the open/variable code `0b11101` |
| `multi_asset_flag`, `lbr_duration_mod`, `reserved_box_present` | `false` |

### AMODE → CoreLayout (Table E-3)

| CoreLayout | Description | matching AMODE |
|-----------:|-------------|----------------|
| 0 | Mono (1/0) | 0b000000 |
| 2 | Stereo (2/0) | 0b000010 |
| 4 | LT/RT (2/0) | 0b000100 |
| 5 | L,C,R (3/0) | 0b000101 |
| 6 | L,R,S (2/1) | 0b000110 |
| 7 | L,C,R,S (3/1) | 0b000111 |
| 8 | L,R,LS,RS (2/2) | 0b001000 |
| 9 | L,C,R,LS,RS (3/2) | 0b001001 |
| 31 | use ChannelLayout | any other AMODE |

Table E-5 channel-mask bits (for the CoreLayout=31 fallback): 0x0001 centre
front, 0x0002 L/R front, 0x0004 L/R surround side-rear, 0x0008 LFE, 0x0010 centre
surround rear, … (sum the bits for channels present).

For the committed `dts_core.ts` fixture (stereo 48 kHz core): SFREQ=0b1101 →
48000; AMODE=0b000010 → CoreLayout 2; LFF=0 → no LFE; frame_duration per NBLKS.
