# Teletext fixtures — provenance

## `teletext_subtitle_synthetic.txt`

Synthetic-but-spec-real EN 300 706 Teletext subtitle page (magazine 8, page
0x88): see the file's own header comment for the full generation recipe and
page plan. Bytes are constructed with this crate's own verified
`encode_hamming_8_4`/`encode_odd_parity`, then stored **bit-reversed**
(wire/LSB-first order — issue #1041): EN 300 706 transmits each byte
LSB-first, so a real DVB capture's PES bytes are the bit-reversal of the
spec's own canonical values. Used by `tests/webvtt_teletext_fixture.rs` and
`caption-convert/tests/teletext_to_webvtt_fixture.rs`.

## `teletext_subtitle_boxed.ts` + `teletext_subtitle_boxed.tsduck-oracle.srt`

Independent-tool oracle for issue #1041 (`tests/webvtt_teletext_tsduck_oracle.rs`).
`teletext_subtitle_synthetic.txt`'s oracle is this project's own reading of EN
300 706 and a hand-generated fixture; this pair instead cross-checks this
crate's Teletext decode against **TSDuck 3.44-4676's own independent Teletext
demux** (`tsp -P teletext`), which implements EN 300 472/706 (including its
own bit-reversal, `REVERSE_8`) from scratch.

### `teletext_subtitle_boxed.ts`

A synthetic, from-scratch MPEG-2 TS: PAT (service 1, PMT PID `0x100`) + PMT
(`PCR_PID 0x101`, one `stream_type 0x06` component on PID `0x101` carrying a
`teletext_descriptor`, tag `0x56`) + PES (`stream_id 0xBD`, `data_identifier
0x10`, `data_unit_id 0x03` SUBTITLE, `data_unit_length 0x2C`) frames carrying
the same magazine-8/page-0x88 plan as the `.txt` fixture above (header/erase,
row 20 "HELLO WORLD", row 22 "THIS IS A TEST", header/erase), each row's text
wrapped in EN 300 706's Start Box (`0x0B`)/End Box (`0x0A`) spacing-attribute
pair (clause 12.2) — the real-world convention TSDuck's own SRT extraction
requires to recognise boxed subtitle text (`tsTeletextDemux.cpp`,
`processTeletextPage`'s `0x0B`/`0x0A` scan); without it TSDuck reports zero
frames even though the address/Hamming/parity decode is correct.

Generated in two steps:

1. **PSI (PAT/PMT) via TSDuck's own compiler** (independent of this crate),
   `tstabcomp` (TSDuck 3.44-4676):

   ```
   tstabcomp --compile pat.xml -o pat.bin
   tstabcomp --compile pmt.xml -o pmt.bin
   ```

   `pat.xml`:
   ```xml
   <tsduck>
     <PAT transport_stream_id="1">
       <service service_id="1" program_map_PID="0x100" />
     </PAT>
   </tsduck>
   ```
   `pmt.xml` (the teletext_descriptor's exact byte layout —
   `lang(3="eng") + (teletext_type<<3|magazine) + BCD page` — is verified
   against `dvb-si/src/descriptors/teletext.rs` / EN 300 468 §6.2.44, and
   injected via TSDuck's `generic_descriptor` so this crate's own transcription,
   not TSDuck's differently-shaped `page_number` convenience field, is what's
   under test; confirmed round-trips as `teletext_type=0x02, magazine=0,
   page=136 (0x88), full page=888` via `tstables pmt.bin`):
   ```xml
   <tsduck>
     <PMT service_id="1" PCR_PID="0x101">
       <component stream_type="0x06" elementary_PID="0x101">
         <generic_descriptor tag="0x56">656e671088</generic_descriptor>
       </component>
     </PMT>
   </tsduck>
   ```

2. **TS/PES packetisation**: TSDuck has no XML-driven ES/PES builder, so the
   PAT/PMT sections above are wrapped into 188-byte TS packets, and the
   Teletext PES frames are built, by a small from-scratch Python script
   (standard MPEG-2 TS/PES framing: `pointer_field` + section for PAT/PMT;
   `00 00 01 BD` + `PES_packet_length` + `data_alignment_indicator=1` +
   PTS-only optional header + `PES_data_field` for each Teletext frame) — not
   committed (one-off), but fully specified above: any TS/PES muxer
   reproduces byte-identical output from the same PAT/PMT sections and frame
   plan.

### `teletext_subtitle_boxed.tsduck-oracle.srt`

TSDuck's own decode of `teletext_subtitle_boxed.ts`, recorded verbatim:

```
tsp -I file teletext_subtitle_boxed.ts -P teletext --pid 0x0101 \
    -o teletext_subtitle_boxed.tsduck-oracle.srt -O drop
```

(`--pid` explicit, matching the PMT's declared PID; the plugin also finds it
automatically via the PMT's `teletext_descriptor` with `--service` or no PID
option at all — verified to produce the same output.)
