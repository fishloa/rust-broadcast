## Table 57 — FTA content management descriptor
_§6.2.18.1, PDF pp. 82-82_

| Syntax | Number of bits | Identifier |
|---|---|---|
| FTA_content_management_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| user_defined | 1 | bslbf |
| reserved_future_use | 3 | bslbf |
| do_not_scramble | 1 | uimsbf |
| control_remote_access_over_internet | 2 | uimsbf |
| do_not_apply_revocation | 1 | uimsbf |
| } |

