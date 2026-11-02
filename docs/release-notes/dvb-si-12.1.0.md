# dvb-si 12.1.0

_Released 2026-11-02._

Minor release: `dvb-si` adds the ATSC caption service descriptor (tag 0x86); see below.

### Added
- `descriptors::atsc_caption_service::AtscCaptionServiceDescriptor` (tag 0x86, ATSC A/65:2013 Table 6.26): per-service `language`, typed `AtscCaptionKind` (608 with `line21_field`, 708 with `caption_service_number`), `easy_reader`, `wide_aspect_ratio`, every reserved run preserved. Tag 0x86 is context dependent (the China descriptor reuses it), so it is not auto-dispatched: enable it with `DescriptorRegistry::with_atsc_caption_service()`. `AnyDescriptor::AtscCaptionService` variant. Verified against the descriptor in `tsduck/input/test-040.ts`.

---

Published from tag `dvb-si-v12.1.0`.
