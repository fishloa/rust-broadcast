---
source: DASH-IF Interoperability Points v4.3 (15 Nov 2018), private/specs/dashif_iop_v4.3_guidelines.pdf
clauses: "Annex G Sample Dual-layer MPD"
pdf_pages: "253-254"
printed_pages: "annex pp.25-26"
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/annex-g-sample-dual-layer-mpd.md
verified_against_render: true
notes_original_wording: true
---

# IOP v4.3 Annex G - Sample dual-layer MPD (structure notes)

The annex shows a `Period` fragment (no `MPD` root start tag printed; the end tag is printed) for Dolby Vision dual-stream content. The full example is in the private file; this note gives the skeleton and the facts it demonstrates. "p." = PDF page.

Skeleton:

- `Period`
  - `AdaptationSet` (video; `subsegmentAlignment` true, `subsegmentStartsWithSAP` 1, `frameRate` 24000/1001)
    - `Representation` id `base-layer`: `mimeType` video/mp4, `codecs` an HEVC string `hvc1.2.<constraint>.L150.B0` (printed with the 9-digit second field `100000000`), 3840x2160, bandwidth 14156144; `BaseURL`, `SegmentBase` with `indexRange` and `Initialization range` (single-file, byte-range indexing)
    - `Representation` id `enhancement-layer`: `codecs` `dvhe.dtr`, `dependencyId` = `base-layer`, 1920x1080, bandwidth 3466528, own `BaseURL`/`SegmentBase`
  - `AdaptationSet` (audio; `codecs` `ec-3`, lang `und`): one Representation (192000 bps) with `AudioChannelConfiguration` scheme `tag:dolby.com,2014:dash:audio_channel_configuration:2011`, value `F801`, plus `BaseURL`/`SegmentBase`

What it demonstrates: the EL is a second Representation in the same AdaptationSet linked to the BL purely by `@dependencyId`; BL and EL have independent codec strings (see 10.4.3.3 and annex D); the sample highlights the BL codec string and `dependencyId`. Printed layout notes: the `codecs` value and the `schemeIdUri` value are wrapped across lines in the PDF (pp.253-254), so a literal copy picks up spurious whitespace; the EL codec string here is `dvhe.dtr` while 10.4.3.3 shows `dvhe.dtr.uhd30`.
