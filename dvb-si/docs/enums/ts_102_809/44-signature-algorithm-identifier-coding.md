## Table 44 — Signature algorithm identifier coding
_§9.4.3, PDF pp. 72-72_

| signature_algorithm_identifier | Description |
|---|---|
| 0x00 | reserved for future use |
| 0x01 | ed25519 without pre-hash, according to IETF RFC 8032 [27] |
| 0x02 | sha256WithRSAEncryption according to IETF RFC 3447 [25] |
| 0x03 | sha384WithRSAEncryption according to IETF RFC 3447 [25] |
| 0x04 | ecdsa-with-SHA256 according to ANSI X9.62 [22] using curve |
|  | secp256r1 according to SEC 2 [26] |
| 0x05 | ecdsa-with-SHA256 according to ANSI X9.62 [22] using curve |
|  | secp384r1 according to SEC 2 [26] |
| 0x06 | ecdsa-with-SHA384 according to ANSI X9.62 [22] using curve |
|  | secp256r1 according to SEC 2 [26] |
| 0x07 | ecdsa-with-SHA384 according to ANSI X9.62 [22] using curve |
|  | secp384r1 according to SEC 2 [26] |
| 0x08 to 0xFF | reserved for future use |

