# Transport Protocol Data Unit (TPDU) framing + transport tag values

_Source: EN 50221 Annex A §A.4.1, Tables A.1-A.16 + Figures A.4-A.17 (PDF pp. 63-70), render-verified_

> SCOPE NOTE: The TPDU framing lives in **Annex A**, which defines the PC-Card
> (PCMCIA) implementation. The TPDU tag values and structures below are the
> command-interface link/transport framing. The PC-Card hardware transport itself
> (PCMCIA electrical / COR / interrupt mechanics) is out of scope for the planned
> crate; this doc captures only the TPDU wire framing that a software stack parses.

The transport protocol is a command-response protocol: the host sends a Command TPDU
(C_TPDU) to the module, and the module replies with a Response TPDU (R_TPDU). The
length_field uses the Table 1 coding (see `apdu-coding.md`).

## Table A.16 — Coding of Transport tags (tpdu_tag, p. 70)

The `tpdu_tag` follows ASN.1 rules and is coded in **one byte**.
Direction column: `<---` host to module, `--->` module to host, `<-->` either.

| tpdu_tag | tag value (hex) | Primitive/Constructed | Direction (host <-> module) |
|----------|-----------------|-----------------------|------------------------------|
| TSB           | `80` | P | `<---` |
| TRCV          | `81` | P | `--->` |
| Tcreate_t_c   | `82` | P | `--->` |
| Tc_t_c_reply  | `83` | P | `<---` |
| Tdelete_t_c   | `84` | P | `<-->` |
| Td_t_c_reply  | `85` | P | `<-->` |
| Trequest_t_c  | `86` | P | `<---` |
| Tnew_t_c      | `87` | P | `--->` |
| Tt_c_error    | `88` | P | `--->` |
| Tdata_last    | `A0` | C | `<-->` |
| Tdata_more    | `A1` | C | `<-->` |

Direction note: the renders show TSB `<----`, TRCV `---->`, Tcreate_t_c `---->`,
Tc_t_c_reply `<----`, Trequest_t_c `<----`, Tt_c_error `---->`; Tdelete_t_c,
Td_t_c_reply, Tdata_last, Tdata_more shown bidirectional `<---->`.

## C_TPDU / R_TPDU structures (Tables A.1-A.2, pp. 63-64)

### Table A.1 — C_TPDU coding (Command TPDU, host -> module)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `C_TPDU() {` | | |
| &nbsp;&nbsp;c_tpdu_tag | 8 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;t_c_id | 8 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<length_value-1; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;data_byte | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

`length_field` codes the length of all following fields (t_c_id + data). The
conditional body length = length_value - 1 (the t_c_id occupies one byte of it).

### Table A.2 — R_TPDU coding (Response TPDU, module -> host)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `R_TPDU() {` | | |
| &nbsp;&nbsp;r_tpdu_tag | 8 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;t_c_id | 8 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<length_value-1; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;data_byte | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;SB_tag | 8 | uimsbf |
| &nbsp;&nbsp;length_field() = 2 | | |
| &nbsp;&nbsp;t_c_id | 8 | uimsbf |
| &nbsp;&nbsp;SB_value | 8 | uimsbf |
| `}` | | |

The R_TPDU has three parts: conditional header (r_tpdu_tag + length_field + t_c_id),
conditional body (data field), and a mandatory Status (SB_tag + length_field=2 +
t_c_id + one-byte SB_value). The status is NOT included in the length_field
calculation.

## SB_value coding (Figure A.6 + Table A.3, p. 64)

| bit8 | bit7..bit1 |
|------|-----------|
| DA | reserved (shall be zero) |

| bit8 (DA) | meaning |
|-----------|---------|
| 0 | no message available |
| 1 | message available |

The 1-bit DA (Data Available) indicator says whether the module has a message in its
output buffer for the host. The host issues a Receive_data C_TPDU (TRCV) to get it.

## Transport connection management objects (§A.4.1.4-A.4.1.10, pp. 65-68)

