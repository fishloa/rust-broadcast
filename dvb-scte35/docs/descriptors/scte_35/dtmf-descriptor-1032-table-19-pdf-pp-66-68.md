## DTMF_descriptor() — §10.3.2, Table 19, PDF pp. 66-68

Optional extension to the splice_insert() command allowing a receiver device
to generate a legacy analog DTMF sequence based on a splice_info_section
being received.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `DTMF_descriptor() {` |  |  |
| &nbsp;&nbsp;splice_descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;identifier | 32 | uimsbf |
| &nbsp;&nbsp;preroll | 8 | uimsbf |
| &nbsp;&nbsp;dtmf_count | 3 | uimsbf |
| &nbsp;&nbsp;reserved | 5 | bslbf |
| &nbsp;&nbsp;`for(i=0; i<dtmf_count; i++) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;DTMF_char | 8 | uimsbf |
| &nbsp;&nbsp;`}` |  |  |
| `}` |  |  |

- **splice_descriptor_tag** — shall be **0x01**.
- **identifier** — shall be **0x43554549** (ASCII "CUEI").
- **preroll** — the time the DTMF is presented to the analog output of the
  device, in **tenths of seconds** (pre-roll range 0 to 25.5 seconds). The
  splice info section shall be sent at least two seconds earlier than this
  value; the minimum suggested pre-roll is 4.0 seconds.
- **dtmf_count** — the number of DTMF characters the device is to generate.
- **DTMF_char** — an ASCII value for the numerals '0' to '9', '*', '#'. The
  sequence shall complete with the last character sent being the timing mark
  for the pre-roll.

