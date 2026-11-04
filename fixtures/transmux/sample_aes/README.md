# Fixture provenance — `tests/fixtures/sample_aes/`

## `eac3_51_1024k.ec3`

Four real E-AC-3 5.1 syncframes (4096 bytes each) from ffmpeg 8.1.2 at
1024 kbit/s, used to test that the E-AC-3 Sample-AES protected block is a
single **syncframe** with the CBC IV carried across the syncframes of one
audio frame (Apple, *MPEG-2 Stream Encryption Format for HTTP Live
Streaming* §2.3.1.3; audit r05-W2).

ffmpeg's own E-AC-3 encoder emits one syncframe per frame, so the test
concatenates the first two syncframes to build a legal two-syncframe access
unit. Syncframe boundaries come only from the wire `frmsiz` field
(ETSI TS 102 366 §E.1.3.1.3), so a concatenation of complete syncframes is
exactly what a demuxer hands the Sample-AES API.

Generated with:

```bash
ffmpeg -y -f lavfi -i "sine=frequency=440:duration=2:sample_rate=48000" \
  -af "pan=5.1|c0=c0|c1=c0|c2=c0|c3=c0|c4=c0|c5=c0" \
  -c:a eac3 -b:a 1024k e51.ec3
python3 - <<'PY'
d = open('e51.ec3', 'rb').read()
l = ((((d[2] & 0x07) << 8) | d[3]) + 1) * 2   # frmsiz -> bytes
open('eac3_51_1024k.ec3', 'wb').write(d[:l * 4])
PY
```

This fixture is now **superseded** by `../sample_aes_eac3/` (the independent oracle).
It is kept only as the original provenance record; no test consumes it.

## Correction (independent oracle, see `../ORACLES.md` B1-B3)

The "IV carried across the syncframes" claim above was checked against ffmpeg 8.1.2 (decryptor) and Bento4 `mp4hls`
(encryptor). For **independent** syncframes (strmtyp 0, each its own audio frame) both **reset the IV at every
syncframe** (`../sample_aes_eac3/`). Carrying across an independent syncframe + its *dependent* syncframes has no
independent oracle. This fixture concatenates two independent syncframes into one "frame"; the independent tools
would treat them as two audio frames.
