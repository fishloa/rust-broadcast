# Content Control resource — multi-stream extensions (CI Plus)

_Source: ETSI TS 103 205 v1.4.1 §6.4.3, Tables 6-13 (PDF pp. 34-38), render-verified_

For multi-stream reception the Content Control resource gains a new resource_type
(`0x008C1041`) in which the constituent APDUs are extended to include the Local TS
identifier (`LTS_id`). The remaining APDUs are unchanged from CI Plus V1.3
(clauses 11.3.1 / 11.3.2 of the proprietary CI Plus spec [3]). §6.4.3.3 specifies
extensions to the SAC protocol message sequences (carried inside the
`cc_sac_data_req` / `cc_sac_data_cnf` envelope) — these are protocol tables, not
new APDU layouts.

## Table 6 — Content Control resource summary (PDF p. 34)

Resource Identifier `0x008C1041` — Class 140, Type 65, Version 1.

| APDU Tag | Tag value | Host | CICAM |
|----------|-----------|:----:|:-----:|
| cc_open_req         | `9F 90 01` |   | → |
| cc_open_cnf         | `9F 90 02` | → |   |
| cc_data_req         | `9F 90 03` |   | → |
| cc_data_cnf         | `9F 90 04` | → |   |
| cc_sync_req         | `9F 90 05` |   | → |
| cc_sync_cnf         | `9F 90 06` | → |   |
| cc_sac_data_req (note 1) | `9F 90 07` | ←→ | ←→ |
| cc_sac_data_cnf (note 1) | `9F 90 08` | ←→ | ←→ |
| cc_sac_sync_req     | `9F 90 09` |   | → |
| cc_sac_sync_cnf     | `9F 90 10` | → |   |
| cc_PIN_capabilities_req   | `9F 90 11` | → |   |
| cc_PIN_capabilities_reply | `9F 90 12` |   | → |
| cc_PIN_cmd          | `9F 90 13` | → |   |
| cc_PIN_reply (note 2) | `9F 90 14` |   | → |
| cc_PIN_event (note 2) | `9F 90 15` |   | → |
| cc_PIN_playback     | `9F 90 16` | → |   |
| cc_PIN_MMI_req      | `9F 90 17` | → |   |

(Direction `→` / `←` as drawn under the Host/CICAM columns in Table 6: a tick
under "Host" with `←` means Host→ ... ; ETSI's Table 6 marks the originating side
with the arrow. `cc_sac_data_req`/`cnf` are bidirectional `←→`.)

- NOTE 1 — The APDU syntax is not extended, but the `LTS_id` field is included in certain SAC protocols, as specified in §6.4.3.3.
- NOTE 2 — This APDU is extended to include the `LTS_id` field.

## §6.4.3.2 — Content Control APDU extensions

### §6.4.3.2.1 cc_PIN_reply APDU — Table 7 (PDF p. 34)

