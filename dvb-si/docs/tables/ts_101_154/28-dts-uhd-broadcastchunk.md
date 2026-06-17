## Table 28 — DTS-UHD BroadcastChunk
_§6.9.3.1, PDF pp. 185-186_

| Syntax | Number of bits | Identifier |
|---|---|---|
| DTSUHD_BCHUNK | 32 | bslbf |
| ByteCount | 8 | uimsbf |
| Version | 3 | uimsbf |
| numLanguages | 5 | uimsbf |
| for (i=0; i ≤ numLanguages; i++) { // LanguageIndex = i | | |
| ISO639_code // Language Table | 24 | bslbf |
| } | | |
| for (i=0; i ≤ numLanguages; i++) { // Language Groups | | |
| b_UserByte | 1 | bslbf |
| reserved_bits | 2 | blsbl |
| numSelectionSets [i] // Preselections per group | 5 | uimsbf |
| for (j = 0; j ≤ numSelectionSets [i]; j++) { // ProgramIndex = j | | |
| AudioDescription // properties of Preselection | 1 | bslbf |
| SpokenSubtitle | 1 | bslbf |
| DialogueEnhancement | 1 | bslbf |
| if (b_UserByte) | | |
| UserByte | 8 | bslbf |
| numComponents | 3 | uimsbf |
| reserved_bits | 2 | bslbf |
| for (k = 0; k ≤ numComponentGroups; k++) { // each Preselection | | |
| StreamID | 3 | uimsbf |
| ComponentGroupID | 5 | uimsbf |
| } // numComponentGroups | | |
| } //numSelectionSets | | |
| } //numLanguages | | |
| CRC16 | 16 | bslbf |

