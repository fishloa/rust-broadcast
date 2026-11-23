# broadcast-hls 0.9.0

_Released 2026-11-22._

Minor for a 0.x crate, breaking on `jiff` and additive almost everywhere else. The release turns two tag families that used to round-trip as opaque lines into typed values (`DateRange`/`Cue`/`Scte35Kind` for `EXT-X-DATERANGE`, `CustomTag`/`CustomTagName` for vendor tags), adds the append-only renderers an LL-HLS writer needs, and adds `ParseOptions` so a vendor tag can stay attached to the segment it describes.

## Breaking changes

- `jiff` 0.2 -> 1.0. `ProgramDateTime::from_timestamp` and `ProgramDateTime::timestamp` take and return a jiff 1.x `Timestamp`. Migration: depend on `jiff` 1.x; the values are otherwise unchanged.
- Builds against `broadcast-common` 14 (the `jiff` 1.0 epoch). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2); a consumer must resolve a single `broadcast-common` 14.

## Added

- `DateRange`, `ClientAttrName`, `Cue`, `CueTrigger`, `DateRangeError` and `ClientAttrNameError`: a typed `#EXT-X-DATERANGE` (RFC 8216bis §4.4.5.1) with `new`/`parse_tag_line`/`to_tag_line`, typed getters and validating setters/builders. An untouched parsed tag round-trips byte-for-byte (attribute order, quoting and decimal text included); a setter replaces its entry in place or appends one; `PartialEq` compares decoded values rather than spelling. Every §4.4.5.1 rule is a typed error, including `END-ON-NEXT`'s `CLASS`/`DURATION`/`END-DATE` constraints and the `END-DATE = START-DATE + DURATION` identity (#3925). `Scte35Kind` with `DateRange::set_scte35`/`with_scte35` builds an `SCTE35-CMD`/`-OUT`/`-IN` hex payload through a validated setter.
- `CustomTag`, `CustomTagName`, `AttributeListError` and `CustomTagNameError`: a vendor tag (`EXT-` name plus an attribute list) with `parse_tag_line`/`to_tag_line`; a name must be a non-empty §4.1 tag this crate does not parse as a standard tag.
- `ParseOptions` and `MediaPlaylist::parse_with_options`: a vendor tag named in `ParseOptions::segment_scoped_tags` stays in the following segment's `pre_tags` instead of being hoisted to `extra_tags`, so a per-segment tag re-renders in its original position.
- `MediaPlaylist::render_header`, `render_segment(index)` and `render_endlist`: the playlist split into the three pieces an append-only writer emits, each validating exactly the fields it writes. `OpenTag` and `OpenSegment::trailing_tags` describe the segment-scoped lines an open (LL-HLS live-edge) segment carries after its parts, so a key rotation between two parts applies only to the parts after it.
- `ParseDiagnosticKind::{DanglingSegmentTags, DateRangeWithoutStartDate}`: a segment tag with no URI line before end of input, and a `#EXT-X-DATERANGE` with no `START-DATE` that no earlier tag with the same `ID` precedes, are reported through `MediaPlaylist::diagnostics` instead of being silently dropped or refused.

## Fixed

- `render_endlist` renders an open segment's tag lines in their wire positions: pre-tags that precede the first `#EXT-X-PART` come before its `#EXT-X-DISCONTINUITY`/`#EXT-X-MAP`/`#EXT-X-PROGRAM-DATE-TIME`/parts, while a tag that followed a part stays after it. A segment renders with the same tag order before and after it closes (#3931).
- An `X-` client attribute may be a signed decimal: RFC 8216bis §4.4.5.1 allows a quoted-string, a hexadecimal sequence or a signed-decimal-floating-point, so `X-A=-1.5` parses, round-trips byte-identically and renders. The typed paths also reject C0, DEL and C1 control characters in attribute values, while the general `AttrValue` and whole-playlist parser keep their prior tolerance so existing callers see no regression (#3925).
- `CustomTagName` accepts an `EXT-X-CUE-*` name: no RFC 8216bis tag carries that prefix (the cue tags are SCTE/Adobe vendor tags), so reserving it refused a legal vendor tag (#4001).

## Changed

- `EXT-X-KEY` `IV` parsing decodes through `broadcast_common::hex::hex_decode_into`, replacing a hand-rolled hex loop that carried an `.expect`; accepted and rejected inputs are unchanged (#3957).
- The `daterange`, `custom_tag` and `options` module docs cite the private RFC 8216bis transcription paths alongside the clause numbers (#3997).
- README and crate root corrected: `EXT-X-DATERANGE` is typed (`DateRange`/`Cue`/`Scte35Kind`), and `CustomTag`/`CustomTagName`, `ParseOptions`, the append-only renderers and the dangling-segment-tag diagnostic are documented (#4010).

## Tests

The parser limits are pinned at exactly max and max+1, `parse_iv` at a wrong-length or non-hex `IV`, every §8 version-row trigger in both playlist kinds, the attribute tokenizer's end-of-input boundaries, the LL-HLS builders through the rendered playlist, and `render_header`'s negative append case. `DateRange`/`CustomTag` round-trip byte-exactly over four committed fixtures.

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #3986 (signed `X-` decimal), #3995 (`OpenTag` `#[non_exhaustive]`), #3996 (`START-DATE` diagnostic), #3997 (citations), #4001 (`EXT-X-CUE-*` names), #4010 (README), #4021 (test bullets under `### Tests`).

---

Published from tag `broadcast-hls-v0.9.0`.
