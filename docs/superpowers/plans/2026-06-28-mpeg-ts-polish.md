# mpeg-ts Polish Pass Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Apply owner code-review findings to `mpeg-ts`: rename `TsPacketBuf` → `OwnedTsPacket`, add typed `ScramblingControl` + `AdaptationFieldControl` enums with #204-compliant label accessors, add `iter_packets` + `extract_ts_payload` helpers, add `discontinuity` field to `OwnedTsPacket`, and update `label_coverage.rs`.

**Architecture:** All changes are confined to `mpeg-ts/src/` (rename `packet_buf.rs` → `owned.rs`, edit `ts.rs` for new enums + helpers, update `lib.rs` re-exports). No new crate dependencies. The two new enums go in `ts.rs` (where `TsHeader` lives); the renamed struct moves from `packet_buf.rs` to `owned.rs`. All gates (build, test, clippy, fmt, doc, label_coverage, no_std, MSRV) must be green.

**Tech Stack:** Rust 1.81 (MSRV), `no_std` + `alloc`, `dvb_common::impl_spec_display!`, `#[non_exhaustive]`, `#[cfg_attr(feature = "serde", derive(serde::Serialize))]`.

## Global Constraints

- MSRV: **1.81** — no newer Rust features.
- `#![no_std]` + `alloc` — no `std::` in `src/`; tests may use `std`.
- All public enums need `name() -> &'static str` + `dvb_common::impl_spec_display!(Name)`.
- `#[non_exhaustive]` on all new enums.
- `#[cfg_attr(feature = "serde", derive(serde::Serialize))]` on new public types.
- Keep raw `scrambling: u8` and `has_adaptation`/`has_payload: bool` fields — only add accessors on top.
- Commit message: `feat(mpeg-ts): typed ScramblingControl + AdaptationFieldControl accessors, rename OwnedTsPacket, iter_packets helper`
- Report to: `/Volumes/External/Projects/rust-dvb/.superpowers/sdd/mt-polish-report.md`

---

## File Map

| File | Action | Purpose |
|---|---|---|
| `mpeg-ts/src/packet_buf.rs` | Delete (rename to `owned.rs`) | Old location for `TsPacketBuf` |
| `mpeg-ts/src/owned.rs` | Create (was `packet_buf.rs`) | `OwnedTsPacket` with renamed struct + `discontinuity` field + `scrambling_control()` + `adaptation_field_control()` |
| `mpeg-ts/src/ts.rs` | Modify | Add `ScramblingControl` + `AdaptationFieldControl` enums; add `scrambling_control()` + `adaptation_field_control()` accessors on `TsHeader`; add `iter_packets()` + `extract_ts_payload()` free functions |
| `mpeg-ts/src/lib.rs` | Modify | Change `pub mod packet_buf` → `pub mod owned`; update re-export from `TsPacketBuf` → `OwnedTsPacket` |
| `mpeg-ts/tests/label_coverage.rs` | Modify | Update `SKIP` comment; verify guard covers new enums |
| `mpeg-ts/examples/demux.rs` | No change needed | Doesn't reference `TsPacketBuf` |

---

### Task 1: Rename `packet_buf.rs` → `owned.rs` and struct `TsPacketBuf` → `OwnedTsPacket`

**Files:**
- Delete: `mpeg-ts/src/packet_buf.rs` (copy content to `owned.rs` first)
- Create: `mpeg-ts/src/owned.rs`
- Modify: `mpeg-ts/src/lib.rs`

**Interfaces:**
- Produces: `pub struct OwnedTsPacket` in `mpeg-ts::owned` — same public fields as `TsPacketBuf`
- Produces: `mpeg_ts::owned::OwnedTsPacket` re-exported from `mpeg_ts` root

- [ ] **Step 1: Create `owned.rs` from `packet_buf.rs`**

Copy `mpeg-ts/src/packet_buf.rs` content to `mpeg-ts/src/owned.rs`, rename every occurrence of `TsPacketBuf` → `OwnedTsPacket`, and update the module-level doc comment to match:

