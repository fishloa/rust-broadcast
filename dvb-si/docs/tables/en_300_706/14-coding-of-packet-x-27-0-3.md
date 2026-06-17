# Table 14: Coding of Packet X/27/0-3

_Source: specs/etsi_en_300_706_v01.02.01_enhanced_teletext.pdf §9-12 packet codings (PDF pp. 23-46). Character-set / display glyph tables (35+) deferred (rendering, not parsing)._


|  Byte | Data Bits | Function  |
| --- | --- | --- |
|  6 | 1-4 | Designation code  |
|  7-12 | 1-4 | Editorial link 0  |
|  13-18 | 1-4 | Editorial link 1  |
|  19-24 | 1-4 | Editorial link 2  |
|  25-30 | 1-4 | Editorial link 3  |
|  31-36 | 1-4 | Editorial link 4  |
|  37-42 | 1-4 | Editorial link 5  |
|  43 | 1, 2, 3 | Link Control Byte - see note. Coded Hamming 8/4. In the absence of any local Code of Practice, these bits should be set to '1'.  |
|   | 4 | '0': Data in packets with Y = 24 is not to be displayed. '1': Data in packets with Y = 24 is to be displayed in row 24.  |
|  44-45 | 1-8 | Cyclic Redundancy Check word (CRC) on data in packets X/0 to X/25 of the associated page - see note. Coded each as 8 bits data. The calculation is described below.  |
|  NOTE: Bytes 43, 44 and 45 have this significance for packets X/27/0 only. These bytes are reserved in packets X/27/1, X/27/2 and X/27/3.  |   |   |


The check word is generated in the following manner using the conceptual model of a 16 bit shift register, figure 13, having as input the modulo-2 sum of an external input and the contents of the 7th, 9th, 12th and 16th stages of the register. Initially the register is cleared to "all zeros". During a sequence of 8 192 clock pulses bytes 14 to 37 from packet X/0 and the following character bytes (bytes 6 to 45) of packets X/1 up to X/25, in ascending address order, form the input. Any absent packets are considered to contain the character "space" (2/0) throughout. For each byte, the bits are applied to the input in the order b8 to b1 inclusive. This order, the reverse of that used in the transmission sequence, is to facilitate decoder operation where the data used is stored in the page memory.

At the transmitting end of the generating process the contents of the register are the basic page check word and it is transmitted along the register beginning with the bit held in the first stage.

The transmission order for the two byte group resulting from the 16-bit cyclic redundancy check on the page is bits 9 to 16 followed by bits 1 to 8 inclusive.

![img-1.jpeg](img-1.jpeg)
Figure 13: Check word generation

## 9.6.2 Packets X/27/4 and X/27/5 - Format 1 - for compositional linking in presentation enhancement applications

Format 1 packets X/27 have valid designation codes of 0100 and 0101. The packets define compositional links to enhancement data pages (i.e. DRCS downloading pages and object definition pages) at Levels 2.5 and 3.5.

The structure of Format 1 packets X/27/4 and X/27/5 is shown in figure 14.

![img-2.jpeg](img-2.jpeg)
Figure 14: Format of Format 1 packets X/27/4 and X/27/5 for compositional linking

Byte 6 is the designation code, coded Hamming 8/4. Bytes 7 to 42 are arranged as 6 groups of $3 + 3$ bytes, each subgroup of 3 bytes being one Hamming 24/18 coded triplet. Each group of 6 bytes defines a linked page address, the groups being numbered 0 to 5 in order of transmission. Bytes 43 to 45 are also Hamming 24/18 coded but the data bits are reserved for future use.



Each linked page address of 3 + 3 bytes contains 36 data bits:

Relative magazine number: 3 bits;
Page number: 8 bits;
Page sub-code flags: 16 bits;
Link function flags: 4 bits;
Compatibility bits 2 bits;
Reserved: 3 bits.

The mapping of these functions within a two triplet group, and the allocation of links to triplets, is shown in table 15.
