# transmux 0.29.0

_Released 2026-11-02._

### Added
- `declared_caption_services(&TrackSpec::es_info_descriptors)` reads the ATSC `caption_service_descriptor` (tag 0x86) from a video track's PMT loop, and `DetectedCaption::declared_language` maps it onto a detected stream: a CEA-708 service by `caption_service_number`, a CEA-608 channel only when every declared 608 entry names the same language (A/65 gives `line21_field` no polarity, so entries cannot be assigned to `CC1` to `CC4` individually). Needs `dvb-si` with `AtscCaptionServiceDescriptor` (issue #1428).

### Changed (BREAKING)
- `CaptionDetector` now reports each CEA-608 channel `CC1` to `CC4` and every CEA-708 service 1 to 63 actually present (issue #1428). A 608 channel is reported once caption text (not control codes, XDS or null padding) has been written to it, routed by field and by the data-channel bit of the control codes; a pair with bad odd parity is ignored. `DetectedCaption::Cea608Field1`/`Cea608Field2` are replaced by `DetectedCaption::Cea608(Cea608Channel)`, and a service is reported when it received a service block with content, no longer only when it defined a window of services 1 to 6.

---

Published from tag `transmux-v0.29.0`.