```rust
//! Owned 188-byte TS packet with pre-parsed header fields.
//!
//! [`OwnedTsPacket`] complements the zero-copy [`crate::ts::TsPacket`] (which holds
//! a borrowed `&[u8; 188]`) with an **owned** `[u8; 188]` suitable for queuing,
//! cloning, and in-place mutation — e.g. for mux pipelines that must rewrite the
//! continuity counter or splice in a new payload.
//!
//! Header parsing delegates to [`crate::ts::TsHeader::parse`]; no bit-twiddling
//! is duplicated here.

use crate::error::{Error, Result};
use crate::ts::{TsHeader, TS_PACKET_SIZE, TS_SYNC_BYTE};

/// Owned 188-byte TS packet with pre-parsed header fields.
///
/// The raw bytes are stored in `raw`; the parsed flags (`pid`, `pusi`, etc.) are
/// pre-extracted at construction time so hot paths avoid repeated byte masking.
///
/// # Payload access
///
/// Use [`payload`](Self::payload) / [`payload_mut`](Self::payload_mut) to obtain
/// a slice that correctly skips the 4-byte header **and** any adaptation field.
///
/// # Building packets
///
/// [`serialize_with_payload`](Self::serialize_with_payload) constructs a plain
/// payload-only packet (no adaptation field) filled with 0xFF stuffing.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct OwnedTsPacket {
    /// The raw 188 bytes (serialized as a byte sequence).
    #[cfg_attr(feature = "serde", serde(serialize_with = "serialize_raw_bytes"))]
    pub raw: [u8; TS_PACKET_SIZE],
    /// 13-bit PID extracted from bytes 1–2.
    pub pid: u16,
    /// Payload Unit Start Indicator (byte 1 bit 6).
    pub pusi: bool,
    /// Adaptation field present flag (byte 3 bit 5).
    pub has_adaptation: bool,
    /// Payload present flag (byte 3 bit 4).
    pub has_payload: bool,
    /// Transport Error Indicator (byte 1 bit 7).
    pub tei: bool,
    /// 2-bit transport_scrambling_control (byte 3 bits 7–6).
    pub scrambling: u8,
    /// 4-bit continuity_counter (byte 3 bits 3–0).
    pub continuity_counter: u8,
    /// Discontinuity flag: `true` if the adaptation-field `discontinuity_indicator`
    /// was set in the source packet, or if the caller marks this as a
    /// continuity-counter discontinuity boundary. Defaults to `false` on parse.
    pub discontinuity: bool,
}

#[cfg(feature = "serde")]
fn serialize_raw_bytes<S: serde::Serializer>(
    bytes: &[u8; TS_PACKET_SIZE],
    s: S,
) -> core::result::Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = s.serialize_seq(Some(bytes.len()))?;
    for b in bytes {
        seq.serialize_element(b)?;
    }
    seq.end()
}

impl OwnedTsPacket {
    /// Parse a 188-byte owned TS packet.
    ///
    /// Returns [`Error::InvalidSyncByte`] if `raw[0] != 0x47`.
    /// Header bit-parsing is delegated to [`TsHeader::parse`].
    /// The `discontinuity` field defaults to `false`; set it manually if needed.
    pub fn parse(raw: [u8; TS_PACKET_SIZE]) -> Result<Self> {
        if raw[0] != TS_SYNC_BYTE {
            return Err(Error::InvalidSyncByte { found: raw[0] });
        }
        let hdr = TsHeader::parse(&raw[..4])?;
        Ok(Self {
            raw,
            pid: hdr.pid,
            pusi: hdr.pusi,
            has_adaptation: hdr.has_adaptation,
            has_payload: hdr.has_payload,
            tei: hdr.tei,
            scrambling: hdr.scrambling,
            continuity_counter: hdr.continuity_counter,
            discontinuity: false,
        })
    }

    /// Typed view of the 2-bit `transport_scrambling_control` field.
    ///
    /// Delegates to [`ScramblingControl::from_bits`]; see its doc for the spec citation.
    pub fn scrambling_control(&self) -> crate::ts::ScramblingControl {
        crate::ts::ScramblingControl::from_bits(self.scrambling)
    }

    /// Typed view of the `adaptation_field_control` 2-bit field, derived from the
    /// stored `has_adaptation`/`has_payload` booleans.
    ///
    /// Delegates to [`AdaptationFieldControl::from_flags`]; see its doc for the spec citation.
    pub fn adaptation_field_control(&self) -> crate::ts::AdaptationFieldControl {
        crate::ts::AdaptationFieldControl::from_flags(self.has_adaptation, self.has_payload)
    }

    /// Return the payload bytes (after the 4-byte header and any adaptation field).
    pub fn payload(&self) -> Option<&[u8]> {
        if !self.has_payload {
            return None;
        }
        let offset = self.payload_offset();
        if offset < TS_PACKET_SIZE {
            Some(&self.raw[offset..])
        } else {
            None
        }
    }

    /// Return a mutable slice of the payload bytes.
    pub fn payload_mut(&mut self) -> Option<&mut [u8]> {
        if !self.has_payload {
            return None;
        }
        let offset = self.payload_offset();
        if offset < TS_PACKET_SIZE {
            Some(&mut self.raw[offset..])
        } else {
            None
        }
    }

    #[inline]
    fn payload_offset(&self) -> usize {
        let mut offset = 4;
        if self.has_adaptation {
            let af_len = self.raw[4] as usize;
            offset += 1 + af_len;
        }
        offset
    }

    /// Build a 188-byte payload-only TS packet (no adaptation field).
    pub fn serialize_with_payload(
        pid: u16,
        pusi: bool,
        cc: u8,
        payload: &[u8],
    ) -> [u8; TS_PACKET_SIZE] {
        let mut pkt = [0xFFu8; TS_PACKET_SIZE];
        let hdr = TsHeader {
            tei: false,
            pusi,
            pid,
            scrambling: 0,
            has_adaptation: false,
            has_payload: true,
            continuity_counter: cc & 0x0F,
        };
        hdr.serialize_into(&mut pkt)
            .expect("serialize TsHeader into 188-byte buf");
        let copy_len = payload.len().min(184);
        pkt[4..4 + copy_len].copy_from_slice(&payload[..copy_len]);
        pkt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_round_trip_and_payload_mut() {
        let payload = [0xAAu8; 184];
        let mut pkt = OwnedTsPacket::parse(OwnedTsPacket::serialize_with_payload(
            0x0100, true, 7, &payload,
        ))
        .unwrap();
        assert_eq!(pkt.pid, 0x0100);
        assert!(pkt.pusi);
        assert_eq!(pkt.continuity_counter, 7);
        assert_eq!(pkt.payload().unwrap()[..184], payload[..]);
        pkt.payload_mut().unwrap()[0] = 0x55;
        assert_eq!(pkt.payload().unwrap()[0], 0x55);
        // discontinuity defaults to false
        assert!(!pkt.discontinuity);
    }

    #[test]
    fn owned_scrambling_control_accessor() {
        use crate::ts::ScramblingControl;
        let make = |scrambling_bits: u8| -> OwnedTsPacket {
            let mut raw = OwnedTsPacket::serialize_with_payload(0x0100, false, 0, &[]);
            // byte 3 bits [7:6] = scrambling
            raw[3] = (raw[3] & 0x3F) | (scrambling_bits << 6);
            OwnedTsPacket::parse(raw).unwrap()
        };
        assert_eq!(make(0b00).scrambling_control(), ScramblingControl::NotScrambled);
        assert_eq!(make(0b01).scrambling_control(), ScramblingControl::Reserved);
        assert_eq!(make(0b10).scrambling_control(), ScramblingControl::EvenKey);
        assert_eq!(make(0b11).scrambling_control(), ScramblingControl::OddKey);
    }

    #[test]
    fn owned_adaptation_field_control_accessor() {
        use crate::ts::AdaptationFieldControl;
        let make = |afc_bits: u8| -> OwnedTsPacket {
            let mut raw = [0xFFu8; TS_PACKET_SIZE];
            raw[0] = TS_SYNC_BYTE;
            raw[1] = 0x00;
            raw[2] = 0x00;
            raw[3] = (afc_bits << 4) & 0x30;
            if afc_bits & 0b10 != 0 {
                raw[4] = 0; // adaptation_field_length = 0
            }
            OwnedTsPacket::parse(raw).unwrap()
        };
        assert_eq!(make(0b00).adaptation_field_control(), AdaptationFieldControl::Reserved);
        assert_eq!(make(0b01).adaptation_field_control(), AdaptationFieldControl::PayloadOnly);
        assert_eq!(make(0b10).adaptation_field_control(), AdaptationFieldControl::AdaptationOnly);
        assert_eq!(make(0b11).adaptation_field_control(), AdaptationFieldControl::AdaptationAndPayload);
    }
}
```

