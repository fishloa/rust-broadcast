The verbatim spec transcriptions are in the private repo (`private/specs-verified/hls/broadcast-hls/iframe-playlists.md` and `multivariant-attribute-rules.md`); this is the public own-words summary of what `broadcast-hls` does. Source: `src/lib.rs`.

# I-frame (trick-play) playlists and multivariant attributes

## Two pieces make up an I-frame rendition

1. In the multivariant playlist, `IFrameVariant` renders and parses `#EXT-X-I-FRAME-STREAM-INF` (RFC 8216 section 4.3.4.2). `MasterPlaylist::iframe_variants` holds them.
2. The referenced media playlist sets `MediaPlaylist::iframes_only`, which emits `#EXT-X-I-FRAMES-ONLY` (RFC 8216 section 4.3.3.6) in the header block and parses back from that tag line.

## IFrameVariant rules the code enforces

- Fields: `bandwidth` (required), optional `average_bandwidth`, `codecs`, `resolution`, the `uri`, and `extra_attrs` for unmodeled attributes.
- Unlike `#EXT-X-STREAM-INF`, the URI is an attribute on the tag line itself, not a following line. The renderer always writes `URI="..."` last, after any extra attributes.
- Render order: `BANDWIDTH`, then `AVERAGE-BANDWIDTH` right after it when present, then `CODECS`, then `RESOLUTION`, then extras in stored order (sorted by name when parsed), then `URI`.
- Parse requires `BANDWIDTH` and `URI`; `BANDWIDTH` and `AVERAGE-BANDWIDTH` must be decimal integers and `RESOLUTION` must be `<w>x<h>`. Anything else is retained in `extra_attrs`, so `REQ-` prefixed attributes survive and count toward the version computation below.
- There is no `FRAME-RATE` field on `IFrameVariant` (only `Variant` has one).
- Rendering rejects, before emitting any output, a URI or `CODECS` value containing a double quote, CR or LF (an injection guard shared with all other tags).

## Version floor for the I-frame media playlist

`MediaPlaylist::computed_version` raises the `#EXT-X-VERSION` floor as follows (the highest triggered row wins):

- `iframes_only`, or any segment with a byte range, requires at least 4.
- `#EXT-X-MAP` (init section) requires at least 6, except when `iframes_only` is also set, where the code uses 5.
- A caller-set `version` is treated as a floor and is only ever raised, never lowered.

I did not find code that adds a version requirement specifically because a multivariant playlist contains `#EXT-X-I-FRAME-STREAM-INF`; the multivariant version computation only reacts to variable substitution and `REQ-` attributes among the I-frame fields (unverified beyond reading those code paths).

## EXT-X-BYTERANGE use

`ByteRange { length, offset: Option<u64> }` is shared by three places that use the same `<n>[@<o>]` notation:

- `MediaSegment::byte_range` renders `#EXT-X-BYTERANGE` right after that segment's `#EXTINF`, before the URI. Without an offset the range follows on from the previous range of the same resource.
- `PartSpec` byte ranges use the same notation as an attribute.
- `MapTag` byte ranges also use it, and there the offset is always present in the model.
- Parsing rejects non-decimal values and any offset plus length that would overflow `u64`.

This is the mechanism that lets an I-frame playlist point each single-frame segment at a slice of one media file. The crate does not itself locate I-frames; the byte values are caller-supplied.

## Multivariant attribute handling that bears on I-frame renditions

- `Variant` carries `average_bandwidth` and `frame_rate`; the doc comments cite Apple's authoring items 9.14 (average bandwidth) and 9.15 (frame rate for variants with video). The crate models and round-trips them but leaves the obligation to produce them to the caller; it does not reject a variant without them.
- `FrameRate` is stored as exact millihertz and rendered as an integer part plus three decimals (for example 29.970), so there is no float rounding. Parse rejects more than three fractional digits and out-of-range values.
- `VideoRange` (SDR, HLG, PQ) is built as an extra attribute, rendered as a bare token.
- Rendition groups (`#EXT-X-MEDIA`) are validated on render: non-empty `GROUP-ID` and `NAME`, no duplicate name within a type and group, and at most one `DEFAULT` per group (`Error::DuplicateNameInGroup`, `Error::DuplicateDefaultInGroup`).
- Known attributes that are quoted strings (`AUDIO`, `VIDEO`, `SUBTITLES`, `STABLE-VARIANT-ID`, `SUPPLEMENTAL-CODECS`, `REQ-VIDEO-LAYOUT`, and so on) are tracked by name so unmodeled ones still render with the correct quoting; `CLOSED-CAPTIONS` is quoted unless it is `NONE`.
- Any attribute name starting `REQ-` on any typed multivariant tag (including `IFrameVariant`) raises the computed protocol version to 12 (RFC 8216bis section 8 row 12). Variable-substitution markers (`{$`) in a variant or I-frame URI raise it to the substitution row.
