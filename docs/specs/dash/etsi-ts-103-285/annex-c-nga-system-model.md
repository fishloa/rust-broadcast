---
title: "Annex C (informative) NGA system model (notes)"
source: ETSI TS 103 285 V1.4.1 (2023-09), private/specs/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf
clauses: Annex C (C.1-C.3.4)
pdf_pages: 142-145
printed_pages: 142-145
private_verbatim: private/specs-verified/dash/etsi-ts-103-285/annex-c-nga-system-model.md
verified_against_render: true
notes_original_wording: true
verification_date: 2026-10-24
---

# ETSI TS 103 285 V1.4.1 - Annex C (informative) NGA system model (notes)

Structured notes in the project's own wording (the standard's text is not reproduced; short quoted phrases only). Strength words are kept: **shall** = mandatory, **should** = recommended, **may** = optional. Verbatim, page-verified text: the private file named in the front matter. Back to [INDEX](INDEX.md).

Normative player rules: [ch10-8](ch10-8-hdr-hfr-nga-decryption.md) (10.18).

## Model blocks (C.2, pp.142-143)

1. DASH player: generic MPD parser, player-specific Preselection selection, ISO BMFF file-format processing.
2. User interface for picking a Preselection (broadcaster app, device UI, or browser).
3. Decoder/renderer/transcoder (transcoder: AC-4 and DTS-UHD models).
4. MPEG-H Audio UI manager (MPEG-H model only).
5. Outputs: loudspeaker, headphones, HDMI/ARC, S/PDIF.
6. External AVR or sound bar that decodes/renders.
7. Player-decoder interface: vendor-proprietary, except where Media Source Extensions are used (some MSE cases need further work, out of scope).

## Processing (C.3, pp.143-145)

- Common: the MPD parser exposes the Preselections to the UI; the selection (user input, defaults, history) is passed to the decoder as the chosen `Preselection@tag`. The player must apply live selection changes with a delay that does not hurt the experience. Decoded signals go to local outputs or over HDMI/ARC/S-PDIF; a bitstream output goes to an external decoder.
- AC-4 (Figure C.1): bitstream goes from the player to the decoder, which offers extra interactivity from in-stream metadata (object selection, dialogue enhancement) beyond the MPD Preselections; with a bitstream output, a selection forces transcoding (decode, apply, re-encode).
- MPEG-H (Figure C.2): the UI manager receives the bitstream and writes the user's modifications back into it; the decoder may be in the TV (PCM out) or an external device (bitstream out over HDMI/S-PDIF).
- DTS-UHD (Figure C.3): decoder offers up to three outputs (stereo PCM, multichannel PCM, transcoder bitstream), partly so S/PDIF/ARC always carry a signal; the UI also configures renderer/transcoder, loudness/DRC and can override aux streams subject to bitstream permissions; the transcoder follows the renderer and can emit up to 22.2 channels or a DTS 5.1 bitstream (decoder detail: ETSI TS 101 154 clause 6.9).

Figures (described, qualitative): each shows the DASH player block (parser, Preselection processing, file-format processing) feeding a codec-specific decoder block across a dotted "interface" line, a UI block linked to both, and arrows to speakers/headphones/HDMI/S-PDIF and an external AVR; colours distinguish bitstream, PCM and transcoded paths.

## Notes on the source

- Typos: "in the in the", missing full stop in C.3.1. C.1 mentions only AC-4 and MPEG-H although C.3.4 adds DTS-UHD.