Extended (for the record start protocol) to include `LTS_id`. Used while
multi-stream mode is active.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `cc_PIN_reply() {` | | |
| &nbsp;&nbsp;cc_PIN_reply_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;reserved | 7 | uimsbf |
| &nbsp;&nbsp;LTS_bound_flag | 1 | uimsbf |
| &nbsp;&nbsp;`if (LTS_bound_flag == 1) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;LTS_id | 8 | uimsbf |
| &nbsp;&nbsp;`} else {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;PINcode_status_field | 8 | uimsbf |
| `}` | | |

Field semantics (§6.4.3.2.1, p. 35):
- **cc_PIN_reply_tag** (24) — value `0x9F9014` identifies this APDU.
- **length_field** — ASN.1 BER, per EN 50221 §8.3.1.
- **LTS_bound_flag** (1) — `0b1` = the reply is associated with a particular Local TS; `0b0` = not associated with an `LTS_id` (e.g. when sent in response to `cc_PIN_cmd()`, `cc_PIN_playback()` or `cc_PIN_MMI_req()`).
- **LTS_id** (8) — Local TS identifier.
- **PINcode_status_field** (8) — refer to §11.3.2.3 of the CI Plus V1.3 specification [3] (not reproduced here — proprietary).

### §6.4.3.2.2 cc_PIN_event APDU — Table 8 (PDF p. 35)

Extended (for the record start protocol) to include `LTS_id`.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `cc_PIN_event() {` | | |
| &nbsp;&nbsp;cc_PIN_event_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;LTS_id | 8 | uimsbf |
| &nbsp;&nbsp;program_number | 16 | uimsbf |
| &nbsp;&nbsp;PINcode_status_field | 8 | uimsbf |
| &nbsp;&nbsp;rating | 8 | uimsbf |
| &nbsp;&nbsp;pin_event_time_utc | 40 | uimsbf |
| &nbsp;&nbsp;pin_event_time_centiseconds | 8 | uimsbf |
| &nbsp;&nbsp;private_data | 8x15 | uimsbf |
| `}` | | |

Field semantics (§6.4.3.2.2, p. 35):
- **cc_PIN_event_tag** (24) — value `0x9F9015` identifies this APDU.
- **length_field** — ASN.1 BER, per EN 50221 §8.3.1.
- **LTS_id** (8) — Local TS identifier.
- **private_data** — `8x15` bits = 15 bytes (transcribed exactly as `8x15`).
- Other fields — refer to §11.3.2.4 of the CI Plus V1.3 specification [3] (proprietary, not reproduced).

## §6.4.3.3 — Content Control protocol extensions

These tables describe the SAC message *content* (datatype loops) carried inside
the `cc_sac_data_req` / `cc_sac_data_cnf` APDUs (tags `0x9F9007` / `0x9F9008`).
They are protocol step tables, not new APDU envelopes. The new `LTS_id` datatype
uses **datatype_id 50** with `datatype_len = 8 bits`.

### §6.4.3.3.1 URI transmission and acknowledgement — Table 9 (PDF p. 36)

| Step | Action | APDU | Content (datatype_id / len) |
|------|--------|------|------------------------------|
| 1 | CICAM sends the URI to the Host | cc_sac_data_req | send_datatype_nbr = 3 → i=0: **25** (uri_message) 64 bits; i=1: **26** (program_number) 16 bits; i=2: **50** (LTS_id) 8 bits. request_datatype_nbr = 1 → i=0: **27** (uri_confirm) |
| 2 | Host sends acknowledgement to the CICAM | cc_sac_data_cnf | send_datatype_nbr = 1 → i=0: **27** (uri_confirm) 256 bits |

### §6.4.3.3.2 Record Start protocol — Table 10 (PDF p. 36)

| Step | Action | APDU | Content |
|------|--------|------|---------|
| 1 | Host informs CICAM of start of recording | cc_sac_data_req | send_datatype_nbr = 3 or 4 → i=0: **38** (operating_mode) 8 bits; i=1: **26** (program_number) 16 bits; i=2: **39** (PINcode data) variable (optional); i=3: **50** (LTS_id) 8 bits. request_datatype_nbr = 1 → i=0: **40** (record_start_status) |
| 2 | CICAM sends acknowledgement to the Host | cc_sac_data_cnf | send_datatype_nbr = 1 → i=0: **40** (record_start_status) 8 bits |

The CICAM shall consider the selected programme unattended when `operating_mode`
is `0x01` (Timeshift) or `0x02` (Unattended_Recording).

### §6.4.3.3.3 Record Stop protocol — Table 11 (PDF p. 37)

| Step | Action | APDU | Content |
|------|--------|------|---------|
| 1 | Host informs CICAM recording has stopped | cc_sac_data_req | send_datatype_nbr = 2 → i=0: **26** (program_number) 16 bits; i=1: **50** (LTS_id) 8 bits. request_datatype_nbr = 1 → i=0: **42** (record_stop_status) |
| 2 | CICAM sends an acknowledgement to the Host | cc_sac_data_cnf | send_datatype_nbr = 1 → i=0: **42** (record_stop_status) 8 bits |

### §6.4.3.3.4 Change Operating Mode protocol — Table 12 (PDF p. 37)

| Step | Action | APDU | Content |
|------|--------|------|---------|
| 1 | Host informs CICAM of change of operating mode | cc_sac_data_req | send_datatype_nbr = 3 → i=0: **38** (operating_mode) 8 bits; i=1: **26** (program_number) 16 bits; i=2: **50** (LTS_id) 8 bits. request_datatype_nbr = 1 → i=0: **41** (mode_change_status) |
| 2 | CICAM sends an acknowledgement to the Host | cc_sac_data_cnf | send_datatype_nbr = 1 → i=0: **41** (mode_change_status) 8 bits |

### §6.4.3.3.5 CICAM to Host License Exchange protocol — Table 13 (PDF p. 38)

| Step | Action | APDU | Content |
|------|--------|------|---------|
| 1 | CICAM supplies the Host with content license | cc_sac_data_req | send_datatype_nbr = 4 or 5 → i=0: **26** (program_number) (note 1) 16 bits; i=1: **34** (license_status) (note 2) 8 bits; i=2: **25** (uri_message) 64 bits; i=3: **33** (cicam_license) variable; i=4: **50** (LTS_id) 8 bits. request_datatype_nbr = 1 → i=0: **35** (license_rcvd_status) (note 3) |
| 2 | Host confirms receipt | cc_sac_data_cnf | send_datatype_nbr = 1 → i=0: **35** (license_rcvd_status) (note 3) 8 bits |

- NOTE 1 — The `program_number` matches the Record Start message's `program_number`.
- NOTE 2 — Table 11.45 in CI Plus V1.3 [3] contains the allowed values and meaning of `license_status`.
- NOTE 3 — Table 11.42 in CI Plus V1.3 [3] contains the allowed values and meaning of `license_rcvd_status`.

The `cicam_license` (datatype 33) body is an **opaque** variable-length license
blob — only its presence and the surrounding wire structure are defined here; the
license bytes themselves are crypto/DRM-private (CI Plus V1.3 [3]). `PINcode data`
(datatype 39) is likewise opaque variable-length.
