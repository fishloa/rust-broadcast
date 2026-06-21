# CICAM Player resource (CI Plus, IP delivery CICAM player mode)

_Source: ETSI TS 103 205 v1.4.1 §8.8, Tables 48-71 (PDF pp. 86-96), render-verified_

The CICAM Player resource lets the Host request the CICAM to initiate and play a
service on its behalf, and lets the CICAM spontaneously request a play session.
The resource is **provided by the Host**. Tags live in the CI Plus `0x9FA0xx`
namespace.

## Table 71 — CICAM player resource summary (PDF p. 96)

Resource Identifier `0x00930041` — Class 147, Type 1, Version 1.

| APDU tag | Tag value | Host | CICAM |
|----------|-----------|:----:|:-----:|
| CICAM_player_verify_req       | `9F A0 00` | → |   |
| CICAM_player_verify_reply     | `9F A0 01` |   | → |
| CICAM_player_capabilities_req | `9F A0 02` |   | → |
| CICAM_player_capabilities_reply | `9F A0 03` | → |   |
| CICAM_player_start_req        | `9F A0 04` |   | → |
| CICAM_player_start_reply      | `9F A0 05` | → |   |
| CICAM_player_play_req         | `9F A0 06` | → |   |
| CICAM_player_status_error     | `9F A0 07` |   | → |
| CICAM_player_control_req      | `9F A0 08` | → |   |
| CICAM_player_info_req         | `9F A0 09` | → |   |
| CICAM_player_info_reply       | `9F A0 0A` |   | → |
| CICAM_player_stop             | `9F A0 0B` | → |   |
| CICAM_player_end              | `9F A0 0C` |   | → |
| CICAM_player_asset_end        | `9F A0 0D` |   | → |
| CICAM_player_update_req       | `9F A0 0E` |   | → |
| CICAM_player_update_reply     | `9F A0 0F` | → |   |

(Direction per Table 48 / Table 71: `verify_req`, `capabilities_reply`,
`play_req`, `control_req`, `info_req`, `stop`, `update_reply` are Host→CICAM;
the rest are CICAM→Host.)

## §8.8.3 — CICAM_player_verify_req APDU — Table 49 (PDF p. 87)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `CICAM_player_verify_req() {` | | |
| &nbsp;&nbsp;CICAM_player_verify_req_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;service_location_length | 16 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<service_location_length; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;service_location_byte | 8 | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

- **CICAM_player_verify_req_tag** — `0x9FA000`.
- **service_location_byte** — bytes forming an XML data structure with a single ServiceLocation element (XML schema in annex D).

## §8.8.4 — CICAM_player_verify_reply APDU — Table 50 (PDF p. 87)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `CICAM_player_verify_reply() {` | | |
| &nbsp;&nbsp;CICAM_player_verify_reply_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 1 | | |
| &nbsp;&nbsp;player_verify_status | 8 | uimsbf |
| `}` | | |

- **CICAM_player_verify_reply_tag** — `0x9FA001`.

### Table 51 — player_verify_status values (PDF p. 87)

| Program_start_status | Value |
|----------------------|-------|
| OK - service playback is possible | `0x00` |
| Error - service playback is not possible | `0x01` |
| Reserved | `0x02`–`0xFF` |

## §8.8.5 — CICAM_player_capabilities_req APDU — Table 52 (PDF p. 88)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `CICAM_player_capabilities_req() {` | | |
| &nbsp;&nbsp;CICAM_player_capabilities_req_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 0 | | |
| `}` | | |

- **CICAM_player_capabilities_req_tag** — `0x9FA002`.

## §8.8.6 — CICAM_player_capabilities_reply APDU — Table 53 (PDF p. 88)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `CICAM_player_capabilities_reply() {` | | |
| &nbsp;&nbsp;CICAM_player_capabilities_reply_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;number_of_component_types | 16 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<number_of_component_types; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;stream_content | 4 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 4 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;component_type | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

- **CICAM_player_capabilities_reply_tag** — `0x9FA003`.
- **stream_content** (4) / **component_type** (8) — coding as in the Component descriptor, ETSI EN 300 468 [10].

