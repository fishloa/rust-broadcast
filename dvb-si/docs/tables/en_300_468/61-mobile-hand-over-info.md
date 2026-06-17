## Table 61 — Mobile hand-over info
_§6.2.19.2, PDF pp. 85-85_

<!-- Auto-transcription mis-split Tables 61/62/63; reconstructed verbatim from the PDF (2026-06-12). -->

| Syntax | Number of bits | Identifier |
|---|---|---|
| mobile_hand-over_info() { |
| hand-over_type | 4 | uimsbf |
| reserved_future_use | 3 | bslbf |
| origin_type | 1 | bslbf |
| if (hand-over_type == 0x1 \|\| hand-over_type == 0x2 \|\| hand-over_type == 0x3) { |
| network_id | 16 | uimsbf |
| } |
| if (origin_type == 0b0) { |
| initial_service_id | 16 | uimsbf |
| } |
| } |

