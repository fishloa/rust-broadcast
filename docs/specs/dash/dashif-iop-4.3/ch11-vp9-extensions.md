---
source: DASH-IF Interoperability Points v4.3 (15 Nov 2018), private/specs/dashif_iop_v4.3_guidelines.pdf
clauses: "11 DASH-IF VP9 Extensions (11.1-11.3)"
pdf_pages: "225-228"
printed_pages: "204-207"
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/ch11-vp9-extensions.md
verified_against_render: true
notes_original_wording: true
---

# IOP v4.3 clause 11 - VP9 (notes)

Own-words notes; exact text in the private file. "p." = PDF page.

## 11.2 DASH aspects of VP9 (pp.225-227)

- ISO BMFF binding per the VP codec ISO-BMFF binding spec [87]; clients must support sample entry `vp09` with a `vpcC` box (in-band configuration record).
- Only VP9 profile 0 (4:2:0, 8-bit) and profile 1 (4:2:0, 10 or 12 bit) for consumer delivery (the same clause's later IOPs also reference profile 2 for HDR; see 11.3.3).
- SAPs are key frames (`frame_type` = 0), type 1 only; fragmenting/segmenting only there. Codec string from the VP binding spec "DASH application" section. Encryption signalled as in Common Encryption 3rd edition for ISO BMFF, with subsample encryption as the binding requires.
- Bitstream switching (`@bitstreamSwitching` true): no edit lists for audio/video sync; first presented sample's composition time = first decode time = `tfdt` baseMediaDecodeTime (negative composition offsets in v1 `trun`); `@presentationTimeOffset` aligns timelines at Period start, init-segment edit lists ignored, PTO at a Period start should match a segment's first frame; same picture aspect ratio for all Representations; dynamic resolution is supported by all VP9 decoders but pixel bit depth may not vary inside a set, so profile is constant while level may vary; the `vpcC` of each init segment carries the highest level and resolution; AdaptationSet `@codecs` = maximum level, Representation `@codecs` optional = maximum level of its segments.

## 11.3 VP9 IOPs (pp.227-228)

| IOP | `@profiles` | Scope | Client features |
|---|---|---|---|
| VP9-HD | `http://dashif.org/guidelines/dashif#vp9` | up to 1080p, 8-bit, up to 30 fps, live and on-demand | clause 3 DASH features, simple live (4.9.2), server-based ad insertion (5.6.1), CENC + key rotation with MPD-based parsing (7), all of 11.2, VP9 profile 0 up to level 4.1 |
| VP9-UHD | `http://dashif.org/guidelines/dash-if-uhd#vp9` | up to 2160p, 8-bit, up to 60 fps | all of VP9-HD, VP9 profile 0 up to level 5.1 |
| VP9-HDR | `http://dashif.org/guidelines/dashif#vp9-hdr` (to 1080p) or `http://dashif.org/guidelines/dash-if-uhd#vp9-hdr` (to 4K) | up to 2160p, 10-bit, up to 60 fps | all of VP9-UHD, VP9 profile 2 up to level 5.1, 10-bit pixel depth |
