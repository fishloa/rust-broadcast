# Auxiliary file system resource — CICAM file retrieval (CI Plus)

_Source: ETSI TS 103 205 v1.4.1 §9, Tables 72-75 (PDF pp. 96-99), render-verified_

CICAM file retrieval is realized via the **auxiliary file system resource**
(resource ID `0x00910041`), a generic read-only mechanism for a CICAM to offer
files to the Host (including CICAM broadcast applications to launch). It inherits
most of its messages and semantics from the Application MMI resource v2 (§14.5 of
the CI Plus V1.3 spec [3]). Tags live in the CI Plus `0x9F94xx` namespace.

## Table 75 — Auxiliary file system resource summary (PDF p. 99)

Resource Identifier `0x00910041` — Class 145, Type 1, Version 1.

| APDU Tag | Tag value | Host | CICAM |
|----------|-----------|:----:|:-----:|
| FileSystemOffer | `9F 94 00` |   | → (CICAM→Host) |
| FileSystemAck   | `9F 94 01` | → (Host→CICAM) | |
| FileRequest     | `9F 94 02` | → (Host→CICAM) | |
| FileAcknowledge | `9F 94 03` |   | → (CICAM→Host) |

## §9.2 — File system offer APDU — Table 72 (PDF p. 98)

Specifies the file system provided by the CICAM. Only one FileSystemOffer shall
be sent per auxiliary file system resource session.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `FileSystemOffer() {` | | |
| &nbsp;&nbsp;FilesystemOffer_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;DomainIdentifierLength | 8 | uimsbf |
| &nbsp;&nbsp;`for (i = 0; i < DomainIdentifierLength; i ++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;DomainIdentifier | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field semantics (§9.2, p. 98):
- **FileSystemOffer_tag** (24) — value `0x9F9400` identifies this message.
- **length_field** — ASN.1 BER, per EN 50221 §8.3.1.
- **DomainIdentifierLength** (8) — number of bytes of `DomainIdentifier` following.
- **DomainIdentifier** — bytes the Host uses to identify the file system provided by the CICAM (may be the same as the `AppDomainIdentifier` used by the Application MMI resource). No specific format; meaning defined by the middleware (out of scope). Examples: a URL (`"www.operator.com/cifilesystem"`); a UUID per IETF RFC 4122 (`"urn:uuid:f81d4fae-7dec-11d0-a765-00a0c91e6bf6"`); a DVB-registered identifier.

## §9.3 — File System Ack APDU — Table 73 (PDF p. 98)

Sent by the Host in response to FileSystemOffer to confirm whether the offered
application domain is supported.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `FileSystemAck() {` | | |
| &nbsp;&nbsp;FilesystemAck_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;AckCode | 8 | bslbf |
| `}` | | |

Field semantics (§9.3, p. 98):
- **FileSystemAck_tag** (24) — value `0x9F9401` identifies this message.
- **length_field** — ASN.1 BER, per EN 50221 §8.3.1.
- **AckCode** (8) — response to the FileSystemOffer; see Table 74.

### Table 74 — AckCode values (PDF p. 98)

| AckCode | Meaning |
|---------|---------|
| `0x00` | Reserved for future use. |
| `0x01` | OK. The application environment is supported by the Host. |
| `0x02` | Unknown DomainIdentifier. The DomainIdentifier is not supported by the Host. |
| `0x03`–`0xFF` | Reserved for future use. |

If the Host responds with AckCode equal to "wrong API", the CICAM shall close this
session of the resource.

## §9.4 — File request APDU (FileRequest, tag `0x9F9402`)

This APDU is **copied from the Application MMI v2 resource** and is fully
described in §14.5.1 of CI Plus V1.3 [3]. It allows the Host to support file
caching and to discover reqtypes supported by the CICAM (file, data, hash, etc.).

⚠ Syntax NOT printed in TS 103 205 — it is by reference to CI Plus V1.3 [3]
§14.5.1 (proprietary). Only the apdu_tag `0x9F9402` and direction (Host→CICAM) are
established here.

## §9.5 — File acknowledge APDU (FileAcknowledge, tag `0x9F9403`)

This APDU is **copied from the Application MMI v2 resource** and is fully
described in §14.5.2 of CI Plus V1.3 [3].

⚠ Syntax NOT printed in TS 103 205 — by reference to CI Plus V1.3 [3] §14.5.2
(proprietary). Only the apdu_tag `0x9F9403` and direction (CICAM→Host) are
established here.

## §9.7 — Coordination with Application MMI

When both the Auxiliary File System and Application MMI resources are open:
- CICAM AppMMI applications (and applications they launch) use the **Application MMI** resource.
- All other applications (including an MHEG-5 CICAM broadcast application accessing the CI_SendMessage (CIS) resident programme, per ETSI MHEG Broadcast Profile [6] §11.10.11) use the **Auxiliary File System** resource.
