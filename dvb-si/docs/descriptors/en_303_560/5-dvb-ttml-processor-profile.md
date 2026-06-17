## Table 5 — DVB TTML processor profile
_§5.2.1.1, PDF pp. 16-16_

| dvb_ttml_profile | TTML processor profile identifier (from TTML registry [10]) | Comment |
|---|---|---|
| 0x00 | etd1†\|im1t | Default conformance point. Requires EBU-TT-D [3] processor compliant with the additional constraints(†) of the default conformance point defined in clause 6.6, or an IMSC1 Text Profile processor |
| 0x01 | im1t | IMSC1 [4] Text Profile processor |
| 0x02 | etd1 | EBU-TT-D [3] (without necessarily supporting the constraints of the default conformance point defined in clause 6.6) |
| 0x03-0xFF | | reserved for future use |

