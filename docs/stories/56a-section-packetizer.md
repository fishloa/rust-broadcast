# Story 56a — `SectionPacketizer` (dvb-si `ts` feature)

Delegated brief for the section→TS packetizer, the first half of GitHub issue
**#56** ("section-to-TS packetizer + SiMux scheduler — the missing output half").
Self-contained: everything you need is here or in the linked in-repo spec
transcriptions. Touch only the files listed. Do **not** commit.

## Spec sources (read FIRST — these are the ground truth, cite them in code)

All wire rules come from in-repo transcriptions of vendored ETSI/ISO PDFs. Do
**not** invent values or citations — every constant must trace to one of these:

- **`dvb-si/docs/iso_13818_1_systems.md`** → "PSI section carriage and
  pointer_field (§2.4.4–2.4.4.2)". The packetizer's core rules: PUSI semantics,
  `pointer_field` meaning, section concatenation, and 0xFF stuffing.
- **`dvb-si/docs/en_300_468.md`** → "§5.1.4 — Repetition rates and random access"
  (informational here; the 25 ms floor belongs to the scheduler in story 56b,
  not this packetizer).

## What to build

A new module `dvb-si/src/mux.rs`, `#[cfg(feature = "ts")]`, registered in
`dvb-si/src/lib.rs` next to `pub mod ts;` / `pub mod demux;`. Module doc comment
must cite ISO/IEC 13818-1 §2.4.4 and name `docs/iso_13818_1_systems.md` (follow
the citation style in `ts.rs:223`).

```rust
pub struct SectionPacketizer {
    pid: u16,
    continuity_counter: u8,
}

impl SectionPacketizer {
    /// Start a packetizer for `pid` with continuity_counter = 0.
    pub fn new(pid: u16) -> Self;
    /// Start at a specific continuity_counter (0..=15) — for resuming a stream.
    pub fn with_continuity(pid: u16, cc: u8) -> Self;

    pub fn pid(&self) -> u16;
    pub fn continuity_counter(&self) -> u8;

    /// Packetize a batch of complete sections into 188-byte TS packets,
    /// appended to `out` (cleared first). Buffer-reuse primary API (mirror the
    /// `feed_*_into` pattern in dvb-bbframe). Returns the number of packets
    /// appended.
    pub fn packetize_into(&mut self, sections: &[&[u8]], out: &mut Vec<[u8; TS_PACKET_SIZE]>) -> usize;

    /// Allocating convenience wrapper over `packetize_into`.
    pub fn packetize(&mut self, sections: &[&[u8]]) -> Vec<[u8; TS_PACKET_SIZE]>;
}
```

Reuse `TS_PACKET_SIZE`, `TS_SYNC_BYTE`, and `TsHeader` (with its
`serialize_into`) from `crate::ts` — do not duplicate header-bit constants.

### The packetize algorithm (implement exactly — it is the inverse of `SectionReassembler::feed`)

Concatenate the batch's sections into one byte stream `data`; record `starts` =
the byte offset where each section begins (`0, len0, len0+len1, …`). Emit packets
left-to-right over `data`, advancing the per-PID `continuity_counter` (wrap
0..=15) on **every** emitted packet (all packets here carry payload). For each
packet, with `pos` = next unconsumed byte of `data` and `nextStart` = smallest
section-start offset `≥ pos`:

- **A section begins in this packet** iff `nextStart - pos ≤ 183`. Then:
  `PUSI = 1`; payload byte 0 = `pointer_field = (nextStart - pos) as u8`; the
  following up to 183 bytes are `data[pos..]`. (`pointer_field` is the count of
  in-progress-section tail bytes before the first new section — §2.4.4.2.)
- **Otherwise** (pure continuation of a spanning section): `PUSI = 0`, no
  pointer byte, up to 184 bytes of `data[pos..]`.
