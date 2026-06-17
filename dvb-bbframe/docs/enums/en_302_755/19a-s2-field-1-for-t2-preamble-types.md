## Table 19a — S2 Field 1 (for T2 preamble types)
_§7.2.1, PDF p. 61 — S1=00X, 011 and 100_

| S2 field 1 | S2 field 2 | FFT/GI size (T2-base profile) | FFT/GI size (T2-Lite profile) | Description |
|---|---|---|---|---|
| 000 | X | FFT Size: 2K — any allowed guard interval | FFT Size: 2K — any allowed guard interval | Indicates the FFT size and guard interval of the symbols in the T2-frame |
| 001 | X | FFT Size: 8K — guard intervals 1/32; 1/16; 1/8 or 1/4 | FFT Size: 8K — guard intervals 1/32; 1/16; 1/8 or 1/4 | |
| 010 | X | FFT Size: 4K — any allowed guard interval | FFT Size: 4K — any allowed guard interval | |
| 011 | X | FFT Size: 1K — any allowed guard interval | FFT Size: 16K — guard intervals 1/128; 19/256 or 19/128 | |
| 100 | X | FFT Size: 16K — any allowed guard interval | FFT Size: 16K — guard intervals 1/32; 1/16; 1/8 or 1/4 | |
| 101 | X | FFT Size: 32K — guard intervals 1/32; 1/16; or 1/8 | Reserved for future use | |
| 110 | X | FFT Size: 8K — guard intervals 1/128; 19/256 or 19/128 | FFT Size: 8K — guard intervals 1/128; 19/256 or 19/128 | |
| 111 | X | FFT Size: 32K — guard intervals 1/128; 19/256 or 19/128 | Reserved for future use | |

