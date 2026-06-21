# Sample decryption resource (CI Plus, IP delivery Host player mode)

_Source: ETSI TS 103 205 v1.4.1 §7.4, Tables 30-39 (PDF pp. 52-60), render-verified_

The Sample decryption resource controls decryption by the CICAM of a set of
consecutive media Samples packaged into an MPEG-2 TS. Tags live in the CI Plus
`0x9F98xx` namespace.

## Table 30 — Sample decryption resource summary (PDF p. 53)

Resource Identifier `0x00920041` — Class 146, Type 1, Version 1.

| APDU Tag | Tag value | Host | CICAM |
|----------|-----------|:----:|:-----:|
| sd_info_req     | `9F 98 00` | → |   |
| sd_info_reply   | `9F 98 01` |   | → |
| sd_start        | `9F 98 02` | → |   |
| sd_start_reply  | `9F 98 03` |   | → |
| sd_update       | `9F 98 04` | → |   |
| sd_update_reply | `9F 98 05` |   | → |

(Host→CICAM for the *_req/start/update; CICAM→Host for the *_reply.)

## §7.4.2 — sd_info_req APDU — Table 31 (PDF p. 53)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `sd_info_req() {` | | |
| &nbsp;&nbsp;sd_info_req_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| `}` | | |

- **sd_info_req_tag** (24) — value `0x9F9800`.

## §7.4.3 — sd_info_reply APDU — Table 32 (PDF p. 53)

Lists `drm_system_id` and UUID for each content protection / DRM system the CICAM
supports for Sample decryption.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `sd_info_reply() {` | | |
| &nbsp;&nbsp;sd_info_reply_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | 16 | |
| &nbsp;&nbsp;number_of_drm_system_id | 8 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<n; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;drm_system_id | 16 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;number_of_drm_uuid | 8 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<n; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;drm_uuid | 128 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

⚠ `length_field()` is shown with "16" in the No.-of-bits column of Table 32 (and
likewise Table 33/38), unlike the bare `length_field()` elsewhere. Transcribed as
rendered.

Field semantics (§7.4.3, p. 54):
- **sd_info_reply_tag** (24) — value `0x9F9801`.
- **number_of_drm_system_id** (8) — number of DRM system identifiers following. This list shall not contain identifiers of broadcast CA systems also supported by the CICAM, unless that CA/DRM system supports both broadcast CA decryption and Sample decryption.
- **drm_system_id** (16) — DRM System implemented by the CICAM for which Sample decryption is supported. Values are the same as `ca_system_id` per the DVB allocation of identifiers and codes [11].
- **number_of_drm_uuid** (8) — number of DRM UUIDs following.
- **drm_uuid** (128) — UUID of a DRM system supported by the CICAM.

