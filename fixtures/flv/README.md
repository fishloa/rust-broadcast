# `fixtures/flv/` — FLV fixtures

## Real captures

| File | Provenance |
|------|------------|
| `av.flv` | Real H.264 + AAC capture; `av.packets.csv` is the ffprobe packet-timing oracle. |
| `aac-7_1.flv` | Real AAC 7.1 capture; `aac-7_1.oracle.csv` is its ffprobe oracle. |

## Hand-built

### `oversize-dims.flv`

Synthesised, not captured. A minimal FLV whose AVC sequence header carries an
SPS with `pic_width_in_mbs_minus1 = 4095` (width 65 536, height 48), followed by
one AVC NALU tag. Used by `transmux/tests/flv.rs` to prove a coded size that
does not fit the IR's `u16` is rejected (the `#997` class) rather than truncated.

Its NALU tag payload is a **well-formed** 4-byte-length-prefixed access unit
(`00 00 00 02 09 10`, matching the `avcC`'s `lengthSizeMinusOne = 3`), so the
demuxer's framing validation accepts the sample and the dimension check is what
actually runs.

Generator (the bytes are also reproduced inline in the test):

```sh
python3 - <<'EOF'
import pathlib
# Exact bytes. Structure: FLV header + PreviousTagSize0, an AVC
# sequence-header tag (avcC `0142001fffe1 000a <oversize SPS> 00`), then one
# AVC NALU tag with a well-formed 4-byte-length NAL (`00000002 0910`, matching
# the avcC's lengthSizeMinusOne = 3).
data = bytes.fromhex(
    "464c56010100000009000000"
    "000900001800000000000000"
    "17000000000142001fffe100"
    "0a6742001ff4000800388000"
    "000000230900000b00000000"
    "000000170100000000000002"
    "091000000016"
)
assert len(data) == 78, len(data)
pathlib.Path("fixtures/flv/oversize-dims.flv").write_bytes(data)
EOF
```