### Table A.4 — Create_T_C coding (header-only object)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `Create_T_C() {` | | |
| &nbsp;&nbsp;create_T_C_tag | 8 | uimsbf |
| &nbsp;&nbsp;length_field() = 1 | | |
| &nbsp;&nbsp;t_c_id | 8 | uimsbf |
| `}` | | |

### C_T_C_Reply coding (labelled "Table A.4" in the PDF, p. 65)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `C_T_C_Reply() {` | | |
| &nbsp;&nbsp;C_T_C_Reply_tag | 8 | uimsbf |
| &nbsp;&nbsp;length_field() = 1 | | |
| &nbsp;&nbsp;t_c_id | 8 | uimsbf |
| `}` | | |

> NOTE: The PDF mislabels both Create_T_C and C_T_C_Reply tables as "Table A.4"
> (typo in the spec). Delete_T_C is Table A.6, Request_T_C is Table A.8, New_T_C is
> Table A.9. The Delete_T_C / D_T_C_Reply structures are the same single-part form as
> Create_T_C / C_T_C_Reply (tag + length_field=1 + t_c_id).

### Table A.8 — Request_T_C coding

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `Request_T_C() {` | | |
| &nbsp;&nbsp;request_T_C_tag | 8 | uimsbf |
| &nbsp;&nbsp;length_field() = 1 | | |
| &nbsp;&nbsp;t_c_id | 8 | uimsbf |
| `}` | | |

### Table A.9 — New_T_C coding

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `New_T_C() {` | | |
| &nbsp;&nbsp;new_T_C_tag | 8 | uimsbf |
| &nbsp;&nbsp;length_field() = 2 | | |
| &nbsp;&nbsp;t_c_id | 8 | uimsbf |
| &nbsp;&nbsp;new_t_c_id | 8 | uimsbf |
| `}` | | |

New_T_C has a body consisting of the transport connection identifier for the new
connection to be established (`new_t_c_id`).

### Table A.10 — T_C_Error coding

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `T_C_Error() {` | | |
| &nbsp;&nbsp;T_C_Error_tag | 8 | uimsbf |
| &nbsp;&nbsp;length_field() = 2 | | |
| &nbsp;&nbsp;t_c_id | 8 | uimsbf |
| &nbsp;&nbsp;error_code | 8 | uimsbf |
| `}` | | |

### Table A.11 — Error code values

| error code | meaning |
|------------|---------|
| 1 | no transport connections available |

## Send Data / Receive Data and chaining (Tables A.12-A.15, pp. 68-69)

- **Send Data** (Table A.12): a C_TPDU whose `c_TPDU_tag` is `M_c_TPDU_tag`
  (= Tdata_more, `A1`) for non-final blocks or `L_c_TPDU_tag` (= Tdata_last, `A0`)
  for the final block; followed by length_field, t_c_id, and a data field that is a
  subset of `(TLV TLV ... TLV)`. The module replies with the status byte (Table A.13).
  A Send Data C_TPDU with L=1 (no data field, t_c_id only) is the poll used to read
  the status byte.
- **Receive Data** (Table A.14): a C_TPDU with `c_TPDU_tag` = TRCV (`81`),
  `c_TPDU_length` = 1, t_c_id. The module replies (Table A.15) with an R_TPDU whose
  `r_TPDU_tag` is `M_r_TPDU_tag` (= Tdata_more) or `L_r_TPDU_tag` (= Tdata_last),
  length_field, t_c_id, data field (subset of TLVs), and Status (SB_value).
- **Chaining** (§A.4.1.3): C_TPDU/R_TPDU data fields may be split into blocks. All
  blocks except the last use the `M_*` tag; the last uses the `L_*` tag. The receiver
  concatenates all received data fields when the last block arrives.
- **Polling** (§A.4.1.12): the host periodically issues a Send Data C_TPDU with
  length 1 (t_c_id only) to learn whether the module has data. While the module still
  has data (status DA=1) the poll is suspended; it restarts when a status byte with
  DA=0 is received. Maximum poll period 100 ms; poll timeout 300 ms.
