## Table 81 — Syntax of the IP signalling descriptor
_§10.8.2, PDF pp. 169-169_

| Syntax | Bits | Mnemonic |
|---|---|---|
| ip_signalling_descriptor () { |  |  |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| platform_id | 24 | uimsbf |
| } |  |  |

descriptor_tag = 0x11.
