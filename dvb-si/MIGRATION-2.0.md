# Migrating `dvb-si` 1.x → 2.0

2.0 turns `dvb-si` from a section-at-a-time parser into a feed-and-iterate
demux with trait-driven dispatch and decoded text. The wire parsing is
unchanged; what changed is the **types you read out** and the **JSON they
serialize to**. This guide lists every breaking change with before/after code.

If you only ever called `Pat::parse(bytes)` and read numeric fields, you are
unaffected. The breaks are concentrated in (a) text fields, (b) language/country
codes, (c) serde output, and (d) anyone who used the old subset `Descriptor`
enum.

---

## 1. Text fields: `&[u8]` → `DvbText<'a>`

Free-form DVB text fields (service names, event names, descriptions, network
names, …) are now [`text::DvbText`] instead of raw `&[u8]`. `DvbText` borrows
the same wire bytes but decodes EN 300 468 Annex A → UTF-8 on demand.

```rust
// 1.x — caller had to know about Annex A and decode by hand
let name: &[u8] = short_event.event_name;
let decoded = dvb_si::text::decode_dvb_string(name); // manual

// 2.0 — decode on demand; raw bytes still available
let name = short_event.event_name;        // DvbText<'a>
let decoded = name.decode();              // Cow<str>, Annex A handled
let raw: &[u8] = name.raw();              // the original wire bytes
println!("{name}");                       // Display = decoded
```

`DvbText` **derefs to `[u8]`**, so existing `.len()`, indexing, and `&text[..]`
slicing keep working (they operate on the raw wire bytes, as before):

```rust
let n = short_event.event_name.len();     // still compiles — byte length
```

## 2. Language / country codes: `[u8; 3]` → `LangCode`

3-byte ISO 639-2 language codes and ISO 3166 country codes are now
[`text::LangCode`] (a newtype over `[u8; 3]`).

```rust
// 1.x
let lang: [u8; 3] = short_event.language_code;
let s = std::str::from_utf8(&lang).unwrap_or("");

// 2.0
let lang = short_event.language_code;     // LangCode
let s = lang.as_str();                    // Cow<str>, lossy on garbage
let bytes: [u8; 3] = lang.0;              // the raw 3 bytes (tuple field)
let bytes2: &[u8; 3] = &lang;             // Deref also works
```

## 3. `Deserialize` dropped on text-bearing structs

Re-encoding decoded UTF-8 back into a DVB charset is lossy, so structs that hold
a `DvbText` derive **`Serialize` only** — `Deserialize` is gone. (`LangCode`
keeps a `Deserialize` impl; only `DvbText` and the structs that contain it lost
it.)

Affected descriptor structs:

`BouquetNameDescriptor`, `ComponentDescriptor`, `DataBroadcastDescriptor`,
`ExtendedEventDescriptor`, `ExtensionDescriptor`,
`MultilingualBouquetNameDescriptor`, `MultilingualComponentDescriptor`,
`MultilingualNetworkNameDescriptor`, `MultilingualServiceNameDescriptor`,
`NetworkNameDescriptor`, `ServiceDescriptor`, `ShortEventDescriptor`
(and any table/struct that embeds one).

```rust
// 1.x — round-tripping a ServiceDescriptor through JSON
let d: ServiceDescriptor = serde_json::from_str(&json)?; // no longer compiles

// 2.0 — these types are serialize-only; to reconstruct, parse from wire bytes
let d = ServiceDescriptor::parse(wire_bytes)?;           // Parse, not Deserialize
```

If you need a deserialize round-trip, keep the raw wire bytes and re-`parse`,
or build the struct field-by-field with `DvbText::new(&bytes)`.

## 4. Subset `Descriptor` enum removed → `AnyDescriptor` + `parse_loop`

The 1.x `descriptors::Descriptor` enum only covered a handful of context-free
descriptors and forced callers to hand-roll a tag match. 2.0 replaces it with
[`descriptors::AnyDescriptor`] (every tag 0x05–0x7F) and a lazy walker,
[`descriptors::parse_loop`], that handles a whole descriptor loop including
unknown tags and per-entry errors.

