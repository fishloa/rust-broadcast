# CENC fragmented-MP4 addressing layouts (audit r05-W7, ISO/IEC 14496-12 §8.8.7 / §8.8.8)

Source: 320x180 10 fps H.264 (GOP 5) + 48 kHz stereo AAC, 2 s => 4 fragments, **two trafs per moof**
(track 1 video = 20 samples, track 2 audio = 95 samples). Key `00112233445566778899aabbccddeeff`,
KID `0123456789abcdef0123456789abcdef`, scheme `cenc` (AES-CTR, video with NAL-aware subsamples), IV size 16
(Bento4). `clear.mp4` = the plaintext oracle; `clear_samples.txt` lists per-sample sha256 of it
(**our decrypt(enc_*) must reproduce these samples exactly, whatever the layout**).

## Layouts produced (first-moof excerpts in `layouts.txt`, from `mp4dump --verbosity 3`)

| files | tfhd flags | addressing | how made |
|---|---|---|---|
| `clear_ffmpeg_default.mp4` (clear) / `enc_default_none.mp4` | `0x39` (base_data_offset present) | data_offset relative to the abs. moof offset; both trafs carry `base data offset = <moof pos>` | ffmpeg default `-movflags frag_keyframe+empty_moov` / re-addressed |
| `clear_ffmpeg_base_moof.mp4` = `clear.mp4` / `enc_base_moof_none.mp4` | `0x20038` (default-base-is-moof) | relative to moof start | ffmpeg `+default_base_moof` |
| `clear_ffmpeg_omit.mp4` / `enc_omit_none.mp4` | `0x38` (neither) | traf 1 base = moof start (`data offset = 959`), **traf 2 base = end of traf 1's data (`data offset = 0`)** | ffmpeg `+omit_tfhd_offset` |
| `enc_{default,base_moof,omit}_explicit.mp4` | as above | **two truns per traf**; 2nd trun has its own data_offset | **hand-split** (`relayout.py --split explicit`) |
| `enc_{default,base_moof,omit}_implicit.mp4`, `clear_multitrun_{base_moof,omit}_implicit.mp4` | as above | **two truns per traf**; 2nd trun has NO data_offset (continues after the 1st trun's data) | **hand-split** (`--split implicit`) |

Multiple truns per traf: **no tool we have emits them** (ffmpeg `frag_custom`/`-frag_duration`, MP4Box `-frag`,
mp4fragment all write one trun per traf), so they are built by `relayout.py`, a small box rewriter that splits every
trun with >=2 samples into halves and re-points data_offsets and `saio`; validated with `mp4dump --verbosity 3`
(layouts.txt), the spec-derived extractor `frag_samples.py` and mp4decrypt. **They are hand-made**, not tool output.

How the encrypted files are made (and why): `mp4encrypt --method MPEG-CENC` (Bento4, independent encryptor)
encrypts `clear.mp4`, but always re-writes fragments as default-base-is-moof. ffmpeg's own CENC muxer
(`-encryption_scheme cenc-aes-ctr`) was tried first and rejected: with video+audio muxed, the **audio of fragments
>= 2 does not decrypt with either ffmpeg or mp4decrypt** (audio-only is fine). So the correctly encrypted
base-moof file is **re-addressed** by `relayout.py` into every form; not one sample byte changes (proved: the
spec-extractor's ciphertext hash is identical across all nine `enc_*` files) and `saio` is re-pointed at the moved `senc`.

## Proof (`proof.log`, run `./proof.sh`)

| file | ciphertext == enc_base_moof_none | mp4decrypt (independent) plaintext == clear | ffmpeg `-decryption_key` frames == clear |
|---|---|---|---|
| enc_default_none / explicit | yes | CORRECT / CORRECT | 115/115 / 115/115 |
| enc_default_implicit | yes | CORRECT | 34/115 (ffmpeg limitation) |
| enc_base_moof_none / explicit | yes | CORRECT / CORRECT | 115/115 / 115/115 |
| enc_base_moof_implicit | yes | CORRECT | 37/115 (ffmpeg limitation) |
| enc_omit_none / explicit / implicit | yes | **WRONG** (audio) | 20/115, 20/115, 8/115 (audio wrong) |

Also: ffmpeg reads the clear forms `clear_ffmpeg_{default,base_moof,omit}` at 115/115 (so its writer and reader agree
with `frag_samples.py` on all three base-offset rules, incl. omit), but even ffmpeg **cannot read clear
implicit-trun files** (`clear_multitrun_*_implicit`: 41/115, 8/115) - a genuine ffmpeg limitation of §8.8.8
("data follows the previous run"), not a fixture error: the spec-derived extractor recovers the exact plaintext.

## What could NOT be independently proven

* **omit_tfhd_offset (+ encryption)**: Bento4 (`mp4decrypt`) mishandles it - it reads the later traf's samples
  from the wrong base, giving wrong audio; ffmpeg decrypts the *video* but not the audio of these files (it does read the
  same layout fine when clear). No independent tool decrypts `enc_omit_*`. The oracle there is the **plaintext
  comparison against `clear.mp4`** plus: (a) the addressing (`data offset = 959` / `0`) is bit-for-bit what
  ffmpeg's own muxer writes for the clear omit form, (b) ffmpeg reads the clear omit form 115/115, (c)
  `frag_samples.py` (spec-derived, this repo) agrees. `saio` in omit files is moof-relative (what ffmpeg's muxer also writes).
* **implicit second-trun (data_offset absent)**: ffmpeg cannot read it; only mp4decrypt (default/base_moof forms) and the
  spec-derived `frag_samples.py` confirm it. Omit + implicit: only the spec-derived extractor.
* Bento4's own `mfra` in its output is stale (moof offsets from the input); `relayout.py` drops `mfra`.

## Files / regenerate

`gen.sh` (ffmpeg 8.1.2 with `-fflags +bitexact`, Bento4 mp4encrypt 1.7 / Bento4 1.6.0.0, python 3.14.6; run twice: identical),
`relayout.py`, `frag_samples.py` (spec-derived per-sample extractor; `--dump` lists offsets), `proof.sh`, `proof.log`,
`layouts.txt`, `clear.mp4`, `clear_samples.txt`, `clear_ffmpeg_*.mp4`, `clear_multitrun_*_implicit.mp4`, `enc_*.mp4` (9).
