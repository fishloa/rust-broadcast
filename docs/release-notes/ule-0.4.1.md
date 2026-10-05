# ule 0.4.1

_Released 2026-10-05._

Patch release: four defects in how the crate frames and reassembles ULE (RFC 4326) SNDUs, all from the #1120 audit, plus one additive public method that exposes a check the serializer now performs. No API is removed or changed; nobody must act. You should upgrade if you feed `ts::UleReceiver` real transport streams (two of the fixes change which SNDUs it recovers) or if you build `PayloadChain`/`Sndu` values by hand (they now fail with an `Err` where they used to panic or emit a misframed chain).

## Fixes in the TS receiver (`ts::UleReceiver`)

- **Legal SNDUs starting with `0xFF` were discarded as padding.** The receiver treated any leading `0xFF` byte as stuffing. A legal `D=1` SNDU whose `Length` is in `0x7F00..=0x7FFE` also starts with `0xFF` on the wire, so its header was silently thrown away and the receiver lost sync for the rest of the packet. Only the genuine 2-byte `0xFFFF` End Indicator, or a single trailing `0xFF` byte too short to hold any header, now ends the packing walk (#1120).
- **Leftover bytes after a PUSI=0 continuation were parsed as a new SNDU.** RFC 4326 section 6/7 lets an SNDU start only where the Payload Pointer of a PUSI=1 packet says one does. Non-padding bytes that followed the end of a completing SNDU in a PUSI=0 packet were nevertheless walked as a new packed SNDU, corrupting every later packing decision. Such bytes now reset the receiver to the Idle State; trailing `0xFF` padding is still accepted (#1120).

## Fixes in serialization

- **`PayloadChain::serialize_into` panicked or misframed on inconsistent Optional extension headers.** It trusted `h_len` over `body.len()`: a `body` longer than `2*h_len-2` could panic on the output slice, a shorter one produced bytes that put the next Type field in the wrong place, and `h_len == 0` panicked with a `usize` underflow inside `serialized_len()` itself. Every header is now validated before any byte is written, and a bad one returns `Error::InvalidExtensionHeader`; `serialized_len()` no longer underflows (#1120).
- **`Sndu::serialize_into` could emit the End Indicator as a header.** `D=1` with `Length=0x7FFF` serializes to `0xFFFF`, which every receiver, including this crate's own `ts::UleReceiver`, reads as "no more SNDUs in this packet", so the SNDU was silently truncated. That combination is now rejected with `Error::InvalidLength` (#1120).

## New API

- `ExtensionHeader::validate(&self) -> Result<()>`: for an `Optional` header it checks that `h_len` is in `1..=5` and that `body.len() == 2*h_len-2`; a `Mandatory` header always passes. This is the same check `PayloadChain::serialize_into` now runs, so you can validate a header at construction time instead of at serialize time.

Migration: if you build `ExtensionHeader::Optional { h_len, h_type, body }` by hand, make sure `body.len() == 2 * h_len as usize - 2` and `h_len <= 5`; previously a violation panicked or produced a bad chain, now it is an `Err`.

## Dependencies

`broadcast-common` `9.3` -> `9.4` (see `broadcast-common-9.4.0.md`). No other `Cargo.toml` change.

---

Published from tag `ule-v0.4.1`.
