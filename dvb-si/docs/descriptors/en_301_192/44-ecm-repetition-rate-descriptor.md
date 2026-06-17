## Table 44 — ECM repetition rate descriptor
_§9.9, PDF pp. 56-56_

| Syntax | No. of bits | Identifier |
|---|---|---|
| ECM repetition rate_descriptor() { |  |  |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| CA_system_ID | 16 | uimsbf |
| ECM repetition rate | 16 | uimsbf |
| for ( i=0; i<N; i++) { |  |  |
| private_data_byte | 8 | uimsbf |
| } |  |  |
| } |  |  |

> **Spec note:** The PDF prints `ECM repetition rate_descriptor` (space between "repetition"
> and "rate") as the structure name, and `ECM repetition rate` (two spaces) as the field
> name — both with spaces rather than underscores. Transcribed faithfully from p.56.

