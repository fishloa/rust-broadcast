---
title: "Clause 8 Content Protection (notes)"
source: ETSI TS 103 285 V1.4.1 (2023-09), private/specs/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf
clauses: 8.1-8.7
pdf_pages: 61-64
printed_pages: 61-64
private_verbatim: private/specs-verified/dash/etsi-ts-103-285/ch08-content-protection.md
verified_against_render: true
notes_original_wording: true
verification_date: 2026-10-24
---

# ETSI TS 103 285 V1.4.1 - Clause 8 Content Protection (notes)

Structured notes in the project's own wording (the standard's text is not reproduced; short quoted phrases only). Strength words are kept: **shall** = mandatory, **should** = recommended, **may** = optional. Verbatim, page-verified text: the private file named in the front matter. Back to [INDEX](INDEX.md).

Related: ISO/IEC 23009-1 ContentProtection rules ([dash-mpd-rules](../../../../specs/rules/dash-mpd-rules.md)); CENC ([cenc-23001-7](../../../../transmux/docs/codec/cenc-23001-7.md)); DASH-IF IOP content protection ([dashif-iop-4.3](../dashif-iop-4.3/)).

## 8.1-8.2 Scope and background (p.61-62)

- The profile defines no complete DRM system. It only fixes how MPD fields and ISO CENC (ISO/IEC 23001-7) are used so several DRMs can share key IDs/IVs and each carries its own opaque payload.
- Protection is optional; if used, clause 8 is mandatory.
- DRM data can sit in `pssh` boxes (init segment `moov`, optionally media segment `moof` for key rotation) and/or in the MPD `ContentProtection` element (UUID form, same SystemID as CENC). MPD carriage lets a player start licence work before fetching the init segment.
- Key rotation must keep random access possible and must deliver new keys/licences early enough for uninterrupted playback.

## 8.3 One key per Adaptation Set (p.62)

Rules (all shall):
1. Every Representation in an AdaptationSet uses the same licence and key, hence the same `default_KID` in each `tenc`.
2. Therefore `ContentProtection` goes on the AdaptationSet.
3. With key rotation, keys/licences are aligned across the set (switching Representation never needs a new licence or key for the same segment).
4. If HD and SD have different licence rights they go in separate AdaptationSets, each with its own descriptors.

## 8.4 ContentProtection descriptors (p.62)

Client: shall process them; may ignore unknown `@schemeIdUri`.
Content: every protected AdaptationSet shall carry
- one `urn:mpeg:dash:mp4protection:2011` descriptor with `@value` = `cenc` or `cbcs`; it should add `@default_KID` (namespace `urn:mpeg:cenc:2013`, equal to the `tenc` value);
- one UUID-URN descriptor per supported DRM (may hold DRM data).
Client exception: if a descriptor identifies both a known DRM and the `cenc` scheme, the player shall not drop the set merely because the mp4protection descriptor is absent.

## 8.5 CENC usage (p.63)

| Item | Rule |
|---|---|
| `pssh` | Player shall handle boxes in init and media segments; may ignore unknown systems. |
| MPD vs `pssh` data | Should be equivalent per SystemID; if they differ, the MPD wins. |
| Key rotation | KID in an AdaptationSet shall not change more often than every 120 s. |
| Random access | Content shall keep SAP type 1/2 usable: any segment decryptable from its own data plus init/MPD, never from a previous segment alone. |

## 8.6 Encryption mode (p.63-64)

- AES-128 CTR or CBC via `cenc` or `cbcs` (23001-7 clause 9).
- `cbcs` AdaptationSets shall not be labelled with the 2014 profile URN `urn:dvb:dash:profile:dvb-dash:2014`.
- A presentation with AVC video shall include at least one AVC video set, and at least one set of each other media type, that is clear or `cenc` (AES-CTR).
- NAL video: subsample encryption; NAL type/length bytes clear; payload of access unit delimiters clear; SEI payload clear when Table 22 says clear.
- Table 22 inventory (SEI message type -> treatment): picture timing, pan-scan rectangle, recovery point, decoded reference picture marking repetition -> clear; filler payload -> clear recommended; ITU-T T.35 user data and unregistered user data -> application decision.
- `saio` offsets shall point inside the `moof` that contains the owning `traf`; a `senc` box may be used; auxiliary info stays in the 23001-7 clause 7 structure referenced by `saio`/`saiz` (8.6.1, p.64).

## 8.7 Clear side content (p.64)

Subtitles and events may stay unencrypted inside an encrypted MPD; `emsg` cannot be encrypted by CENC; players must still meet subtitle/event timing despite decryption delay. Subtitle encryption is undefined.

## Notes on the source (p.63-64)

- 8.6.0 says SEI entries marked "Application specific" need a judgement call, but Table 22 contains no such wording (it says "Application decision ...").
