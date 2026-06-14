# Table 4.2: Bit stream mode

_Source: specs/etsi_ts_102_366_v01.04.01_ac3_eac3_audio.pdf §4 BSI/syncframe + code tables (pp.33-48) and Annex A DVB descriptors (pp.96-100). Audio-decode internals (exponents/mantissas/transforms) out of scope._


|  bsmod | acmod | Type of service  |
| --- | --- | --- |
|  000 | Any | Main audio service: complete main (CM)  |
|  001 | Any | Main audio service: music and effects (ME)  |
|  010 | Any | Associated service: visually impaired (VI)  |
|  011 | Any | Associated service: hearing impaired (HI)  |
|  100 | Any | Associated service: dialogue (D)  |
|  101 | Any | Associated service: commentary (C)  |
|  110 | Any | Associated service: emergency (E)  |
|  111 | 001 | Associated service: voice over (VO)  |
|  111 | 010 to 111 | Main audio service: karaoke  |

### 4.4.2.3 acmod - Audio coding mode - 3 bits

This 3-bit code, shown in Table 4.3, indicates which of the main service channels are in use, ranging from 3/2 to 1/0. If the MSB of acmod is a 1, surround channels are in use and surmixlev follows in the bit stream. If the MSB of acmod is a 0, the surround channels are not in use and surmixlev does not follow in the bit stream. If the LSB of acmod is a 0, the centre channel is not in use. If the LSB of acmod is a 1, the centre channel is in use.



NOTE: The state of acmod sets the number of full-bandwidth channels parameter, nfchans, (e.g. for 3/2 mode, nfchans = 5; for 2/1 mode, nfchans = 3; etc.). The total number of channels, nchans, is equal to nfchans if the life channel is off, and is equal to 1 + nfchans if the life channel is on. If acmod is 0, then two completely independent programme channels (dual mono) are encoded into the bit stream, and are referenced as Ch1, Ch2. In this case, a number of additional items are present in BSI or audblk to fully describe Ch2. Table 4.3 also indicates the channel ordering (the order in which the channels are processed) for each of the modes.
