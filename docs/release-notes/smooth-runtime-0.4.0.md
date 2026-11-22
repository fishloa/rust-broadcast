# smooth-runtime 0.4.0

_Released 2026-11-22._

Minor for a 0.x crate. No API change of its own: the crate builds against `broadcast-common` 14, `broadcast-smooth` 0.3, `media-plane` 0.14 and `transmux` 0.33, which is the breaking part. The work is the audit-22 test pass over the manifest, codec and quality-selection rules.

## Breaking changes

- Builds against new caret epochs of workspace siblings. A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; consumers must resolve a single copy of each at the jiff 1.0 epoch, and the manifest/fragment types it hands to `media-plane` are those versions' types.

## Tests

- **`codec_family` is pinned by its FourCC groups.** Quality switching is only allowed within one codec family, so a constant answer would let an `avc1` ladder switch to `hvc1` (or `AACL` to `EC-3`) and break the decoder, and the alternate HEVC FourCC would be classified as unknown (audit 22 finding 43, #3011).
- **The 404 predicate, the `ManifestFault` labels and the selector's `Debug` are pinned.** Only 404/410 mean "not (yet) there", every fault renders its own label, and a `QualitySelector` still names its type when formatted (audit 22 finding 46, #3014).

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #4021 (test bullets under `### Tests`).

---

Published from tag `smooth-runtime-v0.4.0`.
