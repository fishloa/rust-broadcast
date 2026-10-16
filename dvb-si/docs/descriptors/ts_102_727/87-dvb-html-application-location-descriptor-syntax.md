## Table 87 — DVB-HTML application location descriptor syntax
_§10.10.2, PDF pp. 173-173_

| No. of Bits | Identifier | Value |
|---|---|---|
| dvb_html_application_location_descriptor () { |  |  |
| descriptor_tag | 8 | uimsbf |  |
| descriptor_length | 8 | uimsbf |  |
| physical_root_length | 8 | uimsbf | N1 |
| for(i=0; i<N1; i++) { |  |  |  |
| physical_root_bytes | 8 | uimsbf |  |
| } |  |  |  |
| for(i=0; i<N; i++) { |  |  |  |
| initial_path_bytes | 8 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

descriptor_tag = 0x09. Example (10.10.2.2): physical_root_bytes = "application/", initial_path_bytes = "main/index.htm".
