# Client API 2.0 — typed TS-stream parsing without byte bashing

**Date:** 2026-06-05
**Tracking:** [issue #16](https://github.com/fishloa/rust-dvb/issues/16) (subsumed and extended by this design)
**Release target:** 2.0.0 lockstep across the workspace (one breaking change: text-field newtypes + serde output shape)

## Goal

Clients (zenith's debug endpoint; a tvheadend-class system watching ~30 tuned
services) should feed TS bytes in and get typed, ready-to-use SI data out —
no hand-rolled section loops, no `match tag {…}` walls, no per-field text
decoding. Dispatch must be *discovered from the table/descriptor
implementations themselves* (trait consts + one declarative list), never a
hand-maintained case. Hot-path cost must stay near zero: 30 services parsing
everything may not hog CPU or memory.

## Non-goals

- BIOP object carousels, ES payloads, descrambling — unchanged scope.
- Deserializing decoded text back into DVB charset bytes (lossy; serialize-only).
- dvb-bbframe API changes — `up_iter` is already the right shape for the
  innermost hot path.

## Architecture: four additive layers

```
dvb_si::demux        SiDemux, SectionEvent            (`ts` feature; bytes dep)
dvb_si::tables       AnyTable<'a>, TableDef           (no feature)
dvb_si::descriptors  AnyDescriptor<'a>, parse_loop,
                     DescriptorDef, DescriptorRegistry
dvb_si::text         DvbText<'a>, LangCode            (serde behavior under `serde`)
dvb_t2mi::pump       T2miPump, T2miEvent, AnyPayload  (`ts` feature; bytes dep)
```

Each layer is usable without the one above it. Existing per-tag
`parse()`/`Serialize` and `SectionReassembler` remain.

Composition across crates (the real signal chain):

```rust
// satellite TS → T2-MI PID → BBFrames → inner TS → SI tables
for t2mi in t2mi_pump.feed_ts(&outer_ts_packet)? {
    if let AnyPayload::BbFrame(bb) = t2mi.payload()? {
        for inner_ts in bbframe::up_iter(bb.data_field()) {
            for section in si_demux.feed(&inner_ts)? {
                match section.table()? { /* typed */ }
            }
        }
    }
}
```

## Layer 0 — `SiDemux` (perf core)

```rust
let mut demux = SiDemux::builder()
    .follow_pat(true)       // default on: PAT → auto-add PMT PIDs
    .dvb_si_pids(true)      // default on: PAT/CAT/NIT/SDT/BAT/EIT/RST/TDT/TOT/SAT
    .pid(Pid::from(0x0999)) // extra PIDs
    .emit_repeats(false)    // default off: version gate active
    .build();

for event in demux.feed(&ts_packet)? { … }
```

- Per-PID `SectionReassembler` behind a PID filter table.
- **Version gate** (the perf feature): map keyed
  `(pid, table_id, table_id_extension, section_number)` →
  `(version_number, crc32)`. A completed section is emitted only if the key is
  new, the version changed, or the CRC changed. TDT (no version, no CRC):
  byte-compare of the whole 8-byte section. Steady state on a busy mux: reassemble →
  one map probe → drop. Zero parse, zero alloc, zero events.
- Gate map capped (configurable; default ~64k entries — sized for a full EIT
  schedule), oldest-evicted; evictions counted in stats.
- `SectionEvent { pid, bytes: Bytes }` — `'static`, refcount-clone, sendable.
  Accessors: `table_id()`, `version()`, `table_id_extension()`, `crc_ok()`.
  The reassembler's completed buffer is handed to `Bytes` without a copy.
- `feed()` returns a draining iterator over an internal scratch vec
  (allocation-free after warm-up).

## Layer 1+2 — trait-driven dispatch (no hand-written match anywhere)

Traits (per crate, same shape):

```rust
pub trait TableDef<'a>: Parse<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)];  // EIT [(0x4E,0x6F)]
    const NAME: &'static str;
}
pub trait DescriptorDef<'a>: Parse<'a> {
    const TAG: u8;
    const NAME: &'static str;
}
pub trait PayloadDef<'a>: Parse<'a> {            // dvb-t2mi
    const PACKET_TYPE: u8;
    const NAME: &'static str;
}
```

One declarative `macro_rules!` list per crate names the modules and generates:
the `AnyTable<'a>` / `AnyDescriptor<'a>` / `AnyPayload<'a>` enums (with
`Unknown` fallthrough variants), `From` impls, and the dispatch.

- **Descriptor dispatch:** static 256-entry fn-pointer LUT
  (`[Option<for<'a> fn(&'a [u8]) -> Result<AnyDescriptor<'a>>>; 256]`) built
  at compile time from the list. O(1), no match chain.
- **Table dispatch:** same idea over `TABLE_ID_RANGES`.
- **Completeness enforced by test:** every `DescriptorTag` / `TableId` /
  packet-type variant must have a dispatcher entry.
- Macros start as per-crate `macro_rules!`; promoted to dvb-common only if
  they stay textually identical.

`parse_loop(bytes) -> DescriptorIter<'a>`: lazy iterator yielding
`Result<AnyDescriptor<'a>>`. Truncated tail → one final `Err`, then stops.
Unknown tag → `AnyDescriptor::Unknown { tag, body }`. Never panics. Applies
to every raw loop slice tables already expose (PMT ES loops, SDT service
loops, EIT event loops, NIT/BAT TS loops…).

`SectionEvent::table() -> Result<AnyTable<'_>>` and type-keyed
`SectionEvent::parse::<T: TableDef>() -> Result<T>` both borrow the event's
own bytes (lazy, zero-copy).

### Open registry (client private tags)

`SiDemux` and standalone `DescriptorRegistry`/`TableRegistry` start from the
static LUT and accept runtime registrations:

```rust
registry.register::<MyEacemDescriptor>();
```

Runtime-registered types must be **owned** (`'static`). They surface as
`AnyDescriptor::Other { tag, value: Box<dyn DescriptorObject> }` with
`downcast_ref::<T>()`. `DescriptorObject: Debug + Send + Sync`; under the
`serde` feature also erased-serialize (optional `erased_serde` dep) so custom
descriptors appear decoded in JSON. Crate-known types stay zero-copy enum
variants; only the escape hatch boxes. Same pattern for custom tables
(`AnyTable::Other`).

## Layer 3 — text & serde (the 2.0 break)

- `DvbText<'a>(&'a [u8])` newtype on every DVB-text field;
  `LangCode([u8; 3])` on language/country codes.
- `Deref<Target = [u8]>` (byte-level code keeps compiling), `.decode() ->
  Cow<str>` (Annex A; allocates only when called), `Display`.
- Under `serde`: `DvbText` serializes as the decoded string, `LangCode` as a
  3-char string (`"fre"`, `"FRA"`). Serialize-only (no `Deserialize`),
  consistent with the borrowed-type serde house rule.
- Net effect (issue #16 acceptance):
  `serde_json::to_value(parse_loop(loop_bytes))` →
  `[{"shortEvent":{"language":"fre","eventName":"Journal Météo climat",…}},…]`
- **Breaking:** field types change (`&[u8]` → `DvbText`), JSON shape changes
  (byte arrays → strings). Released honestly as **2.0.0**, lockstep across
  all four crates (release workflow gates tag == every crate version).

## `T2miPump` (dvb-t2mi)

Mirror of `SiDemux` **without a version gate** (T2-MI payloads are not
versioned repeats):

```rust
let mut pump = T2miPump::new();
for pkt in pump.feed_ts(&outer_ts_packet)? {   // or feed_raw()
    match pkt.payload()? {                      // AnyPayload<'_>
        AnyPayload::BbFrame(bb) => { /* → bbframe::up_iter → SiDemux */ }
        …                                       // all 12 packet types + Unknown
    }
}
```

Wraps existing `ts.rs` extraction (T2-MI packets spanning TS packets, TS 102
773 §5.2) and `packet.rs` header parsing. CRC-32 checked per packet; failures
dropped + counted. `T2miEvent` owns its `Bytes`, `'static`. Behind dvb-t2mi's
`ts` feature with a new optional `bytes` dep (same arrangement as dvb-si).

## Error handling & observability

- Never panic on hostile input at any layer (reassembler hardened in audit
  round 4; demux/pump inherit).
- Malformed / CRC-failed input is dropped, not raised — but counted:
  `demux.stats()` / `pump.stats()` expose
  `emitted / suppressed_by_version_gate / crc_failures / reassembly_resets /
  gate_evictions`.
- Errors surface at typed access: `section.table()` → `Result` (unknown
  table_id is `AnyTable::Unknown`, not an error); `parse_loop` yields
  per-descriptor `Result`; registered-custom parse failures yield `Err`
  tagged with the offending tag.

## Testing

1. **End-to-end fixture runs:** `SiDemux` over the live captures; assert the
   discovered table set; feed the same capture twice → second pass emits
   zero events (version-gate proof); stats match.
2. **Issue #16 acceptance verbatim:** captured EIT loop → `parse_loop` →
   JSON shows `"Journal Météo climat"`, `"fre"`, `"FRA"`.
3. **Completeness:** every `DescriptorTag`/`TableId`/packet-type variant has
   a LUT entry.
4. **Registry:** custom owned descriptor registers, appears as `Other`,
   downcasts, serializes via erased-serde.
5. **Chain test:** T2-MI fixture → pump → `up_iter` → `SiDemux` (Rai BBFrame
   fixture if it contains inner SI; otherwise synthetic).
6. **Hostility smoke:** truncated/garbage feeds at every layer — no panics,
   stats increment.
7. Existing 16 suites + round-trips unchanged except serde-expectation
   fixtures updated for the `DvbText`/`LangCode` output (the contained 2.0
   break).

## Decisions log

| Decision | Choice | Why |
|---|---|---|
| Extensibility | Open registry over static LUT | "discovers from the implementation"; private tags slot in |
| Consumption | Pull event iterator | no callback/borrow fights; channels/async-friendly |
| Data model | Owning-`Bytes` events + lazy borrowed views | `'static` ergonomics with zero-copy parse; alloc only at the edge |
| Hot-path strategy | Version gate in demux | SI repeats endlessly; steady state = map probe + drop |
| Scope | dvb-si + dvb-t2mi now; bbframe unchanged | t2mi requested in scope; `up_iter` already right |
| Semver | 2.0.0 lockstep | honest break for `DvbText` fields + JSON shape; ecosystem is days old |
| Custom-type constraint | registered types are owned `'static` | `dyn Any` requires it; documented |
