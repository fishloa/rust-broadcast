# Migrating `dvb-si` 1.x / 2.x → 3.1

3.1 finishes the `DvbText` story for descriptor loops. In 2.0, individual text
fields became [`text::DvbText`] (decode on demand, serialize as decoded UTF-8).
In 3.1 the **table descriptor loops** get the same treatment: every raw
`&[u8]` descriptor-loop field is now a [`descriptors::DescriptorLoop`] that walks
into typed [`AnyDescriptor`]s on demand and serializes as the typed sequence. On
top of that, serde is now **Serialize-only** across the whole workspace (§3), the
SIT **service loop is typed** (§5), and an optional `yoke` feature lets a parsed
view outlive its input buffer (§6, additive — no break).

The wire parsing is **byte-identical** — this release changes only the **field
types** and the **JSON those loops serialize to**.

If you only ever read numeric fields and called `parse_loop(loop.raw())` by
hand, the only change you need is `.raw()`. The breaks are concentrated in
(a) descriptor-loop field types, (b) the serde output of those fields, (c) three
tables that moved from owned to borrowed, (d) the workspace-wide removal of
`Deserialize`, and (e) the typed SIT service loop.

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

// 3.1 — the field IS the loop; .iter() walks it (parse_loop still works on raw)
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

The SIT per-service loop is the one exception that became **fully typed** rather
than a `DescriptorLoop` — see §5.

## 2. Three tables moved from owned to borrowed

`Cat`, `Tsdt`, and `Sit` previously owned their loop bytes (`Vec<u8>`) and had
no lifetime. To align with the zero-copy convention they now **borrow** and gain
a `'a` lifetime parameter.

```rust
// 2.0
let cat: dvb_si::tables::cat::Cat = Cat::parse(&section)?;     // owned, no lifetime

// 3.1
let cat: dvb_si::tables::cat::Cat<'_> = Cat::parse(&section)?; // borrows `section`
```

If you stored a `Cat` / `Tsdt` / `Sit` in a struct, that struct now needs a
lifetime. The section bytes must outlive the table (as with every other borrowed
table in the crate). `Cat::ca_descriptors()` is unchanged and still returns
owned `CatCaEntry` values.

> Need to keep a parsed table around **without** threading a lifetime through your
> own structs? That is exactly what the new `yoke` feature is for — see §6.

## 3. `Deserialize` dropped — serde is Serialize-only

JSON is a **display/export format only**. Across `dvb-si`, `dvb-t2mi`,
`dvb-bbframe`, and `broadcast-common`, **every** `Deserialize` derive and impl is
removed. Parsing FROM JSON is deliberately unsupported — to reconstruct a value,
re-`parse` the wire bytes. `Serialize` is unchanged: every table, descriptor, and
payload still serializes exactly as before.

```rust
// 2.x — owned/plain tables round-tripped through JSON
let pat: Pat = serde_json::from_str(&json)?;   // 3.1: no longer compiles

// 3.1 — serialize for display/export; reconstruct by re-parsing wire bytes
let json = serde_json::to_string(&pat)?;       // Serialize: unchanged
let pat  = Pat::parse(&section_bytes)?;         // Parse, not Deserialize
```

Two threads drive this:

- **`DescriptorLoop` is inherently serialize-only.** The typed walk decodes DVB
  text and dispatches per tag — there's no lossless way back to the raw bytes
  from the serialized form. Every struct that holds a `DescriptorLoop` therefore
  derives `Serialize` only, cascading to its containers: `Sdt`, `SdtService`,
  `Eit`, `EitEvent`, `Pmt`, `PmtStream`, `Nit`, `NitTransportStream`, `Bat`,
  `BatTransportStream`, `Ait`, `AitApplication`, `Tot`, `Rct`, `Rnt`, `Int`,
  `Unt`, `Cat`, `Tsdt`, `Sit`.
- **The workspace follows suit.** The remaining types that *could* still have
  derived `Deserialize` — plain enums (`SdtKind`, `EitKind`, `NitKind`, …), value
  structs (`PatEntry`, `ParentalRatingDescriptor`, `RealTimeParameters`,
  `ApplicationIdentifier`, `CatCaEntry`, …), and the `dvb-t2mi` / `dvb-bbframe`
  owned types — are now `Serialize` only too, so the whole family is consistent.

This also removes the manual `Deserialize` impl for `text::LangCode` and the
now-dead `serde(borrow)` / `serde(bound(deserialize = …))` attributes (they only
served the derived `Deserialize`).

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

// 3.1 — the loop walks into typed, decoded descriptors
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

## 5. SIT service loop is typed

The SIT per-service loop, raw `&'a [u8]` in earlier drafts, is now a typed
`Vec<SitService>` — mirroring `SdtService` and completing table consistency.

```rust
// 2.x — raw bytes, walk them yourself
let sit = Sit::parse(&section)?;
let loop_bytes: &[u8] = sit.service_loop;

// 3.1 — typed entries
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

## 6. Owning a parsed view: the `yoke` feature (additive — not a break)

Every table and descriptor view in this crate borrows its section bytes (`<'a>`).
That keeps parsing zero-copy, but it means you can't drop the input buffer while
still holding the parsed view, store one in a `'static` cache, or send it across
a thread without threading the lifetime through your own types.

The new **`yoke`** feature (off by default) solves this. It derives
[`yoke::Yokeable`] on every public zero-copy view type and adds an `owned` module
with `Owned<Y>` — a `'static`, `Send + Sync`, cheaply-`Clone` bundle of the
backing `Arc<[u8]>` and the parsed view. You get an owned, self-contained handle
**without** re-parsing or hand-writing a mirror type.

```toml
# Cargo.toml
dvb-si = { version = "3.1", features = ["yoke"] }
```

```rust
use std::sync::Arc;
use broadcast_common::Parse;
use dvb_si::owned::Owned;
use dvb_si::tables::pmt::Pmt;

// Move the bytes into an Arc cart; parse once; keep the result forever.
let cart: Arc<[u8]> = Arc::from(section_bytes);          // source bytes consumed
let owned: Owned<Pmt<'static>> = Owned::try_new(cart, |b| Pmt::parse(b))?;

// `owned` is 'static + Send + Sync + Clone — store it, cache it, send it.
let program = owned.get().program_number;
let handle = std::thread::spawn(move || owned.get().streams.len());
```

This is purely **additive**: nothing changes for code that doesn't enable the
feature, and it pulls in no dependencies on default builds. The sibling crates
`dvb-t2mi` and `dvb-bbframe` ship the same `yoke` feature on their own view types.

---

See `CHANGELOG.md` for the complete 3.1.0 entry and the
[crate docs](https://docs.rs/dvb-si) for the full API. The 2.0 guide
([MIGRATION-2.0.md](MIGRATION-2.0.md)) is unchanged and still applies for the
1.x → 2.0 jump.

[`text::DvbText`]: https://docs.rs/dvb-si/latest/dvb_si/text/struct.DvbText.html
[`descriptors::DescriptorLoop`]: https://docs.rs/dvb-si/latest/dvb_si/descriptors/struct.DescriptorLoop.html
[`AnyDescriptor`]: https://docs.rs/dvb-si/latest/dvb_si/descriptors/enum.AnyDescriptor.html
[`yoke::Yokeable`]: https://docs.rs/yoke/latest/yoke/trait.Yokeable.html
