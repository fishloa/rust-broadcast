---
source: DASH-IF Interoperability Points v4.3 (15 Nov 2018), private/specs/dashif_iop_v4.3_guidelines.pdf
clauses: "7 Content Protection and Security (7.1-7.8)"
pdf_pages: "179-197"
printed_pages: "158-176"
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/ch07-content-protection.md
verified_against_render: true
notes_original_wording: true
---

# IOP v4.3 clause 7 - Content protection (client-relevant notes)

Own-words notes; verbatim text, figures and examples in the private file. "p." = PDF page. Common Encryption itself is ISO/IEC 23001-7; MPD descriptor syntax is 23009-1 5.8.4.1/5.8.5.2 (not restated).

## 7.1-7.3 Framework (pp.179-181)

- DASH-IF defines no end-to-end DRM; it defines where protection data lives (MPD `ContentProtection`, `pssh`, `tenc`) so several DRMs can protect the same CENC-encrypted content, each identified by a SystemID.
- HTTPS (7.2): recommended; if the MPD is fetched over HTTPS media should be too (mixed-content rules); licence URLs inside `ContentProtection` can use HTTPS; HTTPS defeats intercepting caches and does not replace DRM (no access control, no protection of buffered content).
- Components (7.3): MPD `ContentProtection` descriptors (scheme URN); `tenc` (default_KID, in the init segment; KIDs in movie-fragment sample group descriptions override it, and their keys must be available when samples are, possibly via `pssh` in `moof`); `senc` with IVs/subsample ranges in each `traf`, located through `saio`/`saiz`; `pssh` (licence data per DRM, may be in init or media segments or in MPD `cenc:pssh`). MPD-carried `pssh` allows faster parsing, earlier access, duplicate-licence detection and adding DRMs without touching content; `pssh` in init segments is discouraged because every init-segment processing in a browser re-triggers a licence request (HTML5/EME players may fail). Key rotation uses `sbgp` + `sgpd` of type `seig`; keys for a Segment's KIDs may sit in `moof` `pssh` (a v1 `pssh` lists KIDs so duplicates can be dropped); media-segment keys must be in one DRM format shared by all users; user-specific data goes out of band (root licence).

## 7.4 Box hierarchy (pp.181-184)

Boxes involved: `moov/pssh` (0..1 per SystemID); `.../stsd/sinf/schm` and `.../sinf/schi/tenc` (one each if encrypted); `moof/traf/saiz|saio|senc` (one each if encrypted); for rotation `moof/traf/sbgp`, `moof/traf/sgpd` (`seig`; one per sample group) and `moof/pssh`. Table 24 maps each box to its definition: `moof` [14496-12 8.32], `moov` [8.1], `pssh` [23001-7 8.1.1; v1 lists KIDs], `saio` [8.7.9], `saiz` [8.7.8], `senc` [23001-7 7.1], `schi` [8.12.6 + 23001-7 4], `schm` [8.12.5 + 4], `seig` [23001-7 6], `sbgp` [+5], `sgpd` [8.9.3 + 5], `sinf` [8.12.1 + 4], `stsd` [8.16], `tenc` [23001-7 8.2.1]. Figures 21-22 are nesting diagrams of these boxes (moov-side for single key, moof-side for rotation). Fig. 21 labels the Scheme Information Box "schm" while text and Table 24 call it `schi` (p.182).

## 7.5 Periodic re-authorization (pp.184-186)

- Purpose: service/entitlement changes at programme boundaries and active-stream counting, not extra cryptographic strength. Requirements include uninterrupted playback, avoiding a client storm, quick recovery, player visibility of the rotation, regional blackout and hybrid broadcast/unicast operation without changing MPD validity.
- Options: explicit signalling (e.g. custom messages) is possible but outside the recommendation; initial `pssh` data should be in the MPD to avoid a first-key storm. (1) Period boundary as minimum key interval - simple but no early warning and periods are decided by content, not DRM. (2) Future keys inside `pssh` (proprietary format; the client picks a random time to fetch licences). Fig. 23: segments 100-102 carry PSSH_10 listing kid_9, kid_10, kid_11; segments 103-105 carry PSSH_11 listing kid_10, kid_11, kid_12. (3) Key hierarchy: EML (entitlement management licence, once per scope, bound to a DRM domain/user) and ECL (carries the media key); with a hierarchy the `default_KID` in `ContentProtection`/`tenc` is the key requested by the DRM client (the EML key).

## 7.6 MPD signalling (pp.186-189)

At least one `ContentProtection` descriptor must be on every AdaptationSet with encrypted content.

