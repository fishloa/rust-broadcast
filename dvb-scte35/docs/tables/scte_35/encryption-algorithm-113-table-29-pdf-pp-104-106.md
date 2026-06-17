## Encryption algorithm — §11.3, Table 29, PDF pp. 104-106

Encryption of the section (from `splice_command_type` through `E_CRC_32`) is
optional. Fixed key encryption (§11.2): the same key is provided to both
transmitter and receiver by unspecified means; up to 256 keys may be held,
selected by `cw_index`. If encryption is implemented, a receive device shall
implement all of the algorithms listed. All DES variants use a 64-bit key
(actually 56 bits plus a checksum) on 8-byte blocks; triple DES needs three
64-bit keys, one per pass ("standard" triple DES uses two keys, the first
and third identical). For DES-ECB and DES-CBC the encrypted data shall be a
multiple of 8 bytes from `splice_command_type` through `E_CRC_32` (pad with
the `alignment_stuffing` loop); DES-CBC uses an initial vector with a fixed
value of zero. NOTE (§11.3): FIPS Publication 46-3 was withdrawn on May 19,
2005, and implementers that require encryption may wish to use a user
private algorithm.

| Value | Encryption algorithm |
|---|---|
| 0 | No encryption |
| 1 | DES – ECB mode |
| 2 | DES – CBC mode |
| 3 | Triple DES EDE3 – ECB mode |
| 4–31 | Reserved |
| 32–63 | User private |

