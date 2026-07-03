# HLS Sample AES Encryption — Byte Layouts

> Source: Apple "MPEG-2 Stream Encryption Format for HTTP Live Streaming"  
> https://developer.apple.com/library/archive/documentation/AudioVideo/Conceptual/HLS_Sample_Encryption/Encryption/Encryption.html  
> https://developer.apple.com/library/archive/documentation/AudioVideo/Conceptual/HLS_Sample_Encryption/

---

## 1. Overview

HLS defines two encryption modes:
- **AES-128** (`METHOD=AES-128`): full-segment encryption. Every segment is AES-128-CBC encrypted as a whole.
- **SAMPLE-AES** (`METHOD=SAMPLE-AES`): sample-level encryption. Each audio/video NAL/frame has a clear header region, with the remainder encrypted in 16-byte AES-128-CBC blocks using a 10%-skip pattern for video.

Both use AES-128-CBC with PKCS7-like handling (actually no padding — partial trailing blocks are left in the clear).

---

## 2. AES-128 (Full-Segment) Mode

```
EXT-X-KEY:METHOD=AES-128,URI="https://keyserver.example.com/key",IV=0x00000000000000000000000000000001
```

- **Cipher:** AES-128-CBC over entire MPEG-2 TS segment.
- **Key:** 128-bit (16 bytes), fetched from `URI` as raw binary.
- **IV:** 128 bits (16 bytes).
  - If `IV` attribute present: use that literal 128-bit hex value.
  - If `IV` attribute absent: IV = media sequence number as a 128-bit big-endian integer (zero-padded to 16 bytes). E.g., sequence 5 → `00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 05`.
- **Padding:** CBC with PKCS#7 padding applied to the final block.
- **Granularity:** entire TS segment is one CBC ciphertext; IV does not reset between TS packets.

---

## 3. SAMPLE-AES — H.264 Video

Reference: Apple HLS Sample Encryption spec §2.

### 3.1 Which NAL types are encrypted

Only **NAL unit types 1 and 5** (coded slice / IDR) with length **> 48 bytes** are encrypted. All other NAL types (SPS, PPS, SEI, etc.) are **never encrypted** and passed through unmodified.

NAL units of types 1 or 5 that are ≤ 48 bytes long are also **not encrypted** (left entirely in the clear).

### 3.2 Encrypted NAL unit structure

```
Encrypted_nal_unit {
    nal_unit_type_byte      [1 byte]   CLEAR — the NAL header byte
    unencrypted_leader      [31 bytes] CLEAR — first 31 bytes of NAL payload
    // total clear prefix = 32 bytes
    while (bytes_remaining() > 0) {
        if (bytes_remaining() > 16) {
            encrypted_block [16 bytes] ENCRYPTED — AES-128-CBC
        }
        unencrypted_block   [min(144, bytes_remaining()) bytes]  CLEAR
    }
}
```

Field summary:

| Region | Size | Status |
|--------|------|--------|
| NAL unit type byte | 1 byte | Clear |
| Unencrypted leader | 31 bytes | Clear |
| **Total clear prefix** | **32 bytes** | **Clear** |
| Encrypted blocks | 16 bytes each | Encrypted (AES-128-CBC) |
| Unencrypted skip blocks | up to 144 bytes (9 × 16) | Clear |
| Trailing partial block | 0–15 bytes | Clear (no padding applied) |

### 3.3 Skip pattern (10% encryption)

After each 16-byte encrypted block, up to 144 bytes (9 blocks of 16) are left unencrypted. The ratio is approximately 1 encrypted block per 10 blocks = 10% encryption.

```
[32 clear] [16 enc] [144 clear] [16 enc] [144 clear] … [0-15 clear trailer]
```

### 3.4 IV handling for H.264

- The IV is **reset to its original value** at the start of each new protected block.
- IV does NOT carry over from one protected block to the next.
- This means each 16-byte encrypted block within a single NAL uses the same IV independently — each encrypted block is independently decryptable.

### 3.5 Emulation prevention bytes

- Emulation prevention bytes (`00 00 03`) are first **removed** (unescaping) from the raw NAL byte stream before applying the skip-encrypt pattern.
- After encryption, emulation prevention bytes are **re-inserted** (re-escaping) over the entire NAL unit if any encryption occurred.
- The emulation prevention pass operates on the entire final NAL, not just the encrypted regions.

### 3.6 NAL framing in TS

In MPEG-2 TS, H.264 is carried as an Annex B byte stream (with `00 00 00 01` or `00 00 01` start codes). The encryption applies to the NAL unit payload bytes after the start code.

---

## 4. SAMPLE-AES — AAC Audio

### 4.1 Encrypted ADTS frame structure

```
Encrypted_AAC_Frame {
    ADTS_Header             [7 or 9 bytes]  CLEAR — ADTS sync word + header
    unencrypted_leader      [16 bytes]      CLEAR
    // total clear prefix = 23–25 bytes
    while (bytes_remaining() >= 16) {
        encrypted_block     [16 bytes]      ENCRYPTED — AES-128-CBC
    }
    unencrypted_trailer     [0–15 bytes]    CLEAR — partial final block
}
```

| Region | Size | Status |
|--------|------|--------|
| ADTS header | 7 bytes (no CRC) or 9 bytes (with CRC) | Clear |
| Unencrypted leader | 16 bytes | Clear |
| **Total clear prefix** | **23–25 bytes** | **Clear** |
| Encrypted blocks | 16 bytes each (integer multiple) | Encrypted (AES-128-CBC) |
| Trailing partial block | 0–15 bytes | Clear |

### 4.2 IV for AAC

