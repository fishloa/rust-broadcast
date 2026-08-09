# DASH-IF CPIX 2.3 — Content Protection Information Exchange Format

> Source: **DASH-IF Implementation Guidelines: Content Protection Information Exchange
> Format (CPIX), Version 2.3**, "Commit Snapshot, 3 September 2020".
> HTML: `https://dash-industry-forum.github.io/docs/CPIX2.3/Cpix.html`
> PDF: `https://dash-industry-forum.github.io/docs/CPIX2.3/Cpix.pdf`
> XSD: `https://dash-industry-forum.github.io/docs/CPIX2.3/XmlSchema.zip`
> Fetched: 2026-08-09. Also published as **ETSI TS 103799 V1.1.1**.
>
> This is a *living document* maintained on GitHub
> (`https://github.com/Dash-Industry-Forum/CPIX`); v2.3.1 and a v2.4 community-review
> draft exist and mostly add clarifications — see [README.md](README.md) "Version
> currency" for what changed and why 2.3 was read instead.
>
> Transcribed directly from the published ReSpec HTML (clean semantic markup — tables,
> headings and `<dfn>` element/attribute definitions are native DOM, not reconstructed
> from a PDF render), not `pdftotext`, per this project's no-mangled-tables rule. Section
> numbers below are the spec's own (§5.2.x = "Hierarchical Data Model" subsections,
> §6.x = "Key Management").

## 0. Scope (§1) and terms (§3.2)

> "A CPIX document contains keys and DRM information used for encrypting and protecting
> content and can be used for exchanging this information among entities needing it in
> many possibly different workflows for preparing, for example, DASH or HLS content."

CPIX 2.3's headline additions over 2.2: the `commonEncryptionScheme` attribute (CENC
scheme identifier on a `ContentKey`), the `CPIX@version` attribute, and guidance on using
one content key across multiple encryption schemes (§6.4 below).

Terms (§3.2), verbatim:

| Term | Definition |
|---|---|
| Content | One or more audio-visual elementary streams and the associated MPD if in DASH format. |
| Content Key | A cryptographic key used for encrypting part of the Content. |
| Content Protection | The mechanism ensuring that only authorized devices get access to Content. |
| DRM Signaling | The DRM specific information to be added in Content for proper operation of the DRM system when authorizing a device for this Content. It is made of proprietary information for licensing and key retrieval. |
| Document Key | A cryptographic key used for encrypting the Content Key(s) in the CPIX document. |
| PSSH | Protection System Specific Header box that is part of an ISOBMFF file. This box contains DRM Signaling. |
| Content Key Context | The portion of a media stream which is encrypted with a specific Content Key. |

