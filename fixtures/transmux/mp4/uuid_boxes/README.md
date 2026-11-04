# Fixture provenance — `tests/fixtures/mp4/uuid_boxes/`

Real, tool-generated media with real `uuid` child boxes, for the audit item-1
fix (`uuid` children were re-serialised corrupted: `parse_box`'s body excludes
the 16-byte `usertype` that `BoxHeader::header_size()` counts).

None of the muxers installed here (ffmpeg 8.1.2, GPAC/MP4Box 26.07, Bento4
1.6.0.0) emits a `uuid` box from its CLI, so each file is a real muxer's output
with `uuid` children spliced in by `gen.py`. The spliced *payloads* are real,
spec-defined structures: a PlayReady/ISMV `pssh` whose body is the Widevine
pssh from `fixtures/cpix/widevine-pssh-from-complex.bin`, and a Smooth `tfxd`
([MS-SSTR] §2.2.4.4, the extended type this crate's `smooth` module
implements).

## Generate (deterministic — run twice, byte-identical)

```bash
ffmpeg -y -f lavfi -i "testsrc=size=64x64:rate=5:duration=1" \
  -c:v libx264 -preset ultrafast -pix_fmt yuv420p -g 5 \
  -movflags cmaf+frag_keyframe+empty_moov+default_base_moof \
  -f mp4 base_clear.mp4

python3 gen.py base_clear.mp4 moov_uuid.mp4 segment_uuid.mp4
```

`gen.py` grows every enclosing box and adds each inserted byte count to every
`trun.data_offset` that addresses data past the insertion point, so a
`default-base-is-moof` fragment keeps resolving.

## Files

| File | What it carries |
|---|---|
| `base_clear.mp4` | the unmodified ffmpeg CMAF output (the generator's input) |
| `moov_uuid.mp4` | `base_clear.mp4` + a `uuid` child as `moov`'s first child |
| `segment_uuid.mp4` | `moov_uuid.mp4` + a `uuid` child at `moof` level (after `traf`) and one at `traf` level (after `trun`) |

## Independent verification (Bento4 `mp4dump`, never this crate's parser)

```
$ mp4dump moov_uuid.mp4 | grep D08A4F
  [D08A4F1810F3-4A82-B6C8-32D8-ABA183D3] size=24+64

$ mp4dump segment_uuid.mp4 | grep -E "D08A4F|6D1D9B|\[moof\]|\[traf\]|\[trun\]|mdat"
  [D08A4F1810F3-4A82-B6C8-32D8-ABA183D3] size=24+64
[moof] size=8+208
  [traf] size=8+140
    [trun] size=12+32, version=1, flags=205
    [6D1D9B0542D5-44E6-80E2-141D-AFF757B2] size=24+20
  [6D1D9B0542D5-44E6-80E2-141D-AFF757B2] size=24+20
[mdat] size=8+5823
```

`size=24+64` / `size=24+20` is the oracle for the extended type and the box
size: 8-byte header + 16-byte `usertype` + payload. A parser that kept only
`BoxRef::body` (usertype dropped) re-serialises `+56` / `+12` instead, and
`mp4dump` then reads the usertype as payload bytes.
