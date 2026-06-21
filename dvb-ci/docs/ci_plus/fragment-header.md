# CI Plus (USB) media-interface fragment header

_Source: ETSI TS 103 605 V1.1.1 §7.7, Table 3 "Fragment header syntax" (PDF p. 21), Table 4 "Message descriptors" (PDF p. 23), Table 5 "Use of fragment header fields" (PDF p. 24), render-verified_

In the second-generation (USB) Common Interface, content is carried over the
media endpoint as **Fragments**, each preceded by a **fragment header**
(TS 103 605 §7.7). The header carries the subsample clear/encrypted byte
boundaries and the crypto parameters (scrambling control, IV-reload period,
padding), plus a trailing descriptor loop that conveys the initialization
vector and key identifier.

This is the only wire-syntax table that TS 103 605 itself prints. The CI Plus
**command-interface** resource APDUs (content_control, host language/country,
operator profile, CAM upgrade, application MMI, SAC) are NOT printed in
TS 103 605 — its §6.3 only references the proprietary CI Plus specification [3]
(see notes at the bottom of this file). The freely-redistributable command APDU
layouts are in `resource-ids.md` + the per-resource files (sourced to
TS 101 699).

## Table 3 — fragment_header() syntax (PDF p. 21)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `fragment_header() {` | | |
| &nbsp;&nbsp;protocol_version | 8 | uimsbf |
| &nbsp;&nbsp;LTS_id | 8 | uimsbf |
| &nbsp;&nbsp;track_id | 8 | uimsbf |
| &nbsp;&nbsp;flush | 1 | bslbf |
| &nbsp;&nbsp;first_fragment | 1 | bslbf |
| &nbsp;&nbsp;last_fragment | 1 | bslbf |
| &nbsp;&nbsp;reserved_future_use | 5 | bslbf |
| &nbsp;&nbsp;number_subsamples | 32 | uimsbf |
| &nbsp;&nbsp;`for (i=0;i<N;i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;clear_bytes | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;encrypted_bytes | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;crypto_reload_period | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;scrambling_control | 2 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;padding_size | 6 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;padding_offset | 16 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;descriptor_length | 16 | uimsbf |
| &nbsp;&nbsp;`for (i=0;i<N;i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;descriptor() | | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Note: the subsample loop runs `number_subsamples` iterations (the spec writes
the loop bound as `N`; per §7.7.1 semantics `N` = `number_subsamples`). The
descriptor loop runs over `descriptor_length` bytes.

### Field semantics (§7.7.1, p. 21-22)

- **protocol_version** (8) — syntax version of this fragment header. For the
  current document shall be 0.
- **LTS_id** (8) — identifies an MPEG transport stream, or together with
  `track_id` a sample track. Data format (TS vs sample) is inferred from
  `LTS_id` via the command-interface resources.
- **track_id** (8) — with `LTS_id` identifies a sample track. Shall be 0 when
  the data format is MPEG transport stream.
- **flush** (1) — Host→CICAM: when 1, the CICAM removes all buffered data for
  this `LTS_id`; the CICAM acknowledges by returning the fragment header with
  flush=1 on the first fragment after the flush completes. In sample mode a
  flush=1 header means the following fragment starts a new sample.
- **first_fragment** (1) — start of a sample (sample mode). Set 1 = following
  fragment data is the start of a new sample. Always 0 for MPEG TS.
- **last_fragment** (1) — end of a sample (sample mode). Set 1 = following
  fragment data is the last data of the sample. Always 0 for MPEG TS.
- **reserved_future_use** (5).
- **number_subsamples** (32) — number of subsamples in the fragment. Shall be 0
  **if and only if** the data format is MPEG transport stream.
- **clear_bytes** (16) — size of the clear data at the start of the subsample.
- **encrypted_bytes** (16) — size of the encrypted data following the clear
  data; includes any padding added by the CICAM.
- **crypto_reload_period** (8) — number of cipher blocks of encrypted data
  after which the IV is reloaded when decrypting. 0 = IV loaded only at the
  start of the encrypted subsample. For AES the only non-zero valid value is 11;
  for DES the only non-zero valid value is 23. Only used in CICAM→Host
  transfers; for Host→CICAM shall be 0.
- **scrambling_control** (2) — which scrambling key encrypts the encrypted bytes
  in CICAM→Host Media-Sample transfers; same values as the transport scrambling
  control bits in §5.6.1 of the CI Plus specification [4]. 0 in all other cases.
- **padding_size** (6) — size in bytes of random padding added to the encrypted
  data so its length is an integral number of cipher blocks. Only used in
  CICAM→Host transfers; 0 for Host→CICAM. See §7.5.2.
- **padding_offset** (16) — number of encrypted bytes before the padding. See
  §7.5.2.
- **descriptor_length** (16) — number of descriptor bytes following. Only the
  descriptors defined in §7.7.2 have defined semantics.

Encoding rules (§7.7.1):
- a) Sample with no clear bytes → `number_subsamples`=1, `clear_bytes`=0,
  `encrypted_bytes`=size of sample.
- b) Sample with no encrypted bytes → `number_subsamples`=1, `clear_bytes`=size
  of sample, `encrypted_bytes`=0.

## Table 4 — Message descriptors (fragment-header descriptor loop) (PDF p. 23)

The descriptor loop carries zero or more descriptors. Semantics from
ETSI TS 103 205 [4] §7.5.5.4 apply. Tag allocation:

