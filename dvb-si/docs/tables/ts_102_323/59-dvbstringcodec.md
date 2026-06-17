## Table 59 — dvbStringCodec
_§9.4.3.4.3, PDF pp. 66-66_

| Syntax | No. of bits | Identifier |
|---|---|---|
| dvbStringCodec () { | | |
| if( isFollowingSkippableElement == 1) { | | |
| string offset | 16 | uimsbf |
| resynchronizeCodec(string offset) | | |
| } | | |
| getNextStringFromBuffer() | | |
| } | | |

