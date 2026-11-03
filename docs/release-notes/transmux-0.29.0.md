# transmux 0.29.0

_Released 2026-11-03._

Minor release: caption service discovery from the ATSC `caption_service_descriptor` and Smooth Streaming client helpers (breaking: `DetectedCaption` variants).

### Added
- `declared_caption_services(&TrackSpec::es_info_descriptors)` reads the ATSC `caption_service_descriptor` (tag 0x86) from a video track's PMT loop, and `DetectedCaption::declared_language` maps it onto a detected stream: a CEA-708 service by `caption_service_number`, a CEA-608 channel only when every declared 608 entry names the same language (A/65 gives `line21_field` no polarity, so entries cannot be assigned to `CC1` to `CC4` individually). Needs `dvb-si` with `AtscCaptionServiceDescriptor` (issue #1428).
- `smooth_fragment` module (issue #1460): client-side checks a Smooth Streaming (MS-SSTR) puller runs before demuxing. `manifest_declares_protection` finds a manifest `<Protection>` element with `quick-xml`; `fragment_state` classifies a fragment as `FragmentState::Clear`, `Encrypted` (`senc`/`saiz`/`saio` or the PIFF sample-encryption `uuid`, `PIFF_SAMPLE_ENCRYPTION_UUID`, inside `moof`/`traf`) or `Undetermined` (fails closed on a `moof` that cannot be walked); `fragment_track_id` reads `tfhd@track_ID` and `fragment_tfxd` the `tfxd` box (MS-SSTR 2.2.4.4). `multimux`'s Smooth pull and `broadcast-client`'s Smooth source share these walkers.
- `StreamIndex::same_stream`: stream identity by `Type` + `Name` + `Url` template (MS-SSTR 2.2.3), so two audio `StreamIndex` elements are no longer collapsed onto the first.

### Breaking
- `CaptionDetector` now reports each CEA-608 channel `CC1` to `CC4` and every CEA-708 service 1 to 63 actually present (issue #1428). A 608 channel is reported once caption text (not control codes, XDS or null padding) has been written to it, routed by field and by the data-channel bit of the control codes; a pair with bad odd parity is ignored. `DetectedCaption::Cea608Field1`/`Cea608Field2` are replaced by `DetectedCaption::Cea608(Cea608Channel)`, and a service is reported when it received a service block with content, no longer only when it defined a window of services 1 to 6.

## Upgrading

- Replace the two removed variants with the single typed variant:

```rust
// before
match detected {
    DetectedCaption::Cea608Field1 => { /* field 1 */ }
    DetectedCaption::Cea608Field2 => { /* field 2 */ }
    DetectedCaption::Cea708(svc) => { /* ... */ }
}
// after
match detected {
    DetectedCaption::Cea608(channel) => { /* channel is Cea608Channel::Cc1..Cc4 */ }
    DetectedCaption::Cea708(svc) => { /* ... */ }
}
```

`CaptionDetector` now reports each CEA-608 channel `CC1` to `CC4` and every CEA-708 service 1 to 63 actually present, so consumers that assumed one entry per field may see more entries.

---

Published from tag `transmux-v0.29.0`.
