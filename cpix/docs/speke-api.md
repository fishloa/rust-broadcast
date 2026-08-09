# AWS SPEKE — Secure Packager and Encoder Key Exchange API

> Source: AWS "Secure Packager and Encoder Key Exchange API Specification", the public
> AWS documentation set at `https://docs.aws.amazon.com/speke/latest/documentation/`.
> Fetched 2026-08-09. Pages quoted below (all under that base URL):
> `what-is-speke.html`, `the-speke-api.html`, `the-speke-api-v2.html`,
> `speke-constraints-v2.html`, `standard-payload-components-v2.html`,
> `encryption-contract-v2.html`, `content-key-encryption-v2.html`,
> `vod-workflow-method-v2.html`.
>
> SPEKE is **not an independent wire format** — it is a REST API + HTTP-header profile
> layered on top of DASH-IF CPIX (see [`cpix-2.3-dashif.md`](cpix-2.3-dashif.md)), plus a
> a set of *restrictions* on which CPIX elements/attributes a compliant key provider must
> accept. Everything below should be read as a delta against that document, not a
> separate schema.

## 1. What SPEKE is (`what-is-speke.html`)

> "SPEKE defines the standard for communication between encryptors and packagers of
> media content and digital rights management (DRM) key providers... SPEKE uses the
> DASH Industry Forum Content Protection Information Exchange Format (DASH-IF-CPIX)
> data structure definition for key exchange, with some restrictions."

Two roles: the **encryptor** (a packager — e.g. `transmux`/`multimux` in this
workspace) that needs content keys + DRM signalling to produce protected output, and the
**key provider** (the DRM platform) that supplies both and separately issues licenses to
players. SPEKE covers only the encryptor↔key-provider leg — never the player↔license-
server leg (that's Widevine License Server / PlayReady License Server / FairPlay
SPC↔CKC, all vendor-proprietary — see `transmux/docs/drm/pssh.md` §5's FairPlay note and
§3 below).

Two API generations exist and are both still supported: **v1** (original) and **v2**
(CPIX-version-aware, adds mandatory `commonEncryptionScheme`/`contentId`/`version`,
multi-key support, and deprecates several SPEKE-namespace tags in favour of native CPIX
elements). v1 does not change going forward; a key provider can support both.

## 2. SPEKE v2 — API-level profile

### 2.1 HTTP transport (`the-speke-api-v2.html`, `vod-workflow-method-v2.html`)

> Request syntax (illustrative, not a fixed URL format):
> `POST https://speke-compatible-server/speke/v2.0/copyProtection`

- **Method:** `POST` only — "A SPEKE-compliant encryptor acts as a client and sends
  POST operations to the key provider endpoint."
- **Request body:** a CPIX document (`Content-Type: application/xml`).
- **Response body:** a CPIX document (same content type) on success.

**Request headers:**

| Header | Occurs | Description |
|---|---|---|
| `Authorization` (AWS SigV4) | 1..1 | AWS request signing. |
| `X-Amz-Security-Token` | 1..1 | AWS SigV4. |
| `X-Amz-Date` | 1..1 | AWS SigV4. |
| `Content-Type` | 1..1 | `application/xml` |
| `X-Speke-Version` | 1..1 | `MajorVersion.MinorVersion`, e.g. `"2.0"`. **New in v2** — its absence is "typical of SPEKE v1.0 legacy workflows"; a v2-aware key provider only processes the CPIX body if *both* this header and `CPIX@version` are present and supported. |

**Response headers:**

