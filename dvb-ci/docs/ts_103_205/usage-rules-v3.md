# Usage Rules Information (URI) version 3 (CI Plus)

_Source: ETSI TS 103 205 v1.4.1 §11, Tables 90-92 (PDF pp. 110-112), render-verified_

CI Plus URI version 3 extends the CI Plus URI versions 1 and 2 message syntax
(CI Plus V1.3 [3] §5.7.5.2). It adds the **trick_mode_control_info** signal,
applicable to content with `emi_copy_control` set to "one generation copy is
permitted" (`0b10`), to signal trick-mode control enabled/disabled.

URI v3 applies only to Content Control type 64 version 3 and later, or Content
Control type 65 (see annex A). The Host may declare support for URI v3 during URI
version negotiation only on a Content Control session using such a resource ID; a
CICAM may send a URI v3 only when the Host declared support.

The URI message is the `uri_message` datatype carried in the SAC URI transmission
protocol (datatype_id 25; see `content-control.md` §6.4.3.3.1). Note: §11 refers
to "Table 76" of CI Plus V1.3 [3] for the default-values style, but the tables
actually rendered in TS 103 205 §11 are numbered **90, 91, 92**.

## Table 90 — Default values for CI Plus URI version 3 (PDF p. 111)

| Field | Default Initial Value |
|-------|-----------------------|
| protocol_version | `0x03` |
| emi_copy_control_info | `0b11` |
| aps_copy_control_info | `0b00` |
| ict_copy_control_info | `0b0` |
| rct_copy_control_info | `0b0` |
| dot_copy_control_info | `0b0` |
| rl_copy_control_info | `0b00000000` |
| trick_mode_control_info | `0b0` |
| reserved bits | `0b0` |

## Table 91 — URI Version 3 message syntax (PDF p. 111)

| Field | No. of bits | Mnemonic |
|-------|-------------|----------|
| `uri_message() {` | | |
| &nbsp;&nbsp;protocol_version | 8 | uimsbf |
| &nbsp;&nbsp;aps_copy_control_info | 2 | uimsbf |
| &nbsp;&nbsp;emi_copy_control_info | 2 | uimsbf |
| &nbsp;&nbsp;ict_copy_control_info | 1 | uimsbf |
| &nbsp;&nbsp;`if (emi_copy_control_info == 00) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;rct_copy_control_info | 1 | uimsbf |
| &nbsp;&nbsp;`} else {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved = 0 | 1 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;reserved for future use | 1 | uimsbf |
| &nbsp;&nbsp;`if (emi_copy_control_info == 11) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;dot_copy_control_info | 1 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;rl_copy_control_info | 8 | uimsbf |
| &nbsp;&nbsp;`} else {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved = 0x00 | 9 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`if (emi_copy_control_info == 10) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;trick_mode_control_info | 1 | uimsbf |
| &nbsp;&nbsp;`} else {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved = 0 | 1 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;reserved for future use | 39 | uimsbf |
| `}` | | |

Field semantics:
- **protocol_version** (8) — `0x03` for URI v3.
- **emi_copy_control_info** (2) — encryption-mode indicator; `0b10` = "one generation copy is permitted" (the case enabling `trick_mode_control_info`); `0b11` = "no more copies" (the case carrying `dot`/`rl`); `0b00` = "copying not restricted" (the case carrying `rct`).
- **trick_mode_control_info** (1) — trick-mode inhibit bit; see Table 92. Present only when `emi_copy_control_info == 0b10`. Interpretation rules in the Host are out of scope.
- The fixed total width is 64 bits (matching the `uri_message` datatype length 64 bits used in the SAC protocol tables). Other fields (`aps`/`ict`/`rct`/`dot`/`rl`) carry forward the v1/v2 semantics from CI Plus V1.3 [3] §5.7.5.

### Table 92 — Allowed values for trick_mode_control_info (PDF p. 112)

| Contents | Value (binary) | Comment |
|----------|----------------|---------|
| `0x0` | 0 | Trick mode control disabled |
| `0x1` | 1 | Trick mode control enabled |
