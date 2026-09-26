# transmux 0.24.2 — 2026-09-25

Security patch. **Upgrade if transmux parses or encrypts input you do not control** — MP4/fMP4
files, TS, RTMP streams, SDP, or third-party init segments. No API changes; drop-in for 0.24.x.

These fixes come from a full-read audit of the workspace (2026-09-24). Each one ships with a
regression test that fails against 0.24.1.

## Security

| Advisory | Area | Before this release |
|---|---|---|
| GHSA-q6vr-4pp5-mcfv | KLV (`KlvItem`, `UasLocalSet`) | a long-form BER length overflowed the offset arithmetic and panicked |
| GHSA-3554-x4jf-7frw | `esds` (`EsdsBox`) | an ES_Descriptor size larger than the box, or a size-0 box, panicked the fMP4 demuxer |
| GHSA-c722-96gr-hgq2 | `sinf` (`ProtectionSchemeInfoBox`) | an oversized `frma` or a header-only `schm` panicked |
| GHSA-gmmr-7cqm-5p32 | `pssh` v1 | a body with no `DataSize` field panicked |
| GHSA-2mg2-jj9h-c5vr | progressive MP4 demux | one `stts`/`ctts` run could allocate ~17 GB from a ~100-byte file (the 0.24.1 fix bounded the sample total but not each run) |
| GHSA-9m92-jr3c-4cq7 | `CencDecryptor` progressive path | `stsz`/`stco` counts allocated before their length checks |
| GHSA-j76h-pqjr-p54h | `sgpd` | zero-length entries could be pushed up to 2³² times (the 0.24.1 fix bounded only the initial capacity) |
| GHSA-643q-pgwj-8v2h | H.264/H.265 SPS | `read_ue` returned 0 at end of data, so a ~20-byte SPS could loop effectively forever; out-of-range fields overflowed |
| GHSA-6vc8-3c25-4c9w | RTMP chunk reader | a fmt 1/2 header after an incomplete message carried stale bytes into the new one and underflowed its length |
| GHSA-mq46-69j5-7gqj | `CencEncryptor::encrypt` | a sample rejected mid-call left earlier samples encrypted with the IV counter unchanged, so a retry reused IVs |
| GHSA-59ph-4f24-q79x | `KeyMap`/`CencEncryptor`/`CencDecryptor`/`cli::Args`/`cli::CliError::BadKey` | derived `Debug` printed raw content key bytes (`KeyMap`, `CencEncryptor`, `CencDecryptor`) or the raw `<KID>:<key>` CLI argument (`cli::Args`, `BadKey`) — a log line or crash report at debug/trace level could leak key material |
| GHSA-v965-v82c-2f8x | `progressive_demux` `stsc` expansion | a chunk-run entry's `first_chunk` was iterated to as written, up to `u32::MAX`, instead of being clamped to the track's actual chunk count — a malformed file could cost billions of loop iterations per entry |

## Behaviour changes

All of these turn a panic, a runaway allocation or silent corruption into an `Err`. Valid input
is unaffected:

- **`BitReader::read_ue`** now returns an error at end of data, and for more than 32 leading
  zero bits. H.264/H.265 SPS parsing rejects values outside their spec ranges:
  `chroma_format_idc > 3`, `bit_depth_*_minus8` above 6 (H.264) or 8 (H.265),
  `num_ref_frames_in_pic_order_cnt_cycle > 255`, and `num_short_term_ref_pic_sets > 64`.
- **An `esds` box with `size == 0`** now extends to the end of its enclosing data, per
  ISO/IEC 14496-12 §4.2. It previously panicked.
- **Progressive demux** rejects a sample-table run that doesn't fit the chunk layout. On the
  lenient per-track path the track is skipped and the reason recorded, as for other malformed
  tracks.
- **`CencEncryptor::encrypt`** now works out every sample's subsample map before encrypting any
  sample. A rejected call leaves the media byte-identical and the IV counter unchanged, as the
  method's documentation already promised.
- **RTMP:** fmt 0, 1 and 2 chunk headers each start a fresh message (RTMP 1.0 §5.3.1.2).
- **Content keys no longer print via `Debug`.** `KeyMap`, `CencEncryptor` and `CencDecryptor` now
  hand-write `Debug` instead of deriving it: content key bytes are redacted (KIDs, which are not
  secret, still print), and `CencDecryptor` summarizes the protected file by length rather than
  dumping its bytes. `cli::Args` hand-writes `Debug` so a `--key <KID>:<key>` argument prints only
  its KID half, and `cli::CliError::BadKey` now stores an already-redacted form of the offending
  argument (the KID half if it parsed, otherwise just its length), so neither its `Display` nor
  its derived `Debug` can print key hex.

## Testing

- **`tests/hostile_input_bounds.rs`** (new) runs the allocation regressions under a per-thread
  64 MiB allocator cap. Against 0.24.1 they abort as soon as they cross the cap instead of
  exhausting memory.
- **Existing suite** is unchanged apart from the CENC encrypt allocation measurement, which moved
  from 68/3 392/51 to 57/4 040/40 (allocs/bytes/deallocs) with the planning-phase change.

MSRV 1.95.0.
