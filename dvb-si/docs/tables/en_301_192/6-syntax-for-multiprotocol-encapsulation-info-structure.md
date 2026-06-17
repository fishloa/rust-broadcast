## Table 6 — Syntax for multiprotocol_encapsulation_info structure
_§7.2.1, PDF pp. 19-19_

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| multiprotocol_encapsulation_info () { |  |  |
| MAC_address_range | 3 | uimsbf |
| MAC_IP_mapping_flag | 1 | bslbf |
| alignment_indicator | 1 | bslbf |
| reserved | 3 | bslbf |
| max_sections_per_datagram | 8 | uimsbf |
| } |  |  |

> **Spec note:** The PDF prints `MAC_IP_mapping_flag` with an underscore between IP and
> mapping (confirmed p.19). The field `max_sections_per_datagram` uses underscores
> throughout (confirmed p.19).