```rust
// 1.x — hand-rolled walk over a descriptor loop
let mut pos = 0;
while pos + 2 <= loop_bytes.len() {
    let tag = loop_bytes[pos];
    let len = loop_bytes[pos + 1] as usize;
    let body = &loop_bytes[pos + 2..pos + 2 + len];
    match tag {
        0x4D => { let d = ShortEventDescriptor::parse(&loop_bytes[pos..])?; /* … */ }
        0x55 => { /* … */ }
        _    => { /* unknown: skip */ }
    }
    pos += 2 + len;
}

// 2.0 — one lazy iterator; never panics, surfaces Unknown + per-entry Err
use dvb_si::descriptors::{parse_loop, AnyDescriptor};
for item in parse_loop(loop_bytes) {
    match item? {
        AnyDescriptor::ShortEvent(se) => println!("{}", se.event_name.decode()),
        AnyDescriptor::ParentalRating(_) => { /* … */ }
        AnyDescriptor::Unknown { tag, body } => { /* preserved */ }
        _ => {}
    }
}
```

For private/context-dependent tags, register them with
[`descriptors::DescriptorRegistry`] and use its `parse_loop`.

## 5. serde JSON shape change

Two shape changes follow from the type changes above:

1. **Decoded text.** `DvbText` serializes as its **decoded UTF-8 string**, not a
   byte array. `LangCode` serializes as a 3-char string, not a byte array.
2. **External camelCase tagging** on `AnyTable` / `AnyDescriptor`: a parsed value
   serializes as `{ "<camelCaseVariant>": { … } }`.

```jsonc
// 1.x — short_event_descriptor JSON (bytes, no Annex A applied)
{
  "language_code": [102, 114, 101],
  "event_name":    [69, 109, 105, 115, 115, 105, 111, 110],
  "text":          [ /* … bytes … */ ]
}

// 2.0 — decoded strings, and wrapped in the camelCase variant key via AnyDescriptor
{
  "shortEvent": {
    "language_code": "fre",
    "event_name":    "Emission Spéciale Politique",
    "text":          "…"
  }
}
```

Note the enum **variant key** is camelCase (`shortEvent`) but the inner struct
**field names stay snake_case** (`event_name`, `language_code`) — only the
`AnyTable`/`AnyDescriptor` enums carry `rename_all = "camelCase"`.

## 6. `pid::well_known` constants: `u16` → `Pid`

The reserved-PID constants are now [`pid::Pid`] values, not bare `u16`.

```rust
// 1.x
let pat_pid: u16 = dvb_si::pid::well_known::PAT;       // 0x0000

// 2.0
let pat_pid = dvb_si::pid::well_known::PAT;            // Pid
let raw: u16 = pat_pid.value();                        // 0x0000
let raw2: u16 = u16::from(pat_pid);                    // also works
let p = dvb_si::pid::Pid::from(0x0012u16);             // u16 → Pid
```

`Pid` is `Copy`, `Ord`, `Hash`, and `Display`s as `0xNNNN`.

## 7. New in 2.0 (additive — no action required)

- [`demux::SiDemux`] — PID-filtered, version-gated, PAT-following section pump
  (feature `ts`). Feed 188-byte TS packets, get `SectionEvent`s for changed
  sections only.
- [`tables::AnyTable`] — dispatch any complete section by table_id.
- [`descriptors::DescriptorRegistry`] — register private descriptors at runtime.
- `examples/si_dump.rs` — `cargo run -p dvb-si --example si_dump -- file.ts [--json]`.

---

See `CHANGELOG.md` for the complete 2.0.0 entry and the
[crate docs](https://docs.rs/dvb-si) for the full API.

[`text::DvbText`]: https://docs.rs/dvb-si/latest/dvb_si/text/struct.DvbText.html
[`text::LangCode`]: https://docs.rs/dvb-si/latest/dvb_si/text/struct.LangCode.html
[`descriptors::AnyDescriptor`]: https://docs.rs/dvb-si/latest/dvb_si/descriptors/enum.AnyDescriptor.html
[`descriptors::parse_loop`]: https://docs.rs/dvb-si/latest/dvb_si/descriptors/fn.parse_loop.html
[`descriptors::DescriptorRegistry`]: https://docs.rs/dvb-si/latest/dvb_si/descriptors/struct.DescriptorRegistry.html
[`tables::AnyTable`]: https://docs.rs/dvb-si/latest/dvb_si/tables/enum.AnyTable.html
[`pid::Pid`]: https://docs.rs/dvb-si/latest/dvb_si/pid/struct.Pid.html
[`demux::SiDemux`]: https://docs.rs/dvb-si/latest/dvb_si/demux/struct.SiDemux.html
