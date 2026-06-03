# Changelog

## 0.1.0 — unreleased

First substantive release covering the common DVB SI / MPEG-2 PSI tables and descriptors.

### Added
- `Section<'a>` — generic PSI/SI section framing with CRC-32 validation
- `TsPacket<'a>` + `SectionReassembler` under feature `ts`
- Tables: `Pat`, `Pmt`, `Sdt`, `Eit` with serialize round-trip tests
- Descriptors: `NetworkNameDescriptor`, `Iso639LanguageDescriptor`,
  `ServiceDescriptor`, `ShortEventDescriptor`, `StreamIdentifierDescriptor`,
  `TeletextDescriptor`, `SubtitlingDescriptor`, `Ac3Descriptor`,
  `EnhancedAc3Descriptor`
- Annex A text decoding subset: ISO 6937, ISO 8859-n, UTF-8, UCS-2 BE,
  emphasis markers, CRLF
- Annex C MPEG-2 CRC-32 table + `crc32()` function
- `TableId` / `DescriptorTag` / `pid::well_known` typed constant modules
- Feature flags: `chrono`, `ts`, `smallvec`, `serde`, `rayon`

### Notes
- PMT and EIT tables parse their outer structure; per-descriptor semantic
  parsing is split across the descriptor modules (tags above) and the
  consumer is expected to walk `descriptors` / `es_info` slices.