A CPIX document is a plain XML file (namespace `urn:dashif:org:cpix`, root element
`CPIX`), not a binary/box format — it exists to move key + DRM-signalling material
*between systems* (packager ↔ key server), never to a client device. There is no
conformance statement — the spec deliberately does not define pass/fail conformance
because the container is generic (§3, "no conformance is defined for this
specification").

## 1. Cardinality/optionality notation used below

Copied from the spec's own convention (§5.2, "Structure Overview"): each attribute row
is `name (X, type)` where `X` is one of:

| Code | Meaning |
|---|---|
| `M` | Mandatory |
| `O` | Optional |
| `OD` | Optional with a documented default |

Each child-element row is `Name (min...max, Type)` — e.g. `DeliveryDataList (0...1,
DeliveryDataList)` reads "zero or one `DeliveryDataList` child, of type
`DeliveryDataList`"; `ContentKey (0...N, ContentKey)` reads "zero or more".

## 2. `CPIX` — the root element (§5.2.1)

> "The root element that carries the Content Protection Information for a set of media
> assets."

| Attribute/child | Card. | Type | Description |
|---|---|---|---|
| `id` | O | `xs:ID` | Identifier for the CPIX document. Recommended unique within its publication scope. |
| `contentId` | O | `xs:string` | Identifier for the protected asset/content. Recommended unique within scope. |
| `name` | O | `xs:string` | A name for the presentation. |
| `version` | O | `xs:string` | Version of the *CPIX Guidelines* this document targets, `majorVersion.minorVersion` (e.g. `"2.3"`). If a client doesn't support all features of the stated version, it must fall back per the API's own rules. |
| `DeliveryDataList` | 0...1 | `DeliveryDataList` | Container for `DeliveryData` elements. **Absent → Content Keys are delivered in the clear.** |
| `ContentKeyList` | 0...1 | `ContentKeyList` | Container for `ContentKey` elements. |
| `DRMSystemList` | 0...1 | `DRMSystemList` | Container for `DRMSystem` elements. Absent → no DRM signalling data in the document. |
| `ContentKeyPeriodList` | 0...1 | `ContentKeyPeriodList` | Container for `ContentKeyPeriod` elements. |
| `ContentKeyUsageRuleList` | 0...1 | `ContentKeyUsageRuleList` | Container for `ContentKeyUsageRule` elements. Absent → no Content Key Contexts defined; an external mechanism must synchronise the workflow instead. |
| `UpdateHistoryItemList` | 0...1 | `UpdateHistoryItemList` | Container for `UpdateHistoryItem` elements. |
| `Signature` | 0...N | `ds:Signature` | [XMLDSIG-CORE] signature(s), over the whole document or over any subset of `@id`-bearing elements. Every signature must carry an X.509 certificate identifying the signer + its public key. |

Note `CPIX@id` vs `CPIX@contentId`: `id` identifies *this document* (a specific
exchange), `contentId` identifies *the asset* the keys protect — a re-sent/updated
document for the same asset keeps `contentId` but may get a new `id`.

## 3. `DeliveryDataList` / `DeliveryData` (§5.2.2–5.2.3)

`DeliveryDataList`:

| Attribute/child | Card. | Type | Description |
|---|---|---|---|
| `id` | O | `xs:ID` | Element identifier. |
| `updateVersion` | O | `xs:integer` | Matches an `UpdateHistoryItem@updateVersion`. |
| `DeliveryData` | 0...N | `DeliveryData` | One per entity able to access the encrypted Content Keys in this document. **Absent from the parent `CPIX` → keys are in the clear.** |

`DeliveryData` — the encryption-key-hierarchy entry for one recipient:

| Attribute/child | Card. | Type | Description |
|---|---|---|---|
| `id` | O | `xs:ID` | Element identifier. |
| `updateVersion` | O | `xs:integer` | As above. |
| `name` | O | `xs:string` | Name of this DeliveryData. |
| `DeliveryKey` | 1 | `ds:KeyInfoType` | X.509 certificate identifying the intended recipient + the public key used to encrypt the Document Key (§6.1 below). |
| `DocumentKey` | 1 | `cpix:KeyType` | The 256-bit key that encrypts every `ContentKey` in this document (an extension of RFC 6030 `KeyType` with `id`/`Algorithm` made optional), itself encrypted with this recipient's public key. |
| `MACMethod` | 0...1 | `pskc:MACMethodType` | MAC algorithm + MAC key (encrypted with the Delivery Key) for authenticated encryption of Content Keys — §6.1.2. |
| `Description` | 0...1 | `xs:string` | Free-text description. |
| `SendingEntity` | 0...1 | `xs:string` | Name of the entity generating this document. |
| `SenderPointOfContact` | 0...1 | `xs:string` | Contact info (e.g. email) for the sender. |
| `ReceivingEntity` | 0...1 | `xs:string` | Name of the entity able to decrypt this DeliveryData's Content Keys. |

## 4. `ContentKeyList` / `ContentKey` (§5.2.4–5.2.5)

`ContentKeyList`: `id` (O, `xs:ID`), `updateVersion` (O, `xs:integer`), `ContentKey`
(0...N).

`ContentKey` — an extension of RFC 6030 `KeyType` (its `id`/`Algorithm` are optional
here); the key value can be in the clear (`pskc:PlainValue`) or encrypted under the
Document Key (`pskc:EncryptedValue` + `pskc:ValueMAC`, §6.1):

| Attribute | Card. | Type | Description |
|---|---|---|---|
| `id` | O | `xs:ID` | Element identifier. |
| `Algorithm` | O | `xs:ID` | Inherited from RFC 6030, made optional here. |
| `kid` | **M** | `cpix:KeyIdType` | Unique Content Key identifier, formatted per [MPEGCENC] §11.2 (the CENC `key_ID` — a 16-byte value, conventionally rendered as a UUID). |
| `explicitIV` | O | `xs:base64binary` | A single 128-bit IV to associate with this key, base64-encoded, for DRM systems whose client can't extract the IV from the content and needs it delivered with the key by the license server. Not recommended except for that compatibility case; ignored otherwise even if present. |
| `dependsOnKey` | O | `xs:string` | Marks this key as a **leaf key** in a 2-level key hierarchy (§6.3); value = the `kid` of the **root key**. The referenced key must not itself be a leaf. Absent → this key is either a root key or not part of a hierarchy — CPIX does not distinguish those two cases. |
| `commonEncryptionScheme` | O | `xs:string`, length 4 | The CENC protection-scheme fourCC (`'cenc'`/`'cbcs'`/`'cbc1'`/`'cens'` — [MPEGCENC]) this key is intended for. **Must not be set together with `dependsOnKey`** — in a hierarchy, the *root* key's scheme governs the whole hierarchy. Omitted → any scheme may be used. Keep this aligned with whatever scheme identifier appears inside the corresponding `DRMSystem` signalling data (§5). |

## 5. `DRMSystemList` / `DRMSystem` / `HLSSignalingData` (§5.2.6–5.2.8)

`DRMSystemList`: `id` (O), `updateVersion` (O), `DRMSystem` (0...N) — "DRM Signaling of
a DRM system associated with a Content Key."

`DRMSystem` — one DRM system's manifest/init-data signalling for one `ContentKey`. This
is the element that actually carries `pssh` boxes, MPD `ContentProtection` XML, HLS
`EXT-X-KEY`/`EXT-X-SESSION-KEY` lines and the Smooth Streaming `ProtectionHeader`:

| Attribute/child | Card. | Type | Description |
|---|---|---|---|
| `id` | O | `xs:ID` | Element identifier. |
| `updateVersion` | O | `xs:integer` | As above. |
| `systemId` | **M** | `xs:string` | The DRM system's UUID (see `docs/drm/pssh.md` §1 in `transmux` for the values — Widevine/PlayReady/FairPlay/ClearKey; values are published at `dashif.org/identifiers/content_protection/`). |
| `kid` | **M** | `xs:string` | Matches the `kid` of the `ContentKey` this element signals for. |
| `name` | O | `xs:string` | Human-readable DRM system name+version; usable as the DASH MPD `ContentProtection/@value`. |
| `PSSH` | 0...1 | `xs:base64binary` | The **full** ISOBMFF `pssh` box (ISO/IEC 23001-7 §12.1 — see `transmux/docs/drm/pssh.md` §2) to add to the file. Mandatory *in practice* whenever the media is ISOBMFF. When the referenced key is a hierarchy leaf key, the box goes under `moof`; otherwise (root key or no hierarchy) it goes under `moov` and the DRM signalling should instead be carried via `ContentProtectionData` for manifest purposes. |
| `ContentProtectionData` | 0...1 | `xs:base64binary` | A **base64-encoded, well-formed, standalone XML fragment** to insert under the DASH MPD's `ContentProtection` element for this DRM system (e.g. the DASH-IF `dashif:*` children per [DASHIFIOP] §7.7.1). Must not be used when the key is a hierarchy leaf. Meaningful only when a DASH manifest is generated. |
| `URIExtXKey` | 0...1 | `xs:base64binary` | **Deprecated** — the raw data for an HLS `EXT-X-KEY` `URI` parameter. Use `HLSSignalingData` instead. |
| `HLSSignalingData` | 0...2 | `HLSSignalingData` | The **full** HLS tag line(s) (`#EXT-X-KEY` or `#EXT-X-SESSION-KEY`), base64-encoded UTF-8, no BOM — see §6 below. At most two, one per `playlist` value (`master`/`media`); must not be used when the key is a leaf. |
| `SmoothStreamingProtectionHeaderData` | 0...1 | `xs:string` | The inner text of the Smooth Streaming manifest's `ProtectionHeader` element, UTF-8, no BOM. Must not be used for a leaf key. |
| `HDSSignalingData` | 0...1 | `xs:base64binary` | The full `drmAdditionalHeader` element (open/close tags included) for an HDS (Flash) playlist. Must not be used for a leaf key. |

The spec explicitly allows non-DASH-IF child elements after all of the above, for
signalling formats it doesn't define (implementations must place them last).

`HLSSignalingData` (§5.2.8) — a wrapper for one base64-encoded HLS tag line:

| Attribute | Card. | Type | Description |
|---|---|---|---|
| `playlist` | O, restricted `xs:string` | one of `master`/`media` | Which playlist this line goes in. Two `HLSSignalingData` children of one `DRMSystem` **must** have different `playlist` values. If omitted, this is the (sole) media-playlist line and there is no master-playlist signalling for that DRM system. |

The `master`-playlist line uses `#EXT-X-SESSION-KEY`; the `media`-playlist line uses
`#EXT-X-KEY`. Both are the same tag family — see `transmux/docs/drm/hls-sample-aes.md`
§9 for the tag-attribute grammar, and §7 below for a real decoded example.

## 6. `ContentKeyPeriodList` / `ContentKeyPeriod` (§5.2.9–5.2.10) — key rotation

`ContentKeyPeriodList`: `id` (O), `updateVersion` (O), `ContentKeyPeriod` (0...N) — "For
every Content Key, `ContentKeyPeriod` elements cover non-overlapping periods of time.
The concatenation of all periods of time may not fully cover the Content, as some parts
may be in the clear."

`ContentKeyPeriod`:

| Attribute | Card. | Type | Description |
|---|---|---|---|
| `id` | O | `xs:ID` | Element identifier — referenced by `KeyPeriodFilter@periodId` (§8). |
| `index` | O | `xs:integer` | Numeric sequence index for the period. **Mutually exclusive** with `start`/`end`. |
| `start` | O | `xs:dateTime` | Wall-clock (Live) or media time (VOD) start. Mutually **inclusive** with `end`, mutually exclusive with `index`. |
| `end` | O | `xs:dateTime` | End time. Interval is **`[start, end)`** — the key is in use at `start` but not at `end`. |

If neither `start`/`end` nor `index` semantics are needed precisely (the encryptor
rotates on its own internal schedule, e.g. hourly, without publishing exact boundaries),
periods are referenced purely by `index`/sequence.

## 7. `ContentKeyUsageRuleList` / `ContentKeyUsageRule` + filters (§5.2.11–5.2.13)

`ContentKeyUsageRuleList`: `id` (O), `updateVersion` (O), `ContentKeyUsageRule`
(0...N) — "A rule which defines a Content Key Context."

`ContentKeyUsageRule` — maps a `ContentKey` to the tracks/periods/labels it protects:

| Attribute/child | Card. | Type | Description |
|---|---|---|---|
| `id` | O | `xs:ID` | Element identifier. |
| `kid` | **M** | `xs:string` | The `ContentKey` this rule maps. In a key hierarchy, must reference a **leaf** key, never a root key. |
| `intendedTrackType` | O | `xs:string` | Free-text label for the media-track type this rule matches (e.g. `UHD`, `UHD+HFR`) — business-logic metadata, not itself part of the matching logic (contrast with `LabelFilter` below). |
| `KeyPeriodFilter` | 0...N | `KeyPeriodFilter` | Restricts the rule to a `ContentKeyPeriod`. |
| `LabelFilter` | 0...N | `LabelFilter` | Restricts the rule to samples carrying a matching pre-agreed label. |
| `VideoFilter` | 0...N | `VideoFilter` | Video-only constraints. |
| `AudioFilter` | 0...N | `AudioFilter` | Audio-only constraints. |
| `BitrateFilter` | 0...N | `BitrateFilter` | Bitrate constraints. |

Non-DASH-IF filter elements may appear, but must come after all of the above.

**Combination logic (§5.2.13.1, verbatim):** multiple filters of the *same* type on one
rule are OR'd together; different filter *types* on one rule are AND'd. E.g. two
`LabelFilter`s (`stream-1`, `stream-2`) plus one `VideoFilter` matches `(stream-1 OR
stream-2) AND video`. A CPIX document is invalid if any possible Content Key Context
would match more than one Content Key. A rule with an unrecognised child (unknown filter
type) or an unevaluable constraint (e.g. `minPixels` when the pixel count isn't known)
is "unusable" — its Content Key must not be mapped by that rule, but the rest of the
document remains processable.

### 7.1 `KeyPeriodFilter` (§5.2.13.2)

| Attribute | Card. | Type | Description |
|---|---|---|---|
| `periodId` | **M** | `xs:IDREF` | References a `ContentKeyPeriod@id`. Matches only samples in that period. |

### 7.2 `LabelFilter` (§5.2.13.3)

| Attribute | Card. | Type | Description |
|---|---|---|---|
| `label` | **M** | `xs:string` | Matches samples carrying this label. The label's *meaning* is implementation-defined, agreed out-of-band between producer and consumer — it is a pre-agreed trigger string (e.g. `UHD`), not a claim about track type in itself. |

Contrast with `intendedTrackType`: `LabelFilter` values must be pre-agreed between
parties and drive the actual key/track matching; `intendedTrackType` is descriptive
business-logic metadata attached to the rule and plays no role in matching.

### 7.3 `VideoFilter` (§5.2.13.4)

Present (even with no attributes) → matches only video samples.

| Attribute | Card. | Type | Default | Description |
|---|---|---|---|---|
| `minPixels` | OD | `xs:integer` | 0 | Minimum encoded width×height (before PAR/SAR). |
| `maxPixels` | OD | `xs:integer` | `MAX_UINT32` | Maximum encoded width×height. |
| `hdr` | O | `xs:boolean` | — | Whether the stream is HDR. |
| `wcg` | O | `xs:boolean` | — | Whether the stream is WCG. |
| `minFps` | O | `xs:integer` | — | Minimum nominal fps (half the field rate for interlaced). |
| `maxFps` | O | `xs:integer` | — | Maximum nominal fps. |

`[minPixels, maxPixels]` is inclusive at both ends; `(minFps, maxFps]` is inclusive only
at `maxFps` (i.e. content at exactly `minFps` is *excluded*).

### 7.4 `AudioFilter` (§5.2.13.5)

Present (even with no attributes) → matches only audio samples.

| Attribute | Card. | Type | Default | Description |
|---|---|---|---|---|
| `minChannels` | OD | `xs:integer` | 0 | Minimum channel count. |
| `maxChannels` | OD | `xs:integer` | `MAX_UINT32` | Maximum channel count. |

`[minChannels, maxChannels]` is inclusive at both ends.

### 7.5 `BitrateFilter` (§5.2.13.6)

| Attribute | Card. | Type | Default | Description |
|---|---|---|---|---|
| `minBitrate` | OD | `xs:integer` | 0 | Minimum nominal bitrate, b/s. At least one of `minBitrate`/`maxBitrate` must be given. |
| `maxBitrate` | OD | `xs:integer` | `MAX_UINT32` | Maximum nominal bitrate, b/s. |

`[minBitrate, maxBitrate]` inclusive at both ends.

## 8. `UpdateHistoryItemList` / `UpdateHistoryItem` (§5.2.14–5.2.15)

`UpdateHistoryItemList`: `id` (O), `UpdateHistoryItem` (0...N) — one entry per update
made to the document, in principle.

`UpdateHistoryItem`:

| Attribute | Card. | Type | Description |
|---|---|---|---|
| `id` | O | `xs:ID` | Element identifier. |
| `updateVersion` | **M** | `xs:integer` | The ID other elements reference via their own `updateVersion` attribute. Recommended unique within scope. |
| `index` | **M** | `xs:string` | Monotonically increasing version number for the update, starting at 1; unique per `UpdateHistoryItem`. |
| `source` | **M** | `xs:string` | Identifier of the entity that made this update. |
| `date` | **M** | `xs:dateTime` | When the update was made. |

(SPEKE explicitly ignores this whole element — see `speke-api.md` §2.)

## 9. Key Management (§6) — encryption, MAC, and signing of the document itself

### 9.1 Keys used to secure the document (§6.1.1)

Three key tiers, all internal to the CPIX document:

- **Content Keys** — one per `ContentKey` element; typically 128-bit AES keys (Common
  Encryption). Either the media-encryption key directly, or (in a hierarchy) a leaf that
  depends on a root key.
- **Document Key** — one 256-bit AES key per CPIX document. Encrypts every `ContentKey`
  in the document. Carried inside each `DeliveryData` element, itself encrypted with
  that recipient's public key (the Delivery Key).
- **Delivery Key** — a recipient's public key, identified via their X.509 certificate in
  `DeliveryData/DeliveryKey`. Encrypts the Document Key per [XMLENC-CORE]. One
  `DeliveryData` element exists per intended recipient, so the same Document Key (hence
  the same encrypted Content Keys) is reusable across all recipients — only the small
  Document Key needs re-encrypting per recipient, not every Content Key.

### 9.2 Authenticated encryption (§6.1.2)

A **MAC Key** is generated per document, used to MAC every encrypted Content Key;
`DeliveryData` identifies the MAC algorithm and carries the MAC Key, itself encrypted
under the Delivery Key. Implementations **must** verify the MAC before attempting to
decrypt any encrypted Content Key — the MAC exists to guard against cryptographic
vulnerabilities in the receiver, not as a general authentication mechanism. The MAC
covers the `CipherValue` (the concatenated IV + encrypted Content Key) and is stored in
`pskc:ValueMAC` under the `Secret` element.

### 9.3 Digital signature (§6.1.3)

Any element with an `@id` can be signed per [XMLDSIG-CORE]; the whole document
(including other signatures) can also be signed as a unit. Implementations *should*
verify that expected signatures are present and valid, and refuse to process a document
if they are missing/invalid/untrusted. Modifying signed data invalidates its
signature(s) — re-signing after any edit is required.

### 9.4 Mandatory algorithms (§6.1.4)

| Usage | Algorithm |
|---|---|
| Content Key wrapping | AES256-CBC, PKCS#7 padding |
| Encrypted key MAC | HMAC-SHA512 |
| Document Key wrapping | RSA-OAEP-MGF1-SHA1 |
| Digital signature | RSASSA-PKCS1-v1_5 |
| Digital signature digest | SHA-512 |
| Digital signature canonicalization | Canonical XML 1.0 (omits comments) |

Recommended minimum RSA key size: 3072 bits; certificates signed with SHA-1 are
discouraged. (SPEKE tightens this to a *hard* requirement of 2048-bit RSA and drops
XMLDSIG signature verification entirely — see `speke-api.md` §2.)

### 9.5 Key rotation support (§6.2)

One `ContentKey` per crypto-period, each tied to a `ContentKeyPeriod` (by `start`/`end`
or `index`). Clear (unencrypted) periods within otherwise-rotating content are not
represented explicitly in CPIX — the document simply has no `ContentKey` for that
period; whether a given span of content is in the clear has to come from the
playlist/manifest or another out-of-band source (EPG, metadata server), not from CPIX
itself.

### 9.6 Hierarchical keys (§6.3)

CPIX supports exactly **two-level** hierarchies: each leaf key has exactly one root key
required to use it (`ContentKey@dependsOnKey`, §4 above). Only leaf keys may encrypt
media — root keys must never be referenced by a `ContentKeyUsageRule`. The DRM-specific
mechanism for using a root key to unlock a leaf key is DRM-system-specific and out of
CPIX's scope; CPIX only records the *reference*, and changes where `DRMSystem`
signalling data is placed accordingly (`PSSH` under `moof` instead of `moov`;
`ContentProtectionData`/`HLSSignalingData`/etc. must not be used at all for a leaf
key — see §5 above).

### 9.7 One Content Key, several encryption schemes (§6.4)

[MPEGCENC] defines several *non-interoperable* protection schemes (`cenc`/`cbcs`/`cbc1`/
`cens`). The same Content Key value **may** (not recommended, but permitted) be reused
across differently-scheme-encrypted versions of the same content — in CPIX terms this
means one *separate document per scheme*, differing in `ContentKey@commonEncryptionScheme`
and potentially in the `DRMSystem` child elements too (some DRM systems encode the
scheme identifier inside their own signalling payload — see `transmux/docs/drm/pssh.md`
§4 for Widevine's `protection_scheme` protobuf field, cross-corroborated against a real
CPIX document in §7 below).

## 10. Examples (§7)

The spec's own §7 does not inline example documents — it says only: "Example CPIX
documents are available on GitHub." That is exactly the
`Dash-Industry-Forum/cpix-test-vectors` repository this project vendored real fixtures
from (`fixtures/cpix/`, MIT-licensed) — see [README.md](README.md) §"Fixtures" and the
worked byte-level decode in §11 below.

## 11. Cross-corroboration against a real fixture — Widevine `pssh` inside a real CPIX document

`fixtures/cpix/Complex.xml` (from the vendored test-vector set) contains this
`DRMSystem` element (systemId = Widevine, ISOBMFF signalling only, decoded from its
base64 `ContentProtectionData`/`HLSSignalingData` child text):

```xml
<DRMSystem systemId="edef8ba9-79d6-4ace-a3c8-27dcd51d21ed" kid="b4c3188b-eddd-453d-9bc2-1cbca7566239">
  <ContentProtectionData><!-- base64 of: --><pssh xmlns="urn:mpeg:cenc:2013">AAAAOHBzc2gAAAAA7e+LqXnWSs6jyCfc1R0h7QAAABgSELTDGIvt3UU9m8IcvKdWYjlI49yVmwY=</pssh></ContentProtectionData>
  <HLSSignalingData playlist="master"><!-- base64 of --> #EXT-X-SESSION-KEY:METHOD=SAMPLE-AES-CTR,URI="data:text/plain;base64,AAAAOHBzc2gAAAAA7e+LqXnWSs6jyCfc1R0h7QAAABgSELTDGIvt3UU9m8IcvKdWYjlI49yVmwY=",KEYID=0xB4C3188BEDDD453D9BC21CBCA7566239,KEYFORMAT="urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed",KEYFORMATVERSIONS="1"</HLSSignalingData>
</DRMSystem>
```

Base64-decoding the inner `<pssh>` element's text yields the raw `pssh` box (hex,
56 bytes):

```
00 00 00 38 70 73 73 68 00 00 00 00
ed ef 8b a9 79 d6 4a ce a3 c8 27 dc d5 1d 21 ed   -- SystemID = Widevine (matches §5 above)
00 00 00 18                                       -- DataSize = 24
12 10 b4 c3 18 8b ed dd 45 3d 9b c2 1c bc a7 56 62 39   -- protobuf field 2 (key_id), len 16
48 e3 dc 95 9b 06                                       -- protobuf field 9 (protection_scheme), varint
```

This decodes byte-for-byte against `transmux/docs/drm/pssh.md` §2 (box layout) and §4
(Widevine `WidevineCencHeader` protobuf field table):

- `size`/`type`/`version`/`flags`/`SystemID` match the version-0 `pssh` box layout
  exactly, and the SystemID is the documented Widevine UUID.
- Protobuf field 2 (`key_id`, tag byte `0x12` = `(2<<3)|2`) carries 16 bytes
  `b4c3188b-eddd-453d-9bc2-1cbca7566239` — the **same** UUID as this `DRMSystem`'s
  `kid` attribute, confirming the CENC-big-endian (no swap) key-id convention documented
  for Widevine.
- Protobuf field 9 (`protection_scheme`, tag byte `0x48` = `(9<<3)|0`, varint) decodes to
  `0x63656E63` = ASCII `"cenc"` — exactly the fourCC `transmux/docs/drm/pssh.md` §4
  documents for that field, and it correctly reflects the HLS line's
  `METHOD=SAMPLE-AES-CTR` (CENC's `cenc` scheme is CTR-mode; `cbcs` would show up as
  `0x63626373`).

This is independent, real-world (DASH-IF-published, not this project's own) evidence
that the existing `transmux::drm` PSSH/Widevine-protobuf implementation and its
`docs/drm/pssh.md` transcription are correct, not just internally self-consistent —
see the decode script recorded in `fixtures/PROVENANCE.md`.

## References (from the spec's own bibliography, as cited above)

- [MPEGCENC] ISO/IEC 23001-7, Common encryption in ISO base media file format files.
- [DASHIFIOP] DASH-IF Interoperability Points guidelines.
- [XMLDSIG-CORE] W3C XML Signature Syntax and Processing.
- [XMLENC-CORE] W3C XML Encryption Syntax and Processing.
- RFC 6030 (PSKC — Portable Symmetric Key Container), reused for `KeyType`/`Secret`/
  `PlainValue`/`EncryptedValue`/`ValueMAC`/`MACMethodType`.