- [ ] **Step 2: Update `lib.rs` — swap `packet_buf` for `owned`**

Edit `mpeg-ts/src/lib.rs`:
- Change `pub mod packet_buf;` → `pub mod owned;`
- Change the `packet_buf::TsPacketBuf` entry in the doc table to `owned::OwnedTsPacket`
- Add `pub use owned::OwnedTsPacket;` re-export (if there was a `TsPacketBuf` re-export, replace it; otherwise just add)

The updated lib.rs table row:
```
| [`owned`] | [`owned::OwnedTsPacket`] | Owned 188-byte TS packet — for queuing, cloning, and in-place mutation |
```

- [ ] **Step 3: Delete `packet_buf.rs`**

```bash
rm /Volumes/External/Projects/rust-dvb/mpeg-ts/src/packet_buf.rs
```

- [ ] **Step 4: Check it builds**

```bash
cd /Volumes/External/Projects/rust-dvb && cargo build -p mpeg-ts --all-features --locked 2>&1 | head -40
```

Expected: no errors referencing `TsPacketBuf` or `packet_buf`.

- [ ] **Step 5: Commit**

```bash
git add mpeg-ts/src/owned.rs mpeg-ts/src/lib.rs && git rm mpeg-ts/src/packet_buf.rs
git commit -m "refactor(mpeg-ts): rename TsPacketBuf→OwnedTsPacket, packet_buf→owned"
```

