## Table 24 — Result_data
_§7.3.2.3.2, PDF pp. 39-40_

| Syntax | No. of bits | Identifier |
|---|---|---|
| result data () { | | |
| year offset | 16 | uimsbf |
| for(j=0; j<Table size; j += sizeof(Result)) { | | |
| Status | 2 | uimsbf |
| acquisition flag | 1 | bslbf |
| re resolve flag | 1 | bslbf |
| result type | 2 | bslbf |
| imi flag | 1 | bslbf |
| Reserved | 1 | bslbf |
| if(status=='00') { | | |
| num results | 8 | uimsbf |
| for(r=0; r<num results; r++) { | | |
| if(result type == '00') { | | |
| CRID prepend ptr | 16 | uimsbf |
| result CRID data ptr | 16 | uimsbf |
| } | | |
| else if(result type == '01') { | | |
| dvb binary locator() | | |
| } | | |
| else if(result type == '10') { | | |
| locator format | 4 | uimsbf |
| locator length | 12 | uimsbf |
| if (locator format == 0x00) { | | |
| for(j=0; j<(locator length - 1); j++) { | | |
| URI byte | 8 | uimsbf |
| } | | |
| 0x00 | 8 | uimsbf |
| } | | |
| else if (locator format == 0x01) { | | |
| dvb binary locator() | | |
| } | | |
| else if (locator format == 0x02) { | | |
| scheduled decomposed binary locator() | | |
| } | | |
| else if (locator format == 0x03) { | | |
| on-demand decomposed binary locator() | | |
| } | | |
| else if (locator format == 0x04) { | | |
| extended on-demand decomposed binary locator() | | |
| } | | |
| else { | | |
| for(j=0; j<locator length; j++) { | | |
| locator byte | 8 | uimsbf |
| } | | |
| } | | |
| } | | |
| else { | | |
| DVB reserved length | 16 | uimsbf |
| for (i=0; i<DVB reserved length; i++) { | | |
| DVB reserved byte | 8 | uimsbf |
| } | | |
| } | | |
| if(result type != '00' && imi flag == '1') { | | |
| imi prepend ptr | 16 | uimsbf |
| result imi data ptr | 16 | uimsbf |
| } | | |
| } | | |
| } | | |
| if (status == '01' \|\| (status == '00' && re resolve flag == '1') { | | |
| reserved | 7 | bslbf |
| reresolve date | 9 | uimsbf |
| reresolve time | 16 | uimsbf |
| } | | |
| } | | |
| } | | |

