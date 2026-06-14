# Table 19: Coding of Packet 8/30 Format 2

_Source: specs/etsi_en_300_706_v01.02.01_enhanced_teletext.pdf §9-12 packet codings (PDF pp. 23-46). Character-set / display glyph tables (35+) deferred (rendering, not parsing)._


|  Bytes | Bits | Function  |
| --- | --- | --- |
|  6 | 1 | Designation code (Hamming 8/4 coded)  |
|  6 |   | ‘0’ = Multiplexed function as defined in clause 4.1 (see note 1) ‘1’ = Non-multiplexed function as defined in clause 4.2 (see note 1)  |
|  6 | 2-4 | When set to 100, bytes 7 to 45 have the functions designated in this table.  |
|  7-12 | 1-4 | Initial Teletext Page (for storage in a decoder without user action) (All bytes Hamming 8/4 coded) (see notes 1 and 2)  |
|  7 |   | Page Units (LSB - MSB)  |
|  8 | 1-4 | Page Tens (LSB - MSB)  |
|  9 | 1-4 | Sub-code value S1 (LSB - MSB)  |
|  10 | 1-3 | Sub-code value S2 (LSB - MSB)  |
|  10 | 4 | (Absolute) Magazine address bit, weight 20  |
|  11 | 1-4 | Sub-code value S3 (LSB - MSB)  |
|  12 | 1-2 | Sub-code value S4 (LSB - MSB)  |
|  12 | 3-4 | (Absolute) Magazine address bits, weight 21 and 22 respectively  |
|  13-25 |  | Programme Identification Data Bytes used for Programme Delivery Control (PDC) applications. Function and coding is defined in clause 8.2.1 of EN 300 231 [1].  |
|  26-45 |  | Status Display (coded 7 bits plus odd parity) (see note 1). These bytes are coded with odd parity characters from the default G0 character set and, where appropriate, using the characters common to the range of options. The use of national option characters is not recommended. It is intended to display a transmission status message, e.g. the programme title.  |
|  NOTE 1: When packets 8/30 Format 1 are also present in a given transmission, the multiplexed operation flag in the designation code and the data in bytes 7 to 12 and 26 to 45 should be the same for both formats. NOTE 2: When no particular page number is to be specified, the page number FF is transmitted. When no particular page sub-code is to be specified the page sub-code 3F7F is transmitted. When the page address FF:3F7F is transmitted, no page is specified.  |   |   |

# 10 System Components for Presentation

This clause defines system components which are related to the presentation of Teletext data.

# 10.1 Basic Teletext - Presentation Levels 1 and 1.5

Figure 17 summarizes the packets used in systems with presentation Levels 1 and 1.5.

![img-5.jpeg](img-5.jpeg)
Figure 17: System components for presentation Levels 1 and 1.5