---

### Task 2: Add `ScramblingControl` and `AdaptationFieldControl` enums to `ts.rs`

**Files:**
- Modify: `mpeg-ts/src/ts.rs`

**Interfaces:**
- Produces: `pub enum ScramblingControl` with `from_bits(u8) -> Self`, `name() -> &'static str`, `Display`
- Produces: `pub enum AdaptationFieldControl` with `from_flags(bool, bool) -> Self`, `name() -> &'static str`, `Display`
- Produces: `TsHeader::scrambling_control(&self) -> ScramblingControl`
- Produces: `TsHeader::adaptation_field_control(&self) -> AdaptationFieldControl`

- [ ] **Step 1: Add `ScramblingControl` enum before `TsHeader`**

Insert in `ts.rs` after the constants block (around line 36, before `TsHeader` definition):

```rust
/// 2-bit `transport_scrambling_control` field — ITU-T H.222.0 (08/2023) Table 2-4
/// (defines only `00` = not scrambled); DVB extends `01`/`10`/`11` in ETSI TS 100 289
/// V1.1.1 §5.1 Table 1 (even/odd CW, reserved).
///
/// The MPEG-2 spec leaves `01`/`10`/`11` as user-defined; DVB's common-scrambling
/// convention assigns `10` = even control word, `11` = odd control word. `01` is
/// reserved for future DVB use.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum ScramblingControl {
    /// `00` — not scrambled. The only MPEG-2-defined value (H.222.0 Table 2-4).
    NotScrambled,
    /// `01` — reserved for future DVB use (TS 100 289 §5.1 Table 1).
    Reserved,
    /// `10` — TS packet payload scrambled with the **even** control word
    ///  (DVB common scrambling, TS 100 289 §5.1 Table 1).
    EvenKey,
    /// `11` — TS packet payload scrambled with the **odd** control word
    ///  (DVB common scrambling, TS 100 289 §5.1 Table 1).
    OddKey,
}

impl ScramblingControl {
    /// Decode from the 2-bit `transport_scrambling_control` value (masked to `[1:0]`).
    pub fn from_bits(bits: u8) -> Self {
        match bits & 0b11 {
            0b00 => Self::NotScrambled,
            0b01 => Self::Reserved,
            0b10 => Self::EvenKey,
            0b11 => Self::OddKey,
            _ => unreachable!(),
        }
    }

    /// Short label for this value, per the #204 convention.
    pub fn name(&self) -> &'static str {
        match self {
            Self::NotScrambled => "not_scrambled",
            Self::Reserved     => "reserved",
            Self::EvenKey      => "even_key",
            Self::OddKey       => "odd_key",
        }
    }
}

dvb_common::impl_spec_display!(ScramblingControl);
```

- [ ] **Step 2: Add `AdaptationFieldControl` enum after `ScramblingControl`**

