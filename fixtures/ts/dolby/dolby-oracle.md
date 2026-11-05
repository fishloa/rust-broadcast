# Dolby (AC-3 / E-AC-3) fMP4 fixture oracle — #426

Real ffmpeg-encoded fixtures + the **byte-exact** `dac3`/`dec3` box bodies that
ffmpeg writes when muxing the same stream to MP4. The gate builds these boxes from
the elementary-stream syncframe BSI and must reproduce the oracle bytes exactly.

Regenerate the oracle:
```bash
ffmpeg -y -i fixtures/ts/dolby/ac3.ts  -c:a copy -f mp4 /tmp/ac3.mp4
ffmpeg -y -i fixtures/ts/dolby/eac3.ts -c:a copy -f mp4 /tmp/eac3.mp4
# then find the 'dac3' / 'dec3' box inside the 'ac-3' / 'ec-3' sample entry.
```

## `fixtures/ts/dolby/ac3.ts` — AC-3

- Stream: 44100 Hz, 1 channel (mono), 192 kbps (ffprobe).
- Sample entry fourcc: **`ac-3`**. Contains a **`dac3`** box (AC3SpecificBox, ETSI
  TS 102 366 §F.4).
- **`dac3` body = `50 09 40`** (3 bytes). Bit layout (§F.4.1):

  | field | bits | value | meaning |
  |---|---|---|---|
  | fscod | 2 | 1 | 44.1 kHz |
  | bsid | 5 | 8 | AC-3 |
  | bsmod | 3 | 0 | complete main (CM) |
  | acmod | 3 | 1 | 1/0 (mono) |
  | lfeon | 1 | 0 | no LFE |
  | bit_rate_code | 5 | 10 | 192 kbps (= frmsizecod >> 1) |
  | reserved | 5 | 0 | — |

  These come from the AC-3 syncframe: `fscod`+`frmsizecod` in `syncinfo()`,
  `bsid`/`bsmod`/`acmod`/`lfeon` in `bsi()`. `bit_rate_code = frmsizecod >> 1`.

## `fixtures/ts/dolby/eac3.ts` — E-AC-3

- Stream: 44100 Hz, 1 channel, ~192 kbps.
- Sample entry fourcc: **`ec-3`**. Contains a **`dec3`** box (EC3SpecificBox, §F.6).
- **`dec3` body = `06 00 60 02 00`** (5 bytes). Bit layout (§F.6.1):

  | field | bits | value | meaning |
  |---|---|---|---|
  | data_rate | 13 | 192 | 192 kbps |
  | num_ind_sub | 3 | 0 | 1 independent substream |
  | — substream 0 — | | | |
  | fscod | 2 | 1 | 44.1 kHz |
  | bsid | 5 | 16 | E-AC-3 |
  | reserved | 1 | 0 | — |
  | asvc | 1 | 0 | — |
  | bsmod | 3 | 0 | CM |
  | acmod | 3 | 1 | mono |
  | lfeon | 1 | 0 | — |
  | reserved | 3 | 0 | — |
  | num_dep_sub | 4 | 0 | no dependent substreams |
  | reserved | 1 | 0 | (present because num_dep_sub == 0) |

  E-AC-3 `data_rate` (kbps) derives from the syncframe `frmsiz` + `fscod` +
  `numblkscod` (frame is `(frmsiz+1)*2` bytes; see §E.1.3.1.3).

## Gate contract (ungameable)

Parse the AC-3 / E-AC-3 elementary stream (demux the TS, find the `0x0B77`
syncword, parse `syncinfo()`+`bsi()` for AC-3 and the E-AC-3 syncframe BSI),
build the `dac3` / `dec3` box, and assert the box body equals the hex above
**byte-for-byte**. Then build the `ac-3` / `ec-3` sample entry via the init-segment
path and confirm the embedded config box matches. The numbers come from ffmpeg's
own muxer, not from the code under test.