| Descriptor | Tag Value | ff=0 | ff=1 | lf=1 | lf=0 |
|------------|-----------|:----:|:----:|:----:|:----:|
| Forbidden | `0x00` | | | | |
| ciplus_initialization_vector_descriptor | `0xD0` | | * | | |
| ciplus_key_identifier_descriptor | `0xD1` | | * | | |
| Reserved | `0xD2`–`0xEF` | | | | |
| Host defined | `0xF0`–`0xFE` | * | * | * | * |
| Forbidden | `0xFF` | | | | |

NOTE: `ff` = the `first_fragment` field, `lf` = the `last_fragment` field. A "*"
means the descriptor may be included when that field has the marked value. The
IV and key-identifier descriptors are allowed only when `first_fragment` = 1.

### §7.7.2.2 / §7.7.2.3 — IV and key-identifier descriptors

- **ciplus_initialization_vector_descriptor()** (tag `0xD0`) — used by the Host
  to provide the IV associated with the following sample. Allowed only when
  `first_fragment` = 1. Its syntax is defined in ETSI TS 103 205 [4] §7.5.5.4.2,
  Table 46 (NOT printed in TS 103 605). It is a **TLV**: `descriptor_tag`(8)=`0xD0`
  + `descriptor_length`(8) + `descriptor_length`×`IV_data_byte`(8) — variable-length
  opaque IV octets (the length is runtime `descriptor_length`, not fixed). Full
  layout transcribed in [`../ts_103_205/ci-plus-descriptors.md`](../ts_103_205/ci-plus-descriptors.md).
- **ciplus_key_identifier_descriptor()** (tag `0xD1`) — used by the Host to
  provide the content key identifier for the following sample. Allowed only when
  `first_fragment` = 1. Syntax in ETSI TS 103 205 [4] §7.5.5.4.3, Table 47 (NOT
  printed in TS 103 605). Also a **TLV**: `descriptor_tag`(8)=`0xD1` +
  `descriptor_length`(8) + `descriptor_length`×`key_id_data_byte`(8) — variable-length
  opaque key-identifier octets. Full layout in
  [`../ts_103_205/ci-plus-descriptors.md`](../ts_103_205/ci-plus-descriptors.md).

A generic CI-extension descriptor envelope (used by the host-defined
`0xF0`–`0xFE` range and by the IV/key-id descriptors) is `descriptor_tag` (8) +
`descriptor_length` (8) + body bytes — confirm the exact header against
TS 103 205 §7.5.5.4 when implementing.

## Table 5 — Use of fragment header fields (informative) (PDF p. 24)

Per data type & direction (`-` = not used / don't-care):

| Field | H→CICAM MPEG TS | H→CICAM Samples | CICAM→H MPEG TS | CICAM→H Samples |
|-------|-----------------|-----------------|-----------------|-----------------|
| LTS_id | LTS_id | LTS_id | same as received | same as received |
| flush | 0 | 1 to flush, 0 otherwise | 0 | 1 to confirm flush, 0 otherwise |
| first_fragment | 0 | 0 or 1 | 0 | same as received |
| last_fragment | 0 | 0 or 1 | 0 | same as received |
| track_id | 0 | track_id | 0 | same as received |
| number_subsamples | 0 | > 0 | 0 | > 0 |
| clear_bytes | - | size of clear data | - | same as received |
| encrypted_bytes | - | size of encrypted data | - | size of encrypted bytes (padding incl.) |
| crypto_reload_period | - | 0 | - | 0, 11 or 23 |
| scrambling_control | - | 00 | - | 10 or 11 |
| padding_size | - | 0 | - | size of padding added by the CICAM |
| padding_location | - | 0 | - | ≥ 0 for CI Plus1.4 compatibility, 0 for CI PLUS 2.0 native |
| descriptors | `0xD0`, `0xD1` (see note), Host defined | `0xD0`, `0xD1`, Host defined | same as received | same as received |

NOTE (Table 5): when receiving MPEG TS packets in Host player mode using MPEG
DASH. (Table 5 names the field `padding_location`; Table 3 names the on-wire
field `padding_offset` — they refer to the same offset concept.)

---

## CI Plus command-interface APDUs — deferred by TS 103 605

TS 103 605 §6.3 ("Modification to resources and APDU") adjusts but does NOT
print the layout of these CI Plus command-interface APDUs; they are defined in
the proprietary CI Plus specification [3] / TS 103 205 [4]:

- **request_ci_cam_reset** — CI Plus spec [3] §11.1.2. Per TS 103 605 §6.3.2.1
  shall NOT be sent by the CICAM (introduced with resource version 3).
- **data_rate_info** — CI Plus spec [3] §11.1.3.1. Per §6.3.2.2 shall NOT be
  sent by the Host (introduced with resource version 3).
- **Low Speed communication** resource — CI Plus spec [3] §14.1, §14.2. Per
  §6.3.3 shall NOT be offered by the Host over USB; IP runs over the USB
  CDC-EEM Networking Interface (§8) instead.
- **cam_firmware_upgrade_complete** — CI Plus spec [3] §14.3.5.5. Per §6.3.4.1
  the `reset_request_status` field shall be ignored by the Host.

⚠ These four APDU layouts are NOT in any of the redistributable PDFs
(TS 103 605 / TS 101 699). They live in the proprietary CI Plus v1.4.x spec.
The `cam_firmware_upgrade` / content-control (`cc_*`) / SAC / host
language-country / operator-profile resources are CI-Plus-proprietary and are
NOT transcribed here — only their existence and TS 103 605 references are noted.
See the report for the re-check list.
