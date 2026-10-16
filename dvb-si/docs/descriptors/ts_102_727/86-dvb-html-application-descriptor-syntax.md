## Table 86 — DVB-HTML application descriptor syntax
_§10.10.1, PDF pp. 172-172_

| No. of Bits | Identifier | Value |
|---|---|---|
| dvb_html_application_descriptor(){ |  |  |
| descriptor_tag | 8 | uimsbf |  |
| descriptor_length | 8 | uimsbf |  |
| appid_set_length | 8 | uimsbf | N1 |
| for(i=0; i<N1; i++) { |  |  |  |
| application_id | 16 | bslbf |  |
| } |  |  |  |
| for(j=0; j<N; j++) { |  |  |  |
| parameter_bytes | 8 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

descriptor_tag = 0x08. NOTE (transcription): the syntax loops `i<N1` over 16-bit application_id
(N1 = appid_set_length) whereas the prose says appid_set_length "specifies the length of the list
of application_ids" -- count versus byte length is not resolved by the spec text.
