# broadcast-dash

MPEG-DASH (ISO/IEC 23009-1) manifest syntax: the MPD model, its bounded parser, the `SegmentTemplate` substitution grammar and the `xs:duration` reader.

```toml
[dependencies]
broadcast-dash = "0.1"
```

## What it does / what it does not

**Does:** parse an MPD into a typed, read-only model (`Mpd`, `Period`, `AdaptationSet`, `Representation`, the three Segment addressing layers, `UTCTiming`, `ContentProtection` and the descriptor lists), under hostile-input bounds (`limits::Limits`), with the DASH role, continuity, URL-parameter and essential-property scheme vocabulary (`descriptors`), the attribute-inheritance rules a segment description needs (`inherit`), the `$Identifier$`/`%0Nd` template grammar (`template`), and the W3C XML Schema `xs:duration` reader (`duration`) every duration-valued attribute uses.

It is the read side of DASH, kept below the container hub, and it also writes an `Mpd` back to XML via `Mpd::to_xml` (the parser's exact inverse). Its XML reader is `broadcast_common::xml`, shared with the Smooth Streaming manifest reader.

**Does not:** plan, address or fetch segments, track the live edge, read a clock or talk to a server. Those are `dash-runtime`'s (`DashClient`) and `broadcast-client`'s.


## Examples

- `parse_mpd` — parse a committed MPD and print its Periods, AdaptationSets and
  Representations (`id`, bandwidth, codecs, segment addressing kind).
- `build_mpd` — build a small static MPD (one video + one audio Representation
  with `SegmentTemplate`) in code and render it with `Mpd::to_xml`.

Run with `cargo run -p broadcast-dash --example parse_mpd` (and `build_mpd`).
