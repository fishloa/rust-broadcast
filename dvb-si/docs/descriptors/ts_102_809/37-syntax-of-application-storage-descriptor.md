## Table 37 — Syntax of application storage descriptor
_§5.4.0, PDF pp. 51-51_

|  | No.of bits | Identifier | Value |
|---|---|---|---|
| application_storage_descriptor() { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x10 |
| descriptor_length | 8 | uimsbf |  |
| Storage_property | 8 | uimsbf |  |
| not_launchable_from_broadcast | 1 | bslbf |  |
| launchable_completely_from_cache | 1 | bslbf |  |
| is_launchable_with_older_version | 1 | bslbf |  |
| reserved | 5 | bslbf |  |
| reserved | 1 | bslbf |  |
| version | 31 | uimsbf |  |
| priority | 8 | uimsbf |  |
| } |  |  |  |

