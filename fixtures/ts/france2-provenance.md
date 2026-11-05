# `france2.ts` — multi-audio + multi-subtitle DVB capture

First 16000 TS packets (3,008,000 bytes, byte-exact copy — PSI/descriptors
verbatim) of an 8-second France 2 DVB-T capture, for the DVB-player track-picker
work (issue #582): every elementary stream carries the ES_info descriptors a
player selects tracks by.

One program, PMT PID `0x6E`:

| PID | stream_type | ES_info descriptors | meaning |
|---|---|---|---|
| 0x78 | 0x1B | `520101` | H.264 video (stream_identifier) |
| 0x82 | 0x06 | `0a04 667265` + `7a…` | audio, ISO-639 `fre`, E-AC-3 descriptor (0x7A) |
| 0x83 | 0x06 | `0a04 716164` + `7a…` | audio, ISO-639 `qad`, E-AC-3 |
| 0x84 | 0x06 | `0a04 716161` + `7a…` | audio, ISO-639 `qaa`, E-AC-3 |
| 0x8C | 0x06 | `59 08 667261 24 00010001` | DVB subtitling (desc 0x59) |
| 0x8E | 0x06 | `59 08 667261 14 00010001` | DVB subtitling (desc 0x59) |

Source: `rust-skyfire/fixtures/france2-8s.ts` (full 8 s), truncated to whole
packets for a leaner repo fixture.
