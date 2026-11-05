# Test fixtures

## The rule

Every fixture lives at `<tier>/<topic>/[<set>/]<file>`, where `<tier>` is `fixtures/`
(public, redistributable) or `private/fixtures/` (the private submodule). File and
directory names are kebab-case, except inside upstream packages and test suites
whose own names are kept. No path contains a crate name: a fixture is filed by what
it is, never by which crate reads it.

## Topics

The only directories permitted directly under either tier (the guard
`workspace-guards/tests/guards/fixtures_one_home.rs` holds the list in `TOPIC_DIRS`):

| Topic | What belongs there |
|---|---|
| `ts` | MPEG-2 transport streams and TS-level oracles |
| `ps` | MPEG-2 program streams |
| `pes` | Bare PES packets |
| `mp4` | ISO BMFF files: progressive, fragmented, CMAF, LL-CMAF |
| `mkv` | Matroska and WebM |
| `flv` | FLV files |
| `mxf` | MXF files |
| `hls` | M3U8 playlists and HLS segment sets |
| `dash` | MPDs and DASH segment sets |
| `smooth` | Smooth Streaming manifests and fragments |
| `si` | DVB/ATSC service information sections, tables, descriptors |
| `scte35` | SCTE 35 splice data and SCTE 214 carriage |
| `simulcrypt` | DVB SimulCrypt messages |
| `mabr` | DVB multicast ABR configuration |
| `atsc3` | ATSC 3.0 signalling |
| `cpix` | CPIX key-exchange documents |
| `rtp` | RTP/RTCP packets and captures |
| `rtsp` | RTSP session transcripts |
| `rtmp` | RTMP sessions |
| `srt` | SRT packets and handshakes |
| `rist` | RIST packets |
| `webrtc` | WHIP/WHEP, SDP, STUN, SRTP vectors |
| `flute` | FLUTE/ALC/NORM objects |
| `ule` | ULE SNDUs |
| `t2mi` | T2-MI streams |
| `bbframe` | DVB-S2/T2 BBFrame streams |
| `http` | Recorded HTTP exchanges and live-channel captures |
| `audio` | Elementary audio and loudness material |
| `video` | Elementary video and HDR/Dolby Vision material |
| `captions` | CEA-608/708 caption data |
| `subtitles` | Teletext, DVB, TTML, WebVTT, SRT subtitle material |
| `crypto` | Encryption material: SAMPLE-AES, CSA, keys |
| `tls` | Certificates and keys |
| `logs` | Recorded traces and measurement logs |
| `suites` | Third-party test suites kept under their own names |
| `candidates` | Material not yet wired to a test or awaiting a licence decision |
| `golden` | Expected outputs, as `golden/<topic>/<file>` |

## Two tiers

- `fixtures/` is public: captures and generated vectors that may be redistributed.
- `private/fixtures/` is the private submodule (`private/`): captures that are not
  redistributable, have an unclear licence, or were captured live. Tests skip loudly
  when `private/` is not checked out; with `REQUIRE_PRIVATE_FIXTURES=1` a missing
  private fixture is a hard failure.

## Loading

Tests read fixtures only through `test_fixtures` (`read`, `read_to_string`, `path`,
`require`, `private`, `tsduck`, `corpus`) with the path relative to the tier, for
example `test_fixtures::read("ts/h264-aac.ts")`. They never hand-build paths and
never `include_bytes!`/`include_str!` a fixture.

## PROVENANCE.md

Every fixture is covered by the nearest enclosing `PROVENANCE.md` (its set, its
topic, or the tier root), which states where the files came from, how they were made
and under what licence. README, GENERATE, MANIFEST and ORACLES notes are folded into it. `LICENSE`,
`ATTRIBUTION.txt` and `SHA256SUMS` files sit beside the data they cover.

## Goldens

Expected outputs live at `fixtures/golden/<topic>/<file>`, filed by what the output
is (an MPD under `golden/dash/`, an SDP under `golden/rtp/`), never by crate. Tests and
any bless/update mode read and write them through
`test_fixtures::path("golden/<topic>/<file>")`, never a hand-built path. A golden is
changed only by a deliberate, reviewed diff, never re-blessed to make a test pass. No
tracked file may match `*/tests/golden/*`.
