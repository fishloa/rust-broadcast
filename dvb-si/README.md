# dvb_si

ETSI EN 300 468 v1.19.1 DVB Service Information parser and builder.

## Current coverage (0.1.0)

Early-stage crate. Today:

**Framing**
- Section header (§5.1.1) long + short form, CRC-32 validation
- `TsPacket` + `SectionReassembler` (feature `ts`, uses `bytes::BytesMut` pool)

**Tables**
- PAT (ISO/IEC 13818-1 §2.4.4.3)
- PMT (ISO/IEC 13818-1 §2.4.4.8) — outer structure; descriptors stay raw
- SDT actual + other (ETSI §5.2.3) — outer structure; descriptors stay raw
- EIT present/following + schedule actual/other (ETSI §5.2.4); chrono-gated
  `start_time()` decodes MJD+BCD to `DateTime<Utc>`

**Descriptors**
- 0x0A iso_639_language
- 0x40 network_name
- 0x48 service
- 0x4D short_event
- 0x52 stream_identifier
- 0x56 teletext
- 0x59 subtitling
- 0x6A AC-3 (Annex D)
- 0x7A Enhanced AC-3 (Annex D, opaque-body for now)

**Text**
- Annex A subset: ISO 6937 (single-byte + diacritic combine), ISO 8859-n via
  `encoding_rs`, UTF-8 (selector 0x15), UCS-2 BE (selector 0x11). Annex A.2
  control codes: emphasis markers stripped, CR/LF → space.

**CRC**
- Annex C MPEG-2 CRC-32 with compile-time precomputed table

## Not yet

- NIT / BAT / TDT / TOT / RST / DIT / SIT / ST tables
- SAT family
- Descriptors outside the zenith-relevant set (see above)
- Full Annex A emphasis-pair handling

## Principles

- **Spec fidelity.** Every field appears in the parsed struct.
- **No magic numbers.** Every hex literal outside `#[cfg(test)]` references a named constant or enum variant.
- **Zero-copy where possible.** Parsed types borrow from input via `<'a>` lifetimes.
- **Parse and construct.** Every parser has a symmetric serializer; round-trip is tested.

## Usage

```rust
use dvb_common::Parse;
use dvb_si::tables::sdt::Sdt;

let sdt = Sdt::parse(&section_bytes)?;
for service in &sdt.services {
    println!("service_id = {}", service.service_id);
}
```

## Features

Default: `chrono`, `ts`, `smallvec`, `serde`.

```toml
# Tight build
dvb_si = { version = "0.1", default-features = false }

# Bulk tooling
dvb_si = { version = "0.1", features = ["rayon"] }
```

## Authoritative reference

ETSI EN 300 468 v1.19.1 (2025-02) — "Digital Video Broadcasting (DVB);
Specification for Service Information (SI) in DVB systems".

Structured markdown reference: `docs/dvb_si/` (in the zenith repo).

## License

MIT or Apache-2.0 at your option.
