# `dts_core.ts` — DTS Coherent Acoustics core in MPEG-2 TS

Real DTS **core substream** (sync `0x7FFE8001`) muxed into an MPEG-2 Transport
Stream, for the transmux DTS TS→IR spoke (issue #560). Generated with ffmpeg's
`dca` encoder — not hand-built bytes.

## Command

```sh
ffmpeg -hide_banner -loglevel error \
  -f lavfi -i "sine=frequency=440:duration=2:sample_rate=48000" \
  -f lavfi -i "sine=frequency=660:duration=2:sample_rate=48000" \
  -filter_complex "[0:a][1:a]amerge=inputs=2[a]" -map "[a]" \
  -c:a dca -strict -2 -ac 2 -b:a 768k \
  -f mpegts dts_core.ts
```

Two distinct tones (L 440 Hz / R 660 Hz) so the two channels are distinguishable.

## Oracle (ffprobe)

- Container: MPEG-2 TS, program 1, audio PID `0x0100`, **stream_type `0x82`**
  (DVB user-private DTS, ETSI TS 101 154 §G).
- Codec: `dts` (DCA core), **48000 Hz**, **stereo (2ch)**, ~1.984 s.
- Size: 219960 bytes = 1170 × 188-byte TS packets (whole packets).
- Core frame: sync `0x7FFE8001`; SFREQ `0b1101` (48 kHz); see
  [`../../../transmux/docs/codec/dts-core-frame.md`](../../../transmux/docs/codec/dts-core-frame.md)
  for the frame-header layout used to parse it.
