# cc-data 0.7.1

_Released 2026-11-02._

### Added
- `Cea608Decoder::has_caption_text(channel)`: whether a displayable character has been written to a CC1-CC4 channel outside text mode (control codes, XDS and null padding never count). `Cea608Channel::number()` (1 to 4) and `PartialOrd`/`Ord`/`Hash` on `Cea608Channel`.
- `Cea708Decoder::services_seen()`: the service numbers 1 to 63 that have received a service block with a non-null byte, including extended service numbers 7 to 63 that the decoder does not interpret (issue #1428).
### Documentation
- Fixed documentation links/citations to resolve to existing paths (docs/tools hygiene).

---

Published from tag `cc-data-v0.7.1`.
