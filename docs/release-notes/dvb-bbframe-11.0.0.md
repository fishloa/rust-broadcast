# dvb-bbframe 11.0.0

_Released 2026-10-05._

**Major (breaking), one signature change.** Normal-Mode (NM) user-packet extraction now honours `UPL`, ISSYI and NPD framing instead of cutting every 188 bytes, and NM CRC-8 mismatches are now detected. The break is `packet::NmTsIter::new`, which takes an explicit stride and is fallible. If you construct `NmTsIter` directly you must change the call; if you use `BbframePump` or `CarryOverExtractor` you get the fixed behaviour with no code change. Lockstep sibling of [dvb-t2mi 11.0.0](dvb-t2mi-11.0.0.md), which builds on this crate.

## Dependency changes

```toml
-broadcast-common = { version = "9.3", default-features = false }
+broadcast-common = { version = "9.4", default-features = false }
```

## Breaking change: `NmTsIter::new`

```rust
// before
let it = NmTsIter::new(data);
// after: stride is the per-user-packet stride in bytes; construction can fail
let it = NmTsIter::new(data, stride)?;
```

A non-zero stride below 188 returns the new `Error::InvalidStride`; stride 0 stays the documented empty iterator. Previously a short stride panicked inside `next()` on short input, and the iterator could overflow on `pos + stride`; both are gone (#1033). To derive the stride for a frame use the new `packet::nm_stride_bytes(&Bbheader) -> Option<usize>`.

## Behaviour changes and fixes

- NM user-packet framing (#1033). `NmTsIter`, `up_iter`, `CarryOverExtractor::feed_nm` and `feed_nm_into` always cut every 188 bytes and ignored `UPL`, so a real off-air capture with ISSYI=1 (190-byte stride) was misframed after the first user packet, and NPD/DNP framing was never accounted for. The stride is now derived from `UPL`, ISSYI and NPD per EN 302 755 §5.1.8, and the per-UP CRC-8 chain (§5.1.6) is checked. New `CarryOverStats` fields: `nm_upl_invalid` and `crc8_mismatches`.
- A CRC-8 mismatch now sets the Transport Error Indicator (ISO/IEC 13818-1 §2.4.3.2) on the affected output packet to alert downstream receivers (EN 302 755 §5.1.6). Previously the chain was checked but only counted, so corrupted packets went out unflagged (#1094). Consumers that treated TEI-clear as "good" will now see TEI set on corrupted NM packets.
- `Bbheader::serialize_into` now validates `dfl <= DFL_MAX_BITS` and rejects mode inconsistencies (NM cannot carry `issy_in_header`; HEM cannot carry non-zero `upl`/`sync`), so it no longer emits a header its own parser refuses (#1094).
- `BbframePump` allocates its 256-element extractor array on the heap (`Box`) instead of the stack, avoiding a 59 KB stack frame on small-stack targets (#1094).

---

Published from tag `dvb-bbframe-v11.0.0`.