| Scheme | `@schemeIdUri` | `@value` | Client reading |
|---|---|---|---|
| mp4protection | `urn:mpeg:dash:mp4protection:2011` | `cenc` | Shows encryption is CENC without DRM knowledge; carries `@cenc:default_KID` (UUID string form), enough to identify/acquire a licence or reuse a stored one. Compare KIDs across AdaptationSets and stored licences to know which unique licences are needed. On starting an AdaptationSet the client should ask the DRM system whether the key for that KID is available and must not assume it is. |
| UUID scheme | `urn:uuid:<SystemID>` (IDs listed at http://www.dashif.org/identifiers/protection) | free text, DRM name + version | Marks a DRM available for licence acquisition; may hold `cenc:pssh` (base64 `pssh` box including header). |
| `cenc:pssh` | extension element in namespace `cenc:` | - | Lets clients spread licence requests before live init segments exist; with `cenc:default_KID` on each set the client can tell whether it lacks the key (no purchase), already has it, or which licences to fetch before `availabilityStartTime`. |
| Clear Key | UUID `e2719d58-a985-b3c9-781a-b030af78d30e` | `ClearKey1.0` | Optional child `Laurl` (namespace `http://dashif.org/guidelines/clearKey`, attribute `Lic_type`, value `EME-1.0` = licence format of the W3C Clear Key spec 9.1.4). Do NOT treat W3C Common PSSH SystemID `1077efec-c0b2-4d02-ace3-3c1e52e2fb4b` as a Clear Key indication. The MPD never holds Clear Key init data; the client builds it at run time from the default KIDs of mp4protection descriptors. Use a secure connection; GET with JSON licence request in the body (may add tokens/device/user ID); Clear Key must never manage a key also used by a DRM. |

The printed Clear Key example uses a placeholder `xxxxxxxx-...` UUID and an unquoted `xmlns:ck` value, unlike the fixed UUID in the text (p.188).

## 7.7 Additional constraints (pp.189-194)

| Subject | Rule |
|---|---|
| `tenc` | Same `default_KID` in all Representations of an AdaptationSet; different sets may differ. Same licence per protection system for all Representations of a set (also for root licences under key rotation); if any Representation is encrypted all must be, with the same default_KID. |
| `pssh` boxes | Should not be in init segments; if present, each init segment of a set holds an equivalent `pssh` per SystemID (a switch needs no new licence). W3C common `pssh`, if used, lists only the default_KID. With `pssh` in init segments, clients must parse and de-duplicate licence requests; a `cenc:pssh` is parsed at most once per AdaptationSet and a new request is implied only by a new `cenc:default_KID`. |
| MPD | `ContentProtection` always on the AdaptationSet, applies to all Representations. mp4protection descriptor with `value=cenc` is required for encrypted content and must carry `@cenc:default_KID`, which must match `tenc` (the `tenc` box is the source of truth). `cenc:pssh` should be present for each UUID-scheme descriptor (base64 of a full `pssh` box, enough for licence acquisition); `@value` should hold DRM name/version. A player in a browser may pass this data to the CDM via EME before downloading segments. |
| Precedence / scheduling | If `pssh` is in both MPD and init segment, the MPD one wins. Request licences on first MPD processing when information is available (avoids a storm at `availabilityStartTime`); prefetch licences for a new Period ahead of its start. |
| Periodic re-auth | No key rotation inside a segment; each fragment should carry one `pssh` per SystemID in `moof` with enough information when combined with `moov`/MPD `pssh`, the `seig` KID and the sample-to-group boxes; KID should be observable from a v1 `pssh`; otherwise compare `pssh` bytes to detect updates. |
| Different qualities | SD/HD/UHD with different licence rights go into separate AdaptationSets, each with its own default_KID; seamless switching between them is allowed but not required and is hard (different keys, output rules, decode parameters); players may fetch several licences and switch if able. Representation switching inside a set needs no key change. Sets with `@bitstreamSwitching` true are initialised once per Period; false means partial re-initialisation per switch. |
| Multiple Periods | A new licence / changed `cenc:default_KID` must start at a Period start; the same default_KID can span Periods (client avoids re-downloading) but DRM may rebuild its whole context between Periods with different or no licences. |
| 7.7.9 client-DRM interaction | The client uses default_KID as the only abstraction towards the DRM system; other keys the DRM needs stay transparent. Client must derive required media keys from the default KIDs of the selected AdaptationSets, should request missing keys, may request additional usable keys, must request every needed key explicitly (never assume one request unlocks others), and may batch requests. |

## 7.8 Workflow overview (pp.194-197)

Roles in Fig. 24: Content Provider, Encoder, Packager/Encryptor, MPD Creator, Player & DRM Client, DRM Service, exchanging DRM information (KIDs, keys, asset IDs, licence acquisition data). Descriptions: the Encoder needs an asset identifier per track tied to KID, Representation, optional `pssh` and licence; the Packager writes default_KID into `tenc`, IVs/subsample ranges (`saio`/`saiz`), optionally `pssh`, and sample-group boxes for multi-key tracks; the MPD Creator should include CENC/DRM descriptors, default_KID per set and enough UUID-descriptor data to get a licence. Fig. 25 is a 10-step player flow: (1) MPD default_KID, (2) SystemID check, (3) init segment `tenc` default_KID, (4)-(5) licence request/response (by player or DRM client), (6) licence to DRM client, key decryption, (7)-(9) encrypted samples to DRM client and clear samples back, (10) decode and play. Licences need not be stored to find keys by KID; how the DRM finds the key is DRM-specific.
