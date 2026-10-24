---
source: DASH-IF Interoperability Points v4.3 (15 Nov 2018), private/specs/dashif_iop_v4.3_guidelines.pdf
clauses: "9 Multi-Channel Audio Extensions (9.1-9.4)"
pdf_pages: "200-209"
printed_pages: "179-188"
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/ch09-multichannel-audio.md
verified_against_render: true
notes_original_wording: true
---

# IOP v4.3 clause 9 - Multi-channel audio extensions (notes)

Own-words notes; exact text in the private file. Clients may choose which codecs to support (9.1). "p." = PDF page.

## 9.2 Codec signalling (pp.200-205)

| Codec | `@codecs` | Encapsulation reference | SAP | Other rules | Cite |
|---|---|---|---|---|---|
| Enhanced AC-3 (Dolby Digital Plus) | `ec-3` | ETSI TS 102 366 Annex F | 1 | for E-AC-3 and AC-4 `AudioChannelConfiguration` uses scheme `tag:dolby.com,2014:dash:audio_channel_configuration:2011` | 9.2.1, Table 25 |
| Dolby TrueHD | `mlpa` | Dolby spec [36] | 1 | - | Table 25 |
| AC-4 | `ac-4` | ETSI TS 103 190-1 Annex E | 1 | - | Table 25 |
| DTS Digital Surround | `dtsc` | DTS 9302J81100 | 1 | all DTS formats SAP 1; seamless switching/MPD signalling in DTS 9302K62400 | 9.2.2, Table 26 |
| DTS-HD High Resolution / Master Audio | `dtsh` | same | 1 | - | Table 26 |
| DTS Express | `dtse` | same | 1 | - | Table 26 |
| DTS-HD Lossless (no core) | `dtsl` | same | 1 | - | Table 26 |
| MPEG Surround | `mp4a.40.30` (= AOT 2 + 5 + 30) | ISO/IEC 14496-14 | 1 | down-mix coded as HE-AACv2; level 4 of Baseline MPEG Surround profile; segments start with SAP 1 | 9.2.3, Table 27 |
| HE-AAC multichannel | `mp4a.40.2`, `mp4a.40.5`, `mp4a.40.29` | 14496-14 | 1 | level 4 = 5.1, level 6 = 7.1; profile level not expressible in RFC 6381 so channel layout goes in `ChannelConfiguration`; explicit backward-compatible SBR signalling; loudness/DRC info should be in the stream and decoders must handle `dynamic_range_info()` and `MPEG4_ancillary_data()` | 9.2.4, Table 28 |
| MPEG-H 3D Audio LC | `mhm1.0x0B` (L1), `mhm1.0x0C` (L2), `mhm1.0x0D` (L3) | ISO/IEC 23008-3 | 1 | one sample = one `mpegh3daFrame()`; `mpegh3daProfileLevelIndication` = 0x0B/0x0C/0x0D matching the level; `referenceChannelLayout` equals `ChannelConfiguration`; SAP-1 samples are immediate-playout frames with `AudioPreRoll()` holding a valid config and preferably one pre-roll frame; `mpegh3daConfig()` may differ per Representation | 9.2.5, Table 29 |
| MPEG-D USAC (xHE-AAC) | `mp4a.40.42` | ISO/IEC 23000-19 (CMAF) | 1 | CMAF profile `cxha` (up to 5.1); switching sets follow CMAF constraints; `profiles` MIME sub-parameter of `@mimeType` should include `cxha`; if a channel configuration is in the movie header the same must appear in `AudioChannelConfiguration` (values from CMAF Table K.2); each CMAF fragment starts with SAP 1 | 9.2.6, Table 30 |

DTS table: encapsulation and SAP cells span all four rows. Table 28/30 are laid out in narrow columns in the PDF but hold exactly the facts above.

## 9.3 Client guidelines (p.205)

A multichannel-capable client should play the multichannel signal correctly for its output (decode and output PCM, pass through to an AVR, downmix for headphones, or pick an alternate stereo AdaptationSet) and should switch bitrates seamlessly (no audible gap, no A/V sync loss). No expectation of seamless switching between formats.

## 9.4 Extensions as @profiles identifiers (pp.205-209)

A multichannel audio client supports the clause-3 DASH features, CENC with key rotation and MPD/movie-box parsing of DRM data, and the 9.3 guidelines (9.4.1). Content authors are encouraged to use the HTTP-URL construction of [6] 5.1.4. Each extension below is signalled with an `@profile` attribute value; content may claim it if it is multichannel audio and a client supporting 9.4.1 plus the codec and its DASH-specific rules (clause named) can play it.

| Extension | Identifier | Codec rules from |
|---|---|---|
| Enhanced AC-3 | `http://dashif.org/guidelines/dashif#ec-3` | 9.2.1.2 |
| Dolby TrueHD | `http://dashif.org/guidelines/dashif#mlpa` | 9.2.1.2 |
| AC-4 | `http://dashif.org/guidelines/dashif#ac-4` | 9.2.1.2 |
| DTS Digital Surround | `http://dashif.org/guidelines/dashif#dtsc` | 9.2.2.2 |
| DTS-HD HR / MA | `http://dashif.org/guidelines/dashif#dtsh` | 9.2.2.2 |
| DTS Express | `http://dashif.org/guidelines/dashif#dtse` | 9.2.2.2 |
| DTS-HD Lossless (no core) | `http://dashif.org/guidelines/dashif#dtsl` | 9.2.2.2 |
| MPEG Surround | `http://dashif.org/guidelines/dashif#mps` | 9.2.3.2 |
| HE-AACv2 level 4 | `http://dashif.org/guidelines/dashif#heaac-mc51` | 9.2.4.2 |
| HE-AACv2 level 6 | `http://dashif.org/guidelines/dashif#heaac-mc71` | 9.2.4.2 |
| MPEG-H 3D Audio | `http://dashif.org/guidelines/dashif#mpegh-3da` | 9.2.5 |
| MPEG-D USAC | `http://dashif.org/guidelines/dashif#cxha` | 9.2.6.2 |

Supporters named (not normative): Dolby, DTS, Fraunhofer, BuyDRM, Sony for the first groups; Fraunhofer IIS, Dolby, castLabs, Bitmovin, Sony for USAC. Printed quirk: 9.4.6.2 is missing from the numbering (9.4.6.1 jumps to 9.4.6.3); the MPEG Surround overview points to "section 5.3.3" for the HE-AACv2 down-mix coding, which in this document is 6.3.2.
