# Real E-AC-3 stream with a dependent substream - LOCAL ONLY, not committed

transmux's E-AC-3 dependent-substream handling (#1186, #1246) had only
spec-built tests (real independent frames from the Dolby capture plus
synthesised dependent frames), because none of the committed captures contains a
dependent substream. This directory documents the real one found and how to
reproduce the artifacts; **no media is committed** because the licence of the
source is not stated.

## Source

`the_great_wall_7.1.eac3` (707 KB, 5.02 s): https://fate-suite.ffmpeg.org/eac3/the_great_wall_7.1.eac3,
the FFmpeg FATE sample for E-AC-3 7.1 (added 2018-03-27). The FATE suite states no
licence for its samples and this one is movie audio (ffprobe: "Dolby Digital Plus
+ Dolby Atmos", 7.1, 48 kHz), so it is treated as non-redistributable: `make.sh`
downloads it into the git-ignored `.test-streams/eac3-dep/`, and the test that uses
it skips loudly when it is absent.

Structure (walked with an independent script and by ffprobe / MP4Box): 157 access
units, each an **AC-3 syncframe** (bsid 6, 2304 bytes, independent substream 0)
followed by an **E-AC-3 dependent syncframe** (`strmtyp` 1, `substreamid` 0, bsid 16,
2304 bytes, `chanmape` = 1, `chanmap` = Ls, Rs, Lrs/Rrs). Per ETSI TS 102 366
§E.1.3.1.2 an AC-3 stream inside an E-AC-3 bit stream "shall be treated as an
independent substream assigned substream ID 0", and Annex J.1.5.2.2 allows bsid 6
or 8 for independent substream 0 when a dependent substream is present.

## Derived with real tools (by `make.sh`, local)

| File | Producer |
|---|---|
| `gw_mp4box.mp4` | `MP4Box -add the_great_wall_7.1.eac3` (GPAC 26.07): `ec-3`, `dec3` = `24 00 0c 0f 02 02` + Atmos extension `01 0c` |
| `gw_ffmpeg.mp4` | `ffmpeg -c copy -f mp4` (8.1.2): `dec3` = `12 00 0c 0f 02 10` |
| `gw_ffmpeg.ts` | `ffmpeg -c copy -f mpegts`: PMT `stream_type 0x87` + registration `EAC3`, one PES per access unit |
| `gw_gpac.ts` | `gpac -i ... -o gw_gpac.ts`: `stream_type 0x06` + DVB descriptor `0x7A` + registration `EAC3`, uneven PES sizes (9376, 8824, 269, 4622, ... bytes) so access units straddle PES boundaries |
| `gw_mp4box.gpac_inspect.txt` | `gpac -i gw_mp4box.mp4 inspect:deep:full`: per-sample dts/dur/size/CRC32 |

The two producers disagree on `data_rate` (MP4Box 1152 = both substreams,
ffmpeg 576 = independent only) and on `chan_loc` (MP4Box bit 1 = Lrs/Rrs,
ffmpeg bit 4 = Lsd/Rsd). The dependent frame's `chanmap` has Lrs/Rrs (Table
E.1.4 bit 6), which Table F.6.1 maps to `chan_loc` bit 1: MP4Box is right and
that is what transmux must produce.

## What was searched before downloading

Scanned for E-AC-3 syncframe chains with `strmtyp` 1 (also recognising AC-3 core
frames): `fixtures/` (all committed captures), `.test-streams/` (incl.
`new-candidates/shaka`, `tsduck-test`, the France TNT captures) and
`private/fixtures/`: **none** contains a dependent substream. FFmpeg FATE `eac3/`
lists one dependent-substream sample (this one); the other FATE E-AC-3 files are
single-substream. The Shaka sample noted in `.test-streams/new-candidates` has no
dependents either (187 frames, all `strmtyp` 0).