## §8.8.7 — CICAM_player_start_req APDU — Table 54 (PDF p. 89)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `CICAM_player_start_req() {` | | |
| &nbsp;&nbsp;CICAM_player_start_req_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;input_max_bitrate | 16 | uimsbf |
| &nbsp;&nbsp;output_max_bitrate | 16 | uimsbf |
| &nbsp;&nbsp;linearChannel | 1 | bslbf |
| &nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;PMT_length | 16 | uimsbf |
| &nbsp;&nbsp;`for (int i=0; i<PMT_length; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;PMT_byte | 8 | bslbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

- **CICAM_player_start_req_tag** — `0x9FA004`.
- **input_max_bitrate** (16) — bitrate requested for Host→CICAM delivery, in units of 10 kbps rounded up. E.g. 512 kbps = `0x0034`.
- **output_max_bitrate** (16) — bitrate requested for CICAM→Host delivery, in units of 10 kbps.
- **linearChannel** (1) — set = linear channel with no timeshift; not set = VOD asset or timeshifted linear channel.
- **PMT_byte** — a byte of the PMT (MPEG table, first byte = PMT `table_id`).

## §8.8.8 — CICAM_player_start_reply APDU — Table 55 (PDF p. 90)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `CICAM_player_start_reply() {` | | |
| &nbsp;&nbsp;CICAM_player_start_reply_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 2 | | |
| &nbsp;&nbsp;LTS_id | 8 | uimsbf |
| &nbsp;&nbsp;input_status | 8 | uimsbf |
| `}` | | |

- **CICAM_player_start_reply_tag** — `0x9FA005`.
- **LTS_id** — Local TS allocated for data delivery; also uniquely identifies the player session. Ignored if `input_status` is non-zero.

### Table 56 — input_status values (PDF p. 90)

| input_status | Value |
|--------------|-------|
| OK - a Local TS has switched to Input Mode | `0x00` |
| Request refused | `0x01` |
| Insufficient bitrate available | `0x02` |
| No remaining player sessions available | `0x03` |
| Reserved | `0x04`–`0xFF` |

## §8.8.9 — CICAM_player_play_req APDU — Table 57 (PDF p. 90)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `CICAM_player_play_req() {` | | |
| &nbsp;&nbsp;CICAM_player_play_req_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;service_location_length | 16 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<service_location_length; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;service_location_byte | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

- **CICAM_player_play_req_tag** — `0x9FA006`.
- **service_location_byte** — XML ServiceLocation element (schema in annex D).

## §8.8.10 — CICAM_player_status_error APDU — Table 58 (PDF p. 91)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `CICAM_player_status_error() {` | | |
| &nbsp;&nbsp;CICAM_player_status_error_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 3 | | |
| &nbsp;&nbsp;reserved | 7 | uimsbf |
| &nbsp;&nbsp;valid_LTS_id | 1 | uimsbf |
| &nbsp;&nbsp;LTS_id | 8 | |
| &nbsp;&nbsp;player_status | 8 | |
| `}` | | |

- **CICAM_player_status_error_tag** — `0x9FA007`.
- **valid_LTS_id** (1) — `1` when the error relates to an established session with a known `LTS_id`; `0` = session not yet established (`LTS_id` undefined).

### Table 59 — play_status values (PDF p. 91)

| play_status | Value |
|-------------|-------|
| Reserved | `0x00` |
| Error - content play is not possible (e.g. unsupported content format or protocol) | `0x01` |
| Error - unrecoverable error | `0x02` |
| Error - content blocked (e.g. no content license available) | `0x03` |
| Reserved | `0x04`–`0xFF` |

## §8.8.11 — CICAM_player_control_req APDU — Table 60 (PDF p. 92)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `CICAM_player_control_req() {` | | |
| &nbsp;&nbsp;CICAM_player_control_req_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;LTS_id | 8 | uimsbf |
| &nbsp;&nbsp;Command | 8 | uimsbf |
| &nbsp;&nbsp;`if (command == 0x01) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;seek_mode | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;seek_position | 32 | tcimsbf |
| &nbsp;&nbsp;`} else if (command == 0x02) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;Speed | 16 | tcimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

- **CICAM_player_control_req_tag** — `0x9FA008`.
- **seek_position** (32, tcimsbf, signed) — seek position in milliseconds. `0xFFFFFFFF` = "jump to live" (live) / end of asset (VoD).
- **speed** (16, tcimsbf, signed) — play speed as hundredths of nominal. 100 = nominal, 200 = ×2 fwd, 50 = ×0.5 fwd, 0 = pause, −100 = nominal reverse.

### Table 61 — Command values (PDF p. 92)

| Command | Value |
|---------|-------|
| Reserved | `0x00` |
| Set position | `0x01` |
| Set speed | `0x02` |

### Table 62 — seek_mode values (PDF p. 92)

| seek_mode | Value |
|-----------|-------|
| Absolute | `0x00` |
| Relative to current position | `0x01` |

## §8.8.12 — CICAM_player_info_req APDU — Table 63 (PDF p. 93)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `CICAM_player_info_req() {` | | |
| &nbsp;&nbsp;CICAM_player_info_req_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 1 | | |
| &nbsp;&nbsp;LTS_id | 8 | uimsbf |
| `}` | | |

- **CICAM_player_info_req_tag** — `0x9FA009`.

## §8.8.13 — CICAM_player_info_reply APDU — Table 64 (PDF p. 93)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `CICAM_player_info_reply() {` | | |
| &nbsp;&nbsp;CICAM_player_info_reply_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 11 | | |
| &nbsp;&nbsp;LTS_id | 8 | uimsbf |
| &nbsp;&nbsp;duration | 32 | uimsbf |
| &nbsp;&nbsp;position | 32 | uimsbf |
| &nbsp;&nbsp;speed | 16 | uimsbf |
| `}` | | |

- **CICAM_player_info_reply_tag** — `0x9FA00A`.
- **duration** (32) — total content duration in seconds; `0xFFFFFFFF` if not known (e.g. linear).
- **position** (32) — current play position in seconds since the start; `0xFFFFFFFF` if not known.
- **speed** (16) — signed integer, current playout speed in hundredths of nominal.

## §8.8.14 — CICAM_player_stop APDU — Table 65 (PDF p. 94)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `CICAM_player_stop() {` | | |
| &nbsp;&nbsp;CICAM_player_stop_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 1 | | |
| &nbsp;&nbsp;LTS_id | 8 | uimsbf |
| `}` | | |

- **CICAM_player_stop_tag** — `0x9FA00B`.

## §8.8.15 — CICAM_player_end APDU — Table 66 (PDF p. 94)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `CICAM_player_end() {` | | |
| &nbsp;&nbsp;CICAM_player_end_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 1 | | |
| &nbsp;&nbsp;LTS_id | 8 | uimsbf |
| `}` | | |

- **CICAM_player_end_tag** — `0x9FA00C`.

## §8.8.16 — CICAM_player_asset_end APDU — Table 67 (PDF p. 94)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `CICAM_player_asset_end() {` | | |
| &nbsp;&nbsp;CICAM_player_asset_end_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 2 | | |
| &nbsp;&nbsp;LTS_id | 8 | uimsbf |
| &nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;beginning | 1 | bslbf |
| `}` | | |

- **CICAM_player_asset_end_tag** — `0x9FA00D`.
- **reserved** — shall be `0x7F`.
- **beginning** (1) — `1` = start of the asset reached; otherwise end of the asset reached.

## §8.8.17 — CICAM_player_update_req APDU — Table 68 (PDF p. 95)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `CICAM_player_update_req() {` | | |
| &nbsp;&nbsp;CICAM_player_update_req_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;LTS_id | 8 | uimsbf |
| &nbsp;&nbsp;PMT_length | 16 | uimsbf |
| &nbsp;&nbsp;`for(int i=0; i<PMT_length; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;PMT_byte | 8 | bslbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

- **CICAM_player_update_req_tag** — `0x9FA00E`.
- **PMT_length** — number of bytes forming the PMT; shall not be zero.
- **PMT_byte** — a byte of the PMT (MPEG table, first byte = PMT `table_id`).

## §8.8.18 — CICAM_player_update_reply APDU — Table 69 (PDF p. 95)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `CICAM_player_update_reply() {` | | |
| &nbsp;&nbsp;CICAM_player_start_reply_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() = 2 | | |
| &nbsp;&nbsp;LTS_id | 8 | uimsbf |
| &nbsp;&nbsp;update_status | 8 | uimsbf |
| `}` | | |

- **CICAM_player_update_reply_tag** — `0x9FA00F`.
  ⚠ The first syntax row in Table 69 (p. 95) is literally printed as
  `CICAM_player_start_reply_tag` (an evident copy/paste slip from Table 55); the
  field-list text gives the authoritative tag `0x9FA00F` and names it
  `CICAM_player_update_reply_tag`. Use `0x9FA00F`.

### Table 70 — update_status values (PDF p. 96)

| update_status | Value |
|---------------|-------|
| OK - the Host has processed the updated PMT and is ready to receive the Local TS | `0x00` |
| Request refused | `0x01` |
| Reserved | `0x02`–`0xFF` |
