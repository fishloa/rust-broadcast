# CA PMT Reply object (ca_pmt_reply)

_Source: EN 50221 §8.4.3.5, Table 26 + CA_enable value table (PDF p. 32), render-verified_

apdu_tag `Tca_pmt_reply` = `9F 80 33`, Resource = CA Support, Direction app `<---` host
(i.e. sent by the application to the host).

This object is always sent by the application to the host after reception of a CA PMT
object with `ca_pmt_cmd_id` set to `query`. It may also be sent after reception of a
CA PMT object with `ca_pmt_cmd_id` set to `ok_mmi` to indicate the result of the MMI
dialogue (`descrambling possible` if the user has purchased, `descrambling not
possible (because no entitlement)` if not).

## Table 26 — CA PMT Reply object coding

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `ca_pmt_reply () {` | | |
| &nbsp;&nbsp;ca_pmt_reply_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;program_number | 16 | uimsbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;version_number | 5 | uimsbf |
| &nbsp;&nbsp;current_next_indicator | 1 | bslbf |
| &nbsp;&nbsp;CA_enable_flag | 1 | bslbf |
| &nbsp;&nbsp;`if (CA_enable_flag == 1)` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;CA_enable&nbsp;&nbsp;/* at programme level */ | 7 | uimsbf |
| &nbsp;&nbsp;`else if (CA_enable_flag == 0)` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;`for (i=0; i<n; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 3 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;elementary_PID&nbsp;&nbsp;/* elementary stream PID */ | 13 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;CA_enable_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`if (CA_enable_flag == 1)` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;CA_enable&nbsp;&nbsp;/* at elementary stream level */ | 7 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`else if (CA_enable_flag == 0)` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

## CA_enable values (Table, p. 32)

| CA_enable (meaning) | value |
|---------------------|-------|
| Descrambling possible | `01` |
| Descrambling possible under conditions (purchase dialogue) | `02` |
| Descrambling possible under conditions (technical dialogue) | `03` |
| Descrambling not possible (because no entitlement) | `71` |
| Descrambling not possible (for technical reasons) | `73` |
| RFU | other values `00`-`7F` |

Semantics:
- "descrambling possible" — the application can descramble with no extra condition
  (e.g. user has a subscription, or has authorised the purchase of the ES).
- "descrambling possible under conditions (purchase dialogue)" — application must
  enter a purchase dialogue with the user before descrambling (pay-per-view).
- "descrambling possible under conditions (technical dialogue)" — application must
  enter a technical dialogue (e.g. ask the user to select fewer ES because of
  limited descrambling capabilities).

## Field notes

- The syntax contains one possible `CA_enable` at programme level and, for each
  elementary stream, one possible `CA_enable` at ES level.
  - When both are present, only the ES-level `CA_enable` applies for that ES.
  - When none is present, the host does not interpret the `ca_pmt_reply` object.
- `CA_enable` is a 7-bit field (only present when its `CA_enable_flag` == 1);
  otherwise the 7 bits are reserved.