## §7.4.4 — sd_start APDU — Table 33 (PDF p. 55)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `sd_start() {` | | |
| &nbsp;&nbsp;sd_start_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;LTS_id | 8 | uimsbf |
| &nbsp;&nbsp;program_number | 16 | uimsbf |
| &nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;ts_flag | 1 | uimsbf |
| &nbsp;&nbsp;`if (ts_flag == 1) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;number_of_metadata_records | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i=0; i<number_of_metadata_records; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;drm_metadata_source | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;drm_system_id | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;drm_uuid | 128 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;drm_metadata_length | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`for (i=0; i<N; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;drm_metadata_byte | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`if (ts_flag == 0) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;number_of_sample_tracks | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i=0; i<number_of_sample_tracks; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 3 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;track_PID | 13 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;number_of_metadata_records | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`for (i=0; i<number_of_metadata_records; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;drm_metadata_source | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;drm_system_id | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;drm_uuid | 128 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;drm_metadata_length | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`for (i=0; i<N; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;drm_metadata_byte | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field semantics (§7.4.4, pp. 55-56):
- **sd_start_tag** (24) — value `0x9F9802`.
- **LTS_id** (8) — identifier of the Local TS.
- **program_number** (16) — used by the CICAM in URI messages. For non-TS content the Host may set it to any value.
- **ts_flag** (1) — `0b1` = the request is for a TS; `0b0` = non-TS format.
- **number_of_Sample_Tracks** (8) — number of Sample Tracks to be decrypted.
- **track_PID** (13) — PID on which the Samples for the described Sample Track are sent by the Host. Used as the Track identifier in subsequent `sd_update()` APDUs. Values `0x0000`–`0x001F` are reserved.
- **drm_metadata_source** (8) — source of the metadata, per Table 34.
- **drm_system_id** (16) — DRM system the metadata relates to; same values as `ca_system_id` [11]. `0xFFFF` if not used to identify the DRM.
- **drm_uuid** (128) — UUID of the DRM; all bytes `0xFF` if not used.
- **drm_metadata_length** (16) — length in bytes of the following DRM metadata.
- **drm_metadata_byte** (8) — the DRM metadata (opaque, encoded per Table 34).

### Table 34 — DRM metadata source (PDF p. 56)

| DRM Metadata source | Value |
|---------------------|-------|
| Undefined | `0x00` |
| Content Access Streaming Descriptor (CASD) — DRM Generic Data (DRMGenericData copied to drm_metadata_byte as UTF-8) | `0x01` |
| Content Access Streaming Descriptor (CASD) — DRM Private Data (DRMPrivateData copied as UTF-8) | `0x02` |
| Common Encryption (CENC) — Protection System Specific Header ('pssh') box (Data field of 'pssh' box copied) | `0x03` |
| Media Presentation Description (MPD) — Content Protection Element (ContentProtection from MPD Representation, UTF-8) | `0x04` |
| ISOBMFF — Protection Scheme Information ('sinf') box (Data field of 'sinf' box copied) | `0x05` |
| Online SDT (OSDT) — DRM Generic Data (DRMGenericData copied as UTF-8) | `0x06` |
| Online SDT (OSDT) — DRM Private Data (DRMPrivateData copied as UTF-8) | `0x07` |
| Reserved | `0x08`–`0xFF` |

The `drm_metadata_byte` bodies are **opaque** to the wire parser — DRM-system /
container-format specific blobs (pssh/sinf/CASD/MPD/OSDT data). Only the source
code, length and byte sequence are structural.

## §7.4.5 — sd_start_reply APDU — Table 35 (PDF p. 57)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `sd_start_reply() {` | | |
| &nbsp;&nbsp;sd_start_reply_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;LTS_id | 8 | uimsbf |
| &nbsp;&nbsp;transmission_status | 8 | uimsbf |
| &nbsp;&nbsp;drm_status | 8 | uimsbf |
| &nbsp;&nbsp;drm_system_id | 16 | uimsbf |
| &nbsp;&nbsp;drm_uuid | 16*8 | uimsbf |
| &nbsp;&nbsp;buffer_size | 16 | uimsbf |
| &nbsp;&nbsp;data_block_size | 16 | uimsbf |
| `}` | | |

Field semantics (§7.4.5, pp. 57-58):
- **sd_start_reply_tag** (24) — value `0x9F9803`.
- **LTS_id** (8) — identifier of the Local TS.
- **transmission_status** (8) — see Table 36.
- **drm_status** (8) — see Table 37.
- **drm_system_id** (16) — DRM system the CICAM will use; `0xFFFF` if not used.
- **drm_uuid** (`16*8` = 128 bits) — UUID of the DRM; all bytes `0xFF` if not used. ⚠ width printed as `16*8` (= 128 bits / 16 bytes), unlike the `128` used in Tables 32/33.
- **buffer_size** (16) — CICAM buffer allocated for decryption, in transport packets. Minimum returned shall be 5 000 TS packets; shared between tracks if multiple. Shall not change in a subsequent `sd_start_reply()`.
- **data_block_size** (16) — number of TS packets the Host should send to ensure previously sent data is fully processed. `0` = no transfer-size constraints.

### Table 36 — transmission_status values (PDF p. 58)

| transmission_status | Value |
|---------------------|-------|
| Ready to receive | `0x00` |
| Error - CICAM busy | `0x01` |
| Error - other reason | `0x02` |
| Reserved | `0x03`–`0xFF` |

### Table 37 — drm_status values (PDF p. 58)

| drm_status | Value |
|------------|-------|
| Decryption possible | `0x00` |
| Status currently undetermined | `0x01` |
| Error - no entitlement | `0x02` |
| Reserved | `0x03`–`0xFF` |

## §7.4.6 — sd_update APDU — Table 38 (PDF p. 59)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `sd_update() {` | | |
| &nbsp;&nbsp;sd_update_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;LTS_id | 8 | uimsbf |
| &nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;ts_flag | 1 | bslbf |
| &nbsp;&nbsp;`if (ts_flag == 1) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;number_of_metadata_records | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i=0; i<number_of_metadata_records; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;drm_metadata_source | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;drm_system_id | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;drm_uuid | 128 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;drm_metadata_length | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`for (i=0; i<N; i++) { drm_metadata_byte (8) }` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`if (ts_flag == 0) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;number_of_Sample_Tracks | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i=0; i<number_of_Sample_Tracks; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 3 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;Sample_track_PID | 13 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;number_of_metadata_records | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`for (i=0; i<number_of_metadata_records; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;drm_metadata_source (8) / drm_system_id (16) / drm_uuid (128) / drm_metadata_length (16) | | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`for (i=0; i<N; i++) { drm_metadata_byte (8) }` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field semantics (§7.4.6, pp. 59-60):
- **sd_update_tag** (24) — value `0x9F9804`.
- **LTS_id** (8) — identifier of the Local TS for which the update applies.
- **ts_flag** (1) — `0b1` if related to a TS Sample Track, else `0`. Shall have the same value as in `sd_start()`.
- **Sample_track_PID** (13) — PID on which the Samples for the described Sample Track are available.
- Remaining `drm_*` fields — as in `sd_start` / Table 34.

## §7.4.7 — sd_update_reply APDU — Table 39 (PDF p. 60)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `sd_update_reply() {` | | |
| &nbsp;&nbsp;sd_update_reply_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;LTS_id | 8 | uimsbf |
| &nbsp;&nbsp;drm_status | 8 | uimsbf |
| `}` | | |

Field semantics (§7.4.7, p. 60):
- **sd_update_reply_tag** (24) — value `0x9F9805`.
- **LTS_id** (8) — identifier of the Local TS.
- **drm_status** (8) — DRM status of the CICAM; values per Table 37.
