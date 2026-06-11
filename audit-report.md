# 🩺 dvb-rs Audit Report

## 🔴 Critical — Real Bugs

### 1. SAT `BitReader`/`BitWriter` silently corrupts on OOB
**`dvb-si/src/tables/sat.rs:53` & `:91`** — `read_u` guard `if byte_idx < self.data.len()` silently reads 0 when over-running the buffer. `write_u` silently skips writes beyond the buffer. `skip` and `write_zero` have zero bounds checks. A wrong `section_length` on a SAT produces silent corruption instead of an error.

### 2. PAT silently drops partial trailing entries
**`dvb-si/src/tables/pat.rs:89`** — When trailing bytes are < 4, the loop `break`s instead of erroring. `pmt.rs` and every other table that has fixed-size entries returns `Err(InvalidSectionLength)` for this case.

### 3. CAT `ca_descriptors()` silently truncates corrupt descriptors
**`dvb-si/src/tables/cat.rs:72-73`** — If a descriptor length overruns `self.descriptors.len()`, the walker `break`s and returns whatever it parsed so far. Callers get partial data with no error.

### 4. dvb-bbframe `CarryOverExtractor` silently swallows errors
**`dvb-bbframe/src/packet.rs:206, 211, 214, 231, 282, 285`** — Six separate `return` sites silently return empty output when: NPD reinsertion is unimplemented, header parse fails, mode mismatch, or `syncd_bytes` straddles a boundary incorrectly. Callers get zero packets with no indication of failure. Should return `Result`.

### 5. `issy_in_header` has no typed accessor
**`dvb-bbframe/src/header.rs:247`** — `pub issy_in_header: Option<[u8; 3]>` is raw bytes. The crate already has `Issy`/`SignallingKind` enums in `issy.rs`, but `Bbheader` exposes raw `[u8; 3]`. Callers must manually call `decode_issy_long`. Should be `pub fn issy(&self) -> Option<Issy>`.

### 6. `decode_issy_short`/`decode_issy_long` return `Option` instead of `Result`
**`dvb-bbframe/src/issy.rs:125-149`** — Both functions return `None` when the form bit doesn't match. The rest of the crate uses `Result<_, Error>`. Passing a short ISSY to the long decoder (or vice versa) gives no diagnostic.

---

## 🟠 High — Missing Typed Accessors / Raw Byte Exposure

### Raw byte arrays with existing-but-unwired decode logic
| File | Field | Fix |
|------|-------|-----|
| `descriptors/private_data_indicator.rs:25` | `private_data_specifier: [u8; 4]` | Should be `u32` or newtype with `Display` |
| `tables/eit.rs:79` | `start_time_raw: [u8; 5]` | Has MJD/BCD decode in `dvb_common::time`; needs `start_time()` accessor |
| `tables/eit.rs:81` | `duration_raw: [u8; 3]` | Needs BCD→duration accessor |
| `tables/mpe.rs:84` | `mac_address: [u8; 6]` | Should be `MacAddress` newtype with `Display` (`XX:XX:XX:XX:XX:XX`) |
| `tables/mpe.rs:117` | `checksum: [u8; 4]` | `Checksum` newtype |
| `descriptors/local_time_offset.rs:33` | `time_of_change_raw: [u8; 5]` | Should mirror TDT's `utc_time()` pattern |

### Raw byte slices with no typed view
| File | Field | Issue |
|------|-------|-------|
| `descriptors/default_authority.rs:20` | `default_authority: &[u8]` | ASCII text; should be `DvbText<'a>` |
| `descriptors/service_identifier.rs:22` | `textual_service_identifier: &[u8]` | ASCII text; should be `DvbText<'a>` |
| `descriptors/telephone.rs:50-58` | 5 fields: `country_prefix`, `international_area_code`, `operator_code`, `national_area_code`, `core_number` | All ISO 8859-1 char runs |
| `tables/cit.rs:46` | `unique_string: &[u8]` | CRID unique string; could be `DvbText<'a>` |
| `tables/cit.rs:78` | `prepend_strings: &[u8]` | Has `resolve()` but raw field still pub |
| `tables/tdt.rs:23`, `tot.rs:33` | `utc_time_raw: [u8; 5]` | Has `chrono`-gated accessor; should have non-feature-gated fallback |

### Fields that should be enums
| File | Field | Issue |
|------|-------|-------|
| `descriptors/extension/uri_linkage.rs:14` | `uri_linkage_type: u8` | 0x00/0x01 magic comparisons; needs `UriLinkageType` enum |
| `descriptors/extension/ac4.rs:20` | `ac4_channel_mode: Option<u8>` | 2-bit field; should be enum per spec's channel_mode table |
| `t2mi/payload/fef_composite.rs:29` | `s2_field: u8` | `FefNullPayload` has `S2Field1` enum + `is_mixed()`; `FefCompositePayload` and `FefIqPayload` don't |
| `t2mi/payload/fef_iq.rs:19` | `s2_field: u8` | Same as above — inconsistency with FefNullPayload |