```rust
/// 2-bit `adaptation_field_control` field — ITU-T H.222.0 (08/2023) Table 2-5.
///
/// Decoders shall discard packets with value `00` (`Reserved`). Null packets use `01`
/// (`PayloadOnly`). The two flags `has_adaptation`/`has_payload` on [`TsHeader`] carry
/// the decoded booleans; this enum provides the typed composite view.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum AdaptationFieldControl {
    /// `00` — reserved; decoders shall discard (H.222.0 Table 2-5).
    Reserved,
    /// `01` — no adaptation_field, payload only (H.222.0 Table 2-5).
    PayloadOnly,
    /// `10` — adaptation_field only, no payload (H.222.0 Table 2-5).
    AdaptationOnly,
    /// `11` — adaptation_field followed by payload (H.222.0 Table 2-5).
    AdaptationAndPayload,
}

impl AdaptationFieldControl {
    /// Derive from the two decoded boolean flags stored on [`TsHeader`].
    pub fn from_flags(has_adaptation: bool, has_payload: bool) -> Self {
        match (has_adaptation, has_payload) {
            (false, false) => Self::Reserved,
            (false, true)  => Self::PayloadOnly,
            (true,  false) => Self::AdaptationOnly,
            (true,  true)  => Self::AdaptationAndPayload,
        }
    }

    /// Short label for this value, per the #204 convention.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Reserved             => "reserved",
            Self::PayloadOnly          => "payload_only",
            Self::AdaptationOnly       => "adaptation_only",
            Self::AdaptationAndPayload => "adaptation_and_payload",
        }
    }
}

dvb_common::impl_spec_display!(AdaptationFieldControl);
```

- [ ] **Step 3: Add accessors to `TsHeader`**

In the `impl TsHeader` block, after `serialize_into`, add:

```rust
/// Typed view of the 2-bit `transport_scrambling_control` field.
///
/// See [`ScramblingControl`] for the spec citation (H.222.0 Table 2-4 + TS 100 289 §5.1).
pub fn scrambling_control(&self) -> ScramblingControl {
    ScramblingControl::from_bits(self.scrambling)
}

/// Typed view of the `adaptation_field_control` 2-bit field, derived from the
/// `has_adaptation`/`has_payload` flags.
///
/// See [`AdaptationFieldControl`] for the spec citation (H.222.0 Table 2-5).
pub fn adaptation_field_control(&self) -> AdaptationFieldControl {
    AdaptationFieldControl::from_flags(self.has_adaptation, self.has_payload)
}
```

- [ ] **Step 4: Add unit tests for all 4 values of each enum in `ts.rs`**

In the existing `mod tests` block, add:

```rust
#[test]
fn scrambling_control_all_values() {
    assert_eq!(ScramblingControl::from_bits(0b00), ScramblingControl::NotScrambled);
    assert_eq!(ScramblingControl::from_bits(0b01), ScramblingControl::Reserved);
    assert_eq!(ScramblingControl::from_bits(0b10), ScramblingControl::EvenKey);
    assert_eq!(ScramblingControl::from_bits(0b11), ScramblingControl::OddKey);
    // name() labels
    assert_eq!(ScramblingControl::NotScrambled.name(), "not_scrambled");
    assert_eq!(ScramblingControl::Reserved.name(),     "reserved");
    assert_eq!(ScramblingControl::EvenKey.name(),      "even_key");
    assert_eq!(ScramblingControl::OddKey.name(),       "odd_key");
    // Display delegates to name()
    assert_eq!(ScramblingControl::NotScrambled.to_string(), "not_scrambled");
    assert_eq!(ScramblingControl::OddKey.to_string(),       "odd_key");
    // Masking: only low 2 bits matter
    assert_eq!(ScramblingControl::from_bits(0xFF), ScramblingControl::OddKey);
}

#[test]
fn adaptation_field_control_all_values() {
    assert_eq!(AdaptationFieldControl::from_flags(false, false), AdaptationFieldControl::Reserved);
    assert_eq!(AdaptationFieldControl::from_flags(false, true),  AdaptationFieldControl::PayloadOnly);
    assert_eq!(AdaptationFieldControl::from_flags(true,  false), AdaptationFieldControl::AdaptationOnly);
    assert_eq!(AdaptationFieldControl::from_flags(true,  true),  AdaptationFieldControl::AdaptationAndPayload);
    // name()
    assert_eq!(AdaptationFieldControl::Reserved.name(),             "reserved");
    assert_eq!(AdaptationFieldControl::PayloadOnly.name(),          "payload_only");
    assert_eq!(AdaptationFieldControl::AdaptationOnly.name(),       "adaptation_only");
    assert_eq!(AdaptationFieldControl::AdaptationAndPayload.name(), "adaptation_and_payload");
    // Display
    assert_eq!(AdaptationFieldControl::PayloadOnly.to_string(), "payload_only");
}

#[test]
fn ts_header_scrambling_control_accessor() {
    let hdr = TsHeader {
        tei: false, pusi: false, pid: 0x0100,
        scrambling: 0b10,
        has_adaptation: false, has_payload: true, continuity_counter: 0,
    };
    assert_eq!(hdr.scrambling_control(), ScramblingControl::EvenKey);
}

#[test]
fn ts_header_adaptation_field_control_accessor() {
    let hdr_payload_only = TsHeader {
        tei: false, pusi: false, pid: 0x0100,
        scrambling: 0, has_adaptation: false, has_payload: true, continuity_counter: 0,
    };
    assert_eq!(hdr_payload_only.adaptation_field_control(), AdaptationFieldControl::PayloadOnly);

    let hdr_both = TsHeader {
        tei: false, pusi: false, pid: 0x0100,
        scrambling: 0, has_adaptation: true, has_payload: true, continuity_counter: 0,
    };
    assert_eq!(hdr_both.adaptation_field_control(), AdaptationFieldControl::AdaptationAndPayload);
}
```