- IV is reset to its original value at the start of each ADTS frame's encrypted region.
- No emulation prevention applies to audio.

---

## 5. SAMPLE-AES — AC-3 (Dolby Digital)

```
Encrypted_AC3_Frame {
    unencrypted_leader      [16 bytes]  CLEAR — starts with syncinfo header
    while (bytes_remaining() >= 16) {
        encrypted_block     [16 bytes]  ENCRYPTED
    }
    unencrypted_trailer     [0–15 bytes] CLEAR
}
```

- IV reset at each AC-3 frame boundary.
- No emulation prevention.

---

## 6. SAMPLE-AES — Enhanced AC-3 (E-AC-3)

```
Encrypted_Enhanced_AC3_syncframe {
    unencrypted_leader      [16 bytes]  CLEAR
    while (bytes_remaining() >= 16) {
        encrypted_block     [16 bytes]  ENCRYPTED
    }
    unencrypted_trailer     [0–15 bytes] CLEAR
}
```

- IV is **NOT** reset at syncframe boundaries within an audio frame.
- IV is reset only at the **beginning of each E-AC-3 audio frame**.
- One audio frame may contain multiple syncframes with continuous IV state across them.

---

## 7. Summary: Clear/Encrypted Region Table

| Stream Type | Clear prefix | Trigger for encryption | Encrypted block | Trailer |
|---|---|---|---|---|
| H.264 (NAL type 1 or 5) | 32 bytes (1 NAL hdr + 31 payload) | NAL length > 48 bytes | 16 bytes, ~10% skip pattern | 0–15 bytes clear |
| AAC (ADTS) | 23–25 bytes (ADTS hdr + 16) | Frame length sufficient | 16-byte multiples | 0–15 bytes clear |
| AC-3 | 16 bytes | Frame length sufficient | 16-byte multiples | 0–15 bytes clear |
| Enhanced AC-3 | 16 bytes per syncframe | Frame length sufficient | 16-byte multiples | 0–15 bytes clear |

---

## 8. Audio Setup Information Structure

Required for muxing in MP4/fragmented MP4 with SAMPLE-AES:

```
audio_setup_information {
    audio_type          [4 bytes]  ASCII format identifier (see table)
    priming             [2 bytes]  encoder priming value (or 0x0000)
    version             [1 byte]  = 0x01
    setup_data_length   [1 byte]  length of following setup_data
    setup_data          [N bytes] codec-specific setup data
}
```

**Audio format identifiers (4-byte ASCII):**

| Format | Identifier |
|--------|-----------|
| AAC-LC | `'zaac'` = 0x7A616163 |
| AAC-HE v1 | `'zach'` = 0x7A616368 |
| AAC-HE v2 | `'zacp'` = 0x7A616370 |
| AC-3 | `'zac3'` = 0x7A616333 |
| Enhanced AC-3 | `'zec3'` = 0x7A656333 |

---

## 9. EXT-X-KEY Tag Parameters

### AES-128 (full segment):

```m3u8
#EXT-X-KEY:METHOD=AES-128,URI="https://keyserver.example.com/key",IV=0xAAAAAAAAAAAAAAAABBBBBBBBBBBBBBBB
```

- `METHOD=AES-128`
- `URI`: HTTPS URL returning raw 16-byte AES key
- `IV`: optional 128-bit hex (with `0x` prefix); if absent, IV = media sequence number as u128 BE zero-padded

### SAMPLE-AES with Apple FairPlay (FPS):

```m3u8
#EXT-X-KEY:METHOD=SAMPLE-AES,URI="skd://asset-identifier",KEYFORMAT="com.apple.streamingkeydelivery",KEYFORMATVERSIONS="1"
```

| Attribute | Value | Notes |
|-----------|-------|-------|
| `METHOD` | `SAMPLE-AES` | Required |
| `URI` | `skd://<asset-id>` | `skd://` = streaming key delivery; asset-id is application-defined |
| `KEYFORMAT` | `"com.apple.streamingkeydelivery"` | Identifies FairPlay key delivery |
| `KEYFORMATVERSIONS` | `"1"` | Version of the KEYFORMAT |
| `IV` | optional | If absent: derived from media sequence number |

### SAMPLE-AES with W3C ClearKey (for testing):

```m3u8
#EXT-X-KEY:METHOD=SAMPLE-AES,URI="data:text/plain;base64,<base64-key>",KEYFORMAT="identity"
```

### SAMPLE-AES with Widevine:

```m3u8
#EXT-X-KEY:METHOD=SAMPLE-AES,URI="data:text/plain;base64,<widevine-pssh-b64>",KEYFORMAT="urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed",KEYFORMATVERSIONS="1"
```

The `URI` carries the base64-encoded Widevine PSSH box.

---

## 10. IV Derivation — General Rule

For `METHOD=SAMPLE-AES` when no `IV` attribute is present:
- IV = media sequence number as a **128-bit big-endian unsigned integer** (zero-padded to 16 bytes).
- Example: sequence number 7 → `00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 07`

When `IV` is explicitly set:
- The literal 128-bit value is used for all segments/samples in that key block.
- For SAMPLE-AES video/audio, the IV is reset per NAL/frame as described in §3.4/§4.2.

---

## 11. Cipher Details

- **Algorithm:** AES-128-CBC (NIST SP 800-38A)
- **Key length:** 128 bits (16 bytes)
- **Block size:** 16 bytes
- **Padding:** None (partial trailing blocks left in the clear, not padded)
- **CBC chaining:** Each encrypted block uses the previous ciphertext block as IV; first block uses the IV parameter. BUT: for SAMPLE-AES video/audio, the IV resets to its original value at each NAL/frame boundary (not a continuous chain across the whole segment).
