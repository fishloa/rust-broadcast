# H.264 HLS Sample-AES oracle (audit r05-W1)

Settles, with three independent implementations, **how AES-128-CBC is chained inside one NAL**.

## Answer (pinned by `proof.log`)

| Question | Answer | Evidence |
|---|---|---|
| Chain across the encrypted 16-byte blocks of one NAL, or IV reset per block? | **Chained.** IV is set at the start of *each NAL* only; block *n* uses ciphertext block *n-1* as IV. | ffmpeg decodes `enc_chained.ts` identically to clear: **10/10** frames; `enc_reset.ts` (IV reset per block): **0/10**. Bento4's encryptor output is byte-identical to the chained python reference (40/40 slice NALs). |
| Clear leader | First **32 bytes** of the (unescaped) NAL (header byte + 31) clear. | same decode proofs |
| Pattern | Then 16 encrypted bytes out of every 160 (16 enc + 144 clear), counted from offset 32. | same |
| Which NALs | Types **1 and 5** with **more than 48 bytes**; everything else (incl. <=48-byte slices, SPS/PPS/SEI/AUD) clear. | fixture has 19 slices <= 48 B and 14 > 160 B (48 slices, 10..870 B) |
| Tail rule | A candidate block is encrypted only if **more than 16 bytes remain** in the NAL; a tail of *exactly* 16 bytes stays **clear**, as does any partial (<16) tail. | `tailprobe_*`: slices padded so the last candidate block has exactly 16 / 15 / 17 bytes left. `>16` rule: **10/10**; `>=16` rule: **0/10** (IDR with an exact-16 tail corrupts every later frame). Bento4 also byte-identical to the `>16` rule (40/40) and decodes 10/10. |
| Emulation prevention | The NAL is **unescaped, encrypted, then re-escaped** (0x03 inserted where the ciphertext contains `00 00 0[0-3]`). | `escprobe_*`: block-0 ciphertext forced to `00 00 01 ..`; Bento4's output is **byte-identical on the wire** (incl. the inserted 03) to the python reference; ffmpeg decodes 10/10. |

Three implementations agree: python reference (`sample_aes_ref.py`), Bento4 `mp4hls` encryptor, ffmpeg decryptor.

### Known limits (stated plainly)

* **ffmpeg never decrypts the last two access units of a TS** (`10/12` with 12 frames; `28/30` with 30 in a
  side experiment): an ffmpeg EOF artifact, not a rule difference (their NALs come out byte-for-byte still
  ciphertext). Hence 12 frames are encoded and **only the first 10 are compared**. `proof.log` also prints the
  all-frames line.
* **No independent oracle for EP bytes inside the CLEAR regions** (`epclear_*`: `00 00 01` planted at offset 10
  and `00 00 02` at offset 100 of one IDR slice). Bento4 encrypts the still-escaped bytes and re-escapes
  (double `03`, byte mismatch on that NAL, ffmpeg then decodes 0/10); ffmpeg's decryptor emits NALs
  *unescaped* so a framemd5 comparison is not meaningful either (python reference also 0/10). The python
  reference follows Apple's text (unescape before encrypt). Treat that one corner as spec-derived only.
* ffmpeg also emits the decrypted NAL unescaped, so ES-level comparisons (`compare_es.py`) are made modulo EP;
  `--raw` compares wire bytes (used against Bento4, where they are equal).
* The 0xAA padding used by `tailprobe` is not conformant bitstream padding; ffmpeg's CABAC slice decoder ignores
  bytes after end_of_slice (clear tailprobe decodes identically to `clear.h264`, 10/10). Zero padding
  (cabac_zero_words) was tried first and cannot be used: ffmpeg's decryptor then emits `00 00 00` runs.

## Files

| file | what |
|---|---|
| `clear.h264` | Annex-B, 12 frames 320x180 10 fps, IDR at 0 and 10, 4 slices/frame, 4-byte start codes (renormalised) |
| `sample_aes_ref.py` | ~80-line pycryptodome reference; `--reset-per-block` (WRONG rule), `--tail gt|ge` |
| `enc_chained.h264/.ts/.m3u8`, `enc_reset.*` | encrypted ES, TS (`ffmpeg -c copy`), playlist `#EXT-X-KEY:METHOD=SAMPLE-AES,URI="key.bin",IV=0x101112..1f` |
| `key.bin` | `000102030405060708090a0b0c0d0e0f`; IV `101112131415161718191a1b1c1d1e1f` |
| `tailprobe_*`, `escprobe_*`, `epclear_*` | the three corner-case probes (see above) |
| `*.framemd5` | ffmpeg `-f framemd5` of the clear decode and of each encrypted decode |
| `bento4_sample_aes/{clear,tailprobe_clear,escprobe_clear,epclear_clear}/` | Bento4 `mp4hls` encryptor output: `segment-0.ts`, `stream.m3u8` (IV patched in), `bento4_encrypted.h264` (ES exactly as Bento4 wrote it, extracted with `ffmpeg -c copy -f h264`), `bento4_decoded.framemd5`; shared `key.bin` |
| `proof.log`, `bento4_sample_aes/proof.log` | the recorded evidence |

**What our decryptor must do with Bento4's file:** `bento4_encrypted.h264` (clear/`enc_chained` family) must
decrypt (key `key.bin`, IV above, per-NAL chained) to `clear.h264`'s slice NALs, equal **modulo emulation
prevention** (compare unescaped; the wire bytes of the encrypted ES equal `enc_chained.h264`/`tailprobe_enc_gt.h264`/
`escprobe_enc.h264` exactly, SPS/PPS/SEI/AUD NALs are untouched and identical).

## Regenerate

```
./gen.sh          # ES, TS, playlists, framemd5, proof.log   (ffmpeg, python3 + pycryptodome)
./gen_bento4.sh   # bento4_sample_aes/                        (+ Bento4 mp4hls)
```
Tool versions used: ffmpeg 8.1.2 (libx264, Apple clang, `ffmpeg -version`), Python 3.14.6, pycryptodome 3.23.0,
Bento4 mp4hls 1.2.0 r641 (Bento4 1.6.0.0). Bento4 runs in `--encryption-iv-mode fps` (key||IV given as 32 bytes,
deterministic; the playlist then lacks `IV=` so `gen_bento4.sh` patches it in). Both scripts were run twice:
all outputs byte-identical.