- [ ] **Step 5: Build + test**

```bash
cd /Volumes/External/Projects/rust-dvb && cargo test -p mpeg-ts --all-features --locked 2>&1 | tail -20
```

Expected: all tests pass, including new ones.

- [ ] **Step 6: Commit**

```bash
git add mpeg-ts/src/ts.rs
git commit -m "feat(mpeg-ts): ScramblingControl + AdaptationFieldControl enums with #204 labels"
```

---

### Task 3: Add `iter_packets` and `extract_ts_payload` helpers to `ts.rs`

**Files:**
- Modify: `mpeg-ts/src/ts.rs`

**Interfaces:**
- Produces: `pub fn iter_packets(buf: &[u8]) -> impl Iterator<Item = TsPacket<'_>>`
- Produces: `pub fn extract_ts_payload(pkt: &[u8]) -> Option<&[u8]>`

- [ ] **Step 1: Add helpers before the `#[cfg(test)]` block in `ts.rs`**

```rust
/// Iterate over all valid TS packets in a byte buffer.
///
/// Slices `buf` into 188-byte chunks (using [`chunks_exact`](slice::chunks_exact))
/// and yields each chunk for which [`TsPacket::parse`] succeeds. Chunks with a bad
/// sync byte (`!= 0x47`) or insufficient length are silently skipped — use
/// [`resync::TsResync`](crate::resync::TsResync) for byte-stream resynchronisation
/// before calling this when byte alignment is not guaranteed.
///
/// # Example
///
/// ```no_run
/// # use mpeg_ts::ts::iter_packets;
/// # let data: &[u8] = &[];
/// for pkt in iter_packets(data) {
///     println!("PID: 0x{:04X}", pkt.header.pid);
/// }
/// ```
pub fn iter_packets(buf: &[u8]) -> impl Iterator<Item = TsPacket<'_>> {
    buf.chunks_exact(TS_PACKET_SIZE)
        .filter_map(|chunk| TsPacket::parse(chunk).ok())
}

