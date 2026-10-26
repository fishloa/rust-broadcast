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

> **Sections 2-3 moved.** §2 (the ISO BMFF `pssh` box layout, ISO/IEC 14496-12 §8.1.1) and §3 (PlayReady Object /
> WRMHEADER layouts, KID byte order, key checksum) are transcriptions of third-party standards and vendor docs
> and live only in the private companion repo: `private/specs-verified/mpeg/transmux/drm/pssh.md` (old path
> `transmux/docs/drm/pssh.md`, see `docs/specs/PRIVATE-MAP.md`). This file keeps the DRM System ID registry
> facts (§1), the Widevine `WidevineCencHeader` proto layout (§4, derived from the Apache-2.0
> `widevine_pssh_data.proto`) and the FairPlay notes (§5).

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