- Only the **first** section start in a packet gets the pointer; further
  concatenated sections in the same packet just follow contiguously (the
  reassembler's drain loop recovers them — do not insert extra pointers).
- When `data` is exhausted mid-packet, **0xFF-stuff** the remaining payload bytes
  to fill all 188 (the reassembler treats a `0xFF` where a `table_id` is expected
  as stuffing). Stuffing occurs only at the end of the batch.
- TS header for every packet: `pid = self.pid`, `has_payload = true`,
  `has_adaptation = false`, `tei = false`, `scrambling = 0`, the computed `pusi`
  and `continuity_counter`. Build it via `TsHeader { … }.serialize_into(&mut pkt[..4])`.

Edge cases that MUST be handled (they are the acceptance test matrix): a section
ending exactly at a packet boundary (next section's `pointer_field = 0` in a
fresh PUSI packet); multiple short sections in one packet; a `pointer_field`
with a preceding spanning-section tail; a single section larger than one packet
(spans many continuation packets); a 1-byte-body section; a maximal 4096-byte
section.

## Tests (in-module `#[cfg(test)]`) — the reassembler is the oracle

The hard project invariant is byte-identical round-trip. Write:

1. **Round-trip property/iteration test:** for a spread of section-size mixes —
   include 1-byte body, sizes that land a section boundary exactly at 184/183/188,
   several short sections in one batch, and a >4096-spanning mix — `packetize`
   then feed each packet's payload+PUSI to `crate::ts::SectionReassembler` (use
   `TsPacket::parse` to recover `payload`/`pusi`), draining with a
   `while let Some(s) = r.pop_section()` loop, and assert the popped sections are
   **byte-identical** to the inputs in order. (No external proptest dep — a
   fixed table of representative sizes computed in the test is fine and keeps
   MSRV/`--no-default-features`-of-dev-deps clean.)
2. **Continuity counter:** asserts CC increments per packet and wraps 15→0 across
   a multi-packet batch, and that a second `packetize` call continues the CC.
3. **PUSI placement:** asserts PUSI is set exactly on packets where a section
   begins, and `pointer_field` equals the tail length before it.
4. **Stuffing:** asserts the final packet's unused tail is `0xFF` and that the
   reassembler discards it (no extra section, `is_empty()` afterwards).
5. **Fixture round-trip:** parse the sections out of `dvb-si/tests/fixtures/m6-single.ts`
   via the existing demux/reassembler, re-`packetize` them on their PID, feed the
   result through `SiDemux`, and assert zero malformed/dropped sections and the
   same `AnyTableSection` set. (If wiring `SiDemux` here is heavy, a
   `SectionReassembler` round-trip on the extracted sections is an acceptable
   substitute — but the property test above is mandatory.)

## Constraints (project conventions — non-negotiable)

- **No magic numbers** outside `#[cfg(test)]`: every literal is a named const or
  reuses a `crate::ts` const. The only bare literals allowed are `0`/`1` arithmetic
  and the `183`/`184` payload-capacity values **if** given named consts
  (`PUSI_PAYLOAD_CAP = 183`, `PAYLOAD_CAP = 184`) with a `// §2.4.4` comment.
- `0xFF` stuffing byte → a named const (e.g. `STUFFING_BYTE`).
- Symmetric with the reassembler; this is the `Serialize`-direction counterpart.
- `#[cfg(feature = "serde", …)]` is not needed (no new public data structs beyond
  the packetizer, which holds only `u16`/`u8`).
- Must build `--no-default-features` (the module is `ts`-gated, so it simply
  compiles out) and on MSRV 1.75.

## Gate commands (run ALL, fix until every one is green before finishing)

```
cargo build  --workspace --all-features --locked
cargo test   -p dvb-si --all-features --locked
cargo build  --workspace --no-default-features --locked
cargo clippy --workspace --all-features --all-targets --locked -- -D warnings
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc -p dvb-si --all-features --no-deps --locked
```

## Boundaries

- Touch only `dvb-si/src/mux.rs` (new) and `dvb-si/src/lib.rs` (one `pub mod`
  line + the `#[cfg(feature = "ts")]`). Do **not** touch `ts.rs`, `demux.rs`, or
  any table/descriptor module.
- Do **not** implement the `SiMux` scheduler — that is story 56b.
- Do **not** commit; leave the working tree for the orchestrator to audit.