/// Extract the payload bytes from a raw 188-byte TS packet slice.
///
/// Returns `None` when:
/// - `pkt` is fewer than 4 bytes,
/// - `adaptation_field_control` is `00` (reserved) or `10` (adaptation only), or
/// - the adaptation field length would place the payload start past the packet end.
///
/// No sync-byte check is performed — the caller is responsible for ensuring the
/// slice is properly aligned. Spec: ITU-T H.222.0 (08/2023) §2.4.3.3 Table 2-5.
pub fn extract_ts_payload(pkt: &[u8]) -> Option<&[u8]> {
    if pkt.len() < 4 {
        return None;
    }
    let afc = (pkt[3] >> 4) & 0x3;
    match afc {
        0x1 => {
            // payload only: payload starts at byte 4
            if pkt.len() > 4 { Some(&pkt[4..]) } else { None }
        }
        0x3 => {
            // adaptation field + payload
            if pkt.len() < 5 {
                return None;
            }
            let af_len = pkt[4] as usize;
            let start = 5 + af_len;
            if start < pkt.len() { Some(&pkt[start..]) } else { None }
        }
        _ => None,
    }
}
```

- [ ] **Step 2: Add unit test for `iter_packets`**

In the `mod tests` block:

```rust
#[test]
fn iter_packets_yields_valid_and_skips_bad_sync() {
    // Two valid packets back-to-back, then one bad-sync packet.
    let pkt1 = make_packet(0x00, 0x00, PAYLOAD_FLAG, &[0xAA; 10]);
    let pkt2 = make_packet(0x40, 0x64, PAYLOAD_FLAG, &[0xBB; 10]);
    let mut bad = [0u8; TS_PACKET_SIZE];
    bad[0] = 0x00; // bad sync byte

    let mut buf = Vec::new();
    buf.extend_from_slice(&pkt1);
    buf.extend_from_slice(&pkt2);
    buf.extend_from_slice(&bad);

    let pkts: Vec<_> = super::iter_packets(&buf).collect();
    assert_eq!(pkts.len(), 2, "bad sync packet must be skipped");
    assert_eq!(pkts[0].header.pid, 0x0000);
    assert_eq!(pkts[1].header.pid, 0x0064);
}

#[test]
fn extract_ts_payload_payload_only() {
    let pkt = make_packet(0x00, 0x00, PAYLOAD_FLAG, &[0xAB; 10]);
    let p = super::extract_ts_payload(&pkt).expect("payload present");
    assert_eq!(p[0], 0xAB);
    assert_eq!(p.len(), TS_PACKET_SIZE - 4);
}

#[test]
fn extract_ts_payload_adaptation_only_returns_none() {
    let pkt = make_packet(0x00, 0x00, ADAPTATION_FLAG, &[]);
    assert!(super::extract_ts_payload(&pkt).is_none());
}
```

- [ ] **Step 3: Build + test**

```bash
cd /Volumes/External/Projects/rust-dvb && cargo test -p mpeg-ts --all-features --locked 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add mpeg-ts/src/ts.rs
git commit -m "feat(mpeg-ts): iter_packets + extract_ts_payload helpers"
```

---

### Task 4: Update `label_coverage.rs` with comment about coded-u8 fields

**Files:**
- Modify: `mpeg-ts/tests/label_coverage.rs`

**Interfaces:**
- Consumes: `ScramblingControl` and `AdaptationFieldControl` now in `src/ts.rs`
- Produces: guard passes (enums have Display via `impl_spec_display!`)

- [ ] **Step 1: Add `ScramblingControl` and `AdaptationFieldControl` to the comment/note in `label_coverage.rs`**

The existing guard already works for new enums (it scans `src/` for `pub enum` and checks `impl_spec_display!` or `Display for`). We just need to add a clarifying comment about the coded-u8 fields audit.

Update the `SKIP` comment in `label_coverage.rs`:

```rust
/// Enums that are intentionally **not** spec/field labels.
///
/// - `Error` — structured error enum, not a spec/field label.
///
/// # Coded `u8` fields vs. public enums
///
/// This guard checks that every `pub enum` in `src/` has a `Display` impl
/// (via `dvb_common::impl_spec_display!`). It does **not** check raw `u8`
/// fields that encode spec values without a typed enum accessor — those
/// require manual review.
///
/// Audited coded-u8 fields in `mpeg-ts`:
/// - `TsHeader::scrambling` (2-bit) → typed accessor `scrambling_control()` → `ScramblingControl` (covered by guard)
/// - `TsHeader::continuity_counter` (4-bit) → counter, not a spec label, no enum needed
/// - `TsHeader::pid` (13-bit) → see `pid::Pid`; raw u16 on `TsHeader` by design
const SKIP: &[&str] = &["Error"];
```

- [ ] **Step 2: Run label_coverage**

```bash
cd /Volumes/External/Projects/rust-dvb && cargo test -p mpeg-ts --test label_coverage --locked 2>&1
```

Expected: `test every_public_spec_enum_has_a_display_impl ... ok`

- [ ] **Step 3: Commit**

```bash
git add mpeg-ts/tests/label_coverage.rs
git commit -m "test(mpeg-ts): annotate label_coverage with coded-u8 field audit notes"
```

---

### Task 5: Full gate sweep + amend commit message

**Files:** none new

- [ ] **Step 1: Full build + test**

```bash
cd /Volumes/External/Projects/rust-dvb && cargo build --workspace --all-features --locked && cargo test --workspace --all-features --locked 2>&1 | tail -30
```

- [ ] **Step 2: no-default-features build**

```bash
cd /Volumes/External/Projects/rust-dvb && cargo build --workspace --no-default-features --locked 2>&1 | tail -10
```

- [ ] **Step 3: no_std embedded target build**

```bash
cd /Volumes/External/Projects/rust-dvb && cargo build -p mpeg-ts --no-default-features --target thumbv7em-none-eabi --locked 2>&1 | tail -10
```

Expected: success (no_std, no panic runtime needed — mpeg-ts is `#![no_std]`).

