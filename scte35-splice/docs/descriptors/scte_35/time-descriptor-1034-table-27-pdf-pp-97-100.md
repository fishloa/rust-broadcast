## time_descriptor() — §10.3.4, Table 27, PDF pp. 97-100

Optional extension to the splice_insert(), splice_null() and time_signal()
commands that allows a programmer's wall clock time to be sent to a client,
in the time format of the Precision Time Protocol [PTP] (TAI: no leap
seconds, unlike UTC; PTP uses the same epoch as Unix time, 00:00 January 1,
1970). For the highest accuracy, use with a command whose
`time_specified_flag` == 1. The repetition rate should be at least once
every 5 seconds; when it is the only descriptor in a time_signal() or
splice_null() command, the encoder should not insert a key frame.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `time_descriptor() {` |  |  |
| &nbsp;&nbsp;splice_descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;identifier | 32 | uimsbf |
| &nbsp;&nbsp;TAI_seconds | 48 | uimsbf |
| &nbsp;&nbsp;TAI_ns | 32 | uimsbf |
| &nbsp;&nbsp;UTC_offset | 16 | uimsbf |
| `}` |  |  |

- **splice_descriptor_tag** — shall be **0x03**.
- **descriptor_length** — shall be **0x10**.
- **identifier** — shall be **0x43554549** (ASCII "CUEI").
- **TAI_seconds** — 48-bit TAI seconds value.
- **TAI_ns** — 32-bit TAI nanoseconds value.
- **UTC_offset** — 16 bits, used in the conversion from TAI time to UTC or
  NTP time per: `UTC seconds = TAI seconds − UTC_offset`;
  `NTP seconds = TAI seconds − UTC_offset + 2,208,988,800`.

