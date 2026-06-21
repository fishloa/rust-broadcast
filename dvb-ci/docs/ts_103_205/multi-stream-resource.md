# Multi-stream resource (CI Plus extensions)

_Source: ETSI TS 103 205 v1.4.1 §6.4.2, Tables 2-5 (PDF pp. 31-33), render-verified_

The multistream resource carries an APDU for the CICAM to indicate its
multi-stream capabilities, plus APDUs to manage PID selection in the Local TSs.
This is a **new** CI Plus resource (it has no EN 50221 equivalent). All apdu_tags
here live in the CI Plus `0x9F92xx` namespace and may collide with EN 50221 /
TS 101 699 tags in other resource namespaces — that is expected.

## Table 2 — Multi-stream resource summary (PDF p. 31)

Resource Identifier `0x00900041` — Class 144, Type 1, Version 1.

| Application Object (APDU Tag) | Tag value | Host | CICAM |
|-------------------------------|-----------|:----:|:-----:|
| CICAM_multistream_capability | `9F 92 00` |   | → (CICAM→Host) |
| PID_select_req               | `9F 92 01` |   | → (CICAM→Host) |
| PID_select_reply             | `9F 92 02` | → (Host→CICAM) | |

Direction arrows as printed in Table 2: `CICAM_multistream_capability` and
`PID_select_req` flow CICAM→Host (`←` in the Host/CICAM "Direction" pair, i.e.
from CICAM); `PID_select_reply` flows Host→CICAM (`→`).

## §6.4.2.2 — CICAM_multistream_capability APDU

### Table 3 — CICAM_multi-stream_capability APDU (PDF p. 32)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `CICAM_multistream_capability () {` | | |
| &nbsp;&nbsp;CICAM_multistream_capability_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;max_local_TS | 8 | uimsbf |
| &nbsp;&nbsp;max_descramblers | 16 | uimsbf |
| `}` | | |

Field semantics:
- **CICAM_multistream_capability_tag** (24) — value `0x9F9200` identifies this APDU.
- **length_field** — length of APDU payload in ASN.1 BER format per CENELEC EN 50221 §8.3.1.
- **max_local_TS** (8) — the maximum number of Local TSs the CICAM is able to receive concurrently.
- **max_descramblers** (16) — the total number of descramblers the CICAM is able to provide concurrently for all Local TSs.

## §6.4.2.3 — PID_select_req APDU

Sent CICAM→Host to request a specific set of PIDs in the received Local TS. The
CICAM provides PIDs in descending priority order; each PID has a
`critical_for_descrambling_flag`.

### Table 4 — PID_select_req APDU (PDF p. 32)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `PID_select_req () {` | | |
| &nbsp;&nbsp;PID_select_req_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;LTS_id | 8 | uimsbf |
| &nbsp;&nbsp;num_PID | 8 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<num_PID; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;critical_for_descrambling_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;PID | 13 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field semantics (§6.4.2.3, p. 33):
- **PID_select_req_tag** (24) — value `0x9F9201` identifies this APDU.
- **length_field** — ASN.1 BER, per EN 50221 §8.3.1.
- **LTS_id** (8) — Local TS identifier.
- **num_PID** (8) — number of PIDs contained in the following loop.
- **critical_for_descrambling_flag** (1) — `1` = the associated PID is critical for descrambling; `0` = not critical.
- **PID** (13) — requested PID value. The Host shall ignore any request for a PID value of `0x1FFF`.

## §6.4.2.4 — PID_select_reply APDU

Sent Host→CICAM to acknowledge a `PID_select_req`, confirming whether selection
of the requested PIDs could be enabled. The CICAM shall wait for the
`PID_select_reply` before issuing the next `PID_select_req` for the same `LTS_id`.

### Table 5 — PID_select_reply APDU (PDF p. 33)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `PID_select_reply () {` | | |
| &nbsp;&nbsp;PID_select_reply_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;LTS_id | 8 | uimsbf |
| &nbsp;&nbsp;reserved | 7 | uimsbf |
| &nbsp;&nbsp;PID_selection_flag | 1 | uimsbf |
| &nbsp;&nbsp;num_PID | 8 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<num_PID; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;PID_selected_flag | 1 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;PID | 13 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

⚠ Mnemonic for `reserved` (the 7-bit field after `LTS_id`) and `PID_selected_flag`
is printed as `uimsbf` in Table 5 (p. 33), not `bslbf` — transcribed exactly as
rendered. The `reserved` (2) inside the loop is `bslbf`.

Field semantics (§6.4.2.4, p. 33):
- **PID_select_reply_tag** (24) — value `0x9F9202` identifies this APDU.
- **length_field** — ASN.1 BER, per EN 50221 §8.3.1.
- **LTS_id** (8) — Local TS identifier.
- **PID_selection_flag** (1) — status of PID selection for the Local TS. If the whole TS is sent as the Local TS, this shall be `0b0` and `num_PID` shall be `0x0`. If PID selection is applied, this shall be `0b1` and the Host informs the CICAM of the selected PIDs via `num_PID` and the list.
- **num_PID** (8) — number of PIDs contained in the following loop.
- **PID_selected_flag** (1) — status of the corresponding requested PID. `0b1` = the PID could be selected successfully; `0b0` = the PID could not be selected by the Host.
- **PID** (13) — PID value to which `PID_selected_flag` applies.