- [ ] **Step 4: clippy**

```bash
cd /Volumes/External/Projects/rust-dvb && cargo clippy --workspace --all-features --all-targets --locked -- -D warnings 2>&1 | tail -20
```

Expected: no warnings.

- [ ] **Step 5: fmt check**

```bash
cd /Volumes/External/Projects/rust-dvb && cargo fmt --all --check 2>&1
```

Expected: no diffs. If there are diffs, run `cargo fmt --all` and commit the result.

- [ ] **Step 6: doc check**

```bash
cd /Volumes/External/Projects/rust-dvb && RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked 2>&1 | tail -20
```

Expected: no warnings.

- [ ] **Step 7: MSRV check**

```bash
cd /Volumes/External/Projects/rust-dvb && rustup run 1.81 cargo build --workspace --all-features --locked 2>&1 | tail -10
```

Expected: success.

- [ ] **Step 8: label_coverage**

```bash
cd /Volumes/External/Projects/rust-dvb && cargo test -p mpeg-ts --test label_coverage --locked 2>&1
```

Expected: pass.

---

### Task 6: Write report + final squash commit

- [ ] **Step 1: Ensure `.superpowers/sdd/` directory exists**

```bash
mkdir -p /Volumes/External/Projects/rust-dvb/.superpowers/sdd/
```

- [ ] **Step 2: Write report**

Write `/Volumes/External/Projects/rust-dvb/.superpowers/sdd/mt-polish-report.md` with:
- Status (PASS/FAIL)
- Commit SHA + subject
- Each item done (1–6)
- Gate results (build, test, clippy, fmt, doc, label_coverage, no_std thumbv7em, MSRV 1.81)
- Any concerns

- [ ] **Step 3: Final commit**

```bash
git add /Volumes/External/Projects/rust-dvb/.superpowers/sdd/mt-polish-report.md
git commit -m "feat(mpeg-ts): typed ScramblingControl + AdaptationFieldControl accessors, rename OwnedTsPacket, iter_packets helper"
```

---

## Self-Review

**Spec coverage:**
- [x] Item 1: `TsPacketBuf` → `OwnedTsPacket` rename — Task 1
- [x] Item 2: `ScramblingControl` enum + `from_bits` + `name()` + `impl_spec_display!` + accessor on `TsHeader` + `OwnedTsPacket` — Task 2
- [x] Item 3: `AdaptationFieldControl` enum + `from_flags` + `name()` + `impl_spec_display!` + accessor on `TsHeader` + `OwnedTsPacket` — Task 2
- [x] Item 4: `iter_packets` + `extract_ts_payload` helpers — Task 3
- [x] Item 5: `discontinuity: bool` on `OwnedTsPacket` — Task 1 (included in `owned.rs`)
- [x] Item 6: `label_coverage.rs` updated — Task 4
- [x] All verify gates — Task 5
- [x] Report — Task 6

**Placeholder scan:** No TBDs, TODOs, or vague steps — all code is written out.

**Type consistency:**
- `ScramblingControl::from_bits(u8)` → Task 2 defines it; `OwnedTsPacket::scrambling_control()` in Task 1 calls `crate::ts::ScramblingControl::from_bits(self.scrambling)` — consistent.
- `AdaptationFieldControl::from_flags(bool, bool)` → Task 2 defines it; `OwnedTsPacket::adaptation_field_control()` calls `crate::ts::AdaptationFieldControl::from_flags(...)` — consistent.
- `TsHeader::scrambling_control()` / `TsHeader::adaptation_field_control()` — Task 2 defines these.
- `OwnedTsPacket::scrambling_control()` / `OwnedTsPacket::adaptation_field_control()` — Task 1 (`owned.rs`) defines these delegating to `crate::ts::...`.
