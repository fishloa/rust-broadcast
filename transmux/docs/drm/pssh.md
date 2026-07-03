# DRM PSSH Payloads — Multi-DRM Byte Layouts

> Spec sources:
> - PlayReady: https://learn.microsoft.com/en-us/playready/specifications/playready-header-specification  
>   (also raw MD: https://raw.githubusercontent.com/MicrosoftDocs/PlayReady/main/Docs/Specifications/playready-header-specification.md)
> - Widevine proto: https://raw.githubusercontent.com/shaka-project/shaka-packager/main/packager/media/base/widevine_pssh_data.proto
> - DRM System IDs: https://dashif.org/identifiers/content_protection/
> - ISO BMFF PSSH box: ISO/IEC 14496-12 §8.1.1 (CENC common encryption)

---

## 1. DRM System IDs (16-byte UUIDs)

These appear in the PSSH box `SystemID` field (16 bytes, big-endian UUID byte order).

| DRM System | UUID (hyphenated) | Raw 16 bytes (hex) |
|---|---|---|
| Widevine | `edef8ba9-79d6-4ace-a3c8-27dcd51d21ed` | `ED EF 8B A9 79 D6 4A CE A3 C8 27 DC D5 1D 21 ED` |
| PlayReady | `9a04f079-9840-4286-ab92-e65be0885f95` | `9A 04 F0 79 98 40 42 86 AB 92 E6 5B E0 88 5F 95` |
| FairPlay (Apple) | `94ce86fb-07ff-4f43-adb8-93d2fa968ca2` | `94 CE 86 FB 07 FF 4F 43 AD B8 93 D2 FA 96 8C A2` |
| W3C Common/ClearKey (PSSH box) | `1077efec-c0b2-4d02-ace3-3c1e52e2fb4b` | `10 77 EF EC C0 B2 4D 02 AC E3 3C 1E 52 E2 FB 4B` |
| ClearKey DASH-IF | `e2719d58-a985-b3c9-781a-b030af78d30e` | `E2 71 9D 58 A9 85 B3 C9 78 1A B0 30 AF 78 D3 0E` |
| ClearKey AES-128 | `3ea8778f-7742-4bf9-b18b-e834b2acbd47` | `3E A8 77 8F 77 42 4B F9 B1 8B E8 34 B2 AC BD 47` |
| ClearKey SAMPLE-AES | `be58615b-19c4-4684-88b3-c8c57e99e957` | `BE 58 61 5B 19 C4 46 84 88 B3 C8 C5 7E 99 E9 57` |
| Nagra MediaAccess PRM 3.0 | `adb41c24-2dbf-4a6d-958b-4457c0d27b95` | `AD B4 1C 24 2D BF 4A 6D 95 8B 44 57 C0 D2 7B 95` |
| Marlin Adaptive Streaming | `5e629af5-38da-4063-8977-97ffbd9902d4` | `5E 62 9A F5 38 DA 40 63 89 77 97 FF BD 99 02 D4` |

Note: all UUIDs in the PSSH box use **network byte order (big-endian)**. PlayReady stores its own KID internally in a different byte order — see §3 below.

---

## 2. ISO BMFF PSSH Box Layout

The container box wrapping DRM-specific init data (ISO/IEC 14496-12 §8.1.1):

```
Box header:
  [4 bytes] size          — u32 BE, total box size including header
  [4 bytes] box type      — ASCII "pssh"
  [1 byte]  version       — 0 or 1
  [3 bytes] flags         — typically 0x000000

version 0:
  [16 bytes] SystemID     — DRM UUID (big-endian)
  [4 bytes]  DataSize     — u32 BE
  [DataSize] Data         — DRM-specific payload

version 1 (adds KID list before Data):
  [16 bytes] SystemID     — DRM UUID (big-endian)
  [4 bytes]  KID_count    — u32 BE
  [16*N]     KIDs         — N × 16-byte key IDs (CENC big-endian UUID order)
  [4 bytes]  DataSize     — u32 BE
  [DataSize] Data         — DRM-specific payload
```

Total minimum size (version 0, no data): 32 bytes.

---

## 3. PlayReady — PRO + WRMHEADER

### 3.1 PlayReady Object (PRO) Binary Layout

The `Data` field of the PlayReady PSSH box contains a **PlayReady Object (PRO)**:

```
Offset  Size  Type      Field
0       4     DWORD LE  Length — total PRO size in bytes (≤ 15 KB)
4       2     WORD  LE  PlayReady Object Record Count
6       …     BYTE[]    Array of PlayReady Object Records
```

Each **PlayReady Object Record**:

```
Offset  Size  Type      Field
0       2     WORD  LE  Record Type
2       2     WORD  LE  Record Length — size of Record Value in bytes
4       …     BYTE[]    Record Value
```

**Record Type values:**

| Value  | Meaning |
|--------|---------|
| 0x0001 | PlayReady Header (PRH) — contains WRMHEADER XML encoded as UTF-16LE |
| 0x0002 | Reserved |
| 0x0003 | Embedded License Store (ELS) — empty store recommended at 10 KB |

### 3.2 PlayReady Header (PRH) — WRMHEADER XML

The Record Value for type 0x0001 is the WRMHEADER XML encoded as **UTF-16LE** (no BOM, no `<?xml?>` declaration).

Namespace: `http://schemas.microsoft.com/DRM/2007/03/PlayReadyHeader`

#### Syntax Requirements (all versions 4.x)
- Canonicalized (W3C Canonical XML v1.1)
- All element/attribute names are **case-sensitive** and UPPERCASE
- All XML nodes must use **explicit closing tags** (no self-closing `/>`)
- Namespace attributes before non-namespace attributes
- All attributes in **alphabetical order** (critical: `ALGID` before `CHECKSUM` before `VALUE`)

#### v4.0.0.0 (PlayReady 1.x, 2008)

Single key. ALGID and KEYLEN in PROTECTINFO. KID at DATA level.

```xml
<WRMHEADER xmlns="http://schemas.microsoft.com/DRM/2007/03/PlayReadyHeader" version="4.0.0.0">
  <DATA>
    <PROTECTINFO>
      <ALGID>AESCTR</ALGID>        <!-- or COCKTAIL -->
      <KEYLEN>16</KEYLEN>          <!-- 16 for AESCTR, 7 for COCKTAIL -->
    </PROTECTINFO>
    <KID>q5HgCTj40kGeNVhTH9Gexw==</KID>   <!-- base64(16-byte GUID, LE) -->
    <CHECKSUM>w+OZVr8vzrQ=</CHECKSUM>       <!-- optional from PR SDK 1.5+ -->
    <LA_URL>https://rm.example.com/rightsmanager.asmx</LA_URL>
    <LUI_URL>https://rm.example.com/acquire</LUI_URL>
    <DS_ID>AH+03juKbUGbHl1V/QIwRA==</DS_ID>
    <CUSTOMATTRIBUTES>...</CUSTOMATTRIBUTES>
  </DATA>
</WRMHEADER>
```

#### v4.1.0.0 (PlayReady 2.x, September 2011)

KID moved inside PROTECTINFO as an element with attributes. KEYLEN removed. DECRYPTORSETUP added.

```xml
<WRMHEADER xmlns="http://schemas.microsoft.com/DRM/2007/03/PlayReadyHeader" version="4.1.0.0">
  <DATA>
    <PROTECTINFO>
      <KID ALGID="AESCTR" CHECKSUM="base64val" VALUE="base64-guid"></KID>
    </PROTECTINFO>
    <LA_URL>https://...</LA_URL>
    <DS_ID>base64-guid</DS_ID>
    <DECRYPTORSETUP>ONDEMAND</DECRYPTORSETUP>  <!-- optional -->
  </DATA>
</WRMHEADER>
```

KID attributes (alphabetical order required): `ALGID` (required: "AESCTR" or "COCKTAIL"), `CHECKSUM` (optional), `VALUE` (required).

#### v4.2.0.0 (PlayReady 3.x, April 2015)

Multiple KIDs supported. PROTECTINFO now contains `<KIDS>` container.

```xml
<WRMHEADER xmlns="http://schemas.microsoft.com/DRM/2007/03/PlayReadyHeader" version="4.2.0.0">
  <DATA>
    <PROTECTINFO>
      <KIDS>
        <KID ALGID="AESCTR" CHECKSUM="xNvWVxoWk04=" VALUE="0IbHou/5s0yzM80yOkKEpQ=="></KID>
        <KID ALGID="AESCTR" CHECKSUM="GnKaQIRacPU=" VALUE="/qgG2xbs4k2SKCxx6bhWqw=="></KID>
      </KIDS>
    </PROTECTINFO>
    <LA_URL>https://...</LA_URL>
    <DS_ID>AH+03juKbUGbHl1V/QIwRA==</DS_ID>
  </DATA>
</WRMHEADER>
```

#### v4.3.0.0 (PlayReady 4.x, July 2017)

Adds AESCBC support. ALGID optional in license requests. LICENSEREQUESTED attribute.

```xml
<WRMHEADER xmlns="http://schemas.microsoft.com/DRM/2007/03/PlayReadyHeader" version="4.3.0.0">
  <DATA>
    <PROTECTINFO LICENSEREQUESTED="true">
      <KIDS>
        <KID ALGID="AESCBC" VALUE="PV1LM/VEVk+kEOB8qqcWDg=="></KID>
        <!-- CHECKSUM must be OMITTED when ALGID="AESCBC" -->
        <KID ALGID="AESCTR" CHECKSUM="base64val" VALUE="tuhDoKUN7EyxDPtMRNmhyA=="></KID>
      </KIDS>
    </PROTECTINFO>
    <LA_URL>https://...</LA_URL>
    <DS_ID>AH+03juKbUGbHl1V/QIwRA==</DS_ID>
    <DECRYPTORSETUP>ONDEMAND</DECRYPTORSETUP>
  </DATA>
</WRMHEADER>
```

Rules for v4.3:
- If ALGID is present in any KID when multiple KIDs exist, all KIDs must have ALGID and they must all match.
- ALGID="AESCBC" → **no CHECKSUM** attribute.
- ALGID="AESCTR" → CHECKSUM optional but recommended.

### 3.3 KID Byte-Order: PlayReady GUID (LE) vs CENC UUID (BE)

**This is the critical interop trap.**

PlayReady stores the KID as the base64 encoding of a Windows GUID in **little-endian memory layout**:

```
Windows GUID memory layout:
  [4 bytes] Data1  — DWORD, stored little-endian in memory
  [2 bytes] Data2  — WORD,  stored little-endian in memory
  [2 bytes] Data3  — WORD,  stored little-endian in memory
  [8 bytes] Data4  — BYTE array, stored as-is (big-endian / network order)
```

CENC (ISO 14496-12) and Widevine use the KID as a standard RFC 4122 UUID in **big-endian (network) byte order** across all 16 bytes.

**Conversion: PlayReady KID bytes → CENC UUID bytes**

Given PlayReady KID raw bytes `B[0..15]` (after base64 decode):

```
CENC[0]  = B[3]   \
CENC[1]  = B[2]    | byte-swap Data1 (DWORD)
CENC[2]  = B[1]    |
CENC[3]  = B[0]   /
CENC[4]  = B[5]   \  byte-swap Data2 (WORD)
CENC[5]  = B[4]   /
CENC[6]  = B[7]   \  byte-swap Data3 (WORD)
CENC[7]  = B[6]   /
CENC[8..15] = B[8..15]   (Data4 — no swap)
```

**Example:**

PlayReady KID VALUE attribute: `"PV1LM/VEVk+kEOB8qqcWDg=="`

After base64 decode (16 bytes hex):
```
3D 5D 4B 33  F5 44  56 4F  A4 10 E0 7C AA A7 16 0E
```

Applying the swap:
- Data1 swap: `33 4B 5D 3D` → bytes at positions 3,2,1,0 → CENC[0..3] = `33 4B 5D 3D`
- Data2 swap: `F5 44` → `44 F5` → CENC[4..5] = `44 F5`
- Data3 swap: `56 4F` → `4F 56` → CENC[6..7] = `4F 56`  
- Data4 unchanged: `A4 10 E0 7C AA A7 16 0E`

CENC UUID: `334B5D3D-44F5-4F56-A410-E07CAAA7160E`

**Rule of thumb:** When building a multi-DRM packager, store the canonical KID as a CENC big-endian UUID, and swap to/from PlayReady LE-GUID format only at the PSSH/WRMHEADER boundary.

### 3.4 Key Checksum Algorithm

**AESCTR:**
1. Encrypt the 16-byte KID (in PlayReady LE-GUID bytes) with the 16-byte content key using AES-128-ECB.
2. Take the first 8 bytes of the result.
3. Base64-encode → the `CHECKSUM` attribute value.

**COCKTAIL:**
1. Create a 21-byte buffer: content key (7 bytes) padded with zeros to 21 bytes.
2. Apply SHA-1 five times iteratively: `buf = SHA1(buf)`.
3. Take the first 7 bytes.
4. Base64-encode → CHECKSUM.

**AESCBC:** No checksum. Omit the `CHECKSUM` attribute entirely.

---

## 4. Widevine — `WidevineCencHeader` Protobuf

Source: https://raw.githubusercontent.com/shaka-project/shaka-packager/main/packager/media/base/widevine_pssh_data.proto

The PSSH `Data` field for Widevine contains a serialized `WidevineCencHeader` protobuf (also called `WidevinePsshData`):

```proto
syntax = "proto2";

message WidevineCencHeader {
  enum Algorithm {
    UNENCRYPTED = 0;
    AESCTR      = 1;
  }

  // Wire type encodings (standard protobuf):
  //   varint        = wire type 0
  //   length-delimited = wire type 2

  optional Algorithm algorithm = 1;         // varint, wire type 0
  repeated bytes     key_id    = 2;         // length-delimited, wire type 2
                                            //   each key_id = 16 bytes CENC UUID (big-endian)
  optional string    provider  = 3;         // length-delimited, wire type 2
  optional bytes     content_id = 4;        // length-delimited, wire type 2
  optional string    policy    = 6;         // length-delimited, wire type 2
  optional uint32    crypto_period_index = 7;  // varint, wire type 0
  optional bytes     grouped_license = 8;   // length-delimited, wire type 2
  optional uint32    protection_scheme = 9; // varint, wire type 0
                                            //   'cenc' = 0x63656E63
                                            //   'cbcs' = 0x63626373
}

// Alternative outer message in some tools:
message WidevineHeader {
  repeated string key_ids  = 2;  // hex-encoded key IDs as strings
  optional string provider = 3;
  optional bytes  content_id = 4;
}
```

**Field number → wire encoding table:**

| Field # | Name | Wire Type | Type | Notes |
|---------|------|-----------|------|-------|
| 1 | algorithm | 0 (varint) | enum | 0=UNENCRYPTED, 1=AESCTR |
| 2 | key_id | 2 (len-delim) | bytes (repeated) | 16-byte CENC UUID per key |
| 3 | provider | 2 (len-delim) | string | e.g. "widevine_test" |
| 4 | content_id | 2 (len-delim) | bytes | arbitrary content identifier |
| 6 | policy | 2 (len-delim) | string | |
| 7 | crypto_period_index | 0 (varint) | uint32 | for key rotation |
| 8 | grouped_license | 2 (len-delim) | bytes | |
| 9 | protection_scheme | 0 (varint) | uint32 | FourCC: 'cenc'=0x63656E63, 'cbcs'=0x63626373 |

**Wire encoding:** standard protobuf binary encoding. Field tag = `(field_number << 3) | wire_type`.

Example: field 2 (key_id) tag byte = `(2 << 3) | 2` = `0x12`.

**Minimum valid PSSH Data** (algorithm=AESCTR + one key_id):
```
0A 01 01                     -- field 1 (algorithm), length 1, value 1 (AESCTR)
12 10 <16 key bytes>         -- field 2 (key_id), length 16, then 16 bytes
```

**Widevine key_id byte order:** Widevine uses **CENC big-endian UUID order** for key_id bytes. No endian swap needed vs CENC. (Contrast with PlayReady LE-GUID.)

---

## 5. FairPlay — Init Data

FairPlay Streaming (FPS) differs fundamentally from Widevine/PlayReady: **it has no rich public PSSH payload spec**.

### What IS public:
- Apple FPS uses an `skd://` URI scheme as its "init data" for CENC/HLS contexts.
- The URI is in the form `skd://<asset-identifier>` where the asset identifier is application-defined (often a UUID or URL-encoded content key context).
- The PSSH box for FairPlay (SystemID `94ce86fb-07ff-4f43-adb8-93d2fa968ca2`) carries only the UTF-8 encoded `skd://` URI as its `Data` payload in many packager implementations.
- There is **no public specification** for the FairPlay Server-to-Client Payload (SPC) structure — this is Apple's proprietary protocol documented only in the FairPlay Streaming SDK (NDA-gated developer program).

### What implementers can do:
- For HLS playlists: emit `#EXT-X-KEY:METHOD=SAMPLE-AES,URI="skd://<asset-id>",KEYFORMAT="com.apple.streamingkeydelivery",KEYFORMATVERSIONS="1"` (see hls-sample-aes.md §4).
- For DASH/CENC PSSH: wrap the `skd://` URI as UTF-8 bytes in a PSSH box with FairPlay system ID.
- The actual key delivery exchange (SPC/CKC messages) requires Apple's FairPlay SDK.

**Example FairPlay PSSH Data field (common packager convention):**
```
Data = UTF-8 bytes of "skd://<asset-id>"
```
No additional framing around the URI inside the PSSH `Data`.

---

## Gap summary for this section

| Item | Status |
|------|--------|
| PlayReady PRO binary format | COMPLETE (Microsoft public docs) |
| PlayReady WRMHEADER v4.0–v4.3 XML | COMPLETE |
| PlayReady KID byte-swap (GUID→UUID) | COMPLETE |
| Widevine WidevineCencHeader proto fields | COMPLETE (shaka-packager open source) |
| DRM system UUIDs | COMPLETE (DASH-IF public registry) |
| FairPlay PSSH Data content | PARTIAL — skd:// URI convention is public; SPC/CKC key exchange protocol is Apple NDA |
