# Migrating `dvb-si` 2.x → 3.0.x

> **3.0.1 note.** 3.0.1 effectively replaces 3.0.0 (yanked). On top of the 3.0.0
> changes below it adds two breaks: serde is now **Serialize-only** across the
> whole workspace (all `Deserialize` derives/impls removed — see §5) and the SIT
> **service loop is now typed** (`Vec<SitService>` instead of a raw `&[u8]` —
> see §6). Everything in §§1–4 still applies.

3.0 finishes the `DvbText` story for descriptor loops. In 2.0, individual text
fields became [`text::DvbText`] (decode on demand, serialize as decoded UTF-8).
In 3.0 the **table descriptor loops** get the same treatment: every raw
`&[u8]` descriptor-loop field is now a [`descriptors::DescriptorLoop`] that walks
into typed [`AnyDescriptor`]s on demand and serializes as the typed sequence.

The wire parsing is **byte-identical** — this release changes only the **field
types** and the **JSON those loops serialize to**.

If you only ever read numeric fields and called `parse_loop(loop.raw())` by
hand, the only change you need is `.raw()`. The breaks are concentrated in
(a) descriptor-loop field types, (b) the serde output of those fields, and
(c) three tables that moved from owned to borrowed.

---

## 1. Descriptor-loop fields: `&[u8]` / `Vec<u8>` → `DescriptorLoop<'a>`

Every SI descriptor loop inside a table is now a `DescriptorLoop<'a>` instead of
a raw byte slice. `DescriptorLoop` borrows the same wire bytes but walks them
into typed descriptors only when you ask.

```rust
// 2.0 — hand the raw slice to parse_loop yourself
use dvb_si::descriptors::{parse_loop, AnyDescriptor};
for item in parse_loop(service.descriptors) {        // &[u8]
    if let Ok(AnyDescriptor::Service(sd)) = item { /* … */ }
}

// 3.0 — the field IS the loop; .iter() walks it (parse_loop still works on raw)
use dvb_si::descriptors::AnyDescriptor;
for item in service.descriptors.iter() {             // DescriptorLoop<'a>
    if let Ok(AnyDescriptor::Service(sd)) = item { /* … */ }
}
let raw: &[u8] = service.descriptors.raw();          // the original wire bytes
```

`DescriptorLoop` **derefs to `[u8]`**, so existing `.len()`, `.is_empty()`,
indexing, and `&loop[..]` slicing keep working — they operate on the **raw wire
bytes** (byte counts, not entry counts). To count entries, use `.iter().count()`.

`parse_loop` is unchanged and still public — use it for free byte slices that
aren't a struct field. The whole `DescriptorLoop` walk delegates to it.

### Affected fields

| Module | Field(s) |
|--------|----------|
| `sdt`  | `SdtService.descriptors` |
| `eit`  | `EitEvent.descriptors` |
| `pmt`  | `PmtStream.es_info`, `Pmt.program_info` |
| `nit`  | `NitTransportStream.descriptors`, `Nit.network_descriptors` |
| `bat`  | `BatTransportStream.descriptors`, `Bat.bouquet_descriptors` |
| `ait`  | `AitApplication.descriptors`, `Ait.common_descriptors` |
| `tot`  | `Tot.descriptors` |
| `rct`  | `Rct.descriptors` (only — `link_info_loop` stays raw `&[u8]`) |
| `rnt`  | `Rnt.common_descriptors` (only — `resolution_providers` stays raw) |
| `int`  | `Int.platform_descriptors` (only — `loops` stays raw) |
| `unt`  | `Unt.common_descriptors` (only — `platform_loop` stays raw) |
| `cat`  | `Cat.descriptors` (was `Vec<u8>`) |
| `tsdt` | `Tsdt.descriptors` (was `Vec<u8>`) |
| `sit`  | `Sit.transmission_info_descriptors` (was `Vec<u8>`) |

### What stayed raw (deliberately not migrated)

These are **not** flat SI descriptor loops, so they remain raw byte slices:

- `int.loops` — EN 301 192 target/operational sub-loop pairs;
  `unt.platform_loop` — TS 102 006 DSM-CC `compatibilityDescriptor` group
  records. Both are length-prefixed sub-structures, **not** flat tag/length
  descriptor sequences.
- `rct.link_info_loop` — link_info() entries (their own 12-bit-length framing).
- `rnt.resolution_providers` — resolution-provider records.

(In **3.0.0** `sit.service_loop` was a borrowed `&'a [u8]`. In **3.0.1** it
became the typed `Sit.services: Vec<SitService>` — see §6.)

## 2. Three tables moved from owned to borrowed

`Cat`, `Tsdt`, and `Sit` previously owned their loop bytes (`Vec<u8>`) and had
no lifetime. To align with the zero-copy convention they now **borrow** and gain
a `'a` lifetime parameter.

```rust
// 2.0
let cat: dvb_si::tables::cat::Cat = Cat::parse(&section)?;     // owned, no lifetime

// 3.0
let cat: dvb_si::tables::cat::Cat<'_> = Cat::parse(&section)?; // borrows `section`
```

If you stored a `Cat` / `Tsdt` / `Sit` in a struct, that struct now needs a
lifetime. The section bytes must outlive the table (as with every other borrowed
table in the crate). `Cat::ca_descriptors()` is unchanged and still returns
owned `CatCaEntry` values.

