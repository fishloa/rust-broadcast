## Table 2-47 — Frame rate code
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.3, Table 2-47; PDF p.78. The 4-bit code values are as defined in §6.3.3 of Rec. ITU-T H.262 | ISO/IEC 13818-2 (Table 2-47 references that clause for the codes); the "also includes" column lists the additional frame rates permitted to be present when multiple_frame_rate_flag = '1'._

| frame_rate_code | Coded as | Also includes |
|---|---|---|
| 0x0 | forbidden |  |
| 0x1 | 23.976 |  |
| 0x2 | 24.0 | 23.976 |
| 0x3 | 25.0 |  |
| 0x4 | 29.97 | 23.976 |
| 0x5 | 30.0 | 23.976, 24.0, 29.97 |
| 0x6 | 50.0 | 25.0 |
| 0x7 | 59.94 | 23.976, 29.97 |
| 0x8 | 60.0 | 23.976, 24.0, 29.97, 30.0, 59.94 |
| 0x9–0xF | reserved |  |
