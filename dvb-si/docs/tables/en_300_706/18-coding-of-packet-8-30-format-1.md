# Table 18: Coding of Packet 8/30 Format 1

_Source: specs/etsi_en_300_706_v01.02.01_enhanced_teletext.pdf §9-12 packet codings (PDF pp. 23-46). Character-set / display glyph tables (35+) deferred (rendering, not parsing)._


|  Bytes | Bits | Function  |
| --- | --- | --- |
|  6 | 1-4 | Designation code (Hamming 8/4 coded)  |
|  6 | 1 | '0' = Multiplexed function as defined in clause 4.1 (see note 1) '1' = Non-multiplexed function as defined in clause 4.2 (see note 1)  |
|  6 | 2-4 | When set to 000, bytes 7 to 45 have the functions designated in this table.  |
|  7-12 |  | Initial Teletext Page (for storage by a decoder without user action) (All bytes Hamming 8/4 coded.) (see notes 1 and 2)  |
|  7 | 1-4 | Page Units (LSB - MSB)  |
|  8 | 1-4 | Page Tens (LSB - MSB)  |
|  9 | 1-4 | Sub-code value S1 (LSB - MSB)  |
|  10 | 1-3 | Sub-code value S2 (LSB - MSB)  |
|  10 | 4 | (Absolute) Magazine address bit, weight 20  |
|  11 | 1-4 | Sub-code value S3 (LSB - MSB)  |
|  12 | 1-2 | Sub-code value S4 (LSB - MSB)  |
|  12 | 3-4 | (Absolute) Magazine address bits, weight 21 and 22 respectively  |
|  13-14 |  | Network Identification Code (coded 8 bits data) This permanently assigned code uniquely defines the network. The allocation of NI codes to networks is defined in TR 101 231 [6]. NOTE: The 16 bit NI value is transmitted most significant bit first. Thus the MSB is mapped to byte 13, bit 1 and the LSB to byte 14, bit 8.  |
|  15 |  | Time Offset Code (coded 8 bits data)  |
|  15 | 1 | Reserved for future use.  |
|  15 | 2-6 | Defines an offset, in hour units, between local time and Co-ordinated Universal Time (UTC). Bit: 2 3 4 5 6 Value (hours): 1/2 1 2 4 8  |
|  15 | 7 | Offset polarity. Negative offsets are west of Greenwich. '0' = positive offset '1' = negative offset  |
|  15 | 8 | Reserved for future use.  |
|  16-18 |  | Modified Julian Date (coded 8 bits data) A 5-digit (decimal) number defining Modified Julian Date (MJD), incrementing daily at midnight UTC. Reference point is 31 January 1982, MJD 45000. Each digit is incremented by one prior to transmission. Pairs of 4-bit values are assembled into bytes and the bytes are transmitted least significant bit first.  |
|  16 | 5-8 | Reserved  |
|  17 | 1-4 | 10^{4} (LSB - MSB)  |
|  17 | 5-8 | 10^{3} (LSB - MSB)  |
|  18 | 1-4 | 10^{2} (LSB - MSB)  |
|  18 | 5-8 | 10^{1} (LSB - MSB)  |
|  18 | 1-4 | 10^{0} (LSB - MSB)  |
|  19-21 |  | Universal Time Co-ordinated (coded 8 bits data) 6-digit number defining Universal Time Co-ordinated(UTC). The transmission relates to the next following second. Each digit is incremented by one prior to transmission.  |
|  19 | 5-8 | Hours Tens (LSB - MSB)  |
|  19 | 1-4 | Hours Units (LSB - MSB)  |
|  20 | 5-8 | Minutes Tens (LSB - MSB)  |
|  20 | 1-4 | Minutes Units (LSB - MSB)  |
|  21 | 5-8 | Seconds Tens (LSB - MSB)  |
|  21 | 1-4 | Seconds Units (LSB - MSB)  |
|  22-25 |  | Reserved  |
|  26-45 |  | Status Display (coded 7 bits plus odd parity). (see note 1) These bytes are coded with odd parity characters from the default G0 character set and, where appropriate, using the characters common to the range of options. The use of national option characters is not recommended. It is intended to display a transmission status message, e.g. the programme title.  |
|  NOTE 1: When packets 8/30 Format 2 are also present in a given transmission, the multiplexed operation flag in the designation code and the data in bytes 7 to 12 and 26 to 45 should be the same for both formats. NOTE 2: When no particular page number is to be specified, the page number FF is transmitted. When no particular page sub-code is to be specified the page sub-code 3F7F is transmitted. When the page address FF:3F7F is transmitted, no page is specified.  |   |   |



## 9.8.2 Packet 8/30 Format 2

Packets 8/30 Format 2 have designation code values of 0010 or 0011. They carry broadcast service data relating to the TV channel, including:

- multiplexed transmission flag;
- initial Teletext page number;
- TV programme identification data for VCR control;
- status display.

![img-4.jpeg](img-4.jpeg)
Figure 16: Coding of Packet 8/30 Format 2

The coding of bytes 7 to 45 shown in table 19 applies when the designation value is 0010 or 0011. See EN 300 231 [1] for the specification of the Programme Identification Data transmitted in this packet.
