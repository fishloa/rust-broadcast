## Table 44 — Terrestrial delivery system descriptor
_§6.2.13.4, PDF pp. 76-76_

| Syntax | Number of bits | Identifier |
|---|---|---|
| terrestrial_delivery_system_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| centre_frequency | 32 | uimsbf |
| bandwidth | 3 | bslbf |
| priority | 1 | bslbf |
| time_slicing_indicator | 1 | bslbf |
| MPE-FEC_indicator | 1 | bslbf |
| reserved_future_use | 2 | bslbf |
| constellation | 2 | bslbf |
| hierarchy_information | 3 | bslbf |
| code_rate_HP_stream | 3 | bslbf |
| code_rate_LP_stream | 3 | bslbf |
| guard_interval | 2 | bslbf |
| transmission_mode | 2 | bslbf |
| other_frequency_flag | 1 | bslbf |
| reserved_future_use | 32 | bslbf |
| } |

