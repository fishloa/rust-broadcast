# ssai-runtime 0.2.0

_Released 2026-10-05._

Breaking (0.x minor) release of the sans-IO SCTE-35 SSAI session core. Two things drive it. Per-session HLS Interstitial rendering now validates every attribute value, so the render functions return `Result` (an ad server could previously inject arbitrary playlist tag lines through a `"`, CR or LF in an asset URI). And splice-point conditioning gained a wrapping-clock variant, because the linear version dropped any break whose cue and nearest boundary straddled the 33-bit PTS wrap (about every 26.5 hours on a 24/7 channel). **Breaking for** anyone calling `InterstitialDateRange::to_tag_line` or `render_session_playlist`, or matching on `Error` exhaustively in a way that ignores new variants. **Security-relevant for** any deployment where `AdDecisionProvider` output comes from a third-party ad server: upgrade.

Read together with: [broadcast-hls-0.3.0.md](broadcast-hls-0.3.0.md) (this crate now builds against it and exposes its error type), [playout-runtime-0.2.0.md](playout-runtime-0.2.0.md) (its consumer for conditioning).

## Breaking changes

### `to_tag_line` and `render_session_playlist` are fallible (#1140)

`InterstitialDateRange::to_tag_line` now returns `Result<String>` (was `String`) and `render_session_playlist` returns `Result<MediaPlaylist>` (was `MediaPlaylist`).

```rust
// 0.1
let line: String = range.to_tag_line();
let playlist: MediaPlaylist = render_session_playlist(&base, Some(&range));
// 0.2
let line: String = range.to_tag_line()?;
let playlist: MediaPlaylist = render_session_playlist(&base, Some(&range))?;
```

What changed underneath:

- Every attribute value (`ID`, `START-DATE`, `X-ASSET-URI`/`X-ASSET-LIST` from an `AdDecisionProvider`, `X-SNAP`, `X-RESTRICT`) goes through `broadcast_hls::AttrValue`'s checked constructors and the shared `broadcast_hls::render_attribute_list`. A `"`, CR or LF in an ad-decision value is now `Err(Error::HlsAttrValue(..))` from the rendering entry point. Before, it terminated the attribute list and injected arbitrary tag lines into every viewer's session playlist.
- `DURATION`, `X-RESUME-OFFSET` and `X-PLAYOUT-LIMIT` are rejected with `Error::InvalidDuration { what, value }` if NaN, infinite or negative, on parse and on render (before, `NaN as i64 == 0` let a NaN duration render as the bare token `NaN`).
- The module's own quoted-comma attribute splitter is replaced by `broadcast_hls::parse_attribute_list`, the tokenizer `timed-metadata` now also uses (the same algorithm existed three times in the workspace, audit r14-SSAI-O1).

### `render_session_playlist` and `render_into` require `EXT-X-PROGRAM-DATE-TIME`

When a break is requested (`active` is `Some`) and the base playlist has no `#EXT-X-PROGRAM-DATE-TIME`, both `render_session_playlist` and `SessionPlaylistBase::render_into` return `Error::MissingProgramDateTime`. RFC 8216bis §4.4.5.1 requires one alongside any `EXT-X-DATERANGE`; the old code emitted the tag unconditionally (#1125). With `active = None` nothing changes. If your base playlists lack the tag, your breaks will now fail instead of rendering an invalid playlist: add the tag upstream.

### New `Error` variants

`Error` is `#[non_exhaustive]`, so these are not source-breaking for a wildcard match: `PtsOutOfRange { pts, modulus }`, `MissingProgramDateTime`, `HlsAttrValue(broadcast_hls::Error)`, `InvalidDuration { what, value }`, `TagParse(String)`.

**`HlsAttrValue` puts `broadcast_hls::Error` in this crate's public API**, so a `broadcast-hls` caret-epoch move is a major-class change here too. This release builds against `broadcast-hls` 0.3 (was 0.2).

## Added

- `splice::condition_splice_point_wrapping(requested_pts, candidates, max_delta_ticks, modulus)` and `splice::PTS_MODULUS_33` (`1 << 33`). Distances and snap direction are measured on the circle of `modulus` ticks, so a cue at `2^33 - 100` snaps to a boundary at `50` as 150 ticks `After` instead of being dropped as `NoAlignedBoundary` (#1125).

  ```rust
  use ssai_runtime::splice::{condition_splice_point_wrapping, PTS_MODULUS_33};
  let p = condition_splice_point_wrapping((1u64 << 33) - 100, &[50], 300, PTS_MODULUS_33)?;
  // p.delta_ticks == 150, p.direction == SnapDirection::After
  ```

  Inputs must already be below the modulus; a zero modulus or an out-of-range PTS or candidate is `Error::PtsOutOfRange`, never silently reduced. An exact half-circle tie resolves forward (`After`).
- Two equidistant candidates resolve to the one `After` the cue, regardless of slice order. This also applies to the existing linear `condition_splice_point`, so its result for such ties can change if the old slice order happened to put the earlier candidate first.
- `playlist::SessionPlaylistBase`: renders the base playlist once and splices each viewer's tag line into the text, with no per-viewer deep clone of every segment (pinned by a counting-allocator test). The line goes before the first `#EXT-X-PART:` or `#EXTINF:`, so for an LL-HLS base it lands before the first segment's parts, never between a segment's parts and its `EXTINF`. `render_session_playlist` still deep-clones per call; use `SessionPlaylistBase` on a per-viewer hot path (#1125).

## Dependency changes

From `git diff ssai-runtime-v0.1.0..HEAD -- ssai-runtime/Cargo.toml`: `broadcast-common` 9.3 to 9.4, `broadcast-hls` 0.2 to 0.3, and `scte35-splice` 2.1 to 3.0 (the last is used by tests and examples only, not by the library's public API).

---

Published from tag `ssai-runtime-v0.2.0`.
