# ssai-runtime 0.3.0

_Released 2026-10-17._

**Epoch-forced major-class release; no API or behaviour change of its own.** ssai-runtime rebuilds against `broadcast-common` 10 and `broadcast-hls` 0.4 (its dev-dependencies `scte35-splice` 4 and `mp4-emsg` 0.5 move too), and the workspace epoch-purity rule makes that a `0.2` -> `0.3` bump. `broadcast-hls` types are part of this crate's public surface (the interstitial playlist session is built from a `broadcast_hls::MediaPlaylist` and `Error::HlsAttrValue` wraps `broadcast_hls::Error`), so callers must use the matching `broadcast-hls` 0.4. The README and crate-root docs are rewritten (what it does and does not do, standards table for the HLS Interstitials draft and SCTE 35, feature table, compiling quick start).

## Why this version

- **Proposed version:** 0.2.1 -> **0.3.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** none (documentation only).
- **cargo-semver-checks vs live 0.2.1 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** no change; epoch cascade from broadcast-common. Siblings it depends on that move epoch in this wave: broadcast-common, broadcast-hls, mp4-emsg, scte35-splice.
- **Caret epoch:** 0.2 -> 0.3; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-broadcast-hls = "0.3"
+broadcast-hls = "0.4"
-mp4-emsg = "0.4" # dev-dependency
+mp4-emsg = "0.5" # dev-dependency
-scte35-splice = "3.0" # dev-dependency
+scte35-splice = "4.0" # dev-dependency
```

## Breaking changes and migration

No source change is needed for this crate's own API. Your dependency graph must resolve a single copy of each moved sibling:

```toml
broadcast-common = "10.0"
broadcast-hls    = "0.4"   # MediaPlaylist is passed into the playlist session; Error::HlsAttrValue wraps broadcast_hls::Error
```

`Serialize::to_bytes` was removed in `broadcast-common` 10 (it could panic); use `try_to_bytes()?` if you serialize through the shared trait.

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README and crate-root docs rewritten to the cluster standard: what it does / does not, standards table (HLS Interstitials draft, SCTE 35), feature table, compiling quick start.

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `broadcast-hls` 0.3 -> 0.4, `mp4-emsg` 0.4 -> 0.5, `scte35-splice` 3 -> 4). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

## Merged changes since `ssai-runtime-v0.2.1`

- docs: cluster C crate READMEs + crate-root docs (standards tables, feature flags, scope, verified editions) (#1284)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- clock33: shared PCR constants + modular distance; migrate 5 crates; fix ts-fix post-wrap observations (#1206) [stacked on #1196] (#1207)
- Audit sweep of the nine small crates: fit_mask serializers, hostility + guard tests (#1202) [stacked on #1199] (#1203)
