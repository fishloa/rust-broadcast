# `st337` real-fixture + oracle-cross-check provenance (issue #670)

## The real fixture: `tests/fixtures/eac3_frame0.bin`

834 real bytes: the **first E-AC-3 syncframe** of this workspace's own real,
ffmpeg-encoded capture `fixtures/ts/dolby/eac3.ts` (already used as a Dolby
fixture oracle for issue #426 — see `fixtures/ts/dolby/DOLBY-ORACLE.md`).

Extraction:

```bash
ffmpeg -y -i fixtures/ts/dolby/eac3.ts -c:a copy -f eac3 /tmp/extracted.ec3
python3 -c "
data = open('/tmp/extracted.ec3','rb').read()
idx = data.find(b'\x0b\x77', 2)   # offset of the 2nd sync word == frame 0's byte length
open('tests/fixtures/eac3_frame0.bin','wb').write(data[:idx])
"
```

`ffprobe -show_packets` independently confirms frame 0's `size=834` (matching
the byte-search above, not just a re-derivation of the same heuristic).

`tests/fixture_eac3.rs` wraps these 834 bytes in a hand-built **ST 337
four-word burst** (real `Pa`/`Pb` sync constants, `Pc` computed per the
verified §7.2.4 bit layout, `Pd = 834*8 = 6672` bits per the spec's literal
"length_code is in bits" text), parses it back with this crate, and asserts
the extracted payload is byte-identical to the committed 834-byte file.

## Independent cross-check oracle: `ffmpeg -f spdif`

`ffmpeg`'s `spdif` muxer implements IEC 61937 (the consumer analog of ST 337
explicitly acknowledged in ST 337 §8/Annex A/B), which shares this exact
burst-preamble structure. It was run as an independent oracle:

```bash
ffmpeg -y -i /tmp/extracted.ec3 -c:a copy -f spdif /tmp/out.spdif
xxd /tmp/out.spdif | head -4
```

First 8 bytes of `out.spdif` (one burst's preamble, **little-endian 16-bit
words**):

```
72 f8 1f 4e 15 00 42 03
```

Decoded (all values little-endian per word, matching this crate's own byte
convention — see judgment call below):

| Word | Bytes | LE value | Meaning |
|---|---|---|---|
| Pa | `72 f8` | `0xF872` | **matches** ST 337 Table 6's 16-bit-mode sync word 1 exactly |
| Pb | `1f 4e` | `0x4E1F` | **matches** ST 337 Table 6's 16-bit-mode sync word 2 exactly |
| Pc | `15 00` | `0x0015` | `data_type=21`, `data_mode=0`(16-bit), `error_flag=0`, `data_type_dependent=0`, `data_stream_number=0` — internally consistent with a 16-bit-mode, error-free, main-stream E-AC-3 burst |
| Pd | `42 03` | `0x0342` = 834 | **This is the frame's BYTE count, not its bit count** (834*8 = 6672 would be the bit count) |

### What this confirms

1. **Byte order**: this crate's chosen little-endian 2-bytes-per-word
   convention for representing the logical word stream is the one real
   software (`ffmpeg`) actually uses. (Nothing in ST 337's own text mandates
   an endianness for a byte-array *representation* of the word stream — that
   choice is inherent to any byte-oriented implementation of a word-based
   format — so this crate adopts the real, verified convention rather than
   inventing one.)
2. **Pa/Pb sync constants**: exact match to ST 337 Table 6's 16-bit-mode
   values, independently of the PDF transcription (i.e. two independent
   sources — the fetched, verified PDF text, and running software — agree).
3. **Pc bit layout**: `data_type=21` is the well-known IEC 61937 code point
   for E-AC-3 (also hard-coded in `ffmpeg`'s own `libavformat/spdifenc.c` as
   `IEC61937_EAC3 = 21`). This crate does **not** claim `21` is SMPTE ST 338's
   registered value for E-AC-3 (ST 338 was not available to verify — see
   `docs/st337.md` scope decision 3) — it is cited here only as the value a
   real, independent implementation used for the exact same real audio bytes,
   which is sufficient to validate the *bit-layout math* (data_type/
   data_mode/error_flag/data_type_dependent/data_stream_number packing into
   Pc), which is this crate's own responsibility.
4. **Payload byte-swap**: the payload bytes following `Pa-Pd` in `out.spdif`
   are the same 834 E-AC-3 bytes but with **every consecutive byte pair
   swapped** relative to their natural (big-endian-per-16-bit-word) order in
   the elementary stream (verified byte-for-byte for all 46 payload bytes
   checked). This is `ffmpeg`'s implementation of "pack the ES as 16-bit
   words, then write each word little-endian" applied uniformly to the
   payload, not something ST 337 itself mandates for `burst_payload` content
   (§7.5 explicitly defers all payload-content formatting to per-data-type
   specs). **This crate does not perform this swap** — `burst_payload` is
   treated as a fully opaque byte slice (see `docs/st337.md` — "parse the
   container, not the codec"), so `tests/fixture_eac3.rs` wraps the E-AC-3
   bytes **unswapped**. A caller wanting bit-identical interop with
   `ffmpeg`/IEC-61937-consuming hardware would need to apply this swap
   themselves before/after calling this crate — that's a codec/consumer-format
   transport detail, not part of ST 337's own burst framing.

### The one real discrepancy: `length_code` (Pd)

ST 337's own fetched, verified text (§7.2.5) is unambiguous:

> "The length_code shall indicate the length of the burst_payload **in
> bits**."

`ffmpeg`'s `Pd = 834` is the frame's **byte** count. `834 * 8 = 6672`
(`0x1A10`) would be the correct *bit* count. This is a real, documented,
independently-confirmed (via `ffprobe -show_packets`, not just a byte-search
heuristic) difference between:

- **SMPTE ST 337** (professional, what this crate implements) — bits, per
  its own literal text.
- **IEC 61937** (consumer, what `ffmpeg -f spdif` implements) — bytes, at
  least for the AC-3/E-AC-3 data type (this is a known, documented quirk of
  IEC 61937's "Burst-info" part for that data type family, distinct from the
  base/generic burst-info definition).

ST 337 §8.4 itself anticipates exactly this class of professional/consumer
divergence ("The coding of the burst_payload might also differ for some data
types..."). **This crate's `tests/fixture_eac3.rs` therefore does NOT try to
byte-match `ffmpeg`'s `Pd` value** — it computes `length_code = 6672` (bits)
per ST 337's own text, and documents the `ffmpeg` byte-count value here as
the cross-check finding it is, not a bug.

## Verdict

- Real E-AC-3 fixture wrap: **working** (`tests/fixture_eac3.rs`).
- `ffmpeg -f spdif` cross-check: **working**, and load-bearing — it
  independently confirmed the Pa/Pb sync constants, the byte-order
  convention, and the Pc bit-packing math against real running software,
  while also surfacing the genuine bits-vs-bytes `length_code` divergence
  documented above (which is a property of the *spec family*, not this
  crate).
