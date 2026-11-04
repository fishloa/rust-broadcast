# RTP/SDP oracle fixtures

Produced locally with ffmpeg 8.1 (no packet is sent; `-f rtp` only writes the
SDP when a `sdp_file` is given):

```
ffmpeg -y -i ../../../../fixtures/ts/h264/high.ts     -c copy -f rtp \
       -sdp_file high-ffmpeg.sdp      rtp://127.0.0.1:5999
ffmpeg -y -i ../../../../fixtures/ts/h264/baseline.ts -c copy -f rtp \
       -sdp_file baseline-ffmpeg.sdp  rtp://127.0.0.1:5998
```

The source `.ts` files are the workspace's own H.264 fixtures
(`fixtures/ts/h264/high.ts` is High profile, `profile_idc` 0x64;
`baseline.ts` is Baseline, `profile_idc` 0x42). The SDPs are the negotiated
session description a real RTSP client would receive, so the `sprop-parameter-
sets` / `config` values inside them are real encoder output rather than
hand-written bytes.

`high-ffmpeg.avcc.txt` is the hex of the `avcC` box ffmpeg writes when remuxing
the same stream to MP4 (`-c copy out.mp4`), read from the resulting file with

```
python3 -c "d=open('high.mp4','rb').read(); i=d.find(b'avcC')-4; \
  import struct; n=struct.unpack('>I',d[i:i+4])[0]; print(d[i:i+n].hex())"
```

It is the independent ground truth for the High profile chroma/bit-depth
trailer that earlier versions of `avc_config_from_sps_pps` omitted.
