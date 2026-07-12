# transmux-py

Python bindings for [`transmux`](https://crates.io/crates/transmux) — demux a media
container into transmux's neutral `Media`/`Track`/`Sample` intermediate
representation and get it back as plain Python `dict`s: clean, PTS-tagged,
opaque coded sample bytes ready to hand to something like PyAV, a
WebCodecs-equivalent decoder, or ONNX preprocessing — without needing to
understand transmux's own Rust IR types (docs/IDEAS.md item #7, issue #668).

Built with [PyO3](https://pyo3.rs) + [maturin](https://www.maturin.rs); ships as an
abi3 wheel (one wheel for CPython ≥ 3.9). Not part of the Rust workspace — it
consumes the published `transmux` crate by version, the same convention as the
sibling binding [`dvb-si-py`](../python/).

## Install

```console
$ pip install transmux-py
```

## Usage

```python
import transmux_py as tm

with open("capture.ts", "rb") as f:
    data = f.read()

media = tm.demux_ts(data)
print(media["movie_timescale"])

for track in media["tracks"]:
    print(track["codec"], track["codec_string"], track["timescale"])
    for sample in track["samples"]:
        # sample["data"] is the opaque coded access unit (bytes) —
        # length-prefixed NAL units for AVC/HEVC, raw frames for AAC/etc.
        # hand it straight to a decoder keyed off track["codec_string"].
        if sample["is_sync"]:
            ...  # a keyframe / random-access point
```

### Returned shape

```text
{
  "movie_timescale": int,
  "tracks": [
    {
      "track_id": int,
      "timescale": int,               # ticks/sec for this track's duration/composition_offset fields
      "start_decode_time": int,       # absolute DTS anchor, in `timescale` ticks
      "source_pid": int | None,       # TS elementary-stream PID, when known
      "codec": str,                   # short family label, e.g. "avc", "aac", "data"
      "codec_string": str | None,     # RFC 6381 codec string, when cheaply derivable
      "width": int | None,
      "height": int | None,
      "channel_count": int | None,
      "sample_rate": int | None,      # Hz
      "samples": [
        {
          "data": bytes,              # opaque coded access unit
          "duration": int,            # in `timescale` ticks
          "is_sync": bool,            # keyframe / random-access point
          "composition_offset": int,  # pts - dts, in `timescale` ticks
        },
        ...
      ],
    },
    ...
  ],
}
```

Note on the read-only design: unlike `dvb-si-py`, which converts Rust →
`serde_json::Value` → Python because `dvb-si`'s table types are fully
`serde::Serialize`, transmux's pipeline IR (`Media`/`Track`/`TrackSpec`/
`CodecConfig`/`Sample`) carries **no** serde derive at all — the crate's
`serde` feature only reaches its lower-level ISOBMFF box types, not this IR.
So this binding hand-converts the real Rust structs into `PyDict`s field by
field instead of reusing that json round-trip.

Only MPEG-2 TS input (`demux_ts`) is exposed in this first version — the one
demux entry point that is both one-shot/batch-callable and does not require
understanding transmux's incremental/streaming APIs. `transmux::media::Fmp4Demux`
(fragmented ISOBMFF/CMAF input) exists in the Rust crate too and is a natural
follow-up if fMP4-sourced ML pipelines need it.

## Build from source

```console
$ pip install maturin
$ maturin develop          # build + install into the active virtualenv
$ maturin build --release  # build a wheel
```

Run the test suite (uses a real committed fixture, `fixtures/ts/h264_aac.ts`):

```console
$ maturin develop
$ pytest -q
```

## License

MIT OR Apache-2.0.
