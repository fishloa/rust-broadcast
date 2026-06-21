# Conditional Access Support resource — multi-stream extensions (CI Plus)

_Source: ETSI TS 103 205 v1.4.1 §6.4.4, Tables 14-16 (PDF pp. 38-40), render-verified_

For multi-stream functionality the Conditional Access Support resource gets a new
resource_type (**resource_type = 2, version = 1**) in which the `ca_pmt()` and
`ca_pmt_reply()` APDU objects are extended by adding the Local TS identifier
(`LTS_id`). The `ca_pmt()` APDU is **also** extended with a `PMT_PID` field so the
CICAM need not parse the Local TS to obtain it. Host and CICAM use these extended
APDUs while multi-stream mode is active.

These differ from the EN 50221 `ca_pmt` / `ca_pmt_reply` (see
`../en_50221/ca-pmt.md` / `../en_50221/ca-pmt-reply.md`) by the leading `LTS_id`
field and, for `ca_pmt`, the added `PMT_PID` field; the apdu_tag is unchanged
(`ca_pmt_tag` = `0x9F8032`, `ca_pmt_reply_tag` = `0x9F8033`). The
`ca_pmt_list_management` value space is narrowed to a subset (Table 15).

## §6.4.4.2 — ca_pmt APDU — Table 14 (PDF p. 39)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `ca_pmt () {` | | |
| &nbsp;&nbsp;ca_pmt_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;LTS_id | 8 | uimsbf |
| &nbsp;&nbsp;ca_pmt_list_management | 8 | uimsbf |
| &nbsp;&nbsp;program_number | 16 | uimsbf |
| &nbsp;&nbsp;reserved | 3 | bslbf |
| &nbsp;&nbsp;PMT_PID | 13 | uimsbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;version_number | 5 | uimsbf |
| &nbsp;&nbsp;current_next_indicator | 1 | bslbf |
| &nbsp;&nbsp;reserved | 4 | bslbf |
| &nbsp;&nbsp;program_info_length | 12 | uimsbf |
| &nbsp;&nbsp;`if (program_info_length != 0) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;ca_pmt_cmd_id&nbsp;&nbsp;/* at program level */ | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i = 0; i < n; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;CA_descriptor()&nbsp;&nbsp;/* CA descriptor at programme level */ | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`for (i = 0; i < n; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;stream_type | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 3 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;elementary_PID&nbsp;&nbsp;/* elementary stream PID */ | 13 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 4 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;ES_info_length | 12 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`if (ES_info_length != 0) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;ca_pmt_cmd_id&nbsp;&nbsp;/* at ES level */ | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`for (i = 0; i < n; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;CA_descriptor ()&nbsp;&nbsp;/* CA descriptor at elementary stream level */ | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field semantics (§6.4.4.2, p. 39):
- **LTS_id** (8) — Local TS identifier.
- **PMT_PID** (13) — the PID of the PMT of the selected service. Whenever the PMT PID of the selected service changes, a `ca_pmt()` APDU with `ca_pmt_list_management` = `0x05` (update) shall be sent by the Host.
- **ca_pmt_list_management** (8) — in multi-stream mode each programme appears on a separate Local TS, so only the subset of values in Table 15 may be used.
- Other fields — refer to Table 25 of the DVB-CI (EN 50221) specification [1].

### Table 15 — ca_pmt_list_management (multi-stream subset, PDF p. 39)

| ca_pmt_list_management | Value |
|------------------------|-------|
| Only     | `0x03` |
| Update   | `0x05` |
| Reserved | Other values |

In multi-stream mode "Only" is used to start a new programme in the associated
Local TS; this does not affect other Local TSs that may be running.

## §6.4.4.3 — ca_pmt_reply APDU — Table 16 (PDF p. 40)

`LTS_id` is added to the `ca_pmt_reply` object.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `ca_pmt_reply () {` | | |
| &nbsp;&nbsp;ca_pmt_reply_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;LTS_id | 8 | uimsbf |
| &nbsp;&nbsp;program_number | 16 | uimsbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;version_number | 5 | uimsbf |
| &nbsp;&nbsp;current_next_indicator | 1 | bslbf |
| &nbsp;&nbsp;ca_enable_flag | 1 | bslbf |
| &nbsp;&nbsp;`if (ca_enable_flag == 1) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;ca_enable&nbsp;&nbsp;/* at programme level */ | 7 | uimsbf |
| &nbsp;&nbsp;`} else {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`for (i = 0; i < n; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 3 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;elementary_PID | 13 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;ca_enable_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`if (ca_enable_flag == 1) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;ca_enable&nbsp;&nbsp;/* at elementary stream level */ | 7 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`} else {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field semantics (§6.4.4.3, p. 40):
- **LTS_id** (8) — Local TS identifier.
- Other fields — refer to Table 26 of the DVB-CI (EN 50221) specification [1].

Note: in the multi-stream `ca_pmt_reply`, `version_number` and
`current_next_indicator` precede the programme-level `ca_enable_flag`, whereas the
EN 50221 `ca_pmt_reply` (Table 26) lacks the leading `LTS_id`. Layout transcribed
exactly as rendered in Table 16.