| Header | Occurs | Description |
|---|---|---|
| `X-Speke-User-Agent` | 1..1 | Identifies the key provider. (v2 rename of v1's `Speke-User-Agent`.) |
| `Content-Type` | 1..1 | `application/xml` |
| `X-Speke-Version` | 1..1 | Echoes the request's value — **the key provider must not change it.** |

**Status codes:** `200` = success (CPIX response body); `4XX`/`5XX` = client/server
error (error message body). §2.4 below has the exact error taxonomy. A key provider
**must never** answer `200` with an error condition packed inside — `422` is suggested
as the catch-all SPEKE/CPIX error code.

The AWS pages are explicit that these are **illustrative** examples ("You can't run the
examples because they aren't part of a complete SPEKE implementation") — the URL path
shown is not a fixed contract, only the method/headers/body shape is.

### 2.2 What v2 changed relative to v1 (`the-speke-api-v2.html`)

| v1 | v2 |
|---|---|
| SPEKE-namespace tags (`SPEKE:ProtectionHeader`, `SPEKE:KeyFormat`, `SPEKE:KeyFormatVersions`) | Deprecated in favour of native CPIX elements: `CPIX:DRMSystem.SmoothStreamingProtectionHeaderData`, `CPIX:DRMSystem.HLSSignalingData`. |
| `CPIX:URIExtXKey` | Deprecated → `CPIX:DRMSystem.HLSSignalingData`. |
| `CPIX@id` | Replaced by `CPIX@contentId` as the mandatory content identifier. |
| — | New mandatory attributes: `CPIX@version`, `ContentKey@commonEncryptionScheme`. |
| — | New optional element: `DRMSystem.ContentProtectionData`. |
| Effectively single-key-oriented | Explicit multi-content-key support. |
| No version cross-check | `X-Speke-Version` HTTP header + `CPIX@version` XML attribute cross-versioning. |
| `Speke-User-Agent` | Renamed `X-Speke-User-Agent`. |
| Heartbeat API | Deprecated entirely in v2. |

v1 implementations are not required to change — a key provider can serve both versions
side by side, gated on the `X-Speke-Version` header's presence.

### 2.3 CPIX profile restrictions SPEKE v2 imposes (`speke-constraints-v2.html`)

SPEKE v2 is best understood as picking the **Encryptor Consumer** CPIX workflow and then
narrowing it hard:

- **No XMLDSIG support at all** — "SPEKE doesn't support digital signature verification
  (XMLDSIG) for request or response payloads." (CPIX §9.3/§6.1.3 is simply unused.)
- **RSA key size floor raised**: SPEKE *requires* 2048-bit RSA certificates (CPIX itself
  only *recommends* 3072-bit — §9.4 of `cpix-2.3-dashif.md`).
- **`UpdateHistoryItemList` ignored** entirely, even if present in a response.
- **Key hierarchies ignored**: `ContentKey@dependsOnKey`, if present in a response, is
  ignored by SPEKE.
- **`BitrateFilter` and `VideoFilter@wcg` ignored** if present.
- Only elements/attributes marked "Supported" in the payload-components tables (§3
  below) or the encryption-contract page may be used at all in a SPEKE CPIX exchange.
- Any element/attribute the encryptor *does* include must come back with a valid value
  in the response, or the encryptor must error out.
- **Key rotation**: SPEKE tracks periods only via `ContentKeyPeriod@index` (not
  `start`/`end`), reached through `KeyPeriodFilter`.
- **HLS signalling is mandatory-paired**: a `DRMSystem` that needs HLS signalling must
  carry *two* `HLSSignalingData` children — one `playlist="media"`, one
  `playlist="master"`.
- `ContentKey@explicitIV` may be supplied by the encryptor in the request; the key
  provider may add/override it in the response even if the request omitted it.
- The encryptor — not the key provider — creates the `KID`. It stays stable across
  requests for the same content ID + key period; the key provider must echo it back.
- `CPIX@contentId` is **mandatory** in the request and immutable by the key
  provider — empty → error `"Missing CPIX@contentId"`.
- `CPIX@id`, if present, is ignored by the key provider entirely.
- `CPIX@version` is **mandatory** and immutable by the key provider — empty → error
  `"Missing CPIX@version"`; unsupported value → `"Unsupported CPIX@version"`.
- `ContentKey@commonEncryptionScheme` is **mandatory per key**, immutable by the key
  provider:
  - Missing → `"Missing ContentKey@commonEncryptionScheme for KID <id>"`.
  - **A single CPIX document cannot mix schemes** across its `ContentKey`
    elements → `"Non compliant ContentKey@commonEncryptionScheme combination"`.
  - Scheme incompatible with a requested `DRMSystem` → `"ContentKey@commonEncryptionScheme
    non compatible with DRMSystem <id>"`.
- If a response's `DRMSystem@PSSH` and its `ContentProtectionData` inner `<pssh>`
  disagree, the **encryptor** must error out (not the key provider) — see §2.5 CPIX
  cross-check below.

### 2.4 Error taxonomy (`speke-constraints-v2.html`)

The key provider must return one of these exact error strings in the response body,
with a `4XX`/`5XX` status (never `200`):

| Situation | Error message |
|---|---|
| `CPIX@contentId` absent | `Missing CPIX@contentId` |
| `CPIX@version` absent | `Missing CPIX@version` |
| `CPIX@version` unsupported | `Unsupported CPIX@version` |
| `ContentKey@commonEncryptionScheme` absent for a key | `Missing ContentKey@commonEncryptionScheme for KID <id>` |
| Multiple `commonEncryptionScheme` values in one document | `Non compliant ContentKey@commonEncryptionScheme combination` |
| Scheme incompatible with requested DRM system | `ContentKey@commonEncryptionScheme non compatible with DRMSystem <id>` |
| `X-Speke-Version` value unsupported | `Unsupported SPEKE version` |
| Malformed encryption contract (see §4) | `Malformed encryption contract` |
| Encryption contract breaches DRM security-level constraints | `Requested CPIX encryption contract not supported` |
| No `VideoFilter`/`AudioFilter` anywhere (missing contract) | `Missing CPIX encryption contract` |

On receiving *any* error body, the encryptor must stop — it must **not** retry the same
request downgraded to SPEKE v1 versioning.

## 3. Standard payload components (`standard-payload-components-v2.html`)

A SPEKE v2 CPIX document has (up to) four list sections. `ContentKeyList`,
`DRMSystemList`, `ContentKeyUsageRuleList` are mandatory for both Live and VOD;
`ContentKeyPeriodList` is optional, used only for live key rotation.

### 3.1 `CPIX` root + `ContentKeyList`

Minimal unencrypted example (`standard-payload-components-v2.html`, verbatim):

```xml
<cpix:CPIX contentId="abc123" version="2.3" xmlns:cpix="urn:dashif:org:cpix" xmlns:pskc="urn:ietf:params:xml:ns:keyprov:pskc">
    <cpix:ContentKeyList>
        <cpix:ContentKey explicitIV="OFj2IjCsPJFfMAxmQxLGPw==" kid="98ee5596-cd3e-a20d-163a-e382420c6eff" commonEncryptionScheme="cbcs">
            <cpix:Data>
                <pskc:Secret>
                    <pskc:PlainValue>5dGAgwGuUYu4dHeHtNlxJw==</pskc:PlainValue>
                </pskc:Secret>
            </cpix:Data>
        </cpix:ContentKey>
    </cpix:ContentKeyList>
    ...
</cpix:CPIX>
```

`ContentKey@commonEncryptionScheme` values, per [MPEGCENC] (ISO/IEC 23001-7:2016):

| Value | Meaning |
|---|---|
| `cenc` | AES-CTR, full-sample **and** video NAL subsample encryption |
| `cbc1` | AES-CBC, full-sample **and** video NAL subsample encryption |
| `cens` | AES-CTR, partial video NAL **pattern** encryption |
| `cbcs` | AES-CBC, partial video NAL **pattern** encryption |

Element-support table (from the same page):

| Element | Mandatory attrs | Optional attrs | Mandatory children | Optional children |
|---|---|---|---|---|
| `<cpix:CPIX>` | `contentId`, `version`, `xmlns:cpix`, `xmlns:pskc` | `name`, `xmlns:enc` | one `ContentKeyList`, one `DRMSystemList`, one `ContentKeyUsageRuleList` | one `DeliveryDataList`, one `ContentKeyPeriodList` |
| `<cpix:ContentKeyList>` | — | `id` | ≥1 `ContentKey` | — |
| `<cpix:ContentKey>` | `kid`, `commonEncryptionScheme`, `Data` | `id`, `Algorithm`, `explicitIV` | one `pskc:Secret` | — |
| `<pskc:Secret>` | `PlainValue` or `EncryptedValue` | `ValueMAC` | — | `enc:EncryptionMethod`, `enc:CipherData` |

### 3.2 `DRMSystemList`

Example (single PlayReady entry, verbatim):

```xml
<cpix:DRMSystemList>
    <cpix:DRMSystem kid="98ee5596-cd3e-a20d-163a-e382420c6eff" systemId="9a04f079-9840-4286-ab92-e65be0885f95">
        <cpix:HLSSignalingData playlist="media">HicXmbZ2m[...]4==</cpix:HLSSignalingData>
        <cpix:HLSSignalingData playlist="master">HicXmbZ2m[...]jEi</cpix:HLSSignalingData>
        <cpix:ContentProtectionData>t7WwH24FI[...]YCC</cpix:ContentProtectionData>
        <cpix:PSSH>FFFFanBzc[...]A==</cpix:PSSH>
        <cpix:SmoothStreamingProtectionHeaderData>s5RrJ12HL[...]UBB</cpix:SmoothStreamingProtectionHeaderData>
    </cpix:DRMSystem>
</cpix:DRMSystemList>
```

DRM systemIDs are the DASH-IF registry values — `transmux/docs/drm/pssh.md` §1
transcribes them (Widevine/PlayReady/FairPlay/ClearKey), cross-corroborated against a
real CPIX document in `cpix-2.3-dashif.md` §11.

Element-support table:

| Element | Mandatory attrs | Optional attrs | Mandatory children | Optional children |
|---|---|---|---|---|
| `<cpix:DRMSystemList>` | — | `id` | ≥1 `DRMSystem` | — |
| `<cpix:DRMSystem>` | `kid`, `systemId` | `id`, `name`, `PSSH` | — | `ContentProtectionData`, `SmoothStreamingProtectionHeaderData`, two `HLSSignalingData` (different `playlist` values) |

Cross-field rules stated on the page:

- `DRMSystem@PSSH` is mandatory whenever ISO-BMFF encapsulation applies to the segments.
- `DRMSystem.ContentProtectionData`'s inner `<pssh>` element is used by the encryptor
  **only** for MPD manifest signalling.
- **If both `DRMSystem@PSSH` and `ContentProtectionData`'s inner `<pssh>` are present,
  they must be byte-identical** (§2.3 above: violation → encryptor errors).
- If HLS signalling is used at all, both `playlist="media"` and `playlist="master"`
  `HLSSignalingData` elements are required in both the request and the response.

### 3.3 `ContentKeyPeriodList` (live only)

```xml
<cpix:ContentKeyPeriodList>
    <cpix:ContentKeyPeriod id="keyPeriod_0909829f-40ff-4625-90fa-75da3e53278f" index="1" />
</cpix:ContentKeyPeriodList>
```

| Element | Mandatory attrs | Optional attrs | Mandatory children | Optional children |
|---|---|---|---|---|
| `<cpix:ContentKeyPeriodList>` | — | `id` | ≥1 `ContentKeyPeriod` | — |
| `<cpix:ContentKeyPeriod>` | `id`, `index` | — | — | — |

Keys used with rotation must reference one of these periods (via `KeyPeriodFilter`,
§3.4).

### 3.4 `ContentKeyUsageRuleList` — the "encryption contract"

```xml
<cpix:ContentKeyUsageRuleList>
    <cpix:ContentKeyUsageRule kid="98ee5596-cd3e-a20d-163a-e382420c6eff" intendedTrackType="ALL">
        <cpix:KeyPeriodFilter periodId="keyPeriod_0909829f-40ff-4625-90fa-75da3e53278f"/>
        <cpix:AudioFilter />
        <cpix:VideoFilter />
    </cpix:ContentKeyUsageRule>
</cpix:ContentKeyUsageRuleList>
```

At least one `AudioFilter` or `VideoFilter` is required per rule.

| Element | Mandatory attrs | Optional attrs | Mandatory children | Optional children |
|---|---|---|---|---|
| `<cpix:ContentKeyUsageRuleList>` | — | `id` | ≥1 `ContentKeyUsageRule` | — |
| `<cpix:ContentKeyUsageRule>` | `kid`, `intendedTrackType` | — | ≥1 `AudioFilter` or `VideoFilter` | `KeyPeriodFilter` |
| `<cpix:KeyPeriodFilter>` | `periodId` | — | — | — |
| `<cpix:AudioFilter>` | — | `minChannels`, `maxChannels` | — | — |
| `<cpix:VideoFilter>` | — | `minPixels`, `maxPixels`, `hdr`, `minFps`, `maxFps` | — | — |

## 4. The Encryption Contract in detail (`encryption-contract-v2.html`)

The "encryption contract" is the `ContentKeyUsageRuleList` — which keys protect which
tracks. Best practice (not mandatory) is at least two keys: one audio, one video.

**Rules:**

- `ContentKeyUsageRule@intendedTrackType` value must be **unique** across the document's
  rules; it may combine sub-components with `+` (e.g. `SD+HD`, `HDR+HFR+UHD`).
- `intendedTrackType="ALL"` → all audio+video tracks share one key; the rule must then
  have exactly one bare `<AudioFilter/>` and one bare `<VideoFilter/>` (no other filter
  combination is valid in that case).
- Any other `intendedTrackType` value → the number of `AudioFilter`/`VideoFilter`
  children must match the number of `+`-joined sub-components.

**SPEKE's filter support subset** (narrower than full CPIX):

| CPIX filter | Supported by SPEKE? | Supported attributes | Unsupported attributes |
|---|---|---|---|
| `VideoFilter` | Yes | `minPixels`, `maxPixels`, `hdr`, `minFps`, `maxFps` | `wcg` |
| `AudioFilter` | Yes | `minChannels`, `maxChannels` | — |
| `KeyPeriodFilter` | Yes | `periodId` (mandatory) | — |
| `BitrateFilter` | **No** | — | — |
| `LabelFilter` | **No** | — | — |

**Error-handling table** (verbatim, condensed):

| Situation | Encryptor should/shall | Key provider should/shall |
|---|---|---|
| No rule covers some track | Verify (out-of-band) that track really shouldn't be encrypted; else error | N/A — no streamset visibility |
| Multiple rules overlap on one track | Apply the *last* rule in document order | N/A |
| Contract changes mid-cycle | Error, stop | Must never modify a received contract |
| Malformed contract (cardinality/unsupported filter) | Error, don't send | Error: `Malformed encryption contract` |
| Contract breaches DRM security-level constraints (e.g. one key for both audio and UHD video) | Error if it knows the constraint | Error: `Requested CPIX encryption contract not supported` |
| No `VideoFilter`/`AudioFilter` anywhere | Never send such a document | Error: `Missing CPIX encryption contract` |

Ten worked examples are given on the page (single key/all tracks; separate audio+video
keys; unencrypted audio; SD/HD split; SD/HD/UHD split; four/five-way video splits;
multi-attribute filter combination; stereo-vs-multichannel audio splits) — the shapes
are all built from the same three filter types above and are reproduced in full in the
fetched page; not duplicated here since they're mechanical recombinations of §3.4's
element set.

## 5. VOD workflow — full request/response example (`vod-workflow-method-v2.html`)

*Request* (clear keys, one audio key + one video key, FairPlay+Widevine+PlayReady all
signalled, condensed to omit repeated boilerplate — full version fetched and archived in
this doc's source citation above):

```xml
<cpix:CPIX contentId="abc123" version="2.3" xmlns:cpix="urn:dashif:org:cpix" xmlns:pskc="urn:ietf:params:xml:ns:keyprov:pskc">
    <cpix:ContentKeyList>
        <cpix:ContentKey explicitIV="OFj2IjCsPJFfMAxmQxLGPw==" kid="98ee5596-cd3e-a20d-163a-e382420c6eff" commonEncryptionScheme="cbcs"></cpix:ContentKey>
        <cpix:ContentKey explicitIV="L6jzdXrXAFbCJGBuMrrKrG==" kid="53abdba2-f210-43cb-bc90-f18f9a890a02" commonEncryptionScheme="cbcs"></cpix:ContentKey>
    </cpix:ContentKeyList>
    <cpix:DRMSystemList>
        <!-- FairPlay -->
        <cpix:DRMSystem kid="98ee5596-cd3e-a20d-163a-e382420c6eff" systemId="94ce86fb-07ff-4f43-adb8-93d2fa968ca2">
            <cpix:HLSSignalingData playlist="media"></cpix:HLSSignalingData>
            <cpix:HLSSignalingData playlist="master"></cpix:HLSSignalingData>
        </cpix:DRMSystem>
        <!-- (repeated per key for FairPlay/Widevine/PlayReady; Widevine+PlayReady also
             carry empty ContentProtectionData/PSSH[/SmoothStreamingProtectionHeaderData
             for PlayReady] placeholders the key provider is expected to fill in) -->
    </cpix:DRMSystemList>
    <cpix:ContentKeyUsageRuleList>
        <cpix:ContentKeyUsageRule kid="98ee5596-cd3e-a20d-163a-e382420c6eff" intendedTrackType="VIDEO">
            <cpix:VideoFilter />
        </cpix:ContentKeyUsageRule>
        <cpix:ContentKeyUsageRule kid="53abdba2-f210-43cb-bc90-f18f9a890a02" intendedTrackType="AUDIO">
            <cpix:AudioFilter />
        </cpix:ContentKeyUsageRule>
    </cpix:ContentKeyUsageRuleList>
</cpix:CPIX>
```

The **response** has the identical shape but with every `HLSSignalingData`/
`ContentProtectionData`/`PSSH`/`SmoothStreamingProtectionHeaderData` element filled in
(base64 payloads, elided with `[...]` on the AWS page) and the `ContentKey/Data`
sub-elements populated with the actual key material:

```xml
<cpix:ContentKey explicitIV="OFj2IjCsPJFfMAxmQxLGPw==" kid="98ee5596-cd3e-a20d-163a-e382420c6eff" commonEncryptionScheme="cbcs">
    <cpix:Data>
        <pskc:Secret>
            <pskc:PlainValue>5dGAgwGuUYu4dHeHtNlxJw==</pskc:PlainValue>
        </pskc:Secret>
    </cpix:Data>
</cpix:ContentKey>
```

I.e.: **the encryptor sends the shape it wants filled (keys, DRM system list with empty
signalling placeholders, and the usage-rule contract); the key provider echoes the same
document back with every placeholder populated.** This request/response symmetry is the
core of the SPEKE exchange and is a useful design invariant for a future `cpix` crate:
"parse a request CPIX, and produce a response CPIX with the same structure but
populated" is a natural API split (`CpixDocument::request_shape()` /
`CpixDocument::fill_response(...)` or similar — a design decision for the implementation
phase, not resolved here).

## 6. Content key encryption (`content-key-encryption-v2.html`)

Optional (transport-layer + AWS SigV4 auth alone is an acceptable minimum): the
encryptor can ask the key provider to encrypt content keys for delivery, which uses
CPIX's `DeliveryDataList`/`DocumentKey`/`MACMethod` mechanism (§9.1–9.2 of
`cpix-2.3-dashif.md`) unchanged, with the same "no XMLDSIG, 2048-bit RSA minimum"
restriction as everywhere else in SPEKE (§2.3 above).

Request-side `DeliveryDataList` (just the certificate — the encryptor's request has no
`DocumentKey`/`MACMethod`, since it doesn't know the Document Key yet):

```xml
<cpix:DeliveryDataList>
    <cpix:DeliveryData id="<ORIGIN SERVER ID>">
        <cpix:DeliveryKey>
            <ds:X509Data>
                <ds:X509Certificate><X.509 CERTIFICATE, BASE-64 ENCODED></ds:X509Certificate>
            </ds:X509Data>
        </cpix:DeliveryKey>
    </cpix:DeliveryData>
</cpix:DeliveryDataList>
```

Response-side adds the encrypted `DocumentKey` + `MACMethod`:

```xml
<cpix:DeliveryData id="<ORIGIN SERVER ID>">
    <cpix:DeliveryKey>
        <ds:X509Data>
            <ds:X509Certificate><X.509 CERTIFICATE, BASE-64 ENCODED></ds:X509Certificate>
        </ds:X509Data>
    </cpix:DeliveryKey>
    <cpix:DocumentKey Algorithm="http://www.w3.org/2001/04/xmlenc#aes256-cbc">
        <cpix:Data>
            <pskc:Secret>
                <pskc:EncryptedValue>
                    <enc:EncryptionMethod Algorithm="http://www.w3.org/2001/04/xmlenc#rsa-oaep-mgf1p" />
                    <enc:CipherData>
                        <enc:CipherValue><RSA CIPHER VALUE></enc:CipherValue>
                    </enc:CipherData>
                </pskc:EncryptedValue>
                <pskc:ValueMAC>qnei/5TsfUwDu+8bhsZrLjDRDngvmnUZD2eva7SfXWw=</pskc:ValueMAC>
            </pskc:Secret>
        </cpix:Data>
    </cpix:DocumentKey>
    <cpix:MACMethod Algorithm="http://www.w3.org/2001/04/xmldsig-more#hmac-sha512">
        <cpix:Key>
            <pskc:EncryptedValue>
                <enc:EncryptionMethod Algorithm="http://www.w3.org/2001/04/xmlenc#rsa-oaep-mgf1p" />
                <enc:CipherData>
                    <enc:CipherValue><RSA CIPHER VALUE></enc:CipherValue>
                </enc:CipherData>
            </pskc:EncryptedValue>
            <pskc:ValueMAC>DGqdpHUfFKxdsO9+EWrPjtdTCVfjPLwwtzEcFC/j0xY=</pskc:ValueMAC>
        </cpix:Key>
    </cpix:MACMethod>
</cpix:DeliveryData>
```

The `Algorithm` URIs used here (`aes256-cbc`, `rsa-oaep-mgf1p`, `hmac-sha512`) are the
[XMLENC-CORE]/[XMLDSIG-CORE] standard URIs and match the CPIX §6.1.4 mandatory-algorithm
table (`cpix-2.3-dashif.md` §9.4) exactly — `rsa-oaep-mgf1p` is XMLENC's registered name
for RSA-OAEP with MGF1-SHA1, so this is not a SPEKE-specific deviation.

`ContentKeyList` then carries either `pskc:EncryptedValue` (encrypted) or
`pskc:PlainValue` (clear) for each key's `Data/Secret` — both forms shown on the AWS
page and reproduced in `cpix-2.3-dashif.md` §4.

## 7. SPEKE v1 — pointers only (not separately transcribed in full)

v1 (`the-speke-api.html` + linked subpages) is the predecessor: same CPIX-over-REST
shape, but:

- Cites the older `DASH-IF-CPIX-v2-0.pdf` spec, not CPIX 2.3.
- Uses the now-deprecated SPEKE-namespace tags (`SPEKE:ProtectionHeader`, etc. — §2.2
  above) instead of the native CPIX equivalents.
- Has a Heartbeat API (§ removed entirely in v2 — not investigated further here since
  it's dead in the target version).
- No `X-Speke-Version`/`CPIX@version` cross-check — a v1 request is exactly what a v2
  key provider recognises as "legacy" per §2.1 above.

Since this project would target v2 semantics for any new key-server integration (v1 is
maintained only for existing deployments, per AWS's own "existing implementations don't
need to change" framing), the v1-specific pages were not transcribed field-by-field;
revisit only if a real v1-only key-provider integration is required.

## 8. What is NOT SPEKE-specific and is already covered elsewhere in this workspace

- The `pssh` box layout, DRM system UUIDs, PlayReady `WRMHEADER`, and Widevine protobuf
  referenced throughout this document are `transmux`'s, in `transmux/docs/drm/pssh.md`
  — not re-derived here.
- HLS `EXT-X-KEY`/`EXT-X-SESSION-KEY` tag grammar (the actual text a `HLSSignalingData`
  element's base64 payload decodes to) is `transmux/docs/drm/hls-sample-aes.md` §9 — not
  re-derived here.