---

## 🟡 Medium — Protocol / Spec Gaps

### Missing byte-identical round-trip tests (~65 descriptor files + ~15 extension files)
Only `bouquet_name.rs` and `network_name.rs` assert `assert_eq!(&raw, &buf[..])`. All others only check `parse(serialize(x)) == x`, not that `serialize(x)` produces byte-for-byte identical output to the original wire bytes.

### Missing `Yokeable` derive on all 20+ extension descriptor types
**`descriptors/extension/*.rs`** — Every extension struct borrows `&'a [u8]` but none have `#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]`. Main descriptors like `ca.rs:46` and `service.rs:206` all have it.

### DSM-CC SSI=0 sections silently assume L-form CRC-32
**`tables/dsmcc.rs`** — Module doc acknowledges SSI=0 sections carry a 4-byte checksum (not CRC-32), but the parser always treats them as CRC-32. A real SSI=0 DSM-CC stream will fail CRC validation.

### `0x70` hardcoded across 5 short-form table files
**`tables/{tdt,tot,rst,st,dit}.rs`** — All hardcode `0x70` for byte 1 of short-form sections. `mod.rs` defines `SECTION_B1_FLAGS_DVB` and `SECTION_B1_FLAGS_PSI` but no `SECTION_B1_FLAGS_SHORT`.

### RST uses wrong error variant for alignment
**`tables/rst.rs:79-84`** — `section_length % ENTRY_LEN != 0` returns `SectionLengthOverflow` but it's an alignment error, not an overflow.

### Text decoder silently passes unsupported charsets through as Latin-1
**`text/mod.rs:546`** — When `bytes[2]` names an unsupported ISO 8859 part (e.g., 12), the `_ =>` arm returns raw bytes as `char`, producing plausible-looking garbage instead of `U+FFFD` replacement characters.

---

## 🟢 Low — Code Quality

### Inconsistent constant naming patterns across tables
Some use `HEADER_LEN`/`FIXED_BODY_LEN` (`int.rs`, `unt.rs`, `rnt.rs`), others use `MIN_HEADER_LEN`/`EXTENSION_HEADER_LEN` (`nit.rs`, `sdt.rs`). Same concept, different names.

### Double hash lookup in demux gate
**`demux.rs:484,493`** — `contains_key` followed by `insert` can be collapsed to `HashMap::entry()`, eliminating a hash + lookup.

### Fresh `Vec` allocation on every `feed()` call
**`demux.rs:393`** — `let mut completed: Vec<Bytes> = Vec::new()` allocates every call even when zero sections complete. `scratch` already shows the correct reusable pattern.

### `up_iter()` returns `Box<dyn Iterator>` — unnecessary heap allocation
**`dvb-bbframe/packet.rs:137-145`** — Returns `Box<dyn Iterator>` when an `enum` return would avoid allocation + vtable overhead.

### Pervasive unnamed magical bit-masks in `issy.rs`
**`dvb-bbframe/issy.rs:126-177`** — `0x80`, `0x7F`, `0x3F`, `0x40`, `0x03`, `0x03FF`, `0x0F_FFFF`, shifts `20, 18, 8, 16, 15` — all unnamed. Should be `const` items matching spec field names.

### `Bbheader`/`Matype` all-pub fields allow by-construction bypass of validation
**`dvb-bbframe/header.rs:233-248`** — `dfl=65535`, `ext=255`, NM mode with `issy_in_header=Some(...)`, etc. all bypass `parse()` validation. Either make these private with getters or accept the escape hatch is intentional.

### `remaining()` methods panic on OOB via unchecked slice
**`dvb-bbframe/packet.rs:56,106`** — `self.data[self.pos..]` panics if `pos > data.len()`. While `pos` is only advanced in bounds-checked `next()`, the method is `pub`. Should use `get(..)`.

---

## ✅ Clean / No Issues Found

- **Memory leaks**: None. All `Drop`-safe types, no reference cycles, all scratch buffers properly cleared.
- **CRC implementations**: Correct for both CRC-32 MPEG-2 (dvb-common/t2mi) and CRC-8 (bbframe).
- **Performance hot paths**: Zero-copy throughout. No `O(n²)` algorithms.
- **Index-out-of-bounds**: All guarded by length checks except the noted SAT `BitReader`/`BitWriter` case.
- **nit.rs / sdt.rs / tsdt.rs**: Gold-standard implementations.
