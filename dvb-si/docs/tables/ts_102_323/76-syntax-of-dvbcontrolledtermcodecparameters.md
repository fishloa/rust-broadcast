## Table 76 — Syntax of dvbControlledTermCodecParameters
_§9.4.4.2, PDF pp. 74-75_

| Syntax | No. of bits | Identifier |
|---|---|---|
| dvbControlledTermCodecParameters() { | | |
| nbClassificationSchemes | 8+ | vluimsbf8 |
| for (j=0; j<nbClassificationSchemes; j++) { | | |
| ClassificationSchemeURI Length[j] | 8+ | vluimsbf8 |
| ClassificationSchemeURI[j] | 8*ClassificationSchemeURI_Length[j] | bslbf |
| } | | |
| } | | |