## 3. `Deserialize` dropped on tables/structs that hold a loop

`DescriptorLoop` is **serialize-only** (the typed walk decodes DVB text and
dispatches per tag — there's no lossless way back to the raw bytes from the
serialized form). Every struct that now holds a `DescriptorLoop` therefore
derives `Serialize` only, cascading to its containers.

```rust
// 2.0 — owned tables round-tripped through JSON
let cat: Cat = serde_json::from_str(&json)?;   // no longer compiles

// 3.0 — these types are serialize-only; reconstruct by re-parsing wire bytes
let cat = Cat::parse(&section_bytes)?;          // Parse, not Deserialize
```

Structs that lost `Deserialize`: `Sdt`, `SdtService`, `Eit`, `EitEvent`,
`Pmt`, `PmtStream`, `Nit`, `NitTransportStream`, `Bat`, `BatTransportStream`,
`Ait`, `AitApplication`, `Tot`, `Rct`, `Rnt`, `Int`, `Unt`, `Cat`, `Tsdt`,
`Sit`. (Plain enums like `SdtKind`, `EitKind`, `NitKind` and value structs like
`ApplicationIdentifier`, `CatCaEntry` keep their `Deserialize`.)

## 4. serde JSON shape change

A `DescriptorLoop` serializes as a **JSON array of typed descriptors**, not an
array of raw bytes. Each entry is the camelCase-tagged `AnyDescriptor` (matching
`parse_loop` output); a per-entry parse error becomes `{"parseError": "<msg>"}`
rather than being silently dropped.

```jsonc
// 2.0 — SdtService.descriptors was a raw byte array
{
  "service_id": 1,
  "descriptors": [72, 9, 1, 3, 66, 66, 67, 3, 79, 78, 69]
}

// 3.0 — the loop walks into typed, decoded descriptors
{
  "service_id": 1,
  "descriptors": [
    {
      "service": {
        "service_type": 1,
        "provider_name": "BBC",
        "service_name": "ONE"
      }
    }
  ]
}
```

As in 2.0, the **variant key** is camelCase (`service`, `shortEvent`) while the
inner struct **field names stay snake_case** (`service_name`, `provider_name`) —
only `AnyDescriptor` carries `rename_all = "camelCase"`.

## 5. serde is Serialize-only across the whole workspace (3.0.1)

JSON is a **display/export format only**. As of 3.0.1, **every** `Deserialize`
derive and impl is removed from `dvb-si`, `dvb-t2mi`, `dvb-bbframe`, and
`dvb-common`. Parsing FROM JSON is deliberately unsupported — to reconstruct a
value, re-`parse` the wire bytes. `Serialize` is unchanged: every table,
descriptor, and payload still serializes exactly as before.

```rust
// 3.0.0 — owned/plain tables still round-tripped through JSON
let pat: Pat = serde_json::from_str(&json)?;   // 3.0.1: no longer compiles

// 3.0.1 — serialize for display/export; reconstruct by re-parsing wire bytes
let json = serde_json::to_string(&pat)?;       // Serialize: unchanged
let pat  = Pat::parse(&section_bytes)?;         // Parse, not Deserialize
```

This also removes the manual `Deserialize` impl for `text::LangCode` and the
now-dead `serde(borrow)` / `serde(bound(deserialize = …))` attributes (they only
served the derived `Deserialize`). Types that still derived `Deserialize` in
3.0.0 — plain enums (`SdtKind`, …), value structs (`PatEntry`,
`ParentalRatingDescriptor`, `RealTimeParameters`, …) and the `dvb-t2mi` /
`dvb-bbframe` owned types — are now `Serialize` only.

## 6. SIT service loop is typed (3.0.1, #23)

The SIT per-service loop, raw `&'a [u8]` in 3.0.0, is now a typed
`Vec<SitService>` — mirroring `SdtService` and completing table consistency.

```rust
// 3.0.0 — raw bytes, walk them yourself
let sit = Sit::parse(&section)?;
let loop_bytes: &[u8] = sit.service_loop;

// 3.0.1 — typed entries
let sit = Sit::parse(&section)?;
for svc in &sit.services {                       // Vec<SitService<'a>>
    println!("service {} status {}", svc.service_id, svc.running_status);
    for d in svc.descriptors.iter() { /* typed AnyDescriptor */ }
}
```

```rust
pub struct SitService<'a> {
    pub service_id: u16,
    pub running_status: u8,          // 3 bits
    pub descriptors: DescriptorLoop<'a>,
}
```

The JSON shape changes accordingly: `service_loop` (a raw byte array) is gone;
`services` is now an array of typed objects, each with its own typed
`descriptors` sequence (same shape as `SdtService`).

---

See `CHANGELOG.md` for the complete 3.0.x entries and the
[crate docs](https://docs.rs/dvb-si) for the full API. The 2.0 guide
([MIGRATION-2.0.md](MIGRATION-2.0.md)) is unchanged and still applies for the
1.x → 2.0 jump.

[`text::DvbText`]: https://docs.rs/dvb-si/latest/dvb_si/text/struct.DvbText.html
[`descriptors::DescriptorLoop`]: https://docs.rs/dvb-si/latest/dvb_si/descriptors/struct.DescriptorLoop.html
[`AnyDescriptor`]: https://docs.rs/dvb-si/latest/dvb_si/descriptors/enum.AnyDescriptor.html
