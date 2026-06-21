# Multi-stream Host Control resource (CI Plus)

_Source: ETSI TS 103 205 v1.4.1 §6.4.5, Tables 17-22 (PDF pp. 40-44); base tags in §13.2, Tables 97-102 (PDF pp. 126-130), render-verified_

The multi-stream Host Control resource type (**resource ID `0x00200081`**) is
based on DVB Host Control version 3 (§13, resource ID `0x00200043`). The
multi-stream version lets the CICAM request either foreground tuning (for
presentation) or background tuning (not for presentation). The Host responds to a
tune request, if successful, with `tune_reply()` informing the CICAM which
`LTS_id` carries the requested stream.

The multi-stream tune APDUs (`tune_broadcast_req`, `tune_triplet_req`,
`tune_lcn_req`, `tune_ip_req`) are modified versions of the DVB Host Control v3
APDUs: they add `background_tune_flag` (and reuse `tune_quietly_flag` /
`keep_app_running_flag`). All other APDUs retain the same syntax as DVB Host
Control v3 (§13).

## Table 17 — Multi-stream Host Control APDUs (PDF p. 41)

| APDU | Direction |
|------|-----------|
| tune_broadcast_req | CICAM → Host |
| tune_triplet_req   | CICAM → Host |
| tune_lcn_req       | CICAM → Host |
| tune_ip_req        | CICAM → Host |
| tune_reply         | Host → CICAM |
| ask_release        | Host → CICAM |
| ask_release_reply  | CICAM → Host |
| tuner_status_req   | CICAM → Host |
| tuner_status_reply | Host → CICAM |

## apdu_tag values

The tag values are inherited from the DVB Host Control v3 resource (§13.2). The
PDF prints these tags explicitly:

| APDU | Tag value | Source |
|------|-----------|--------|
| tune_triplet_req   | `0x9F8409` | §13.2.2 Table 98 (p. 128) |
| tune_lcn_req       | `0x9F8407` | §13.2.3 Table 99 (p. 128) |
| tune_ip_req        | `0x9F8408` | §13.2.4 Table 100 (p. 129) |
| tuner_status_req   | `0x9F840A` | §13.2.5 Table 101 (p. 129) |
| tuner_status_reply | `0x9F840B` | §13.2.6 Table 102 (p. 130) |
| tune_broadcast_req | ⚠ not printed | extends CI Plus V1.3 [3] §14.6.2.1 |
| tune_reply         | ⚠ not printed | refer Table 14.30 CI Plus V1.3 [3] |
| ask_release / ask_release_reply | ⚠ not printed | DVB Host Control V3 (proprietary base) |

⚠ The `tune_broadcast_req_tag`, `tune_reply_tag`, `ask_release` and
`ask_release_reply` tags are NOT numerically printed in TS 103 205 — they are
referenced to CI Plus V1.3 [3] §14.6.x / Table 14.30 (proprietary). Only the
syntax is given here.

