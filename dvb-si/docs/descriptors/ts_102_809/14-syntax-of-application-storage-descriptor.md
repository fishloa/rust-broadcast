## Table 14 — Syntax of application storage descriptor
_§5.2.11.2, PDF pp. 28-28_

|  | No.of bits | Identifier |  |
|---|---|---|---|
| application_storage_descriptor() { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x10 |
| descriptor_length | 8 | uimsbf |  |
| storage_property | 8 | uimsbf |  |
| not_launchable_from_broadcast | 1 | bslbf |  |
| launchable_completely_from_cache | 1 | bslbf |  |
| is_launchable_with_older_version | 1 | bslbf |  |
| Reserved | 5 | bslbf |  |
| Reserved | 1 | bslbf |  |
| Version | 31 | uimsbf |  |
| Priority | 8 | uimsbf |  |
| } |  |  |  |

