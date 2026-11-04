# Independent oracle fixtures (audit r05 W1 / W2 / W7)

"Independent" = a tool that shares no code with this crate (ffmpeg 8.1.2, Bento4 1.6.0.0, pycryptodome 3.23.0).
Every dir has `gen.sh` (deterministic: run twice, byte-identical), tool versions in its README, and a recorded `proof.log`.

| # | Fixture (dir) | Claim it proves | Generator | Independent tool(s) | Result |
|---|---|---|---|---|---|
| A1 | `sample_aes_h264/enc_chained.*` vs `enc_reset.*` | H.264 Sample-AES CBC **chains across the encrypted blocks of a NAL**; IV reset only per NAL | `gen.sh`, `sample_aes_ref.py` | ffmpeg decrypt | chained **10/10** frames = clear; per-block reset **0/10** |
| A2 | `sample_aes_h264/bento4_sample_aes/clear/` | an independent ENCRYPTOR writes the chained form | `gen_bento4.sh` | Bento4 `mp4hls` SAMPLE-AES (encrypt), ffmpeg (decrypt) | ES byte-identical to python reference (40/40 slice NALs); ffmpeg 10/10 |
| A3 | `sample_aes_h264/tailprobe_*` | tail rule: block encrypted only if **>16 bytes remain**; exactly-16 tail stays clear | `prep_clear.py` | ffmpeg, Bento4 | `>16`: 10/10 (Bento4 40/40 identical); `>=16`: 0/10 |
| A4 | `sample_aes_h264/escprobe_*` | encrypted output gets 0x03 emulation prevention where ciphertext has `00 00 0[0-3]`; NAL is unescaped before encrypt | `prep_clear.py` | Bento4 (wire-byte identical), ffmpeg | identical incl. the inserted 03; ffmpeg 10/10 |
| A5 | `sample_aes_h264/epclear_*` | EP bytes inside the **clear** regions of a slice | - | Bento4, ffmpeg | **NO oracle**: Bento4 double-escapes, ffmpeg emits unescaped NALs; python reference (Apple text) only |
| B1 | `sample_aes_eac3/enc_reset.*` vs `enc_carried.*` | E-AC-3 (independent syncframes): 16-byte clear leader, whole 16-byte blocks encrypted, partial tail clear, **IV reset every syncframe** | `gen.sh`, `sample_aes_eac3_ref.py` | ffmpeg decrypt (TS) | reset **9/9** audio frames = clear; carried **1/9** |
| B2 | `sample_aes_eac3/bento4/{ts,packed}/` | independent ENCRYPTOR agrees | `gen_bento4.sh` | Bento4 `mp4hls` (packed + ts) | ES byte-identical to reset reference (13 syncframes); ffmpeg 9/9 on Bento4 TS |
| B3 | (none) | IV carried across an independent + **dependent** syncframes of one access unit | - | none obtainable | **NO oracle**; spec-text (Apple §2.3.1.3) only. Note ffmpeg/Bento4 reset per *independent* syncframe |
| B4 | (none) | AC-3 (non-E) Sample-AES | - | ffmpeg packed path fails on encrypted ac3/ec3 | not built |
| C1 | `cenc_frag_layouts/enc_default_*` | CENC decrypt with tfhd `base_data_offset` (single, explicit-2-trun, implicit-2-trun) | `gen.sh`, `relayout.py` | Bento4 mp4encrypt (encrypt), mp4decrypt, ffmpeg | mp4decrypt CORRECT 3/3; ffmpeg 115/115 (single, explicit), fails implicit (its limitation) |
| C2 | `cenc_frag_layouts/enc_base_moof_*` | same with default-base-is-moof | same | same | mp4decrypt CORRECT 3/3; ffmpeg same as C1 |
| C3 | `cenc_frag_layouts/enc_omit_*` | omit-tfhd-offset: traf 2 base = end of traf 1 data (single / explicit / implicit multi-trun) | same | mp4decrypt, ffmpeg | **NO independent decryptor is correct** (mp4decrypt WRONG audio; ffmpeg wrong audio). Oracle = plaintext == `clear.mp4` (`clear_samples.txt`); layout offsets identical to ffmpeg's own muxer output; ffmpeg reads clear omit 115/115 |
| C4 | `cenc_frag_layouts/clear_ffmpeg_{default,base_moof,omit}.mp4` | the three addressing forms as a real muxer writes them | `gen.sh` | ffmpeg (writer + reader), mp4dump | all read 115/115; dumps in `layouts.txt` |
| C5 | `cenc_frag_layouts/*multitrun*`, `enc_*_{explicit,implicit}` | multiple truns per traf, explicit and implicit data offset | `relayout.py` (**hand-split; no tool emits it**) | mp4dump validated; mp4decrypt (default/base_moof only); spec-derived `frag_samples.py` | see C1-C3; implicit unreadable by ffmpeg even in clear |
| C6 | (rejected) ffmpeg `-encryption_scheme` output | - | - | ffmpeg muxer | audio of fragments >= 2 does not decrypt in ffmpeg **or** mp4decrypt; not used as an oracle |

**Claims with NO independent oracle:** A5 (EP inside clear regions), B3 (IV carry across dependent syncframes), B4 (AC-3),
C3 (any decryptor of omit_tfhd_offset+CENC), implicit-trun with ffmpeg, and `saio` semantics for omit (moof-relative,
matching ffmpeg's muxer only).
Corrections to existing repo claims: `sample_aes/README.md` / `sample_aes::tests::eac3_multi_syncframe_carries_iv`
carry the IV across two **independent** syncframes; ffmpeg and Bento4 reset it there (see B1/B2/B3).
