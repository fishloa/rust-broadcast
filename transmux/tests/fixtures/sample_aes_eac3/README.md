# E-AC-3 HLS Sample-AES oracle (audit r05-W2)

## Answer (pinned by `proof.log`, `bento4/proof.log`)

For **independent syncframes** (each syncframe its own audio frame / access unit - every stream ffmpeg can
produce) the rule is:

* first **16 bytes** of each syncframe clear; then every **whole 16-byte block** AES-128-CBC encrypted; the
  trailing partial block (<16 bytes) clear (fixture: 360-byte syncframes = 16 + 21 blocks + **8 clear**);
* the **IV is RESET to the playlist IV at the start of every syncframe** - *not* carried.

| Evidence | Result |
|---|---|
| ffmpeg decrypt of `enc_reset.ts` (13 syncframes in ONE PES, so a carried chain would be visible) | decoded audio frames identical to clear: **9/9** compared (12/13 overall; last frame = ffmpeg EOF artifact) |
| ffmpeg decrypt of `enc_carried.ts` | **1/9** (only the first syncframe, where chain == IV) |
| Bento4 `mp4hls --encryption-mode SAMPLE-AES` ES (both `--audio-format packed` and `ts`) vs python reference `--reset-per-syncframe` | **byte-identical**, 4680 bytes / 13 syncframes; != carried variant |
| ffmpeg decrypt of Bento4's TS | **9/9** identical to clear |

So: python reference, Bento4 encryptor and ffmpeg decryptor agree.

## What is NOT covered by any independent oracle (important)

The crate's documented rule (`transmux/docs/drm/hls-sample-aes.md` §6, Apple §2.3.1.3) is "IV not reset at
syncframe boundaries **within an audio frame**; reset at the beginning of each audio frame". ffmpeg and Bento4
treat every independent syncframe (strmtyp 0) as a separate audio frame, hence reset - consistent with Apple's text
when 1 syncframe == 1 audio frame. **Carrying the IV across an independent syncframe + its dependent syncframes
(strmtyp 1, e.g. 7.1) inside one access unit has no independent oracle**: ffmpeg's encoder cannot emit dependent
substreams and neither tool's behaviour on them was observable. That part is spec-text-derived only.
Practical consequence: a TS PES payload holding N *independent* syncframes must be split per access unit and the IV
reset for each; feeding two concatenated independent syncframes to a single "audio frame" call with a carried
chain (as the now-removed `sample_aes/eac3_51_1024k.ec3` unit test used to assert) is **not** what ffmpeg/Bento4 do.

Other limits: AC-3 (non-E) was not built - ffmpeg's packed-audio SAMPLE-AES path does not decode encrypted
AC-3/E-AC-3 (it probes the ciphertext), only its TS path decrypts E-AC-3; ffmpeg cannot mux encrypted E-AC-3 (muxer
must decode), so `enc_*.ts` are built from the clear TS by swapping the PES payload (`ts_swap_es.py`, same
length). ffmpeg writes PMT `stream_type 0x87` + `EAC3` registration; Bento4 writes `stream_type 0xC2` with an
`apad`/`zec3` setup descriptor on PID 0x101 - both are accepted by ffmpeg.

## Files

`clear.ec3` (13 x 360-byte syncframes, stereo 48 kHz 90 kbit/s sine) - `sample_aes_eac3_ref.py` (reference, `--reset-per-syncframe`,
`--leader N`) - `enc_reset.ec3`, `enc_carried.ec3` (+ `.ts` / `.m3u8`, playlist IV `101112..1f`, `key.bin` =
`000102..0f`) - `*.framemd5` - `nokey.framemd5` (control: same TS without key tag does not decode) -
`bento4/{ts,packed}/` (`segment-0.ts`/`.ec3`, `bento4_encrypted_es.ec3` = ES as Bento4 wrote it, `stream.m3u8`,
`bento4_decoded.framemd5`, `key.bin`) - `proof.log`, `bento4/proof.log`.

`bento4_encrypted_es.ec3` **must equal our decryptor's input and `enc_reset.ec3`** (they are identical); decrypting
it (per-syncframe IV reset) must give `clear.ec3`.

## Regenerate

```
./gen.sh && ./gen_bento4.sh
```
ffmpeg 8.1.2, Python 3.14.6, pycryptodome 3.23.0, Bento4 mp4hls 1.2.0 r641 (`fps` IV mode, key||IV deterministic).
Run twice: byte-identical.
