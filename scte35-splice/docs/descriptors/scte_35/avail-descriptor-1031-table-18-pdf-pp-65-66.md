## avail_descriptor() — §10.3.1, Table 18, PDF pp. 65-66

Optional extension to the splice_insert() command allowing an authorization
identifier to be sent for an avail (replicating analog cue-tone
functionality). Multiple copies may be included using the descriptor loop.
Intended only for use with a splice_insert() command, within a
splice_info_section.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `avail_descriptor() {` |  |  |
| &nbsp;&nbsp;splice_descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;identifier | 32 | uimsbf |
| &nbsp;&nbsp;provider_avail_id | 32 | uimsbf |
| `}` |  |  |

- **splice_descriptor_tag** — shall be **0x00**.
- **descriptor_length** — shall be **0x08**.
- **identifier** — shall be **0x43554549** (ASCII "CUEI").
- **provider_avail_id** — 32-bit number that a receiving device may utilize
  to alter its behavior during or outside of an avail; may be used in a
  manner similar to analog cue tones (e.g. a network directing an
  affiliate/head-end to black out a sporting event).

