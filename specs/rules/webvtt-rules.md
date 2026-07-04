# WebVTT — cue syntax + HLS segmentation rules

Sources: **W3C WebVTT** (`https://www.w3.org/TR/webvtt1/`, free) and **RFC 8216
§3.5** (WebVTT-in-HLS, `X-TIMESTAMP-MAP`). Curated for the 608/708 → WebVTT
writer (#568). The full spec HTML is not vendored (large, freely fetchable);
the rules below are what the writer emits — verify emitted output against a
validator (`webvtt-py` / ffmpeg).

## File structure (W3C §4)

```
WEBVTT⏎           ← required signature (may be followed by a space/tab + text, then ⏎)
⏎                 ← blank line
[cue]             ← zero or more cue blocks, each separated by a blank line
```

- The file is UTF-8. Line terminators: CR, LF, or CRLF (emit LF).
- Optional header blocks (STYLE, REGION, NOTE) may follow the signature before
  cues; the first-pass writer emits none (plain cues only) — document that as a
  known simplification.

## Cue block (W3C §4.1–§4.3)

```
[cue-identifier]⏎          ← OPTIONAL: any non-empty line without "-->"
start --> end [settings]⏎  ← the timings line
payload line(s)⏎           ← cue text; may span multiple lines; no blank line inside
```

### Timestamp (W3C §4.3.1)
`(hh:)?mm:ss.ttt` — hours optional (≥1 needed only when ≥60 min); `mm` and `ss`
are exactly two digits (00–59), `ttt` exactly three digits (milliseconds).
`start` must be ≥ the previous cue's start (cues in non-decreasing start order);
`end` > `start`. The `-->` is surrounded by single spaces.

### Cue settings (W3C §4.3.2, space-separated `name:value`)
`vertical:rl|lr` · `line:<n>|<pct>%[,align]` · `position:<pct>%[,align]` ·
`size:<pct>%` · `align:start|center|end|left|right`. First pass: emit `align`
and `line`/`position` only where cheaply derivable from 608/708 row/column;
otherwise omit (player defaults). Document placement losses honestly.

### Cue payload text (W3C §6.4)
Plain text; the four required escapes: `&amp;` `<` → `&lt;` `>` → `&gt;`
(`&amp;` first). Inline tags (`<b>`, `<i>`, `<u>`, `<c.class>`, `<v speaker>`)
optional; first pass may emit plain text + `<i>`/`<b>`/`<u>` where 608/708
style flags are set.

## HLS segmented WebVTT (RFC 8216 §3.5)

Each WebVTT **segment** is its own file that MUST begin with the `WEBVTT`
signature, and SHOULD carry an `X-TIMESTAMP-MAP` header to tie WebVTT-local time
to the MPEG-2 TS (PES) timeline shared with the audio/video segments:

```
WEBVTT
X-TIMESTAMP-MAP=MPEGTS:<n>,LOCAL:<webvtt-timestamp>
```

- `MPEGTS:<n>` — a 33-bit 90 kHz MPEG-2 TS timestamp (the PES PTS of the point
  mapped by `LOCAL:`), modulo 2^33.
- `LOCAL:<ts>` — the WebVTT timestamp (`(hh:)?mm:ss.ttt`) that corresponds to
  that MPEGTS value; conventionally `00:00:00.000`.
- A cue's absolute presentation time = its WebVTT time − LOCAL + (MPEGTS / 90000).
  Emit one `X-TIMESTAMP-MAP` per segment; cue times inside the segment stay on
  the WebVTT-local clock. Segmentation is aligned to the media segment
  boundaries (same durations as the CMAF/TS segments).

## 608/708 → cue mapping notes (implementation)

- **CEA-608** (CTA-608-E; decode via `cc-data`): CC1 field. pop-on
  (RCL/ENM/EOC caption-frame commits → one cue spanning display→erase),
  roll-up (CR scrolls; each committed row → cue), paint-on. Cue start = PTS of
  the commit command; end = PTS of the next erase/replace.
- **CEA-708** (CTA-708-E): service 1, window/pen basics → text; styling minimal.
- Carriage: `cc_data()` in the picture-user-data (ETSI TS 101 154 Table B.9,
  via `cc-data`) and H.264/HEVC SEI user_data_registered_itu_t_t35 (A/53) —
  both feed the same 608/708 byte-pair stream. Losses (placement, styling,
  non-service-1 708) documented per feature; round-trip is NOT claimed (lossy).