## §6.4.5.2 — tune_broadcast_req APDU — Table 18 (PDF p. 42)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `tune_broadcast_req() {` | | |
| &nbsp;&nbsp;tune_broadcast_req_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;reserved | 4 | uimsbf |
| &nbsp;&nbsp;background_tune_flag | 1 | uimsbf |
| &nbsp;&nbsp;tune_quietly_flag | 1 | uimsbf |
| &nbsp;&nbsp;keep_app_running_flag | 1 | uimsbf |
| &nbsp;&nbsp;pmt_flag | 1 | uimsbf |
| &nbsp;&nbsp;service_id | 16 | uimsbf |
| &nbsp;&nbsp;reserved | 4 | uimsbf |
| &nbsp;&nbsp;descriptor_loop_length | 12 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<N; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;descriptor() | | |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`if (pmt_flag == 1) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;program_map_section() | | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field semantics (§6.4.5.2, p. 42):
- **background_tune_flag** (1) — `0b0` = tune is to be presented to the user; `0b1` = tune is to be performed in the background (not presented). If a background request, `tune_quietly_flag` and `keep_app_running_flag` shall be ignored.
- Other fields — refer to §13.2.1.

## §6.4.5.3 — tune_triplet_req APDU — Table 19 (PDF p. 42)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `tune_triplet_req () {` | | |
| &nbsp;&nbsp;tune_triplet_req_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;reserved | 5 | uimsbf |
| &nbsp;&nbsp;background_tune_flag | 1 | uimsbf |
| &nbsp;&nbsp;tune_quietly_flag | 1 | uimsbf |
| &nbsp;&nbsp;keep_app_running_flag | 1 | uimsbf |
| &nbsp;&nbsp;original_network_id | 16 | uimsbf |
| &nbsp;&nbsp;transport_stream_id | 16 | uimsbf |
| &nbsp;&nbsp;service_id | 16 | uimsbf |
| &nbsp;&nbsp;delivery_system_descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;`if (delivery_system_descriptor_tag == 0x7f) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;descriptor_tag_extension | 8 | uimsbf |
| &nbsp;&nbsp;`} else {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

- **background_tune_flag** — see §6.4.5.2. Other fields — refer to §13.2.2.

## §6.4.5.4 — tune_lcn_req APDU — Table 20 (PDF p. 43)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `tune_lcn_req () {` | | |
| &nbsp;&nbsp;tune_lcn_req_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;reserved | 7 | uimsbf |
| &nbsp;&nbsp;background_tune_flag | 1 | uimsbf |
| &nbsp;&nbsp;tune_quietly_flag | 1 | uimsbf |
| &nbsp;&nbsp;keep_app_running_flag | 1 | uimsbf |
| &nbsp;&nbsp;logical_channel_number | 14 | uimsbf |
| `}` | | |

- **background_tune_flag** — see §6.4.5.2. Other fields — refer to §13.2.3.

## §6.4.5.5 — tune_ip_req APDU — Table 21 (PDF p. 43)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `tune_ip_req () {` | | |
| &nbsp;&nbsp;tune_ip_req_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;reserved | 1 | uimsbf |
| &nbsp;&nbsp;background_tune_flag | 1 | uimsbf |
| &nbsp;&nbsp;tune_quietly_flag | 1 | uimsbf |
| &nbsp;&nbsp;keep_app_running_flag | 1 | uimsbf |
| &nbsp;&nbsp;service_location_length | 12 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<N; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;service_location_data | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

⚠ In Table 21 the `reserved` field before `background_tune_flag` is printed as
**1 bit** (so `reserved`(1) + 3 flags = a 4-bit prefix before `service_location_length`).
This differs from the §13.2.4 base `tune_ip_req` (Table 100, p. 129) which shows
`reserved` = **2 bits** + `tune_quietly_flag` + `keep_app_running_flag` (no
`background_tune_flag`). Transcribed exactly as each renders.

- **background_tune_flag** — see §6.4.5.2. Other fields — refer to §13.2.4.

## §6.4.5.6 — tune_reply APDU — Table 22 (PDF p. 44)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `tune_reply () {` | | |
| &nbsp;&nbsp;tune_reply_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;LTS_id | 8 | uimsbf |
| &nbsp;&nbsp;status_field | 8 | uimsbf |
| `}` | | |

Field semantics (§6.4.5.6, p. 44):
- **LTS_id** (8) — indicates the Local TS identifier where the Local TS resulting from the tune request can be found. The `LTS_id` shall be ignored if the tune request is unsuccessful.
- Other fields — refer to Table 14.30 of CI Plus V1.3 [3] (proprietary; `status_field` value `0x05` = service not found per §13.2.3).

---

## Base DVB Host Control v3 tune APDU syntaxes (§13.2, reference)

These are the unmodified v3 layouts (resource ID `0x00200043`) that the
multi-stream variants above are derived from. Provided for completeness — the
multi-stream tables (18-21) override these where a `background_tune_flag` is added.

### Table 97 — tune_broadcast_req (v3 base) (PDF p. 127)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `tune_broadcast_req() {` | | |
| &nbsp;&nbsp;tune_broadcast_req_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;reserved | 5 | uimsbf |
| &nbsp;&nbsp;tune_quietly_flag | 1 | uimsbf |
| &nbsp;&nbsp;keep_app_running_flag | 1 | uimsbf |
| &nbsp;&nbsp;pmt_flag | 1 | uimsbf |
| &nbsp;&nbsp;service_id | 16 | uimsbf |
| &nbsp;&nbsp;reserved | 4 | uimsbf |
| &nbsp;&nbsp;descriptor_loop_length | 12 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<n; i++) { descriptor() }` | | |
| &nbsp;&nbsp;`if (pmt_flag == 1) { program_map_section() }` | | |
| `}` | | |

### Table 98 — tune_triplet_req (v3, tag `0x9F8409`) (PDF p. 128)

Note: the PDF labels the first row `tune_lcn_triplet_tag` then states
`tune_triplet_req_tag` = `0x9F8409` in the field list. Syntax: `reserved`(6),
`tune_quietly_flag`(1), `Keep_app_running_flag`(1), `original_network_id`(16),
`transport_stream_id`(16), `service_id`(16), `delivery_system_descriptor_tag`(8),
then `if (==0x7f) descriptor_tag_extension`(8) `else reserved`(8).
⚠ first-row mnemonic printed as `tune_lcn_triplet_tag` (evident label slip;
field-list text gives the authoritative tag `0x9F8409`).

### Table 99 — tune_lcn_req (v3, tag `0x9F8407`) (PDF p. 128)

`reserved`? not shown; syntax: `tune_lcn_req_tag`(24), `length_field()`,
`tune_quietly_flag`(1), `keep_app_running_flag`(1), `logical_channel_number`(14).
`logical_channel_number` range 0..9999.

### Table 100 — tune_ip_req (v3, tag `0x9F8408`) (PDF p. 129)

`tune_ip_req_tag`(24), `length_field()`, `reserved`(2), `tune_quietly_flag`(1),
`keep_app_running_flag`(1), `service_location_length`(12), loop
`service_location_data`(8).

### Table 101 — tuner_status_req (v3, tag `0x9F840A`) (PDF p. 129)

`tuner_status_req_tag`(24), `length_field() = 0`.

### Table 102 — tuner_status_reply (v3, tag `0x9F840B`) (PDF p. 130)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `tuner_status_reply() {` | | |
| &nbsp;&nbsp;tuner_status_reply_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;IP_tune_capable_flag | 1 | uimsbf |
| &nbsp;&nbsp;num_dsd | 7 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<num_dsd; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 7 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;connected_flag | 1 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;delivery_system_descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`if (delivery_system_descriptor_tag == 0x7f) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;descriptor_tag_extension | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`} else {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

- **IP_tune_capable_flag** (1) — `0b1` if the Host can accept `tune_ip_req()` APDUs for IP-delivered services.
- **num_dsd** (7) — number of used DSD types supported by the Host.
- **connected_flag** (1) — `0b1` if the Host believes at least one tuner of this DSD type is connected.
