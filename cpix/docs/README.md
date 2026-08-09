# cpix — spec table reference

Prep work for issue **#745** ("multimux: DRM key-server integration (Widevine /
PlayReady / FairPlay)"). This directory is **docs only** — no `Cargo.toml`, no Rust
source, not a workspace member. It exists so that a future `cpix` crate (per
`docs/ROADMAP.md`'s crate-placement table: `#745 → cpix, NEW`) starts implementation
from a reviewable, spec-cited transcription and a real fixture set, rather than from
memory of the spec. **No Rust was written for this task.**

## What was already covered before this task (checked first, not re-transcribed)

The bulk of the *DRM signalling byte layouts* this issue mentions were already fully
implemented and documented, in `transmux`, before this task started:

| Already in the workspace | Where |
|---|---|
| `CencScheme` (`cenc`/`cbcs` ISO/IEC 23001-7 scheme identity) | `broadcast-common::cenc` |
| `sinf`/`frma`/`schm`/`schi`/`rinf`/`stvi` box layouts (ISO/IEC 14496-12) | `docs/specs/transmux/protection-scheme.md` |
| `tenc`/`pssh`/`senc` CENC boxes, CENC/CBCS encrypt+decrypt | `transmux/src/cenc*.rs` |
| **Multi-DRM `pssh` payload generation**: DRM system UUIDs (Widevine/PlayReady/FairPlay/ClearKey), the version-0/1 `pssh` box layout, PlayReady PRO+WRMHEADER (all 4 XML versions, v4.0–v4.3) with the GUID↔UUID KID byte-swap, the Widevine `WidevineCencHeader` protobuf field table, and the FairPlay `skd://` URI convention | `transmux/src/drm.rs` + `transmux/docs/drm/pssh.md` (+ biting tests in `transmux/tests/drm_pssh.rs`) |
| HLS `#EXT-X-KEY`/`#EXT-X-SESSION-KEY` rendering (`cenc_ext_x_key`) | `broadcast-hls` |
| **HLS SAMPLE-AES byte layouts** (H.264/AAC/AC-3/E-AC-3 clear-prefix + skip-pattern + IV-reset rules) and the exact `EXT-X-KEY` attribute grammar for AES-128 / FairPlay / Widevine / ClearKey | `transmux/docs/drm/hls-sample-aes.md` |
| Real CENC/CBCS-encrypted fixtures (Bento4-produced, issue #564) | `fixtures/transmux/h264_{cenc,cbcs}.mp4` + `fixtures/transmux/h264_cenc-PROVENANCE.md`; also `fixtures/mp4/cenc.mp4` |

**Conclusion: the box/payload-format half of #745 is done.** What was missing — the
actual gap this task fills — is the **interchange format between a packager and a key
server**: DASH-IF CPIX (the document that carries content keys + DRM signalling between
systems) and AWS SPEKE (the concrete REST API profile over CPIX most cloud key
providers implement). Neither `cpix` nor `speke` appeared anywhere in the workspace
before this task (confirmed by a case-insensitive grep across `*.rs`/`*.md`, excluding
`target/`) except as forward-looking mentions in `docs/ROADMAP.md`.

## Sources

- **DASH-IF CPIX 2.3** — `https://dash-industry-forum.github.io/docs/CPIX2.3/Cpix.html`
  (+ PDF/XSD at the same path). "Commit Snapshot, 3 September 2020." Also published as
  ETSI TS 103799 V1.1.1. Transcribed in [`cpix-2.3-dashif.md`](cpix-2.3-dashif.md).
- **AWS SPEKE v1/v2** — `https://docs.aws.amazon.com/speke/latest/documentation/`.
  Transcribed in [`speke-api.md`](speke-api.md).
- Both fetched 2026-08-09, both freely public (no login, no NDA, no paywall).

### Version currency — why CPIX 2.3, not 2.3.1 or 2.4

`dashif.org/guidelines/specifications/` lists newer artifacts:

- **CPIX 2.3.1** — a GitHub-only release (no ETSI publication planned); PDF/XSD linked
  from that page. Not read separately: AWS's own SPEKE v2 documentation (fetched
  2026-08-09) explicitly cites **2.3**, not 2.3.1, as the version it aligns to
  (`standard-payload-components-v2.html`: "For detailed information ... see the DASH
  Industry Forum CPIX 2.3 specification"), so 2.3 is the version actually load-bearing
  for the SPEKE integration this issue targets.
- **CPIX 2.4** — a **community-review draft** (review window closed; ETSI publication
  "under preparation" per the search results at the time of writing). Draft status means
  its schema could still change before ratification — not an authoritative source yet.

If a future implementation targets a key provider that requires 2.3.1 or 2.4-specific
behaviour, that delta needs its own pass; nothing here rules that out, it just wasn't
in scope for what SPEKE (the concrete, currently-deployed integration target) needs.

### How it was read

Both sources are clean semantic HTML (DASH-IF: a ReSpec/Bikeshed-generated spec page;
AWS: their standard doc-site markup) — extracted with `lxml.html`, walking headings and
`<table>` elements directly, **not** OCR'd and **not** run through `pdftotext` (no PDF
was used as the source of record for either; the CPIX PDF is a rendering of the same
HTML DASH-IF publishes, and was fetched only to confirm it exists/matches, not as the
transcription source). This sidesteps the project's usual PDF-table-mangling risk
entirely, at the cost of depending on the HTML staying live — the exact URLs and fetch
date are recorded above for reproducibility, and the CPIX HTML page mirrors the PDF
DASH-IF itself publishes at a stable path.

## Files in this directory

| File | Covers |
|---|---|
| [`cpix-2.3-dashif.md`](cpix-2.3-dashif.md) | The full CPIX 2.3 XSD data model (§5.2 in the spec): `CPIX` root, `DeliveryData(List)`, `ContentKey(List)`, `DRMSystem(List)` + `HLSSignalingData`, `ContentKeyPeriod(List)`, `ContentKeyUsageRule(List)` + its five filter types, `UpdateHistoryItem(List)`; the Key Management chapter (§6: document/content/delivery/MAC key hierarchy, mandatory algorithms, key rotation, 2-level key hierarchies, multi-scheme reuse); and a byte-level cross-corroboration of a real CPIX document's Widevine `pssh` payload against `transmux/docs/drm/pssh.md`. |
| [`speke-api.md`](speke-api.md) | AWS SPEKE v1/v2: HTTP method/headers/status codes, the v1→v2 delta, SPEKE's restriction profile over full CPIX (what's ignored, what's mandatory, the exact error-message taxonomy), the standard payload-component tables, the full "encryption contract" (`ContentKeyUsageRuleList`) semantics including SPEKE's filter subset, a complete real VOD request/response example, and the content-key-encryption (`DeliveryDataList`) flow. |

**21 distinct CPIX elements** are documented with a full attribute/child table across
the two files (`CPIX`, `DeliveryDataList`, `DeliveryData`, `ContentKeyList`,
`ContentKey`, `DRMSystemList`, `DRMSystem`, `HLSSignalingData`, `ContentKeyPeriodList`,
`ContentKeyPeriod`, `ContentKeyUsageRuleList`, `ContentKeyUsageRule`, `KeyPeriodFilter`,
`LabelFilter`, `VideoFilter`, `AudioFilter`, `BitrateFilter`, `UpdateHistoryItemList`,
`UpdateHistoryItem`, plus the PSKC-borrowed `Secret`/`EncryptedValue`/`PlainValue`/
`ValueMAC` and XMLENC/XMLDSIG-borrowed `EncryptionMethod`/`CipherData`/`X509Certificate`
reused unmodified from RFC 6030 / [XMLENC-CORE] / [XMLDSIG-CORE]).

## Fixtures (`fixtures/cpix/`)

Real DASH-IF-published CPIX documents, vendored from the **official CPIX test-vector
repository** DASH-IF's own spec §7 ("Examples are available on GitHub") points to:
`https://github.com/Dash-Industry-Forum/cpix-test-vectors`, commit
`e5cef98097a6115d6f561ace5eebba84ce950f3a` (2021-05-07), **MIT licence** (Axinom 2016,
generated by `Axinom.Cpix` v2.2.0). Full provenance table in `fixtures/PROVENANCE.md`.

Five representative documents were vendored (out of the repo's full set, which also has
intentionally-invalid signature/MAC fixtures and X.509 test certificates not needed for
this prep pass):

| File | What it exercises |
|---|---|
| `ClearContentKeysOnly.xml` | Simplest valid document: `ContentKeyList` only, keys in the clear. |
| `Complex.xml` | The full model: `DeliveryDataList` (4 recipients, encrypted+signed), `ContentKeyList` (encrypted keys), `DRMSystemList` with real Widevine/PlayReady/FairPlay `pssh`+HLS+Smooth-Streaming signalling data, digital signatures. This is the fixture the byte-level `pssh` cross-check in `cpix-2.3-dashif.md` §11 decodes. |
| `EncryptedContentKeys.xml` | `DeliveryData` for a single recipient, `pskc:EncryptedValue`/`ValueMAC` content-key encryption. |
| `KeyRotationMultiKeyMulitPeriod.xml` | `ContentKeyPeriodList` + `KeyPeriodFilter`-based key rotation across two periods, two tracks. |
| `UsageRulesBasedOnLabels.xml` | `LabelFilter`-based usage rules (no `VideoFilter`/`AudioFilter`) — the "several labels OR'd, mapped to one key" case from §7.1's combination-logic rule. |

Also vendored: `widevine-pssh-from-complex.bin` — the raw 56-byte `pssh` box extracted
from `Complex.xml`'s `ContentProtectionData`, used for the cross-corroboration in
`cpix-2.3-dashif.md` §11. This is a genuine third-party artifact (DASH-IF's own test
data, not hand-built) that happens to double-check `transmux::drm`'s existing Widevine
protobuf encoder/PSSH builder — **not wired into any Rust test by this task** (that
would be implementation work, out of scope here), but flagged in the report as a
worthwhile addition to `transmux/tests/drm_pssh.rs` whenever that file is next touched.

## Explicit list of what is `UNVERIFIED` or out of scope (per the "never invent a
value" rule)

Nothing in the CPIX or SPEKE transcription itself is marked `UNVERIFIED` — both sources
were read directly from live, freely-public HTML with no paywall/NDA, and the one
concrete numeric/byte claim worth independently checking (a real Widevine `pssh` box's
contents) was decoded and cross-checked byte-by-byte in `cpix-2.3-dashif.md` §11, not
asserted from memory.

What genuinely could not be obtained, and why — **all pre-existing gaps, restated here
for completeness, not newly discovered**; already flagged as such in
`transmux/docs/drm/pssh.md`'s own "Gap summary":

1. **FairPlay SPC/CKC key-exchange protocol** — the actual license request/response
   messages a FairPlay-capable player and Apple's FairPlay Server exchange. This is
   Apple's proprietary FairPlay Streaming SDK, gated behind Apple's developer
   NDA/licensing program. **Not attempted.** What *is* public and already documented
   (`transmux/docs/drm/pssh.md` §5, `hls-sample-aes.md` §9): the `skd://` URI
   convention for FairPlay's PSSH `Data` payload and its `KEYFORMAT=
   "com.apple.streamingkeydelivery"` HLS signalling. CPIX/SPEKE carry that same
   `skd://`-URI convention unchanged inside `HLSSignalingData` — there is no
   FairPlay-specific CPIX extension to transcribe beyond what's already documented.
2. **Widevine's and PlayReady's actual license-server protocols** (the player↔DRM
   license exchange, as opposed to the packager↔key-server exchange this issue and
   CPIX/SPEKE cover) — also vendor-proprietary/NDA, and explicitly **out of scope**
   per this task's brief ("Vendor-confidential material is out of scope... Do not
   attempt to obtain or reconstruct them"). CPIX/SPEKE never touch this layer either —
   they stop at handing the packager a `pssh`/WRMHEADER/HLS-key-line; the license
   exchange is a separate, DRM-specific protocol between the *player* and the DRM
   vendor's license server, not something a CPIX document or the SPEKE API describes
   at all.
3. **CPIX 2.3.1/2.4 deltas** — see "Version currency" above; not read, believed
   low-impact for SPEKE integration specifically, but not verified.
4. **SPEKE v1-specific field-by-field detail** (as opposed to the v1→v2 delta table,
   which *is* transcribed) — deliberately not transcribed in full; see `speke-api.md`
   §7 for the reasoning (v2 is the current target; v1 pages exist only for legacy
   deployments AWS itself says don't need to migrate).

Nothing above was guessed or reconstructed from partial information — each is named as
an explicit boundary, matching the treatment `dvb-mabr/docs/README.md` gives its own
gaps list.

## Overlap with existing crates and issues in this workspace

- **`transmux`** already owns every DRM *box/payload* format this issue needs
  (`transmux::drm`, `transmux::cenc`) — a future `cpix` crate should depend on
  `transmux` (or, if `no_std`/dependency-weight matters, at minimum reuse the DRM
  system-ID constants and PSSH-builder functions rather than duplicating them) instead
  of re-deriving `pssh`/WRMHEADER/Widevine-protobuf logic. `docs/ROADMAP.md`'s own
  placement note for #745 says as much: "`cpix` (DASH-IF CPIX XML key-exchange doc
  parse/build) + `transmux`/`multimux` PSSH/license integration (CENC already
  present)".
- **`broadcast-hls`** already renders `#EXT-X-KEY` (`cenc_ext_x_key`) — a `cpix` crate
  parsing `HLSSignalingData` should hand off to (or share conventions with) that
  existing renderer rather than building a second HLS-tag serializer.
- **`multimux`** is the actual consumer named in the issue title, but has no `docs/`
  directory of its own yet and is explicitly out of scope for this prep pass (only spec
  transcription + fixtures were requested; the multimux-side integration design — how a
  key-server client plugs into its output pipeline — is implementation work for later).
- No overlap with `acap-multimux/` or `ssai-runtime/` was touched or needed; neither
  directory was read or written by this task.
